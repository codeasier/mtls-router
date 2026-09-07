use crate::protocol::MANAGEMENT_PROTOCOL_VERSION;
use bytes::Bytes;
use http::{header, Method, Request, Response, StatusCode};
use http_body_util::Full;
use serde_json::{json, Map, Value};
use std::future::Future;

use super::{HEALTH_PATH, VERSION_PATH};

const CONTENT_TYPE_JSON: &str = "application/json; charset=utf-8";
const HEALTH_PROBE_FAILED: &str = "upstream probe failed";

/// Build and process identity for GET `/version`.
///
/// `management_protocol_version` is code-owned (`"4"`) and cannot be supplied
/// by an [`InfoProvider`], matching Go `internal/version.ManagementProtocolVersion`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildInfo {
    version: String,
    commit: String,
    build_date: String,
    deployment_id: String,
}

impl Default for BuildInfo {
    fn default() -> Self {
        Self {
            version: "dev".to_owned(),
            commit: "unknown".to_owned(),
            build_date: "unknown".to_owned(),
            deployment_id: "dev".to_owned(),
        }
    }
}

impl BuildInfo {
    pub fn new(
        version: impl Into<String>,
        commit: impl Into<String>,
        build_date: impl Into<String>,
        deployment_id: impl Into<String>,
    ) -> Self {
        Self {
            version: version.into(),
            commit: commit.into(),
            build_date: build_date.into(),
            deployment_id: deployment_id.into(),
        }
    }

    /// Desktop compile-time identity. Not wired into the sidecar path.
    pub fn from_compile_env() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            commit: option_env!("MTLS_ROUTER_COMMIT")
                .unwrap_or("unknown")
                .to_owned(),
            build_date: option_env!("MTLS_ROUTER_BUILD_DATE")
                .unwrap_or("unknown")
                .to_owned(),
            deployment_id: env!("MTLS_DEPLOYMENT_ID").to_owned(),
        }
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn commit(&self) -> &str {
        &self.commit
    }

    pub fn build_date(&self) -> &str {
        &self.build_date
    }

    pub fn deployment_id(&self) -> &str {
        &self.deployment_id
    }

    pub fn management_protocol_version(&self) -> &'static str {
        MANAGEMENT_PROTOCOL_VERSION
    }
}

/// Extra `/version` JSON fields. Identity keys written after this merge cannot
/// be overridden, matching Go `routermeta.InfoProvider`.
pub trait InfoProvider: Send + Sync {
    fn info(&self) -> Map<String, Value>;
}

impl<F> InfoProvider for F
where
    F: Fn() -> Map<String, Value> + Send + Sync,
{
    fn info(&self) -> Map<String, Value> {
        self()
    }
}

pub fn handle_version<B>(
    request: &Request<B>,
    identity: &BuildInfo,
    provider: Option<&dyn InfoProvider>,
) -> Response<Full<Bytes>> {
    if request.method() != Method::GET {
        return method_not_allowed();
    }

    let mut payload = Map::new();
    payload.insert("started_at".to_owned(), Value::String(utc_now_rfc3339()));
    if let Some(provider) = provider {
        for (key, value) in provider.info() {
            payload.insert(key, value);
        }
    }
    payload.insert(
        "version".to_owned(),
        Value::String(identity.version.clone()),
    );
    payload.insert("commit".to_owned(), Value::String(identity.commit.clone()));
    payload.insert(
        "build_date".to_owned(),
        Value::String(identity.build_date.clone()),
    );
    payload.insert(
        "deployment_id".to_owned(),
        Value::String(identity.deployment_id.clone()),
    );
    payload.insert(
        "management_protocol_version".to_owned(),
        Value::String(identity.management_protocol_version().to_owned()),
    );
    payload.insert("pid".to_owned(), Value::from(std::process::id()));
    json_response(StatusCode::OK, Value::Object(payload), false)
}

