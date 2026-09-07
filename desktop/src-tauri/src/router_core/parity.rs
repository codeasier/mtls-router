//! Black-box parity with the frozen Go router HTTP contract.
//!
//! This suite covers routing, hop-by-hop / secret redaction, SSE flush,
//! exact `/version` and `/health`, probe-before-bind, and closed 502s —
//! the fixtures from `internal/proxy/service_contract_test.go` and
//! `internal/routermeta/handlers_test.go`. Tests speak only to the
//! supervisor listen port.
//!
//! It does not claim connection-management or timing identity:
//! - Upstream connections are dialed per request (no keep-alive pool).
//!   Go `http.Transport` reuses idle connections.
//! - Proxied requests have a supervisor [`super::DEFAULT_REQUEST_TIMEOUT`]
//!   covering connect + send + response headers. Go sets no
//!   `ResponseHeaderTimeout`. This is in-process isolation, not the probe
//!   budget ([`super::DEFAULT_TIMEOUT`] / `-timeout` / `MTLS_TIMEOUT`).
//! - Proxied requests are capped at [`super::DEFAULT_MAX_CONCURRENT`] until
//!   response headers. Go has no equivalent.
//!
//! Exact `/version` and `/health` are exempt from both the request timeout
//! and the concurrency cap so `/health` cannot fail at the HTTP layer
//! under load or a slow probe (probe failure still returns 200 + degraded).

use super::proxy::ProxyLogger;
use super::test_support::{
    boxed_bytes, boxed_channel, start_mtls_optional_handler, start_mtls_server, MtlsServerOptions,
};
use super::{
    BuildInfo, ProxyLog, RouterDefaults, RouterEnv, RouterFlags, RouterSupervisor, RuntimePhase,
    StartupReason, SupervisorConfig, SupervisorLimits, HEALTH_PATH, VERSION_PATH,
};
use bytes::Bytes;
use http::header::{self, HeaderMap, HeaderName, HeaderValue};
use http::{Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Notify};
use tokio::time::timeout;

const MODEL: &str = "advertised-model-fixture";
const BEARER: &str = "Bearer sk-fixture-auth-4d5e6f7g8h9i";
const REQUEST_BODY: &str = "request-body-canary-1a2b3c";
const RESPONSE_BODY: &str = "response-body-canary-0j1k2l";
const QUERY: &str = "query-canary-3m4n5o";
const MESSAGES_BODY: &str = r#"{"model":"advertised-model-fixture","stream":true,"messages":[{"role":"user","content":"request-body-canary-1a2b3c"}],"tools":[{"type":"future_anthropic_tool_20991231","name":"deferred_fixture","description":"open-list tool fixture","defer_loading":true,"input_schema":{"type":"object","properties":{"value":{"type":"string"}}},"input_examples":[{"value":"deferred-field-canary"}]}]}"#;
const COUNT_TOKENS_BODY: &str = r#"{"model":"advertised-model-fixture","messages":[{"role":"user","content":"request-body-canary-1a2b3c"}]}"#;
const CATALOG_BODY: &str =
    r#"{"object":"list","data":[{"id":"advertised-model-fixture","object":"model"}]}"#;
const COUNT_TOKENS_RESPONSE: &str = r#"{"input_tokens":17}"#;

struct ContractCase {
    method: Method,
    uri: &'static str,
    body: &'static str,
    headers: &'static [(&'static str, &'static str)],
    stream: bool,
    release: Option<Arc<Notify>>,
}

impl ContractCase {
    fn key(&self) -> String {
        format!("{} {}", self.method, self.uri)
    }
}

