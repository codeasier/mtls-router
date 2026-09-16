use crate::redaction::sanitize_text;
use crate::types::{PollSnapshot, RouterHealth, RouterStatus};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
};
use uuid::Uuid;

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_SUMMARY_BYTES: usize = 16 * 1024;
const MAX_LAST_ERROR_BYTES: usize = 512;

const CLOSED_STARTUP_STAGES: &[&str] = &[
    "log_directory",
    "log_open",
    "process_launch",
    "process_inspect",
    "readiness",
    "identity_validate",
    "state_reconcile",
    "state_persist",
    "process_exit",
    "unknown",
];

const CLOSED_LISTEN_REFUSALS: &[&str] = &["access_denied", "address_in_use", "failed"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticSnapshot {
    pub schema_version: u32,
    pub captured_at: String,
    pub classification: String,
    pub desktop: String,
    pub manager: String,
    pub management_protocol: String,
    pub deployment_id: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub router_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen_addr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_checked_at: Option<String>,
    pub health_stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_error_stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manager_stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manager_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_error: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen_refusal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_executable: Option<String>,
}

pub fn classify(snapshot: &PollSnapshot, now: DateTime<Utc>) -> &'static str {
    if let Some(error) = snapshot.status_error.as_ref() {
        return match error.code.as_str() {
            "SIDECAR_MISSING" | "SIDECAR_INVALID" => "sidecar_invalid",
            _ => "control_plane_unread",
        };
    }

    let Some(status) = snapshot.status.as_ref() else {
        return "not_started";
    };

    match status.state.as_str() {
        "start_failed" => "start_failed",
        "stale" => "router_state_stale",
        "unknown_occupant" => "occupied",
        "legacy_managed" => "legacy_managed",
        "starting" => "starting",
        "stopping" => "stopping",
        "absent" => "not_started",
        "degraded" => "degraded",
        "desktop_owned" | "external_compatible" => {
            let Some(health) = snapshot.health.as_ref() else {
                return "health_stale";
            };
            if health_is_stale_at(health, now) {
                return "health_stale";
            }
            if health.status != "ok" {
                return "upstream_unhealthy";
            }
            match status.state.as_str() {
                "desktop_owned" => "healthy",
                "external_compatible" => "external",
                _ => "unknown",
            }
        }
        _ => "unknown",
    }
}

fn health_is_stale_at(health: &RouterHealth, now: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(&health.checked_at)
        .map(|checked_at| {
            now.signed_duration_since(checked_at.with_timezone(&Utc))
                .num_milliseconds()
                > 30_000
        })
        .unwrap_or(true)
}

pub fn from_poll(
    snapshot: &PollSnapshot,
    manager: Option<(&str, &str)>,
    now: DateTime<Utc>,
) -> DiagnosticSnapshot {
    let health_stale = snapshot
        .health
        .as_ref()
        .map(|health| health_is_stale_at(health, now))
        .unwrap_or(true);

    let (status_stage, status_code) = snapshot
        .status
        .as_ref()
        .map(|status| (status.manager_stage.clone(), status.manager_code.clone()))
        .unwrap_or((None, None));
    let latched = snapshot
        .status
        .as_ref()
        .map(parse_latched)
        .unwrap_or_default();
    let ring = (snapshot.status_error.is_some() || snapshot.health_error.is_some())
        .then_some(manager)
        .flatten();
    let manager_stage = status_stage
        .or(latched.stage)
        .or_else(|| ring.map(|(stage, _)| stage.to_owned()));
    let manager_code = status_code
        .or(latched.code)
        .or_else(|| ring.map(|(_, code)| code.to_owned()));

    DiagnosticSnapshot {
        schema_version: SCHEMA_VERSION,
        captured_at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
        classification: classify(snapshot, now).to_owned(),
        desktop: env!("CARGO_PKG_VERSION").to_owned(),
        manager: env!("MTLS_MANAGER_VERSION").to_owned(),
        management_protocol: env!("MTLS_MANAGEMENT_PROTOCOL_VERSION").to_owned(),
        deployment_id: env!("MTLS_DEPLOYMENT_ID").to_owned(),
        target: env!("MTLS_TARGET_TRIPLE").to_owned(),
        router_state: snapshot.status.as_ref().map(|status| status.state.clone()),
        owner: snapshot
            .status
            .as_ref()
            .and_then(|status| status.owner.clone()),
        listen_addr: snapshot
            .status
            .as_ref()
            .and_then(|status| status.listen_addr.clone()),
        pid: snapshot.status.as_ref().and_then(|status| status.pid),
        health_status: snapshot.health.as_ref().map(|health| health.status.clone()),
        health_checked_at: snapshot
            .health
            .as_ref()
            .map(|health| health.checked_at.clone()),
        health_stale,
        status_error_code: snapshot
            .status_error
            .as_ref()
            .map(|error| error.code.clone()),
        status_error_stage: snapshot
            .status_error
            .as_ref()
            .and_then(|error| error.stage.clone()),
        health_error_code: snapshot
            .health_error
            .as_ref()
            .map(|error| error.code.clone()),
        manager_stage,
        manager_code,
        last_error: latched.last_error,
        os_error: latched.os_error,
        listen_refusal: latched.listen_refusal,
        recorded_state: None,
        recorded_pid: None,
        recorded_started_at: None,
        recorded_executable: None,
    }
}

