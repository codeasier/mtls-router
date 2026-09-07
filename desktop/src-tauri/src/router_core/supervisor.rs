use crate::protocol::ErrorCode;
use crate::redaction::{contain_panic, proxy_error_json, ContainedFailure, ContainedFailureKind};
use bytes::Bytes;
use http::{header, Request, Response, StatusCode};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::{json, Map};
use std::convert::Infallible;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::config::validate_upstream_url;
use super::proxy::ProxyLogger;
use super::{
    prepare_startup, serve_request, BuildInfo, Prober, ReverseProxy, RouterDefaults, RouterEnv,
    RouterFlags, StartupError, StartupReason, TlsMaterials,
};

const DEFAULT_COMMAND_QUEUE: usize = 8;
const DEFAULT_MAX_CONCURRENT: u32 = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SupervisorLimits {
    pub command_queue: usize,
    pub max_concurrent_requests: u32,
    pub command_timeout: Duration,
    pub request_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub read_header_timeout: Duration,
}

impl Default for SupervisorLimits {
    fn default() -> Self {
        Self {
            command_queue: DEFAULT_COMMAND_QUEUE,
            max_concurrent_requests: DEFAULT_MAX_CONCURRENT,
            command_timeout: Duration::from_secs(2),
            request_timeout: Duration::from_secs(10),
            shutdown_timeout: Duration::from_secs(5),
            read_header_timeout: Duration::from_secs(10),
        }
    }
}

pub struct SupervisorConfig {
    pub defaults: RouterDefaults,
    pub env: RouterEnv,
    pub flags: RouterFlags,
    pub materials: TlsMaterials,
    pub identity: BuildInfo,
    pub limits: SupervisorLimits,
    pub logger: Option<ProxyLogger>,
}

impl fmt::Debug for SupervisorConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SupervisorConfig")
            .field("listen_addr", &self.defaults.listen_addr)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimePhase {
    Stopped,
    Starting,
    Running,
    Degraded,
    Failed,
}

#[derive(Clone, Debug)]
pub struct RuntimeStatus {
    pub phase: RuntimePhase,
    pub listen_addr: Option<String>,
    pub bound: bool,
    pub active_requests: u32,
    pub max_active_requests: u32,
    pub last_failure: Option<ContainedFailure>,
    pub last_startup_error: Option<StartupReason>,
    pub runtime_thread_id: ThreadId,
}

#[derive(Debug)]
pub enum SupervisorError {
    Startup(StartupError),
    Failure(ContainedFailure),
    AlreadyRunning,
    CommandTimeout,
    Disconnected,
}

impl SupervisorError {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Startup(error) => error.reason().as_str(),
            Self::Failure(failure) => failure_reason(failure.kind),
            Self::AlreadyRunning => "router_already_running",
            Self::CommandTimeout => "command_timeout",
            Self::Disconnected => "router_failure",
        }
    }

    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Startup(_) => ErrorCode::RouterStartFailed,
            Self::Failure(failure) => failure.code,
            Self::AlreadyRunning => ErrorCode::RouterAlreadyRunning,
            Self::CommandTimeout => ErrorCode::OperationTimeout,
            Self::Disconnected => ErrorCode::RouterDegraded,
        }
    }
}

impl fmt::Display for SupervisorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for SupervisorError {}

fn failure_reason(kind: ContainedFailureKind) -> &'static str {
    match kind {
        ContainedFailureKind::RouterPanic => "router_panic",
        ContainedFailureKind::RequestTimeout => "request_timeout",
        ContainedFailureKind::ListenFailed => "listen_failed",
        ContainedFailureKind::ShutdownTimeout => "shutdown_timeout",
    }
}

fn contained(kind: ContainedFailureKind) -> ContainedFailure {
    ContainedFailure {
        kind,
        code: match kind {
            ContainedFailureKind::RouterPanic => ErrorCode::RouterDegraded,
            ContainedFailureKind::RequestTimeout | ContainedFailureKind::ShutdownTimeout => {
                ErrorCode::OperationTimeout
            }
            ContainedFailureKind::ListenFailed => ErrorCode::RouterStartFailed,
        },
    }
}

