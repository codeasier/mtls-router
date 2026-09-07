//! Manager session behind the existing Tauri transport API.
//!
//! Assembles occupant identity, state-file protection, isolated or desktop
//! Agent state, optional trusted-router HTTP, and (in production) the
//! embedded router backend plus the legacy migration runtime. The desktop
//! runtime serves protocol v4 in-process through [`production_factory`]; the
//! fixture factory shares the same host for contract tests.
//!
//! Blocking Agent / occupant / trusted HTTP work stays on a dedicated session
//! thread so the Tauri async runtime is not stalled.

use std::sync::{mpsc, Arc};
use std::thread;

use base64::Engine;
use serde_json::{json, Value};

use crate::error::CommandError;
use crate::manager::{TransportChild, TransportEvent, TransportFactory, TransportSession};
use crate::protocol::{
    ErrorCode, Method, ProtocolError, Request, Response, MANAGEMENT_PROTOCOL_VERSION,
};
use crate::redaction::sanitize_protocol_error;

use super::agent::modelconfig::{decode_structural, Config as PresetConfig, MAX_CONFIG_SIZE};
use super::agent::{
    isolated_service, AgentService, Detector, Options as AgentOptions, TrustedCatalog,
};
use super::backend::FakeBackend;
use super::embedded::{EmbeddedBackend, EmbeddedLifecycle};
use super::errors::{closed_error, invalid_params, unknown_method};
use super::legacy::{CurrentLineage, LegacyRuntime};
use super::lifecycle::{Backend, Manager};
use super::metadata;
use super::occupant::{OccupantConfig, OccupantService};
use super::paths::Paths;
use super::process::{self, Identity as ProcessIdentity, Status};
use super::trustedrouter::{normalize_listener, Channel, Coordinator, Listener, TrustedLifecycle};

pub const DEFAULT_LISTEN: &str = "127.0.0.1:19099";

#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub listen_addr: String,
    pub desktop_session: String,
    pub parent: ProcessIdentity,
    pub manager_identity: ProcessIdentity,
    pub paths: Paths,
    pub isolate_agent: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            listen_addr: DEFAULT_LISTEN.to_owned(),
            desktop_session: String::new(),
            parent: ProcessIdentity::default(),
            manager_identity: ProcessIdentity::default(),
            paths: Paths::resolve().unwrap_or_else(|_| isolated_paths()),
            isolate_agent: true,
        }
    }
}

fn isolated_paths() -> Paths {
    let root = std::env::temp_dir().join(format!(
        "mtls-manager-session-{}",
        uuid::Uuid::new_v4().simple()
    ));
    super::paths::resolve_with("linux", &root, |_| String::new())
}

pub fn fixture_config() -> SessionConfig {
    let paths = isolated_paths();
    let _ = std::fs::create_dir_all(&paths.desktop_data_dir);
    let identity = current_identity().unwrap_or_default();
    SessionConfig {
        listen_addr: DEFAULT_LISTEN.to_owned(),
        desktop_session: String::new(),
        parent: identity.clone(),
        manager_identity: identity,
        paths,
        isolate_agent: true,
    }
}

pub fn current_identity() -> std::result::Result<ProcessIdentity, process::ProcessError> {
    let pid = i32::try_from(std::process::id()).map_err(|_| process::ProcessError::Io)?;
    process::inspect(pid)
}

pub fn complete_identity(value: &ProcessIdentity) -> bool {
    value.pid > 0 && !value.started_at.is_empty() && !value.executable.is_empty()
}

pub fn desktop_eligible(desktop_session: &str, parent: &ProcessIdentity) -> bool {
    if desktop_session.is_empty() || !complete_identity(parent) {
        return false;
    }
    matches!(
        process::validate(parent, &parent.executable),
        Ok(Status::Genuine)
    )
}

pub fn resolve_listen(listen_addr: &str) -> std::result::Result<Listener, ProtocolError> {
    let value = if listen_addr.trim().is_empty() {
        DEFAULT_LISTEN
    } else {
        listen_addr
    };
    normalize_listener(value).map_err(invalid_params)
}

pub fn resolve_manager_identity(
    configured: &ProcessIdentity,
) -> std::result::Result<ProcessIdentity, process::ProcessError> {
    if configured.pid == 0 {
        current_identity()
    } else {
        Ok(configured.clone())
    }
}

