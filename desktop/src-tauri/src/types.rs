use serde::{Deserialize, Serialize};
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

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct OccupantInspection {
    pub pid: u32,
    pub process_name: String,
    pub executable: String,
    pub listen_addr: String,
    pub confirmation_token: String,
    pub expires_at: String,
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
    pub path: String,
    pub role: String,
    pub format: String,
    pub operation: String,
    #[serde(default)]
    pub backup_path: String,
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
}
