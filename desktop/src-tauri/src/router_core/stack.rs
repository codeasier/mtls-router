use bytes::Bytes;
use http::{Method, Request};
use http_body_util::Empty;
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tokio_rustls::TlsConnector;

pub const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:19099";
pub const VERSION_PATH: &str = "/version";
pub const HEALTH_PATH: &str = "/health";
pub const DEFAULT_TLS_MIN: &str = "tls1.2";
pub const CRYPTO_BACKEND: &str = "ring";

const CARGO_TOML: &str = include_str!("../../Cargo.toml");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LockedHttpTlsVersions {
    pub bytes: &'static str,
    pub http: &'static str,
    pub http_body: &'static str,
    pub http_body_util: &'static str,
    pub hyper: &'static str,
    pub hyper_util: &'static str,
    pub rustls: &'static str,
    pub rustls_pki_types: &'static str,
    pub tokio_rustls: &'static str,
}

pub const LOCKED_HTTP_TLS_VERSIONS: LockedHttpTlsVersions = LockedHttpTlsVersions {
    bytes: "1.12.1",
    http: "1.4.2",
    http_body: "1.0.1",
    http_body_util: "0.1.3",
    hyper: "1.10.1",
    hyper_util: "0.1.20",
    rustls: "0.23.43",
    rustls_pki_types: "1.15.1",
    tokio_rustls: "0.26.4",
};

#[derive(Debug)]
pub struct PemParseError;

impl std::fmt::Display for PemParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid PEM material")
    }
}

impl std::error::Error for PemParseError {}

pub struct LockedHttpTlsStack {
    listen_addr: SocketAddr,
    client_config: Arc<ClientConfig>,
    tls_connector: TlsConnector,
}

impl LockedHttpTlsStack {
    pub fn new() -> Self {
        let listen_addr = SocketAddr::from_str(DEFAULT_LISTEN_ADDR)
            .expect("DEFAULT_LISTEN_ADDR is a valid socket address");
        let client_config = Arc::new(locked_rustls_client_config());
        let tls_connector = TlsConnector::from(client_config.clone());
        Self {
            listen_addr,
            client_config,
            tls_connector,
        }
    }

    pub fn listen_addr(&self) -> SocketAddr {
        self.listen_addr
    }

    pub fn client_config(&self) -> &ClientConfig {
        self.client_config.as_ref()
    }

    pub fn tls_connector(&self) -> &TlsConnector {
        &self.tls_connector
    }
}

impl Default for LockedHttpTlsStack {
    fn default() -> Self {
        Self::new()
    }
}

fn locked_rustls_client_config() -> ClientConfig {
    ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the locked TLS 1.2/1.3 versions")
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth()
}

pub fn is_exact_management_route(path: &str) -> bool {
    path == VERSION_PATH || path == HEALTH_PATH
}

pub fn local_http_request(path: &str) -> Request<Empty<Bytes>> {
    Request::builder()
        .method(Method::GET)
        .uri(format!("http://{DEFAULT_LISTEN_ADDR}{path}"))
        .body(Empty::new())
        .expect("local HTTP request")
}

