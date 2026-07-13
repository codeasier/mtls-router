use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use crate::{
    manager::ManagerClient,
    scheduler::PollScheduler,
    types::{NativeLanguage, PollSnapshot, RouterStatus},
};
use serde_json::json;

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    App, AppHandle, Manager, Runtime, WindowEvent,
};
use tauri_plugin_opener::OpenerExt;

const MAIN_WINDOW: &str = "main";
const OPEN_ID: &str = "open";
const START_ID: &str = "start";
const STOP_ID: &str = "stop";
const LOGS_ID: &str = "logs";
const QUIT_ID: &str = "quit";

pub type ActionFuture<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'static>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterSnapshot {
    pub state: String,
    pub owner: Option<String>,
}

impl RouterSnapshot {
    pub fn new(state: impl Into<String>, owner: Option<impl Into<String>>) -> Self {
        Self {
            state: state.into(),
            owner: owner.map(Into::into),
        }
    }
}

#[derive(Clone)]
pub struct Controller {
    status: Arc<dyn Fn() -> ActionFuture<RouterSnapshot> + Send + Sync>,
    start: Arc<dyn Fn() -> ActionFuture<RouterSnapshot> + Send + Sync>,
    stop: Arc<dyn Fn() -> ActionFuture<RouterSnapshot> + Send + Sync>,
    open_logs: Arc<dyn Fn() -> ActionFuture<()> + Send + Sync>,
}