impl DiagnosticSnapshot {
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        push_line(
            &mut lines,
            "schema_version",
            self.schema_version.to_string(),
        );
        push_line(&mut lines, "captured_at", self.captured_at.clone());
        push_line(&mut lines, "classification", self.classification.clone());
        push_line(&mut lines, "desktop", self.desktop.clone());
        push_line(&mut lines, "manager", self.manager.clone());
        push_line(
            &mut lines,
            "management_protocol",
            self.management_protocol.clone(),
        );
        push_line(&mut lines, "deployment_id", self.deployment_id.clone());
        push_line(&mut lines, "target", self.target.clone());
        push_optional_line(&mut lines, "router_state", self.router_state.as_deref());
        push_optional_line(&mut lines, "owner", self.owner.as_deref());
        push_optional_line(&mut lines, "listen_addr", self.listen_addr.as_deref());
        if let Some(pid) = self.pid {
            push_line(&mut lines, "pid", pid.to_string());
        }
        push_optional_line(&mut lines, "health_status", self.health_status.as_deref());
        push_optional_line(
            &mut lines,
            "health_checked_at",
            self.health_checked_at.as_deref(),
        );
        push_line(&mut lines, "health_stale", self.health_stale.to_string());
        push_optional_line(
            &mut lines,
            "status_error_code",
            self.status_error_code.as_deref(),
        );
        push_optional_line(
            &mut lines,
            "status_error_stage",
            self.status_error_stage.as_deref(),
        );
        push_optional_line(
            &mut lines,
            "health_error_code",
            self.health_error_code.as_deref(),
        );
        push_optional_line(&mut lines, "manager_stage", self.manager_stage.as_deref());
        push_optional_line(&mut lines, "manager_code", self.manager_code.as_deref());
        push_optional_line(&mut lines, "last_error", self.last_error.as_deref());
        if let Some(os_error) = self.os_error {
            push_line(&mut lines, "os_error", os_error.to_string());
        }
        push_optional_line(&mut lines, "listen_refusal", self.listen_refusal.as_deref());
        push_optional_line(&mut lines, "recorded_state", self.recorded_state.as_deref());
        if let Some(pid) = self.recorded_pid {
            push_line(&mut lines, "recorded_pid", pid.to_string());
        }
        push_optional_line(
            &mut lines,
            "recorded_started_at",
            self.recorded_started_at.as_deref(),
        );
        push_optional_line(
            &mut lines,
            "recorded_executable",
            self.recorded_executable.as_deref(),
        );

