use crate::Result;
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub fn lock(directory: &Path) -> Result<File> {
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(directory.join("state.lock"))
            .map_err(|e| e.to_string())?;
        file.lock_exclusive().map_err(|e| e.to_string())?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("state.lock"))
            .map_err(|e| e.to_string())?;
        file.lock_exclusive().map_err(|e| e.to_string())?;
        Ok(file)
    }
}

pub fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err("local state exceeds 1 MiB".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| format!("invalid local state: {e}"))
}

pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path.parent().ok_or("missing parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
    temporary.write_all(bytes).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    File::open(directory)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    write(
        path,
        &serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
}
