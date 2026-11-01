#![cfg(target_os = "linux")]
use hps_config::LinuxCaptureConfig;
use hps_platform::transparent::{CaptureSockets, PreparedCapture};
use std::{
    collections::BTreeSet,
    os::fd::AsFd,
    path::{Path, PathBuf},
    time::Duration,
};

struct Group(PathBuf);
impl Drop for Group {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}
fn loaded_program_ids() -> BTreeSet<u32> {
    aya::programs::loaded_programs()
        .filter_map(Result::ok)
        .map(|info| info.id())
        .collect()
}
#[tokio::test]
#[ignore = "requires actual privileged Linux, private writable cgroup v2, TCX and an independently compiled object"]
async fn real_transparent_programs_load_disabled_with_owned_links_and_release_every_resource() {
    let object = std::env::var("HPS_EBPF_OBJECT").expect("HPS_EBPF_OBJECT");
    let root = std::env::var("HPS_TEST_CGROUP_ROOT")
        .expect("HPS_TEST_CGROUP_ROOT must name an isolated writable cgroup mount");
    let group =
        Group(Path::new(&root).join(format!("hps-transparent-load-{}", std::process::id())));
    std::fs::create_dir(&group.0).unwrap();
    let specification = LinuxCaptureConfig {
        object: object.into(),
        local_cgroup: Some(group.0.clone()),
        tcp_port: 0,
        udp_port: 0,
        ..Default::default()
    };
    let mark = specification.bypass_mark;
    let tcp4 = hps_net::transparent::bind_tcp("0.0.0.0:0".parse().unwrap(), mark, 128).unwrap();
    let tcp6 = hps_net::transparent::bind_tcp("[::]:0".parse().unwrap(), mark, 128).unwrap();
    let udp4 = hps_net::transparent::bind_udp("0.0.0.0:0".parse().unwrap(), mark).unwrap();
    let udp6 = hps_net::transparent::bind_udp("[::]:0".parse().unwrap(), mark).unwrap();
    let before = loaded_program_ids();
    let prepared = PreparedCapture::read(&specification).unwrap();
    prepared.verify().unwrap();
    let fingerprint = prepared.fingerprint.clone();
    let capture = prepared
        .load(CaptureSockets {
            tcp4: Some(tcp4.as_fd()),
            tcp6: Some(tcp6.as_fd()),
            udp4: Some(udp4.as_fd()),
            udp6: Some(udp6.as_fd()),
        })
        .unwrap();
    let programs = capture.program_ids().unwrap();
    let maps = capture.map_ids().unwrap();
    assert_eq!(programs.len(), 7);
    assert!(programs.iter().all(|id| *id > 0 && !before.contains(id)));
    assert!(!maps.is_empty() && maps.iter().all(|id| *id > 0));
    assert!(capture.links_present().unwrap());
    let stats = capture.stats().unwrap();
    assert!(
        stats
            .iter()
            .all(|s| s.captured_packets == 0 && s.marked_sockets == 0)
    );
    let conflicting = PreparedCapture::read(&specification)
        .unwrap()
        .load(CaptureSockets {
            tcp4: Some(tcp4.as_fd()),
            tcp6: Some(tcp6.as_fd()),
            udp4: Some(udp4.as_fd()),
            udp6: Some(udp6.as_fd()),
        });
    assert!(conflicting.err().is_some_and(|e| e.contains("ownership")));
    assert!(capture.links_present().unwrap());
    capture.shutdown().unwrap();
    capture.shutdown().unwrap();
    assert!(!capture.links_present().unwrap());
    tokio::time::timeout(Duration::from_secs(3), async {
        while programs.iter().any(|id| loaded_program_ids().contains(id)) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3),async {
        loop {
            if maps.iter().all(|id| matches!(aya::maps::MapInfo::from_id(*id), Err(aya::maps::MapError::SyscallError(ref e)) if e.io_error.raw_os_error()==Some(libc::ENOENT))) { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    let restarted = PreparedCapture::read(&specification)
        .unwrap()
        .load(CaptureSockets {
            tcp4: Some(tcp4.as_fd()),
            tcp6: Some(tcp6.as_fd()),
            udp4: Some(udp4.as_fd()),
            udp6: Some(udp6.as_fd()),
        })
        .unwrap();
    assert!(restarted.links_present().unwrap());
    restarted.shutdown().unwrap();
    println!(
        "{}",
        serde_json::json!({"scope":"disabled_loader_lifecycle_only","kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"object_sha256":fingerprint,"program_ids":programs,"map_ids":maps,"tcx_links":2,"namespace_conflict_rejected":true,"namespace_reacquired":true,"disabled_no_capture":true,"programs_released":true,"maps_released":true,"forwarding_verified":false})
    );
}
