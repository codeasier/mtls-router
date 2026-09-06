use std::io;
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::manager_core::apikeyusage::{Period, REQUEST_TIMEOUT};
use crate::manager_core::httpget::{TestRequest, TestResponse, TestServer};
use crate::manager_core::process::{Identity, Status};
use crate::manager_core::types::Classification;
use crate::protocol::{ErrorCode, RouterOwner};

use super::channel::{trusted_state_matches, Channel, VERSION_BUDGET};
use super::coordinator::{Coordinator, StartOutcome, TrustedLifecycle, USAGE_ESTABLISH_BUDGET};
use super::listener::normalize_listener;
use super::{Binding, Discovery, RemoteVersion, RouterIdentity, StartedIdentity};

const CHANNEL_KEY: &str = "channel-key-canary-7831";

fn trusted_fixture(listener: &super::Listener) -> Discovery {
    Discovery {
        classification: Classification::ExternalCompatible,
        owner: "cli".into(),
        listen_addr: listener.router_base_url.clone(),
        version: RemoteVersion {
            pid: 91,
            deployment_id: "prod-a".into(),
            management_protocol_version: "4".into(),
        },
        state: RouterIdentity {
            pid: 91,
            owner: "cli".into(),
            listen_addr: listener.router_base_url.clone(),
            binary_path: "/router".into(),
            process_started_at: "start".into(),
            process_executable: "/router".into(),
            deployment_id: "prod-a".into(),
            management_protocol_version: "4".into(),
        },
    }
}

fn genuine_process(
    _: &Identity,
    _: &str,
) -> Result<Status, crate::manager_core::process::ProcessError> {
    Ok(Status::Genuine)
}

fn version_body() -> Vec<u8> {
    br#"{"pid":91,"deployment_id":"prod-a","management_protocol_version":"4"}"#.to_vec()
}

fn listener_for(url: &str) -> super::Listener {
    let host = url.trim_start_matches("http://");
    normalize_listener(host).expect("listener")
}

struct LifecycleStub {
    calls: AtomicUsize,
    owner: Mutex<Option<RouterOwner>>,
    remaining: Mutex<Option<Duration>>,
    state: StartedIdentity,
    error: Option<ErrorCode>,
}

impl LifecycleStub {
    fn new(state: StartedIdentity) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            owner: Mutex::new(None),
            remaining: Mutex::new(None),
            state,
            error: None,
        }
    }
}

impl TrustedLifecycle for LifecycleStub {
    fn start(&self, owner: RouterOwner, deadline: Instant) -> StartOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.owner.lock().unwrap() = Some(owner);
        *self.remaining.lock().unwrap() = deadline.checked_duration_since(Instant::now());
        StartOutcome {
            state: self.state.clone(),
            error: self.error.map(|code| {
                crate::manager_core::errors::closed_error(code, "router could not be started")
            }),
        }
    }
}

#[test]
fn normalize_listener_strict_numeric_loopback() {
    let cases = [
        (
            "127.0.0.1:19099",
            Some((
                "127.0.0.1:19099",
                "http://127.0.0.1:19099",
                "http://127.0.0.1:19099/v1",
            )),
        ),
        (
            "127.42.1.9:1",
            Some((
                "127.42.1.9:1",
                "http://127.42.1.9:1",
                "http://127.42.1.9:1/v1",
            )),
        ),
        (
            "[::1]:65535",
            Some(("[::1]:65535", "http://[::1]:65535", "http://[::1]:65535/v1")),
        ),
        ("localhost:19099", None),
        ("0.0.0.0:19099", None),
        ("192.168.1.2:19099", None),
        ("[::2]:19099", None),
        ("127.0.0.1:0", None),
        ("127.0.0.1:65536", None),
        ("127.0.0.1", None),
    ];
    for (input, want) in cases {
        match (normalize_listener(input), want) {
            (Ok(got), Some((authority, router, api))) => {
                assert_eq!(got.authority, authority);
                assert_eq!(got.router_base_url, router);
                assert_eq!(got.api_base_url, api);
            }
            (Err(_), None) => {}
            other => panic!("{input}: {other:?}"),
        }
    }
}

