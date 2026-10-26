use crate::{
    commands::AppState,
    error::{CommandError, Result},
    runtime,
};
use router_identity::{ConnectionSummary, NativeClient};
use serde::Serialize;
use std::path::Path;
use tauri_plugin_opener::OpenerExt;

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
    note_identity("status", &summary);
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
