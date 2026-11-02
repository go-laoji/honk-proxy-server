use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
};

#[derive(Debug)]
pub struct Bundle {
    pub build_id: String,
    pub pages: Vec<String>,
    pub files: BTreeMap<String, Vec<u8>>,
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| "web asset missing")?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read web asset")?;
    if bytes.len() as u64 > limit {
        return Err("web asset exceeds byte limit".into());
    }
    Ok(bytes)
}
fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 256
        && path.is_ascii()
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}
fn collect(
    root: &Path,
    directory: &Path,
    depth: usize,
    entries: &mut usize,
    files: &mut BTreeSet<String>,
) -> Result<(), String> {
    if depth > 8 {
        return Err("web artifact directory nesting exceeds limit".into());
    }
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        *entries += 1;
        if *entries > 8192 {
            return Err("web artifact entry count exceeds limit".into());
        }
        let path = entry.map_err(|e| e.to_string())?.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.is_dir() {
            collect(root, &path, depth + 1, entries, files)?;
        } else if meta.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("non UTF-8 web artifact path")?
                .replace('\\', "/");
            if !safe_path(&relative) || meta.len() > 8 * 1024 * 1024 {
                return Err("invalid or oversized web artifact file".into());
            }
            files.insert(relative);
        } else {
            return Err("web artifacts must not contain symlinks or special files".into());
        }
    }
    Ok(())
}
pub fn verify(root: &Path, source_pages: &[(String, String)]) -> Result<Bundle, String> {
    let mut files = BTreeSet::new();
    collect(root, root, 0, &mut 0, &mut files)?;
    let manifest_bytes = read_bounded(&root.join("web-manifest.json"), 1_048_576)
        .map_err(|_| "shared web assets missing: run pnpm build or use --no-default-features")?;
    if manifest_bytes.len() > 1_048_576 {
        return Err("web manifest exceeds 1 MiB".into());
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&manifest_bytes).map_err(|_| "invalid web manifest JSON")?;
    let assets: BTreeMap<String, String> = serde_json::from_value(manifest["assets"].clone())
        .map_err(|_| "invalid web asset digest map")?;
    if assets.len() > 4096 {
        return Err("web asset count exceeds limit".into());
    }
    let id = manifest["web_build_id"]
        .as_str()
        .ok_or("missing web build ID")?;
    let computed = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&assets).map_err(|e| e.to_string())?)
    );
    if id != computed {
        return Err("web build ID does not match canonical asset digests".into());
    }
    let pages: Vec<String> =
        serde_json::from_value(manifest["pages"].clone()).map_err(|_| "invalid web page list")?;
    let expected: Vec<String> = source_pages.iter().map(|(_, file)| file.clone()).collect();
    let ids: BTreeSet<_> = source_pages.iter().map(|(id, _)| id).collect();
    if expected.len() != 18
        || ids.len() != 18
        || pages != expected
        || pages.iter().collect::<BTreeSet<_>>().len() != 18
    {
        return Err("shared artifact must contain the exact 18 authoritative MPA pages".into());
    }
    let expected_files: BTreeSet<_> = assets
        .keys()
        .cloned()
        .chain(["web-manifest.json".into()])
        .collect();
    if files != expected_files {
        return Err("web artifact file set differs from its manifest".into());
    }
    let mut total = 0;
    let mut blobs = BTreeMap::from([("web-manifest.json".to_owned(), manifest_bytes)]);
    for (path, digest) in &assets {
        if !safe_path(path) || path == "web-manifest.json" {
            return Err("invalid web asset path".into());
        }
        let bytes = read_bounded(&root.join(path), 8 * 1024 * 1024)?;
        total += bytes.len();
        if total > 32 * 1024 * 1024 {
            return Err("shared web artifact exceeds 32 MiB".into());
        }
        if digest != &format!("{:x}", Sha256::digest(&bytes)) {
            return Err(format!("web asset digest mismatch: {path}"));
        }
        blobs.insert(path.clone(), bytes);
    }
    for (page, file) in source_pages {
        if !safe_path(page) || !safe_path(file) || file.contains('/') || !file.ends_with(".html") {
            return Err("invalid authoritative MPA page identity".into());
        }
        let html = std::str::from_utf8(blobs.get(file).ok_or("MPA page missing from digest map")?)
            .map_err(|_| "MPA page is not UTF-8")?;
        if !html.contains(&format!("data-page=\"{page}\"")) {
            return Err(format!("MPA page identity mismatch: {file}"));
        }
    }
    Ok(Bundle {
        build_id: id.into(),
        pages,
        files: blobs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> Vec<(String, String)> {
        let mut pages = Vec::new();
        let mut assets = BTreeMap::new();
        for i in 0..18 {
            let id = format!("page-{i}");
            let name = format!("{id}.html");
            let bytes =
                format!("<body data-page=\"{id}\"><div id=\"root\"></div></body>").into_bytes();
            fs::write(root.join(&name), &bytes).unwrap();
            assets.insert(name.clone(), format!("{:x}", Sha256::digest(bytes)));
            pages.push((id, name));
        }
        let id = format!("{:x}", Sha256::digest(serde_json::to_vec(&assets).unwrap()));
        let value = serde_json::json!({"web_build_id":id,"pages":pages.iter().map(|(_,f)|f).collect::<Vec<_>>(),"assets":assets});
        fs::write(
            root.join("web-manifest.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        pages
    }
    #[test]
    fn corrupt_missing_and_foreign_files_are_rejected_before_embedding() {
        let dir = tempfile::tempdir().unwrap();
        let pages = fixture(dir.path());
        let bundle = verify(dir.path(), &pages).unwrap();
        assert_eq!(bundle.pages.len(), 18);
        assert_eq!(bundle.files.len(), 19);
        assert_eq!(bundle.build_id.len(), 64);
        let file = dir.path().join("page-0.html");
        let original = fs::read(&file).unwrap();
        fs::write(&file, "stale content").unwrap();
        assert!(
            verify(dir.path(), &pages)
                .unwrap_err()
                .contains("digest mismatch")
        );
        fs::write(&file, original).unwrap();
        fs::write(dir.path().join("unlisted.txt"), "must not embed").unwrap();
        assert!(verify(dir.path(), &pages).is_err());
        fs::remove_file(dir.path().join("unlisted.txt")).unwrap();
        fs::remove_file(file).unwrap();
        assert!(verify(dir.path(), &pages).is_err());
    }
    #[test]
    fn verified_bytes_do_not_change_when_another_build_replaces_the_input() {
        let dir = tempfile::tempdir().unwrap();
        let pages = fixture(dir.path());
        let verified = verify(dir.path(), &pages).unwrap();
        let before = verified.files["page-0.html"].clone();
        fs::write(dir.path().join("page-0.html"), "another build").unwrap();
        assert_eq!(verified.files["page-0.html"], before);
        assert!(verify(dir.path(), &pages).is_err());
    }
    #[test]
    fn page_contract_and_manifest_identity_cannot_be_replaced_by_spa_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let mut pages = fixture(dir.path());
        pages[0].0 = "other".into();
        assert!(
            verify(dir.path(), &pages)
                .unwrap_err()
                .contains("identity mismatch")
        );
        let pages = fixture(dir.path());
        let path = dir.path().join("web-manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["web_build_id"] = "invented".into();
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(verify(dir.path(), &pages).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_artifacts_never_embed_files_outside_the_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let pages = fixture(dir.path());
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::remove_file(dir.path().join("page-0.html")).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("page-0.html")).unwrap();
        assert!(verify(dir.path(), &pages).unwrap_err().contains("symlinks"));
    }
}