#[test]
fn channel_validates_and_fetches_on_exactly_one_connection() {
    let dials = Arc::new(AtomicUsize::new(0));
    let connections = Arc::new(Mutex::new(Vec::new()));
    let seen = connections.clone();
    let server = TestServer::start(move |request: &TestRequest| {
        seen.lock().unwrap().push(request.connection_id);
        match request.path.as_str() {
            "/version" => {
                assert!(request.authorization.is_empty());
                TestResponse::json(200, version_body())
            }
            "/v1/models" => {
                assert_eq!(request.authorization, format!("Bearer {CHANNEL_KEY}"));
                TestResponse::json(200, br#"{"data":[{"id":"model-a"}]}"#.to_vec())
            }
            _ => TestResponse::json(404, Vec::new()),
        }
    });
    let listener = listener_for(&server.url());
    let dial_count = dials.clone();
    let authority = listener.authority.clone();
    let models = Channel {
        simplify: false,
        dial: Some(Arc::new(move |addr| {
            assert_eq!(addr, authority);
            dial_count.fetch_add(1, Ordering::SeqCst);
            TcpStream::connect(addr)
        })),
        validate_process: Some(Arc::new(genuine_process)),
    }
    .fetch(
        &listener,
        &trusted_fixture(&listener),
        CHANNEL_KEY,
        Instant::now() + Duration::from_secs(5),
    )
    .expect("fetch");
    assert_eq!(models, ["model-a"]);
    let ids = connections.lock().unwrap().clone();
    assert_eq!(dials.load(Ordering::SeqCst), 1);
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1]);
}

#[test]
fn channel_fetches_usage_on_exactly_one_connection() {
    let dials = Arc::new(AtomicUsize::new(0));
    let connections = Arc::new(Mutex::new(Vec::new()));
    let seen = connections.clone();
    let server = TestServer::start(move |request: &TestRequest| {
        seen.lock().unwrap().push(request.connection_id);
        match request.path.as_str() {
            "/version" => {
                assert!(request.authorization.is_empty());
                TestResponse::json(200, version_body())
            }
            "/v1/usage" => {
                assert_eq!(request.query, "period=7d");
                assert_eq!(request.authorization, format!("Bearer {CHANNEL_KEY}"));
                TestResponse::json(
                    200,
                    br#"{"period":"7d","summary":{"requests":4,"prompt_tokens":8,"completion_tokens":2,"cost":0.5},"by_model":[]}"#.to_vec(),
                )
            }
            _ => TestResponse::json(404, Vec::new()),
        }
    });
    let listener = listener_for(&server.url());
    let dial_count = dials.clone();
    let snapshot = Channel {
        simplify: false,
        dial: Some(Arc::new(move |addr| {
            dial_count.fetch_add(1, Ordering::SeqCst);
            TcpStream::connect(addr)
        })),
        validate_process: Some(Arc::new(genuine_process)),
    }
    .fetch_usage(
        &listener,
        &trusted_fixture(&listener),
        Period::SevenDays,
        CHANNEL_KEY,
        Instant::now() + Duration::from_secs(5),
    )
    .expect("usage");
    assert_eq!(snapshot.summary.requests, 4);
    assert_eq!(snapshot.summary.cost, 0.5);
    let ids = connections.lock().unwrap().clone();
    assert_eq!(dials.load(Ordering::SeqCst), 1);
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1]);
}

#[test]
fn channel_simplifies_mixed_catalog() {
    let validations = Arc::new(AtomicU64::new(0));
    let server = TestServer::start(|request: &TestRequest| match request.path.as_str() {
        "/version" => {
            assert!(request.authorization.is_empty());
            TestResponse::json(200, version_body())
        }
        "/v1/models" => {
            assert_eq!(request.authorization, format!("Bearer {CHANNEL_KEY}"));
            TestResponse::json(
                200,
                br#"{"data":[{"id":"provider/model"},{"id":"model-a"}]}"#.to_vec(),
            )
        }
        _ => TestResponse::json(404, Vec::new()),
    });
    let listener = listener_for(&server.url());
    let calls = validations.clone();
    let models = Channel {
        simplify: true,
        dial: None,
        validate_process: Some(Arc::new(move |identity, path| {
            calls.fetch_add(1, Ordering::SeqCst);
            genuine_process(identity, path)
        })),
    }
    .fetch(
        &listener,
        &trusted_fixture(&listener),
        CHANNEL_KEY,
        Instant::now() + Duration::from_secs(5),
    )
    .expect("fetch");
    assert_eq!(models, ["model-a"]);
    assert_eq!(validations.load(Ordering::SeqCst), 1);
}