pub async fn handle_health<B, F, Fut, E>(request: &Request<B>, probe: F) -> Response<Full<Bytes>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    if request.method() != Method::GET {
        return method_not_allowed();
    }

    let body = match probe().await {
        Ok(()) => json!({
            "status": "ok",
            "upstream": "reachable",
        }),
        Err(_) => json!({
            "status": "degraded",
            "upstream": "unreachable",
            "error": HEALTH_PROBE_FAILED,
        }),
    };
    json_response(StatusCode::OK, body, false)
}

/// Exact-path dispatcher for `/version` and `/health`. Other paths, including
/// `/version/` and `/healthz`, return `None` so a later mux can fall through.
pub async fn dispatch_management<B, F, Fut, E>(
    request: &Request<B>,
    identity: &BuildInfo,
    provider: Option<&dyn InfoProvider>,
    probe: F,
) -> Option<Response<Full<Bytes>>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    match request.uri().path() {
        VERSION_PATH => Some(handle_version(request, identity, provider)),
        HEALTH_PATH => Some(handle_health(request, probe).await),
        _ => None,
    }
}

fn method_not_allowed() -> Response<Full<Bytes>> {
    json_response(
        StatusCode::METHOD_NOT_ALLOWED,
        json!({ "error": "method not allowed" }),
        true,
    )
}

fn json_response(status: StatusCode, value: Value, set_allow: bool) -> Response<Full<Bytes>> {
    let mut body = serde_json::to_vec(&value).expect("management JSON is serializable");
    body.push(b'\n');
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, CONTENT_TYPE_JSON)
        .header(header::CACHE_CONTROL, "no-store");
    if set_allow {
        builder = builder.header(header::ALLOW, "GET");
    }
    builder
        .body(Full::new(Bytes::from(body)))
        .expect("management JSON response")
}

