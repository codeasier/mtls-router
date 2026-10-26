//! Local desktop application events. Router access logs stay in their session files.
//!
//! Lines are JSON and only carry a closed field set. Writes never return errors to callers.
//! Update failures and quit use the durable path, which flushes before it returns.

use crate::redaction::sanitize_text;
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub const FILE_NAME: &str = "app.log";
pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const CHANNEL_CAP: usize = 256;
const MAX_LINE_BYTES: usize = 2048;

static GLOBAL: RwLock<Option<Arc<AppLog>>> = RwLock::new(None);
static RECENT: Mutex<Vec<(String, Instant)>> = Mutex::new(Vec::new());
static OPERATION: Mutex<Option<String>> = Mutex::new(None);

#[cfg(test)]
thread_local! {
    static LOCAL_LOG: std::cell::RefCell<Option<Arc<AppLog>>> = std::cell::RefCell::new(None);
}

pub struct OperationGuard;

impl Drop for OperationGuard {
    fn drop(&mut self) {
        if let Ok(mut slot) = OPERATION.lock() {
            *slot = None;
        }
    }
}

pub fn begin_operation() -> OperationGuard {
    if let Ok(mut slot) = OPERATION.lock() {
        *slot = Some(Uuid::new_v4().to_string());
    }
    OperationGuard
}

pub struct Fields<'a> {
    pub level: &'static str,
    pub event: &'static str,
    pub component: &'static str,
    pub outcome: &'static str,
    pub code: Option<&'a str>,
    pub stage: Option<&'a str>,
    pub duration_ms: Option<u64>,
    pub detail: &'a [(&'static str, &'a str)],
    pub durable: bool,
}

pub fn record(fields: Fields<'_>) {
    let Some(log) = current() else {
        return;
    };
    let Some(line) = render_line(&fields, &log.session_id) else {
        return;
    };
    if suppress(&fields) {
        return;
    }
    log.enqueue(line, fields.durable);
}

pub fn record_command_failure(error: &crate::error::CommandError) {
    if error.code.starts_with("UPDATE_") {
        return;
    }
    record(Fields {
        level: "error",
        event: "command.failed",
        component: "command",
        outcome: "failure",
        code: Some(error.code.as_str()),
        stage: error.stage.as_deref(),
        duration_ms: None,
        detail: &[],
        durable: false,
    });
}

pub fn flush() {
    if let Some(log) = current() {
        log.flush();
    }
}

