use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=HPS_IP2REGION_DIR");
    let explicit = env::var_os("HPS_IP2REGION_DIR");
    let root = explicit.as_ref().map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../target/ip2region")
    });
    let pointer = root.join("current");
    println!("cargo:rerun-if-changed={}", pointer.display());
    let seed = if pointer.is_file() || explicit.is_some() {
        let version = fs::read_to_string(pointer).expect("missing packaged ip2region snapshot");
        let version = version.trim();
        assert!(
            version.split_once('-').is_some_and(|(v4, v6)| {
                [v4, v6]
                    .into_iter()
                    .all(|sha| sha.len() == 40 && sha.bytes().all(|c| c.is_ascii_hexdigit()))
            }),
            "invalid packaged ip2region snapshot"
        );
        let snapshot = root.join(version);
        let files = ["manifest.json", "ip2region_v4.xdb", "ip2region_v6.xdb"]
            .map(|name| snapshot.join(name));
        let [manifest, ipv4, ipv6] = files.map(|path| {
            println!("cargo:rerun-if-changed={}", path.display());
            path.canonicalize()
                .expect("missing packaged ip2region file")
        });
        format!(
            "Some((include_bytes!({manifest:?}), include_bytes!({ipv4:?}), include_bytes!({ipv6:?})))"
        )
    } else {
        "None".into()
    };
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("ip2region_seed.rs");
    fs::write(output, seed).expect("write packaged ip2region data references");
}