pub fn occupant_for(config: &SessionConfig, listen: &Listener) -> OccupantService {
    let manager_identity = resolve_manager_identity(&config.manager_identity).unwrap_or_default();
    OccupantService::new(
        OccupantConfig {
            listen_addr: listen.authority.clone(),
            desktop_pid: config.parent.pid,
            manager_identity,
            state_paths: config.paths.state_files().to_vec(),
            ..OccupantConfig::default()
        },
        Default::default(),
    )
}

pub fn agent_for(config: &SessionConfig) -> AgentService {
    if config.isolate_agent {
        return isolated_service();
    }
    AgentService::new(AgentOptions {
        state_dir: config.paths.agent_state_dir(),
        detector: super::agent::Detector::default(),
        preset: None,
        simplify: false,
    })
    .unwrap_or_else(|_| isolated_service())
}

pub fn fixture_manager() -> Manager<FakeBackend> {
    manager_with(fixture_config(), FakeBackend::default(), None)
}

pub fn manager_with<B: Backend>(
    config: SessionConfig,
    backend: B,
    trusted: Option<Arc<dyn TrustedCatalog>>,
) -> Manager<B> {
    let listen = resolve_listen(&config.listen_addr).expect("fixture listen");
    let occupant = occupant_for(&config, &listen);
    let agent = agent_for(&config);
    let mut manager = Manager::new(metadata::info(), backend)
        .with_occupant(occupant)
        .with_agent(agent)
        .with_desktop_log_path(config.paths.desktop_log_file.clone());
    if let Some(trusted) = trusted {
        manager = manager.with_trusted(trusted);
    }
    manager
}

/// Builds a trusted coordinator only when a verified desktop session exists.
/// Fixtures keep `trusted: None` unless a test injects `StaticCatalog`.
pub fn production_trusted(
    config: &SessionConfig,
    discover: impl Fn() -> super::trustedrouter::Discovery + Send + Sync + 'static,
    lifecycle: Arc<dyn TrustedLifecycle>,
    absent_start_ok: impl Fn() -> bool + Send + Sync + 'static,
    simplify: bool,
) -> Option<Arc<dyn TrustedCatalog>> {
    let listen = resolve_listen(&config.listen_addr).ok()?;
    if !desktop_eligible(&config.desktop_session, &config.parent) {
        return None;
    }
    Some(Arc::new(Coordinator {
        listener: listen,
        deployment_id: env!("MTLS_DEPLOYMENT_ID").to_owned(),
        protocol_version: MANAGEMENT_PROTOCOL_VERSION.to_owned(),
        discover: Arc::new(discover),
        lifecycle,
        channel: Channel {
            simplify,
            ..Channel::default()
        },
        desktop_eligible: Arc::new({
            let session = config.desktop_session.clone();
            let parent = config.parent.clone();
            move || desktop_eligible(&session, &parent)
        }),
        absent_start_ok: Arc::new(absent_start_ok),
    }))
}

/// Go `preset.Load`: strict standard base64, size-bounded, structurally
/// validated. The error carries nothing so preset contents cannot leak.
pub fn load_embedded_preset(encoded: &str) -> Result<Option<PresetConfig>, ()> {
    if encoded.is_empty() {
        return Ok(None);
    }
    let max_encoded = MAX_CONFIG_SIZE.div_ceil(3) * 4;
    if encoded.len() > max_encoded
        || encoded
            .chars()
            .any(|value| matches!(value, '\r' | '\n' | ' ' | '\t'))
    {
        return Err(());
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| ())?;
    if decoded.len() > MAX_CONFIG_SIZE {
        return Err(());
    }
    decode_structural(&decoded).map(Some).map_err(|_| ())
}

/// Everything one desktop process shares across transport sessions: the
/// embedded backend must outlive any session respawn so the bound router is
/// never dropped by recovery.
pub struct ProductionRuntime {
    pub session: SessionConfig,
    pub current: CurrentLineage,
    pub backend: Arc<EmbeddedBackend>,
    pub preset: Option<PresetConfig>,
    pub simplify: bool,
}