#[test]
fn channel_forced_redial_fails_before_key_transmission() {
    let model_requests = Arc::new(AtomicUsize::new(0));
    let key_observed = Arc::new(AtomicUsize::new(0));
    let models = model_requests.clone();
    let keys = key_observed.clone();
    let server = TestServer::start(move |request: &TestRequest| {
        if !request.authorization.is_empty() {
            keys.fetch_add(1, Ordering::SeqCst);
        }
        if request.path == "/version" {
            return TestResponse {
                status: 200,
                body: version_body(),
                connection_close: true,
            };
        }
        models.fetch_add(1, Ordering::SeqCst);
        TestResponse::json(200, Vec::new())
    });
    let listener = listener_for(&server.url());
    let error = Channel {
        validate_process: Some(Arc::new(genuine_process)),
        ..Channel::default()
    }
    .fetch(
        &listener,
        &trusted_fixture(&listener),
        CHANNEL_KEY,
        Instant::now() + Duration::from_secs(5),
    )
    .expect_err("redial");
    assert_eq!(error.code, ErrorCode::ModelDiscoveryFailed);
    assert_eq!(model_requests.load(Ordering::SeqCst), 0);
    assert_eq!(key_observed.load(Ordering::SeqCst), 0);
}

#[test]
fn channel_rejects_process_swap_before_key_transmission() {
    let key_observed = Arc::new(AtomicUsize::new(0));
    let keys = key_observed.clone();
    let server = TestServer::start(move |request: &TestRequest| {
        if !request.authorization.is_empty() {
            keys.fetch_add(1, Ordering::SeqCst);
        }
        TestResponse::json(200, version_body())
    });
    let listener = listener_for(&server.url());
    let error = Channel {
        validate_process: Some(Arc::new(|_, _| Ok(Status::Stale))),
        ..Channel::default()
    }
    .fetch(
        &listener,
        &trusted_fixture(&listener),
        CHANNEL_KEY,
        Instant::now() + Duration::from_secs(5),
    )
    .expect_err("stale process");
    assert_eq!(error.code, ErrorCode::ModelCatalogStale);
    assert_eq!(key_observed.load(Ordering::SeqCst), 0);
}

#[test]
fn channel_rejects_version_identity_mismatch_before_key_transmission() {
    let key_observed = Arc::new(AtomicUsize::new(0));
    let keys = key_observed.clone();
    let server = TestServer::start(move |request: &TestRequest| {
        if !request.authorization.is_empty() {
            keys.fetch_add(1, Ordering::SeqCst);
        }
        TestResponse::json(
            200,
            br#"{"pid":92,"deployment_id":"other","management_protocol_version":"1"}"#.to_vec(),
        )
    });
    let listener = listener_for(&server.url());
    let error = Channel {
        validate_process: Some(Arc::new(genuine_process)),
        ..Channel::default()
    }
    .fetch(
        &listener,
        &trusted_fixture(&listener),
        CHANNEL_KEY,
        Instant::now() + Duration::from_secs(5),
    )
    .expect_err("mismatch");
    assert_eq!(error.code, ErrorCode::ModelCatalogStale);
    assert_eq!(key_observed.load(Ordering::SeqCst), 0);
}

