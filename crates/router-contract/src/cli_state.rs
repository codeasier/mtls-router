use crate::{process, protocol::MANAGEMENT_PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const STATE_FILE: &str = "router-state.json";
pub fn default_directory(home: &Path, xdg_state: Option<&str>) -> PathBuf {
    xdg_state
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"))
        .join("codeasier-router")
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessRecord {
    pub schema_version: u32,
    pub pid: i32,
    pub listen_addr: String,
    pub process_started_at: String,
    pub process_executable: String,
    pub owner: String,
    pub router_version: String,
    pub deployment_id: String,
    pub management_protocol_version: String,
    pub upstream_auth: String,
    pub auth_protocol_version: String,
}
impl ProcessRecord {
    pub fn new(
        identity: process::Identity,
        listen_addr: String,
        version: String,
        deployment_id: String,
    ) -> Self {
        Self {
            schema_version: 1,
            pid: identity.pid,
            listen_addr,
            process_started_at: identity.started_at,
            process_executable: identity.executable,
            owner: "cli".into(),
            router_version: version,
            deployment_id,
            management_protocol_version: MANAGEMENT_PROTOCOL_VERSION.into(),
            upstream_auth: "per_user_mtls".into(),
            auth_protocol_version: "1".into(),
        }
    }
}