pub fn parse_pem_certificates(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, PemParseError> {
    let certificates = CertificateDer::pem_slice_iter(pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| PemParseError)?;
    if certificates.is_empty() {
        return Err(PemParseError);
    }
    Ok(certificates)
}

pub fn parse_pem_private_key(pem: &[u8]) -> Result<PrivateKeyDer<'static>, PemParseError> {
    PrivateKeyDer::from_pem_slice(pem).map_err(|_| PemParseError)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redaction::{sanitize_text, REDACTED_PEM};
    use hyper_util::client::legacy::connect::HttpConnector;
    use hyper_util::rt::TokioExecutor;

    #[test]
    fn default_listen_and_management_routes_match_go_router() {
        let stack = LockedHttpTlsStack::new();
        assert_eq!(stack.listen_addr().to_string(), DEFAULT_LISTEN_ADDR);
        assert_eq!(DEFAULT_LISTEN_ADDR, "127.0.0.1:19099");
        assert_eq!(VERSION_PATH, "/version");
        assert_eq!(HEALTH_PATH, "/health");
        assert_eq!(DEFAULT_TLS_MIN, "tls1.2");
        assert!(is_exact_management_route(VERSION_PATH));
        assert!(is_exact_management_route(HEALTH_PATH));
        assert!(!is_exact_management_route("/version/"));
        assert!(!is_exact_management_route("/version/extra"));
        assert!(!is_exact_management_route("/healthz"));
        assert!(!is_exact_management_route("/"));
    }

    #[test]
    fn cargo_toml_pins_the_locked_http_tls_stack() {
        let versions = LOCKED_HTTP_TLS_VERSIONS;
        assert!(CARGO_TOML.contains(&format!(r#"bytes = "={}"#, versions.bytes)));
        assert!(CARGO_TOML.contains(&format!(r#"http = "={}"#, versions.http)));
        assert!(CARGO_TOML.contains(&format!(r#"http-body = "={}"#, versions.http_body)));
        assert!(CARGO_TOML.contains(&format!(
            r#"http-body-util = "={}""#,
            versions.http_body_util
        )));
        assert!(CARGO_TOML.contains(&format!(r#"hyper = {{ version = "={}""#, versions.hyper)));
        assert!(CARGO_TOML.contains(&format!(
            r#"hyper-util = {{ version = "={}""#,
            versions.hyper_util
        )));
        assert!(CARGO_TOML.contains(&format!(r#"rustls = {{ version = "={}""#, versions.rustls)));
        assert!(CARGO_TOML.contains(&format!(
            r#"rustls-pki-types = "={}""#,
            versions.rustls_pki_types
        )));
        assert!(CARGO_TOML.contains(&format!(
            r#"tokio-rustls = {{ version = "={}""#,
            versions.tokio_rustls
        )));
        assert!(CARGO_TOML.contains(r#"features = ["logging", "ring", "std", "tls12"]"#));
        assert!(CARGO_TOML.contains(r#"features = ["logging", "ring", "tls12"]"#));
        assert!(!CARGO_TOML.contains("aws-lc"));
        assert!(!CARGO_TOML.contains("aws_lc"));
        assert!(!CARGO_TOML.contains("native-tls"));
        assert!(!CARGO_TOML.contains("rustls-pemfile"));
        assert_eq!(CRYPTO_BACKEND, "ring");
    }

    #[test]
    fn rustls_stack_uses_ring_and_does_not_bind() {
        let stack = LockedHttpTlsStack::new();
        let _ = rustls::crypto::ring::default_provider();
        let _ = stack.client_config();
        let _ = stack.tls_connector();
        let _ = TokioExecutor::new();
        let _ = HttpConnector::new();
        assert_eq!(stack.listen_addr().ip().to_string(), "127.0.0.1");
        assert_eq!(stack.listen_addr().port(), 19099);
    }

    #[test]
    fn local_http_request_uses_plain_http_and_exact_paths() {
        let request = local_http_request(VERSION_PATH);
        assert_eq!(request.method(), Method::GET);
        assert_eq!(request.uri().to_string(), "http://127.0.0.1:19099/version");
        assert_eq!(request.uri().scheme_str(), Some("http"));
        assert_eq!(request.uri().path(), VERSION_PATH);

        let health = local_http_request(HEALTH_PATH);
        assert_eq!(health.uri().path(), HEALTH_PATH);
        assert_eq!(health.uri().scheme_str(), Some("http"));
    }

    #[test]
    fn pem_parser_rejects_invalid_material_without_echoing_it() {
        let pem = b"-----BEGIN CERTIFICATE-----\nsk-canary-secret-key\n-----END CERTIFICATE-----\n";
        let cert_error = parse_pem_certificates(pem).expect_err("invalid cert PEM");
        let key_error = parse_pem_private_key(pem).expect_err("invalid key PEM");
        let cert_message = cert_error.to_string();
        let key_message = key_error.to_string();
        assert_eq!(cert_message, "invalid PEM material");
        assert_eq!(key_message, "invalid PEM material");
        assert!(!cert_message.contains("BEGIN"));
        assert!(!key_message.contains("BEGIN"));
        assert!(!cert_message.contains("sk-canary-secret-key"));
        assert!(!key_message.contains("sk-canary-secret-key"));
        let sanitized = sanitize_text(&String::from_utf8_lossy(pem));
        assert!(sanitized.contains(REDACTED_PEM));
        assert!(!sanitized.contains("BEGIN CERTIFICATE"));
        assert!(!sanitized.contains("sk-canary-secret-key"));
    }
}