pub struct RouterSupervisor {
    tx: Option<mpsc::Sender<Command>>,
    shared: Arc<SharedState>,
    thread: Option<thread::JoinHandle<()>>,
    limits: SupervisorLimits,
}

impl fmt::Debug for RouterSupervisor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = self.shared.snapshot();
        f.debug_struct("RouterSupervisor")
            .field("thread_id", &status.runtime_thread_id)
            .field("phase", &status.phase)
            .finish_non_exhaustive()
    }
}

impl Drop for RouterSupervisor {
    fn drop(&mut self) {
        self.tx.take();
    }
}

struct SharedState {
    status: Mutex<RuntimeStatus>,
    active: AtomicU32,
    max_active: AtomicU32,
    bound: AtomicBool,
}

impl SharedState {
    fn new(thread_id: ThreadId) -> Self {
        Self {
            status: Mutex::new(RuntimeStatus {
                phase: RuntimePhase::Stopped,
                listen_addr: None,
                bound: false,
                active_requests: 0,
                max_active_requests: 0,
                last_failure: None,
                last_startup_error: None,
                runtime_thread_id: thread_id,
            }),
            active: AtomicU32::new(0),
            max_active: AtomicU32::new(0),
            bound: AtomicBool::new(false),
        }
    }

    fn snapshot(&self) -> RuntimeStatus {
        let mut status = self
            .status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        status.active_requests = self.active.load(Ordering::SeqCst);
        status.max_active_requests = self.max_active.load(Ordering::SeqCst);
        status.bound = self.bound.load(Ordering::SeqCst);
        status
    }

    fn set_phase(
        &self,
        phase: RuntimePhase,
        listen_addr: Option<String>,
        bound: bool,
        last_failure: Option<ContainedFailure>,
        last_startup_error: Option<StartupReason>,
    ) {
        self.bound.store(bound, Ordering::SeqCst);
        let mut status = self
            .status
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        status.phase = phase;
        status.listen_addr = if phase == RuntimePhase::Stopped {
            None
        } else {
            listen_addr
        };
        status.bound = bound && phase != RuntimePhase::Stopped;
        if phase == RuntimePhase::Stopped {
            self.bound.store(false, Ordering::SeqCst);
            status.bound = false;
        }
        if phase == RuntimePhase::Starting {
            status.last_failure = None;
            status.last_startup_error = None;
        } else {
            if last_failure.is_some() {
                status.last_failure = last_failure;
            }
            if last_startup_error.is_some() {
                status.last_startup_error = last_startup_error;
            }
        }
    }

    fn record_failure(&self, failure: ContainedFailure) {
        let mut status = self
            .status
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        status.phase = RuntimePhase::Degraded;
        status.last_failure = Some(failure);
    }

    fn begin_request(&self) {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
    }