        let mut summary = lines.join("\n");
        if summary.len() > MAX_SUMMARY_BYTES {
            let mut end = MAX_SUMMARY_BYTES;
            while end > 0 && !summary.is_char_boundary(end) {
                end -= 1;
            }
            summary.truncate(end);
            summary.push_str("[truncated]");
        }
        summary
    }

    pub fn write_atomic(&self, path: &Path) -> std::io::Result<()> {
        let encoded = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let parent = path.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "diagnostic snapshot path has no parent",
            )
        })?;
        fs::create_dir_all(parent)?;
        let tmp = parent.join(format!(".last-diagnostics-{}.tmp", Uuid::new_v4()));
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
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    pub fn load(path: &Path) -> Option<Self> {
        let bytes = fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn attach_recorded_state(&mut self, path: &Path) {
        match read_recorded_identity(path) {
            Err(RecordedStateRead::Absent) => {}
            Err(kind) => {
                self.recorded_state = Some(kind.as_str().to_owned());
                self.recorded_pid = None;
                self.recorded_started_at = None;
                self.recorded_executable = None;
            }
            Ok(identity) => {
                self.recorded_state = Some("readable".to_owned());
                if identity.pid > 0 {
                    self.recorded_pid = u32::try_from(identity.pid).ok();
                }
                if !identity.process_started_at.is_empty() {
                    self.recorded_started_at = Some(sanitize_text(&identity.process_started_at));
                }
                if let Some(name) = executable_basename(&identity.process_executable) {
                    self.recorded_executable = Some(name);
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
struct LatchedDiagnostic {
    last_error: Option<String>,
    stage: Option<String>,
    code: Option<String>,
    os_error: Option<u64>,
    listen_refusal: Option<String>,
}

fn parse_latched(status: &RouterStatus) -> LatchedDiagnostic {
    let mut text = String::new();
    if let Some(error) = status.last_error.as_deref() {
        text.push_str(error);
        text.push(' ');
    }
    if let Some(logs) = status.recent_logs.as_ref() {
        for line in logs {
            text.push_str(line);
            text.push(' ');
        }
    }
    let last_error = status
        .last_error
        .as_deref()
        .map(sanitize_text)
        .map(|value| bound_last_error(&value))
        .filter(|value| !value.is_empty());
    LatchedDiagnostic {
        last_error,
        stage: token_value(&text, "stage")
            .filter(|value| CLOSED_STARTUP_STAGES.contains(value))
            .map(str::to_owned),
        code: token_value(&text, "code")
            .filter(|value| closed_error_code(value))
            .map(str::to_owned),
        os_error: token_value(&text, "os_error")
            .and_then(|value| value.parse().ok())
            .filter(|value| *value != 0),
        listen_refusal: token_value(&text, "listen_refusal")
            .filter(|value| CLOSED_LISTEN_REFUSALS.contains(value))
            .map(str::to_owned),
    }
}

fn token_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.split_whitespace().find_map(|token| {
        token
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix('='))
            .filter(|value| !value.is_empty())
    })
}

fn closed_error_code(code: &str) -> bool {
    code.len() <= 64
        && code
            .bytes()
            .all(|value| value.is_ascii_uppercase() || value.is_ascii_digit() || value == b'_')
}

fn bound_last_error(value: &str) -> String {
    if value.len() <= MAX_LAST_ERROR_BYTES {
        return value.to_owned();
    }
    let mut end = MAX_LAST_ERROR_BYTES;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = value[..end].to_owned();
    truncated.push_str("[truncated]");
    truncated
}

#[derive(Deserialize)]
struct RecordedIdentity {
    #[serde(default)]
    pid: i32,
    #[serde(default)]
    process_started_at: String,
    #[serde(default)]
    process_executable: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordedStateRead {
    Absent,
    Corrupt,
    PermissionDenied,
    Unreadable,
}

impl RecordedStateRead {
    fn as_str(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Corrupt => "corrupt",
            Self::PermissionDenied => "permission_denied",
            Self::Unreadable => "unreadable",
        }
    }
}

fn read_recorded_identity(path: &Path) -> Result<RecordedIdentity, RecordedStateRead> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(RecordedStateRead::Absent)
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            return Err(RecordedStateRead::PermissionDenied)
        }
        Err(_) => return Err(RecordedStateRead::Unreadable),
    };
    let bytes = bytes
        .strip_prefix(&[0xef, 0xbb, 0xbf])
        .unwrap_or(bytes.as_slice());
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let value = RecordedIdentity::deserialize(&mut de).map_err(|_| RecordedStateRead::Corrupt)?;
    de.end().map_err(|_| RecordedStateRead::Corrupt)?;
    Ok(value)
}

fn executable_basename(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path).trim();
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    Some(sanitize_text(name))
}

fn push_line(lines: &mut Vec<String>, key: &str, value: String) {
    lines.push(format!("{key}={value}"));
}

fn push_optional_line(lines: &mut Vec<String>, key: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        push_line(lines, key, value.to_owned());
    }
}

#[derive(Clone)]
pub struct DiagnosticStore {
    inner: Arc<Mutex<Option<DiagnosticSnapshot>>>,
    path: PathBuf,
    recorded_state_path: Option<PathBuf>,
    persist: Sender<DiagnosticSnapshot>,
}

impl DiagnosticStore {
    pub fn new(path: PathBuf) -> Self {
        Self::with_recorded_state(path, None)
    }

