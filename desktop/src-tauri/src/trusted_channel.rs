//! Trusted single-connection HTTP/1.1 client for the loopback router.
//!
//! Mirrors Go `internal/manager/trustedrouter.Channel`: opens one TCP
//! connection, verifies `/version`, process identity, and `/health` on that
//! same connection, and only then sends authenticated requests. No redial,
//! no redirect, no system proxy.

use crate::image_limits::{CONNECT_TIMEOUT_SECS, GENERATION_TIMEOUT_SECS, VERSION_BODY_LIMIT};
use crate::router_process::{self, RouterIdentity, RouterProcessStatus};
use std::collections::HashSet;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufStream};
use tokio::net::TcpStream;
use zeroize::Zeroizing;

const MAX_HTTP_HEAD_BYTES: usize = 64 * 1024;

/// Configuration for establishing a trusted channel.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedChannelConfig {
    #[serde(rename = "listen_addr")]
    pub listen_address: String,
    #[serde(rename = "pid")]
    pub expected_pid: u32,
    #[serde(rename = "started_at")]
    pub expected_started_at: String,
    #[serde(rename = "process_started_at")]
    pub expected_process_started_at: String,
    #[serde(rename = "process_executable")]
    pub expected_process_executable: String,
    #[serde(rename = "deployment_id")]
    pub expected_deployment_id: String,
    #[serde(rename = "management_protocol_version")]
    pub expected_protocol_version: String,
    #[serde(rename = "binary_path")]
    pub expected_binary_path: String,
}

/// Version response from `/version`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct VersionInfo {
    pub pid: u32,
    pub started_at: String,
    #[serde(default)]
    pub deployment_id: String,
    #[serde(default)]
    pub management_protocol_version: String,
}

/// Health response from `/health`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct HealthInfo {
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrustedChannelError {
    ConnectFailed(String),
    NonLoopbackAddress,
    Redirect,
    ProtocolUpgrade,
    ConnectionClose,
    BadStatus(u16),
    BodyTooLarge,
    BodyReadFailed,
    VersionMismatch,
    ProcessMismatch,
    NotVerified,
    HealthNotOk,
    HealthParseFailed,
    VersionParseFailed,
    IoError(String),
}

impl std::fmt::Display for TrustedChannelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectFailed(msg) => write!(f, "connect failed: {msg}"),
            Self::NonLoopbackAddress => write!(f, "address is not loopback"),
            Self::Redirect => write!(f, "server attempted redirect"),
            Self::ProtocolUpgrade => write!(f, "server attempted protocol upgrade"),
            Self::ConnectionClose => write!(f, "server closed connection"),
            Self::BadStatus(code) => write!(f, "unexpected status {code}"),
            Self::BodyTooLarge => write!(f, "response body exceeds limit"),
            Self::BodyReadFailed => write!(f, "failed to read response body"),
            Self::VersionMismatch => write!(f, "router version identity mismatch"),
            Self::ProcessMismatch => write!(f, "router process identity mismatch"),
            Self::NotVerified => write!(f, "router connection is not verified"),
            Self::HealthNotOk => write!(f, "router health is not ok"),
            Self::HealthParseFailed => write!(f, "failed to parse health response"),
            Self::VersionParseFailed => write!(f, "failed to parse version response"),
            Self::IoError(msg) => write!(f, "io error: {msg}"),
        }
    }
}

impl std::error::Error for TrustedChannelError {}

pub type TrustedResult<T> = Result<T, TrustedChannelError>;

/// A single-connection trusted HTTP/1.1 client.
pub struct TrustedChannel {
    stream: BufStream<TcpStream>,
    host: String,
    connection_closed: bool,
    trusted: bool,
}

impl TrustedChannel {
    /// Opens a single TCP connection to the loopback router.
    /// Fails if the address is not loopback.
    pub async fn connect(config: &TrustedChannelConfig) -> TrustedResult<Self> {
        let authority = loopback_authority(&config.listen_address)?;
        let stream = tokio::time::timeout(
            Duration::from_secs(CONNECT_TIMEOUT_SECS),
            TcpStream::connect(&authority),
        )
        .await
        .map_err(|_| TrustedChannelError::ConnectFailed("timeout".into()))?
        .map_err(|e| TrustedChannelError::ConnectFailed(e.to_string()))?;
        stream
            .set_nodelay(true)
            .map_err(|e| TrustedChannelError::ConnectFailed(e.to_string()))?;
        let host = authority;
        Ok(Self {
            stream: BufStream::new(stream),
            host,
            connection_closed: false,
            trusted: false,
        })
    }

