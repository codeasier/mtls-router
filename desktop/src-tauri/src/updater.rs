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

fn command_error(stage: &'static str, code: &'static str, message: &'static str) -> CommandError {
    command_error_detail(stage, code, message, &[])
}

fn command_error_detail(
    stage: &'static str,
    code: &'static str,
    message: &'static str,
    detail: &[(&'static str, &str)],
) -> CommandError {
    let event = match code {
        "UPDATE_SIGNATURE_FAILED" => "update.verify_failed",
        "UPDATE_BLOCKED_ROUTER_STATE" | "UPDATE_BLOCKED_LEGACY_ROUTER" => "update.preflight",
        "UPDATE_INSTALL_FAILED" | "UPDATE_BUSY" => "update.install_failed",
        "UPDATE_RESTART_BLOCKED" => "update.restart_blocked",
        "UPDATE_DOWNLOAD_FAILED" => "update.download_finished",
        _ => "update.check_finished",
    };
    crate::app_log::record(crate::app_log::Fields {
        level: "error",
        event,
        component: "updater",
        outcome: if code == "UPDATE_BUSY" {
            "denied"
        } else {
            "failure"
        },
        code: Some(code),
        stage: Some(stage),
        duration_ms: None,
        detail,
        durable: true,
    });
    CommandError::recoverable(code, message).with_stage(stage)
}

fn note_update(
    event: &'static str,
    stage: &'static str,
    outcome: &'static str,
    detail: &[(&'static str, &str)],
    durable: bool,
) {
    crate::app_log::record(crate::app_log::Fields {
        level: "info",
        event,
        component: "updater",
        outcome,
        code: None,
        stage: Some(stage),
        duration_ms: None,
        detail,
        durable,
    });
}

fn download_failure(error: &tauri_plugin_updater::Error) -> CommandError {
    match error {
        tauri_plugin_updater::Error::Minisign(_)
        | tauri_plugin_updater::Error::Base64(_)
        | tauri_plugin_updater::Error::SignatureUtf8(_) => command_error(
            "verify",
            "UPDATE_SIGNATURE_FAILED",
            "cannot verify the update",
        ),
        _ => command_error(
            "download",
            "UPDATE_DOWNLOAD_FAILED",
            "cannot download the update",
        ),
    }
}

fn is_new_stable_version(current: &Version, remote: &Version) -> bool {
    remote.pre.is_empty() && remote.build.is_empty() && remote > current
}

fn updater(app: &AppHandle, timeout: Duration) -> Result<tauri_plugin_updater::Updater> {
    app.updater_builder()
        .timeout(timeout)
        .version_comparator(|current, remote| is_new_stable_version(&current, &remote.version))
        .build()
        .map_err(|_| {
            command_error(
                "configure",
                "UPDATE_NOT_CONFIGURED",
                "online updates are unavailable in this build",
            )
        })
}

async fn check(app: &AppHandle, timeout: Duration) -> Result<Option<Update>> {
    updater(app, timeout)?
        .check()
        .await
        .map_err(|_| command_error("check", "UPDATE_CHECK_FAILED", "cannot check for updates"))
}

fn update_info(update: &Update) -> UpdateInfo {
    UpdateInfo {
        version: update.version.clone(),
        notes: update.body.clone().filter(|value| !value.trim().is_empty()),
        published_at: update.date.map(|value| value.to_string()),
    }
}

#[tauri::command]
pub async fn update_check(app: AppHandle) -> crate::error::CommandResult<UpdateCheckResult> {
    note_update("update.check_started", "check", "success", &[], false);
    let update = check(&app, CHECK_TIMEOUT).await?;
    if let Some(update) = update.as_ref() {
        if let Some(version) = stable_version_text(&update.version) {
            note_update(
                "update.available",
                "check",
                "success",
                &[("version", version.as_str())],
                false,
            );
        }
    } else {
        note_update("update.check_finished", "check", "success", &[], false);
    }
    Ok(UpdateCheckResult {
        available: update.is_some(),
        current_version: env!("CARGO_PKG_VERSION").to_owned(),
        update: update.as_ref().map(update_info),
    })
}

fn stable_version_text(value: &str) -> Option<String> {
    let version = Version::parse(value).ok()?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return None;
    }
    Some(version.to_string())
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateRouterPreflight {
    Proceed,
    Stop,
}

fn preflight_detail<'a>(status: &'a RouterStatus) -> Vec<(&'static str, &'a str)> {
    let mut detail = vec![("state", status.state.as_str())];
    if let Some(owner) = status.owner.as_deref() {
        detail.push(("owner", owner));
    }
    detail
}

fn update_router_preflight(status: &RouterStatus) -> Result<UpdateRouterPreflight> {
    let decision = match (status.state.as_str(), status.owner.as_deref()) {
        ("absent", None)
        | ("start_failed", Some("desktop"))
        | ("external_compatible", Some("cli"))
        | ("degraded", Some("cli")) => Ok(UpdateRouterPreflight::Proceed),
        ("desktop_owned", Some("desktop"))
        | ("degraded", Some("desktop"))
        | ("legacy_managed", Some("desktop")) => Ok(UpdateRouterPreflight::Stop),
        _ => Err(command_error_detail(
            "preflight",
            "UPDATE_BLOCKED_ROUTER_STATE",
            "correlated router state could not be verified before the update",
            &preflight_detail(status),
        )),
    };
    if decision.is_ok() {
        note_update(
            "update.preflight",
            "preflight",
            "success",
            &preflight_detail(status),
            true,
        );
    }
    decision
}

async fn stop_owned_router_with<ReadStatus, ReadStatusFuture, Stop, StopFuture>(
    mut read_status: ReadStatus,
    mut stop: Stop,
) -> Result<bool>
where
    ReadStatus: FnMut() -> ReadStatusFuture,
    ReadStatusFuture: std::future::Future<Output = Result<RouterStatus>>,
    Stop: FnMut() -> StopFuture,
    StopFuture: std::future::Future<Output = Result<RouterStatus>>,
{
    let status = read_status().await?;
    match update_router_preflight(&status)? {
        UpdateRouterPreflight::Proceed => Ok(false),
        UpdateRouterPreflight::Stop => {
            stop().await?;
            let remaining = read_status().await?;
            if update_router_preflight(&remaining) != Ok(UpdateRouterPreflight::Proceed) {
                return Err(command_error(
                    "preflight",
                    "UPDATE_BLOCKED_LEGACY_ROUTER",
                    "a correlated legacy router could not be stopped before the update",
                ));
            }
            Ok(true)
        }
    }
}

async fn install_after_router_preflight_with<
    ReadStatus,
    ReadStatusFuture,
    Stop,
    StopFuture,
    Install,
    Restart,
    RestartFuture,
>(
    read_status: ReadStatus,
    stop: Stop,
    install: Install,
    restart: Restart,
) -> Result<()>
where
    ReadStatus: FnMut() -> ReadStatusFuture,
    ReadStatusFuture: std::future::Future<Output = Result<RouterStatus>>,
    Stop: FnMut() -> StopFuture,
    StopFuture: std::future::Future<Output = Result<RouterStatus>>,
    Install: FnOnce() -> bool,
    Restart: FnOnce() -> RestartFuture,
    RestartFuture: std::future::Future<Output = ()>,
{
    let stopped_router = stop_owned_router_with(read_status, stop).await?;
    if !install() {
        if stopped_router {
            restart().await;
        }
        return Err(command_error(
            "install",
            "UPDATE_INSTALL_FAILED",
            "cannot install the update",
        ));
    }
    Ok(())
}

async fn download_and_install(app: &AppHandle, state: &AppState, update: Update) -> Result<()> {
    note_update("update.download_started", "download", "success", &[], true);
    let progress_app = app.clone();
    let mut downloaded = 0_u64;
    let mut next_quarter = 1_u64;
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded = downloaded.saturating_add(chunk as u64);
                let _ =
                    progress_app.emit(UPDATE_PROGRESS_EVENT, UpdateProgress { downloaded, total });
                if let Some(total) = total.filter(|total| *total > 0) {
                    let quarter = downloaded.saturating_mul(4) / total;
                    if quarter >= next_quarter && quarter < 4 {
                        next_quarter = quarter + 1;
                        let downloaded_text = downloaded.to_string();
                        let total_text = total.to_string();
                        note_update(
                            "update.download_milestone",
                            "download",
                            "success",
                            &[
                                ("downloaded", downloaded_text.as_str()),
                                ("total", total_text.as_str()),
                            ],
                            false,
                        );
                    }
                }
            },
            || {},
        )
        .await
        .map_err(|error| download_failure(&error))?;
    note_update("update.download_finished", "download", "success", &[], true);

    install_after_router_preflight_with(
        || state.manager.call("router.status", json!({})),
        || crate::orchestration::stop(&state.manager, &state.scheduler),
        || {
            #[cfg(windows)]
            {
                install_windows_verified(&bytes).is_ok()
            }
            #[cfg(not(windows))]
            {
                update.install(bytes).is_ok()
            }
        },
        || async {
            let _ = crate::orchestration::start(&state.manager, &state.scheduler).await;
        },
    )
    .await
}

#[tauri::command]
pub async fn update_install(
    version: String,
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> crate::error::CommandResult<()> {
    let _operation = crate::app_log::begin_operation();
    note_update("update.install_requested", "install", "success", &[], true);
    let expected = parse_expected_version(&version)?;
    let output = state
        .lifecycle
        .run_operation(async {
            let update = check(&app, INSTALL_TIMEOUT).await?.ok_or_else(|| {
                command_error(
                    "recheck",
                    "UPDATE_NOT_AVAILABLE",
                    "the update is no longer available",
                )
            })?;
            let announced = parse_expected_version(&update.version)?;
            if announced != expected {
                return Err(command_error(
                    "recheck",
                    "UPDATE_CHANGED",
                    "the available update changed; check again",
                ));
            }
            download_and_install(&app, &state, update).await
        })
        .await
        .ok_or_else(|| {
            command_error(
                "lifecycle",
                "UPDATE_BUSY",
                "another lifecycle operation is in progress",
            )
        })?;

    if output.quit_action == QuitAction::ExecuteQuit {
        crate::tray::execute_quit(app);
        return output.value.map_err(Into::into);
    }
    output.value?;
    let prepared = state.lifecycle.prepare_restart();
    match complete_successful_install(cfg!(windows), prepared)? {
        InstallCompletion::ExitProcess => {
            note_update("update.succeeded", "install", "success", &[], true);
            crate::app_log::flush();
            std::process::exit(0)
        }
        InstallCompletion::RestartApp => {
            note_update("update.succeeded", "install", "success", &[], true);
            crate::app_log::flush();
            app.restart()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstallCompletion {
    ExitProcess,
    RestartApp,
}

fn complete_successful_install(windows: bool, restart_prepared: bool) -> Result<InstallCompletion> {
    if windows {
        // ShellExecuteW success only means the installer launched. Staying
        // alive races NSIS replacing in-use files, even if restart is blocked.
        return Ok(InstallCompletion::ExitProcess);
    }
    if !restart_prepared {
        return Err(command_error(
            "restart",
            "UPDATE_RESTART_BLOCKED",
            "restart is blocked after the update was applied",
        ));
    }
    Ok(InstallCompletion::RestartApp)
}

#[cfg(any(windows, test))]
fn installer_launch_succeeded(result: isize) -> bool {
    result > 32
}

#[cfg(windows)]
fn install_windows_verified(bytes: &[u8]) -> std::io::Result<()> {
    use std::{fs, io::Write, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOW};

    // This accepts only the signature-verified bytes returned by Update::download.
    // Release packaging publishes raw NSIS executables, never MSI or ZIP here.
    if !bytes.starts_with(b"MZ") {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
    }
    let directory = std::env::temp_dir().join(format!("codeasier-update-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory)?;
    let installer = directory.join("CodeasierRouter-update.exe");
    let result = (|| {
        crate::windows_security::restrict_private(&directory, true)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&installer)?;
        crate::windows_security::restrict_private(&installer, false)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        let path: Vec<_> = installer.as_os_str().encode_wide().chain(Some(0)).collect();
        let launched = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                windows_sys::w!("open"),
                path.as_ptr(),
                windows_sys::w!("/P /R /UPDATE"),
                std::ptr::null(),
                SW_SHOW,
            )
        };
        if !installer_launch_succeeded(launched as isize) {
            return Err(std::io::Error::other("update installer launch failed"));
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&installer);
        let _ = fs::remove_dir(&directory);
    }
    // On success the external installer still needs this file. Its temporary
    // directory is left to OS temp cleanup, as in the upstream updater.
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_shell_launch_errors_are_not_install_success() {
        for code in 0..=32 {
            assert!(!installer_launch_succeeded(code));
        }
        assert!(installer_launch_succeeded(33));
        assert!(installer_launch_succeeded(1024));
    }

    #[test]
    fn launched_windows_installer_exits_even_when_restart_is_blocked() {
        assert_eq!(
            complete_successful_install(true, false).unwrap(),
            InstallCompletion::ExitProcess
        );
        assert_eq!(
            complete_successful_install(false, true).unwrap(),
            InstallCompletion::RestartApp
        );
        let error = complete_successful_install(false, false).unwrap_err();
        assert_eq!(error.code, "UPDATE_RESTART_BLOCKED");
        assert!(!error.message.contains("installed"));
    }

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
    fn update_comparison_rejects_prerelease_build_metadata_and_downgrades() {
        let current = Version::new(1, 2, 3);
        assert!(is_new_stable_version(&current, &Version::new(1, 2, 4)));
        for remote in ["1.2.4-beta.1", "1.2.4+build", "1.2.3", "1.2.2"] {
            assert!(!is_new_stable_version(
                &current,
                &Version::parse(remote).unwrap()
            ));
        }
    }

    fn status(state: &str, owner: Option<&str>) -> RouterStatus {
        RouterStatus {
            state: state.to_owned(),
            owner: owner.map(str::to_owned),
            ..RouterStatus::default()
        }
    }

    fn runtime() -> &'static tokio::runtime::Runtime {
        use std::sync::OnceLock;
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().unwrap())
    }

    #[test]
    fn legacy_router_is_stopped_rechecked_then_installed() {
        use std::{
            collections::VecDeque,
            sync::{Arc, Mutex},
        };

        runtime().block_on(async {
            let events = Arc::new(Mutex::new(Vec::new()));
            let statuses = Arc::new(Mutex::new(VecDeque::from([
                Ok(status("legacy_managed", Some("desktop"))),
                Ok(status("absent", None)),
            ])));
            let result = install_after_router_preflight_with(
                {
                    let events = events.clone();
                    let statuses = statuses.clone();
                    move || {
                        events.lock().unwrap().push("status");
                        let result = statuses.lock().unwrap().pop_front().unwrap();
                        async move { result }
                    }
                },
                {
                    let events = events.clone();
                    move || {
                        events.lock().unwrap().push("stop");
                        async { Ok(status("absent", None)) }
                    }
                },
                {
                    let events = events.clone();
                    move || {
                        events.lock().unwrap().push("install");
                        true
                    }
                },
                {
                    let events = events.clone();
                    move || async move {
                        events.lock().unwrap().push("restart");
                    }
                },
            )
            .await;

            assert!(result.is_ok());
            assert_eq!(
                &*events.lock().unwrap(),
                &["status", "stop", "status", "install"]
            );
        });
    }

    #[test]
    fn start_failed_desktop_owned_proceeds_without_stop() {
        use std::sync::{Arc, Mutex};

        runtime().block_on(async {
            let events = Arc::new(Mutex::new(Vec::new()));
            let result = install_after_router_preflight_with(
                {
                    let events = events.clone();
                    move || {
                        events.lock().unwrap().push("status");
                        async { Ok(status("start_failed", Some("desktop"))) }
                    }
                },
                {
                    let events = events.clone();
                    move || {
                        events.lock().unwrap().push("stop");
                        async { Ok(status("absent", None)) }
                    }
                },
                {
                    let events = events.clone();
                    move || {
                        events.lock().unwrap().push("install");
                        true
                    }
                },
                {
                    let events = events.clone();
                    move || async move {
                        events.lock().unwrap().push("restart");
                    }
                },
            )
            .await;

            assert!(result.is_ok());
            assert_eq!(&*events.lock().unwrap(), &["status", "install"]);
        });
    }

    #[test]
    fn stale_and_unknown_router_states_block_before_stop_or_install() {
        use std::sync::{Arc, Mutex};

        runtime().block_on(async {
            for blocked in ["stale", "unknown_occupant", "future_state"] {
                let events = Arc::new(Mutex::new(Vec::new()));
                let result = install_after_router_preflight_with(
                    {
                        let events = events.clone();
                        move || {
                            events.lock().unwrap().push("status");
                            let value = status(blocked, Some("desktop"));
                            async move { Ok(value) }
                        }
                    },
                    {
                        let events = events.clone();
                        move || {
                            events.lock().unwrap().push("stop");
                            async { Ok(status("absent", None)) }
                        }
                    },
                    {
                        let events = events.clone();
                        move || {
                            events.lock().unwrap().push("install");
                            true
                        }
                    },
                    {
                        let events = events.clone();
                        move || async move {
                            events.lock().unwrap().push("restart");
                        }
                    },
                )
                .await
                .unwrap_err();

                assert_eq!(result.code, "UPDATE_BLOCKED_ROUTER_STATE");
                assert_eq!(&*events.lock().unwrap(), &["status"]);
            }
        });
    }

    #[test]
    fn ambiguous_stop_failure_is_not_retried_and_never_installs() {
        use std::sync::{Arc, Mutex};

        runtime().block_on(async {
            let events = Arc::new(Mutex::new(Vec::new()));
            let stop_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let result = install_after_router_preflight_with(
                {
                    let events = events.clone();
                    move || {
                        events.lock().unwrap().push("status");
                        async { Ok(status("legacy_managed", Some("desktop"))) }
                    }
                },
                {
                    let events = events.clone();
                    let stop_calls = stop_calls.clone();
                    move || {
                        events.lock().unwrap().push("stop");
                        stop_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        async {
                            Err(CommandError::new(
                                "INVALID_RESPONSE",
                                "manager response was ambiguous",
                            ))
                        }
                    }
                },
                {
                    let events = events.clone();
                    move || {
                        events.lock().unwrap().push("install");
                        true
                    }
                },
                {
                    let events = events.clone();
                    move || async move {
                        events.lock().unwrap().push("restart");
                    }
                },
            )
            .await
            .unwrap_err();

            assert_eq!(result.code, "INVALID_RESPONSE");
            assert_eq!(stop_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert_eq!(&*events.lock().unwrap(), &["status", "stop"]);
        });
    }

    #[test]
    fn unresolved_post_stop_state_blocks_install() {
        use std::{
            collections::VecDeque,
            sync::{Arc, Mutex},
        };

        runtime().block_on(async {
            for remaining in [
                "legacy_managed",
                "desktop_owned",
                "stale",
                "unknown_occupant",
                "future_state",
            ] {
                let events = Arc::new(Mutex::new(Vec::new()));
                let statuses = Arc::new(Mutex::new(VecDeque::from([
                    Ok(status("legacy_managed", Some("desktop"))),
                    Ok(status(remaining, Some("desktop"))),
                ])));
                let result = install_after_router_preflight_with(
                    {
                        let events = events.clone();
                        let statuses = statuses.clone();
                        move || {
                            events.lock().unwrap().push("status");
                            let result = statuses.lock().unwrap().pop_front().unwrap();
                            async move { result }
                        }
                    },
                    {
                        let events = events.clone();
                        move || {
                            events.lock().unwrap().push("stop");
                            async { Ok(status("absent", None)) }
                        }
                    },
                    {
                        let events = events.clone();
                        move || {
                            events.lock().unwrap().push("install");
                            true
                        }
                    },
                    {
                        let events = events.clone();
                        move || async move {
                            events.lock().unwrap().push("restart");
                        }
                    },
                )
                .await
                .unwrap_err();

                assert_eq!(result.code, "UPDATE_BLOCKED_LEGACY_ROUTER");
                assert_eq!(&*events.lock().unwrap(), &["status", "stop", "status"]);
            }
        });
    }

    #[test]
    fn post_stop_status_failure_blocks_install() {
        use std::{
            collections::VecDeque,
            sync::{Arc, Mutex},
        };

        runtime().block_on(async {
            let events = Arc::new(Mutex::new(Vec::new()));
            let statuses = Arc::new(Mutex::new(VecDeque::from([
                Ok(status("legacy_managed", Some("desktop"))),
                Err(CommandError::manager_failed()),
            ])));
            let result = install_after_router_preflight_with(
                {
                    let events = events.clone();
                    let statuses = statuses.clone();
                    move || {
                        events.lock().unwrap().push("status");
                        let result = statuses.lock().unwrap().pop_front().unwrap();
                        async move { result }
                    }
                },
                {
                    let events = events.clone();
                    move || {
                        events.lock().unwrap().push("stop");
                        async { Ok(status("absent", None)) }
                    }
                },
                {
                    let events = events.clone();
                    move || {
                        events.lock().unwrap().push("install");
                        true
                    }
                },
                {
                    let events = events.clone();
                    move || async move {
                        events.lock().unwrap().push("restart");
                    }
                },
            )
            .await
            .unwrap_err();

            assert_eq!(result.code, "MANAGER_FAILED");
            assert_eq!(&*events.lock().unwrap(), &["status", "stop", "status"]);
        });
    }

    #[test]
    fn failed_install_restarts_only_a_router_stopped_by_preflight() {
        use std::{
            collections::VecDeque,
            sync::{Arc, Mutex},
        };

        runtime().block_on(async {
            for (initial, statuses, expected) in [
                (
                    "legacy_managed",
                    VecDeque::from([
                        Ok(status("legacy_managed", Some("desktop"))),
                        Ok(status("absent", None)),
                    ]),
                    vec!["status", "stop", "status", "install", "restart"],
                ),
                (
                    "absent",
                    VecDeque::from([Ok(status("absent", None))]),
                    vec!["status", "install"],
                ),
                (
                    "start_failed",
                    VecDeque::from([Ok(status("start_failed", Some("desktop")))]),
                    vec!["status", "install"],
                ),
            ] {
                let events = Arc::new(Mutex::new(Vec::new()));
                let statuses = Arc::new(Mutex::new(statuses));
                let result = install_after_router_preflight_with(
                    {
                        let events = events.clone();
                        let statuses = statuses.clone();
                        move || {
                            events.lock().unwrap().push("status");
                            let result = statuses.lock().unwrap().pop_front().unwrap();
                            async move { result }
                        }
                    },
                    {
                        let events = events.clone();
                        move || {
                            events.lock().unwrap().push("stop");
                            async { Ok(status("absent", None)) }
                        }
                    },
                    {
                        let events = events.clone();
                        move || {
                            events.lock().unwrap().push("install");
                            false
                        }
                    },
                    {
                        let events = events.clone();
                        move || async move {
                            events.lock().unwrap().push("restart");
                        }
                    },
                )
                .await
                .unwrap_err();

                assert_eq!(result.code, "UPDATE_INSTALL_FAILED", "{initial}");
                assert_eq!(&*events.lock().unwrap(), expected.as_slice(), "{initial}");
            }
        });
    }

    #[test]
    fn updater_preflight_accepts_only_explicit_safe_state_and_owner_pairs() {
        let status = |state: &str, owner: Option<&str>| RouterStatus {
            state: state.to_owned(),
            owner: owner.map(str::to_owned),
            ..RouterStatus::default()
        };
        for (state, owner) in [
            ("absent", None),
            ("start_failed", Some("desktop")),
            ("external_compatible", Some("cli")),
            ("degraded", Some("cli")),
        ] {
            assert_eq!(
                update_router_preflight(&status(state, owner)).unwrap(),
                UpdateRouterPreflight::Proceed,
                "{state}/{owner:?}"
            );
        }
        for (state, owner) in [
            ("desktop_owned", Some("desktop")),
            ("degraded", Some("desktop")),
            ("legacy_managed", Some("desktop")),
        ] {
            assert_eq!(
                update_router_preflight(&status(state, owner)).unwrap(),
                UpdateRouterPreflight::Stop,
                "{state}/{owner:?}"
            );
        }
        for (state, owner) in [
            ("start_failed", None),
            ("stale", Some("desktop")),
            ("unknown_occupant", None),
            ("future_state", None),
            ("absent", Some("desktop")),
            ("external_compatible", Some("desktop")),
            ("desktop_owned", Some("cli")),
            ("legacy_managed", None),
        ] {
            assert_eq!(
                update_router_preflight(&status(state, owner))
                    .unwrap_err()
                    .code,
                "UPDATE_BLOCKED_ROUTER_STATE",
                "{state}/{owner:?}"
            );
        }
    }

    fn logged(body: impl FnOnce(&std::path::Path)) -> String {
        let directory =
            std::env::temp_dir().join(format!("mtls-update-log-{}", uuid::Uuid::new_v4()));
        crate::app_log::with_temp(&directory, || body(&directory));
        let text = std::fs::read_to_string(directory.join("app.log")).unwrap_or_default();
        let _ = std::fs::remove_dir_all(directory);
        text
    }

    #[test]
    fn download_failure_writes_download_code_without_the_secret() {
        let secret = "sk-supersecretvalue";
        let text = logged(|_| {
            let error = download_failure(&tauri_plugin_updater::Error::Network(secret.to_owned()));
            assert_eq!(error.code, "UPDATE_DOWNLOAD_FAILED");
            assert_eq!(error.stage.as_deref(), Some("download"));
        });
        assert!(text.contains("UPDATE_DOWNLOAD_FAILED"));
        assert!(text.contains("\"stage\":\"download\""));
        assert!(text.contains("update.download_finished"));
        assert!(!text.contains(secret));
    }

    #[test]
    fn signature_failure_writes_verify_code_without_the_secret() {
        let secret = "sk-supersecretvalue";
        let text = logged(|_| {
            let error = download_failure(&tauri_plugin_updater::Error::SignatureUtf8(
                secret.to_owned(),
            ));
            assert_eq!(error.code, "UPDATE_SIGNATURE_FAILED");
            assert_eq!(error.stage.as_deref(), Some("verify"));
        });
        assert!(text.contains("UPDATE_SIGNATURE_FAILED"));
        assert!(text.contains("\"event\":\"update.verify_failed\""));
        assert!(!text.contains(secret));
    }

    #[test]
    fn preflight_block_writes_router_state_code() {
        let text = logged(|_| {
            let error = update_router_preflight(&status("future_state", None)).unwrap_err();
            assert_eq!(error.code, "UPDATE_BLOCKED_ROUTER_STATE");
        });
        assert!(text.contains("UPDATE_BLOCKED_ROUTER_STATE"));
        assert!(text.contains("\"stage\":\"preflight\""));
        assert!(text.contains("\"state\":\"future_state\""));
    }

    #[test]
    fn install_failure_writes_install_code() {
        let text = logged(|_| {
            runtime().block_on(async {
                let error = install_after_router_preflight_with(
                    || async { Ok(status("absent", None)) },
                    || async { Ok(status("absent", None)) },
                    || false,
                    || async {},
                )
                .await
                .unwrap_err();
                assert_eq!(error.code, "UPDATE_INSTALL_FAILED");
            });
        });
        assert!(text.contains("UPDATE_INSTALL_FAILED"));
        assert!(text.contains("\"stage\":\"install\""));
    }
}