impl Controller {
    pub fn new<S, F, T, L>(status: S, start: F, stop: T, open_logs: L) -> Self
    where
        S: Fn() -> ActionFuture<RouterSnapshot> + Send + Sync + 'static,
        F: Fn() -> ActionFuture<RouterSnapshot> + Send + Sync + 'static,
        T: Fn() -> ActionFuture<RouterSnapshot> + Send + Sync + 'static,
        L: Fn() -> ActionFuture<()> + Send + Sync + 'static,
    {
        Self {
            status: Arc::new(status),
            start: Arc::new(start),
            stop: Arc::new(stop),
            open_logs: Arc::new(open_logs),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Normal,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Presentation {
    severity: Severity,
    status: StatusText,
    can_start: bool,
    can_stop: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusText {
    Checking,
    ManagerUnavailable,
    Stopped,
    Starting,
    Running,
    ExternalRunning,
    UpstreamUnavailable,
    Stopping,
    StartFailed,
    PortOccupied,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NativeStrings {
    window_title: &'static str,
    open: &'static str,
    start: &'static str,
    stop: &'static str,
    logs: &'static str,
    quit: &'static str,
}

fn native_strings(language: NativeLanguage) -> NativeStrings {
    match language {
        NativeLanguage::ZhCn => NativeStrings {
            window_title: "CodeasierRouter 控制台",
            open: "打开控制台",
            start: "启动路由",
            stop: "停止路由",
            logs: "查看日志",
            quit: "退出",
        },
        NativeLanguage::En => NativeStrings {
            window_title: "CodeasierRouter Console",
            open: "Open",
            start: "Start router",
            stop: "Stop router",
            logs: "View logs",
            quit: "Quit",
        },
    }
}

fn status_text(language: NativeLanguage, status: StatusText) -> &'static str {
    match (language, status) {
        (NativeLanguage::ZhCn, StatusText::Checking) => "状态：正在检查",
        (NativeLanguage::ZhCn, StatusText::ManagerUnavailable) => "状态：管理器不可用",
        (NativeLanguage::ZhCn, StatusText::Stopped) => "状态：路由已停止",
        (NativeLanguage::ZhCn, StatusText::Starting) => "状态：路由正在启动",
        (NativeLanguage::ZhCn, StatusText::Running) => "状态：路由正在运行",
        (NativeLanguage::ZhCn, StatusText::ExternalRunning) => "状态：外部路由正在运行",
        (NativeLanguage::ZhCn, StatusText::UpstreamUnavailable) => "状态：上游不可用",
        (NativeLanguage::ZhCn, StatusText::Stopping) => "状态：路由正在停止",
        (NativeLanguage::ZhCn, StatusText::StartFailed) => "状态：路由启动失败",
        (NativeLanguage::ZhCn, StatusText::PortOccupied) => "状态：端口已被占用",
        (NativeLanguage::ZhCn, StatusText::Unknown) => "状态：路由状态未知",
        (NativeLanguage::En, StatusText::Checking) => "Status: checking",
        (NativeLanguage::En, StatusText::ManagerUnavailable) => "Status: manager unavailable",
        (NativeLanguage::En, StatusText::Stopped) => "Status: router stopped",
        (NativeLanguage::En, StatusText::Starting) => "Status: router starting",
        (NativeLanguage::En, StatusText::Running) => "Status: router running",
        (NativeLanguage::En, StatusText::ExternalRunning) => "Status: external router running",
        (NativeLanguage::En, StatusText::UpstreamUnavailable) => "Status: upstream unavailable",
        (NativeLanguage::En, StatusText::Stopping) => "Status: router stopping",
        (NativeLanguage::En, StatusText::StartFailed) => "Status: router start failed",
        (NativeLanguage::En, StatusText::PortOccupied) => "Status: port occupied",
        (NativeLanguage::En, StatusText::Unknown) => "Status: router state unknown",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuitDecision {
    StopThenExit,
    ExitWithoutStop,
}

struct TrayState<R: Runtime> {
    tray: TrayIcon<R>,
    status: MenuItem<R>,
    open: MenuItem<R>,
    start: MenuItem<R>,
    stop: MenuItem<R>,
    logs: MenuItem<R>,
    quit: MenuItem<R>,
    controller: Controller,
    language: Mutex<NativeLanguage>,
    presentation: Mutex<Presentation>,
    operation_active: AtomicBool,
}

pub fn setup(
    app: &App,
    manager: ManagerClient,
    scheduler: PollScheduler,
    log_file: &str,
) -> tauri::Result<()> {
    let controller = manager_controller(
        app.handle().clone(),
        manager,
        scheduler,
        PathBuf::from(log_file),
    );
    let language = NativeLanguage::default();
    let strings = native_strings(language);
    let initial_presentation = Presentation {
        severity: Severity::Warning,
        status: StatusText::Checking,
        can_start: false,
        can_stop: false,
    };
    let initial_status = status_text(language, initial_presentation.status);
    let status = MenuItem::with_id(app, "status", initial_status, false, None::<&str>)?;
    let open = MenuItem::with_id(app, OPEN_ID, strings.open, true, None::<&str>)?;
    let start = MenuItem::with_id(app, START_ID, strings.start, false, None::<&str>)?;
    let stop = MenuItem::with_id(app, STOP_ID, strings.stop, false, None::<&str>)?;
    let logs = MenuItem::with_id(app, LOGS_ID, strings.logs, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ID, strings.quit, true, None::<&str>)?;
    let first_separator = PredefinedMenuItem::separator(app)?;
    let second_separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &first_separator,
            &open,
            &start,
            &stop,
            &logs,
            &second_separator,
            &quit,
        ],
    )?;

    let tray = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip(initial_status)
        .icon(status_icon(Severity::Warning))
        .icon_as_template(cfg!(target_os = "macos"))
        .on_menu_event(handle_menu_event)
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    app.manage(TrayState {
        tray,
        status,
        open,
        start,
        stop,
        logs,
        quit,
        controller,
        language: Mutex::new(language),
        presentation: Mutex::new(initial_presentation),
        operation_active: AtomicBool::new(false),
    });

    Ok(())
}

pub fn handle_window_event<R: Runtime>(window: &tauri::Window<R>, event: &WindowEvent) {
    if should_hide_on_close(
        window.label(),
        matches!(event, WindowEvent::CloseRequested { .. }),
    ) {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = window.hide();
        }
    }
}

pub fn update_status<R: Runtime>(app: &AppHandle<R>, snapshot: &RouterSnapshot) {
    let Some(state) = app.try_state::<TrayState<R>>() else {
        return;
    };
    apply_presentation(&state, presentation(snapshot));
}

pub fn update_poll_snapshot<R: Runtime>(app: &AppHandle<R>, snapshot: &PollSnapshot) {
    if snapshot.status_error.is_some() {
        apply_error(app);
        return;
    }
    let Some(status) = snapshot.status.clone() else {
        return;
    };
    let mut tray_snapshot: RouterSnapshot = status.into();
    if matches!(
        tray_snapshot.state.as_str(),
        "desktop_owned" | "external_compatible"
    ) && (snapshot.health_stale
        || snapshot
            .health
            .as_ref()
            .is_some_and(|health| health.status != "ok"))
    {
        tray_snapshot.state = "degraded".to_owned();
    }
    update_status(app, &tray_snapshot);
}

pub fn set_language<R: Runtime>(app: &AppHandle<R>, language: NativeLanguage) -> tauri::Result<()> {
    let Some(state) = app.try_state::<TrayState<R>>() else {
        return Ok(());
    };
    *state
        .language
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = language;

    let strings = native_strings(language);
    state.open.set_text(strings.open)?;
    state.start.set_text(strings.start)?;
    state.stop.set_text(strings.stop)?;
    state.logs.set_text(strings.logs)?;
    state.quit.set_text(strings.quit)?;
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        window.set_title(strings.window_title)?;
    }

    let presentation = *state
        .presentation
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    apply_presentation_text(&state, presentation, language)?;
    Ok(())
}

fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, event: tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        OPEN_ID => show_main_window(app),
        START_ID => run_lifecycle_action(app.clone(), LifecycleAction::Start),
        STOP_ID => run_lifecycle_action(app.clone(), LifecycleAction::Stop),
        LOGS_ID => open_logs(app.clone()),
        QUIT_ID => request_quit(app.clone()),
        _ => {}
    }
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[derive(Clone, Copy)]
enum LifecycleAction {
    Start,
    Stop,
}

fn run_lifecycle_action<R: Runtime>(app: AppHandle<R>, action: LifecycleAction) {
    let state = app.state::<TrayState<R>>();
    if state.operation_active.swap(true, Ordering::AcqRel) {
        return;
    }
    let controller = state.controller.clone();
    let transient = match action {
        LifecycleAction::Start => RouterSnapshot::new("starting", None::<String>),
        LifecycleAction::Stop => RouterSnapshot::new("stopping", Some("desktop")),
    };
    update_status(&app, &transient);
    tauri::async_runtime::spawn(async move {
        let result = match action {
            LifecycleAction::Start => (controller.start)().await,
            LifecycleAction::Stop => (controller.stop)().await,
        };
        match result {
            Ok(snapshot) => update_status(&app, &snapshot),
            Err(_) => apply_error(&app),
        }
        app.state::<TrayState<R>>()
            .operation_active
            .store(false, Ordering::Release);
    });
}

fn open_logs<R: Runtime>(app: AppHandle<R>) {
    let controller = app.state::<TrayState<R>>().controller.clone();
    tauri::async_runtime::spawn(async move {
        let _ = (controller.open_logs)().await;
    });
}

fn request_quit<R: Runtime>(app: AppHandle<R>) {
    let controller = app.state::<TrayState<R>>().controller.clone();
    tauri::async_runtime::spawn(async move {
        let decision = match (controller.status)().await {
            Ok(snapshot) => quit_decision(Some(&snapshot)),
            Err(_) => quit_decision(None),
        };
        if decision == QuitDecision::StopThenExit {
            let _ = (controller.stop)().await;
        }
        app.exit(0);
    });
}

fn should_hide_on_close(label: &str, close_requested: bool) -> bool {
    label == MAIN_WINDOW && close_requested
}

fn apply_error<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<TrayState<R>>() else {
        return;
    };
    apply_presentation(
        &state,
        Presentation {
            severity: Severity::Error,
            status: StatusText::ManagerUnavailable,
            can_start: false,
            can_stop: false,
        },
    );
}