    /// Performs the full trust chain: /version, process identity, /health.
    /// Must be called before any authenticated request.
    pub async fn verify_trust(&mut self, config: &TrustedChannelConfig) -> TrustedResult<()> {
        self.verify_trust_with_timeout(config, Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .await
    }

    async fn verify_trust_with_timeout(
        &mut self,
        config: &TrustedChannelConfig,
        timeout: Duration,
    ) -> TrustedResult<()> {
        tokio::time::timeout(timeout, self.verify_trust_inner(config))
            .await
            .map_err(|_| TrustedChannelError::IoError("trust verification timeout".into()))?
    }

    async fn verify_trust_inner(&mut self, config: &TrustedChannelConfig) -> TrustedResult<()> {
        self.verify_version(config).await?;
        self.verify_process(config).await?;
        self.verify_health().await?;
        if self.connection_closed {
            return Err(TrustedChannelError::ConnectionClose);
        }
        self.trusted = true;
        Ok(())
    }

    async fn verify_version(&mut self, config: &TrustedChannelConfig) -> TrustedResult<()> {
        let body = self.get_bounded("/version", VERSION_BODY_LIMIT).await?;
        let version: VersionInfo =
            serde_json::from_slice(&body).map_err(|_| TrustedChannelError::VersionParseFailed)?;
        if version.pid != config.expected_pid
            || version.started_at != config.expected_started_at
            || version.deployment_id != config.expected_deployment_id
            || version.management_protocol_version != config.expected_protocol_version
        {
            return Err(TrustedChannelError::VersionMismatch);
        }
        Ok(())
    }

    async fn verify_process(&mut self, config: &TrustedChannelConfig) -> TrustedResult<()> {
        let status = router_process::validate(
            &RouterIdentity {
                pid: config.expected_pid,
                started_at: config.expected_process_started_at.clone(),
                executable: config.expected_process_executable.clone(),
            },
            &config.expected_binary_path,
        );
        match status {
            RouterProcessStatus::Genuine => Ok(()),
            RouterProcessStatus::Stale => Err(TrustedChannelError::ProcessMismatch),
            RouterProcessStatus::Absent => Err(TrustedChannelError::ProcessMismatch),
        }
    }

    async fn verify_health(&mut self) -> TrustedResult<()> {
        let body = self.get_bounded("/health", 64 * 1024).await?;
        let health: HealthInfo =
            serde_json::from_slice(&body).map_err(|_| TrustedChannelError::HealthParseFailed)?;
        if health.status != "ok" {
            return Err(TrustedChannelError::HealthNotOk);
        }
        Ok(())
    }

    /// Fetches the `/v1/models/image` catalog with an API key.
    /// The caller must have called `verify_trust` first.
    pub async fn fetch_catalog(
        &mut self,
        api_key: &str,
        max_body: usize,
    ) -> TrustedResult<Vec<u8>> {
        tokio::time::timeout(
            Duration::from_secs(CONNECT_TIMEOUT_SECS),
            self.get_authed("/v1/models/image", api_key, max_body),
        )
        .await
        .map_err(|_| TrustedChannelError::IoError("catalog timeout".into()))?
    }

    /// Sends a generation request to `/v1/images/generations` with an API key.
    /// The caller must have called `verify_trust` first.
    pub async fn generate(
        &mut self,
        api_key: &str,
        request_body: &[u8],
        max_body: usize,
    ) -> TrustedResult<Vec<u8>> {
        tokio::time::timeout(
            Duration::from_secs(GENERATION_TIMEOUT_SECS),
            self.post_authed("/v1/images/generations", api_key, request_body, max_body),
        )
        .await
        .map_err(|_| TrustedChannelError::IoError("generation timeout".into()))?
    }

    async fn get_bounded(&mut self, path: &str, max_body: usize) -> TrustedResult<Vec<u8>> {
        self.send_request("GET", path, &[], None, max_body).await
    }

    async fn get_authed(
        &mut self,
        path: &str,
        api_key: &str,
        max_body: usize,
    ) -> TrustedResult<Vec<u8>> {
        if !self.trusted {
            return Err(TrustedChannelError::NotVerified);
        }
        let auth = Zeroizing::new(format!("Bearer {api_key}"));
        self.send_request(
            "GET",
            path,
            &[("Authorization", auth.as_str())],
            None,
            max_body,
        )
        .await
    }

    async fn post_authed(
        &mut self,
        path: &str,
        api_key: &str,
        body: &[u8],
        max_body: usize,
    ) -> TrustedResult<Vec<u8>> {
        if !self.trusted {
            return Err(TrustedChannelError::NotVerified);
        }
        let auth = Zeroizing::new(format!("Bearer {api_key}"));
        let cl = body.len().to_string();
        let headers: [(&str, &str); 3] = [
            ("Authorization", auth.as_str()),
            ("Content-Type", "application/json"),
            ("Content-Length", cl.as_str()),
        ];
        self.send_request("POST", path, &headers, Some(body), max_body)
            .await
    }

    async fn send_request(
        &mut self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
        max_body: usize,
    ) -> TrustedResult<Vec<u8>> {
        if self.connection_closed {
            return Err(TrustedChannelError::ConnectionClose);
        }
        let mut request = Zeroizing::new(format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\n",
            self.host
        ));
        for (name, value) in headers {
            request.push_str(name);
            request.push_str(": ");
            request.push_str(value);
            request.push_str("\r\n");
        }
        request.push_str("Connection: keep-alive\r\n");
        request.push_str("\r\n");
        self.stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| TrustedChannelError::IoError(e.to_string()))?;
        if let Some(body) = body {
            self.stream
                .write_all(body)
                .await
                .map_err(|e| TrustedChannelError::IoError(e.to_string()))?;
        }
        self.stream
            .flush()
            .await
            .map_err(|e| TrustedChannelError::IoError(e.to_string()))?;
        self.read_response(max_body).await
    }

    async fn read_response(&mut self, max_body: usize) -> TrustedResult<Vec<u8>> {
        let status = self.read_status_line().await?;
        let headers = self.read_headers().await?;
        if self.is_redirect(&status, &headers) {
            return Err(TrustedChannelError::Redirect);
        }
        if self.is_upgrade(&headers) {
            return Err(TrustedChannelError::ProtocolUpgrade);
        }
        if header_has_token(&headers, "connection", "close") {
            self.connection_closed = true;
        }
        let body = self.read_body(&headers, max_body).await?;
        if status != 200 {
            return Err(TrustedChannelError::BadStatus(status));
        }
        Ok(body)
    }

    async fn read_status_line(&mut self) -> TrustedResult<u16> {
        let mut line = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            let n = self
                .stream
                .read(&mut byte)
                .await
                .map_err(|e| TrustedChannelError::IoError(e.to_string()))?;
            if n == 0 {
                return Err(TrustedChannelError::ConnectionClose);
            }
            if byte[0] == b'\n' {
                break;
            }
            line.push(byte[0]);
            if line.len() > MAX_HTTP_HEAD_BYTES {
                return Err(TrustedChannelError::BodyTooLarge);
            }
        }
        let line = String::from_utf8_lossy(&line);
        let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
        if parts.len() < 2 || !parts[0].starts_with("HTTP/1.") {
            return Err(TrustedChannelError::IoError(format!(
                "invalid status line: {line}"
            )));
        }
        parts[1]
            .parse::<u16>()
            .map_err(|_| TrustedChannelError::IoError(format!("invalid status code: {line}")))
    }

    async fn read_headers(&mut self) -> TrustedResult<Vec<(String, String)>> {
        let mut headers = Vec::new();
        let mut total = 0_usize;
        loop {
            let mut line = Vec::new();
            loop {
                let mut byte = [0u8; 1];
                let n = self
                    .stream
                    .read(&mut byte)
                    .await
                    .map_err(|e| TrustedChannelError::IoError(e.to_string()))?;
                if n == 0 {
                    return Err(TrustedChannelError::ConnectionClose);
                }
                if byte[0] == b'\n' {
                    break;
                }
                line.push(byte[0]);
                total = total.saturating_add(1);
                if total > MAX_HTTP_HEAD_BYTES {
                    return Err(TrustedChannelError::BodyTooLarge);
                }
            }
            let line_str = String::from_utf8_lossy(&line);
            let trimmed = line_str.trim();
            if trimmed.is_empty() {
                break;
            }
            let (name, value) = trimmed
                .split_once(':')
                .ok_or_else(|| TrustedChannelError::IoError("malformed response header".into()))?;
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
                || value
                    .bytes()
                    .any(|byte| byte.is_ascii_control() && byte != b'\t')
            {
                return Err(TrustedChannelError::IoError(
                    "malformed response header".into(),
                ));
            }
            headers.push((name.to_owned(), value.trim().to_owned()));
        }
        Ok(headers)
    }

    async fn read_body(
        &mut self,
        headers: &[(String, String)],
        max_body: usize,
    ) -> TrustedResult<Vec<u8>> {
        let transfer_encodings: Vec<&str> = headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case("transfer-encoding"))
            .map(|(_, value)| value.as_str())
            .collect();
        let content_lengths: Vec<usize> = headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
            .map(|(_, value)| value.parse::<usize>())
            .collect::<Result<_, _>>()
            .map_err(|_| TrustedChannelError::BodyReadFailed)?;
        if transfer_encodings.len() > 1 || content_lengths.len() > 1 {
            return Err(TrustedChannelError::BodyReadFailed);
        }
        if let Some(encoding) = transfer_encodings.first() {
            if !content_lengths.is_empty() || !encoding.eq_ignore_ascii_case("chunked") {
                return Err(TrustedChannelError::BodyReadFailed);
            }
            return self.read_chunked_body(max_body).await;
        }
        let content_length = content_lengths.first().copied();
        match content_length {
            Some(len) => {
                if len > max_body {
                    return Err(TrustedChannelError::BodyTooLarge);
                }
                let mut body = vec![0u8; len];
                self.stream
                    .read_exact(&mut body)
                    .await
                    .map_err(|_| TrustedChannelError::BodyReadFailed)?;
                Ok(body)
            }
            None => {
                if !self.connection_closed {
                    return Err(TrustedChannelError::BodyReadFailed);
                }
                let mut body = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let n = self
                        .stream
                        .read(&mut buf)
                        .await
                        .map_err(|e| TrustedChannelError::IoError(e.to_string()))?;
                    if n == 0 {
                        break;
                    }
                    body.extend_from_slice(&buf[..n]);
                    if body.len() > max_body {
                        return Err(TrustedChannelError::BodyTooLarge);
                    }
                }
                Ok(body)
            }
        }
    }

    async fn read_chunked_body(&mut self, max_body: usize) -> TrustedResult<Vec<u8>> {
        let mut body = Vec::new();
        loop {
            let mut line = Vec::new();
            loop {
                let mut byte = [0u8; 1];
                let n = self
                    .stream
                    .read(&mut byte)
                    .await
                    .map_err(|e| TrustedChannelError::IoError(e.to_string()))?;
                if n == 0 {
                    return Err(TrustedChannelError::ConnectionClose);
                }
                if byte[0] == b'\n' {
                    break;
                }
                line.push(byte[0]);
                if line.len() > 128 {
                    return Err(TrustedChannelError::BodyReadFailed);
                }
            }
            let line_str = String::from_utf8_lossy(&line);
            let size_text = line_str.trim().split(';').next().unwrap_or("");
            let size = usize::from_str_radix(size_text, 16)
                .map_err(|_| TrustedChannelError::IoError("invalid chunk size".into()))?;
            if size == 0 {
                loop {
                    let mut trailer = Vec::new();
                    loop {
                        let mut byte = [0_u8; 1];
                        self.stream
                            .read_exact(&mut byte)
                            .await
                            .map_err(|_| TrustedChannelError::BodyReadFailed)?;
                        trailer.push(byte[0]);
                        if trailer.len() > MAX_HTTP_HEAD_BYTES {
                            return Err(TrustedChannelError::BodyTooLarge);
                        }
                        if byte[0] == b'\n' {
                            break;
                        }
                    }
                    if trailer == b"\r\n" || trailer == b"\n" {
                        break;
                    }
                }
                break;
            }
            if body.len() + size > max_body {
                return Err(TrustedChannelError::BodyTooLarge);
            }
            let mut chunk = vec![0u8; size + 2];
            self.stream
                .read_exact(&mut chunk)
                .await
                .map_err(|_| TrustedChannelError::BodyReadFailed)?;
            if chunk[size..] != *b"\r\n" {
                return Err(TrustedChannelError::BodyReadFailed);
            }
            body.extend_from_slice(&chunk[..size]);
        }
        Ok(body)
    }

    fn is_redirect(&self, status: &u16, _headers: &[(String, String)]) -> bool {
        (300..400).contains(status)
    }

    fn is_upgrade(&self, headers: &[(String, String)]) -> bool {
        headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("upgrade") && !v.is_empty())
    }
}

