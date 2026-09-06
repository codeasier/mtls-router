//! Durable router identity state files.
//!
//! Frozen against Go `internal/manager/state` Read/WriteJSON. This module is
//! not the default sidecar runtime path.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouterState {
    #[serde(default)]
    pub pid: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub listen_addr: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub binary_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub log_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub started_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub process_started_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub process_executable: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub desktop_session_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub installation_id: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub package_generation: i32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub manager_pid: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub manager_process_started_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub manager_process_executable: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub manager_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub router_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub deployment_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub management_protocol_version: String,
}

fn is_zero(value: &i32) -> bool {
    *value == 0
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateError {
    NotFound,
    PermissionDenied,
    Corrupt,
    Io,
}

impl StateError {
    fn from_io(error: &io::Error) -> Self {
        match error.kind() {
            ErrorKind::NotFound => Self::NotFound,
            ErrorKind::PermissionDenied => Self::PermissionDenied,
            _ => Self::Io,
        }
    }
}

/// Decodes one JSON object. Missing and permission errors stay distinguishable
/// from trailing or malformed JSON (`Corrupt`).
pub fn read(path: impl AsRef<Path>) -> Result<RouterState, StateError> {
    let bytes = fs::read(path.as_ref()).map_err(|error| StateError::from_io(&error))?;
    let bytes = strip_bom(&bytes);
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let value = RouterState::deserialize(&mut de).map_err(|_| StateError::Corrupt)?;
    match de.end() {
        Ok(()) => Ok(value),
        Err(_) => Err(StateError::Corrupt),
    }
}

pub fn write(path: impl AsRef<Path>, value: &RouterState) -> Result<(), StateError> {
    let path = path.as_ref();
    let dir = path.parent().ok_or(StateError::Io)?;
    fs::create_dir_all(dir).map_err(|error| StateError::from_io(&error))?;
    let encoded = serde_json::to_vec_pretty(value).map_err(|_| StateError::Io)?;
    let tmp = dir.join(format!(
        ".{}-{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state"),
        uuid::Uuid::new_v4().simple()
    ));
    fs::write(&tmp, encoded).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        StateError::from_io(&error)
    })?;
    if let Err(error) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(StateError::from_io(&error));
    }
    Ok(())
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes)
}

/// True when a readable state file records this positive PID.
pub fn protects_pid(paths: &[impl AsRef<Path>], pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    paths.iter().any(|path| {
        read(path)
            .ok()
            .is_some_and(|value| value.pid > 0 && value.pid == pid)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager_core::process;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "mtls-manager-state-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    #[test]
    fn read_distinguishes_missing_corrupt_and_valid() {
        let path = temp_path("read");
        assert_eq!(read(&path), Err(StateError::NotFound));

        fs::write(&path, b"not-json").unwrap();
        assert_eq!(read(&path), Err(StateError::Corrupt));

        fs::write(&path, b"{\"pid\":4101}{\"pid\":1}").unwrap();
        assert_eq!(read(&path), Err(StateError::Corrupt));

        fs::write(&path, b"\xEF\xBB\xBF{\"pid\":4101}\n").unwrap();
        assert_eq!(read(&path).unwrap().pid, 4101);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn write_then_read_round_trips_identity_fields() {
        let path = temp_path("write");
        let value = RouterState {
            pid: 4101,
            process_started_at: "start".into(),
            process_executable: "/bin/router".into(),
            owner: "desktop".into(),
            ..RouterState::default()
        };
        write(&path, &value).unwrap();
        assert_eq!(read(&path).unwrap(), value);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn protects_pid_skips_read_errors_and_non_positive() {
        let dir = temp_path("protect-dir");
        fs::create_dir_all(&dir).unwrap();
        let desktop = dir.join("desktop-state.json");
        let cli = dir.join("cli-state.json");
        let missing = dir.join("missing-state.json");
        write(
            &desktop,
            &RouterState {
                pid: 4101,
                ..RouterState::default()
            },
        )
        .unwrap();
        fs::write(&cli, b"not-json").unwrap();

        assert!(protects_pid(&[&desktop, &cli], 4101));
        assert!(!protects_pid(&[&cli, &missing], 4201));
        assert!(!protects_pid(&[&desktop], 0));
        assert!(!protects_pid(&[&desktop], 4102));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn same_identity_uses_state_process_triple() {
        let started = "2026-09-06T00:00:00.000000000Z";
        let live = process::Identity {
            pid: 9,
            started_at: started.into(),
            executable: "/bin/router".into(),
        };
        let value = RouterState {
            pid: 9,
            process_started_at: started.into(),
            process_executable: "/bin/router".into(),
            ..RouterState::default()
        };
        let managed = process::Identity {
            pid: value.pid,
            started_at: value.process_started_at,
            executable: value.process_executable,
        };
        assert_eq!(process::same_identity(&live, &managed), Ok(true));
    }
}