fn go_service_contract_cases() -> Vec<ContractCase> {
    vec![
        ContractCase {
            method: Method::GET,
            uri: "/v1/models",
            body: "",
            headers: &[("authorization", BEARER)],
            stream: false,
            release: None,
        },
        ContractCase {
            method: Method::POST,
            uri: "/v1/messages?beta=true",
            body: MESSAGES_BODY,
            headers: &[
                ("authorization", BEARER),
                ("content-type", "application/json"),
                ("anthropic-version", "2023-06-01"),
                (
                    "anthropic-beta",
                    "tools-2025-04-14,deferred-tools-2025-06-01",
                ),
                ("anthropic-open-list-2099", "open-list-header-canary"),
            ],
            stream: true,
            release: Some(Arc::new(Notify::new())),
        },
        ContractCase {
            method: Method::POST,
            uri: "/v1/messages/count_tokens",
            body: COUNT_TOKENS_BODY,
            headers: &[
                ("authorization", BEARER),
                ("content-type", "application/json"),
                ("anthropic-version", "2023-06-01"),
            ],
            stream: false,
            release: None,
        },
        ContractCase {
            method: Method::POST,
            uri: "/v1/chat/completions",
            body: r#"{"model":"advertised-model-fixture","stream":true,"messages":[{"role":"user","content":"request-body-canary-1a2b3c"}]}"#,
            headers: &[
                ("authorization", BEARER),
                ("content-type", "application/json"),
            ],
            stream: true,
            release: Some(Arc::new(Notify::new())),
        },
        ContractCase {
            method: Method::POST,
            uri: "/v1/completions",
            body: r#"{"model":"advertised-model-fixture","stream":true,"prompt":"request-body-canary-1a2b3c"}"#,
            headers: &[
                ("authorization", BEARER),
                ("content-type", "application/json"),
            ],
            stream: true,
            release: Some(Arc::new(Notify::new())),
        },
        ContractCase {
            method: Method::POST,
            uri: "/v1/responses",
            body: r#"{"model":"advertised-model-fixture","stream":true,"input":"request-body-canary-1a2b3c"}"#,
            headers: &[
                ("authorization", BEARER),
                ("content-type", "application/json"),
            ],
            stream: true,
            release: Some(Arc::new(Notify::new())),
        },
    ]
}

fn capture_logger() -> (ProxyLogger, Arc<Mutex<Vec<ProxyLog>>>) {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let captured = logs.clone();
    (
        Arc::new(move |log: ProxyLog| {
            captured.lock().expect("logs").push(log);
        }),
        logs,
    )
}

fn parity_config(
    upstream: &str,
    listen: &str,
    materials: super::TlsMaterials,
    identity: BuildInfo,
    logger: Option<ProxyLogger>,
) -> SupervisorConfig {
    let mut defaults = RouterDefaults::default();
    defaults.listen_addr = listen.to_owned();
    defaults.upstream_url = upstream.to_owned();
    defaults.timeout = Duration::from_secs(2);
    SupervisorConfig {
        defaults,
        env: RouterEnv::default(),
        flags: RouterFlags::default(),
        materials,
        identity,
        limits: SupervisorLimits::default(),
        logger,
    }
}

async fn http1_send<B>(base: &str, request: Request<B>) -> Response<Incoming>
where
    B: http_body::Body<Data = Bytes> + Send + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let uri: http::Uri = base.parse().expect("base");
    let stream = TcpStream::connect((uri.host().expect("host"), uri.port_u16().expect("port")))
        .await
        .expect("connect supervisor");
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .expect("handshake");
    tokio::spawn(connection);
    sender.send_request(request).await.expect("send")
}

async fn http1_empty(base: &str, method: Method, path: &str) -> Response<Incoming> {
    http1_send(
        base,
        Request::builder()
            .method(method)
            .uri(path)
            .body(Empty::<Bytes>::new())
            .expect("request"),
    )
    .await
}

async fn collect(response: Response<Incoming>) -> (StatusCode, HeaderMap, Bytes) {
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (status, headers, bytes)
}

async fn read_at_least(body: &mut Incoming, n: usize) -> Bytes {
    let mut buf = Vec::new();
    while buf.len() < n {
        let frame = body.frame().await.expect("frame").expect("ok");
        if let Ok(data) = frame.into_data() {
            buf.extend_from_slice(&data);
        }
    }
    Bytes::from(buf)
}

fn header_values(headers: &HeaderMap, name: &str) -> Vec<String> {
    headers
        .get_all(HeaderName::from_bytes(name.as_bytes()).expect("header name"))
        .iter()
        .map(|value| value.to_str().unwrap_or_default().to_owned())
        .collect()
}