fn header_has_token(headers: &[(String, String)], name: &str, token: &str) -> bool {
    headers.iter().any(|(key, value)| {
        key.eq_ignore_ascii_case(name)
            && value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case(token))
    })
}

fn loopback_authority(addr: &str) -> TrustedResult<String> {
    let authority = addr.strip_prefix("http://").unwrap_or(addr);
    if authority.contains('/') || authority.contains('?') || authority.contains('#') {
        return Err(TrustedChannelError::NonLoopbackAddress);
    }
    let host = if authority.starts_with('[') {
        authority
            .find(']')
            .map(|end| &authority[1..end])
            .unwrap_or(authority)
    } else {
        authority
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(authority)
    };
    let allowed: HashSet<&str> = ["127.0.0.1", "localhost", "::1"].into_iter().collect();
    if allowed.contains(host) {
        return Ok(authority.to_owned());
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if ip.is_loopback() {
            return Ok(authority.to_owned());
        }
    }
    Err(TrustedChannelError::NonLoopbackAddress)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn read_request_head(stream: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        loop {
            let mut byte = [0_u8; 1];
            stream.read_exact(&mut byte).await.unwrap();
            bytes.push(byte[0]);
            if bytes.ends_with(b"\r\n\r\n") {
                return String::from_utf8(bytes).unwrap();
            }
        }
    }

    async fn write_json_response(stream: &mut TcpStream, body: &str) {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
    }

    fn trusted_config(address: std::net::SocketAddr, started_at: &str) -> TrustedChannelConfig {
        let identity = router_process::inspect(std::process::id()).unwrap();
        TrustedChannelConfig {
            listen_address: format!("http://{address}"),
            expected_pid: identity.pid,
            expected_started_at: started_at.into(),
            expected_process_started_at: identity.started_at,
            expected_process_executable: identity.executable.clone(),
            expected_deployment_id: "prod-a".into(),
            expected_protocol_version: "4".into(),
            expected_binary_path: identity.executable,
        }
    }

    fn version_json(started_at: &str, pid: u32) -> String {
        serde_json::json!({
            "version": "v1",
            "commit": "fixture",
            "build_date": "2026-08-01",
            "pid": pid,
            "started_at": started_at,
            "deployment_id": "prod-a",
            "management_protocol_version": "4",
        })
        .to_string()
    }

    #[test]
    fn rejects_non_loopback_address() {
        assert!(matches!(
            loopback_authority("10.0.0.1:12345"),
            Err(TrustedChannelError::NonLoopbackAddress)
        ));
        assert!(matches!(
            loopback_authority("0.0.0.0:12345"),
            Err(TrustedChannelError::NonLoopbackAddress)
        ));
    }

    #[test]
    fn accepts_loopback_addresses() {
        assert_eq!(
            loopback_authority("http://127.0.0.1:12345").unwrap(),
            "127.0.0.1:12345"
        );
        assert!(loopback_authority("127.0.1.2:12345").is_ok());
        assert!(loopback_authority("localhost:12345").is_ok());
        assert!(loopback_authority("[::1]:12345").is_ok());
    }

    #[tokio::test]
    async fn authenticated_request_is_blocked_until_trust_succeeds() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let reader = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut byte = [0_u8; 1];
            tokio::time::timeout(Duration::from_millis(100), stream.read(&mut byte))
                .await
                .is_ok()
        });
        let config = TrustedChannelConfig {
            listen_address: address.to_string(),
            expected_pid: 1,
            expected_started_at: "start".into(),
            expected_process_started_at: "process-start".into(),
            expected_process_executable: "/router".into(),
            expected_deployment_id: "prod-a".into(),
            expected_protocol_version: "4".into(),
            expected_binary_path: "/router".into(),
        };
        let mut channel = TrustedChannel::connect(&config).await.unwrap();
        assert!(matches!(
            channel.fetch_catalog("secret", 1024).await,
            Err(TrustedChannelError::NotVerified)
        ));
        assert!(
            !reader.await.unwrap(),
            "request bytes were sent before trust"
        );
    }

    #[tokio::test]
    async fn trust_and_authentication_share_one_tcp_connection() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let started_at = "2026-08-01T00:00:00Z";
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(read_request_head(&mut stream)
                .await
                .starts_with("GET /version HTTP/1.1\r\n"));
            write_json_response(&mut stream, &version_json(started_at, std::process::id())).await;

            assert!(read_request_head(&mut stream)
                .await
                .starts_with("GET /health HTTP/1.1\r\n"));
            write_json_response(&mut stream, r#"{"status":"ok","upstream":"reachable"}"#).await;

            let catalog_request = read_request_head(&mut stream).await;
            assert!(catalog_request.starts_with("GET /v1/models/image HTTP/1.1\r\n"));
            assert!(catalog_request.contains("Authorization: Bearer fixture-key\r\n"));
            write_json_response(&mut stream, r#"{"object":"list","data":[]}"#).await;
        });
        let config = trusted_config(address, started_at);

        let mut channel = TrustedChannel::connect(&config).await.unwrap();
        channel.verify_trust(&config).await.unwrap();
        let catalog = channel.fetch_catalog("fixture-key", 1024).await.unwrap();
        assert_eq!(catalog, br#"{"object":"list","data":[]}"#);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn trust_failures_never_send_authorization_or_redial() {
        for scenario in [
            "pid",
            "deployment",
            "protocol",
            "redirect",
            "upgrade",
            "close",
            "health",
            "process",
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let started_at = "2026-08-01T00:00:00Z";
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let version_request = read_request_head(&mut stream).await;
                assert!(!version_request.contains("Authorization:"));
                match scenario {
                    "redirect" => {
                        stream
                            .write_all(b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\n\r\n")
                            .await
                            .unwrap();
                    }
                    "upgrade" => {
                        stream
                            .write_all(
                                b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nContent-Length: 0\r\n\r\n",
                            )
                            .await
                            .unwrap();
                    }
                    "close" => {
                        let body = version_json(started_at, std::process::id());
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        stream.write_all(response.as_bytes()).await.unwrap();
                    }
                    "pid" | "deployment" | "protocol" => {
                        let mut version: serde_json::Value = serde_json::from_str(&version_json(
                            started_at,
                            if scenario == "pid" {
                                std::process::id() + 1
                            } else {
                                std::process::id()
                            },
                        ))
                        .unwrap();
                        if scenario == "deployment" {
                            version["deployment_id"] = "other".into();
                        } else if scenario == "protocol" {
                            version["management_protocol_version"] = "other".into();
                        }
                        write_json_response(&mut stream, &version.to_string()).await;
                    }
                    "health" | "process" => {
                        write_json_response(
                            &mut stream,
                            &version_json(started_at, std::process::id()),
                        )
                        .await;
                        if scenario == "health" {
                            let health_request = read_request_head(&mut stream).await;
                            assert!(!health_request.contains("Authorization:"));
                            write_json_response(
                                &mut stream,
                                r#"{"status":"degraded","upstream":"unreachable"}"#,
                            )
                            .await;
                        }
                    }
                    _ => unreachable!(),
                }
                assert!(
                    tokio::time::timeout(Duration::from_millis(50), listener.accept())
                        .await
                        .is_err(),
                    "{scenario} attempted to redial"
                );
            });
            let mut config = trusted_config(address, started_at);
            if scenario == "process" {
                config.expected_process_started_at.push_str("-changed");
            }
            let mut channel = TrustedChannel::connect(&config).await.unwrap();
            let error = channel.verify_trust(&config).await.unwrap_err();
            match scenario {
                "pid" | "deployment" | "protocol" => {
                    assert_eq!(error, TrustedChannelError::VersionMismatch)
                }
                "redirect" => assert_eq!(error, TrustedChannelError::Redirect),
                "upgrade" => assert_eq!(error, TrustedChannelError::ProtocolUpgrade),
                "close" => assert_eq!(error, TrustedChannelError::ConnectionClose),
                "health" => assert_eq!(error, TrustedChannelError::HealthNotOk),
                "process" => assert_eq!(error, TrustedChannelError::ProcessMismatch),
                _ => unreachable!(),
            }
            drop(channel);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn version_timeout_fails_before_authorization() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request_head(&mut stream).await;
            assert!(!request.contains("Authorization:"));
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        let config = trusted_config(address, "2026-08-01T00:00:00Z");
        let mut channel = TrustedChannel::connect(&config).await.unwrap();
        assert!(matches!(
            channel
                .verify_trust_with_timeout(&config, Duration::from_millis(20))
                .await,
            Err(TrustedChannelError::IoError(message)) if message.contains("timeout")
        ));
        server.await.unwrap();
    }

    #[test]
    fn version_info_parses() {
        let json = br#"{"version":"v1","commit":"abc","build_date":"today","pid":12345,"started_at":"start","deployment_id":"prod-a","management_protocol_version":"4"}"#;
        let v: VersionInfo = serde_json::from_slice(json).unwrap();
        assert_eq!(v.pid, 12345);
        assert_eq!(v.deployment_id, "prod-a");
        assert_eq!(v.management_protocol_version, "4");
    }

    #[test]
    fn version_info_accepts_real_router_fields() {
        let json = br#"{"pid":1,"started_at":"start","deployment_id":"a","management_protocol_version":"4","version":"v1","commit":"abc","build_date":"today"}"#;
        assert!(serde_json::from_slice::<VersionInfo>(json).is_ok());
    }

    #[test]
    fn health_info_parses() {
        let json = br#"{"status":"ok","upstream":"reachable"}"#;
        let h: HealthInfo = serde_json::from_slice(json).unwrap();
        assert_eq!(h.status, "ok");
    }

    #[test]
    fn conformance_vectors_load_and_cover_required_scenarios() {
        let manifest = env!("CARGO_MANIFEST_DIR");
        let path = format!(
            "{manifest}/../../internal/manager/trustedrouter/testdata/conformance_vectors.json"
        );
        let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        let cf: serde_json::Value = serde_json::from_slice(&data).expect("parse vectors");
        let vectors = cf["vectors"].as_array().expect("vectors is array");
        assert!(!vectors.is_empty(), "no conformance vectors");
        let required = [
            "version_pid_mismatch",
            "version_deployment_mismatch",
            "version_protocol_mismatch",
            "process_identity_mismatch",
            "non_loopback_address",
            "redirect_response",
            "connection_close_after_version",
            "protocol_upgrade",
            "timeout_during_version",
            "cancel_during_generation",
            "redial_attempt",
            "health_not_ok",
        ];
        let names: std::collections::HashSet<&str> = vectors
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect();
        for req in &required {
            assert!(names.contains(*req), "missing required vector: {req}");
        }
        for v in vectors {
            let outcome = v["expected_outcome"].as_str().unwrap();
            assert!(
                outcome == "fail_before_auth"
                    || outcome == "fail_after_auth"
                    || outcome == "success",
                "invalid outcome: {outcome}"
            );
            if outcome == "fail_before_auth" {
                assert_eq!(
                    v["auth_must_never_be_sent"], true,
                    "fail_before_auth must never send auth"
                );
            }
        }
        let invariants = cf["invariants"].as_array().expect("invariants is array");
        assert!(invariants.len() >= 5, "need at least 5 invariants");
    }
}
