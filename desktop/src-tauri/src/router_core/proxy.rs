use crate::redaction::{proxy_error_json, public_upstream_origin, request_path, AccessLogRecord};
use bytes::Bytes;
use http::header::{HeaderName, HeaderValue};
use http::{header, Method, Request, Response, StatusCode, Uri};
use http_body::{Body, Frame, SizeHint};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use super::connect::{open_mtls_http1, origin_form_path, upstream_host_header, ConnectError};
use super::{dispatch_management, BuildInfo, InfoProvider, MtlsTransport};

const HOP_BY_HOP: &[&str] = &[
    "connection",
    "proxy-connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProxyErrorKind {
    ClientBodyRead,
    Timeout,
    Upstream,
}

impl ProxyErrorKind {
    pub fn status(self) -> StatusCode {
        match self {
            Self::ClientBodyRead => StatusCode::BAD_REQUEST,
            Self::Timeout => StatusCode::GATEWAY_TIMEOUT,
            Self::Upstream => StatusCode::BAD_GATEWAY,
        }
    }

    pub fn reason(self) -> &'static str {
        match self {
            Self::ClientBodyRead => "client_body_read_failure",
            Self::Timeout => "upstream_timeout",
            Self::Upstream => "upstream_failure",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProxyLog {
    Access(AccessLogRecord),
    RequestFailed {
        method: String,
        path: String,
        status: u16,
        reason: &'static str,
    },
    StreamFailed,
}

pub type ProxyLogger = Arc<dyn Fn(ProxyLog) + Send + Sync>;

#[derive(Clone)]
pub struct ReverseProxy {
    upstream: Uri,
    transport: MtlsTransport,
    logger: Option<ProxyLogger>,
}

impl std::fmt::Debug for ReverseProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReverseProxy")
            .field(
                "upstream_origin",
                &public_upstream_origin(&self.upstream.to_string()),
            )
            .finish_non_exhaustive()
    }
}

impl ReverseProxy {
    pub fn new(upstream: Uri, transport: MtlsTransport) -> Self {
        Self {
            upstream,
            transport,
            logger: None,
        }
    }

    pub fn with_logger(mut self, logger: ProxyLogger) -> Self {
        self.logger = Some(logger);
        self
    }

    pub async fn handle<B>(&self, request: Request<B>) -> Response<BoxBody<Bytes, Infallible>>
    where
        B: Body<Data = Bytes> + Send + Unpin + 'static,
        B::Error: Send + 'static,
    {
        let start = Instant::now();
        let method = request.method().as_str().to_owned();
        let raw_target = request
            .uri()
            .path_and_query()
            .map(|path| path.as_str().to_owned())
            .unwrap_or_else(|| request.uri().path().to_owned());
        match self.proxy_once(request).await {
            Ok(response) => self.wrap_access_log(response, method, raw_target, start),
            Err(kind) => {
                self.emit(ProxyLog::RequestFailed {
                    method: method.clone(),
                    path: request_path(&raw_target),
                    status: kind.status().as_u16(),
                    reason: kind.reason(),
                });
                let response = proxy_error_response(kind);
                let bytes = error_body_len(kind);
                self.emit(ProxyLog::Access(AccessLogRecord::new(
                    method,
                    &raw_target,
                    kind.status().as_u16(),
                    bytes,
                    start.elapsed(),
                )));
                response
            }
        }
    }

    async fn proxy_once<B>(&self, request: Request<B>) -> Result<Response<Incoming>, ProxyErrorKind>
    where
        B: Body<Data = Bytes> + Send + Unpin + 'static,
        B::Error: Send + 'static,
    {
        let failed = Arc::new(AtomicBool::new(false));
        let (parts, body) = request.into_parts();
        let outgoing_uri = rewrite_upstream_uri(&parts.uri, &self.upstream)?;
        let host = upstream_host_header(&self.upstream).map_err(|_| ProxyErrorKind::Upstream)?;
        let mut headers = parts.headers;
        strip_hop_by_hop(&mut headers);
        headers.insert(
            header::HOST,
            HeaderValue::from_str(&host).map_err(|_| ProxyErrorKind::Upstream)?,
        );

        let mut outgoing = Request::new(ClientBody {
            inner: body,
            failed: failed.clone(),
        });
        *outgoing.method_mut() = parts.method;
        *outgoing.uri_mut() = origin_form_path(&outgoing_uri)
            .parse()
            .map_err(|_| ProxyErrorKind::Upstream)?;
        *outgoing.headers_mut() = headers;

        let (mut sender, _connection) = open_mtls_http1(&self.transport, &self.upstream)
            .await
            .map_err(|error| match error {
                ConnectError::Setup | ConnectError::Transport => ProxyErrorKind::Upstream,
            })?;
        let response = match sender.send_request(outgoing).await {
            Ok(response) => response,
            Err(error) => return Err(classify_send_error(&error, failed.load(Ordering::SeqCst))),
        };
        let (mut parts, body) = response.into_parts();
        strip_hop_by_hop(&mut parts.headers);
        normalize_sse_headers(&mut parts.headers);
        Ok(Response::from_parts(parts, body))
    }