fn assert_no_secret_leak(surface: &str) {
    for secret in [
        BEARER,
        BEARER.trim_start_matches("Bearer "),
        REQUEST_BODY,
        RESPONSE_BODY,
        QUERY,
        "api_key",
        "beta=true",
        "Anthropic-Version",
        "Anthropic-Beta",
        "open-list-header-canary",
        "deferred-field-canary",
        "BEGIN CERTIFICATE",
        "BEGIN RSA PRIVATE KEY",
        "BEGIN PRIVATE KEY",
    ] {
        assert!(
            !surface.contains(secret),
            "parity surface leaked {secret}: {surface}"
        );
    }
}

fn access_paths(logs: &[ProxyLog]) -> Vec<String> {
    logs.iter()
        .filter_map(|log| match log {
            ProxyLog::Access(record) => Some(record.path.clone()),
            ProxyLog::RequestFailed { path, .. } => Some(path.clone()),
            ProxyLog::StreamFailed => None,
        })
        .collect()
}

async fn start_contract_upstream(
    cases: Arc<Vec<ContractCase>>,
    probe_status: Arc<AtomicU16>,
) -> super::test_support::TestMtlsServer {
    let expected: HashMap<String, usize> = cases
        .iter()
        .enumerate()
        .map(|(index, case)| (case.key(), index))
        .collect();
    start_mtls_optional_handler(move |request| {
        let cases = cases.clone();
        let expected = expected.clone();
        let probe_status = probe_status.clone();
        async move {
            let method = request.method().clone();
            let target = request
                .uri()
                .path_and_query()
                .map(|path| path.as_str().to_owned())
                .unwrap_or_else(|| request.uri().path().to_owned());
            let path = request.uri().path().to_owned();
            let headers = request.headers().clone();
            if method == Method::GET && path == "/" {
                let status = probe_status.load(Ordering::SeqCst);
                return Some(
                    Response::builder()
                        .status(StatusCode::from_u16(status).expect("probe status"))
                        .body(boxed_bytes(Bytes::new()))
                        .expect("probe"),
                );
            }
            if path == "/v1/failure" {
                let body = request
                    .into_body()
                    .collect()
                    .await
                    .expect("failure body")
                    .to_bytes();
                assert_eq!(body, REQUEST_BODY);
                assert_eq!(header_values(&headers, "authorization"), vec![BEARER]);
                return None;
            }
            let key = format!("{method} {target}");
            let Some(&index) = expected.get(&key) else {
                return Some(
                    Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .body(boxed_bytes("unexpected"))
                        .expect("unexpected"),
                );
            };
            let case = &cases[index];
            let body = request
                .into_body()
                .collect()
                .await
                .expect("upstream body")
                .to_bytes();
            assert_eq!(body, case.body.as_bytes(), "{} body mismatch", case.uri);
            for (name, value) in case.headers {
                assert_eq!(
                    header_values(&headers, name),
                    vec![*value],
                    "{} header {name}",
                    case.uri
                );
            }
            if case.stream {
                let release = case.release.clone().expect("stream release");
                let (tx, rx) = mpsc::channel(2);
                tokio::spawn(async move {
                    let _ = tx
                        .send(Bytes::from(format!("data: {RESPONSE_BODY}-first\n\n")))
                        .await;
                    release.notified().await;
                    let _ = tx
                        .send(Bytes::from(format!("data: {RESPONSE_BODY}-second\n\n")))
                        .await;
                });
                return Some(
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
                        .body(boxed_channel(rx))
                        .expect("sse"),
                );
            }
            let payload = if case.uri == "/v1/models" {
                CATALOG_BODY
            } else {
                COUNT_TOKENS_RESPONSE
            };
            Some(
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(boxed_bytes(payload))
                    .expect("json"),
            )
        }
    })
    .await
}

