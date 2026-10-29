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
    Install: FnOnce() -> std::result::Result<(), InstallFault>,
    Restart: FnOnce() -> RestartFuture,
    RestartFuture: std::future::Future<Output = Result<()>>,
{
    let stopped_router = stop_owned_router_with(read_status, stop).await?;
    if let Err(fault) = install() {
        // Record the install cause before recovery so a later start cannot
        // hide which preparation or launch step failed.
        let error = record_install_failure(&fault);
        if stopped_router {
            record_restore(&restart().await);
        }
        return Err(error);
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
                install_windows_verified(&bytes)
            }
            #[cfg(not(windows))]
            {
                update
                    .install(bytes)
                    .map(|_| ())
                    .map_err(|error| plugin_install_fault(&error))
            }
        },
        || async {
            crate::orchestration::start(&state.manager, &state.scheduler)
                .await
                .map(|_| ())
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
        persist_success_before_deferred_quit(output.quit_action, output.value.is_ok());
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

fn persist_success_before_deferred_quit(quit_action: QuitAction, install_succeeded: bool) {
    if quit_action == QuitAction::ExecuteQuit && install_succeeded {
        note_update("update.succeeded", "install", "success", &[], true);
        crate::app_log::flush();
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstallPhase {
    Format,
    TempDir,
    RestrictDir,
    CreateFile,
    RestrictFile,
    Write,
    Sync,
    Launch,
    Plugin,
}

impl InstallPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Format => "format",
            Self::TempDir => "temp_dir",
            Self::RestrictDir => "restrict_dir",
            Self::CreateFile => "create_file",
            Self::RestrictFile => "restrict_file",
            Self::Write => "write",
            Self::Sync => "sync",
            Self::Launch => "launch",
            Self::Plugin => "plugin",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InstallFault {
    phase: InstallPhase,
    os_error: Option<i32>,
    /// ShellExecuteW's own return when launch fails inside 0..=32.
    shell_result: Option<i32>,
}

fn io_fault(phase: InstallPhase, error: &std::io::Error) -> InstallFault {
    InstallFault {
        phase,
        os_error: error.raw_os_error().filter(|code| *code > 0),
        shell_result: None,
    }
}

fn launch_fault(code: isize) -> InstallFault {
    InstallFault {
        phase: InstallPhase::Launch,
        os_error: None,
        shell_result: i32::try_from(code)
            .ok()
            .filter(|value| (0..=32).contains(value)),
    }
}

fn plugin_install_fault(error: &tauri_plugin_updater::Error) -> InstallFault {
    let os_error = match error {
        tauri_plugin_updater::Error::Io(error) => error.raw_os_error().filter(|code| *code > 0),
        _ => None,
    };
    InstallFault {
        phase: InstallPhase::Plugin,
        os_error,
        shell_result: None,
    }
}

fn record_install_failure(fault: &InstallFault) -> CommandError {
    let os_error = fault.os_error.map(|value| value.to_string());
    let shell_result = fault.shell_result.map(|value| value.to_string());
    let mut detail = vec![("phase", fault.phase.as_str())];
    if let Some(value) = os_error.as_deref() {
        detail.push(("os_error", value));
    }
    if fault.phase == InstallPhase::Launch {
        detail.push(("result", shell_result.as_deref().unwrap_or("out_of_range")));
    }
    crate::app_log::record(crate::app_log::Fields {
        level: "error",
        event: "update.install_failed",
        component: "updater",
        outcome: "failure",
        code: Some("UPDATE_INSTALL_FAILED"),
        stage: Some("install"),
        duration_ms: None,
        detail: &detail,
        durable: true,
    });
    CommandError::recoverable("UPDATE_INSTALL_FAILED", "cannot install the update")
        .with_stage("install")
}

fn record_restore(result: &Result<()>) {
    match result {
        Ok(()) => note_update("update.restore", "restore", "success", &[], true),
        Err(error) => {
            crate::app_log::record(crate::app_log::Fields {
                level: "error",
                event: "update.restore",
                component: "updater",
                outcome: "failure",
                code: Some(error.code.as_str()),
                stage: Some("restore"),
                duration_ms: None,
                detail: &[],
                durable: true,
            });
        }
    }
}

#[cfg(any(windows, test))]
fn installer_launch_succeeded(result: isize) -> bool {
    result > 32
}

#[cfg(any(windows, test))]
trait InstallSteps {
    fn create_dir(&mut self) -> std::io::Result<()>;
    fn restrict_dir(&mut self) -> std::io::Result<()>;
    fn create_file(&mut self) -> std::io::Result<()>;
    fn restrict_file(&mut self) -> std::io::Result<()>;
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()>;
    fn sync_all(&mut self) -> std::io::Result<()>;
    fn launch(&mut self) -> std::result::Result<(), isize>;
}

#[cfg(any(windows, test))]
fn execute_install_phases(
    bytes: &[u8],
    steps: &mut impl InstallSteps,
) -> std::result::Result<(), InstallFault> {
    // Release packaging publishes raw NSIS executables, never MSI or ZIP here.
    // `bytes` are the signature-verified payload from Update::download.
    if !bytes.starts_with(b"MZ") {
        return Err(InstallFault {
            phase: InstallPhase::Format,
            os_error: None,
            shell_result: None,
        });
    }
    steps
        .create_dir()
        .map_err(|error| io_fault(InstallPhase::TempDir, &error))?;
    steps
        .restrict_dir()
        .map_err(|error| io_fault(InstallPhase::RestrictDir, &error))?;
    steps
        .create_file()
        .map_err(|error| io_fault(InstallPhase::CreateFile, &error))?;
    steps
        .restrict_file()
        .map_err(|error| io_fault(InstallPhase::RestrictFile, &error))?;
    steps
        .write_all(bytes)
        .map_err(|error| io_fault(InstallPhase::Write, &error))?;
    steps
        .sync_all()
        .map_err(|error| io_fault(InstallPhase::Sync, &error))?;
    steps.launch().map_err(launch_fault)?;
    Ok(())
}

#[cfg(windows)]
struct WindowsInstaller {
    directory: std::path::PathBuf,
    installer: std::path::PathBuf,
    file: Option<std::fs::File>,
}

#[cfg(windows)]
impl InstallSteps for WindowsInstaller {
    fn create_dir(&mut self) -> std::io::Result<()> {
        std::fs::create_dir(&self.directory)
    }

    fn restrict_dir(&mut self) -> std::io::Result<()> {
        crate::windows_security::restrict_private(&self.directory, true)
    }

    fn create_file(&mut self) -> std::io::Result<()> {
        self.file = Some(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&self.installer)?,
        );
        Ok(())
    }

    fn restrict_file(&mut self) -> std::io::Result<()> {
        crate::windows_security::restrict_private(&self.installer, false)
    }

    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        let Some(file) = self.file.as_mut() else {
            return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
        };
        file.write_all(bytes)
    }

    fn sync_all(&mut self) -> std::io::Result<()> {
        let Some(file) = self.file.as_mut() else {
            return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
        };
        file.sync_all()
    }

    fn launch(&mut self) -> std::result::Result<(), isize> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOW};

        self.file.take();
        let path: Vec<_> = self
            .installer
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let launched = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                windows_sys::w!("open"),
                path.as_ptr(),
                windows_sys::w!("/P /R /UPDATE"),
                std::ptr::null(),
                SW_SHOW,
            )
        } as isize;
        if installer_launch_succeeded(launched) {
            Ok(())
        } else {
            Err(launched)
        }
    }
}

