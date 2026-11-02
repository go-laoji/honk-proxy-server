//! Actual attachment queries, not just the continued existence of program IDs.
use super::{Capture, error, interface_index};
use aya::programs::SchedClassifier;
use std::os::fd::AsRawFd;

pub(super) struct CgroupAttachment {
    pub attach_type: u32,
    pub id: u32,
}
// Linux UAPI bpf_attr.query prefix through prog_cnt (6.12 and later). Zeroed
// trailing padding is explicit; bounded IDs avoid allocation based on foreign
// program counts. Aya 0.14 does not expose cgroup sock query publicly.
#[repr(C)]
#[derive(Default)]
struct Query {
    target_fd: u32,
    attach_type: u32,
    query_flags: u32,
    attach_flags: u32,
    prog_ids: u64,
    prog_cnt: u32,
    reserved: u32,
}
const _: () = assert!(std::mem::size_of::<Query>() == 32);
fn cgroup_contains(fd: &std::fs::File, attachment: &CgroupAttachment) -> Result<bool, String> {
    let mut ids = [0u32; 256];
    let mut query = Query {
        target_fd: fd.as_raw_fd() as u32,
        attach_type: attachment.attach_type,
        prog_ids: ids.as_mut_ptr() as u64,
        prog_cnt: ids.len() as u32,
        ..Default::default()
    };
    // SAFETY: query and the fixed output array are initialized and live through
    // the synchronous syscall; no inherited/effective or modifying flags used.
    let result =
        unsafe { libc::syscall(libc::SYS_bpf, 16, &mut query, std::mem::size_of::<Query>()) };
    if result != 0 {
        return Err(error(
            "cgroup attachment query",
            std::io::Error::last_os_error(),
        ));
    }
    if query.prog_cnt as usize > ids.len() {
        return Err("cgroup attachment query exceeded the bounded ID buffer".into());
    }
    Ok(ids[..query.prog_cnt as usize].contains(&attachment.id))
}
pub(super) fn links_present(capture: &Capture) -> Result<bool, String> {
    use std::os::unix::fs::MetadataExt;
    let guard = capture.state.lock().unwrap();
    let Some(state) = guard.as_ref() else {
        return Ok(false);
    };
    if crate::linux_objects::netns_cookie().map_err(|e| error("current namespace", e))?
        != state.policy.netns_cookie
    {
        return Ok(false);
    }
    if let Some(cgroup) = &state.cgroup {
        let opened = cgroup
            .metadata()
            .map_err(|e| error("cgroup descriptor", e))?;
        let path = capture
            .specification
            .local_cgroup
            .as_ref()
            .ok_or("capture cgroup scope missing")?;
        let named = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(error("cgroup identity", e)),
        };
        if (opened.dev(), opened.ino()) != (named.dev(), named.ino()) {
            return Ok(false);
        }
        for attachment in &capture.cgroup_attachments {
            if !cgroup_contains(cgroup, attachment)? {
                return Ok(false);
            }
        }
    }
    for attachment in &capture.attachments {
        if interface_index(&attachment.interface).ok() != Some(attachment.index) {
            return Ok(false);
        }
        let (_, programs) = SchedClassifier::query_tcx(&attachment.interface, attachment.direction)
            .map_err(|e| error("TCX link inspection", e))?;
        if !programs.iter().any(|p| p.id() == attachment.id) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use crate::transparent::{CaptureSockets, PreparedCapture, test_support::Group};
    use hps_config::LinuxCaptureConfig;
    use hps_ebpf_abi::FlowKey;
    use std::{os::fd::AsFd, path::Path, sync::Arc, time::Duration};

    #[tokio::test]
    #[ignore = "requires isolated privileged Linux, cgroup v2 and actual eBPF object"]
    async fn real_scope_queries_detect_detached_hooks_and_keep_hot_metadata_independent() {
        let root = std::env::var("HPS_TEST_CGROUP_ROOT").unwrap();
        for (index, name) in ["hps_tp_send4", "hps_tp_create"].into_iter().enumerate() {
            let group = Group(Path::new(&root).join(format!(
                "hps-transparent-inspection-{}-{index}",
                std::process::id()
            )));
            std::fs::create_dir(&group.0).unwrap();
            let config = LinuxCaptureConfig {
                object: std::env::var("HPS_EBPF_OBJECT").unwrap().into(),
                local_cgroup: Some(group.0.clone()),
                ..Default::default()
            };
            let mark = config.bypass_mark;
            let tcp4 =
                hps_net::transparent::bind_tcp("0.0.0.0:0".parse().unwrap(), mark, 128).unwrap();
            let tcp6 =
                hps_net::transparent::bind_tcp("[::]:0".parse().unwrap(), mark, 128).unwrap();
            let udp4 = hps_net::transparent::bind_udp("0.0.0.0:0".parse().unwrap(), mark).unwrap();
            let udp6 = hps_net::transparent::bind_udp("[::]:0".parse().unwrap(), mark).unwrap();
            let capture = Arc::new(
                PreparedCapture::read(&config)
                    .unwrap()
                    .load(CaptureSockets {
                        tcp4: Some(tcp4.as_fd()),
                        tcp6: Some(tcp6.as_fd()),
                        udp4: Some(udp4.as_fd()),
                        udp6: Some(udp6.as_fd()),
                    })
                    .unwrap(),
            );
            capture.prepare_routes().unwrap();
            assert!(capture.links_present().unwrap());
            capture.activate(7, true).unwrap();
            assert_eq!(capture.settled_revision().unwrap(), 7);
            let maps = capture.map_ids().unwrap();
            let marker = maps
                .iter()
                .copied()
                .find(|id| {
                    aya::maps::MapInfo::from_id(*id)
                        .is_ok_and(|info| info.name() == hps_ebpf_abi::TP_ABI_MAP.as_bytes())
                })
                .unwrap();
            let mut marker = aya::maps::Array::<_, u32>::try_from(aya::maps::Map::Array(
                aya::maps::MapData::from_id(marker).unwrap(),
            ))
            .unwrap();
            assert!(marker.set(0, 0, 0).is_err());
            drop(marker);
            let cookie = capture.netns_cookie().unwrap();
            // A route query can hold state for seconds. Flow lookups and exact
            // retirement use a separate, short map mutex instead of that lock.
            let state = capture.state.lock().unwrap();
            let (send, receive) = std::sync::mpsc::channel();
            let hot = capture.clone();
            let worker = std::thread::spawn(move || {
                let key = FlowKey {
                    netns_cookie: hot.netns_cookie().unwrap(),
                    ..Default::default()
                };
                send.send(hot.flow(key).unwrap().is_none()).unwrap();
            });
            let independent = receive.recv_timeout(Duration::from_millis(250));
            drop(state);
            worker.join().unwrap();
            assert!(independent.unwrap());

            let journal =
                std::path::PathBuf::from(format!("/run/hps/transparent-netns-{cookie}.json"));
            let before = std::fs::read(&journal).unwrap();
            let changed = capture.clone();
            std::thread::spawn(move || {
                assert_eq!(unsafe { libc::unshare(libc::CLONE_NEWNET) }, 0);
                assert!(!changed.links_present().unwrap());
                assert!(changed.routes_present().is_err());
                assert!(
                    changed
                        .state
                        .lock()
                        .unwrap()
                        .as_mut()
                        .unwrap()
                        .routes
                        .as_mut()
                        .unwrap()
                        .cleanup()
                        .is_err()
                );
                assert!(changed.activate(8, true).is_err());
                assert!(changed.settled_revision().is_err());
                assert!(changed.suspend().is_err());
            })
            .join()
            .unwrap();
            assert_eq!(before, std::fs::read(&journal).unwrap());
            assert!(capture.routes_present().unwrap());
            assert!(capture.links_present().unwrap());

            // Preserve an independent FD so the program and its maps still
            // exist after detachment. Existence alone must not report ready.
            let keep = {
                let mut guard = capture.state.lock().unwrap();
                let bpf = &mut guard.as_mut().unwrap().bpf;
                if index == 0 {
                    let program: &mut aya::programs::CgroupSockAddr =
                        bpf.program_mut(name).unwrap().try_into().unwrap();
                    let info = program.info().unwrap();
                    let keep = info.fd().unwrap();
                    program.unload().unwrap();
                    keep
                } else {
                    let program: &mut aya::programs::CgroupSock =
                        bpf.program_mut(name).unwrap().try_into().unwrap();
                    let info = program.info().unwrap();
                    let keep = info.fd().unwrap();
                    program.unload().unwrap();
                    keep
                }
            };
            assert!(!capture.links_present().unwrap());
            assert!(capture.activate(8, true).is_err());
            capture.shutdown().unwrap();
            drop(keep);
            assert!(!capture.links_present().unwrap());
            assert!(!capture.routes_present().unwrap());
            assert!(capture.flows().is_err());
            tokio::time::timeout(Duration::from_secs(2), async {
                while maps
                    .iter()
                    .any(|id| aya::maps::MapInfo::from_id(*id).is_ok())
                {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            println!(
                "{}",
                serde_json::json!({"scope":"actual_capture_attachment_inspection_only","kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"object_sha256":capture.fingerprint,"detached_program":name,"program_fd_remained_open":true,"detachment_detected":true,"foreign_namespace_refused":true,"original_journal_preserved":true,"metadata_independent_of_route_mutex":true,"settled_revision":7,"retirement_barrier":"protocol_zero_packet_socket_release","maps_released":true,"full_transparent_datapath_ready":false})
            );
        }
    }
}

#[cfg(test)]
#[test]
#[ignore = "requires actual privileged Linux and a separately retained ABI v3 eBPF object"]
fn real_legacy_capture_object_is_rejected_before_any_link() {
    use super::{PreparedCapture, test_support::Group};
    let root = std::env::var("HPS_TEST_CGROUP_ROOT").unwrap();
    let group = Group(
        std::path::Path::new(&root).join(format!("hps-transparent-legacy-{}", std::process::id())),
    );
    std::fs::create_dir(&group.0).unwrap();
    let config = hps_config::LinuxCaptureConfig {
        object: std::env::var("HPS_LEGACY_EBPF_OBJECT").unwrap().into(),
        local_cgroup: Some(group.0.clone()),
        ..Default::default()
    };
    let before = aya::programs::loaded_programs()
        .filter_map(Result::ok)
        .map(|p| p.id())
        .collect::<std::collections::BTreeSet<_>>();
    let prepared = PreparedCapture::read(&config).unwrap();
    let error = prepared.verify().unwrap_err();
    assert!(error.contains("ABI v4 marker missing"), "{error}");
    let after = aya::programs::loaded_programs()
        .filter_map(Result::ok)
        .map(|p| p.id())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(before, after);
    println!(
        "{}",
        serde_json::json!({"scope":"legacy_transparent_abi_rejection_only","kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"legacy_object_sha256":prepared.fingerprint,"required_abi":hps_ebpf_abi::TP_ABI_VERSION,"rejected_before_attach":true,"full_transparent_datapath_ready":false})
    );
}
