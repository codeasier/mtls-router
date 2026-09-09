//! Persistent loopback listen override for one desktop installation.
//!
//! `19099` remains the factory default. An override is this installation's
//! only listener: the next launch must not try `19099` first. The stored
//! address is exactly `127.0.0.1:<port>` (`1–65535`). LAN binds, `0.0.0.0`,
//! `localhost`, and `[::1]` are rejected so Windows TCP4 occupant inspection
//! stays aligned with the configured listener.

use crate::error::{CommandError, Result};
use crate::manager_core::DEFAULT_LISTEN;
use crate::types::RouterStatus;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub const SCHEMA_VERSION: u32 = 1;
pub const OVERRIDE_FILE: &str = "listen-override.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OverrideRecord {
    schema_version: u32,
    listen_addr: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ListenConfig {
    pub listen_addr: String,
    pub factory_default: String,
    pub overridden: bool,
}

impl ListenConfig {
    fn factory() -> Self {
        Self {
            listen_addr: DEFAULT_LISTEN.to_owned(),
            factory_default: DEFAULT_LISTEN.to_owned(),
            overridden: false,
        }
    }

    fn overridden(listen_addr: String) -> Self {
        Self {
            listen_addr,
            factory_default: DEFAULT_LISTEN.to_owned(),
            overridden: true,
        }
    }
}

pub fn override_path(data_dir: &Path) -> PathBuf {
    data_dir.join(OVERRIDE_FILE)
}

/// Accept only `127.0.0.1:<1-65535>` with no leading zeros on the port.
pub fn parse_exact_loopback_listen(value: &str) -> std::result::Result<String, &'static str> {
    let value = value.trim();
    let Some(port_text) = value.strip_prefix("127.0.0.1:") else {
        return Err("listen override must be 127.0.0.1:<port>");
    };
    if port_text.is_empty()
        || !port_text.bytes().all(|b| b.is_ascii_digit())
        || (port_text.len() > 1 && port_text.starts_with('0'))
    {
        return Err("listen port must be between 1 and 65535");
    }
    let port: u16 = port_text
        .parse()
        .map_err(|_| "listen port must be between 1 and 65535")?;
    if port == 0 {
        return Err("listen port must be between 1 and 65535");
    }
    Ok(format!("127.0.0.1:{port}"))
}

pub fn listen_from_port(port: u16) -> Result<String> {
    if port == 0 {
        return Err(CommandError::invalid_params(
            "listen port must be between 1 and 65535",
        ));
    }
    parse_exact_loopback_listen(&format!("127.0.0.1:{port}")).map_err(CommandError::invalid_params)
}

pub fn load_configured(data_dir: &Path) -> Result<ListenConfig> {
    let path = override_path(data_dir);
    match fs::read(&path) {
        Ok(bytes) => {
            let record: OverrideRecord = serde_json::from_slice(&bytes).map_err(|_| {
                CommandError::new(
                    "LISTEN_OVERRIDE_INVALID",
                    "desktop listen override is corrupt",
                )
            })?;
            if record.schema_version != SCHEMA_VERSION {
                return Err(CommandError::new(
                    "LISTEN_OVERRIDE_INVALID",
                    "desktop listen override is incompatible",
                ));
            }
            let listen_addr = parse_exact_loopback_listen(&record.listen_addr).map_err(|_| {
                CommandError::new(
                    "LISTEN_OVERRIDE_INVALID",
                    "desktop listen override is corrupt",
                )
            })?;
            if listen_addr == DEFAULT_LISTEN {
                return Ok(ListenConfig::factory());
            }
            Ok(ListenConfig::overridden(listen_addr))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ListenConfig::factory()),
        Err(_) => Err(CommandError::new(
            "LISTEN_OVERRIDE_INVALID",
            "desktop listen override is unreadable",
        )),
    }
}

pub fn store_override(data_dir: &Path, listen_addr: &str) -> Result<ListenConfig> {
    let listen_addr =
        parse_exact_loopback_listen(listen_addr).map_err(CommandError::invalid_params)?;
    if listen_addr == DEFAULT_LISTEN {
        return clear_override(data_dir);
    }
    fs::create_dir_all(data_dir).map_err(|_| {
        CommandError::new(
            "LISTEN_OVERRIDE_INVALID",
            "cannot create listen override directory",
        )
    })?;
    write_atomic(
        &override_path(data_dir),
        &OverrideRecord {
            schema_version: SCHEMA_VERSION,
            listen_addr: listen_addr.clone(),
        },
    )?;
    Ok(ListenConfig::overridden(listen_addr))
}

pub fn clear_override(data_dir: &Path) -> Result<ListenConfig> {
    let path = override_path(data_dir);
    match fs::remove_file(&path) {
        Ok(()) => Ok(ListenConfig::factory()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ListenConfig::factory()),
        Err(_) => Err(CommandError::new(
            "LISTEN_OVERRIDE_INVALID",
            "cannot clear listen override",
        )),
    }
}

pub fn is_reserved_listen_failure(status: &RouterStatus) -> bool {
    if status.state != "start_failed" {
        return false;
    }
    std::iter::once(status.last_error.as_deref().unwrap_or(""))
        .chain(status.recent_logs.iter().flatten().map(String::as_str))
        .any(line_has_access_denied)
}