fn utc_now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router_core::{
        is_exact_management_route, load_router_config, local_http_request, test_support,
        MtlsTransport, Prober, RouterDefaults, RouterEnv, RouterFlags, StartupError, StartupReason,
        TlsMinVersion, HEALTH_PATH, VERSION_PATH,
    };
    use http_body_util::BodyExt;
    use std::convert::Infallible;
    use std::time::Duration;

    fn get(path: &str) -> Request<()> {
        Request::builder()
            .method(Method::GET)
            .uri(format!("http://127.0.0.1:19099{path}"))
            .body(())
            .expect("request")
    }

    fn method_request(method: Method, path: &str) -> Request<()> {
        Request::builder()
            .method(method)
            .uri(format!("http://127.0.0.1:19099{path}"))
            .body(())
            .expect("request")
    }

    async fn body_json(response: Response<Full<Bytes>>) -> (StatusCode, http::HeaderMap, Value) {
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let value = serde_json::from_slice::<Value>(&bytes)
            .unwrap_or_else(|error| panic!("body not JSON: {error} (body={:?})", bytes));
        (status, headers, value)
    }

    fn assert_method_not_allowed(status: StatusCode, headers: &http::HeaderMap, body: &Value) {
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(headers.get(header::ALLOW).unwrap(), "GET");
        assert!(headers
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("application/json"));
        assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
        assert_eq!(body["error"], "method not allowed");
    }

    #[test]
    fn build_info_defaults_match_go_local_build() {
        let info = BuildInfo::default();
        assert_eq!(info.version(), "dev");
        assert_eq!(info.commit(), "unknown");
        assert_eq!(info.build_date(), "unknown");
        assert_eq!(info.deployment_id(), "dev");
        assert_eq!(info.management_protocol_version(), "4");
        assert_eq!(
            info.management_protocol_version(),
            MANAGEMENT_PROTOCOL_VERSION
        );
        let compiled = BuildInfo::from_compile_env();
        assert_eq!(compiled.management_protocol_version(), "4");
        assert!(!compiled.deployment_id().is_empty());
    }

    #[tokio::test]
    async fn version_handler_returns_json() {
        let provider = || {
            let mut extra = Map::new();
            extra.insert("extra".to_owned(), json!("stub"));
            extra
        };
        let (status, headers, got) = body_json(handle_version(
            &get(VERSION_PATH),
            &BuildInfo::default(),
            Some(&provider),
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("application/json"));
        assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
        for key in [
            "version",
            "commit",
            "build_date",
            "deployment_id",
            "management_protocol_version",
            "pid",
            "started_at",
        ] {
            assert!(got.get(key).is_some(), "/version body missing {key}: {got}");
        }
        assert_eq!(got["extra"], "stub");
        assert_eq!(got["management_protocol_version"], "4");
        assert_ne!(got["management_protocol_version"], "");
        assert_eq!(got["pid"], json!(std::process::id()));
    }

    #[tokio::test]
    async fn version_handler_provider_can_keep_started_at_stable() {
        let provider = || {
            let mut extra = Map::new();
            extra.insert("started_at".to_owned(), json!("2026-06-21T00:00:00Z"));
            extra
        };
        let identity = BuildInfo::default();
        let first = body_json(handle_version(
            &get(VERSION_PATH),
            &identity,
            Some(&provider),
        ))
        .await
        .2;
        let second = body_json(handle_version(
            &get(VERSION_PATH),
            &identity,
            Some(&provider),
        ))
        .await
        .2;
        assert_eq!(first["started_at"], "2026-06-21T00:00:00Z");
        assert_eq!(second["started_at"], "2026-06-21T00:00:00Z");
    }

    #[tokio::test]
    async fn version_handler_provider_cannot_override_build_or_process_identity() {
        let provider = || {
            let mut extra = Map::new();
            extra.insert("pid".to_owned(), json!(-1));
            extra.insert("deployment_id".to_owned(), json!("forged"));
            extra.insert("management_protocol_version".to_owned(), json!("forged"));
            extra.insert("version".to_owned(), json!("forged"));
            extra.insert("commit".to_owned(), json!("forged"));
            extra.insert("build_date".to_owned(), json!("forged"));
            extra
        };
        let identity = BuildInfo::new("v-test", "commit-test", "date-test", "deploy-test");
        let got = body_json(handle_version(
            &get(VERSION_PATH),
            &identity,
            Some(&provider),
        ))
        .await
        .2;
        assert_eq!(got["pid"], json!(std::process::id()));
        assert_ne!(got["pid"], json!(-1));
        assert_eq!(got["deployment_id"], "deploy-test");
        assert_eq!(got["management_protocol_version"], "4");
        assert_eq!(got["version"], "v-test");
        assert_eq!(got["commit"], "commit-test");
        assert_eq!(got["build_date"], "date-test");
    }

    #[tokio::test]
    async fn version_and_health_reject_non_get() {
        for path in [VERSION_PATH, HEALTH_PATH] {
            for method in [Method::POST, Method::HEAD, Method::PUT] {
                let request = method_request(method, path);
                let response = if path == VERSION_PATH {
                    handle_version(&request, &BuildInfo::default(), None)
                } else {
                    handle_health(&request, || async { Ok::<(), Infallible>(()) }).await
                };
                let (status, headers, body) = body_json(response).await;
                assert_method_not_allowed(status, &headers, &body);
            }
        }
    }

    #[tokio::test]
    async fn health_handler_returns_ok_when_probe_succeeds() {
        let (status, headers, got) = body_json(
            handle_health(&get(HEALTH_PATH), || async { Ok::<(), Infallible>(()) }).await,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
        assert_eq!(got["status"], "ok");
        assert_eq!(got["upstream"], "reachable");
        assert!(got.get("error").is_none());
    }

    #[tokio::test]
    async fn health_handler_returns_degraded_but_still_200_when_probe_fails() {
        const UPSTREAM_CANARY: &str =
            "https://user:auth-canary@upstream.example/private?api_key=sk-health-canary";
        let (status, _, got) = body_json(
            handle_health(&get(HEALTH_PATH), || async {
                Err::<(), _>(format!("probe failed for {UPSTREAM_CANARY}"))
            })
            .await,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "health must never fail the HTTP layer"
        );
        assert_eq!(got["status"], "degraded");
        assert_eq!(got["upstream"], "unreachable");
        assert_eq!(got["error"], HEALTH_PROBE_FAILED);
        let encoded = got.to_string();
        assert!(!encoded.contains(UPSTREAM_CANARY));
        assert!(!encoded.contains("sk-health-canary"));
        assert!(!encoded.contains("auth-canary"));
        assert!(!encoded.contains("upstream.example"));
    }

    #[tokio::test]
    async fn dispatch_uses_exact_paths_and_does_not_bind() {
        let identity = BuildInfo::default();
        let version = dispatch_management(&get(VERSION_PATH), &identity, None, || async {
            Ok::<(), Infallible>(())
        })
        .await
        .expect("exact /version");
        assert_eq!(version.status(), StatusCode::OK);

        let health = dispatch_management(&get(HEALTH_PATH), &identity, None, || async {
            Ok::<(), Infallible>(())
        })
        .await
        .expect("exact /health");
        assert_eq!(health.status(), StatusCode::OK);

        let queried = local_http_request("/version?cache=1");
        assert_eq!(queried.uri().path(), VERSION_PATH);
        assert!(
            dispatch_management(&get("/version?cache=1"), &identity, None, || async {
                Ok::<(), Infallible>(())
            },)
            .await
            .is_some()
        );

        for path in ["/version/", "/version/extra", "/healthz", "/", "/health/"] {
            assert!(!is_exact_management_route(path));
            assert!(
                dispatch_management(&get(path), &identity, None, || async {
                    Ok::<(), Infallible>(())
                },)
                .await
                .is_none(),
                "path {path} must not be a management route"
            );
        }
    }

    #[tokio::test]
    async fn health_live_probe_stays_http_200_when_upstream_degrades() {
        let ok = test_support::start_mtls_server(test_support::MtlsServerOptions {
            status: 204,
            ..test_support::MtlsServerOptions::default()
        })
        .await;
        let mut defaults = RouterDefaults::default();
        defaults.upstream_url = ok.url.clone();
        let config = load_router_config(
            defaults.clone(),
            &RouterEnv::default(),
            &RouterFlags::default(),
        )
        .unwrap();
        let transport = MtlsTransport::new(&ok.certs.materials(), config.tls_min()).unwrap();
        let prober = Prober::new(&config, &transport).unwrap();
        let (status, _, got) =
            body_json(handle_health(&get(HEALTH_PATH), || prober.probe()).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(got["status"], "ok");

        let mut failed = test_support::start_mtls_server(test_support::MtlsServerOptions {
            status: 503,
            ..test_support::MtlsServerOptions::default()
        })
        .await;
        defaults.upstream_url = format!(
            "{}/private-path?api_key=sk-health-handler-canary",
            failed.url
        );
        defaults.timeout = Duration::from_secs(1);
        let degraded =
            load_router_config(defaults, &RouterEnv::default(), &RouterFlags::default()).unwrap();
        let degraded_transport =
            MtlsTransport::new(&failed.certs.materials(), TlsMinVersion::Tls12).unwrap();
        let degraded_prober = Prober::new(&degraded, &degraded_transport).unwrap();
        let (status, _, got) =
            body_json(handle_health(&get(HEALTH_PATH), || degraded_prober.probe()).await).await;
        failed.shutdown();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(got["status"], "degraded");
        assert_eq!(got["error"], HEALTH_PROBE_FAILED);
        let encoded = got.to_string();
        assert!(!encoded.contains("sk-health-handler-canary"));
        assert!(!encoded.contains("private-path"));
        assert!(!encoded.contains(&failed.url));
        assert_eq!(
            StartupError::new(StartupReason::UpstreamProbeFailed).to_string(),
            "upstream_probe_failed"
        );
    }
}