#[test]
fn channel_rejects_non_ok_version_before_key_transmission() {
    for (name, status) in [("redirect", 302), ("upgrade", 101)] {
        let key_observed = Arc::new(AtomicUsize::new(0));
        let keys = key_observed.clone();
        let server = TestServer::start(move |request: &TestRequest| {
            if !request.authorization.is_empty() {
                keys.fetch_add(1, Ordering::SeqCst);
            }
            TestResponse::json(status, Vec::new())
        });
        let listener = listener_for(&server.url());
        let error = Channel {
            validate_process: Some(Arc::new(genuine_process)),
            ..Channel::default()
        }
        .fetch(
            &listener,
            &trusted_fixture(&listener),
            CHANNEL_KEY,
            Instant::now() + Duration::from_secs(5),
        )
        .expect_err(name);
        assert_eq!(error.code, ErrorCode::ModelDiscoveryFailed);
        assert_eq!(key_observed.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn coordinator_absent_starts_once_under_requested_owner() {
    let listener = normalize_listener("127.0.0.1:19099").unwrap();
    let trusted = trusted_fixture(&listener);
    let discoveries = Arc::new(AtomicUsize::new(0));
    let seen = discoveries.clone();
    let after = trusted.clone();
    let starter = Arc::new(LifecycleStub::new(StartedIdentity {
        pid: trusted.state.pid,
        owner: trusted.state.owner.clone(),
        listen_addr: trusted.state.listen_addr.clone(),
        deployment_id: trusted.state.deployment_id.clone(),
        management_protocol_version: trusted.state.management_protocol_version.clone(),
    }));
    let coordinator = Coordinator {
        listener: listener.clone(),
        deployment_id: "prod-a".into(),
        protocol_version: "4".into(),
        discover: Arc::new(move || {
            let count = seen.fetch_add(1, Ordering::SeqCst);
            if count == 0 {
                Discovery::absent()
            } else {
                after.clone()
            }
        }),
        lifecycle: starter.clone(),
        channel: Channel {
            dial: Some(Arc::new(|_: &str| {
                Err(io::Error::other("stop after trust test"))
            })),
            validate_process: Some(Arc::new(genuine_process)),
            simplify: false,
        },
        desktop_eligible: Arc::new(|| true),
        absent_start_ok: Arc::new(|| true),
    };
    let error = coordinator
        .fetch(
            RouterOwner::Desktop,
            "secret",
            Instant::now() + Duration::from_secs(5),
        )
        .expect_err("dial fail");
    assert_eq!(error.code, ErrorCode::ModelDiscoveryFailed);
    assert_eq!(starter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(*starter.owner.lock().unwrap(), Some(RouterOwner::Desktop));
    assert_eq!(discoveries.load(Ordering::SeqCst), 2);
}

#[test]
fn coordinator_fetch_usage_gives_start_its_own_budget() {
    let listener = normalize_listener("127.0.0.1:19099").unwrap();
    let trusted = trusted_fixture(&listener);
    let discoveries = Arc::new(AtomicUsize::new(0));
    let seen = discoveries.clone();
    let after = trusted.clone();
    let starter = Arc::new(LifecycleStub::new(StartedIdentity {
        pid: trusted.state.pid,
        owner: trusted.state.owner.clone(),
        listen_addr: trusted.state.listen_addr.clone(),
        deployment_id: trusted.state.deployment_id.clone(),
        management_protocol_version: trusted.state.management_protocol_version.clone(),
    }));
    let coordinator = Coordinator {
        listener: listener.clone(),
        deployment_id: "prod-a".into(),
        protocol_version: "4".into(),
        discover: Arc::new(move || {
            if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                Discovery::absent()
            } else {
                after.clone()
            }
        }),
        lifecycle: starter.clone(),
        channel: Channel {
            dial: Some(Arc::new(|_: &str| Err(io::Error::other("stop")))),
            validate_process: Some(Arc::new(genuine_process)),
            simplify: false,
        },
        desktop_eligible: Arc::new(|| true),
        absent_start_ok: Arc::new(|| true),
    };
    let _ = coordinator.fetch_usage(
        RouterOwner::Desktop,
        Period::SevenDays,
        "secret",
        Instant::now() + Duration::from_secs(60),
    );
    let remaining = starter.remaining.lock().unwrap().expect("deadline");
    assert!(
        remaining >= Duration::from_secs(19) && remaining <= Duration::from_secs(21),
        "{remaining:?}"
    );
}

#[test]
fn coordinator_rejects_unsafe_states_without_start() {
    let listener = normalize_listener("127.0.0.1:19099").unwrap();
    for (name, classification, start_ok, code) in [
        (
            "unknown",
            Classification::UnknownOccupant,
            true,
            ErrorCode::PortOccupied,
        ),
        (
            "stale",
            Classification::Stale,
            true,
            ErrorCode::RouterStateStale,
        ),
        (
            "legacy",
            Classification::LegacyManaged,
            true,
            ErrorCode::RouterLegacyManaged,
        ),
        (
            "unexpected exit latch",
            Classification::Absent,
            false,
            ErrorCode::RouterStateStale,
        ),
    ] {
        let starter = Arc::new(LifecycleStub::new(StartedIdentity::default()));
        let coordinator = Coordinator {
            listener: listener.clone(),
            deployment_id: "prod-a".into(),
            protocol_version: "4".into(),
            discover: Arc::new(move || Discovery {
                classification,
                ..Discovery::absent()
            }),
            lifecycle: starter.clone(),
            channel: Channel::default(),
            desktop_eligible: Arc::new(|| true),
            absent_start_ok: Arc::new(move || start_ok),
        };
        let error = coordinator
            .fetch(
                RouterOwner::Cli,
                "secret",
                Instant::now() + Duration::from_secs(2),
            )
            .expect_err(name);
        assert_eq!(error.code, code, "{name}");
        assert_eq!(starter.calls.load(Ordering::SeqCst), 0, "{name}");
    }
}

#[test]
fn coordinator_rejects_desktop_owner_before_discovery() {
    let discoveries = Arc::new(AtomicUsize::new(0));
    let seen = discoveries.clone();
    let coordinator = Coordinator {
        listener: normalize_listener("127.0.0.1:19099").unwrap(),
        deployment_id: "prod-a".into(),
        protocol_version: "4".into(),
        discover: Arc::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
            Discovery::absent()
        }),
        lifecycle: Arc::new(LifecycleStub::new(StartedIdentity::default())),
        channel: Channel::default(),
        desktop_eligible: Arc::new(|| false),
        absent_start_ok: Arc::new(|| true),
    };
    let error = coordinator
        .fetch(
            RouterOwner::Desktop,
            CHANNEL_KEY,
            Instant::now() + Duration::from_secs(2),
        )
        .expect_err("ineligible");
    assert_eq!(error.code, ErrorCode::InvalidParams);
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
}

#[test]
fn coordinator_rejects_changed_write_binding_before_discovery() {
    let discoveries = Arc::new(AtomicUsize::new(0));
    let seen = discoveries.clone();
    let listener = normalize_listener("127.0.0.1:19099").unwrap();
    let coordinator = Coordinator {
        listener: listener.clone(),
        deployment_id: "prod-a".into(),
        protocol_version: "4".into(),
        discover: Arc::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
            Discovery::absent()
        }),
        lifecycle: Arc::new(LifecycleStub::new(StartedIdentity::default())),
        channel: Channel::default(),
        desktop_eligible: Arc::new(|| true),
        absent_start_ok: Arc::new(|| true),
    };
    let error = coordinator
        .revalidate(
            RouterOwner::Cli,
            CHANNEL_KEY,
            &Binding {
                router_base_url: "http://127.0.0.1:19100".into(),
                api_base_url: "http://127.0.0.1:19100/v1".into(),
                deployment_id: "prod-a".into(),
                protocol_version: "4".into(),
            },
            Instant::now() + Duration::from_secs(2),
        )
        .expect_err("binding");
    assert_eq!(error.code, ErrorCode::ModelCatalogStale);
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
}

