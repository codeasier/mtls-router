use crate::{
    commands::AppState,
    error::{CommandError, Result},
    runtime,
};
use router_identity::{ConnectionSummary, NativeClient};
use serde::Serialize;
use std::{path::Path, sync::Mutex};
use tauri_plugin_opener::OpenerExt;

static STATUS_PHASE: Mutex<Option<String>> = Mutex::new(None);

fn client(state: &AppState) -> Result<NativeClient> {
    runtime::identity_client(Path::new(&state.paths.data_dir))
}
fn error(value: router_identity::IdentityError) -> CommandError {
    CommandError::new(value.code(), "certificate operation could not complete")
}

fn note_identity(stage: &'static str, summary: &ConnectionSummary) {
    let phase = summary.phase.as_str();
    crate::app_log::record(crate::app_log::Fields {
        level: "info",
        event: "identity.changed",
        component: "identity",
        outcome: "success",
        code: None,
        stage: Some(stage),
        duration_ms: None,
        detail: &[("phase", phase)],
        durable: true,
    });
}

fn note_status(summary: &ConnectionSummary) {
    let phase = summary.phase.clone();
    {
        let mut last = STATUS_PHASE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if last.as_deref() == Some(phase.as_str()) {
            return;
        }
        *last = Some(phase);
    }
    note_identity("status", summary);
}

#[derive(Serialize)]
pub struct AuthorizationSummary {
    verification_uri: String,
    short_code: String,
    browser_opened: bool,
}

#[tauri::command]
pub async fn identity_status(
    state: tauri::State<'_, AppState>,
) -> crate::error::CommandResult<ConnectionSummary> {
    let summary = client(&state)?.status().await.map_err(error)?;
    note_status(&summary);
    Ok(summary)
}
#[tauri::command]
pub async fn identity_connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> crate::error::CommandResult<AuthorizationSummary> {
    let started = client(&state)?
        .connect("desktop", "CodeasierRouter Desktop")
        .await
        .map_err(error)?;
    let browser_opened = app
        .opener()
        .open_url(&started.verification_uri, None::<&str>)
        .is_ok();
    crate::app_log::record(crate::app_log::Fields {
        level: "info",
        event: "identity.changed",
        component: "identity",
        outcome: "success",
        code: None,
        stage: Some("connect"),
        duration_ms: None,
        detail: &[],
        durable: true,
    });
    Ok(AuthorizationSummary {
        verification_uri: started.verification_uri,
        short_code: started.short_code,
        browser_opened,
    })
}
#[tauri::command]
pub async fn identity_renew(
    state: tauri::State<'_, AppState>,
) -> crate::error::CommandResult<ConnectionSummary> {
    let summary = client(&state)?.renew().await.map_err(error)?;
    note_identity("renew", &summary);
    Ok(summary)
}
#[tauri::command]
pub async fn identity_disconnect(
    state: tauri::State<'_, AppState>,
) -> crate::error::CommandResult<ConnectionSummary> {
    let summary = client(&state)?.logout().await.map_err(error)?;
    note_identity("logout", &summary);
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(phase: &str) -> ConnectionSummary {
        ConnectionSummary {
            generation: 1,
            phase: phase.to_owned(),
            user_id: None,
            device_id: None,
            expires_at: None,
            revocation_pending: false,
            error_code: None,
        }
    }

    #[test]
    fn repeated_status_records_at_most_one_event_until_phase_changes() {
        let directory =
            std::env::temp_dir().join(format!("mtls-identity-status-{}", uuid::Uuid::new_v4()));
        crate::app_log::with_temp(&directory, || {
            if let Ok(mut phase) = STATUS_PHASE.lock() {
                *phase = None;
            }
            let connected = summary("connected");
            note_status(&connected);
            note_status(&connected);
            note_status(&connected);
            let records = crate::app_log::recent(10);
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].event, "identity.changed");
            assert_eq!(records[0].stage.as_deref(), Some("status"));
            assert_eq!(
                records[0]
                    .detail
                    .as_ref()
                    .and_then(|detail| detail.get("phase"))
                    .map(String::as_str),
                Some("connected")
            );

            note_identity("connect", &connected);
            note_status(&connected);
            assert_eq!(crate::app_log::recent(10).len(), 2);

            note_status(&summary("expired"));
            let records = crate::app_log::recent(10);
            assert_eq!(records.len(), 3);
            assert_eq!(records[2].stage.as_deref(), Some("status"));
            assert_eq!(
                records[2]
                    .detail
                    .as_ref()
                    .and_then(|detail| detail.get("phase"))
                    .map(String::as_str),
                Some("expired")
            );
        });
        let _ = std::fs::remove_dir_all(directory);
    }
}
