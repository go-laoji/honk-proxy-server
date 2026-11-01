use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    for entry in fs::read_dir(dir).expect("read web asset directory") {
        let path = entry.expect("read web asset").path();
        if path.is_dir() {
            collect(root, &path, out);
        } else if path.is_file() {
            out.push((
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
                path,
            ));
        }
    }
}

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../dist");
    println!("cargo:rerun-if-changed={}", root.display());
    let manifest = fs::read_to_string(root.join("web-manifest.json"))
        .expect("shared web assets missing: run `pnpm build` before building the web-ui feature; use --no-default-features for headless builds");
    let manifest: serde_json::Value = serde_json::from_str(&manifest).expect("valid web manifest");
    let id = manifest["web_build_id"].as_str().expect("web build ID");
    let pages = manifest["pages"].as_array().expect("web page list");
    let mut source = format!("pub const BUILD_ID: &str = {id:?};\npub const PAGES: &[&str] = &[\n");
    for page in pages {
        source.push_str(&format!("{:?},\n", page.as_str().unwrap()));
    }
    source.push_str("];\npub fn get(path: &str) -> Option<&'static [u8]> {\nmatch path {\n");
    let mut files = vec![];
    collect(&root, &root, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    for (relative, path) in files {
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
