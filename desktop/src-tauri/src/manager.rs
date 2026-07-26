use crate::{
    error::{CommandError, Result},
    process_identity::ProcessIdentity,
    sidecar::SidecarPaths,
    types::ManagerInfo,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};
use tauri::AppHandle;
use tauri_plugin_shell::{
    process::{CommandChild, CommandEvent},
    ShellExt,
};
use tokio::sync::{mpsc, oneshot, watch};
use zeroize::{Zeroize, Zeroizing};

const MANAGER_INFO: &str = "manager.info";
const ROUTER_STATUS: &str = "router.status";
const ROUTER_START: &str = "router.start";
const FORCE_TERMINATE_OCCUPANT: &str = "router.force_terminate_occupant";

#[derive(Debug)]
pub enum TransportEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Error,
    Terminated,
}

pub trait TransportChild: Send {
    fn write(&mut self, bytes: &[u8]) -> Result<()>;
    fn kill(self: Box<Self>);
}

pub struct TransportSession {
    pub child: Box<dyn TransportChild>,
    pub events: mpsc::Receiver<TransportEvent>,
}

pub trait TransportFactory: Send + Sync {
    fn spawn(&self) -> Result<TransportSession>;
}

struct TauriChild(CommandChild);

impl TransportChild for TauriChild {
    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.0
            .write(bytes)
            .map_err(|_| CommandError::manager_failed())
    }

    fn kill(self: Box<Self>) {
        let _ = self.0.kill();
    }
}

pub struct TauriTransportFactory {
    app: AppHandle,
    sidecars: SidecarPaths,
    parent: ProcessIdentity,
    session_id: String,
}

impl TauriTransportFactory {
    pub fn new(
        app: AppHandle,
        sidecars: SidecarPaths,
        parent: ProcessIdentity,
        session_id: String,
    ) -> Self {
        Self {
            app,
            sidecars,
            parent,
            session_id,
        }
    }
}

impl TransportFactory for TauriTransportFactory {
    fn spawn(&self) -> Result<TransportSession> {
        let args = vec![
            "serve".to_owned(),
            "--router-sidecar".to_owned(),
            self.sidecars.router.to_string_lossy().into_owned(),
            "--desktop-session".to_owned(),
            self.session_id.clone(),
            "--parent-pid".to_owned(),
            self.parent.pid.to_string(),
            "--parent-start".to_owned(),
            self.parent.started_at.clone(),
            "--parent-executable".to_owned(),
            self.parent.executable.to_string_lossy().into_owned(),
        ];
        let (mut events, child) = self
            .app
            .shell()
            .sidecar("mtls-router-manager")
            .map_err(|_| CommandError::new("SIDECAR_MISSING", "packaged manager is missing"))?
            .args(args)
            .spawn()
            .map_err(|_| CommandError::new("SIDECAR_INVALID", "packaged manager cannot execute"))?;
        let (sender, receiver) = mpsc::channel(16);
        tauri::async_runtime::spawn(async move {
            while let Some(event) = events.recv().await {
                let event = match event {
                    CommandEvent::Stdout(bytes) => TransportEvent::Stdout(bytes),
                    CommandEvent::Stderr(bytes) => TransportEvent::Stderr(bytes),
                    CommandEvent::Error(_) => TransportEvent::Error,
                    CommandEvent::Terminated(_) => TransportEvent::Terminated,
                    _ => continue,
                };
                if sender.send(event).await.is_err() {
                    break;
                }
            }
        });
        Ok(TransportSession {
            child: Box::new(TauriChild(child)),
            events: receiver,
        })
    }
}

struct Call {
    method: &'static str,
    params: Value,
    expected_session_epoch: Option<u64>,
    response: oneshot::Sender<Result<ManagerReply>>,
    activity: Option<ActivityGuard>,
}

fn clear_call_params(call: &mut Call) {
    clear_json(&mut call.params);
}

struct ManagerReply {
    value: Value,
    session_epoch: u64,
}

#[derive(Clone)]
pub struct ManagerClient {
    sender: mpsc::Sender<Call>,
    activity: Arc<ActivityState>,
    session_epoch: Arc<AtomicU64>,
    startup_error: Option<CommandError>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ManagerActivity {
    pub active: usize,
    pub generation: u64,
}

struct ActivityGuard {
    activity: Arc<ActivityState>,
}

struct ActivityState {
    current: StdMutex<ManagerActivity>,
    updates: watch::Sender<ManagerActivity>,
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let mut activity = self.activity.current.lock().unwrap();
        {
            activity.active -= 1;
            activity.generation += 1;
        }
        self.activity.updates.send_replace(*activity);
    }
}

