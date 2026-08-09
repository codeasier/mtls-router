use crate::{
    error::{CommandError, Result},
    types::DesktopPaths,
};
use std::{env, path::PathBuf};

pub fn resolve() -> Result<DesktopPaths> {
    let data_dir = if let Some(path) = env::var_os("MTLS_ROUTER_DESKTOP_DATA_DIR") {
        PathBuf::from(path)
    } else {
        platform_data_dir()?
    };
    Ok(DesktopPaths {
        log_directory: data_dir
            .join("mtls-router-logs")
            .to_string_lossy()
            .into_owned(),
        credentials_path: data_dir
            .join("credentials.json")
            .to_string_lossy()
            .into_owned(),
        data_dir: data_dir.to_string_lossy().into_owned(),
        can_prepare_for_uninstall: !cfg!(windows),
    })
}

#[cfg(target_os = "macos")]
fn platform_data_dir() -> Result<PathBuf> {
    home().map(|path| {
        path.join("Library")
            .join("Application Support")
            .join("com.codeasier.mtls-router")
    })
}

#[cfg(windows)]
fn platform_data_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("APPDATA") {
        return Ok(PathBuf::from(path).join("com.codeasier.mtls-router"));
    }
    home().map(|path| {
        path.join("AppData")
            .join("Roaming")
            .join("com.codeasier.mtls-router")
    })
}

#[cfg(target_os = "linux")]
fn platform_data_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(path).join("com.codeasier.mtls-router"));
    }
    home().map(|path| {
        path.join(".local")
            .join("share")
            .join("com.codeasier.mtls-router")
    })
}

fn home() -> Result<PathBuf> {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .ok_or_else(|| CommandError::new("INVALID_PATH", "user data directory is unavailable"))
}