fn apply_presentation<R: Runtime>(state: &TrayState<R>, value: Presentation) {
    *state
        .presentation
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = value;
    let language = *state
        .language
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _ = apply_presentation_text(state, value, language);
}

fn apply_presentation_text<R: Runtime>(
    state: &TrayState<R>,
    value: Presentation,
    language: NativeLanguage,
) -> tauri::Result<()> {
    let label = status_text(language, value.status);
    state.status.set_text(label)?;
    let _ = state.start.set_enabled(value.can_start);
    let _ = state.stop.set_enabled(value.can_stop);
    state.tray.set_tooltip(Some(label))?;
    state.tray.set_icon(Some(status_icon(value.severity)))?;
    Ok(())
}

fn presentation(snapshot: &RouterSnapshot) -> Presentation {
    let desktop_owned = is_verified_desktop_owned(snapshot);
    match snapshot.state.as_str() {
        "absent" => Presentation {
            severity: Severity::Normal,
            status: StatusText::Stopped,
            can_start: true,
            can_stop: false,
        },
        "starting" => Presentation {
            severity: Severity::Warning,
            status: StatusText::Starting,
            can_start: false,
            can_stop: false,
        },
        "desktop_owned" if desktop_owned => Presentation {
            severity: Severity::Normal,
            status: StatusText::Running,
            can_start: false,
            can_stop: true,
        },
        "external_compatible" => Presentation {
            severity: Severity::Normal,
            status: StatusText::ExternalRunning,
            can_start: false,
            can_stop: false,
        },
        "degraded" => Presentation {
            severity: Severity::Warning,
            status: StatusText::UpstreamUnavailable,
            can_start: false,
            can_stop: desktop_owned,
        },
        "stopping" => Presentation {
            severity: Severity::Warning,
            status: StatusText::Stopping,
            can_start: false,
            can_stop: false,
        },
        "start_failed" => Presentation {
            severity: Severity::Error,
            status: StatusText::StartFailed,
            can_start: true,
            can_stop: false,
        },
        "unknown_occupant" => Presentation {
            severity: Severity::Error,
            status: StatusText::PortOccupied,
            can_start: false,
            can_stop: false,
        },
        _ => Presentation {
            severity: Severity::Error,
            status: StatusText::Unknown,
            can_start: false,
            can_stop: false,
        },
    }
}

