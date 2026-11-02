//! Local OS integration shared by CLI and Desktop. Never exposed by the daemon API.
mod files;
pub mod profiles;

pub type Result<T> = std::result::Result<T, String>;

pub fn state_dir() -> Result<std::path::PathBuf> {
    use std::{env, path::PathBuf};
    if let Some(path) = env::var_os("HPS_CLIENT_STATE_DIR") {
        let path = PathBuf::from(path);
        return path
            .is_absolute()
            .then_some(path)
            .ok_or("HPS_CLIENT_STATE_DIR must be absolute".into());
    }
    #[cfg(target_os = "windows")]
    let root = PathBuf::from(env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA unavailable")?);
    #[cfg(target_os = "macos")]
    let root = PathBuf::from(env::var_os("HOME").ok_or("HOME unavailable")?)
        .join("Library/Application Support");
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let root = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .ok_or("client configuration directory unavailable")?;
    Ok(root.join("honk-proxy-server"))
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && id.as_bytes()[0].is_ascii_alphanumeric()
}