#[cfg(windows)]
fn install_windows_verified(bytes: &[u8]) -> std::result::Result<(), InstallFault> {
    let directory = std::env::temp_dir().join(format!("codeasier-update-{}", uuid::Uuid::new_v4()));
    let installer = directory.join("CodeasierRouter-update.exe");
    let mut steps = WindowsInstaller {
        directory,
        installer,
        file: None,
    };
    let result = execute_install_phases(bytes, &mut steps);
    finish_windows_install(&mut steps, result)
}

#[cfg(windows)]
fn finish_windows_install(
    steps: &mut WindowsInstaller,
    result: std::result::Result<(), InstallFault>,
) -> std::result::Result<(), InstallFault> {
    if result.is_err() {
        // Windows can defer deleting an open file until its last handle closes,
        // which would leave the directory nonempty when remove_dir runs.
        drop(steps.file.take());
        let _ = std::fs::remove_file(&steps.installer);
        let _ = std::fs::remove_dir(&steps.directory);
    }
    // On success the external installer still needs this file. Its temporary
    // directory is left to OS temp cleanup, as in the upstream updater.
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn windows_installer_with_open_file() -> WindowsInstaller {
        let directory =
            std::env::temp_dir().join(format!("mtls-update-cleanup-{}", uuid::Uuid::new_v4()));
        let mut steps = WindowsInstaller {
            installer: directory.join("CodeasierRouter-update.exe"),
            directory,
            file: None,
        };
        steps.create_dir().unwrap();
        steps.restrict_dir().unwrap();
        steps.create_file().unwrap();
        steps
    }

    #[cfg(windows)]
    #[test]
    fn windows_prepare_failure_closes_file_before_removing_directory() {
        for phase in [
            InstallPhase::RestrictFile,
            InstallPhase::Write,
            InstallPhase::Sync,
        ] {
            let mut steps = windows_installer_with_open_file();
            if phase != InstallPhase::RestrictFile {
                steps.restrict_file().unwrap();
            }
            if phase == InstallPhase::Sync {
                steps.write_all(b"MZ-verified").unwrap();
            }
            // Inject the failure after real preparation has left the file open.
            let fault = io_fault(phase, &std::io::Error::from_raw_os_error(5));
            assert!(steps.file.is_some());
            assert_eq!(finish_windows_install(&mut steps, Err(fault)), Err(fault));
            assert!(steps.file.is_none(), "{phase:?}");
            assert!(!steps.installer.try_exists().unwrap(), "{phase:?}");
            assert!(!steps.directory.try_exists().unwrap(), "{phase:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_cleanup_failure_preserves_original_install_fault() {
        let mut steps = windows_installer_with_open_file();
        let blocker = steps.directory.join("keep-directory-nonempty");
        std::fs::write(&blocker, b"cleanup failure fixture").unwrap();
        let fault = io_fault(InstallPhase::Write, &std::io::Error::from_raw_os_error(5));

        assert_eq!(finish_windows_install(&mut steps, Err(fault)), Err(fault));
        assert!(steps.file.is_none());
        assert!(!steps.installer.try_exists().unwrap());
        assert!(steps.directory.try_exists().unwrap());
        std::fs::remove_file(blocker).unwrap();
        std::fs::remove_dir(steps.directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_success_keeps_installer_for_external_process() {
        let mut steps = windows_installer_with_open_file();
        steps.write_all(b"MZ-verified").unwrap();
        steps.sync_all().unwrap();
        // Model the handle close in launch, without starting an installer.
        drop(steps.file.take());

        assert_eq!(finish_windows_install(&mut steps, Ok(())), Ok(()));
        assert!(steps.installer.try_exists().unwrap());
        assert!(steps.directory.try_exists().unwrap());
        std::fs::remove_file(steps.installer).unwrap();
        std::fs::remove_dir(steps.directory).unwrap();
    }

    #[test]
    fn windows_shell_launch_errors_are_not_install_success() {
        for code in 0..=32 {
            assert!(!installer_launch_succeeded(code));
        }
        assert!(installer_launch_succeeded(33));
        assert!(installer_launch_succeeded(1024));
    }

    #[test]
    fn deferred_quit_persists_update_success_before_exit() {
        let directory = std::env::temp_dir().join(format!(
            "mtls-update-deferred-quit-{}",
            uuid::Uuid::new_v4()
        ));
        crate::app_log::with_temp(&directory, || {
            persist_success_before_deferred_quit(QuitAction::ExecuteQuit, false);
            persist_success_before_deferred_quit(QuitAction::None, true);
            assert!(crate::app_log::recent(10).is_empty());

            persist_success_before_deferred_quit(QuitAction::ExecuteQuit, true);
            let records = crate::app_log::recent(10);
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].event, "update.succeeded");
            assert_eq!(records[0].component, "updater");
            assert_eq!(records[0].stage.as_deref(), Some("install"));
            assert_eq!(records[0].outcome, "success");
        });
        let _ = std::fs::remove_dir_all(directory);
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
                        Ok(())
                    }
                },
                {
                    let events = events.clone();
                    move || async move {
                        events.lock().unwrap().push("restart");
                        Ok(())
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
                        Ok(())
                    }
                },
                {
                    let events = events.clone();
                    move || async move {
                        events.lock().unwrap().push("restart");
                        Ok(())
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
                            Ok(())
                        }
                    },
                    {
                        let events = events.clone();
                        move || async move {
                            events.lock().unwrap().push("restart");
                            Ok(())
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
                        Ok(())
                    }
                },
                {
                    let events = events.clone();
                    move || async move {
                        events.lock().unwrap().push("restart");
                        Ok(())
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
                            Ok(())
                        }
                    },
                    {
                        let events = events.clone();
                        move || async move {
                            events.lock().unwrap().push("restart");
                            Ok(())
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
                        Ok(())
                    }
                },
                {
                    let events = events.clone();
                    move || async move {
                        events.lock().unwrap().push("restart");
                        Ok(())
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
                            Err(InstallFault {
                                phase: InstallPhase::Write,
                                os_error: Some(5),
                                shell_result: None,
                            })
                        }
                    },
                    {
                        let events = events.clone();
                        move || async move {
                            events.lock().unwrap().push("restart");
                            Ok(())
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
                    || {
                        Err(InstallFault {
                            phase: InstallPhase::Write,
                            os_error: Some(5),
                            shell_result: None,
                        })
                    },
                    || async { Ok(()) },
                )
                .await
                .unwrap_err();
                assert_eq!(error.code, "UPDATE_INSTALL_FAILED");
            });
        });
        assert!(text.contains("UPDATE_INSTALL_FAILED"));
        assert!(text.contains("\"stage\":\"install\""));
        assert!(text.contains("\"phase\":\"write\""));
        assert!(text.contains("\"os_error\":\"5\""));
        assert!(!text.contains("update.restore"));
    }

    struct ScriptedInstall {
        fail_at: Option<InstallPhase>,
        os_error: i32,
        shell_code: isize,
        calls: Vec<&'static str>,
    }

    impl ScriptedInstall {
        fn step(&mut self, name: &'static str, phase: InstallPhase) -> std::io::Result<()> {
            self.calls.push(name);
            if self.fail_at == Some(phase) {
                Err(std::io::Error::from_raw_os_error(self.os_error))
            } else {
                Ok(())
            }
        }
    }

    impl InstallSteps for ScriptedInstall {
        fn create_dir(&mut self) -> std::io::Result<()> {
            self.step("create_dir", InstallPhase::TempDir)
        }

        fn restrict_dir(&mut self) -> std::io::Result<()> {
            self.step("restrict_dir", InstallPhase::RestrictDir)
        }

        fn create_file(&mut self) -> std::io::Result<()> {
            self.step("create_file", InstallPhase::CreateFile)
        }

        fn restrict_file(&mut self) -> std::io::Result<()> {
            self.step("restrict_file", InstallPhase::RestrictFile)
        }

        fn write_all(&mut self, _bytes: &[u8]) -> std::io::Result<()> {
            self.step("write", InstallPhase::Write)
        }

        fn sync_all(&mut self) -> std::io::Result<()> {
            self.step("sync", InstallPhase::Sync)
        }

        fn launch(&mut self) -> std::result::Result<(), isize> {
            self.calls.push("launch");
            if self.fail_at == Some(InstallPhase::Launch) {
                Err(self.shell_code)
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn windows_prepare_and_launch_failures_keep_phase_and_stop() {
        let secret = "sk-supersecretvalue";
        let path = "C:\\Users\\secret\\codeasier-update\\CodeasierRouter-update.exe";
        for (phase, call) in [
            (InstallPhase::TempDir, "create_dir"),
            (InstallPhase::RestrictDir, "restrict_dir"),
            (InstallPhase::CreateFile, "create_file"),
            (InstallPhase::RestrictFile, "restrict_file"),
            (InstallPhase::Write, "write"),
            (InstallPhase::Sync, "sync"),
        ] {
            let mut steps = ScriptedInstall {
                fail_at: Some(phase),
                os_error: 5,
                shell_code: 0,
                calls: Vec::new(),
            };
            let fault = execute_install_phases(b"MZ-verified", &mut steps).unwrap_err();
            assert_eq!(fault.phase, phase, "{call}");
            assert_eq!(fault.os_error, Some(5), "{call}");
            assert_eq!(fault.shell_result, None, "{call}");
            assert_eq!(steps.calls.last().copied(), Some(call));
            assert!(!steps.calls.contains(&"launch"));
        }

        let mut launched = ScriptedInstall {
            fail_at: Some(InstallPhase::Launch),
            os_error: 5,
            shell_code: 5,
            calls: Vec::new(),
        };
        let fault = execute_install_phases(b"MZ-verified", &mut launched).unwrap_err();
        assert_eq!(
            fault,
            InstallFault {
                phase: InstallPhase::Launch,
                os_error: None,
                shell_result: Some(5),
            }
        );
        assert_eq!(launched.calls.len(), 7);

        let mut format = ScriptedInstall {
            fail_at: None,
            os_error: 5,
            shell_code: 0,
            calls: Vec::new(),
        };
        let fault = execute_install_phases(b"PK", &mut format).unwrap_err();
        assert_eq!(fault.phase, InstallPhase::Format);
        assert!(format.calls.is_empty());

        let text = logged(|_| {
            let custom = io_fault(
                InstallPhase::TempDir,
                &std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("{path} {secret}"),
                ),
            );
            assert_eq!(custom.os_error, None);
            record_install_failure(&custom);
            record_install_failure(&io_fault(
                InstallPhase::CreateFile,
                &std::io::Error::from_raw_os_error(5),
            ));
            record_install_failure(&io_fault(
                InstallPhase::Sync,
                &std::io::Error::from_raw_os_error(0),
            ));
            record_install_failure(&launch_fault(0));
            record_install_failure(&launch_fault(31));
            record_install_failure(&launch_fault(-1));
            record_install_failure(&plugin_install_fault(
                &tauri_plugin_updater::Error::Network(format!("{path} {secret}")),
            ));
            record_install_failure(&plugin_install_fault(&tauri_plugin_updater::Error::Io(
                std::io::Error::from_raw_os_error(13),
            )));
        });
        assert!(text.contains("\"phase\":\"temp_dir\""));
        assert!(text.contains("\"phase\":\"create_file\""));
        assert!(text.contains("\"os_error\":\"5\""));
        assert!(text.contains("\"phase\":\"sync\""));
        assert!(text.contains("\"result\":\"0\""));
        assert!(text.contains("\"result\":\"31\""));
        assert!(text.contains("\"result\":\"out_of_range\""));
        assert!(text.contains("\"phase\":\"plugin\""));
        assert!(text.contains("\"os_error\":\"13\""));
        assert!(!text.contains(secret));
        assert!(!text.contains(path));
        assert!(!text.contains("PermissionDenied"));
        assert!(!text.contains("update installer launch failed"));
        let sync_line = text
            .lines()
            .find(|line| line.contains("\"phase\":\"sync\""))
            .unwrap();
        assert!(!sync_line.contains("os_error"));
    }

    #[test]
    fn install_cause_is_recorded_before_restore_outcome() {
        let secret = "sk-supersecretvalue";
        let path = "C:\\Users\\secret\\router.exe";
        let text = logged(|_| {
            runtime().block_on(async {
                use std::sync::atomic::{AtomicUsize, Ordering};
                let reads = AtomicUsize::new(0);
                let error = install_after_router_preflight_with(
                    || {
                        let read = reads.fetch_add(1, Ordering::SeqCst);
                        async move {
                            if read == 0 {
                                Ok(status("desktop_owned", Some("desktop")))
                            } else {
                                Ok(status("absent", None))
                            }
                        }
                    },
                    || async { Ok(status("absent", None)) },
                    || {
                        Err(InstallFault {
                            phase: InstallPhase::RestrictDir,
                            os_error: Some(5),
                            shell_result: None,
                        })
                    },
                    || async {
                        Err(CommandError::new(
                            "MANAGER_FAILED",
                            format!("{path} {secret}"),
                        ))
                    },
                )
                .await
                .unwrap_err();
                assert_eq!(error.code, "UPDATE_INSTALL_FAILED");
                assert_eq!(error.stage.as_deref(), Some("install"));
                assert!(!error.message.contains(path));
            });
        });
        let records: Vec<serde_json::Value> = text
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        let install_at = records
            .iter()
            .position(|record| record["event"] == "update.install_failed")
            .unwrap();
        let restore_at = records
            .iter()
            .position(|record| record["event"] == "update.restore")
            .unwrap();
        assert!(install_at < restore_at);
        assert_eq!(records[install_at]["code"], "UPDATE_INSTALL_FAILED");
        assert_eq!(records[install_at]["detail"]["phase"], "restrict_dir");
        assert_eq!(records[install_at]["detail"]["os_error"], "5");
        assert_eq!(records[restore_at]["outcome"], "failure");
        assert_eq!(records[restore_at]["code"], "MANAGER_FAILED");
        assert_eq!(records[restore_at]["stage"], "restore");
        assert!(records[restore_at].get("detail").is_none());
        assert!(!text.contains(secret));
        assert!(!text.contains(path));

        let restored = logged(|_| {
            runtime().block_on(async {
                use std::sync::atomic::{AtomicUsize, Ordering};
                let reads = AtomicUsize::new(0);
                let error = install_after_router_preflight_with(
                    || {
                        let read = reads.fetch_add(1, Ordering::SeqCst);
                        async move {
                            if read == 0 {
                                Ok(status("legacy_managed", Some("desktop")))
                            } else {
                                Ok(status("absent", None))
                            }
                        }
                    },
                    || async { Ok(status("absent", None)) },
                    || {
                        Err(InstallFault {
                            phase: InstallPhase::Launch,
                            os_error: None,
                            shell_result: Some(2),
                        })
                    },
                    || async { Ok(()) },
                )
                .await
                .unwrap_err();
                assert_eq!(error.code, "UPDATE_INSTALL_FAILED");
            });
        });
        let records: Vec<serde_json::Value> = restored
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        let restore = records
            .iter()
            .find(|record| record["event"] == "update.restore")
            .unwrap();
        assert_eq!(restore["outcome"], "success");
        assert!(restore.get("code").is_none());
        let install = records
            .iter()
            .find(|record| record["event"] == "update.install_failed")
            .unwrap();
        assert_eq!(install["detail"]["phase"], "launch");
        assert_eq!(install["detail"]["result"], "2");
        assert!(install["detail"].get("os_error").is_none());
    }
}