fn quit_decision(snapshot: Option<&RouterSnapshot>) -> QuitDecision {
    if snapshot.is_some_and(is_verified_desktop_owned) {
        QuitDecision::StopThenExit
    } else {
        QuitDecision::ExitWithoutStop
    }
}

fn is_verified_desktop_owned(snapshot: &RouterSnapshot) -> bool {
    matches!(snapshot.state.as_str(), "desktop_owned" | "degraded")
        && snapshot.owner.as_deref() == Some("desktop")
}

fn manager_controller(
    app: AppHandle,
    manager: ManagerClient,
    scheduler: PollScheduler,
    log_file: PathBuf,
) -> Controller {
    let status_manager = manager.clone();
    let status_scheduler = scheduler.clone();
    let start_manager = manager.clone();
    let start_scheduler = scheduler.clone();
    let stop_manager = manager;
    let stop_scheduler = scheduler;

    Controller::new(
        move || {
            let manager = status_manager.clone();
            let scheduler = status_scheduler.clone();
            Box::pin(async move {
                let status: RouterStatus = manager
                    .call("router.status", json!({}))
                    .await
                    .map_err(|error| error.to_string())?;
                scheduler.set_status(status.clone()).await;
                Ok(status.into())
            })
        },
        move || {
            let manager = start_manager.clone();
            let scheduler = start_scheduler.clone();
            Box::pin(async move {
                let status = crate::orchestration::start(&manager, &scheduler)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(status.into())
            })
        },
        move || {
            let manager = stop_manager.clone();
            let scheduler = stop_scheduler.clone();
            Box::pin(async move {
                let status = crate::orchestration::stop(&manager, &scheduler)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(status.into())
            })
        },
        move || {
            let app = app.clone();
            let log_file = log_file.clone();
            Box::pin(async move {
                let directory = log_file
                    .parent()
                    .ok_or_else(|| "log location is unavailable".to_string())?;
                app.opener()
                    .open_path(directory.to_string_lossy(), None::<&str>)
                    .map_err(|_| "cannot open the log location".to_string())
            })
        },
    )
}

impl From<RouterStatus> for RouterSnapshot {
    fn from(status: RouterStatus) -> Self {
        Self {
            state: status.state,
            owner: status.owner,
        }
    }
}

