use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use tauri::{App, AppHandle, Manager, Runtime};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

const INITIALIZED_MARKER: &str = "autostart-initialized";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirstLaunchAction {
    None,
    MarkInitialized,
    EnableAndMark,
}

fn first_launch_action(marker_exists: bool, enabled: bool) -> FirstLaunchAction {
    match (marker_exists, enabled) {
        (true, _) => FirstLaunchAction::None,
        (false, true) => FirstLaunchAction::MarkInitialized,
        (false, false) => FirstLaunchAction::EnableAndMark,
    }
}

pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None)
}

pub fn initialize_default(app: &App) -> Result<(), String> {
    let marker = marker_path(app.app_handle())?;
    let manager = app.autolaunch();
    let enabled = manager
        .is_enabled()
        .map_err(|_| "could not read current-user autostart state".to_string())?;

    match first_launch_action(marker.exists(), enabled) {
        FirstLaunchAction::None => Ok(()),
        FirstLaunchAction::MarkInitialized => write_marker(&marker),
        FirstLaunchAction::EnableAndMark => {
            manager
                .enable()
                .map_err(|_| "could not enable current-user autostart".to_string())?;
            write_marker(&marker)
        }
    }
}

#[tauri::command]
pub fn autostart_get(app: AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|_| "could not read current-user autostart state".to_string())
}

#[tauri::command(rename = "autostart_set")]
pub fn autostart_set_immediate(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    if enabled {
        manager
            .enable()
            .map_err(|_| "could not enable current-user autostart".to_string())?;
    } else {
        manager
            .disable()
            .map_err(|_| "could not disable current-user autostart".to_string())?;
    }

    let actual = manager
        .is_enabled()
        .map_err(|_| "could not verify current-user autostart state".to_string())?;
    if actual != enabled {
        return Err("current-user autostart state did not change".to_string());
    }
    Ok(actual)
}

fn prepare_uninstall_sequence(
    disable: impl FnOnce() -> Result<(), ()>,
    is_enabled: impl FnOnce() -> Result<bool, ()>,
    exit: impl FnOnce(),
) -> Result<(), String> {
    disable().map_err(|_| "could not disable current-user autostart".to_string())?;
    let enabled =
        is_enabled().map_err(|_| "could not verify current-user autostart state".to_string())?;
    if enabled {
        return Err("current-user autostart remains enabled".to_string());
    }
    exit();
    Ok(())
}

#[tauri::command]
pub fn prepare_for_uninstall(app: AppHandle) -> Result<(), String> {
    if cfg!(windows) {
        return Err("the Windows installer handles autostart removal".to_string());
    }
    let manager = app.autolaunch();
    prepare_uninstall_sequence(
        || manager.disable().map_err(|_| ()),
        || manager.is_enabled().map_err(|_| ()),
        || app.exit(0),
    )
}

fn marker_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(INITIALIZED_MARKER))
        .map_err(|_| "could not resolve the application configuration directory".to_string())
}

fn write_marker(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "invalid autostart marker path".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|_| "could not create the application configuration directory".to_string())?;

    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => file
            .write_all(b"1\n")
            .map_err(|_| "could not persist autostart initialization".to_string()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(_) => Err("could not persist autostart initialization".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_launch_enables_autostart_before_marking_initialized() {
        assert_eq!(
            first_launch_action(false, false),
            FirstLaunchAction::EnableAndMark
        );
    }

    #[test]
    fn first_launch_preserves_an_existing_enabled_registration() {
        assert_eq!(
            first_launch_action(false, true),
            FirstLaunchAction::MarkInitialized
        );
    }

    #[test]
    fn later_launch_preserves_an_explicitly_disabled_registration() {
        assert_eq!(first_launch_action(true, false), FirstLaunchAction::None);
    }

    #[test]
    fn marker_creation_is_idempotent() {
        let directory =
            std::env::temp_dir().join(format!("mtls-router-autostart-test-{}", std::process::id()));
        let marker = directory.join(INITIALIZED_MARKER);
        let _ = fs::remove_dir_all(&directory);

        write_marker(&marker).expect("first marker write");
        write_marker(&marker).expect("second marker write");
        assert_eq!(fs::read(&marker).expect("read marker"), b"1\n");

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn uninstall_preparation_disables_autostart_before_exit() {
        let steps = std::cell::RefCell::new(Vec::new());
        prepare_uninstall_sequence(
            || {
                steps.borrow_mut().push("disable");
                Ok(())
            },
            || {
                steps.borrow_mut().push("verify");
                Ok(false)
            },
            || steps.borrow_mut().push("exit"),
        )
        .expect("prepare uninstall");
        assert_eq!(*steps.borrow(), ["disable", "verify", "exit"]);
    }

    #[test]
    fn uninstall_preparation_does_not_exit_when_autostart_remains_enabled() {
        let exited = std::cell::Cell::new(false);
        let result = prepare_uninstall_sequence(|| Ok(()), || Ok(true), || exited.set(true));
        assert!(result.is_err());
        assert!(!exited.get());
    }
}