#[tokio::test]
async fn go_service_contract_is_preserved_through_supervisor() {
    let cases = Arc::new(go_service_contract_cases());
    let probe_status = Arc::new(AtomicU16::new(204));
    let upstream = start_contract_upstream(cases.clone(), probe_status).await;
    let (logger, logs) = capture_logger();
    let supervisor = RouterSupervisor::spawn(parity_config(
        &upstream.url,
        "127.0.0.1:0",
        upstream.certs.materials(),
        BuildInfo::default(),
        Some(logger),
    ));
    let addr = supervisor.start().await.expect("start");
    let base = format!("http://{addr}");

    for case in cases.iter() {
        let mut builder = Request::builder().method(case.method.clone()).uri(case.uri);
        for (name, value) in case.headers {
            builder = builder.header(*name, *value);
        }
        let request = builder
            .body(Full::new(Bytes::from(case.body)))
            .expect("contract request");
        if case.stream {
            let release = case.release.clone().expect("release");
            let mut response = http1_send(&base, request).await;
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                "text/event-stream"
            );
            assert_eq!(
                response.headers().get(header::CACHE_CONTROL).unwrap(),
                "no-cache"
            );
            assert_eq!(
                response.headers().get("x-accel-buffering").unwrap(),
                HeaderValue::from_static("no")
            );
            let first = format!("data: {RESPONSE_BODY}-first\n\n");
            let chunk = timeout(
                Duration::from_secs(2),
                read_at_least(response.body_mut(), first.len()),
            )
            .await
            .expect("first SSE chunk was buffered");
            assert_eq!(&chunk[..first.len()], first.as_bytes());
            release.notify_one();
            let rest = response
                .into_body()
                .collect()
                .await
                .expect("sse rest")
                .to_bytes();
            let want = format!("data: {RESPONSE_BODY}-second\n\n");
            assert_eq!(rest, want);
            continue;
        }
        let (status, _, body) = collect(http1_send(&base, request).await).await;
        assert_eq!(status, StatusCode::OK, "{} status", case.uri);
        if case.uri == "/v1/models" {
            assert_eq!(body, CATALOG_BODY);
            assert!(body
                .windows(MODEL.len())
                .any(|window| window == MODEL.as_bytes()));
        }
        if case.uri == "/v1/messages/count_tokens" {
            assert_eq!(body, COUNT_TOKENS_RESPONSE);
        }
    }

    let failure = Request::builder()
        .method(Method::POST)
        .uri(format!("/v1/failure?api_key={QUERY}"))
        .header(header::AUTHORIZATION, BEARER)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(REQUEST_BODY)))
        .expect("failure");
    let (status, _, body) = collect(http1_send(&base, failure).await).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body, "{\"error\":\"Bad Gateway\"}\n");
    assert_no_secret_leak(&String::from_utf8_lossy(&body));

    tokio::time::sleep(Duration::from_millis(30)).await;
    let captured = logs.lock().expect("logs").clone();
    let encoded = format!("{captured:?}");
    for path in [
        "/v1/models",
        "/v1/messages",
        "/v1/messages/count_tokens",
        "/v1/chat/completions",
        "/v1/completions",
        "/v1/responses",
        "/v1/failure",
    ] {
        assert!(
            access_paths(&captured).iter().any(|got| got == path),
            "access logs missing {path}: {encoded}"
        );
    }
    assert!(captured.iter().any(|log| matches!(
        log,
        ProxyLog::RequestFailed {
            path,
            status: 502,
            reason: "upstream_failure",
            ..
        } if path == "/v1/failure"
    )));
    assert_no_secret_leak(&encoded);

    supervisor.shutdown().await.ok();
}