pub fn install(log_directory: &Path) {
    match AppLog::open(log_directory, MAX_FILE_BYTES) {
        Ok(log) => {
            if let Ok(mut slot) = GLOBAL.write() {
                *slot = Some(Arc::new(log));
            }
        }
        Err(_) => {
            if let Ok(mut slot) = GLOBAL.write() {
                *slot = None;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AppLogRecord {
    pub ts: String,
    pub level: String,
    pub event: String,
    pub component: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<BTreeMap<String, String>>,
}

pub fn recent(limit: usize) -> Vec<AppLogRecord> {
    flush();
    let Some(log) = current() else {
        return Vec::new();
    };
    read_recent(&log.directory, limit)
}

pub(crate) fn read_recent(directory: &Path, limit: usize) -> Vec<AppLogRecord> {
    if limit == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for name in ["app.log.2", "app.log.1", FILE_NAME] {
        let path = directory.join(name);
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            if !line.is_empty() {
                lines.push(line.to_owned());
            }
        }
    }
    let start = lines.len().saturating_sub(limit);
    lines[start..]
        .iter()
        .filter_map(|line| parse_record(line))
        .collect()
}

struct AppLog {
    directory: PathBuf,
    session_id: String,
    tx: std::sync::mpsc::SyncSender<Job>,
}

enum Job {
    Line {
        line: String,
        durable: bool,
        ack: Option<std::sync::mpsc::SyncSender<()>>,
    },
    Flush(std::sync::mpsc::SyncSender<()>),
    Shutdown(std::sync::mpsc::SyncSender<()>),
}

impl AppLog {
    fn open(directory: &Path, limit: u64) -> io::Result<Self> {
        fs::create_dir_all(directory)?;
        let writer = LogWriter::create(directory, limit)?;
        let (tx, rx) = std::sync::mpsc::sync_channel(CHANNEL_CAP);
        thread::Builder::new()
            .name("desktop-app-log".to_owned())
            .spawn(move || worker(writer, rx))
            .map_err(io::Error::other)?;
        Ok(Self {
            directory: directory.to_path_buf(),
            session_id: Uuid::new_v4().to_string(),
            tx,
        })
    }

    fn enqueue(&self, line: String, durable: bool) {
        if durable {
            let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel(1);
            if self
                .tx
                .send(Job::Line {
                    line,
                    durable: true,
                    ack: Some(ack_tx),
                })
                .is_ok()
            {
                let _ = ack_rx.recv_timeout(Duration::from_secs(2));
            }
            return;
        }
        let _ = self.tx.try_send(Job::Line {
            line,
            durable: false,
            ack: None,
        });
    }

    fn flush(&self) {
        let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel(1);
        if self.tx.send(Job::Flush(ack_tx)).is_ok() {
            let _ = ack_rx.recv_timeout(Duration::from_secs(2));
        }
    }
}

impl Drop for AppLog {
    fn drop(&mut self) {
        let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel(1);
        if self.tx.send(Job::Shutdown(ack_tx)).is_ok() {
            let _ = ack_rx.recv_timeout(Duration::from_secs(1));
        }
    }
}

fn worker(mut writer: LogWriter, rx: std::sync::mpsc::Receiver<Job>) {
    while let Ok(job) = rx.recv() {
        match job {
            Job::Line { line, durable, ack } => {
                let _ = writer.write_line(&line);
                if durable {
                    let _ = writer.sync();
                }
                if let Some(ack) = ack {
                    let _ = ack.send(());
                }
            }
            Job::Flush(ack) => {
                let _ = writer.sync();
                let _ = ack.send(());
            }
            Job::Shutdown(ack) => {
                let _ = writer.sync();
                let _ = ack.send(());
                break;
            }
        }
    }
}

pub(crate) struct LogWriter {
    directory: PathBuf,
    file: Option<File>,
    size: u64,
    limit: u64,
}

impl LogWriter {
    pub(crate) fn create(directory: &Path, limit: u64) -> io::Result<Self> {
        fs::create_dir_all(directory)?;
        let path = directory.join(FILE_NAME);
        let file = open_append(&path)?;
        let size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        Ok(Self {
            directory: directory.to_path_buf(),
            file: Some(file),
            size,
            limit: limit.max(1),
        })
    }

    pub(crate) fn write_line(&mut self, line: &str) -> io::Result<()> {
        if !line.ends_with('\n') || line.len() > MAX_LINE_BYTES {
            return Ok(());
        }
        let bytes = line.len() as u64;
        if self.size > 0 && self.size.saturating_add(bytes) > self.limit {
            self.rotate()?;
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("app log is closed"))?;
        file.write_all(line.as_bytes())?;
        self.size = self.size.saturating_add(bytes);
        Ok(())
    }

    fn sync(&mut self) -> io::Result<()> {
        if let Some(file) = self.file.as_mut() {
            file.sync_all()?;
        }
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        if let Some(file) = self.file.as_mut() {
            let _ = file.sync_all();
        }
        self.file.take();
        let primary = self.directory.join(FILE_NAME);
        let first = self.directory.join("app.log.1");
        let second = self.directory.join("app.log.2");
        let _ = fs::remove_file(&second);
        if first.exists() {
            let _ = fs::rename(&first, &second);
        }
        if primary.exists() {
            fs::rename(&primary, &first)?;
        }
        self.file = Some(open_append(&primary)?);
        self.size = 0;
        Ok(())
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(path)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    #[cfg(windows)]
    {
        let _ = crate::windows_security::restrict_private(path, false);
    }
    Ok(file)
}

fn current() -> Option<Arc<AppLog>> {
    #[cfg(test)]
    if let Some(log) = LOCAL_LOG.with(|slot| slot.borrow().clone()) {
        return Some(log);
    }
    GLOBAL.read().ok()?.clone()
}

fn current_operation() -> Option<String> {
    OPERATION.lock().ok()?.clone()
}

fn suppress(fields: &Fields<'_>) -> bool {
    let Ok(mut recent) = RECENT.lock() else {
        return false;
    };
    let now = Instant::now();
    recent.retain(|(_, at)| now.saturating_duration_since(*at) < Duration::from_secs(30));
    let code = fields
        .code
        .filter(|code| closed_code(code))
        .unwrap_or_default();
    let stage = fields
        .stage
        .filter(|stage| closed_stage(stage))
        .unwrap_or("");
    if fields.event == "command.failed" && !code.is_empty() {
        let key = format!("code|{code}|{stage}");
        if recent.iter().any(|(seen, at)| {
            seen == &key && now.saturating_duration_since(*at) < Duration::from_secs(30)
        }) {
            return true;
        }
        recent.push((key, now));
        return false;
    }
    if !code.is_empty() {
        recent.push((format!("code|{code}|{stage}"), now));
    }
    if fields.event.starts_with("app.window_") {
        let key = fields.event.to_owned();
        if recent.iter().any(|(seen, at)| {
            seen == &key && now.saturating_duration_since(*at) < Duration::from_secs(2)
        }) {
            return true;
        }
        recent.push((key, now));
    }
    false
}

fn render_line(fields: &Fields<'_>, session_id: &str) -> Option<String> {
    if !matches!(fields.level, "error" | "warn" | "info" | "debug")
        || !matches!(
            fields.outcome,
            "success" | "failure" | "denied" | "cancelled"
        )
        || !known_event(fields.event)
        || !known_component(fields.component)
    {
        return None;
    }
    let mut object = Map::new();
    object.insert(
        "ts".into(),
        Value::String(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
    );
    object.insert("level".into(), Value::String(fields.level.to_owned()));
    object.insert("event".into(), Value::String(fields.event.to_owned()));
    object.insert(
        "component".into(),
        Value::String(fields.component.to_owned()),
    );
    if let Some(code) = fields.code.filter(|code| closed_code(code)) {
        object.insert("code".into(), Value::String(code.to_owned()));
    } else if fields.code.is_some() {
        object.insert("code".into(), Value::String("UNCLASSIFIED".to_owned()));
    }
    if let Some(stage) = fields.stage.filter(|stage| closed_stage(stage)) {
        object.insert("stage".into(), Value::String(stage.to_owned()));
    }
    object.insert("session_id".into(), Value::String(session_id.to_owned()));
    if let Some(operation) = current_operation().filter(|value| Uuid::parse_str(value).is_ok()) {
        object.insert("operation_id".into(), Value::String(operation));
    }
    object.insert(
        "app_version".into(),
        Value::String(env!("CARGO_PKG_VERSION").to_owned()),
    );
    object.insert("os".into(), Value::String(std::env::consts::OS.to_owned()));
    object.insert(
        "arch".into(),
        Value::String(std::env::consts::ARCH.to_owned()),
    );
    object.insert("outcome".into(), Value::String(fields.outcome.to_owned()));
    if let Some(duration) = fields.duration_ms.filter(|value| *value <= 86_400_000) {
        object.insert("duration_ms".into(), Value::from(duration));
    }
    let detail = sanitize_detail(fields.detail);
    if !detail.is_empty() {
        object.insert("detail".into(), Value::Object(detail));
    }
    let mut line = serde_json::to_string(&Value::Object(object)).ok()?;
    line = sanitize_text(&line);
    if line.contains('\n')
        || line.contains('\r')
        || line.len() + 1 > MAX_LINE_BYTES
        || contains_secret_marker(&line)
    {
        return None;
    }
    line.push('\n');
    Some(line)
}

fn sanitize_detail(detail: &[(&str, &str)]) -> Map<String, Value> {
    let mut object = Map::new();
    for (key, value) in detail {
        if !detail_key_allowed(key) || !detail_value_allowed(key, value) {
            continue;
        }
        let cleaned = sanitize_text(value);
        if contains_secret_marker(&cleaned) || cleaned != *value {
            continue;
        }
        object.insert((*key).to_owned(), Value::String((*value).to_owned()));
    }
    object
}

fn parse_record(line: &str) -> Option<AppLogRecord> {
    let value: Value = serde_json::from_str(line).ok()?;
    let object = value.as_object()?;
    let ts = object.get("ts")?.as_str()?.to_owned();
    if chrono::DateTime::parse_from_rfc3339(&ts).is_err() {
        return None;
    }
    let level = object.get("level")?.as_str()?;
    let event = object.get("event")?.as_str()?;
    let component = object.get("component")?.as_str()?;
    let outcome = object.get("outcome")?.as_str()?;
    if !matches!(level, "error" | "warn" | "info" | "debug")
        || !known_event(event)
        || !known_component(component)
        || !matches!(outcome, "success" | "failure" | "denied" | "cancelled")
    {
        return None;
    }
    let code = match object.get("code") {
        None => None,
        Some(value) => {
            let code = value.as_str()?;
            if code != "UNCLASSIFIED" && !closed_code(code) {
                return None;
            }
            Some(code.to_owned())
        }
    };
    let stage = match object.get("stage") {
        None => None,
        Some(value) => {
            let stage = value.as_str()?;
            if !closed_stage(stage) {
                return None;
            }
            Some(stage.to_owned())
        }
    };
    let duration_ms = match object.get("duration_ms") {
        None => None,
        Some(value) => Some(value.as_u64()?),
    };
    let detail = match object.get("detail") {
        None => None,
        Some(value) => {
            let mut kept = BTreeMap::new();
            for (key, item) in value.as_object()? {
                let text = item.as_str()?;
                if detail_key_allowed(key) && detail_value_allowed(key, text) {
                    kept.insert(key.clone(), text.to_owned());
                }
            }
            if kept.is_empty() {
                None
            } else {
                Some(kept)
            }
        }
    };
    Some(AppLogRecord {
        ts,
        level: level.to_owned(),
        event: event.to_owned(),
        component: component.to_owned(),
        code,
        stage,
        outcome: outcome.to_owned(),
        duration_ms,
        detail,
    })
}

fn known_event(event: &str) -> bool {
    matches!(
        event,
        "app.starting"
            | "app.ready"
            | "app.instance_activated"
            | "app.window_shown"
            | "app.window_hidden"
            | "app.quit_requested"
            | "app.quit_cancelled"
            | "app.quit"
            | "update.check_started"
            | "update.check_finished"
            | "update.available"
            | "update.install_requested"
            | "update.download_started"
            | "update.download_milestone"
            | "update.download_finished"
            | "update.verify_failed"
            | "update.preflight"
            | "update.install_failed"
            | "update.restart_blocked"
            | "update.succeeded"
            | "router.start_requested"
            | "router.start_finished"
            | "router.stop_requested"
            | "router.stop_finished"
            | "command.failed"
            | "settings.changed"
            | "credential.changed"
            | "usage.fetched"
            | "identity.changed"
            | "agent.finished"
            | "workbench.finished"
    )
}

fn known_component(component: &str) -> bool {
    matches!(
        component,
        "lifecycle"
            | "updater"
            | "router"
            | "command"
            | "settings"
            | "credential"
            | "identity"
            | "agent"
            | "workbench"
    )
}

fn closed_code(code: &str) -> bool {
    let mut chars = code.chars();
    matches!(chars.next(), Some(ch) if ch.is_ascii_uppercase())
        && chars.all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
        && code.len() <= 64
}

fn closed_stage(stage: &str) -> bool {
    let mut chars = stage.chars();
    matches!(chars.next(), Some(ch) if ch.is_ascii_lowercase())
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        && stage.len() <= 64
}

fn detail_key_allowed(key: &str) -> bool {
    matches!(
        key,
        "state"
            | "owner"
            | "listen_refusal"
            | "os_error"
            | "downloaded"
            | "total"
            | "count"
            | "bytes"
            | "result"
            | "phase"
            | "agent"
            | "language"
            | "enabled"
            | "version"
            | "period"
    )
}

fn detail_value_allowed(key: &str, value: &str) -> bool {
    match key {
        "enabled" => value == "true" || value == "false",
        "language" => matches!(value, "en" | "zh-CN"),
        "period" => matches!(
            value,
            "1h" | "12h" | "24h" | "7d" | "30d" | "today" | "this_week" | "this_month"
        ),
        "agent" => matches!(value, "claude" | "opencode" | "codex"),
        "version" => {
            value.split('.').count() == 3
                && value
                    .split('.')
                    .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        }
        "os_error" | "downloaded" | "total" | "count" | "bytes" => {
            !value.is_empty()
                && value.len() <= 20
                && value.bytes().all(|byte| byte.is_ascii_digit())
        }
        _ => {
            !value.is_empty()
                && value.len() <= 32
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        }
    }
}

fn contains_secret_marker(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("sk-")
        || lower.contains("begin private")
        || lower.contains("begin certificate")
        || lower.contains("bearer ")
}

#[cfg(test)]
pub fn with_temp<R>(directory: &Path, body: impl FnOnce() -> R) -> R {
    // The override is thread-local. Swapping GLOBAL lets parallel tests append
    // their events to this directory while the body is running.
    struct ClearLocal;
    impl Drop for ClearLocal {
        fn drop(&mut self) {
            LOCAL_LOG.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }

    let opened = AppLog::open(directory, MAX_FILE_BYTES).ok().map(Arc::new);
    LOCAL_LOG.with(|slot| {
        *slot.borrow_mut() = opened.clone();
    });
    let _clear = ClearLocal;
    let result = body();
    drop(_clear);
    drop(opened);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields<'a>(
        event: &'static str,
        component: &'static str,
        code: Option<&'a str>,
        detail: &'a [(&'static str, &'a str)],
    ) -> Fields<'a> {
        Fields {
            level: "error",
            event,
            component,
            outcome: "failure",
            code,
            stage: Some("download"),
            duration_ms: Some(12),
            detail,
            durable: true,
        }
    }

    #[test]
    fn schema_is_closed_json_with_timezone() {
        let line = render_line(
            &fields(
                "update.download_finished",
                "updater",
                Some("UPDATE_DOWNLOAD_FAILED"),
                &[],
            ),
            "11111111-1111-1111-1111-111111111111",
        )
        .unwrap();
        let value: Value = serde_json::from_str(line.trim_end()).unwrap();
        for key in [
            "ts",
            "level",
            "event",
            "component",
            "code",
            "stage",
            "session_id",
            "app_version",
            "os",
            "arch",
            "outcome",
            "duration_ms",
        ] {
            assert!(value.get(key).is_some(), "{key}");
        }
        let ts = value["ts"].as_str().unwrap();
        assert!(ts.contains('T'));
        assert!(chrono::DateTime::parse_from_rfc3339(ts).is_ok());
        assert_eq!(value["code"], "UPDATE_DOWNLOAD_FAILED");
        assert_eq!(value["event"], "update.download_finished");
    }

    #[test]
    fn secret_detail_never_reaches_the_line() {
        let secret = "sk-supersecretvalue";
        let line = render_line(
            &fields(
                "update.verify_failed",
                "updater",
                Some("UPDATE_SIGNATURE_FAILED"),
                &[("state", secret), ("owner", "desktop")],
            ),
            "22222222-2222-2222-2222-222222222222",
        )
        .unwrap();
        assert!(!line.contains(secret));
        assert!(line.contains("UPDATE_SIGNATURE_FAILED"));
        assert!(line.contains("\"owner\":\"desktop\""));
        assert!(!line.contains("state"));
    }

    #[test]
    fn raw_secret_line_is_dropped() {
        let poisoned =
            format!("{{\"event\":\"update.verify_failed\",\"token\":\"sk-supersecretvalue\"}}\n");
        let cleaned = sanitize_text(&poisoned);
        assert!(!cleaned.contains("sk-supersecretvalue"));
        let rendered = render_line(
            &fields(
                "command.failed",
                "command",
                Some("sk-supersecretvalue"),
                &[("result", "sk-supersecretvalue")],
            ),
            "33333333-3333-3333-3333-333333333333",
        )
        .unwrap();
        assert!(!rendered.contains("sk-supersecretvalue"));
        assert!(rendered.contains("UNCLASSIFIED"));
    }

    #[test]
    fn rotation_keeps_three_files_under_the_byte_cap() {
        let directory =
            std::env::temp_dir().join(format!("mtls-app-log-rotate-{}", Uuid::new_v4()));
        let mut writer = LogWriter::create(&directory, 80).unwrap();
        for index in 0..30 {
            let line = format!("{{\"n\":{index:02}}}\n");
            writer.write_line(&line).unwrap();
        }
        writer.sync().unwrap();
        let primary = fs::metadata(directory.join(FILE_NAME)).unwrap().len();
        assert!(primary <= 80, "{primary}");
        assert!(directory.join("app.log.1").is_file());
        assert!(directory.join("app.log.2").is_file());
        assert!(!directory.join("app.log.3").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(directory.join(FILE_NAME))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn temp_sink_ignores_records_from_other_threads() {
        let directory =
            std::env::temp_dir().join(format!("mtls-app-log-isolate-{}", Uuid::new_v4()));
        let started = std::sync::Arc::new(std::sync::Barrier::new(2));
        let release = std::sync::Arc::new(std::sync::Barrier::new(2));
        let started_writer = std::sync::Arc::clone(&started);
        let release_writer = std::sync::Arc::clone(&release);
        let writer = thread::spawn(move || {
            started_writer.wait();
            record(fields(
                "update.preflight",
                "updater",
                Some("UPDATE_BLOCKED_ROUTER_STATE"),
                &[],
            ));
            release_writer.wait();
        });
        with_temp(&directory, || {
            started.wait();
            release.wait();
            writer.join().unwrap();
            record(fields(
                "update.download_finished",
                "updater",
                Some("UPDATE_DOWNLOAD_FAILED"),
                &[],
            ));
            let records = recent(10);
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].code.as_deref(), Some("UPDATE_DOWNLOAD_FAILED"));
            assert!(records
                .iter()
                .all(|record| record.event != "update.preflight"));
        });
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn durable_update_failure_is_on_disk_before_return() {
        let directory =
            std::env::temp_dir().join(format!("mtls-app-log-durable-{}", Uuid::new_v4()));
        with_temp(&directory, || {
            record(fields(
                "update.download_finished",
                "updater",
                Some("UPDATE_DOWNLOAD_FAILED"),
                &[],
            ));
            let text = fs::read_to_string(directory.join(FILE_NAME)).unwrap();
            assert!(text.contains("UPDATE_DOWNLOAD_FAILED"));
            let records = read_recent(&directory, 10);
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].code.as_deref(), Some("UPDATE_DOWNLOAD_FAILED"));
            assert_eq!(records[0].component, "updater");
        });
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn command_failure_logs_code_and_stage_not_the_message() {
        let directory =
            std::env::temp_dir().join(format!("mtls-app-log-command-{}", Uuid::new_v4()));
        let secret = "sk-supersecretvalue";
        with_temp(&directory, || {
            let error = crate::error::CommandError::new("LISTEN_OVERRIDE_UNAVAILABLE", secret)
                .with_stage("listen_override");
            let _logged = crate::error::IpcError::from(error);
            let update = crate::error::CommandError::new("UPDATE_INSTALL_FAILED", secret);
            let _skipped = crate::error::IpcError::from(update);
        });
        let text = fs::read_to_string(directory.join(FILE_NAME)).unwrap_or_default();
        let _ = fs::remove_dir_all(directory);
        assert!(text.contains("\"event\":\"command.failed\""));
        assert!(text.contains("LISTEN_OVERRIDE_UNAVAILABLE"));
        assert!(text.contains("\"stage\":\"listen_override\""));
        assert!(!text.contains(secret));
        assert!(!text.contains("UPDATE_INSTALL_FAILED"));
    }
}