impl ManagerClient {
    pub fn new(factory: Arc<dyn TransportFactory>) -> Self {
        let (sender, receiver) = mpsc::channel(32);
        let (updates, _) = watch::channel(ManagerActivity::default());
        let activity = Arc::new(ActivityState {
            current: StdMutex::new(ManagerActivity::default()),
            updates,
        });
        let session_epoch = Arc::new(AtomicU64::new(0));
        tauri::async_runtime::spawn(run_actor(factory, receiver, session_epoch.clone()));
        Self {
            sender,
            activity,
            session_epoch,
            startup_error: None,
        }
    }

    pub fn failed(error: CommandError) -> Self {
        let (sender, _receiver) = mpsc::channel(1);
        let (updates, _) = watch::channel(ManagerActivity::default());
        let activity = Arc::new(ActivityState {
            current: StdMutex::new(ManagerActivity::default()),
            updates,
        });
        Self {
            sender,
            activity,
            session_epoch: Arc::new(AtomicU64::new(0)),
            startup_error: Some(error),
        }
    }

    pub fn is_busy(&self) -> bool {
        self.activity().active > 0
    }

    pub(crate) fn activity(&self) -> ManagerActivity {
        *self.activity.current.lock().unwrap()
    }

    pub(crate) fn subscribe_activity(&self) -> watch::Receiver<ManagerActivity> {
        self.activity.updates.subscribe()
    }