pub fn production_manager(runtime: &ProductionRuntime) -> Manager<Arc<EmbeddedBackend>> {
    let listen = resolve_listen(&runtime.session.listen_addr).expect("production listen");
    let occupant = occupant_for(&runtime.session, &listen);
    let agent = AgentService::new(AgentOptions {
        state_dir: runtime.session.paths.agent_state_dir(),
        detector: Detector::default(),
        preset: runtime.preset.clone(),
        simplify: runtime.simplify,
    })
    .unwrap_or_else(|_| isolated_service());
    let manager = Manager::new(metadata::info(), runtime.backend.clone())
        .with_occupant(occupant)
        .with_agent(agent)
        .with_desktop_log_path(runtime.session.paths.desktop_log_file.clone())
        .with_legacy(LegacyRuntime::new(
            runtime.current.clone(),
            runtime.session.paths.desktop_state_file.clone(),
            runtime.session.paths.legacy_migration_file(),
        ));
    let discover_backend = runtime.backend.clone();
    let trusted = production_trusted(
        &runtime.session,
        move || discover_backend.trusted_discovery(),
        Arc::new(EmbeddedLifecycle(runtime.backend.clone())),
        manager.absent_start_ok_probe(),
        runtime.simplify,
    );
    match trusted {
        Some(trusted) => manager.with_trusted(trusted),
        None => manager,
    }
}

pub fn production_factory(
    runtime: Arc<ProductionRuntime>,
) -> InProcessFactory<Arc<EmbeddedBackend>> {
    InProcessFactory::new(move || production_manager(&runtime))
}

pub struct InProcessFactory<B> {
    spawn_manager: Arc<dyn Fn() -> Manager<B> + Send + Sync>,
}

impl InProcessFactory<FakeBackend> {
    pub fn fixture() -> Self {
        Self {
            spawn_manager: Arc::new(fixture_manager),
        }
    }
}

impl<B: Backend + 'static> InProcessFactory<B> {
    pub fn new<F>(spawn_manager: F) -> Self
    where
        F: Fn() -> Manager<B> + Send + Sync + 'static,
    {
        Self {
            spawn_manager: Arc::new(spawn_manager),
        }
    }
}

impl<B: Backend + 'static> TransportFactory for InProcessFactory<B> {
    fn spawn(&self) -> crate::error::Result<TransportSession> {
        let manager = (self.spawn_manager)();
        start_session(manager)
    }
}

struct InProcessChild {
    requests: mpsc::Sender<Vec<u8>>,
}

impl TransportChild for InProcessChild {
    fn write(&mut self, bytes: &[u8]) -> crate::error::Result<()> {
        self.requests
            .send(bytes.to_vec())
            .map_err(|_| CommandError::manager_failed())
    }

    fn kill(self: Box<Self>) {
        drop(self.requests);
    }
}

fn start_session<B: Backend + 'static>(
    manager: Manager<B>,
) -> crate::error::Result<TransportSession> {
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(16);
    let (req_tx, req_rx) = mpsc::channel::<Vec<u8>>();
    thread::Builder::new()
        .name("mtls-manager-core".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("manager-core runtime");
            while let Ok(bytes) = req_rx.recv() {
                let response = runtime.block_on(serve_line(&manager, &bytes));
                if runtime
                    .block_on(event_tx.send(TransportEvent::Stdout(response)))
                    .is_err()
                {
                    break;
                }
            }
        })
        .map_err(|_| CommandError::manager_failed())?;
    Ok(TransportSession {
        child: Box::new(InProcessChild { requests: req_tx }),
        events: event_rx,
    })
}

async fn serve_line<B: Backend>(manager: &Manager<B>, bytes: &[u8]) -> Vec<u8> {
    let request = match parse_request(bytes) {
        Ok(request) => request,
        Err(response) => return encode_response(response),
    };
    encode_response(manager.handle(request).await)
}

fn parse_request(bytes: &[u8]) -> std::result::Result<Request, Response> {
    let bytes = trim_request(bytes);
    let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid_request(None))?;
    let id = value.get("id").and_then(Value::as_str).map(str::to_owned);
    match serde_json::from_value::<Request>(value.clone()) {
        Ok(request) => Ok(request),
        Err(_) => {
            let method = value.get("method").and_then(Value::as_str).unwrap_or("");
            if !method.is_empty() && parse_method(method).is_none() {
                Err(Response {
                    id,
                    result: None,
                    error: Some(unknown_method()),
                })
            } else {
                Err(invalid_request(id))
            }
        }
    }
}

fn parse_method(method: &str) -> Option<Method> {
    serde_json::from_value(json!(method)).ok()
}

fn trim_request(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && matches!(bytes[end - 1], b'\n' | b'\r' | b' ' | b'\t') {
        end -= 1;
    }
    &bytes[..end]
}

