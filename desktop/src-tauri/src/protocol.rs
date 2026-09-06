#![allow(dead_code)]

//! Rust representation of the manager protocol v4.
//!
//! This module is deliberately independent from the current sidecar transport.
//! It freezes the wire contract before the manager implementation moves into
//! the Tauri process.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MANAGEMENT_PROTOCOL_VERSION: &str = "4";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    #[serde(rename = "manager.info")]
    ManagerInfo,
    #[serde(rename = "diagnostics.collect")]
    DiagnosticsCollect,
    #[serde(rename = "router.status")]
    RouterStatus,
    #[serde(rename = "router.start")]
    RouterStart,
    #[serde(rename = "router.migrate_legacy")]
    RouterMigrateLegacy,
    #[serde(rename = "router.stop")]
    RouterStop,
    #[serde(rename = "router.health")]
    RouterHealth,
    #[serde(rename = "router.version")]
    RouterVersion,
    #[serde(rename = "router.logs")]
    RouterLogs,
    #[serde(rename = "router.inspect_occupant")]
    RouterInspectOccupant,
    #[serde(rename = "router.force_terminate_occupant")]
    RouterForceTerminateOccupant,
    #[serde(rename = "agent.detect")]
    AgentDetect,
    #[serde(rename = "agent.models")]
    AgentModels,
    #[serde(rename = "agent.render")]
    AgentRender,
    #[serde(rename = "agent.preview")]
    AgentPreview,
    #[serde(rename = "agent.write")]
    AgentWrite,
    #[serde(rename = "agent.cleanup.preview")]
    AgentCleanupPreview,
    #[serde(rename = "agent.cleanup.write")]
    AgentCleanupWrite,
    #[serde(rename = "apikey.usage")]
    ApiKeyUsage,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidRequest,
    UnknownMethod,
    InvalidParams,
    SidecarMissing,
    SidecarInvalid,
    RouterNotFound,
    RouterAlreadyRunning,
    RouterStartFailed,
    RouterNotReady,
    RouterDegraded,
    RouterNotOwned,
    RouterStateStale,
    RouterLegacyManaged,
    PortOccupied,
    AgentNotFound,
    ConfigInvalid,
    ConfigNotWritable,
    PreviewStale,
    BackupFailed,
    WriteFailed,
    RollbackFailed,
    OperationTimeout,
    OccupantNotFound,
    OccupantNotOwned,
    OccupantIdentityUnavailable,
    OccupantChanged,
    OccupantProtected,
    OccupantPermissionDenied,
    OccupantTerminationFailed,
    PortReleaseTimeout,
    ConfirmationExpired,
    ModelAuthFailed,
    ModelDiscoveryFailed,
    ModelResponseInvalid,
    ModelCatalogEmpty,
    ModelCatalogStale,
    ModelConfigInvalid,
    ModelNotAvailable,
    ManagedConfigDrift,
    ModelStateInvalid,
    AgentOperationBusy,
    CodexAuthUnsupported,
    AgentNotManaged,
    UsageAuthFailed,
    UsageUnavailable,
    UsageRequestFailed,
    UsageResponseInvalid,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub id: String,
    pub method: Method,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Response {
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<ErrorDetails>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ErrorDetails {
    pub path: String,
    pub rule: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouterStartParams {
    pub owner: RouterOwner,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RouterOwner {
    Desktop,
    Cli,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ForceTerminateOccupantParams {
    pub confirmation_token: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn freezes_protocol_v4_method_names_and_error_codes() {
        let methods = [
            (Method::ManagerInfo, "manager.info"),
            (Method::DiagnosticsCollect, "diagnostics.collect"),
            (Method::RouterStatus, "router.status"),
            (Method::RouterStart, "router.start"),
            (Method::RouterMigrateLegacy, "router.migrate_legacy"),
            (Method::RouterStop, "router.stop"),
            (Method::RouterHealth, "router.health"),
            (Method::RouterVersion, "router.version"),
            (Method::RouterLogs, "router.logs"),
            (Method::RouterInspectOccupant, "router.inspect_occupant"),
            (
                Method::RouterForceTerminateOccupant,
                "router.force_terminate_occupant",
            ),
            (Method::AgentDetect, "agent.detect"),
            (Method::AgentModels, "agent.models"),
            (Method::AgentRender, "agent.render"),
            (Method::AgentPreview, "agent.preview"),
            (Method::AgentWrite, "agent.write"),
            (Method::AgentCleanupPreview, "agent.cleanup.preview"),
            (Method::AgentCleanupWrite, "agent.cleanup.write"),
            (Method::ApiKeyUsage, "apikey.usage"),
        ];
        for (method, wire_name) in methods {
            assert_eq!(
                serde_json::to_string(&method).unwrap(),
                format!("\"{wire_name}\"")
            );
        }

        let request = Request {
            id: "request-1".into(),
            method: Method::RouterStart,
            params: Some(json!({"owner": "desktop"})),
        };
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"id":"request-1","method":"router.start","params":{"owner":"desktop"}}"#
        );

        let response = Response {
            id: Some("request-1".into()),
            result: None,
            error: Some(ProtocolError {
                code: ErrorCode::PortOccupied,
                message: "local port is occupied".into(),
                details: None,
            }),
        };
        assert_eq!(
            serde_json::to_string(&response).unwrap(),
            r#"{"id":"request-1","error":{"code":"PORT_OCCUPIED","message":"local port is occupied"}}"#
        );
    }

    #[test]
    fn rejects_unknown_or_sensitive_force_termination_fields() {
        for raw in [
            r#"{"confirmation_token":"token","pid":42}"#,
            r#"{"confirmation_token":"token","api_key":"secret"}"#,
        ] {
            assert!(serde_json::from_str::<ForceTerminateOccupantParams>(raw).is_err());
        }
        let parsed: ForceTerminateOccupantParams =
            serde_json::from_str(r#"{"confirmation_token":"token"}"#).unwrap();
        assert_eq!(parsed.confirmation_token, "token");
    }

    #[test]
    fn protocol_error_serialization_does_not_add_sensitive_fields() {
        let error = ProtocolError {
            code: ErrorCode::InvalidParams,
            message: "invalid method parameters".into(),
            details: Some(ErrorDetails {
                path: "/owner".into(),
                rule: "required".into(),
            }),
        };
        let encoded = serde_json::to_string(&error).unwrap();
        assert!(!encoded.contains("api_key"));
        assert!(!encoded.contains("PEM"));
        assert!(!encoded.contains("request_body"));
    }
}
