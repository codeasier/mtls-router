use crate::{
    commands::AppState,
    error::{CommandError, Result},
    lifecycle::QuitAction,
    types::{RouterStatus, UpdateCheckResult, UpdateInfo, UpdateProgress},
};
use semver::Version;
use serde_json::json;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};

const UPDATE_PROGRESS_EVENT: &str = "update-download-progress";
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

fn command_error(code: &'static str, message: &'static str) -> CommandError {
    CommandError::recoverable(code, message)
}

fn updater(app: &AppHandle, timeout: Duration) -> Result<tauri_plugin_updater::Updater> {
    app.updater_builder()
        .timeout(timeout)
        .version_comparator(|current, remote| {
            remote.version.pre.is_empty() && remote.version > current
        })
        .build()
        .map_err(|_| {
            command_error(
                "UPDATE_NOT_CONFIGURED",
                "online updates are unavailable in this build",
            )
        })
}

async fn check(app: &AppHandle, timeout: Duration) -> Result<Option<Update>> {
    updater(app, timeout)?
        .check()
        .await
        .map_err(|_| command_error("UPDATE_CHECK_FAILED", "cannot check for updates"))
}

fn update_info(update: &Update) -> UpdateInfo {
    UpdateInfo {
        version: update.version.clone(),
        notes: update.body.clone().filter(|value| !value.trim().is_empty()),
        published_at: update.date.map(|value| value.to_string()),
    }
}

#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<UpdateCheckResult> {
    let update = check(&app, CHECK_TIMEOUT).await?;
    Ok(UpdateCheckResult {
        available: update.is_some(),
        current_version: env!("CARGO_PKG_VERSION").to_owned(),
        update: update.as_ref().map(update_info),
    })
}

fn parse_expected_version(value: &str) -> Result<Version> {
    let version = Version::parse(value)
        .map_err(|_| CommandError::invalid_params("version must be valid SemVer"))?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err(CommandError::invalid_params(
            "version must identify a stable release",
        ));
    }
    Ok(version)
}

fn is_desktop_owned(status: &RouterStatus) -> bool {
    matches!(status.state.as_str(), "desktop_owned" | "degraded")
        && status.owner.as_deref() == Some("desktop")
}

async fn stop_owned_router(state: &AppState) -> Result<bool> {
    let status: RouterStatus = state.manager.call("router.status", json!({})).await?;
    if is_desktop_owned(&status) {
        crate::orchestration::stop(&state.manager, &state.scheduler).await?;
        return Ok(true);
    }
    Ok(false)
}

async fn download_and_install(app: &AppHandle, state: &AppState, update: Update) -> Result<()> {
    let progress_app = app.clone();
    let mut downloaded = 0_u64;
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded = downloaded.saturating_add(chunk as u64);
                let _ =
                    progress_app.emit(UPDATE_PROGRESS_EVENT, UpdateProgress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|_| command_error("UPDATE_DOWNLOAD_FAILED", "cannot download the update"))?;

    let stopped_router = stop_owned_router(state).await?;
    if update.install(bytes).is_err() {
        if stopped_router {
            let _ = crate::orchestration::start(&state.manager, &state.scheduler).await;
        }
        return Err(command_error(
            "UPDATE_INSTALL_FAILED",
            "cannot install the update",
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn update_install(
    version: String,
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<()> {
    let expected = parse_expected_version(&version)?;
    let output = state
        .lifecycle
        .run_operation(async {
            let update = check(&app, INSTALL_TIMEOUT).await?.ok_or_else(|| {
                command_error("UPDATE_NOT_AVAILABLE", "the update is no longer available")
            })?;
            let announced = parse_expected_version(&update.version)?;
            if announced != expected {
                return Err(command_error(
                    "UPDATE_CHANGED",
                    "the available update changed; check again",
                ));
            }
            download_and_install(&app, &state, update).await
        })
        .await
        .ok_or_else(|| {
            command_error("UPDATE_BUSY", "another lifecycle operation is in progress")
        })?;

    if output.quit_action == QuitAction::ExecuteQuit {
        crate::tray::execute_quit(app);
        return output.value;
    }
    output.value?;
    if !state.lifecycle.prepare_restart() {
        return Err(command_error(
            "UPDATE_RESTART_BLOCKED",
            "the update was installed but restart is blocked",
        ));
    }
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_update_version_must_be_stable_semver() {
        assert_eq!(
            parse_expected_version("1.2.3").unwrap(),
            Version::new(1, 2, 3)
        );
        for invalid in ["v1.2.3", "1.2", "1.2.3-beta.1", "1.2.3+build"] {
            assert!(parse_expected_version(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn only_verified_desktop_owned_router_is_stopped() {
        let status = |state: &str, owner: Option<&str>| RouterStatus {
            state: state.to_owned(),
            owner: owner.map(str::to_owned),
            listen_addr: None,
            pid: None,
            last_error: None,
            recent_logs: None,
        };
        assert!(is_desktop_owned(&status("desktop_owned", Some("desktop"))));
        assert!(is_desktop_owned(&status("degraded", Some("desktop"))));
        assert!(!is_desktop_owned(&status(
            "external_compatible",
            Some("cli")
        )));
        assert!(!is_desktop_owned(&status("degraded", Some("external"))));
    }
}
