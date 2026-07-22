use chrono::DateTime;
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PollError {
    pub code: String,
}

impl PollError {
    pub fn new(code: impl Into<String>) -> Self {
        Self { code: code.into() }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PollSnapshot {
    pub revision: u64,
    pub status: Option<RouterStatus>,
    pub health: Option<RouterHealth>,
    pub health_stale: bool,
    pub status_error: Option<PollError>,
    pub health_error: Option<PollError>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NativeLanguage {
    #[default]
    ZhCn,
    En,
}

impl NativeLanguage {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "zh-CN" => Some(Self::ZhCn),
            "en" => Some(Self::En),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouterStatus {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listen_addr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_logs: Option<Vec<String>>,
}

impl RouterStatus {
    pub fn available(&self) -> bool {
        matches!(
            self.state.as_str(),
            "desktop_owned" | "external_compatible" | "degraded"
        )
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouterHealth {
    pub status: String,
    pub checked_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouterVersion {
    pub version: String,
    pub deployment_id: String,
    pub management_protocol_version: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouterLogs {
    pub lines: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OccupantVerificationMode {
    VerifiedIdentity,
    WindowsPidOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OccupantInspection {
    pub pid: u32,
    pub verification_mode: OccupantVerificationMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    pub listen_addr: String,
    pub confirmation_token: String,
    pub expires_at: String,
}

fn is_manager_rfc3339(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return false;
    }
    for index in [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18] {
        if !bytes[index].is_ascii_digit() {
            return false;
        }
    }
    if bytes[17] > b'5' {
        return false;
    }

    let mut timezone_start = 19;
    if bytes.get(timezone_start) == Some(&b'.') {
        timezone_start += 1;
        let fraction_start = timezone_start;
        while bytes.get(timezone_start).is_some_and(u8::is_ascii_digit) {
            timezone_start += 1;
        }
        if timezone_start == fraction_start {
            return false;
        }
    }

    match &bytes[timezone_start..] {
        [b'Z'] => true,
        [sign @ (b'+' | b'-'), hour_tens, hour_ones, b':', minute_tens, minute_ones] => {
            let _ = sign;
            hour_tens.is_ascii_digit()
                && hour_ones.is_ascii_digit()
                && minute_tens.is_ascii_digit()
                && minute_ones.is_ascii_digit()
        }
        _ => false,
    }
}

impl<'de> Deserialize<'de> for OccupantInspection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireInspection {
            pid: u32,
            verification_mode: OccupantVerificationMode,
            process_name: Option<String>,
            executable: Option<String>,
            listen_addr: String,
            confirmation_token: String,
            expires_at: String,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| D::Error::custom("occupant inspection must be an object"))?;
        let has_process_name = object.contains_key("process_name");
        let has_executable = object.contains_key("executable");
        let wire: WireInspection = serde_json::from_value(value).map_err(D::Error::custom)?;

        if wire.pid == 0 {
            return Err(D::Error::custom("occupant inspection PID must be positive"));
        }
        if wire.listen_addr != "127.0.0.1:19099" {
            return Err(D::Error::custom(
                "occupant inspection listen address must match the fixed endpoint",
            ));
        }
        if wire.confirmation_token.trim().is_empty() {
            return Err(D::Error::custom(
                "occupant inspection confirmation token must not be blank",
            ));
        }
        if !is_manager_rfc3339(&wire.expires_at)
            || DateTime::parse_from_rfc3339(&wire.expires_at).is_err()
        {
            return Err(D::Error::custom(
                "occupant inspection expiry must be RFC3339",
            ));
        }

        match wire.verification_mode {
            OccupantVerificationMode::VerifiedIdentity => {
                if !matches!(wire.process_name.as_deref(), Some(value) if !value.trim().is_empty())
                    || !matches!(wire.executable.as_deref(), Some(value) if !value.trim().is_empty())
                {
                    return Err(D::Error::custom(
                        "verified identity requires process name and executable",
                    ));
                }
            }
            OccupantVerificationMode::WindowsPidOnly => {
                if has_process_name || has_executable {
                    return Err(D::Error::custom(
                        "Windows PID-only inspection cannot include process metadata",
                    ));
                }
            }
        }

        Ok(Self {
            pid: wire.pid,
            verification_mode: wire.verification_mode,
            process_name: wire.process_name,
            executable: wire.executable,
            listen_addr: wire.listen_addr,
            confirmation_token: wire.confirmation_token,
            expires_at: wire.expires_at,
        })
    }
}

#[cfg(test)]
mod occupant_inspection_tests {
    use super::{OccupantInspection, OccupantVerificationMode};
    use serde_json::json;

    #[test]
    fn deserializes_verified_identity_shape() {
        let shape = json!({
            "pid": 4242,
            "verification_mode": "verified_identity",
            "process_name": "example-server",
            "executable": "C:\\example-server.exe",
            "listen_addr": "127.0.0.1:19099",
            "confirmation_token": "verified-token",
            "expires_at": "2026-07-22T12:00:30Z"
        });
        let inspection: OccupantInspection = serde_json::from_value(shape.clone()).unwrap();

        assert_eq!(
            inspection.verification_mode,
            OccupantVerificationMode::VerifiedIdentity
        );
        assert_eq!(inspection.process_name.as_deref(), Some("example-server"));
        assert_eq!(
            inspection.executable.as_deref(),
            Some("C:\\example-server.exe")
        );
        assert_eq!(serde_json::to_value(inspection).unwrap(), shape);
    }

    #[test]
    fn deserializes_windows_pid_only_shape_without_metadata() {
        let shape = json!({
            "pid": 4242,
            "verification_mode": "windows_pid_only",
            "listen_addr": "127.0.0.1:19099",
            "confirmation_token": "pid-only-token",
            "expires_at": "2026-07-22T12:00:30Z"
        });
        let inspection: OccupantInspection = serde_json::from_value(shape.clone()).unwrap();

        assert_eq!(
            inspection.verification_mode,
            OccupantVerificationMode::WindowsPidOnly
        );
        assert_eq!(inspection.process_name, None);
        assert_eq!(inspection.executable, None);
        assert_eq!(serde_json::to_value(inspection).unwrap(), shape);
    }

    #[test]
    fn rejects_unknown_verification_mode() {
        let result = serde_json::from_value::<OccupantInspection>(json!({
            "pid": 4242,
            "verification_mode": "unknown",
            "listen_addr": "127.0.0.1:19099",
            "confirmation_token": "token",
            "expires_at": "2026-07-22T12:00:30Z"
        }));

        assert!(result.is_err());
    }

    #[test]
    fn rejects_unknown_fields() {
        let result = serde_json::from_value::<OccupantInspection>(json!({
            "pid": 4242,
            "verification_mode": "verified_identity",
            "process_name": "example-server",
            "executable": "C:\\example-server.exe",
            "process_owner": "example-user",
            "listen_addr": "127.0.0.1:19099",
            "confirmation_token": "token",
            "expires_at": "2026-07-22T12:00:30Z"
        }));

        assert!(result.is_err());
    }

    #[test]
    fn rejects_invalid_base_fields() {
        for invalid_field in [
            json!({"pid": 0}),
            json!({"listen_addr": "127.0.0.1:19100"}),
            json!({"listen_addr": " "}),
            json!({"confirmation_token": " "}),
            json!({"expires_at": " "}),
            json!({"expires_at": "not-a-timestamp"}),
            json!({"expires_at": "2026-07-22t12:00:30Z"}),
            json!({"expires_at": "2026-07-22T12:00:30z"}),
            json!({"expires_at": "2026-07-22T12:00:60Z"}),
        ] {
            let mut shape = json!({
                "pid": 4242,
                "verification_mode": "windows_pid_only",
                "listen_addr": "127.0.0.1:19099",
                "confirmation_token": "token",
                "expires_at": "2026-07-22T12:00:30+08:00"
            });
            shape.as_object_mut().unwrap().extend(
                invalid_field
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );

            assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
        }
    }

    #[test]
    fn accepts_manager_timestamp_forms() {
        for expires_at in ["2026-07-22T12:00:30Z", "2026-07-22T20:00:30+08:00"] {
            let result = serde_json::from_value::<OccupantInspection>(json!({
                "pid": 4242,
                "verification_mode": "windows_pid_only",
                "listen_addr": "127.0.0.1:19099",
                "confirmation_token": "token",
                "expires_at": expires_at
            }));

            assert!(result.is_ok(), "expected {expires_at} to be accepted");
        }
    }

    #[test]
    fn rejects_verified_identity_without_nonempty_metadata() {
        for metadata in [
            json!({"executable": "C:\\example-server.exe"}),
            json!({"process_name": "example-server"}),
            json!({"process_name": "", "executable": "C:\\example-server.exe"}),
            json!({"process_name": "example-server", "executable": "  "}),
        ] {
            let mut shape = json!({
                "pid": 4242,
                "verification_mode": "verified_identity",
                "listen_addr": "127.0.0.1:19099",
                "confirmation_token": "token",
                "expires_at": "2026-07-22T12:00:30Z"
            });
            shape.as_object_mut().unwrap().extend(
                metadata
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );

            assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
        }
    }

    #[test]
    fn rejects_windows_pid_only_with_process_metadata() {
        for metadata in [
            json!({"process_name": "example-server"}),
            json!({"executable": "C:\\example-server.exe"}),
            json!({"process_name": "", "executable": ""}),
        ] {
            let mut shape = json!({
                "pid": 4242,
                "verification_mode": "windows_pid_only",
                "listen_addr": "127.0.0.1:19099",
                "confirmation_token": "token",
                "expires_at": "2026-07-22T12:00:30Z"
            });
            shape.as_object_mut().unwrap().extend(
                metadata
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );

            assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Diagnostics {
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ManagerInfo {
    pub version: String,
    pub commit: String,
    pub build_date: String,
    pub target: String,
    pub deployment_id: String,
    pub management_protocol_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ComponentVersions {
    pub desktop: String,
    pub manager: String,
    pub router: String,
    pub management_protocol: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentRecoveryFileState {
    pub role: String,
    pub path: String,
    pub format: String,
    pub exists: bool,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentRecoveryState {
    pub eligible: bool,
    #[serde(default)]
    pub reasons: Vec<String>,
    pub files: Vec<AgentRecoveryFileState>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentState {
    pub agent: String,
    pub name: String,
    pub detected: bool,
    #[serde(default)]
    pub command: String,
    pub path: String,
    #[serde(default)]
    pub auth_path: String,
    pub format: String,
    pub exists: bool,
    pub writable: bool,
    pub configured: bool,
    pub invalid: bool,
    #[serde(default)]
    pub migratable: bool,
    pub recovery: AgentRecoveryState,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentDetect {
    pub agents: Vec<AgentState>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentFragment {
    pub agent: String,
    pub role: String,
    pub path: String,
    pub format: String,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentFileEffect {
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub mode: String,
    pub path: String,
    pub role: String,
    pub format: String,
    pub operation: String,
    #[serde(default)]
    pub backup_path: String,
    #[serde(default)]
    pub backup_required: bool,
    #[serde(default)]
    pub backup_pattern: String,
    #[serde(default)]
    pub backup_sensitive: bool,
    #[serde(default)]
    pub preserves: Vec<String>,
    #[serde(default)]
    pub warning: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ManagedCollision {
    pub agent: String,
    pub path: String,
    #[serde(rename = "type")]
    pub collision_type: String,
    pub action: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AgentPreview {
    pub revision_token: String,
    pub model_config: Value,
    pub fragments: Vec<AgentFragment>,
    #[serde(default)]
    pub files: Vec<AgentFileEffect>,
    pub managed_config_drift: bool,
    #[serde(default)]
    pub drifted_agents: Vec<String>,
    #[serde(default)]
    pub managed_collisions: Vec<ManagedCollision>,
    pub requires_codex_auth_approval: bool,
    pub state_change: Option<AgentFileEffect>,
    pub state_backup: Option<AgentFileEffect>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentWriteResult {
    pub transaction_id: String,
    pub agents: Vec<AgentWriteStatus>,
    pub state_change: Option<AgentFileEffect>,
    pub state_backup: Option<AgentFileEffect>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentWriteStatus {
    pub agent: String,
    pub success: bool,
    #[serde(default)]
    pub changed: Vec<String>,
    #[serde(default)]
    pub backups: Vec<String>,
    #[serde(default)]
    pub error_code: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DesktopPaths {
    pub data_dir: String,
    pub log_file: String,
    pub can_prepare_for_uninstall: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_language_accepts_only_supported_values() {
        assert_eq!(NativeLanguage::parse("zh-CN"), Some(NativeLanguage::ZhCn));
        assert_eq!(NativeLanguage::parse("en"), Some(NativeLanguage::En));
        assert_eq!(NativeLanguage::parse("fr"), None);
        assert_eq!(NativeLanguage::parse(""), None);
    }

    #[test]
    fn native_language_defaults_to_chinese() {
        assert_eq!(NativeLanguage::default(), NativeLanguage::ZhCn);
    }

    #[test]
    fn router_status_deserializes_bounded_failure_logs() {
        let status: RouterStatus = serde_json::from_value(serde_json::json!({
            "state": "start_failed",
            "owner": "desktop",
            "last_error": "desktop-owned router exited unexpectedly",
            "recent_logs": ["safe line", "api_key=[REDACTED]"]
        }))
        .unwrap();

        assert_eq!(status.state, "start_failed");
        assert_eq!(
            status.recent_logs,
            Some(vec![
                "safe line".to_owned(),
                "api_key=[REDACTED]".to_owned()
            ])
        );
    }

    #[test]
    fn agent_recovery_and_rebuild_preview_fields_survive_serde() {
        let detection_json = serde_json::json!({
            "agents": [{
                "agent": "claude",
                "name": "Claude Code",
                "detected": true,
                "path": "/home/example/.claude/settings.json",
                "format": "json",
                "exists": true,
                "writable": false,
                "configured": false,
                "invalid": true,
                "migratable": false,
                "recovery": {
                    "eligible": true,
                    "reasons": ["syntax_invalid"],
                    "files": [{
                        "role": "config",
                        "path": "/home/example/.claude/settings.json",
                        "format": "json",
                        "exists": true,
                        "reasons": ["syntax_invalid"]
                    }]
                }
            }]
        });
        let detection: AgentDetect = serde_json::from_value(detection_json).unwrap();
        let detection_value = serde_json::to_value(detection).unwrap();
        assert_eq!(detection_value["agents"][0]["migratable"], false);
        assert_eq!(detection_value["agents"][0]["recovery"]["eligible"], true);
        assert_eq!(
            detection_value["agents"][0]["recovery"]["reasons"],
            serde_json::json!(["syntax_invalid"])
        );
        assert_eq!(
            detection_value["agents"][0]["recovery"]["files"][0],
            serde_json::json!({
                "role": "config",
                "path": "/home/example/.claude/settings.json",
                "format": "json",
                "exists": true,
                "reasons": ["syntax_invalid"]
            })
        );

        let preview_json = serde_json::json!({
            "revision_token": "revision-1",
            "model_config": {"version": 1},
            "fragments": [],
            "files": [{
                "agent": "claude",
                "mode": "rebuild",
                "path": "/home/example/.claude/settings.json",
                "role": "config",
                "format": "json",
                "operation": "replace",
                "backup_required": true,
                "backup_pattern": "/home/example/.claude/settings.json.bak.*",
                "backup_sensitive": true,
                "preserves": ["unrelated files"],
                "warning": "Existing invalid configuration will be rebuilt"
            }],
            "managed_config_drift": false,
            "drifted_agents": [],
            "managed_collisions": [],
            "requires_codex_auth_approval": false,
            "state_change": null,
            "state_backup": null
        });
        let preview: AgentPreview = serde_json::from_value(preview_json).unwrap();
        let preview_value = serde_json::to_value(preview).unwrap();
        assert_eq!(
            preview_value["files"][0],
            serde_json::json!({
                "agent": "claude",
                "mode": "rebuild",
                "path": "/home/example/.claude/settings.json",
                "role": "config",
                "format": "json",
                "operation": "replace",
                "backup_path": "",
                "backup_required": true,
                "backup_pattern": "/home/example/.claude/settings.json.bak.*",
                "backup_sensitive": true,
                "preserves": ["unrelated files"],
                "warning": "Existing invalid configuration will be rebuilt"
            })
        );
    }
}
