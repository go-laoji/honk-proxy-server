use super::{PreparedCapture, test_support::Group};
use aya::maps::{HashMap, Map, MapData};
use hps_config::LinuxCaptureConfig;
use hps_ebpf_abi::{CapturePolicy, TP_ABI_VERSION};
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicBool, Ordering},
};

#[test]
#[ignore = "requires actual privileged Linux and an independently compiled eBPF object"]
fn real_policy_publication_observes_only_complete_snapshots() {
    let group = Group(
        std::path::PathBuf::from(std::env::var("HPS_TEST_CGROUP_ROOT").unwrap())
            .join(format!("hps-transparent-policy-{}", std::process::id())),
    );
    std::fs::create_dir(&group.0).unwrap();
    let specification = LinuxCaptureConfig {
        object: std::env::var("HPS_EBPF_OBJECT").unwrap().into(),
        local_cgroup: Some(group.0.clone()),
        ..Default::default()
    };
    // Verify the actual object and all programs, but do not attach any links or
    // routes. Deliberately distinct fields exercise all 56 bytes under writes.
    let prepared = PreparedCapture::read(&specification).unwrap();
    let (mut bpf, first) = prepared.prepare_bpf().unwrap();
    let second = CapturePolicy {
        abi_version: TP_ABI_VERSION,
        flags: u32::MAX,
        revision: i64::MAX as u64,
        netns_cookie: !first.netns_cookie,
        pidns_device: !first.pidns_device,
        pidns_inode: !first.pidns_inode,
        excluded_tgid: !first.excluded_tgid,
        capture_mark: !first.capture_mark,
        bypass_mark: !first.bypass_mark,
        reserved: u32::MAX,
    };
    let id = match bpf.map("HPS_TP_POLICY").unwrap() {
        Map::HashMap(map) => map.info().unwrap().id(),
        _ => panic!("policy is not a hash"),
    };
    let barrier = Arc::new(Barrier::new(3));
    let complete = AtomicBool::new(false);
    let counts = std::thread::scope(|scope| {
        let mut readers = vec![];
        for _ in 0..2 {
            let barrier = barrier.clone();
            let complete = &complete;
            readers.push(scope.spawn(move || {
                let map = HashMap::<_, u32, CapturePolicy>::try_from(Map::HashMap(
                    MapData::from_id(id).unwrap(),
                ))
                .unwrap();
                barrier.wait();
                let mut reads = 0u64;
                while !complete.load(Ordering::Acquire) || reads < 10_000 {
                    let value = map.get(&0, 0).unwrap();
                    assert!(value == first || value == second, "torn policy: {value:?}");
                    reads += 1;
                }
                reads
            }));
        }
        let mut map =
            HashMap::<_, u32, CapturePolicy>::try_from(bpf.map_mut("HPS_TP_POLICY").unwrap())
                .unwrap();
        barrier.wait();
        for i in 0..10_000 {
            map.insert(0, if i % 2 == 0 { second } else { first }, 2)
                .unwrap();
        }
        complete.store(true, Ordering::Release);
        readers
            .into_iter()
            .map(|r| r.join().unwrap())
            .collect::<Vec<_>>()
    });
    println!(
        "{}",
        serde_json::json!({
            "scope": "kernel_policy_publication_only",
            "kernel": std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),
            "object_sha256": prepared.fingerprint,
            "abi_version": TP_ABI_VERSION,
            "updates": 10_000,
            "reader_lookups": counts,
            "torn_snapshots": 0,
            "full_transparent_datapath_ready": false,
        })
    );
}