    pub(crate) fn session_epoch(&self) -> u64 {
        self.session_epoch.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn invalidate_session_for_test(&self) {
        self.session_epoch.fetch_add(1, Ordering::AcqRel);
    }

    fn begin_activity(&self) -> ActivityGuard {
        let mut activity = self.activity.current.lock().unwrap();
        {
            activity.active += 1;
            activity.generation += 1;
        }
        self.activity.updates.send_replace(*activity);
        ActivityGuard {
            activity: self.activity.clone(),
        }
    }

    pub async fn call<T: DeserializeOwned>(
        &self,
        method: &'static str,
        params: Value,
    ) -> Result<T> {
        if let Some(error) = &self.startup_error {
            return Err(error.clone());
        }
        let (response, result) = oneshot::channel();
        if let Err(error) = self
            .sender
            .send(Call {
                method,
                params,
                expected_session_epoch: None,
                response,
                activity: Some(self.begin_activity()),
            })
            .await
        {
            let mut call = error.0;
            clear_call_params(&mut call);
            return Err(CommandError::manager_failed());
        }
        receive(result).await.map(|(value, _)| value)
    }

    pub async fn call_with_key<T: DeserializeOwned>(
        &self,
        method: &'static str,
        mut params: Value,
        mut key: Zeroizing<String>,
    ) -> Result<T> {
        if let Some(error) = &self.startup_error {
            return Err(error.clone());
        }
        params
            .as_object_mut()
            .ok_or_else(|| CommandError::invalid_params("manager params must be an object"))?
            .insert("api_key".into(), Value::String(key.to_string()));
        key.zeroize();
        drop(key);
        self.call(method, params).await
    }

    pub(crate) async fn call_with_session_epoch<T: DeserializeOwned>(
        &self,
        method: &'static str,
        params: Value,
    ) -> Result<(T, u64)> {
        if let Some(error) = &self.startup_error {
            return Err(error.clone());
        }
        let (response, result) = oneshot::channel();
        self.sender
            .send(Call {
                method,
                params,
                expected_session_epoch: None,
                response,
                activity: Some(self.begin_activity()),
            })
            .await
            .map_err(|_| CommandError::manager_failed())?;
        receive(result).await
    }

    pub(crate) async fn call_for_session<T: DeserializeOwned>(
        &self,
        method: &'static str,
        params: Value,
        expected_session_epoch: u64,
    ) -> Result<T> {
        if let Some(error) = &self.startup_error {
            return Err(error.clone());
        }
        let (response, result) = oneshot::channel();
        self.sender
            .send(Call {
                method,
                params,
                expected_session_epoch: Some(expected_session_epoch),
                response,
                activity: Some(self.begin_activity()),
            })
            .await
            .map_err(|_| CommandError::manager_failed())?;
        receive(result).await.map(|(value, _)| value)
    }

    pub(crate) async fn poll<T: DeserializeOwned>(
        &self,
        method: &'static str,
        params: Value,
        generation: u64,
    ) -> Option<Result<T>> {
        if let Some(error) = &self.startup_error {
            return Some(Err(error.clone()));
        }
        let (response, result) = oneshot::channel();
        let queued = {
            let activity = self.activity.current.lock().unwrap();
            if activity.active > 0 || activity.generation != generation {
                return None;
            }
            self.sender.try_send(Call {
                method,
                params,
                expected_session_epoch: None,
                response,
                activity: None,
            })
        };
        match queued {
            Ok(()) => Some(receive(result).await.map(|(value, _)| value)),
            Err(mpsc::error::TrySendError::Full(_)) => None,
            Err(mpsc::error::TrySendError::Closed(_)) => Some(Err(CommandError::manager_failed())),
        }
    }

    pub(crate) async fn poll_with_session_epoch<T: DeserializeOwned>(
        &self,
        method: &'static str,
        params: Value,
        generation: u64,
    ) -> Option<Result<(T, u64)>> {
        if let Some(error) = &self.startup_error {
            return Some(Err(error.clone()));
        }
        let (response, result) = oneshot::channel();
        let queued = {
            let activity = self.activity.current.lock().unwrap();
            if activity.active > 0 || activity.generation != generation {
                return None;
            }
            self.sender.try_send(Call {
                method,
                params,
                expected_session_epoch: None,
                response,
                activity: None,
            })
        };
        match queued {
            Ok(()) => Some(receive(result).await),
            Err(mpsc::error::TrySendError::Full(_)) => None,
            Err(mpsc::error::TrySendError::Closed(_)) => Some(Err(CommandError::manager_failed())),
        }
    }
}

async fn receive<T: DeserializeOwned>(
    result: oneshot::Receiver<Result<ManagerReply>>,
) -> Result<(T, u64)> {
    let reply = result.await.map_err(|_| CommandError::manager_failed())??;
    let value = serde_json::from_value(reply.value)
        .map_err(|_| CommandError::new("INVALID_RESPONSE", "manager returned an invalid result"))?;
    Ok((value, reply.session_epoch))
}

async fn run_actor(
    factory: Arc<dyn TransportFactory>,
    mut calls: mpsc::Receiver<Call>,
    session_epoch: Arc<AtomicU64>,
) {
    let (mut session, mut failure) = match start_and_handshake(factory.as_ref()).await {
        Ok(session) => (Some(session), None),
        Err(error) => (None, Some(error)),
    };
    let mut request_id = 1_u64;
    let mut recovery_used = false;
    while let Some(mut call) = calls.recv().await {
        if call
            .expected_session_epoch
            .is_some_and(|expected| expected != session_epoch.load(Ordering::Acquire))
        {
            clear_json(&mut call.params);
            let _ = call.response.send(Err(CommandError::new(
                "OCCUPANT_CHANGED",
                "port occupant changed; inspect again",
            )));
            drop(call.activity.take());
            continue;
        }
        let sensitive = matches!(
            call.method,
            "agent.models" | "agent.write" | FORCE_TERMINATE_OCCUPANT
        );
        let replayable = !sensitive;
        let params = if sensitive {
            std::mem::take(&mut call.params)
        } else {
            call.params.clone()
        };
        let result = if let Some(active) = session.as_mut() {
            transact(active, request_id, call.method, params).await
        } else {
            Err(failure.clone().unwrap_or_else(CommandError::manager_failed))
        };
        request_id += 1;

        let result = if result.as_ref().is_err_and(|error| error.recoverable) && !recovery_used {
            recovery_used = true;
            if let Some(active) = session.take() {
                session_epoch.fetch_add(1, Ordering::AcqRel);
                active.child.kill();
            }
            match recover(factory.as_ref(), &mut request_id).await {
                Ok(mut replacement) => {
                    let retried = if !replayable {
                        result
                    } else {
                        let retried =
                            transact(&mut replacement, request_id, call.method, call.params).await;
                        request_id += 1;
                        retried
                    };
                    session = Some(replacement);
                    failure = None;
                    retried
                }
                Err(error)
                    if matches!(error.code.as_str(), "SIDECAR_MISSING" | "SIDECAR_INVALID") =>
                {
                    failure = Some(error.clone());
                    Err(error)
                }
                Err(_) => {
                    failure = Some(CommandError::manager_failed());
                    Err(CommandError::manager_failed())
                }
            }
        } else {
            result
        };
        let _ = call.response.send(result.map(|value| ManagerReply {
            value,
            session_epoch: session_epoch.load(Ordering::Acquire),
        }));
        drop(call.activity.take());
    }
    if let Some(active) = session {
        active.child.kill();
    }
}

async fn start_and_handshake(factory: &dyn TransportFactory) -> Result<TransportSession> {
    let mut session = factory.spawn()?;
    let value = transact(&mut session, 0, MANAGER_INFO, json!({})).await?;
    let info: ManagerInfo = serde_json::from_value(value)
        .map_err(|_| CommandError::new("SIDECAR_INVALID", "manager handshake is malformed"))?;
    validate_handshake(&info)?;
    Ok(session)
}

async fn recover(factory: &dyn TransportFactory, request_id: &mut u64) -> Result<TransportSession> {
    let mut replacement = start_and_handshake(factory).await?;
    let status = transact(&mut replacement, *request_id, ROUTER_STATUS, json!({})).await?;
    *request_id += 1;
    match status.get("state").and_then(Value::as_str) {
        Some("absent" | "desktop_owned" | "external_compatible") => Ok(replacement),
        Some("stale" | "degraded") => {
            transact(
                &mut replacement,
                *request_id,
                ROUTER_START,
                json!({ "owner": "desktop" }),
            )
            .await?;
            *request_id += 1;
            Ok(replacement)
        }
        _ => Err(CommandError::manager_failed()),
    }
}

async fn transact(
    session: &mut TransportSession,
    request_id: u64,
    method: &'static str,
    params: Value,
) -> Result<Value> {
    let id = format!("desktop-{request_id}");
    let mut request = json!({ "id": id, "method": method, "params": params });
    let mut line = serde_json::to_vec(&request)
        .map_err(|_| CommandError::new("INVALID_REQUEST", "cannot serialize manager request"))?;
    clear_json(&mut request);
    line.push(b'\n');
    let write_result = session.child.write(&line);
    line.zeroize();
    write_result?;
    let deadline = watchdog(method)?;
    let response = tokio::time::timeout(deadline, read_response(&mut session.events, &id))
        .await
        .map_err(|_| {
            CommandError::recoverable("OPERATION_TIMEOUT", "manager operation timed out")
        })??;
    if let Some(error) = response.get("error") {
        let code = error
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("MANAGER_FAILED");
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("manager operation failed");
        let mut command_error = CommandError::new(code, message);
        if code == "MODEL_CONFIG_INVALID" {
            if let (Some(path), Some(rule)) = (
                error
                    .get("details")
                    .and_then(|details| details.get("path"))
                    .and_then(Value::as_str),
                error
                    .get("details")
                    .and_then(|details| details.get("rule"))
                    .and_then(Value::as_str),
            ) {
                command_error = command_error.with_validation_details(path, rule);
            }
        }
        return Err(command_error);
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| CommandError::new("INVALID_RESPONSE", "manager response has no result"))
}

fn clear_json(value: &mut Value) {
    match value {
        Value::String(value) => value.zeroize(),
        Value::Array(values) => values.iter_mut().for_each(clear_json),
        Value::Object(values) => values.values_mut().for_each(clear_json),
        _ => {}
    }
}

async fn read_response(events: &mut mpsc::Receiver<TransportEvent>, id: &str) -> Result<Value> {
    let mut buffered = VecDeque::new();
    loop {
        let event = if let Some(event) = buffered.pop_front() {
            event
        } else {
            events
                .recv()
                .await
                .ok_or_else(CommandError::manager_failed)?
        };
        match event {
            TransportEvent::Stdout(bytes) => {
                let response: Value = serde_json::from_slice(&bytes).map_err(|_| {
                    CommandError::recoverable(
                        "INVALID_RESPONSE",
                        "manager protocol output is malformed",
                    )
                })?;
                if response.get("id").and_then(Value::as_str) != Some(id) {
                    return Err(CommandError::recoverable(
                        "INVALID_RESPONSE",
                        "manager response ID does not match the request",
                    ));
                }
                return Ok(response);
            }
            TransportEvent::Stderr(bytes) => {
                eprintln!("mtls-router-manager: {}", String::from_utf8_lossy(&bytes));
            }
            TransportEvent::Error | TransportEvent::Terminated => {
                return Err(CommandError::manager_failed());
            }
        }
    }
}

pub(crate) fn validate_handshake(info: &ManagerInfo) -> Result<()> {
    if info.target != env!("MTLS_MANAGER_TARGET")
        || info.deployment_id != env!("MTLS_DEPLOYMENT_ID")
        || info.management_protocol_version != env!("MTLS_MANAGEMENT_PROTOCOL_VERSION")
        || info.version != env!("MTLS_MANAGER_VERSION")
    {
        return Err(CommandError::new(
            "SIDECAR_INVALID",
            "manager version or target does not match the desktop build",
        ));
    }
    Ok(())
}

fn watchdog(method: &str) -> Result<Duration> {
    let manager_seconds = match method {
        "manager.info" | "router.status" | "router.version" => 1,
        "router.logs" => 2,
        "diagnostics.collect" | "agent.detect" | "agent.render" | "agent.preview" => 5,
        "router.health" => 12,
        "router.inspect_occupant" => 2,
        FORCE_TERMINATE_OCCUPANT => 3,
        "router.stop" => 7,
        "router.start" => 20,
        "agent.models" | "agent.write" => 30,
        _ => return Err(CommandError::invalid_params("unknown manager method")),
    };
    Ok(Duration::from_secs(manager_seconds + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, Mutex, OnceLock};
    use tokio::sync::Semaphore;

    #[test]
    fn failed_send_cleanup_clears_sensitive_call_params() {
        let (response, _result) = oneshot::channel();
        let mut call = Call {
            method: "agent.models",
            params: json!({"api_key": "fixture-secret", "nested": ["sensitive"]}),
            expected_session_epoch: None,
            response,
            activity: None,
        };

        clear_call_params(&mut call);

        assert_eq!(call.params["api_key"], "");
        assert_eq!(call.params["nested"][0], "");
    }

    struct FakeChild {
        writes: Arc<Mutex<Vec<Value>>>,
        responder: mpsc::Sender<TransportEvent>,
        behavior: Behavior,
        reclaimed: bool,
    }

    #[derive(Clone, Copy)]
    enum Behavior {
        Valid,
        Malformed,
        Terminated,
        Delayed,
        InvalidHandshake,
        ValidationError,
        RecoverableRouter,
        ReclaimRejected,
    }

    impl TransportChild for FakeChild {
        fn write(&mut self, bytes: &[u8]) -> Result<()> {
            let value: Value = serde_json::from_slice(bytes).expect("request");
            self.writes.lock().unwrap().push(value.clone());
            let id = value["id"].clone();
            let method = value["method"].as_str().unwrap().to_owned();
            let sender = self.responder.clone();
            let behavior = self.behavior;
            let reclaimed = self.reclaimed;
            if matches!(behavior, Behavior::RecoverableRouter) && method == ROUTER_START {
                self.reclaimed = true;
            }
            tauri::async_runtime::spawn(async move {
                let event = match behavior {
                    Behavior::InvalidHandshake if method == MANAGER_INFO => {
                        let result = json!({
                            "version": env!("MTLS_MANAGER_VERSION"), "commit": "test", "build_date": "test",
                            "target": "wrong/target", "deployment_id": env!("MTLS_DEPLOYMENT_ID"),
                            "management_protocol_version": env!("MTLS_MANAGEMENT_PROTOCOL_VERSION")
                        });
                        TransportEvent::Stdout(
                            serde_json::to_vec(&json!({ "id": id, "result": result })).unwrap(),
                        )
                    }
                    Behavior::Malformed if method != MANAGER_INFO => {
                        TransportEvent::Stdout(b"not-json".to_vec())
                    }
                    Behavior::Terminated if method != MANAGER_INFO => TransportEvent::Terminated,
                    Behavior::Delayed if method != MANAGER_INFO => {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        response(id, method, behavior, reclaimed)
                    }
                    _ => response(id, method, behavior, reclaimed),
                };
                let _ = sender.send(event).await;
            });
            Ok(())
        }

        fn kill(self: Box<Self>) {}
    }

    fn response(id: Value, method: String, behavior: Behavior, reclaimed: bool) -> TransportEvent {
        if matches!(behavior, Behavior::ValidationError) && method == "agent.preview" {
            return TransportEvent::Stdout(
                serde_json::to_vec(&json!({
                    "id": id,
                    "error": {
                        "code": "MODEL_CONFIG_INVALID",
                        "message": "Agent model configuration is invalid",
                        "details": {
                            "path": "/claude/max_output_tokens",
                            "rule": "integer_relationship"
                        }
                    }
                }))
                .unwrap(),
            );
        }
        if matches!(behavior, Behavior::ReclaimRejected) && method == ROUTER_START {
            return TransportEvent::Stdout(
                serde_json::to_vec(&json!({
                    "id": id,
                    "error": { "code": "ROUTER_ALREADY_RUNNING", "message": "router is already running" }
                }))
                .unwrap(),
            );
        }
        let result = match method.as_str() {
            MANAGER_INFO => json!({
                "version": env!("MTLS_MANAGER_VERSION"), "commit": "test", "build_date": "test",
                "target": env!("MTLS_MANAGER_TARGET"), "deployment_id": env!("MTLS_DEPLOYMENT_ID"),
                "management_protocol_version": env!("MTLS_MANAGEMENT_PROTOCOL_VERSION")
            }),
            ROUTER_STATUS if matches!(behavior, Behavior::RecoverableRouter) && reclaimed => {
                json!({ "state": "desktop_owned", "owner": "desktop" })
            }
            ROUTER_STATUS
                if matches!(
                    behavior,
                    Behavior::RecoverableRouter | Behavior::ReclaimRejected
                ) =>
            {
                json!({ "state": "stale", "owner": "desktop" })
            }
            ROUTER_STATUS => json!({ "state": "absent" }),
            ROUTER_START => json!({ "state": "desktop_owned", "owner": "desktop" }),
            _ => json!({ "lines": ["safe"] }),
        };
        TransportEvent::Stdout(serde_json::to_vec(&json!({ "id": id, "result": result })).unwrap())
    }

    struct FakeFactory {
        behaviors: Mutex<VecDeque<Behavior>>,
        writes: Arc<Mutex<Vec<Value>>>,
    }

    impl TransportFactory for FakeFactory {
        fn spawn(&self) -> Result<TransportSession> {
            let behavior = self
                .behaviors
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Behavior::Valid);
            let (sender, events) = mpsc::channel(8);
            Ok(TransportSession {
                child: Box::new(FakeChild {
                    writes: self.writes.clone(),
                    responder: sender,
                    behavior,
                    reclaimed: false,
                }),
                events,
            })
        }
    }

    struct QueuedRecoveryChild {
        replacement: bool,
        writes: Arc<Mutex<Vec<Value>>>,
        responder: mpsc::Sender<TransportEvent>,
        blocked: Arc<Semaphore>,
        release: Arc<Semaphore>,
    }

    impl TransportChild for QueuedRecoveryChild {
        fn write(&mut self, bytes: &[u8]) -> Result<()> {
            let request: Value = serde_json::from_slice(bytes).unwrap();
            self.writes.lock().unwrap().push(request.clone());
            let method = request["method"].as_str().unwrap().to_owned();
            let replacement = self.replacement;
            let responder = self.responder.clone();
            let blocked = self.blocked.clone();
            let release = self.release.clone();
            tauri::async_runtime::spawn(async move {
                let event = if !replacement && method == "router.logs" {
                    blocked.add_permits(1);
                    release.acquire().await.unwrap().forget();
                    TransportEvent::Terminated
                } else {
                    response(request["id"].clone(), method, Behavior::Valid, false)
                };
                responder.send(event).await.unwrap();
            });
            Ok(())
        }

        fn kill(self: Box<Self>) {}
    }

    struct QueuedRecoveryFactory {
        spawns: AtomicUsize,
        writes: Arc<Mutex<Vec<Value>>>,
        blocked: Arc<Semaphore>,
        release: Arc<Semaphore>,
    }

    impl TransportFactory for QueuedRecoveryFactory {
        fn spawn(&self) -> Result<TransportSession> {
            let replacement = self.spawns.fetch_add(1, Ordering::AcqRel) > 0;
            let (responder, events) = mpsc::channel(8);
            Ok(TransportSession {
                child: Box::new(QueuedRecoveryChild {
                    replacement,
                    writes: self.writes.clone(),
                    responder,
                    blocked: self.blocked.clone(),
                    release: self.release.clone(),
                }),
                events,
            })
        }
    }

    fn runtime() -> &'static tokio::runtime::Runtime {
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().unwrap())
    }

    fn client(behaviors: Vec<Behavior>) -> (ManagerClient, Arc<Mutex<Vec<Value>>>) {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let factory = Arc::new(FakeFactory {
            behaviors: Mutex::new(behaviors.into()),
            writes: writes.clone(),
        });
        (ManagerClient::new(factory), writes)
    }

    #[test]
    fn serializes_calls_and_correlates_ids() {
        runtime().block_on(async {
            let (client, writes) = client(vec![Behavior::Valid]);
            let first: Value = client
                .call("router.logs", json!({ "limit": 1 }))
                .await
                .unwrap();
            let second: Value = client
                .call("router.logs", json!({ "limit": 2 }))
                .await
                .unwrap();
            assert_eq!(first["lines"][0], "safe");
            assert_eq!(second["lines"][0], "safe");
            let writes = writes.lock().unwrap();
            assert_eq!(writes[1]["id"], "desktop-1");
            assert_eq!(writes[2]["id"], "desktop-2");
        });
    }

    #[test]
    fn preserves_safe_model_validation_details() {
        runtime().block_on(async {
            let (client, _) = client(vec![Behavior::ValidationError]);
            let error = client
                .call::<Value>("agent.preview", json!({}))
                .await
                .unwrap_err();
            assert_eq!(error.code, "MODEL_CONFIG_INVALID");
            assert_eq!(
                error.details.unwrap(),
                crate::error::ErrorDetails {
                    path: "/claude/max_output_tokens".into(),
                    rule: "integer_relationship".into(),
                }
            );
        });
    }

    #[test]
    fn malformed_or_terminated_manager_gets_one_recovery() {
        runtime().block_on(async {
            for behavior in [Behavior::Malformed, Behavior::Terminated] {
                let (client, _) = client(vec![behavior, Behavior::Valid]);
                assert_eq!(client.session_epoch(), 0);
                let value: Value = client
                    .call("router.logs", json!({ "limit": 1 }))
                    .await
                    .unwrap();
                assert_eq!(value["lines"][0], "safe");
                assert_eq!(client.session_epoch(), 1);
            }
        });
    }

    #[test]
    fn watchdog_is_one_second_beyond_each_manager_deadline() {
        assert_eq!(watchdog("router.status").unwrap(), Duration::from_secs(2));
        assert_eq!(watchdog("router.health").unwrap(), Duration::from_secs(13));
        assert_eq!(
            watchdog("router.inspect_occupant").unwrap(),
            Duration::from_secs(3)
        );
        assert_eq!(
            watchdog(FORCE_TERMINATE_OCCUPANT).unwrap(),
            Duration::from_secs(4)
        );
        assert_eq!(watchdog("router.start").unwrap(), Duration::from_secs(21));
        assert_eq!(watchdog("agent.write").unwrap(), Duration::from_secs(31));
        assert_eq!(watchdog("agent.models").unwrap(), Duration::from_secs(31));
        assert_eq!(watchdog("agent.render").unwrap(), Duration::from_secs(6));
    }

    #[test]
    fn handshake_rejects_mismatched_target_or_build_identity() {
        let mut info = ManagerInfo {
            version: env!("MTLS_MANAGER_VERSION").to_owned(),
            commit: "test".to_owned(),
            build_date: "test".to_owned(),
            target: "wrong/target".to_owned(),
            deployment_id: env!("MTLS_DEPLOYMENT_ID").to_owned(),
            management_protocol_version: env!("MTLS_MANAGEMENT_PROTOCOL_VERSION").to_owned(),
        };
        assert_eq!(
            validate_handshake(&info).unwrap_err().code,
            "SIDECAR_INVALID"
        );
        info.target = env!("MTLS_MANAGER_TARGET").to_owned();
        info.deployment_id = "wrong".to_owned();
        assert_eq!(
            validate_handshake(&info).unwrap_err().code,
            "SIDECAR_INVALID"
        );
        info.deployment_id = env!("MTLS_DEPLOYMENT_ID").to_owned();
        info.version = "wrong".to_owned();
        assert_eq!(
            validate_handshake(&info).unwrap_err().code,
            "SIDECAR_INVALID"
        );
        info.version = env!("MTLS_MANAGER_VERSION").to_owned();
        info.management_protocol_version = "wrong".to_owned();
        assert_eq!(
            validate_handshake(&info).unwrap_err().code,
            "SIDECAR_INVALID"
        );
    }

    #[test]
    fn delayed_response_times_out_and_recovers_once() {
        runtime().block_on(async {
            let (client, _) = client(vec![Behavior::Delayed, Behavior::Valid]);
            let value: Value = client.call(ROUTER_STATUS, json!({})).await.unwrap();
            assert_eq!(value["state"], "absent");
        });
    }

    #[test]
    fn agent_write_is_never_replayed_after_ambiguous_failure() {
        runtime().block_on(async {
            let (client, writes) = client(vec![Behavior::Malformed, Behavior::Valid]);
            let error = client
                .call::<Value>(
                    "agent.write",
                    json!({ "agents": ["claude"], "revision_token": "revision", "api_key": "secret" }),
                )
                .await
                .unwrap_err();
            assert_eq!(error.code, "INVALID_RESPONSE");
            let writes = writes.lock().unwrap();
            assert_eq!(
                writes
                    .iter()
                    .filter(|request| request["method"] == "agent.write")
                    .count(),
                1
            );
        });
    }

    #[test]
    fn agent_models_is_never_replayed_after_ambiguous_failure() {
        runtime().block_on(async {
            let (client, writes) = client(vec![Behavior::Malformed, Behavior::Valid]);
            let error = client
                .call::<Value>(
                    "agent.models",
                    json!({ "owner": "desktop", "agents": ["claude"], "api_key": "secret" }),
                )
                .await
                .unwrap_err();
            assert_eq!(error.code, "INVALID_RESPONSE");
            assert_eq!(
                writes
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|request| request["method"] == "agent.models")
                    .count(),
                1
            );
        });
    }

    #[test]
    fn force_termination_is_never_replayed_after_ambiguous_failure() {
        runtime().block_on(async {
            let (client, writes) = client(vec![Behavior::Malformed, Behavior::Valid]);
            let error = client
                .call::<Value>(
                    FORCE_TERMINATE_OCCUPANT,
                    json!({ "confirmation_token": "single-use" }),
                )
                .await
                .unwrap_err();
            assert_eq!(error.code, "INVALID_RESPONSE");
            assert_eq!(
                writes
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|request| request["method"] == FORCE_TERMINATE_OCCUPANT)
                    .count(),
                1
            );
        });
    }

    #[test]
    fn queued_fenced_force_is_rejected_after_prior_call_recovers_session() {
        runtime().block_on(async {
            let writes = Arc::new(Mutex::new(Vec::new()));
            let blocked = Arc::new(Semaphore::new(0));
            let release = Arc::new(Semaphore::new(0));
            let client = ManagerClient::new(Arc::new(QueuedRecoveryFactory {
                spawns: AtomicUsize::new(0),
                writes: writes.clone(),
                blocked: blocked.clone(),
                release: release.clone(),
            }));
            let first = {
                let client = client.clone();
                tauri::async_runtime::spawn(async move {
                    client
                        .call::<Value>("router.logs", json!({ "limit": 1 }))
                        .await
                })
            };
            blocked.acquire().await.unwrap().forget();
            let force = {
                let client = client.clone();
                tauri::async_runtime::spawn(async move {
                    client
                        .call_for_session::<Value>(
                            FORCE_TERMINATE_OCCUPANT,
                            json!({ "confirmation_token": "must-not-be-sent" }),
                            0,
                        )
                        .await
                })
            };
            while client.activity().active < 2 {
                tokio::task::yield_now().await;
            }
            release.add_permits(1);

            assert!(first.await.unwrap().is_ok());
            let error = force.await.unwrap().unwrap_err();
            assert_eq!(error.code, "OCCUPANT_CHANGED");
            assert_eq!(error.message, "port occupant changed; inspect again");
            assert_eq!(client.session_epoch(), 1);
            let writes = writes.lock().unwrap();
            assert!(writes
                .iter()
                .all(|request| request["method"] != FORCE_TERMINATE_OCCUPANT));
            assert!(!serde_json::to_string(&*writes)
                .unwrap()
                .contains("must-not-be-sent"));
        });
    }

    #[test]
    fn failed_recovery_preserves_sidecar_error_and_disables_the_client() {
        runtime().block_on(async {
            let (client, writes) = client(vec![Behavior::Terminated, Behavior::InvalidHandshake]);
            assert_eq!(
                client
                    .call::<Value>(ROUTER_STATUS, json!({}))
                    .await
                    .unwrap_err()
                    .code,
                "SIDECAR_INVALID"
            );
            assert_eq!(client.session_epoch(), 1);
            assert_eq!(
                client
                    .call::<Value>(ROUTER_START, json!({ "owner": "desktop" }))
                    .await
                    .unwrap_err()
                    .code,
                "SIDECAR_INVALID"
            );
            assert_eq!(client.session_epoch(), 1);
            let writes = writes.lock().unwrap();
            assert_eq!(
                writes
                    .iter()
                    .filter(|request| request["method"] == MANAGER_INFO)
                    .count(),
                2
            );
            assert_eq!(
                writes
                    .iter()
                    .filter(|request| request["method"] == ROUTER_START)
                    .count(),
                0
            );
        });
    }

    #[test]
    fn replacement_manager_reclaims_before_retrying_start_or_status() {
        runtime().block_on(async {
            for method in [ROUTER_START, ROUTER_STATUS] {
                let (client, writes) =
                    client(vec![Behavior::Terminated, Behavior::RecoverableRouter]);
                let params = if method == ROUTER_START {
                    json!({ "owner": "desktop" })
                } else {
                    json!({})
                };
                let value: Value = client.call(method, params).await.unwrap();
                assert_eq!(value["state"], "desktop_owned");
                assert_eq!(client.session_epoch(), 1);

                let methods: Vec<String> = writes
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|request| request["method"].as_str().unwrap().to_owned())
                    .collect();
                assert_eq!(
                    methods,
                    [
                        MANAGER_INFO,
                        method,
                        MANAGER_INFO,
                        ROUTER_STATUS,
                        ROUTER_START,
                        method
                    ]
                );
            }
        });
    }

    #[test]
    fn failed_reclaim_disables_client_without_second_start_attempt() {
        runtime().block_on(async {
            let (client, writes) = client(vec![Behavior::Terminated, Behavior::ReclaimRejected]);
            assert_eq!(
                client
                    .call::<Value>(ROUTER_STATUS, json!({}))
                    .await
                    .unwrap_err()
                    .code,
                "MANAGER_FAILED"
            );
            assert_eq!(
                client
                    .call::<Value>(ROUTER_START, json!({ "owner": "desktop" }))
                    .await
                    .unwrap_err()
                    .code,
                "MANAGER_FAILED"
            );
            let writes = writes.lock().unwrap();
            assert_eq!(
                writes
                    .iter()
                    .filter(|request| request["method"] == ROUTER_START)
                    .count(),
                1
            );
        });
    }
}