    pub fn with_recorded_state(path: PathBuf, recorded_state_path: Option<PathBuf>) -> Self {
        let (persist, persist_rx) = mpsc::channel();
        let writer_path = path.clone();
        let _ = thread::Builder::new()
            .name("mtls-diagnostics-persist".into())
            .spawn(move || persist_snapshots(persist_rx, writer_path));
        Self {
            inner: Arc::new(Mutex::new(None)),
            path,
            recorded_state_path,
            persist,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn capture_and_persist(&self, snapshot: &PollSnapshot, manager: Option<(&str, &str)>) {
        let mut diagnostic = from_poll(snapshot, manager, Utc::now());
        if let Some(path) = &self.recorded_state_path {
            diagnostic.attach_recorded_state(path);
        }
        if let Ok(mut current) = self.inner.lock() {
            *current = Some(diagnostic.clone());
        }
        let _ = self.persist.send(diagnostic);
    }

    pub fn current(&self) -> DiagnosticSnapshot {
        if let Ok(current) = self.inner.lock() {
            if let Some(snapshot) = current.clone() {
                return snapshot;
            }
        }
        DiagnosticSnapshot::load(&self.path)
            .unwrap_or_else(|| from_poll(&PollSnapshot::default(), None, Utc::now()))
    }
}

fn persist_snapshots(rx: Receiver<DiagnosticSnapshot>, path: PathBuf) {
    while let Ok(first) = rx.recv() {
        let mut latest = first;
        while let Ok(next) = rx.try_recv() {
            latest = next;
        }
        let _ = latest.write_atomic(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PollError, RouterStatus};
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 3, 10, 0, 0).unwrap()
    }

    fn poll_snapshot(status: Option<RouterStatus>) -> PollSnapshot {
        PollSnapshot {
            revision: 1,
            status,
            ..PollSnapshot::default()
        }
    }

    fn stale_health() -> RouterHealth {
        RouterHealth {
            status: "degraded".into(),
            checked_at: "2026-09-03T09:59:00Z".into(),
        }
    }

    fn fresh_unhealthy() -> RouterHealth {
        RouterHealth {
            status: "degraded".into(),
            checked_at: "2026-09-03T09:59:45Z".into(),
        }
    }

    fn fresh_ok() -> RouterHealth {
        RouterHealth {
            status: "ok".into(),
            checked_at: "2026-09-03T09:59:45Z".into(),
        }
    }

    #[test]
    fn classification_status_error_wins_over_stale_health() {
        let snapshot = PollSnapshot {
            status: Some(RouterStatus {
                state: "degraded".into(),
                ..RouterStatus::default()
            }),
            health: Some(stale_health()),
            status_error: Some(PollError::new("OPERATION_TIMEOUT")),
            ..PollSnapshot::default()
        };
        assert_eq!(classify(&snapshot, now()), "control_plane_unread");
    }

    #[test]
    fn classification_sidecar_invalid_beats_other_status_error() {
        let snapshot = PollSnapshot {
            status_error: Some(PollError::new("SIDECAR_INVALID")),
            ..PollSnapshot::default()
        };
        assert_eq!(classify(&snapshot, now()), "sidecar_invalid");
    }

    #[test]
    fn classification_sidecar_missing() {
        let snapshot = PollSnapshot {
            status_error: Some(PollError::new("SIDECAR_MISSING")),
            ..PollSnapshot::default()
        };
        assert_eq!(classify(&snapshot, now()), "sidecar_invalid");
    }

    #[test]
    fn classification_fresh_unhealthy_without_status_error() {
        let snapshot = PollSnapshot {
            status: Some(RouterStatus {
                state: "desktop_owned".into(),
                ..RouterStatus::default()
            }),
            health: Some(fresh_unhealthy()),
            ..PollSnapshot::default()
        };
        assert_eq!(classify(&snapshot, now()), "upstream_unhealthy");
    }

    #[test]
    fn classification_start_failed_without_status_error() {
        let snapshot = poll_snapshot(Some(RouterStatus {
            state: "start_failed".into(),
            ..RouterStatus::default()
        }));
        assert_eq!(classify(&snapshot, now()), "start_failed");
    }

    #[test]
    fn classification_router_state_stale() {
        let snapshot = poll_snapshot(Some(RouterStatus {
            state: "stale".into(),
            ..RouterStatus::default()
        }));
        assert_eq!(classify(&snapshot, now()), "router_state_stale");
    }

    #[test]
    fn classification_occupied() {
        let snapshot = poll_snapshot(Some(RouterStatus {
            state: "unknown_occupant".into(),
            ..RouterStatus::default()
        }));
        assert_eq!(classify(&snapshot, now()), "occupied");
    }

    #[test]
    fn classification_legacy_managed() {
        let snapshot = poll_snapshot(Some(RouterStatus {
            state: "legacy_managed".into(),
            ..RouterStatus::default()
        }));
        assert_eq!(classify(&snapshot, now()), "legacy_managed");
    }

    #[test]
    fn classification_starting_and_stopping() {
        let starting = poll_snapshot(Some(RouterStatus {
            state: "starting".into(),
            ..RouterStatus::default()
        }));
        let stopping = poll_snapshot(Some(RouterStatus {
            state: "stopping".into(),
            ..RouterStatus::default()
        }));
        assert_eq!(classify(&starting, now()), "starting");
        assert_eq!(classify(&stopping, now()), "stopping");
    }

    #[test]
    fn classification_degraded_is_closed_regardless_of_health() {
        let health_cases = [
            None,
            Some(stale_health()),
            Some(fresh_unhealthy()),
            Some(fresh_ok()),
        ];
        for health in health_cases {
            let snapshot = PollSnapshot {
                status: Some(RouterStatus {
                    state: "degraded".into(),
                    owner: Some("desktop".into()),
                    ..RouterStatus::default()
                }),
                health,
                ..PollSnapshot::default()
            };
            assert_eq!(
                classify(&snapshot, now()),
                "degraded",
                "health={:?}",
                snapshot.health.as_ref().map(|health| &health.status)
            );
        }
    }

    #[test]
    fn classification_desktop_owned_fresh_ok() {
        let snapshot = PollSnapshot {
            status: Some(RouterStatus {
                state: "desktop_owned".into(),
                ..RouterStatus::default()
            }),
            health: Some(fresh_ok()),
            ..PollSnapshot::default()
        };
        assert_eq!(classify(&snapshot, now()), "healthy");
    }

    #[test]
    fn classification_external_compatible_fresh_ok() {
        let snapshot = PollSnapshot {
            status: Some(RouterStatus {
                state: "external_compatible".into(),
                ..RouterStatus::default()
            }),
            health: Some(fresh_ok()),
            ..PollSnapshot::default()
        };
        assert_eq!(classify(&snapshot, now()), "external");
    }

    #[test]
    fn classification_health_stale_when_missing_or_invalid_checked_at() {
        let missing = PollSnapshot {
            status: Some(RouterStatus {
                state: "desktop_owned".into(),
                ..RouterStatus::default()
            }),
            ..PollSnapshot::default()
        };
        let invalid = PollSnapshot {
            status: Some(RouterStatus {
                state: "desktop_owned".into(),
                ..RouterStatus::default()
            }),
            health: Some(RouterHealth {
                status: "ok".into(),
                checked_at: "not-a-timestamp".into(),
            }),
            ..PollSnapshot::default()
        };
        let stale = PollSnapshot {
            status: Some(RouterStatus {
                state: "desktop_owned".into(),
                ..RouterStatus::default()
            }),
            health: Some(stale_health()),
            ..PollSnapshot::default()
        };
        assert_eq!(classify(&missing, now()), "health_stale");
        assert_eq!(classify(&invalid, now()), "health_stale");
        assert_eq!(classify(&stale, now()), "health_stale");
    }

    #[test]
    fn classification_not_started_for_absent_or_missing_status() {
        let absent = poll_snapshot(Some(RouterStatus {
            state: "absent".into(),
            ..RouterStatus::default()
        }));
        let missing = PollSnapshot::default();
        assert_eq!(classify(&absent, now()), "not_started");
        assert_eq!(classify(&missing, now()), "not_started");
    }

    #[test]
    fn write_overwrites_and_load_round_trips() {
        let directory =
            std::env::temp_dir().join(format!("mtls-router-diagnostic-write-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("last-diagnostics.json");

        let first = from_poll(
            &PollSnapshot {
                status_error: Some(PollError::new("MANAGER_FAILED")),
                ..PollSnapshot::default()
            },
            None,
            now(),
        );
        first.write_atomic(&path).unwrap();

        let second = from_poll(
            &PollSnapshot {
                status: Some(RouterStatus {
                    state: "desktop_owned".into(),
                    ..RouterStatus::default()
                }),
                health: Some(fresh_ok()),
                ..PollSnapshot::default()
            },
            None,
            now(),
        );
        second.write_atomic(&path).unwrap();

        let loaded = DiagnosticSnapshot::load(&path).unwrap();
        assert_eq!(loaded.classification, "healthy");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn load_treats_garbage_as_missing() {
        let directory =
            std::env::temp_dir().join(format!("mtls-router-diagnostic-garbage-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("last-diagnostics.json");
        std::fs::write(&path, b"{not-json").unwrap();
        assert!(DiagnosticSnapshot::load(&path).is_none());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn persist_io_error_keeps_memory_snapshot() {
        let path = PathBuf::from("/no-such-parent-dir-for-mtls-diagnostics/last-diagnostics.json");
        let store = DiagnosticStore::new(path);
        let poll = PollSnapshot {
            status_error: Some(PollError::new("MANAGER_FAILED")),
            ..PollSnapshot::default()
        };
        store.capture_and_persist(&poll, None);
        assert_eq!(store.current().classification, "control_plane_unread");
    }

    #[test]
    fn capture_and_persist_overwrites_current_classification() {
        let directory = std::env::temp_dir().join(format!(
            "mtls-router-diagnostic-overwrite-{}",
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let store = DiagnosticStore::new(directory.join("last-diagnostics.json"));

        store.capture_and_persist(
            &PollSnapshot {
                status_error: Some(PollError::new("MANAGER_FAILED")),
                ..PollSnapshot::default()
            },
            None,
        );
        assert_eq!(store.current().classification, "control_plane_unread");

        store.capture_and_persist(
            &PollSnapshot {
                status: Some(RouterStatus {
                    state: "desktop_owned".into(),
                    ..RouterStatus::default()
                }),
                health: Some(RouterHealth {
                    status: "ok".into(),
                    checked_at: Utc::now().to_rfc3339(),
                }),
                ..PollSnapshot::default()
            },
            None,
        );
        assert_eq!(store.current().classification, "healthy");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn from_poll_does_not_need_manager_process() {
        let snap = from_poll(&PollSnapshot::default(), None, now());
        assert!(!snap.desktop.is_empty());
        assert!(!snap.manager.is_empty());
        assert!(!snap.management_protocol.is_empty());
        assert!(!snap.target.is_empty());
        let encoded = serde_json::to_value(&snap).unwrap();
        assert!(encoded.get("router").is_none());
        assert!(!snap.summary().contains("router="));
    }

    #[test]
    fn load_drops_legacy_router_version_field() {
        let snap = from_poll(&PollSnapshot::default(), None, now());
        let mut encoded = serde_json::to_value(&snap).unwrap();
        encoded
            .as_object_mut()
            .unwrap()
            .insert("router".into(), serde_json::json!("v1"));
        let loaded: DiagnosticSnapshot = serde_json::from_value(encoded).unwrap();
        let rewritten = serde_json::to_value(&loaded).unwrap();
        assert!(rewritten.get("router").is_none());
        assert!(!loaded.summary().contains("router="));
    }

    #[test]
    fn from_poll_prefers_status_manager_fields_over_fallback() {
        let snapshot = PollSnapshot {
            status: Some(RouterStatus {
                state: "starting".into(),
                manager_stage: Some("status-stage".into()),
                manager_code: Some("status-code".into()),
                ..RouterStatus::default()
            }),
            ..PollSnapshot::default()
        };
        let diagnostic = from_poll(&snapshot, Some(("fallback-stage", "fallback-code")), now());
        assert_eq!(diagnostic.manager_stage.as_deref(), Some("status-stage"));
        assert_eq!(diagnostic.manager_code.as_deref(), Some("status-code"));
    }

    #[test]
    fn from_poll_ignores_manager_fallback_on_recovered_poll() {
        let diagnostic = from_poll(
            &PollSnapshot {
                status: Some(RouterStatus {
                    state: "desktop_owned".into(),
                    ..RouterStatus::default()
                }),
                health: Some(fresh_ok()),
                ..PollSnapshot::default()
            },
            Some(("watchdog_timeout", "OPERATION_TIMEOUT")),
            now(),
        );
        assert_eq!(diagnostic.classification, "healthy");
        assert_eq!(diagnostic.manager_stage, None);
        assert_eq!(diagnostic.manager_code, None);
        assert!(!diagnostic.summary().contains("manager_stage="));
        assert!(!diagnostic.summary().contains("manager_code="));
    }

    #[test]
    fn from_poll_uses_manager_fallback_when_status_error_present() {
        let diagnostic = from_poll(
            &PollSnapshot {
                status_error: Some(
                    PollError::new("OPERATION_TIMEOUT").with_stage("watchdog_timeout"),
                ),
                ..PollSnapshot::default()
            },
            Some(("watchdog_timeout", "OPERATION_TIMEOUT")),
            now(),
        );
        assert_eq!(diagnostic.classification, "control_plane_unread");
        assert_eq!(
            diagnostic.manager_stage.as_deref(),
            Some("watchdog_timeout")
        );
        assert_eq!(
            diagnostic.manager_code.as_deref(),
            Some("OPERATION_TIMEOUT")
        );
    }

    #[test]
    fn from_poll_uses_manager_fallback_when_health_error_present() {
        let diagnostic = from_poll(
            &PollSnapshot {
                status: Some(RouterStatus {
                    state: "desktop_owned".into(),
                    ..RouterStatus::default()
                }),
                health: Some(fresh_ok()),
                health_error: Some(PollError::new("OPERATION_TIMEOUT")),
                ..PollSnapshot::default()
            },
            Some(("watchdog_timeout", "OPERATION_TIMEOUT")),
            now(),
        );
        assert_eq!(diagnostic.classification, "healthy");
        assert_eq!(
            diagnostic.manager_stage.as_deref(),
            Some("watchdog_timeout")
        );
        assert_eq!(
            diagnostic.manager_code.as_deref(),
            Some("OPERATION_TIMEOUT")
        );
    }

    #[test]
    fn from_poll_does_not_use_manager_fallback_without_poll_error() {
        let diagnostic = from_poll(
            &PollSnapshot::default(),
            Some(("boot", "SIDECAR_INVALID")),
            now(),
        );
        assert_eq!(diagnostic.manager_stage, None);
        assert_eq!(diagnostic.manager_code, None);
    }

    #[test]
    fn from_poll_reads_latched_start_failed_last_error() {
        let diagnostic = from_poll(
            &poll_snapshot(Some(RouterStatus {
                state: "start_failed".into(),
                owner: Some("desktop".into()),
                last_error: Some("stage=state_reconcile code=ROUTER_STATE_STALE os_error=5".into()),
                recent_logs: Some(vec![
                    "reason=listen_failed listen_refusal=access_denied".into()
                ]),
                ..RouterStatus::default()
            })),
            Some(("watchdog_timeout", "OPERATION_TIMEOUT")),
            now(),
        );
        assert_eq!(diagnostic.classification, "start_failed");
        assert_eq!(diagnostic.manager_stage.as_deref(), Some("state_reconcile"));
        assert_eq!(
            diagnostic.manager_code.as_deref(),
            Some("ROUTER_STATE_STALE")
        );
        assert_eq!(
            diagnostic.last_error.as_deref(),
            Some("stage=state_reconcile code=ROUTER_STATE_STALE os_error=5")
        );
        assert_eq!(diagnostic.os_error, Some(5));
        assert_eq!(diagnostic.listen_refusal.as_deref(), Some("access_denied"));
        assert!(diagnostic
            .summary()
            .contains("manager_stage=state_reconcile"));
        assert!(diagnostic
            .summary()
            .contains("manager_code=ROUTER_STATE_STALE"));
        assert!(diagnostic
            .summary()
            .contains("last_error=stage=state_reconcile"));
        assert!(!diagnostic.summary().contains("watchdog_timeout"));
    }

    #[test]
    fn from_poll_redacts_last_error_and_ignores_unknown_tokens() {
        let diagnostic = from_poll(
            &poll_snapshot(Some(RouterStatus {
                state: "start_failed".into(),
                last_error: Some(
                    "stage=not_a_stage code=bad-code os_error=0 api_key=sk-secret-token-12345678"
                        .into(),
                ),
                recent_logs: Some(vec!["listen_refusal=other".into()]),
                ..RouterStatus::default()
            })),
            None,
            now(),
        );
        assert_eq!(diagnostic.manager_stage, None);
        assert_eq!(diagnostic.manager_code, None);
        assert_eq!(diagnostic.os_error, None);
        assert_eq!(diagnostic.listen_refusal, None);
        let last_error = diagnostic.last_error.as_deref().expect("last_error");
        assert!(!last_error.contains("sk-secret"));
        assert!(last_error.contains("[REDACTED"));
        assert!(!diagnostic.summary().contains("sk-secret"));
    }

    #[test]
    fn attach_recorded_state_keeps_basename_and_closed_corrupt_result() {
        let directory =
            std::env::temp_dir().join(format!("mtls-router-diagnostic-state-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let readable = directory.join("desktop-state.json");
        std::fs::write(
            &readable,
            br#"{"pid":42018,"process_started_at":"2026-09-14T08:59:00.000000000Z","process_executable":"C:\\Users\\fixture\\CodeasierRouter.exe","listen_addr":"http://127.0.0.1:19099"}"#,
        )
        .unwrap();
        let mut snapshot = from_poll(
            &poll_snapshot(Some(RouterStatus {
                state: "start_failed".into(),
                last_error: Some("stage=state_reconcile code=ROUTER_STATE_STALE".into()),
                ..RouterStatus::default()
            })),
            None,
            now(),
        );
        snapshot.attach_recorded_state(&readable);
        assert_eq!(snapshot.recorded_state.as_deref(), Some("readable"));
        assert_eq!(snapshot.recorded_pid, Some(42018));
        assert_eq!(
            snapshot.recorded_started_at.as_deref(),
            Some("2026-09-14T08:59:00.000000000Z")
        );
        assert_eq!(
            snapshot.recorded_executable.as_deref(),
            Some("CodeasierRouter.exe")
        );
        assert!(!snapshot.summary().contains("Users"));
        assert!(!snapshot.summary().contains("127.0.0.1"));

        let corrupt = directory.join("corrupt-state.json");
        std::fs::write(&corrupt, b"{not-json sk-secret-token-12345678").unwrap();
        snapshot.attach_recorded_state(&corrupt);
        assert_eq!(snapshot.recorded_state.as_deref(), Some("corrupt"));
        assert_eq!(snapshot.recorded_pid, None);
        assert_eq!(snapshot.recorded_started_at, None);
        assert_eq!(snapshot.recorded_executable, None);
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains("sk-secret"));
        assert!(!encoded.contains("{not-json"));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn capture_and_persist_attaches_recorded_state_identity() {
        let directory = std::env::temp_dir().join(format!(
            "mtls-router-diagnostic-capture-state-{}",
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let state_path = directory.join("desktop-state.json");
        std::fs::write(
            &state_path,
            br#"{"pid":77,"process_started_at":"2026-09-14T08:59:00.000000000Z","process_executable":"/Applications/CodeasierRouter.app/Contents/MacOS/mtls-router-desktop"}"#,
        )
        .unwrap();
        let store = DiagnosticStore::with_recorded_state(
            directory.join("last-diagnostics.json"),
            Some(state_path),
        );
        store.capture_and_persist(
            &poll_snapshot(Some(RouterStatus {
                state: "start_failed".into(),
                last_error: Some("stage=state_reconcile code=ROUTER_ALREADY_RUNNING".into()),
                ..RouterStatus::default()
            })),
            None,
        );
        let current = store.current();
        assert_eq!(current.manager_stage.as_deref(), Some("state_reconcile"));
        assert_eq!(
            current.manager_code.as_deref(),
            Some("ROUTER_ALREADY_RUNNING")
        );
        assert_eq!(current.recorded_pid, Some(77));
        assert_eq!(
            current.recorded_executable.as_deref(),
            Some("mtls-router-desktop")
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn summary_omits_empty_fields_and_truncates() {
        let mut snapshot = from_poll(&PollSnapshot::default(), None, now());
        let summary = snapshot.summary();
        assert!(!summary.contains("router="));
        assert!(summary.starts_with("schema_version="));

        snapshot.classification = "x".repeat(MAX_SUMMARY_BYTES);
        let truncated = snapshot.summary();
        assert!(truncated.ends_with("[truncated]"));
        assert!(truncated.len() <= MAX_SUMMARY_BYTES + "[truncated]".len());
    }

    #[test]
    fn capture_and_persist_updates_memory_then_flushes_disk() {
        let directory =
            std::env::temp_dir().join(format!("mtls-router-diagnostic-async-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("last-diagnostics.json");
        let store = DiagnosticStore::new(path.clone());
        store.capture_and_persist(
            &PollSnapshot {
                status_error: Some(PollError::new("MANAGER_FAILED")),
                ..PollSnapshot::default()
            },
            None,
        );
        assert_eq!(store.current().classification, "control_plane_unread");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if DiagnosticSnapshot::load(&path)
                .is_some_and(|loaded| loaded.classification == "control_plane_unread")
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "background persist did not flush last-diagnostics.json"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn current_summary_includes_classification() {
        let store = DiagnosticStore::new(std::env::temp_dir().join(format!(
            "mtls-router-diagnostic-current-summary-{}",
            Uuid::new_v4()
        )));
        assert!(store.current().summary().contains("classification="));
    }
}