#[test]
fn trusted_state_matches_only_exact_owned_or_degraded_binding() {
    let listener = normalize_listener("127.0.0.1:19099").unwrap();
    let base = trusted_fixture(&listener);
    for classification in [
        Classification::ExternalCompatible,
        Classification::DesktopOwned,
        Classification::Degraded,
    ] {
        let mut found = base.clone();
        found.classification = classification;
        if classification == Classification::DesktopOwned {
            found.owner = "desktop".into();
            found.state.owner = "desktop".into();
        }
        assert!(
            trusted_state_matches(&found, &listener, "prod-a", "4"),
            "{classification:?}"
        );
    }
    for mutate in [
        |found: &mut Discovery| found.listen_addr = "http://127.0.0.1:19100".into(),
        |found: &mut Discovery| found.state.listen_addr = "http://127.0.0.1:19100".into(),
        |found: &mut Discovery| found.version.deployment_id = "other".into(),
        |found: &mut Discovery| found.state.deployment_id = "other".into(),
        |found: &mut Discovery| found.version.management_protocol_version = "1".into(),
        |found: &mut Discovery| found.state.management_protocol_version = "1".into(),
        |found: &mut Discovery| found.state.owner = "unknown".into(),
    ] {
        let mut found = base.clone();
        mutate(&mut found);
        assert!(
            !trusted_state_matches(&found, &listener, "prod-a", "4"),
            "{found:?}"
        );
    }
}

#[test]
fn apikey_usage_deadline_covers_independent_budgets() {
    assert_eq!(
        crate::protocol::deadline(crate::protocol::Method::ApiKeyUsage),
        USAGE_ESTABLISH_BUDGET + VERSION_BUDGET + REQUEST_TIMEOUT
    );
}