fn status_icon(severity: Severity) -> Image<'static> {
    const SIZE: usize = 20;
    let mut rgba = vec![0; SIZE * SIZE * 4];
    let mut pixel = |x: usize, y: usize| {
        let offset = (y * SIZE + x) * 4;
        rgba[offset..offset + 4].copy_from_slice(&[0, 0, 0, 255]);
    };

    // Compact CR monogram that remains legible as a macOS template image.
    for y in 3..15 {
        for x in 2..5 {
            pixel(x, y);
        }
    }
    for x in 4..9 {
        pixel(x, 3);
        pixel(x, 14);
    }
    for y in 4..7 {
        pixel(8, y);
    }
    for y in 3..15 {
        pixel(11, y);
    }
    for x in 12..16 {
        pixel(x, 3);
        pixel(x, 8);
    }
    for y in 4..8 {
        pixel(16, y);
    }
    for step in 0..6 {
        pixel(12 + step.min(3), 9 + step);
    }

    match severity {
        Severity::Normal => {
            for y in 16..19 {
                for x in 15..18 {
                    pixel(x, y);
                }
            }
        }
        Severity::Warning => {
            for y in 15..19 {
                pixel(16, y);
            }
            pixel(15, 18);
            pixel(17, 18);
        }
        Severity::Error => {
            for step in 0..4 {
                pixel(14 + step, 15 + step);
                pixel(17 - step, 15 + step);
            }
        }
    }
    Image::new_owned(rgba, SIZE as u32, SIZE as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(state: &str, owner: Option<&str>) -> RouterSnapshot {
        RouterSnapshot::new(state, owner)
    }

    #[test]
    fn only_verified_desktop_ownership_enables_stop() {
        assert!(presentation(&snapshot("desktop_owned", Some("desktop"))).can_stop);
        assert!(presentation(&snapshot("degraded", Some("desktop"))).can_stop);
        assert!(!presentation(&snapshot("desktop_owned", None)).can_stop);
        assert!(!presentation(&snapshot("external_compatible", Some("cli"))).can_stop);
    }

    #[test]
    fn start_is_available_only_for_safe_stopped_or_failed_states() {
        assert!(presentation(&snapshot("absent", None)).can_start);
        assert!(presentation(&snapshot("start_failed", None)).can_start);
        assert!(!presentation(&snapshot("unknown_occupant", None)).can_start);
        assert!(!presentation(&snapshot("stale", Some("desktop"))).can_start);
    }

    #[test]
    fn severity_distinguishes_normal_warning_and_error_states() {
        assert_eq!(
            presentation(&snapshot("desktop_owned", Some("desktop"))).severity,
            Severity::Normal
        );
        assert_eq!(
            presentation(&snapshot("degraded", Some("desktop"))).severity,
            Severity::Warning
        );
        assert_eq!(
            presentation(&snapshot("unknown_occupant", None)).severity,
            Severity::Error
        );
    }

    #[test]
    fn quit_stops_only_a_verified_desktop_owned_router() {
        let desktop = snapshot("desktop_owned", Some("desktop"));
        let external = snapshot("external_compatible", Some("cli"));
        let degraded_desktop = snapshot("degraded", Some("desktop"));
        let spoofed = snapshot("desktop_owned", Some("cli"));

        assert_eq!(quit_decision(Some(&desktop)), QuitDecision::StopThenExit);
        assert_eq!(
            quit_decision(Some(&degraded_desktop)),
            QuitDecision::StopThenExit
        );
        assert_eq!(
            quit_decision(Some(&external)),
            QuitDecision::ExitWithoutStop
        );
        assert_eq!(quit_decision(Some(&spoofed)), QuitDecision::ExitWithoutStop);
        assert_eq!(quit_decision(None), QuitDecision::ExitWithoutStop);
    }

    #[test]
    fn status_icons_are_opaque_only_inside_the_indicator() {
        let icon = status_icon(Severity::Normal);
        assert_eq!(icon.width(), 20);
        assert_eq!(icon.height(), 20);
        assert_eq!(icon.rgba()[3], 0);
        let monogram = (8 * 20 + 11) * 4;
        assert_eq!(&icon.rgba()[monogram..monogram + 4], &[0, 0, 0, 255]);
    }

    #[test]
    fn only_main_window_close_requests_are_hidden() {
        assert!(should_hide_on_close("main", true));
        assert!(!should_hide_on_close("other", true));
        assert!(!should_hide_on_close("main", false));
    }

    #[test]
    fn native_strings_cover_chinese_and_english() {
        let chinese = native_strings(NativeLanguage::ZhCn);
        assert_eq!(chinese.window_title, "CodeasierRouter 控制台");
        assert_eq!(chinese.start, "启动路由");
        assert_eq!(
            status_text(NativeLanguage::ZhCn, StatusText::UpstreamUnavailable),
            "状态：上游不可用"
        );

        let english = native_strings(NativeLanguage::En);
        assert_eq!(english.window_title, "CodeasierRouter Console");
        assert_eq!(english.start, "Start router");
        assert_eq!(
            status_text(NativeLanguage::En, StatusText::UpstreamUnavailable),
            "Status: upstream unavailable"
        );
    }

    #[test]
    fn missing_native_preference_uses_chinese_strings() {
        let language = NativeLanguage::default();
        assert_eq!(native_strings(language).open, "打开控制台");
        assert_eq!(
            status_text(language, StatusText::Checking),
            "状态：正在检查"
        );
    }
}
