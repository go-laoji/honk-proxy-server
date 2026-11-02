use std::{env, fs, path::PathBuf};

#[path = "verify.rs"]
mod verify;

fn main() {
    let repository=PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    println!("cargo:rerun-if-env-changed=HPS_WEB_ASSETS_DIR");
    let root = env::var_os("HPS_WEB_ASSETS_DIR").map(PathBuf::from).unwrap_or_else(||repository.join("dist"));
    println!("cargo:rerun-if-changed={}", root.display());
    let source_manifest = repository.join("web/console/pages.manifest.json");
    println!("cargo:rerun-if-changed={}", source_manifest.display());
    let source: serde_json::Value =
        serde_json::from_slice(&fs::read(source_manifest).expect("authoritative page manifest"))
            .expect("valid page manifest");
    let source_pages = source
        .as_array()
        .expect("page array")
        .iter()
        .map(|p| {
            (
                p["id"].as_str().expect("page id").to_owned(),
                p["file"].as_str().expect("page file").to_owned(),
            )
        })
        .collect::<Vec<_>>();
    let bundle = verify::verify(&root, &source_pages)
        .unwrap_or_else(|error| panic!("invalid shared web artifact: {error}"));
    let id = &bundle.build_id;
    let pages = &bundle.pages;
    let mut source = format!("pub const BUILD_ID: &str = {id:?};\npub const PAGES: &[&str] = &[\n");
    for page in pages {
        source.push_str(&format!("{:?},\n", page));
    }
    source.push_str("];\npub fn get(path: &str) -> Option<&'static [u8]> {\nmatch path {\n");
    let frozen = PathBuf::from(env::var("OUT_DIR").unwrap()).join("verified-web");
    for (relative, bytes) in bundle.files {
        let path = frozen.join(&relative);
        fs::create_dir_all(path.parent().unwrap()).expect("create immutable web build directory");
        fs::write(&path, bytes).expect("freeze verified web asset");
        source.push_str(&format!(
            "{relative:?} => Some(include_bytes!({:?})),\n",
            path.canonicalize().unwrap()
        ));
    }
    source.push_str("_ => None,\n}\n}\n");
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("assets.rs"),
        source,
    )
    .unwrap();
}