    fn end_request(&self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

enum Command {
    Start {
        reply: oneshot::Sender<Result<String, SupervisorError>>,
    },
    Stop {
        reply: oneshot::Sender<Result<(), SupervisorError>>,
    },
    #[cfg(test)]
    Hold {
        release: Arc<tokio::sync::Notify>,
        started: oneshot::Sender<()>,
    },
    #[cfg(test)]
    Panic {
        reply: oneshot::Sender<()>,
    },
    Shutdown,
}

#[derive(Clone)]
struct Serving {
    proxy: ReverseProxy,
    prober: Prober,
    identity: BuildInfo,
    started_at: String,
    semaphore: Arc<Semaphore>,
}

impl RouterSupervisor {
    pub fn spawn(config: SupervisorConfig) -> Self {
        let limits = config.limits;
        let queue = limits.command_queue.max(1);
        let (tx, rx) = mpsc::channel(queue);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = thread::Builder::new()
            .name("mtls-router-core".into())
            .spawn(move || {
                let thread_id = thread::current().id();
                let shared = Arc::new(SharedState::new(thread_id));
                let _ = ready_tx.send(shared.clone());
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
                    .build()
                    .expect("dedicated router runtime");
                runtime.block_on(actor_loop(config, rx, shared));
            })
            .expect("dedicated router thread");
        let shared = ready_rx.recv().expect("router shared state");
        Self {
            tx: Some(tx),
            shared,
            thread: Some(thread),
            limits,
        }
    }

    pub fn thread_id(&self) -> ThreadId {
        self.shared.snapshot().runtime_thread_id
    }

    pub fn status(&self) -> RuntimeStatus {
        let mut status = self.shared.snapshot();
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| thread.is_finished())
            && !matches!(status.phase, RuntimePhase::Stopped)
        {
            status.phase = RuntimePhase::Degraded;
            status.last_failure = Some(contain_panic(""));
            status.bound = false;
        }
        status
    }

    pub async fn start(&self) -> Result<String, SupervisorError> {
        let (reply, rx) = oneshot::channel();
        self.send(Command::Start { reply }).await?;
        self.recv(rx).await?
    }

    pub async fn stop(&self) -> Result<(), SupervisorError> {
        let (reply, rx) = oneshot::channel();
        self.send(Command::Stop { reply }).await?;
        self.recv(rx).await?
    }

    pub async fn shutdown(mut self) -> Result<(), SupervisorError> {
        let result = self.stop().await;
        self.tx.take();
        if let Some(thread) = self.thread.take() {
            let _ = tokio::task::spawn_blocking(move || thread.join()).await;
        }
        result
    }

    #[cfg(test)]
    pub async fn hold_for_test(
        &self,
        release: Arc<tokio::sync::Notify>,
    ) -> Result<(), SupervisorError> {
        let (started, rx) = oneshot::channel();
        self.send(Command::Hold { release, started }).await?;
        self.recv(rx).await
    }

    #[cfg(test)]
    pub async fn inject_panic_for_test(&self) -> Result<(), SupervisorError> {
        let (reply, rx) = oneshot::channel();
        self.send(Command::Panic { reply }).await?;
        self.recv(rx).await
    }

    async fn send(&self, command: Command) -> Result<(), SupervisorError> {
        let tx = self.tx.as_ref().ok_or(SupervisorError::Disconnected)?;
        timeout(self.limits.command_timeout, tx.send(command))
            .await
            .map_err(|_| SupervisorError::CommandTimeout)?
            .map_err(|_| SupervisorError::Disconnected)
    }

    async fn recv<T>(&self, rx: oneshot::Receiver<T>) -> Result<T, SupervisorError> {
        timeout(self.limits.command_timeout, rx)
            .await
            .map_err(|_| SupervisorError::CommandTimeout)?
            .map_err(|_| SupervisorError::Disconnected)
    }
}

enum ActorEvent {
    Accepted(std::io::Result<(tokio::net::TcpStream, std::net::SocketAddr)>),
    Command(Option<Command>),
    Joined(Option<Result<(), tokio::task::JoinError>>),
}

async fn actor_loop(
    config: SupervisorConfig,
    mut commands: mpsc::Receiver<Command>,
    shared: Arc<SharedState>,
) {
    let mut listener: Option<TcpListener> = None;
    let mut serving: Option<Serving> = None;
    let mut inflight: JoinSet<()> = JoinSet::new();
    loop {
        let event = if listener.is_some() {
            tokio::select! {
                joined = inflight.join_next(), if !inflight.is_empty() => ActorEvent::Joined(joined),
                accepted = listener.as_ref().expect("listener").accept() => {
                    ActorEvent::Accepted(accepted)
                }
                command = commands.recv() => ActorEvent::Command(command),
            }
        } else {
            tokio::select! {
                joined = inflight.join_next(), if !inflight.is_empty() => ActorEvent::Joined(joined),
                command = commands.recv() => ActorEvent::Command(command),
            }
        };
        match event {
            ActorEvent::Joined(Some(Err(error))) if error.is_panic() => {
                shared.record_failure(contain_panic(""));
            }
            ActorEvent::Joined(_) => {}
            ActorEvent::Accepted(Ok((stream, _))) => {
                if let Some(serving) = serving.clone() {
                    let shared = shared.clone();
                    let limits = config.limits;
                    inflight.spawn(async move {
                        serve_connection(stream, serving, shared, limits).await;
                    });
                }
            }
            ActorEvent::Accepted(Err(_)) => {
                shared.set_phase(
                    RuntimePhase::Failed,
                    None,
                    false,
                    Some(contained(ContainedFailureKind::ListenFailed)),
                    Some(StartupReason::ListenFailed),
                );
                listener.take();
                serving.take();
            }
            ActorEvent::Command(None) => break,
            ActorEvent::Command(Some(Command::Start { reply })) => {
                let result = start_router(&config, &shared, &mut listener, &mut serving).await;
                let _ = reply.send(result);
            }
            ActorEvent::Command(Some(Command::Stop { reply })) => {
                let result = stop_router(
                    &config.limits,
                    &shared,
                    &mut listener,
                    &mut serving,
                    &mut inflight,
                )
                .await;
                let _ = reply.send(result);
            }
            ActorEvent::Command(Some(Command::Shutdown)) => {
                let _ = stop_router(
                    &config.limits,
                    &shared,
                    &mut listener,
                    &mut serving,
                    &mut inflight,
                )
                .await;
                break;
            }
            #[cfg(test)]
            ActorEvent::Command(Some(Command::Hold { release, started })) => {
                let _ = started.send(());
                release.notified().await;
            }
            #[cfg(test)]
            ActorEvent::Command(Some(Command::Panic { reply })) => {
                let panicked = tokio::spawn(async {
                    panic!("sk-supervisor-panic-canary");
                })
                .await;
                if panicked.err().is_some_and(|error| error.is_panic()) {
                    shared.record_failure(contain_panic("sk-supervisor-panic-canary"));
                }
                let _ = reply.send(());
            }
        }
    }
}

async fn start_router(
    config: &SupervisorConfig,
    shared: &SharedState,
    listener: &mut Option<TcpListener>,
    serving: &mut Option<Serving>,
) -> Result<String, SupervisorError> {
    if listener.is_some() {
        return Err(SupervisorError::AlreadyRunning);
    }
    shared.set_phase(RuntimePhase::Starting, None, false, None, None);
    let prepared = match prepare_startup(
        config.defaults.clone(),
        &config.env,
        &config.flags,
        &config.materials,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(error) => {
            shared.set_phase(
                RuntimePhase::Failed,
                None,
                false,
                None,
                Some(error.reason()),
            );
            return Err(SupervisorError::Startup(error));
        }
    };
    let bound = match TcpListener::bind(prepared.config().listen_addr()).await {
        Ok(bound) => bound,
        Err(_) => {
            shared.set_phase(
                RuntimePhase::Failed,
                None,
                false,
                Some(contained(ContainedFailureKind::ListenFailed)),
                Some(StartupReason::ListenFailed),
            );
            return Err(SupervisorError::Startup(StartupError::new(
                StartupReason::ListenFailed,
            )));
        }
    };
    let addr = bound
        .local_addr()
        .map_err(|_| SupervisorError::Startup(StartupError::new(StartupReason::ListenFailed)))?
        .to_string();
    *serving = Some(serving_from_prepared(prepared, config)?);
    *listener = Some(bound);
    shared.set_phase(RuntimePhase::Running, Some(addr.clone()), true, None, None);
    Ok(addr)
}

fn serving_from_prepared(
    prepared: super::PreparedRouter,
    config: &SupervisorConfig,
) -> Result<Serving, SupervisorError> {
    let upstream = validate_upstream_url(prepared.config().upstream_url())
        .map_err(SupervisorError::Startup)?;
    let prober =
        Prober::new(prepared.config(), prepared.transport()).map_err(SupervisorError::Startup)?;
    let mut proxy = ReverseProxy::new(upstream, prepared.transport().clone());
    if let Some(logger) = &config.logger {
        proxy = proxy.with_logger(logger.clone());
    }
    Ok(Serving {
        proxy,
        prober,
        identity: config.identity.clone(),
        started_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        semaphore: Arc::new(Semaphore::new(
            config.limits.max_concurrent_requests.max(1) as usize
        )),
    })
}

async fn stop_router(
    limits: &SupervisorLimits,
    shared: &SharedState,
    listener: &mut Option<TcpListener>,
    serving: &mut Option<Serving>,
    inflight: &mut JoinSet<()>,
) -> Result<(), SupervisorError> {
    listener.take();
    serving.take();
    let idle = async { while inflight.join_next().await.is_some() {} };
    let timed_out = timeout(limits.shutdown_timeout, idle).await.is_err();
    inflight.abort_all();
    while inflight.join_next().await.is_some() {}
    shared.set_phase(RuntimePhase::Stopped, None, false, None, None);
    if timed_out {
        let failure = contained(ContainedFailureKind::ShutdownTimeout);
        shared.record_failure(failure.clone());
        shared.set_phase(
            RuntimePhase::Stopped,
            None,
            false,
            Some(failure.clone()),
            Some(StartupReason::ShutdownFailed),
        );
        return Err(SupervisorError::Failure(failure));
    }
    Ok(())
}

async fn serve_connection(
    stream: tokio::net::TcpStream,
    serving: Serving,
    shared: Arc<SharedState>,
    limits: SupervisorLimits,
) {
    let service = service_fn(move |request: Request<Incoming>| {
        let serving = serving.clone();
        let shared = shared.clone();
        async move {
            let response = supervised_request(request, serving, shared, limits).await;
            Ok::<_, Infallible>(response)
        }
    });
    let mut builder = hyper::server::conn::http1::Builder::new();
    builder.timer(TokioTimer::new());
    builder.header_read_timeout(limits.read_header_timeout);
    let _ = builder
        .serve_connection(TokioIo::new(stream), service)
        .await;
}

async fn supervised_request(
    request: Request<Incoming>,
    serving: Serving,
    shared: Arc<SharedState>,
    limits: SupervisorLimits,
) -> Response<BoxBody<Bytes, Infallible>> {
    let permit = match timeout(
        limits.request_timeout,
        serving.semaphore.clone().acquire_owned(),
    )
    .await
    {
        Ok(Ok(permit)) => permit,
        _ => return timeout_response(),
    };
    shared.begin_request();
    let started_at = serving.started_at.clone();
    let provider = move || {
        let mut extra = Map::new();
        extra.insert("started_at".to_owned(), json!(started_at));
        extra
    };
    let work = serve_request(
        request,
        &serving.identity,
        Some(&provider),
        || serving.prober.probe(),
        &serving.proxy,
    );
    let response = match timeout(limits.request_timeout, work).await {
        Ok(response) => response,
        Err(_) => timeout_response(),
    };
    shared.end_request();
    drop(permit);
    response
}

fn timeout_response() -> Response<BoxBody<Bytes, Infallible>> {
    let mut body = serde_json::to_vec(&proxy_error_json(504)).expect("timeout json");
    body.push(b'\n');
    Response::builder()
        .status(StatusCode::GATEWAY_TIMEOUT)
        .header(header::CONTENT_TYPE, "application/json")
        .body(
            Full::new(Bytes::from(body))
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("timeout response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router_core::test_support::{
        boxed_bytes, start_mtls_handler, start_mtls_server, MtlsServerOptions,
    };
    use http_body_util::Empty;
    use std::sync::atomic::AtomicU32;
    use tokio::net::TcpStream;
    use tokio::sync::Notify;

    fn test_limits() -> SupervisorLimits {
        SupervisorLimits {
            command_queue: 1,
            max_concurrent_requests: 1,
            command_timeout: Duration::from_millis(80),
            request_timeout: Duration::from_millis(80),
            shutdown_timeout: Duration::from_millis(40),
            read_header_timeout: Duration::from_millis(80),
        }
    }

    fn config_for(upstream: &str, listen: &str, materials: TlsMaterials) -> SupervisorConfig {
        let mut defaults = RouterDefaults::default();
        defaults.listen_addr = listen.to_owned();
        defaults.upstream_url = upstream.to_owned();
        defaults.timeout = Duration::from_secs(1);
        SupervisorConfig {
            defaults,
            env: RouterEnv::default(),
            flags: RouterFlags::default(),
            materials,
            identity: BuildInfo::default(),
            limits: test_limits(),
            logger: None,
        }
    }

    async fn http1_get(base: &str, path: &str) -> Response<hyper::body::Incoming> {
        http1_get_result(base, path).await.expect("http get")
    }

    async fn http1_get_result(
        base: &str,
        path: &str,
    ) -> Result<Response<hyper::body::Incoming>, Box<dyn std::error::Error + Send + Sync>> {
        let uri: http::Uri = base.parse()?;
        let stream = TcpStream::connect((uri.host().unwrap(), uri.port_u16().unwrap())).await?;
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
        tokio::spawn(connection);
        Ok(sender
            .send_request(
                Request::builder()
                    .method(http::Method::GET)
                    .uri(path)
                    .body(Empty::<Bytes>::new())?,
            )
            .await?)
    }

    #[tokio::test]
    async fn dedicated_thread_is_not_the_caller() {
        let upstream = start_mtls_server(MtlsServerOptions::default()).await;
        let supervisor = RouterSupervisor::spawn(config_for(
            &upstream.url,
            "127.0.0.1:0",
            upstream.certs.materials(),
        ));
        assert_ne!(supervisor.thread_id(), thread::current().id());
        supervisor.shutdown().await.ok();
    }

    #[tokio::test]
    async fn start_probes_before_binding_and_keeps_closed_reasons() {
        let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_addr = reserved.local_addr().unwrap().to_string();
        drop(reserved);

        let mut failed = start_mtls_server(MtlsServerOptions {
            status: 503,
            ..MtlsServerOptions::default()
        })
        .await;
        let mut config = config_for(
            &format!("{}/private?api_key=sk-supervisor-probe", failed.url),
            &listen_addr,
            failed.certs.materials(),
        );
        config.defaults.timeout = Duration::from_millis(200);
        let supervisor = RouterSupervisor::spawn(config);
        let error = supervisor.start().await.unwrap_err();
        failed.shutdown();
        assert_eq!(error.as_str(), "upstream_probe_failed");
        assert!(!error.to_string().contains("sk-supervisor-probe"));
        assert!(!error.to_string().contains("private"));
        let status = supervisor.status();
        assert_eq!(status.phase, RuntimePhase::Failed);
        assert!(!status.bound);
        assert_eq!(
            status.last_startup_error,
            Some(StartupReason::UpstreamProbeFailed)
        );
        assert!(TcpStream::connect(&listen_addr).await.is_err());
        supervisor.shutdown().await.ok();
    }

    #[tokio::test]
    async fn listen_failure_is_closed_and_occupied_port_is_not_stolen() {
        let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_addr = occupied.local_addr().unwrap().to_string();
        let upstream = start_mtls_server(MtlsServerOptions::default()).await;
        let supervisor = RouterSupervisor::spawn(config_for(
            &upstream.url,
            &listen_addr,
            upstream.certs.materials(),
        ));
        let error = supervisor.start().await.unwrap_err();
        assert_eq!(error.as_str(), "listen_failed");
        assert_eq!(error.code(), ErrorCode::RouterStartFailed);
        assert!(!supervisor.status().bound);
        drop(occupied);
        supervisor.shutdown().await.ok();
    }

    #[tokio::test]
    async fn start_stop_and_second_start_use_stable_codes() {
        let upstream = start_mtls_server(MtlsServerOptions::default()).await;
        let supervisor = RouterSupervisor::spawn(config_for(
            &upstream.url,
            "127.0.0.1:0",
            upstream.certs.materials(),
        ));
        let addr = supervisor.start().await.expect("start");
        assert_eq!(supervisor.status().phase, RuntimePhase::Running);
        assert!(supervisor.status().bound);
        let error = supervisor.start().await.unwrap_err();
        assert_eq!(error.as_str(), "router_already_running");
        assert_eq!(error.code(), ErrorCode::RouterAlreadyRunning);
        let version = http1_get(&format!("http://{addr}"), "/version").await;
        assert_eq!(version.status(), StatusCode::OK);
        supervisor.stop().await.expect("stop");
        assert_eq!(supervisor.status().phase, RuntimePhase::Stopped);
        assert!(!supervisor.status().bound);
        assert!(TcpStream::connect(&addr).await.is_err());
        supervisor.shutdown().await.ok();
    }

    #[tokio::test]
    async fn request_timeout_and_concurrency_do_not_block_control_plane() {
        let seen = Arc::new(AtomicU32::new(0));
        let upstream = start_mtls_handler({
            let seen = seen.clone();
            move |_request| {
                let seen = seen.clone();
                async move {
                    let n = seen.fetch_add(1, Ordering::SeqCst);
                    if n > 0 {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                    Response::builder()
                        .status(StatusCode::NO_CONTENT)
                        .body(boxed_bytes(Bytes::new()))
                        .expect("response")
                }
            }
        })
        .await;
        let mut config = config_for(&upstream.url, "127.0.0.1:0", upstream.certs.materials());
        config.limits.request_timeout = Duration::from_millis(60);
        config.limits.max_concurrent_requests = 1;
        let supervisor = RouterSupervisor::spawn(config);
        let addr = supervisor.start().await.expect("start");
        let first = tokio::spawn({
            let url = format!("http://{addr}");
            async move { http1_get(&url, "/v1/hang").await }
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_ne!(supervisor.status().phase, RuntimePhase::Failed);
        let _ = supervisor.status();
        let second = http1_get(&format!("http://{addr}"), "/v1/hang-2").await;
        let first = first.await.expect("join");
        assert_eq!(first.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(second.status(), StatusCode::GATEWAY_TIMEOUT);
        assert!(supervisor.status().max_active_requests <= 1);
        supervisor.shutdown().await.ok();
    }

    #[tokio::test]
    async fn panic_is_contained_and_control_plane_stays_up() {
        let upstream = start_mtls_server(MtlsServerOptions::default()).await;
        let supervisor = RouterSupervisor::spawn(config_for(
            &upstream.url,
            "127.0.0.1:0",
            upstream.certs.materials(),
        ));
        supervisor.inject_panic_for_test().await.expect("panic");
        let status = supervisor.status();
        assert_eq!(status.phase, RuntimePhase::Degraded);
        let failure = status.last_failure.expect("contained");
        assert_eq!(failure.kind, ContainedFailureKind::RouterPanic);
        assert_eq!(failure.code, ErrorCode::RouterDegraded);
        let encoded = serde_json::to_string(&failure).unwrap();
        assert_eq!(
            encoded,
            r#"{"kind":"router_panic","code":"ROUTER_DEGRADED"}"#
        );
        assert!(!encoded.contains("sk-supervisor-panic-canary"));
        assert_eq!(supervisor.status().phase, RuntimePhase::Degraded);
        supervisor.stop().await.expect("stop after panic");
        supervisor.shutdown().await.ok();
    }

    #[tokio::test]
    async fn bounded_command_queue_times_out_without_blocking_status() {
        let upstream = start_mtls_server(MtlsServerOptions::default()).await;
        let supervisor = RouterSupervisor::spawn(config_for(
            &upstream.url,
            "127.0.0.1:0",
            upstream.certs.materials(),
        ));
        let release = Arc::new(Notify::new());
        supervisor
            .hold_for_test(release.clone())
            .await
            .expect("hold");
        let blocked = supervisor.start().await.unwrap_err();
        assert_eq!(blocked.as_str(), "command_timeout");
        assert_eq!(supervisor.status().phase, RuntimePhase::Stopped);
        release.notify_waiters();
        supervisor.shutdown().await.ok();
    }

    #[tokio::test]
    async fn shutdown_timeout_is_contained() {
        let seen = Arc::new(AtomicU32::new(0));
        let upstream = start_mtls_handler({
            let seen = seen.clone();
            move |_request| {
                let seen = seen.clone();
                async move {
                    if seen.fetch_add(1, Ordering::SeqCst) > 0 {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                    Response::builder()
                        .status(StatusCode::NO_CONTENT)
                        .body(boxed_bytes(Bytes::new()))
                        .expect("response")
                }
            }
        })
        .await;
        let mut config = config_for(&upstream.url, "127.0.0.1:0", upstream.certs.materials());
        config.limits.request_timeout = Duration::from_secs(2);
        config.limits.shutdown_timeout = Duration::from_millis(20);
        config.limits.command_timeout = Duration::from_millis(200);
        let supervisor = RouterSupervisor::spawn(config);
        let addr = supervisor.start().await.expect("start");
        let pending = tokio::spawn({
            let url = format!("http://{addr}");
            async move { http1_get_result(&url, "/v1/slow").await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let error = supervisor.stop().await.unwrap_err();
        assert_eq!(error.as_str(), "shutdown_timeout");
        assert!(!supervisor.status().bound);
        let _ = pending.await;
        supervisor.shutdown().await.ok();
    }
}