#[tokio::test]
async fn request_body_streams_through_supervisor_listen_port() {
    let first_seen = Arc::new(Notify::new());
    let received = Arc::new(Mutex::new(None::<Vec<u8>>));
    let upstream = start_mtls_optional_handler({
        let first_seen = first_seen.clone();
        let received = received.clone();
        move |request| {
            let first_seen = first_seen.clone();
            let received = received.clone();
            async move {
                if request.method() == Method::GET && request.uri().path() == "/" {
                    return Some(
                        Response::builder()
                            .status(StatusCode::NO_CONTENT)
                            .body(boxed_bytes(Bytes::new()))
                            .expect("probe"),
                    );
                }
                let mut body = request.into_body();
                let mut buf = Vec::new();
                while let Some(frame) = body.frame().await {
                    if let Ok(data) = frame.expect("frame").into_data() {
                        buf.extend_from_slice(&data);
                        if buf.len() >= 7 {
                            first_seen.notify_one();
                        }
                    }
                }
                *received.lock().expect("received") = Some(buf);
                Some(
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(boxed_bytes(COUNT_TOKENS_RESPONSE))
                        .expect("json"),
                )
            }
        }
    })
    .await;
    let supervisor = RouterSupervisor::spawn(parity_config(
        &upstream.url,
        "127.0.0.1:0",
        upstream.certs.materials(),
        BuildInfo::default(),
        None,
    ));
    let addr = supervisor.start().await.expect("start");
    let (tx, rx) = mpsc::channel::<Bytes>(2);
    let send = tokio::spawn({
        let base = format!("http://{addr}");
        async move {
            http1_send(
                &base,
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/messages/count_tokens")
                    .header(header::AUTHORIZATION, BEARER)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(boxed_channel(rx))
                    .expect("chunked"),
            )
            .await
        }
    });
    tx.send(Bytes::from(&b"{\"m\":1}"[..])).await.unwrap();
    timeout(Duration::from_secs(2), first_seen.notified())
        .await
        .expect("first request-body chunk was buffered");
    tx.send(Bytes::from(REQUEST_BODY)).await.unwrap();
    drop(tx);
    let (status, _, body) = collect(send.await.expect("join")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, COUNT_TOKENS_RESPONSE);
    assert_eq!(
        received.lock().expect("received").clone().unwrap(),
        format!("{{\"m\":1}}{REQUEST_BODY}").as_bytes()
    );
    supervisor.shutdown().await.ok();
}

