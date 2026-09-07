use bytes::Bytes;
use http::{header::HOST, Method, Request, Uri};
use http_body_util::{BodyExt, Empty};
use hyper::body::Incoming;
use std::future::Future;
use tokio::time::timeout;

use super::config::validate_upstream_url;
use super::connect::{open_mtls_http1, origin_form_path, upstream_host_header, ConnectError};
use super::{MtlsTransport, RouterConfig, StartupError, StartupReason};

#[derive(Clone)]
pub struct Prober {
    url: Uri,
    timeout: std::time::Duration,
    transport: MtlsTransport,
}

impl Prober {
    pub fn new(config: &RouterConfig, transport: &MtlsTransport) -> Result<Self, StartupError> {
        let url = validate_upstream_url(config.upstream_url())
            .map_err(|_| StartupError::new(StartupReason::ProbeSetupFailed))?;
        Ok(Self {
            url,
            timeout: config.probe_timeout(),
            transport: transport.clone(),
        })
    }

    pub fn probe(&self) -> impl Future<Output = Result<(), StartupError>> + Send {
        let url = self.url.clone();
        let timeout_budget = self.timeout;
        let transport = self.transport.clone();
        async move {
            timeout(timeout_budget, probe_once(&transport, &url))
                .await
                .unwrap_or(Err(StartupError::new(StartupReason::UpstreamProbeFailed)))
        }
    }
}

async fn probe_once(transport: &MtlsTransport, url: &Uri) -> Result<(), StartupError> {
    let (mut sender, connection_task) = open_mtls_http1(transport, url)
        .await
        .map_err(ConnectError::into_startup)?;
    let request = probe_request(url)?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| StartupError::new(StartupReason::UpstreamProbeFailed))?;
    let status = response.status();
    drain_body(response).await?;
    drop(sender);
    let _ = connection_task.await;
    if status.as_u16() >= 500 {
        return Err(StartupError::new(StartupReason::UpstreamProbeFailed));
    }
    Ok(())
}

fn probe_request(url: &Uri) -> Result<Request<Empty<Bytes>>, StartupError> {
    Request::builder()
        .method(Method::GET)
        .uri(origin_form_path(url))
        .header(
            HOST,
            upstream_host_header(url).map_err(ConnectError::into_startup)?,
        )
        .body(Empty::new())
        .map_err(|_| StartupError::new(StartupReason::ProbeSetupFailed))
}