fn line_has_access_denied(line: &str) -> bool {
    line.split_whitespace()
        .any(|token| token == "listen_refusal=access_denied")
}

fn write_atomic(path: &Path, value: &OverrideRecord) -> Result<()> {
    let encoded = serde_json::to_vec_pretty(value).map_err(|_| {
        CommandError::new("LISTEN_OVERRIDE_INVALID", "cannot encode listen override")
    })?;
    let parent = path.parent().ok_or_else(|| {
        CommandError::new("LISTEN_OVERRIDE_INVALID", "cannot persist listen override")
    })?;
    let tmp = parent.join(format!(".listen-override-{}.tmp", Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&tmp)?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        #[cfg(unix)]
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
        fs::rename(&tmp, path)?;
        #[cfg(unix)]
        {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            OpenOptions::new().read(true).open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
        .map_err(|_| CommandError::new("LISTEN_OVERRIDE_INVALID", "cannot persist listen override"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_exact_loopback_listen_accepts_only_numeric_ipv4_loopback() {
        assert_eq!(
            parse_exact_loopback_listen("127.0.0.1:19100").unwrap(),
            "127.0.0.1:19100"
        );
        assert_eq!(
            parse_exact_loopback_listen(" 127.0.0.1:1 ").unwrap(),
            "127.0.0.1:1"
        );
        assert_eq!(
            parse_exact_loopback_listen("127.0.0.1:65535").unwrap(),
            "127.0.0.1:65535"
        );
        for invalid in [
            "",
            "127.0.0.1:0",
            "127.0.0.1:65536",
            "127.0.0.1:019100",
            "127.0.0.2:19100",
            "0.0.0.0:19100",
            "localhost:19100",
            "[::1]:19100",
            "127.0.0.1",
            "19100",
        ] {
            assert!(parse_exact_loopback_listen(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn load_configured_uses_factory_default_when_absent_or_default_record() {
        let dir = tempfile();
        assert_eq!(load_configured(&dir).unwrap(), ListenConfig::factory());
        store_override(&dir, DEFAULT_LISTEN).unwrap();
        assert!(!override_path(&dir).exists());
        assert_eq!(load_configured(&dir).unwrap(), ListenConfig::factory());
    }

    #[test]
    fn store_override_is_sole_listener_and_survives_reload() {
        let dir = tempfile();
        let stored = store_override(&dir, "127.0.0.1:19100").unwrap();
        assert_eq!(stored.listen_addr, "127.0.0.1:19100");
        assert!(stored.overridden);
        assert_eq!(load_configured(&dir).unwrap(), stored);
        assert_eq!(clear_override(&dir).unwrap(), ListenConfig::factory());
        assert!(!override_path(&dir).exists());
    }

    #[test]
    fn load_configured_fails_closed_on_corrupt_or_foreign_bind() {
        let dir = tempfile();
        fs::write(override_path(&dir), b"{not-json").unwrap();
        assert_eq!(
            load_configured(&dir).unwrap_err().code,
            "LISTEN_OVERRIDE_INVALID"
        );

        let dir = tempfile();
        fs::write(
            override_path(&dir),
            r#"{"schema_version":1,"listen_addr":"[::1]:19100"}"#,
        )
        .unwrap();
        assert_eq!(
            load_configured(&dir).unwrap_err().code,
            "LISTEN_OVERRIDE_INVALID"
        );

        let dir = tempfile();
        fs::write(
            override_path(&dir),
            r#"{"schema_version":2,"listen_addr":"127.0.0.1:19100"}"#,
        )
        .unwrap();
        assert_eq!(
            load_configured(&dir).unwrap_err().code,
            "LISTEN_OVERRIDE_INVALID"
        );
    }

    #[test]
    fn reserved_listen_failure_requires_start_failed_and_access_denied() {
        let reserved = RouterStatus {
            state: "start_failed".into(),
            last_error: Some("stage=process_launch code=ROUTER_START_FAILED os_error=10013".into()),
            recent_logs: Some(vec![
                "reason=listen_failed listen_refusal=access_denied".into()
            ]),
            ..RouterStatus::default()
        };
        assert!(is_reserved_listen_failure(&reserved));

        let in_use = RouterStatus {
            state: "start_failed".into(),
            recent_logs: Some(vec![
                "reason=listen_failed listen_refusal=address_in_use".into()
            ]),
            ..RouterStatus::default()
        };
        assert!(!is_reserved_listen_failure(&in_use));

        let occupied = RouterStatus {
            state: "unknown_occupant".into(),
            recent_logs: Some(vec![
                "reason=listen_failed listen_refusal=access_denied".into()
            ]),
            ..RouterStatus::default()
        };
        assert!(!is_reserved_listen_failure(&occupied));

        let probe = RouterStatus {
            state: "start_failed".into(),
            recent_logs: Some(vec!["reason=upstream_probe_failed".into()]),
            ..RouterStatus::default()
        };
        assert!(!is_reserved_listen_failure(&probe));
    }

    fn tempfile() -> PathBuf {
        let path = std::env::temp_dir().join(format!("mtls-listen-override-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