    fn wrap_access_log(
        &self,
        response: Response<Incoming>,
        method: String,
        raw_target: String,
        start: Instant,
    ) -> Response<BoxBody<Bytes, Infallible>> {
        let status = response.status().as_u16();
        let (parts, body) = response.into_parts();
        let logged = AccessLogBody {
            inner: Box::pin(body),
            method,
            raw_target,
            status,
            bytes: 0,
            start,
            logger: self.logger.clone(),
            finished: false,
        };
        Response::from_parts(parts, logged.boxed())
    }

    fn emit(&self, log: ProxyLog) {
        if let Some(logger) = &self.logger {
            logger(log);
        }
    }
}

pub async fn serve_request<B, F, Fut, E>(
    request: Request<B>,
    identity: &BuildInfo,
    provider: Option<&dyn InfoProvider>,
    probe: F,
    proxy: &ReverseProxy,
) -> Response<BoxBody<Bytes, Infallible>>
where
    B: Body<Data = Bytes> + Send + Unpin + 'static,
    B::Error: Send + 'static,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    match dispatch_management(&request, identity, provider, probe).await {
        Some(response) => {
            let (parts, body) = response.into_parts();
            Response::from_parts(parts, body.map_err(|never| match never {}).boxed())
        }
        None => proxy.handle(request).await,
    }
}

pub fn rewrite_upstream_uri(incoming: &Uri, upstream: &Uri) -> Result<Uri, ProxyErrorKind> {
    let path = origin_form_path(incoming);
    let scheme = upstream.scheme_str().ok_or(ProxyErrorKind::Upstream)?;
    let authority = upstream_host_header(upstream).map_err(|_| ProxyErrorKind::Upstream)?;
    format!("{scheme}://{authority}{path}")
        .parse()
        .map_err(|_| ProxyErrorKind::Upstream)
}

pub fn normalize_sse_headers(headers: &mut http::HeaderMap) {
    let Some(content_type) = headers.get(header::CONTENT_TYPE) else {
        return;
    };
    let Ok(value) = content_type.to_str() else {
        return;
    };
    let media = value.split(';').next().unwrap_or("").trim();
    if !media.eq_ignore_ascii_case("text/event-stream") {
        return;
    }
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
}

fn strip_hop_by_hop(headers: &mut http::HeaderMap) {
    let extra: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| name.trim().parse().ok())
        .collect();
    for name in extra {
        headers.remove(name);
    }
    for name in HOP_BY_HOP {
        headers.remove(*name);
    }
}

fn proxy_error_response(kind: ProxyErrorKind) -> Response<BoxBody<Bytes, Infallible>> {
    let status = kind.status();
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(boxed_json(proxy_error_json(status.as_u16())))
        .expect("proxy error response")
}

fn boxed_json(value: serde_json::Value) -> BoxBody<Bytes, Infallible> {
    let mut body = serde_json::to_vec(&value).expect("closed proxy JSON");
    body.push(b'\n');
    Full::new(Bytes::from(body))
        .map_err(|never| match never {})
        .boxed()
}

fn error_body_len(kind: ProxyErrorKind) -> u64 {
    let mut body = serde_json::to_vec(&proxy_error_json(kind.status().as_u16())).expect("json");
    body.push(b'\n');
    body.len() as u64
}

fn classify_send_error(error: &hyper::Error, client_body_failed: bool) -> ProxyErrorKind {
    if client_body_failed {
        return ProxyErrorKind::ClientBodyRead;
    }
    if error.is_timeout() || is_timeout_error(error) {
        return ProxyErrorKind::Timeout;
    }
    ProxyErrorKind::Upstream
}

