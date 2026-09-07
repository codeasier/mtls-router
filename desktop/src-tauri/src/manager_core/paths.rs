//! Manager-owned per-user paths.
//!
//! Frozen against Go `internal/manager/paths`. This module is not the default
//! sidecar runtime path.

use std::path::{Path, PathBuf};

const DESKTOP_APP_ID: &str = "com.codeasier.mtls-router";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Paths {
    pub cli_state_dir: PathBuf,
    pub cli_state_file: PathBuf,
    pub cli_log_file: PathBuf,
    pub desktop_data_dir: PathBuf,
    pub desktop_state_file: PathBuf,
    pub desktop_log_file: PathBuf,
    pub desktop_lock_file: PathBuf,
}

impl Paths {
    pub fn resolve() -> Result<Self, &'static str> {
        let home = dirs::home_dir().ok_or("resolve user home directory")?;
        Ok(resolve_with(current_os(), &home, |key| {
            std::env::var(key).unwrap_or_default()
        }))
    }

    /// Like [`Self::resolve`] but pins the desktop data directory to one the
    /// caller already resolved, so both path modules agree in-process.
    pub fn resolve_for_desktop_dir(desktop_data_dir: &Path) -> Result<Self, &'static str> {
        let home = dirs::home_dir().ok_or("resolve user home directory")?;
        let pinned = desktop_data_dir.to_string_lossy().into_owned();
        Ok(resolve_with(current_os(), &home, |key| {
            if key == "MTLS_ROUTER_DESKTOP_DATA_DIR" {
                pinned.clone()
            } else {
                std::env::var(key).unwrap_or_default()
            }
        }))
    }

    pub fn agent_state_dir(&self) -> PathBuf {
        self.desktop_data_dir.join("agent-transactions")
    }

    pub fn state_files(&self) -> [PathBuf; 2] {
        [self.desktop_state_file.clone(), self.cli_state_file.clone()]
    }

    pub fn installation_file(&self) -> PathBuf {
        self.desktop_data_dir.join("installation.json")
    }

    /// Durable once-per-generation legacy migration attempt record.
    pub fn legacy_migration_file(&self) -> PathBuf {
        self.desktop_data_dir.join("legacy-migration.json")
    }
}

pub fn resolve_with(os: &str, home: &Path, getenv: impl Fn(&str) -> String) -> Paths {
    let cli_dir =
        nonempty(getenv("MTLS_ROUTER_STATE_DIR")).unwrap_or_else(|| home.join(".mtls-router"));
    let cli_log =
        nonempty(getenv("MTLS_ROUTER_LOG_PATH")).unwrap_or_else(|| cli_dir.join("mtls-router.log"));
    let desktop_dir = nonempty(getenv("MTLS_ROUTER_DESKTOP_DATA_DIR"))
        .unwrap_or_else(|| desktop_data_dir(os, home, &getenv));
    Paths {
        cli_state_file: cli_dir.join("setup-state.json"),
        cli_log_file: cli_log,
        desktop_state_file: desktop_dir.join("desktop-state.json"),
        desktop_log_file: desktop_dir.join("mtls-router.log"),
        desktop_lock_file: desktop_dir.join("desktop-owner.lock"),
        cli_state_dir: cli_dir,
        desktop_data_dir: desktop_dir,
    }
}

fn desktop_data_dir(os: &str, home: &Path, getenv: &impl Fn(&str) -> String) -> PathBuf {
    match os {
        "windows" => nonempty(getenv("APPDATA"))
            .unwrap_or_else(|| home.join("AppData").join("Roaming"))
            .join(DESKTOP_APP_ID),
        "darwin" => home
            .join("Library")
            .join("Application Support")
            .join(DESKTOP_APP_ID),
        _ => nonempty(getenv("XDG_DATA_HOME"))
            .unwrap_or_else(|| home.join(".local").join("share"))
            .join(DESKTOP_APP_ID),
    }
}

fn nonempty(value: String) -> Option<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

fn current_os() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> String {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        move |key| map.get(key).cloned().unwrap_or_default()
    }

    fn assert_defaults(os: &str, getenv: impl Fn(&str) -> String, desktop: PathBuf) {
        let got = resolve_with(os, Path::new("home"), getenv);
        assert_eq!(
            got.cli_state_file,
            Path::new("home")
                .join(".mtls-router")
                .join("setup-state.json"),
            "{os}"
        );
        assert_eq!(got.desktop_data_dir, desktop, "{os}");
        assert_eq!(
            got.desktop_state_file.parent(),
            Some(got.desktop_data_dir.as_path())
        );
        assert_eq!(
            got.desktop_lock_file.parent(),
            Some(got.desktop_data_dir.as_path())
        );
    }

    #[test]
    fn resolve_platform_defaults() {
        assert_defaults(
            "linux",
            env(&[]),
            Path::new("home")
                .join(".local")
                .join("share")
                .join(DESKTOP_APP_ID),
        );
        assert_defaults(
            "linux",
            env(&[("XDG_DATA_HOME", "data")]),
            PathBuf::from("data").join(DESKTOP_APP_ID),
        );
        assert_defaults(
            "darwin",
            env(&[]),
            Path::new("home")
                .join("Library")
                .join("Application Support")
                .join(DESKTOP_APP_ID),
        );
        assert_defaults(
            "windows",
            env(&[("APPDATA", "roaming")]),
            PathBuf::from("roaming").join(DESKTOP_APP_ID),
        );
    }

    #[test]
    fn resolve_overrides_cli_and_desktop_roots() {
        let got = resolve_with(
            "linux",
            Path::new("home"),
            env(&[
                ("MTLS_ROUTER_STATE_DIR", "cli-state"),
                ("MTLS_ROUTER_LOG_PATH", "cli.log"),
                ("MTLS_ROUTER_DESKTOP_DATA_DIR", "desktop-data"),
            ]),
        );
        assert_eq!(got.cli_state_dir, PathBuf::from("cli-state"));
        assert_eq!(got.cli_log_file, PathBuf::from("cli.log"));
        assert_eq!(got.desktop_data_dir, PathBuf::from("desktop-data"));
        assert_eq!(
            got.agent_state_dir(),
            PathBuf::from("desktop-data/agent-transactions")
        );
        assert_eq!(
            got.installation_file(),
            PathBuf::from("desktop-data/installation.json")
        );
        assert_eq!(
            got.legacy_migration_file(),
            PathBuf::from("desktop-data/legacy-migration.json")
        );
    }
}