fn invalid_request(id: Option<String>) -> Response {
    Response {
        id,
        result: None,
        error: Some(closed_error(ErrorCode::InvalidRequest, "invalid request")),
    }
}

fn encode_response(mut response: Response) -> Vec<u8> {
    if let Some(error) = response.error.take() {
        response.error = Some(sanitize_protocol_error(
            error.code,
            &error.message,
            error.details,
        ));
    }
    serde_json::to_vec(&response).expect("protocol response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::ManagerClient;
    use crate::manager_core::agent::StaticCatalog;
    use crate::manager_core::apikeyusage::{Quota, QuotaUnit, Snapshot, Summary};
    use crate::manager_core::trustedrouter::{Discovery, StartOutcome};
    use crate::protocol::RouterOwner;
    use crate::types::{ManagerInfo, RouterStatus};

    fn runtime_block<F: std::future::Future>(future: F) -> F::Output {
        tauri::async_runtime::block_on(future)
    }

    struct RejectLifecycle;

    impl TrustedLifecycle for RejectLifecycle {
        fn start(&self, _owner: RouterOwner, _deadline: std::time::Instant) -> StartOutcome {
            StartOutcome {
                state: super::super::trustedrouter::StartedIdentity::default(),
                error: Some(closed_error(
                    ErrorCode::RouterNotFound,
                    "router could not be safely started",
                )),
            }
        }
    }

    #[test]
    fn desktop_eligible_requires_session_and_genuine_parent() {
        let identity = current_identity().expect("current process identity");
        assert!(desktop_eligible("desktop-session", &identity));
        assert!(!desktop_eligible("", &identity));
        assert!(!desktop_eligible(
            "desktop-session",
            &ProcessIdentity::default()
        ));
    }

    #[test]
    fn listen_defaults_to_loopback_and_rejects_non_loopback() {
        let listen = resolve_listen("").expect("default listen");
        assert_eq!(listen.authority, "127.0.0.1:19099");
        assert!(resolve_listen("10.0.0.1:19099").is_err());
    }

    #[test]
    fn production_trusted_stays_none_without_verified_desktop() {
        let config = fixture_config();
        assert!(production_trusted(
            &config,
            Discovery::absent,
            Arc::new(RejectLifecycle),
            || true,
            false
        )
        .is_none());
    }

    #[test]
    fn production_trusted_attaches_when_desktop_session_is_genuine() {
        let mut config = fixture_config();
        config.desktop_session = "desktop-session".into();
        config.parent = current_identity().expect("current process identity");
        assert!(production_trusted(
            &config,
            Discovery::absent,
            Arc::new(RejectLifecycle),
            || true,
            false
        )
        .is_some());
    }

    #[test]
    fn embedded_preset_loader_matches_go_strictness() {
        assert_eq!(load_embedded_preset(""), Ok(None));
        let valid = base64::engine::general_purpose::STANDARD
            .encode(br#"{"version":1,"opencode":{"default_model":"m","models":{"m":{}}}}"#);
        assert!(load_embedded_preset(&valid).unwrap().is_some());
        let padded = format!("{valid}\n");
        assert_eq!(load_embedded_preset(&padded), Err(()));
        assert_eq!(load_embedded_preset("not*base64"), Err(()));
        let junk = base64::engine::general_purpose::STANDARD.encode(b"[]");
        assert_eq!(load_embedded_preset(&junk), Err(()));
        let oversized = "A".repeat(MAX_CONFIG_SIZE.div_ceil(3) * 4 + 4);
        assert_eq!(load_embedded_preset(&oversized), Err(()));
    }

    #[test]
    fn manager_client_serves_protocol_v4_fixtures() {
        runtime_block(async {
            let client = ManagerClient::new(Arc::new(InProcessFactory::fixture()));
            let info: ManagerInfo = client.call("manager.info", json!({})).await.unwrap();
            crate::manager::validate_handshake(&info).unwrap();
            assert_eq!(
                info.management_protocol_version,
                MANAGEMENT_PROTOCOL_VERSION
            );

            let status: RouterStatus = client.call("router.status", json!({})).await.unwrap();
            assert_eq!(status.state, "absent");

            let occupant = client
                .call::<Value>("router.inspect_occupant", json!({}))
                .await
                .unwrap_err();
            assert_eq!(occupant.code, "OCCUPANT_NOT_FOUND");

            let extra = client
                .call::<Value>("manager.info", json!({"extra": true}))
                .await
                .unwrap_err();
            assert_eq!(extra.code, "INVALID_PARAMS");

            let models = client
                .call::<Value>(
                    "agent.models",
                    json!({
                        "agents": ["claude"],
                        "owner": "desktop",
                        "api_key": "sk-fixture-not-logged"
                    }),
                )
                .await
                .unwrap_err();
            assert_eq!(models.code, "MODEL_DISCOVERY_FAILED");
            assert_eq!(models.message, "model discovery is unavailable");

            let usage = client
                .call::<Value>(
                    "apikey.usage",
                    json!({
                        "owner": "desktop",
                        "period": "7d",
                        "api_key": "sk-fixture-not-logged"
                    }),
                )
                .await
                .unwrap_err();
            assert_eq!(usage.code, "USAGE_UNAVAILABLE");
            assert_eq!(usage.message, "usage is unavailable");

            let migrate = client
                .call::<Value>("router.migrate_legacy", json!({}))
                .await
                .unwrap_err();
            assert_eq!(migrate.code, "INVALID_REQUEST");
            assert_eq!(migrate.message, "method is unavailable");
        });
    }

    #[test]
    fn inprocess_parse_maps_unknown_method_and_malformed_json() {
        let unknown = parse_request(br#"{"id":"x","method":"not.a.method"}"#).unwrap_err();
        assert_eq!(unknown.id.as_deref(), Some("x"));
        assert_eq!(
            unknown.error.as_ref().map(|error| error.code),
            Some(ErrorCode::UnknownMethod)
        );
        let malformed = parse_request(b"{not-json}").unwrap_err();
        assert_eq!(
            malformed.error.as_ref().map(|error| error.code),
            Some(ErrorCode::InvalidRequest)
        );
    }

    #[test]
    fn manager_client_agent_detect_uses_isolated_home() {
        runtime_block(async {
            let client = ManagerClient::new(Arc::new(InProcessFactory::fixture()));
            let detect: Value = client.call("agent.detect", json!({})).await.unwrap();
            let names: Vec<_> = detect["agents"]
                .as_array()
                .expect("agents")
                .iter()
                .map(|agent| agent["agent"].as_str().unwrap_or_default())
                .collect();
            assert_eq!(names, ["claude", "opencode", "codex"]);
            assert!(detect["agents"]
                .as_array()
                .expect("agents")
                .iter()
                .all(|agent| agent["exists"] == false));
        });
    }

    #[test]
    fn injected_trusted_catalog_serves_usage_through_manager_client() {
        runtime_block(async {
            let factory = InProcessFactory::new(|| {
                let mut catalog = StaticCatalog::default();
                catalog.usage = Some(Snapshot {
                    period: super::super::apikeyusage::Period::SevenDays,
                    as_of: "2026-09-06T00:00:00Z".into(),
                    summary: Summary {
                        requests: 1,
                        prompt_tokens: 2,
                        completion_tokens: 3,
                        cost: 0.5,
                    },
                    quota: Some(Quota {
                        used: 0.5,
                        limit: Some(10.0),
                        unit: QuotaUnit::Usd,
                        resets_at: String::new(),
                    }),
                    quotas: Vec::new(),
                    by_model: Vec::new(),
                });
                manager_with(
                    fixture_config(),
                    FakeBackend::default(),
                    Some(Arc::new(catalog)),
                )
            });
            let client = ManagerClient::new(Arc::new(factory));
            let usage: Value = client
                .call(
                    "apikey.usage",
                    json!({
                        "owner": "desktop",
                        "period": "7d",
                        "api_key": "sk-fixture-not-logged"
                    }),
                )
                .await
                .unwrap();
            assert_eq!(usage["period"], "7d");
            assert_eq!(usage["summary"]["requests"], 1);
            assert_eq!(usage["quota"]["unit"], "usd");
        });
    }

    #[test]
    fn fixture_occupant_records_current_desktop_and_manager_identity() {
        let config = fixture_config();
        let identity = current_identity().expect("current process identity");
        assert_eq!(config.parent.pid, identity.pid);
        assert_eq!(config.manager_identity.pid, identity.pid);
        assert!(!config.paths.desktop_state_file.as_os_str().is_empty());
        assert!(!config.paths.cli_state_file.as_os_str().is_empty());
    }
}