fn is_timeout_error(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(err) = current {
        if err.downcast_ref::<tokio::time::error::Elapsed>().is_some() {
            return true;
        }
        if let Some(io_error) = err.downcast_ref::<io::Error>() {
            if io_error.kind() == io::ErrorKind::TimedOut {
                return true;
            }
        }
        current = err.source();
    }
    false
}

struct ClientBody<B> {
    inner: B,
    failed: Arc<AtomicBool>,
}

impl<B> Body for ClientBody<B>
where
    B: Body<Data = Bytes> + Unpin,
{
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => Poll::Ready(Some(Ok(frame))),
            Poll::Ready(Some(Err(_))) => {
                this.failed.store(true, Ordering::SeqCst);
                Poll::Ready(Some(Err(io::Error::other("client body read error"))))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

struct AccessLogBody<B> {
    inner: Pin<Box<B>>,
    method: String,
    raw_target: String,
    status: u16,
    bytes: u64,
    start: Instant,
    logger: Option<ProxyLogger>,
    finished: bool,
}

impl<B> AccessLogBody<B> {
    fn emit(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some(logger) = &self.logger {
            logger(ProxyLog::Access(AccessLogRecord::new(
                self.method.clone(),
                &self.raw_target,
                self.status,
                self.bytes,
                self.start.elapsed(),
            )));
        }
    }
}

impl<B> Drop for AccessLogBody<B> {
    fn drop(&mut self) {
        self.emit();
    }
}

impl<B> Body for AccessLogBody<B>
where
    B: Body<Data = Bytes>,
{
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        let this = self.get_mut();
        match this.inner.as_mut().poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    this.bytes += data.len() as u64;
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(_))) => {
                if let Some(logger) = &this.logger {
                    logger(ProxyLog::StreamFailed);
                }
                this.emit();
                Poll::Ready(None)
            }
            Poll::Ready(None) => {
                this.emit();
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redaction::sanitize_text;
    use crate::router_core::test_support::{
        boxed_bytes, boxed_channel, start_mtls_handler, start_mtls_server, MtlsServerOptions,
    };
    use crate::router_core::{load_router_config, HEALTH_PATH, VERSION_PATH};
    use crate::router_core::{Prober, RouterDefaults, RouterEnv, RouterFlags, TlsMinVersion};
    use http_body_util::Empty;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio::net::TcpStream;
    use tokio::sync::{mpsc, oneshot, Notify};
    use tokio::time::timeout;

    const BODY_CANARY: &str = "request-body-canary-1a2b3c";
    const QUERY_CANARY: &str = "query-canary-3m4n5o";
    const BEARER: &str = "Bearer sk-fixture-auth-4d5e6f7g8h9i";
    const RESPONSE_CANARY: &str = "response-body-canary-0j1k2l";

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

    fn proxy_for(url: &str, transport: MtlsTransport, logger: Option<ProxyLogger>) -> ReverseProxy {
        let mut proxy = ReverseProxy::new(url.parse().expect("upstream"), transport);
        if let Some(logger) = logger {
            proxy = proxy.with_logger(logger);
        }
        proxy
    }

    async fn collect_json(response: Response<BoxBody<Bytes, Infallible>>) -> (StatusCode, Bytes) {
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        (status, bytes)
    }

    #[test]
    fn director_rewrites_scheme_and_host_only() {
        let incoming = "http://local.test/v1/chat?x=1"
            .parse::<Uri>()
            .expect("incoming");
        let upstream = "https://user:sk-director-canary@upstream.example:8443/private"
            .parse::<Uri>()
            .expect("upstream");
        let rewritten = rewrite_upstream_uri(&incoming, &upstream).expect("rewrite");
        assert_eq!(
            rewritten.to_string(),
            "https://upstream.example:8443/v1/chat?x=1"
        );
        assert_eq!(origin_form_path(&rewritten), "/v1/chat?x=1");
        assert!(!rewritten.to_string().contains("sk-director-canary"));
        assert!(!rewritten.to_string().contains("private"));
        assert!(!rewritten.to_string().contains("user:"));
    }

    #[test]
    fn sse_headers_are_forced_and_non_sse_headers_stay() {
        let mut sse = http::HeaderMap::new();
        sse.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        );
        normalize_sse_headers(&mut sse);
        assert_eq!(sse.get(header::CONTENT_TYPE).unwrap(), "text/event-stream");
        assert_eq!(sse.get(header::CACHE_CONTROL).unwrap(), "no-cache");
        assert_eq!(sse.get("x-accel-buffering").unwrap(), "no");

        let mut other = http::HeaderMap::new();
        other.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        other.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=60"),
        );
        normalize_sse_headers(&mut other);
        assert_eq!(other.get(header::CONTENT_TYPE).unwrap(), "application/json");
        assert_eq!(other.get(header::CACHE_CONTROL).unwrap(), "max-age=60");
        assert!(other.get("x-accel-buffering").is_none());
    }

    #[test]
    fn proxy_errors_use_closed_json_and_status_text() {
        for (kind, status, reason, _text) in [
            (
                ProxyErrorKind::ClientBodyRead,
                400,
                "client_body_read_failure",
                "Bad Request",
            ),
            (
                ProxyErrorKind::Timeout,
                504,
                "upstream_timeout",
                "Gateway Timeout",
            ),
            (
                ProxyErrorKind::Upstream,
                502,
                "upstream_failure",
                "Bad Gateway",
            ),
        ] {
            assert_eq!(kind.status().as_u16(), status);
            assert_eq!(kind.reason(), reason);
            let response = proxy_error_response(kind);
            assert_eq!(response.status().as_u16(), status);
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                "application/json"
            );
        }
        let leaked = "Get \"https://upstream.example/private?token=sk-upstream\": private key";
        let body = serde_json::to_string(&proxy_error_json(502)).unwrap();
        assert_eq!(body, r#"{"error":"Bad Gateway"}"#);
        assert!(!body.contains(leaked));
        assert!(!body.contains("private key"));
        assert_eq!(
            error_body_len(ProxyErrorKind::Upstream),
            (body.len() + 1) as u64
        );
    }

    #[tokio::test]
    async fn forwards_headers_query_and_body_without_buffering() {
        let first_seen = Arc::new(Notify::new());
        let received = Arc::new(Mutex::new(None::<(String, String, String, Vec<u8>)>));
        let upstream = start_mtls_handler({
            let first_seen = first_seen.clone();
            let received = received.clone();
            move |request| {
                let first_seen = first_seen.clone();
                let received = received.clone();
                async move {
                    let method = request.method().as_str().to_owned();
                    let target = request
                        .uri()
                        .path_and_query()
                        .map(|path| path.as_str().to_owned())
                        .unwrap_or_else(|| request.uri().path().to_owned());
                    let auth = request
                        .headers()
                        .get(header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned();
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
                    *received.lock().expect("received") = Some((method, target, auth, buf));
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(boxed_bytes(r#"{"input_tokens":17}"#))
                        .expect("response")
                }
            }
        })
        .await;
        let transport =
            MtlsTransport::new(&upstream.certs.materials(), TlsMinVersion::Tls12).unwrap();
        let (logger, logs) = capture_logger();
        let proxy = proxy_for(&upstream.url, transport, Some(logger));
        let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(2);
        let send = tokio::spawn({
            let proxy = proxy.clone();
            async move {
                let request = Request::builder()
                    .method(Method::POST)
                    .uri(format!(
                        "http://127.0.0.1:19099/v1/messages?beta=true&api_key={QUERY_CANARY}"
                    ))
                    .header(header::AUTHORIZATION, BEARER)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("anthropic-version", "2023-06-01")
                    .body(ResultChannelBody { rx })
                    .expect("request");
                proxy.handle(request).await
            }
        });
        tx.send(Ok(Bytes::from(&b"{\"m\":1}"[..]))).await.unwrap();
        timeout(Duration::from_secs(2), first_seen.notified())
            .await
            .expect("first request-body chunk was buffered");
        tx.send(Ok(Bytes::from(BODY_CANARY))).await.unwrap();
        drop(tx);
        let response = send.await.expect("join");
        let (status, body) = collect_json(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, r#"{"input_tokens":17}"#);
        let (method, target, auth, uploaded) = received.lock().expect("received").clone().unwrap();
        assert_eq!(method, "POST");
        assert_eq!(
            target,
            format!("/v1/messages?beta=true&api_key={QUERY_CANARY}")
        );
        assert_eq!(auth, BEARER);
        assert_eq!(uploaded, format!("{{\"m\":1}}{BODY_CANARY}").as_bytes());

        tokio::time::sleep(Duration::from_millis(20)).await;
        let encoded = format!("{:?}", logs.lock().expect("logs").clone());
        assert!(encoded.contains("path: \"/v1/messages\""));
        for secret in [BODY_CANARY, QUERY_CANARY, BEARER, "api_key", "beta=true"] {
            assert!(!encoded.contains(secret), "access log leaked {secret}");
        }
    }

    #[tokio::test]
    async fn sse_flushes_first_chunk_before_upstream_finishes() {
        let release = Arc::new(Notify::new());
        let upstream = start_mtls_handler({
            let release = release.clone();
            move |_request| {
                let release = release.clone();
                async move {
                    let (tx, rx) = mpsc::channel(2);
                    tokio::spawn(async move {
                        let _ = tx
                            .send(Bytes::from(format!("data: {RESPONSE_CANARY}-first\n\n")))
                            .await;
                        release.notified().await;
                        let _ = tx
                            .send(Bytes::from(format!("data: {RESPONSE_CANARY}-second\n\n")))
                            .await;
                    });
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
                        .body(boxed_channel(rx))
                        .expect("sse")
                }
            }
        })
        .await;
        let transport =
            MtlsTransport::new(&upstream.certs.materials(), TlsMinVersion::Tls12).unwrap();
        let proxy = proxy_for(&upstream.url, transport, None);
        let (base, _shutdown) = start_local_proxy(proxy).await;
        let mut response = http1_request(
            &base,
            Request::builder()
                .method(Method::POST)
                .uri("/v1/chat/completions")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Empty::new())
                .expect("request"),
        )
        .await;
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/event-stream"
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-cache"
        );
        assert_eq!(response.headers().get("x-accel-buffering").unwrap(), "no");
        let first = format!("data: {RESPONSE_CANARY}-first\n\n");
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
            .expect("rest")
            .to_bytes();
        let want = format!("data: {RESPONSE_CANARY}-second\n\n");
        assert!(
            rest.ends_with(want.as_bytes()),
            "remaining SSE = {:?}",
            rest
        );
    }

    #[tokio::test]
    async fn client_body_read_error_is_400_and_failures_stay_closed() {
        let echo = start_mtls_handler(|request| async move {
            let _ = request.into_body().collect().await;
            Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(boxed_bytes(Bytes::new()))
                .expect("response")
        })
        .await;
        let transport = MtlsTransport::new(&echo.certs.materials(), TlsMinVersion::Tls12).unwrap();
        let (logger, logs) = capture_logger();
        let proxy = proxy_for(&echo.url, transport, Some(logger));
        let request = Request::builder()
            .method(Method::POST)
            .uri(format!(
                "http://127.0.0.1:19099/v1/failure?api_key={QUERY_CANARY}"
            ))
            .header(header::AUTHORIZATION, BEARER)
            .body(FailAfterBody {
                first: Some(Bytes::from("partial body")),
            })
            .expect("request");
        let (status, body) = collect_json(proxy.handle(request).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, "{\"error\":\"Bad Request\"}\n");

        let mut failed = start_mtls_server(MtlsServerOptions::default()).await;
        let closed = format!("{}/private-path?api_key={QUERY_CANARY}", failed.url);
        let closed_transport =
            MtlsTransport::new(&failed.certs.materials(), TlsMinVersion::Tls12).unwrap();
        failed.shutdown();
        let failing = proxy_for(&closed, closed_transport, None);
        let (status, body) = collect_json(
            failing
                .handle(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/v1/failure")
                        .body(Empty::new())
                        .expect("request"),
                )
                .await,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body, "{\"error\":\"Bad Gateway\"}\n");
        let encoded = format!(
            "{}{:?}",
            String::from_utf8_lossy(&body),
            logs.lock().expect("logs").clone()
        );
        for secret in [
            QUERY_CANARY,
            BEARER,
            "sk-fixture",
            "private-path",
            "BEGIN CERTIFICATE",
        ] {
            assert!(
                !encoded.contains(secret),
                "proxy output leaked {secret}: {encoded}"
            );
        }
        assert!(!sanitize_text(&encoded).contains(QUERY_CANARY));
    }

    #[tokio::test]
    async fn management_routes_are_not_proxied_or_access_logged() {
        let hit = Arc::new(AtomicBool::new(false));
        let upstream = start_mtls_handler({
            let hit = hit.clone();
            move |_request| {
                hit.store(true, Ordering::SeqCst);
                async move {
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(boxed_bytes("proxied"))
                        .expect("response")
                }
            }
        })
        .await;
        let transport =
            MtlsTransport::new(&upstream.certs.materials(), TlsMinVersion::Tls12).unwrap();
        let (logger, logs) = capture_logger();
        let proxy = proxy_for(&upstream.url, transport.clone(), Some(logger));
        let identity = BuildInfo::default();
        let request = Request::builder()
            .method(Method::GET)
            .uri(format!("http://127.0.0.1:19099{VERSION_PATH}"))
            .body(Empty::new())
            .expect("request");
        let response = serve_request(
            request,
            &identity,
            None,
            || async { Ok::<(), Infallible>(()) },
            &proxy,
        )
        .await;
        let (status, body) = collect_json(response).await;
        assert_eq!(status, StatusCode::OK);
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["management_protocol_version"], "4");
        assert!(!hit.load(Ordering::SeqCst));
        assert!(logs.lock().expect("logs").is_empty());

        let mut defaults = RouterDefaults::default();
        defaults.upstream_url = upstream.url.clone();
        let config =
            load_router_config(defaults, &RouterEnv::default(), &RouterFlags::default()).unwrap();
        let prober = Prober::new(&config, &transport).unwrap();
        let health = serve_request(
            Request::builder()
                .method(Method::GET)
                .uri(HEALTH_PATH)
                .body(Empty::new())
                .expect("request"),
            &identity,
            None,
            || prober.probe(),
            &proxy,
        )
        .await;
        assert_eq!(health.status(), StatusCode::OK);
        assert!(logs.lock().expect("logs").is_empty());
    }

    #[test]
    fn reverse_proxy_debug_omits_upstream_secrets() {
        let server_certs = crate::router_core::test_support::TestCerts::generate();
        let transport =
            MtlsTransport::new(&server_certs.materials(), TlsMinVersion::Tls12).unwrap();
        let proxy = ReverseProxy::new(
            "https://user:sk-debug-canary@upstream.example/private?api_key=1"
                .parse()
                .unwrap(),
            transport,
        );
        let encoded = format!("{proxy:?}");
        assert!(!encoded.contains("sk-debug-canary"));
        assert!(!encoded.contains("private"));
        assert!(!encoded.contains("api_key"));
        assert!(encoded.contains("upstream.example"));
    }

    async fn start_local_proxy(proxy: ReverseProxy) -> (String, oneshot::Sender<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("proxy listen");
        let addr = listener.local_addr().expect("addr");
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let Ok((tcp, _)) = accepted else { break; };
                        let proxy = proxy.clone();
                        tokio::spawn(async move {
                            let service = service_fn(move |request| {
                                let proxy = proxy.clone();
                                async move { Ok::<_, Infallible>(proxy.handle(request).await) }
                            });
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(TokioIo::new(tcp), service)
                                .await;
                        });
                    }
                }
            }
        });
        (format!("http://127.0.0.1:{}", addr.port()), shutdown_tx)
    }

    async fn http1_request<B>(base: &str, request: Request<B>) -> Response<Incoming>
    where
        B: Body<Data = Bytes> + Send + 'static,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        let uri: Uri = base.parse().expect("base");
        let stream = TcpStream::connect((uri.host().expect("host"), uri.port_u16().expect("port")))
            .await
            .expect("connect proxy");
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .expect("handshake");
        tokio::spawn(connection);
        sender.send_request(request).await.expect("send")
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

    struct ResultChannelBody {
        rx: mpsc::Receiver<Result<Bytes, io::Error>>,
    }

    impl Body for ResultChannelBody {
        type Data = Bytes;
        type Error = io::Error;

        fn poll_frame(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
            match self.get_mut().rx.poll_recv(cx) {
                Poll::Ready(Some(Ok(bytes))) => Poll::Ready(Some(Ok(Frame::data(bytes)))),
                Poll::Ready(Some(Err(error))) => Poll::Ready(Some(Err(error))),
                Poll::Ready(None) => Poll::Ready(None),
                Poll::Pending => Poll::Pending,
            }
        }
    }

    struct FailAfterBody {
        first: Option<Bytes>,
    }

    impl Body for FailAfterBody {
        type Data = Bytes;
        type Error = io::Error;

        fn poll_frame(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
            let _ = cx;
            match self.get_mut().first.take() {
                Some(bytes) => Poll::Ready(Some(Ok(Frame::data(bytes)))),
                None => Poll::Ready(Some(Err(io::Error::other("client upload failed")))),
            }
        }
    }
}