async fn drain_body(response: http::Response<Incoming>) -> Result<(), StartupError> {
    let mut body = response.into_body();
    while let Some(frame) = body.frame().await {
        frame.map_err(|_| StartupError::new(StartupReason::UpstreamProbeFailed))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router_core::test_support::{start_mtls_server, MtlsServerOptions};
    use crate::router_core::{
        load_router_config, RouterDefaults, RouterEnv, RouterFlags, TlsMaterials, TlsMinVersion,
    };
    use std::time::Duration;

    fn config_for(upstream: &str, tls_min: TlsMinVersion, timeout: Duration) -> RouterConfig {
        let mut defaults = RouterDefaults::default();
        defaults.upstream_url = upstream.to_owned();
        defaults.tls_min = tls_min.as_str().to_owned();
        defaults.timeout = timeout;
        load_router_config(defaults, &RouterEnv::default(), &RouterFlags::default()).unwrap()
    }

    #[tokio::test]
    async fn probe_succeeds_against_mtls_server() {
        let server = start_mtls_server(MtlsServerOptions {
            status: 204,
            ..MtlsServerOptions::default()
        })
        .await;
        let config = config_for(&server.url, TlsMinVersion::Tls12, Duration::from_secs(1));
        let transport = MtlsTransport::new(&server.certs.materials(), config.tls_min()).unwrap();
        let prober = Prober::new(&config, &transport).unwrap();
        prober.probe().await.expect("probe");
    }

    #[tokio::test]
    async fn probe_tls_minimum_handshake_matches_go() {
        let tls12 = start_mtls_server(MtlsServerOptions {
            status: 204,
            server_versions: vec![&rustls::version::TLS12],
            ..MtlsServerOptions::default()
        })
        .await;
        let tls13 = start_mtls_server(MtlsServerOptions {
            status: 204,
            server_versions: vec![&rustls::version::TLS13],
            ..MtlsServerOptions::default()
        })
        .await;

        let reject = config_for(&tls12.url, TlsMinVersion::Tls13, Duration::from_secs(1));
        let reject_transport =
            MtlsTransport::new(&tls12.certs.materials(), reject.tls_min()).unwrap();
        let error = Prober::new(&reject, &reject_transport)
            .unwrap()
            .probe()
            .await
            .unwrap_err();
        assert_eq!(error.reason(), StartupReason::UpstreamProbeFailed);

        let accept = config_for(&tls13.url, TlsMinVersion::Tls13, Duration::from_secs(1));
        let accept_transport =
            MtlsTransport::new(&tls13.certs.materials(), accept.tls_min()).unwrap();
        Prober::new(&accept, &accept_transport)
            .unwrap()
            .probe()
            .await
            .expect("tls1.3 probe");
    }

    #[tokio::test]
    async fn probe_failures_use_closed_reason_without_upstream_detail() {
        let mut server = start_mtls_server(MtlsServerOptions {
            status: 204,
            ..MtlsServerOptions::default()
        })
        .await;
        let canary = "sk-health-probe-query-canary";
        let private_path = "/private-path";
        let closed = format!("{}{private_path}?api_key={canary}", server.url);
        server.shutdown();

        let config = config_for(&closed, TlsMinVersion::Tls12, Duration::from_secs(1));
        let transport = MtlsTransport::new(&server.certs.materials(), config.tls_min()).unwrap();
        let error = Prober::new(&config, &transport)
            .unwrap()
            .probe()
            .await
            .unwrap_err();
        let encoded = error.to_string();
        assert_eq!(error.reason(), StartupReason::UpstreamProbeFailed);
        assert_eq!(encoded, "upstream_probe_failed");
        assert!(!encoded.contains(canary));
        assert!(!encoded.contains(&server.url));
        assert!(!encoded.contains(private_path));
        assert!(!encoded.contains("api_key"));
    }

    #[tokio::test]
    async fn probe_rejects_5xx_timeout_and_wrong_ca() {
        let unavailable = start_mtls_server(MtlsServerOptions {
            status: 503,
            ..MtlsServerOptions::default()
        })
        .await;
        let config = config_for(
            &unavailable.url,
            TlsMinVersion::Tls12,
            Duration::from_secs(1),
        );
        let transport =
            MtlsTransport::new(&unavailable.certs.materials(), config.tls_min()).unwrap();
        let error = Prober::new(&config, &transport)
            .unwrap()
            .probe()
            .await
            .unwrap_err();
        assert_eq!(error.reason(), StartupReason::UpstreamProbeFailed);

        let slow = start_mtls_server(MtlsServerOptions {
            status: 204,
            delay: Duration::from_millis(80),
            ..MtlsServerOptions::default()
        })
        .await;
        let timed = config_for(&slow.url, TlsMinVersion::Tls12, Duration::from_millis(1));
        let timed_transport = MtlsTransport::new(&slow.certs.materials(), timed.tls_min()).unwrap();
        let error = Prober::new(&timed, &timed_transport)
            .unwrap()
            .probe()
            .await
            .unwrap_err();
        assert_eq!(error.reason(), StartupReason::UpstreamProbeFailed);

        let other = crate::router_core::test_support::TestCerts::generate();
        let mismatch = TlsMaterials::new(
            unavailable.certs.client_cert.clone(),
            unavailable.certs.client_key.clone(),
            other.ca,
        );
        let mismatch_transport = MtlsTransport::new(&mismatch, TlsMinVersion::Tls12).unwrap();
        let error = Prober::new(&config, &mismatch_transport)
            .unwrap()
            .probe()
            .await
            .unwrap_err();
        assert_eq!(error.reason(), StartupReason::UpstreamProbeFailed);
        assert_eq!(error.to_string(), "upstream_probe_failed");
    }
}
