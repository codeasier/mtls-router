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

#[derive(Serialize)]
pub struct AuthorizationSummary {
    verification_uri: String,
    short_code: String,
    browser_opened: bool,
}

#[tauri::command]
pub async fn identity_status(state: tauri::State<'_, AppState>) -> Result<ConnectionSummary> {
    client(&state)?.status().await.map_err(error)
}
#[tauri::command]
pub async fn identity_connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AuthorizationSummary> {
    let started = client(&state)?
        .connect("desktop", "CodeasierRouter Desktop")
        .await
        .map_err(error)?;
    let browser_opened = app
        .opener()
        .open_url(&started.verification_uri, None::<&str>)
        .is_ok();
    Ok(AuthorizationSummary {
        verification_uri: started.verification_uri,
        short_code: started.short_code,
        browser_opened,
    })
}
#[tauri::command]
pub async fn identity_renew(state: tauri::State<'_, AppState>) -> Result<ConnectionSummary> {
    client(&state)?.renew().await.map_err(error)
}
#[tauri::command]
pub async fn identity_disconnect(state: tauri::State<'_, AppState>) -> Result<ConnectionSummary> {
    client(&state)?.logout().await.map_err(error)
}