#[tokio::test]
async fn go_management_and_probe_contract_is_preserved_through_supervisor() {
    let probe_status = Arc::new(AtomicU16::new(204));
    let hit_proxy = Arc::new(Mutex::new(Vec::<String>::new()));
    let upstream = start_mtls_optional_handler({
        let probe_status = probe_status.clone();
        let hit_proxy = hit_proxy.clone();
        move |request| {
            let probe_status = probe_status.clone();
            let hit_proxy = hit_proxy.clone();
            async move {
                let path = request.uri().path().to_owned();
                if request.method() == Method::GET && path == "/" {
                    let status = probe_status.load(Ordering::SeqCst);
                    return Some(
                        Response::builder()
                            .status(StatusCode::from_u16(status).expect("probe status"))
                            .body(boxed_bytes(Bytes::new()))
                            .expect("probe"),
                    );
                }
                hit_proxy.lock().expect("hits").push(path);
                Some(
                    Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .body(boxed_bytes("proxied"))
                        .expect("proxied"),
                )
            }
        }
    })
    .await;
    let (logger, logs) = capture_logger();
    let identity = BuildInfo::new(
        "parity-version",
        "parity-commit",
        "parity-date",
        "parity-deploy",
    );
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve");
    let failed_listen = reserved.local_addr().expect("addr").to_string();
    drop(reserved);

    let mut failed_upstream = start_mtls_server(MtlsServerOptions {
        status: 503,
        ..MtlsServerOptions::default()
    })
    .await;
    let failed = RouterSupervisor::spawn(parity_config(
        &format!(
            "{}/private?api_key=sk-parity-probe-canary",
            failed_upstream.url
        ),
        &failed_listen,
        failed_upstream.certs.materials(),
        identity.clone(),
        None,
    ));
    let error = failed.start().await.unwrap_err();
    failed_upstream.shutdown();
    assert_eq!(error.as_str(), "upstream_probe_failed");
    assert!(!error.to_string().contains("sk-parity-probe-canary"));
    assert!(!error.to_string().contains("private"));
    assert!(!failed.status().bound);
    assert_eq!(failed.status().phase, RuntimePhase::Failed);
    assert_eq!(
        failed.status().last_startup_error,
        Some(StartupReason::UpstreamProbeFailed)
    );
    assert!(TcpStream::connect(&failed_listen).await.is_err());
    failed.shutdown().await.ok();

    let supervisor = RouterSupervisor::spawn(parity_config(
        &upstream.url,
        "127.0.0.1:0",
        upstream.certs.materials(),
        identity,
        Some(logger),
    ));
    let addr = supervisor.start().await.expect("start");
    let base = format!("http://{addr}");

    let (status, headers, body) =
        collect(http1_empty(&base, Method::GET, VERSION_PATH).await).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("application/json"));
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
    let version: Value = serde_json::from_slice(&body).expect("version json");
    for key in [
        "version",
        "commit",
        "build_date",
        "deployment_id",
        "management_protocol_version",
        "pid",
        "started_at",
    ] {
        assert!(version.get(key).is_some(), "/version missing {key}");
    }
    assert_eq!(version["version"], "parity-version");
    assert_eq!(version["commit"], "parity-commit");
    assert_eq!(version["build_date"], "parity-date");
    assert_eq!(version["deployment_id"], "parity-deploy");
    assert_eq!(version["management_protocol_version"], "4");
    assert_eq!(version["pid"], std::process::id());

    let first_started = version["started_at"]
        .as_str()
        .expect("started_at")
        .to_owned();
    let again = collect(http1_empty(&base, Method::GET, VERSION_PATH).await)
        .await
        .2;
    let again: Value = serde_json::from_slice(&again).expect("version json");
    assert_eq!(again["started_at"], first_started);

    let (status, headers, body) = collect(http1_empty(&base, Method::GET, HEALTH_PATH).await).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
    let health: Value = serde_json::from_slice(&body).expect("health json");
    assert_eq!(health["status"], "ok");
    assert_eq!(health["upstream"], "reachable");
    assert!(health.get("error").is_none());

    for path in [VERSION_PATH, HEALTH_PATH] {
        let (status, headers, body) = collect(http1_empty(&base, Method::POST, path).await).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(headers.get(header::ALLOW).unwrap(), "GET");
        let value: Value = serde_json::from_slice(&body).expect("405 json");
        assert_eq!(value["error"], "method not allowed");
    }

    probe_status.store(503, Ordering::SeqCst);
    let (status, _, body) = collect(http1_empty(&base, Method::GET, HEALTH_PATH).await).await;
    assert_eq!(status, StatusCode::OK);
    let degraded: Value = serde_json::from_slice(&body).expect("degraded json");
    assert_eq!(degraded["status"], "degraded");
    assert_eq!(degraded["upstream"], "unreachable");
    assert_eq!(degraded["error"], "upstream probe failed");
    let encoded = degraded.to_string();
    assert!(!encoded.contains("sk-parity-probe-canary"));
    assert!(!encoded.contains(&upstream.url));

    for path in ["/version/", "/healthz", "/health/"] {
        let (status, _, body) = collect(http1_empty(&base, Method::GET, path).await).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{path} must not be management"
        );
        assert_eq!(body, "proxied");
    }

    tokio::time::sleep(Duration::from_millis(20)).await;
    let captured = logs.lock().expect("logs").clone();
    let logged = access_paths(&captured);
    assert!(
        !logged
            .iter()
            .any(|path| path == VERSION_PATH || path == HEALTH_PATH),
        "management routes must not be access-logged: {logged:?}"
    );
    for path in ["/version/", "/healthz", "/health/"] {
        assert!(
            logged.iter().any(|got| got == path),
            "proxied {path} should still be access-logged: {logged:?}"
        );
    }
    let hits = hit_proxy.lock().expect("hits").clone();
    assert!(hits.contains(&"/version/".to_owned()));
    assert!(hits.contains(&"/healthz".to_owned()));
    assert!(hits.contains(&"/health/".to_owned()));
    assert!(!hits
        .iter()
        .any(|path| path == VERSION_PATH || path == HEALTH_PATH));

    supervisor.shutdown().await.ok();
}
