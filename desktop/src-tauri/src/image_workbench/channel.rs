use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::manager_core::process::{self, Identity, ProcessError, Status};

use super::error::{SafeKind, WorkbenchError};

pub const VERSION_LIMIT: usize = 64 * 1024;
pub const HEALTH_LIMIT: usize = 64 * 1024;
pub const CATALOG_LIMIT: usize = 1 << 20;
pub const GENERATION_LIMIT: usize = 32 * 1024 * 1024;
pub const BODY_BUDGET: Duration = Duration::from_secs(180);
const HEADER_LIMIT: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct TrustTarget {
    pub authority: String,
    pub pid: i32,
    pub deployment_id: String,
    pub protocol_version: String,
    pub started_at: String,
    pub executable: String,
    pub binary_path: String,
}

pub type DialFn = Arc<dyn Fn(&str) -> std::io::Result<TcpStream> + Send + Sync>;
pub type ValidateFn = Arc<dyn Fn(&Identity, &str) -> Result<Status, ProcessError> + Send + Sync>;

#[derive(Clone, Default)]
pub struct ChannelHooks {
    pub dial: Option<DialFn>,
    pub validate: Option<ValidateFn>,
}

pub struct BoundSession {
    stream: TcpStream,
    authority: String,
    closed: bool,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub connection_close: bool,
    pub content_type: String,
}

impl BoundSession {
    pub fn connect(
        target: &TrustTarget,
        hooks: &ChannelHooks,
        deadline: Instant,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, WorkbenchError> {
        if !is_loopback_authority(&target.authority) {
            return Err(WorkbenchError::new(SafeKind::Identity));
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(WorkbenchError::new(SafeKind::Cancelled));
        }
        let stream = match &hooks.dial {
            Some(dial) => {
                dial(&target.authority).map_err(|_| WorkbenchError::new(SafeKind::NotReady))?
            }
            None => {
                let timeout = remaining(deadline)?;
                let addr = target
                    .authority
                    .to_socket_addrs()
                    .map_err(|_| WorkbenchError::new(SafeKind::NotReady))?
                    .next()
                    .ok_or_else(|| WorkbenchError::new(SafeKind::NotReady))?;
                TcpStream::connect_timeout(&addr, timeout)
                    .map_err(|_| WorkbenchError::new(SafeKind::Timeout))?
            }
        };
        stream
            .set_nodelay(true)
            .map_err(|_| WorkbenchError::new(SafeKind::NotReady))?;
        let mut session = Self {
            stream,
            authority: target.authority.clone(),
            closed: false,
            cancel,
        };
        session.handshake(target, hooks, deadline)?;
        Ok(session)
    }

    fn handshake(
        &mut self,
        target: &TrustTarget,
        hooks: &ChannelHooks,
        deadline: Instant,
    ) -> Result<(), WorkbenchError> {
        let version = self.request(
            "GET",
            "/version",
            None,
            b"",
            deadline,
            VERSION_LIMIT + 1,
            false,
        )?;
        if is_redirect(version.status) {
            return Err(WorkbenchError::new(SafeKind::Identity));
        }
        if version.status == 101 || version.content_type.contains("upgrade") {
            return Err(WorkbenchError::new(SafeKind::Identity));
        }
        if version.status != 200 || version.body.len() > VERSION_LIMIT {
            return Err(WorkbenchError::new(SafeKind::Identity));
        }
        let remote: WireVersion = serde_json::from_slice(&version.body)
            .map_err(|_| WorkbenchError::new(SafeKind::Identity))?;
        if remote.pid <= 0
            || remote.pid != target.pid
            || remote.deployment_id != target.deployment_id
            || remote.management_protocol_version != target.protocol_version
        {
            return Err(WorkbenchError::new(SafeKind::Identity));
        }
        let identity = Identity {
            pid: target.pid,
            started_at: target.started_at.clone(),
            executable: target.executable.clone(),
        };
        let status = match &hooks.validate {
            Some(validate) => validate(&identity, &target.binary_path),
            None => process::validate(&identity, &target.binary_path),
        };
        if !matches!(status, Ok(Status::Genuine)) {
            return Err(WorkbenchError::new(SafeKind::Identity));
        }
        if version.connection_close {
            self.closed = true;
            return Err(WorkbenchError::new(SafeKind::Redial));
        }
        let health = self.request(
            "GET",
            "/health",
            None,
            b"",
            deadline,
            HEALTH_LIMIT + 1,
            false,
        )?;
        if health.status != 200 {
            return Err(WorkbenchError::new(SafeKind::NotReady));
        }
        let payload: WireHealth = serde_json::from_slice(&health.body)
            .map_err(|_| WorkbenchError::new(SafeKind::NotReady))?;
        if payload.status != "ok" {
            return Err(WorkbenchError::new(SafeKind::NotReady));
        }
        if health.connection_close {
            self.closed = true;
            return Err(WorkbenchError::new(SafeKind::Redial));
        }
        Ok(())
    }

    pub fn get(
        &mut self,
        path: &str,
        authorization: &str,
        deadline: Instant,
        max_body: usize,
    ) -> Result<HttpResponse, WorkbenchError> {
        self.request(
            "GET",
            path,
            Some(authorization),
            b"",
            deadline,
            max_body,
            true,
        )
    }

    pub fn post(
        &mut self,
        path: &str,
        authorization: &str,
        body: &[u8],
        deadline: Instant,
        max_body: usize,
    ) -> Result<HttpResponse, WorkbenchError> {
        self.request(
            "POST",
            path,
            Some(authorization),
            body,
            deadline,
            max_body,
            true,
        )
    }

    pub fn post_sse<F>(
        &mut self,
        path: &str,
        authorization: &str,
        body: &[u8],
        deadline: Instant,
        mut on_delta: F,
    ) -> Result<String, WorkbenchError>
    where
        F: FnMut(&str),
    {
        let headers =
            self.write_and_read_headers("POST", path, Some(authorization), body, deadline, true)?;
        if headers.status != 200 {
            return Err(WorkbenchError::new(SafeKind::ChatFailed));
        }
        let mut raw = String::new();
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        let mut idle_deadline = Instant::now() + BODY_BUDGET;
        loop {
            self.check(deadline.min(idle_deadline))?;
            match self.stream.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {
                    idle_deadline = Instant::now() + BODY_BUDGET;
                    if byte[0] == b'\n' {
                        let text = String::from_utf8_lossy(&line).trim().to_owned();
                        line.clear();
                        if let Some(piece) = sse_content(&text) {
                            raw.push_str(&piece);
                            on_delta(&raw);
                        }
                    } else if byte[0] != b'\r' {
                        if line.len() > 1024 * 1024 {
                            return Err(WorkbenchError::new(SafeKind::ChatFailed));
                        }
                        line.push(byte[0]);
                    }
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    return Err(WorkbenchError::new(SafeKind::Timeout));
                }
                Err(_) => return Err(WorkbenchError::new(SafeKind::ChatFailed)),
            }
        }
        Ok(raw)
    }

    pub fn shutdown(&self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }

    fn request(
        &mut self,
        method: &str,
        path: &str,
        authorization: Option<&str>,
        body: &[u8],
        deadline: Instant,
        max_body: usize,
        authenticated: bool,
    ) -> Result<HttpResponse, WorkbenchError> {
        let headers = self.write_and_read_headers(
            method,
            path,
            authorization,
            body,
            deadline,
            authenticated,
        )?;
        let body = self.read_body(&headers, deadline, max_body)?;
        if headers.connection_close {
            self.closed = true;
        }
        Ok(HttpResponse {
            status: headers.status,
            body,
            connection_close: headers.connection_close,
            content_type: headers.content_type,
        })
    }

    fn write_and_read_headers(
        &mut self,
        method: &str,
        path: &str,
        authorization: Option<&str>,
        body: &[u8],
        deadline: Instant,
        authenticated: bool,
    ) -> Result<ParsedHeaders, WorkbenchError> {
        if self.closed {
            return Err(WorkbenchError::new(SafeKind::Redial));
        }
        self.check(deadline)?;
        if let Some(value) = authorization {
            if value.bytes().any(|byte| byte == b'\r' || byte == b'\n') {
                return Err(WorkbenchError::new(SafeKind::NotReady));
            }
        }
        if !valid_path(path) {
            return Err(WorkbenchError::new(SafeKind::NotReady));
        }
        let mut message = format!("{method} {path} HTTP/1.1\r\nHost: {}\r\n", self.authority);
        if let Some(value) = authorization {
            message.push_str("Authorization: ");
            message.push_str(value);
            message.push_str("\r\n");
        }
        if method == "POST" {
            message.push_str("Content-Type: application/json\r\n");
            message.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        message.push_str("Connection: keep-alive\r\n\r\n");
        apply_deadline(&self.stream, deadline)?;
        self.stream.write_all(message.as_bytes()).map_err(map_io)?;
        if !body.is_empty() {
            self.stream.write_all(body).map_err(map_io)?;
        }
        self.stream.flush().map_err(map_io)?;
        let header_bytes = self.read_until_headers(deadline)?;
        let parsed = parse_headers(&header_bytes)?;
        if authenticated && parsed.status == 101 {
            return Err(WorkbenchError::new(SafeKind::Redial));
        }
        if is_redirect(parsed.status) {
            return Err(WorkbenchError::new(SafeKind::Identity));
        }
        Ok(parsed)
    }

    fn read_until_headers(&mut self, deadline: Instant) -> Result<Vec<u8>, WorkbenchError> {
        let mut buffer = Vec::new();
        let mut byte = [0u8; 1];
        while buffer.len() < HEADER_LIMIT {
            self.check(deadline)?;
            if self.stream.read(&mut byte).map_err(map_io)? != 1 {
                return Err(WorkbenchError::new(SafeKind::NotReady));
            }
            buffer.push(byte[0]);
            if buffer.ends_with(b"\r\n\r\n") {
                return Ok(buffer);
            }
        }
        Err(WorkbenchError::new(SafeKind::NotReady))
    }

    fn read_body(
        &mut self,
        headers: &ParsedHeaders,
        deadline: Instant,
        max_body: usize,
    ) -> Result<Vec<u8>, WorkbenchError> {
        if let Some(length) = headers.content_length {
            if length > max_body {
                return Err(WorkbenchError::new(SafeKind::ImageTooLarge));
            }
            let mut body = vec![0u8; length];
            let mut offset = 0;
            while offset < length {
                self.check(deadline)?;
                let read = self.stream.read(&mut body[offset..]).map_err(map_io)?;
                if read == 0 {
                    return Err(WorkbenchError::new(SafeKind::NotReady));
                }
                offset += read;
            }
            return Ok(body);
        }
        if headers.chunked {
            let mut body = Vec::new();
            loop {
                let line = self.read_line(deadline)?;
                let size = usize::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16)
                    .map_err(|_| WorkbenchError::new(SafeKind::NotReady))?;
                if size == 0 {
                    let _ = self.read_line(deadline)?;
                    return Ok(body);
                }
                if body.len().saturating_add(size) > max_body {
                    return Err(WorkbenchError::new(SafeKind::ImageTooLarge));
                }
                let mut chunk = vec![0u8; size];
                self.stream.read_exact(&mut chunk).map_err(map_io)?;
                body.extend(chunk);
                let trailer = self.read_line(deadline)?;
                if !trailer.is_empty() {
                    return Err(WorkbenchError::new(SafeKind::NotReady));
                }
            }
        }
        Ok(Vec::new())
    }

    fn read_line(&mut self, deadline: Instant) -> Result<String, WorkbenchError> {
        let mut buffer = Vec::new();
        let mut byte = [0u8; 1];
        while buffer.len() < 1024 {
            self.check(deadline)?;
            if self.stream.read(&mut byte).map_err(map_io)? != 1 {
                return Err(WorkbenchError::new(SafeKind::NotReady));
            }
            buffer.push(byte[0]);
            if buffer.ends_with(b"\r\n") {
                buffer.truncate(buffer.len() - 2);
                return String::from_utf8(buffer)
                    .map_err(|_| WorkbenchError::new(SafeKind::NotReady));
            }
        }
        Err(WorkbenchError::new(SafeKind::NotReady))
    }

    fn check(&self, deadline: Instant) -> Result<(), WorkbenchError> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(WorkbenchError::new(SafeKind::Cancelled));
        }
        apply_deadline(&self.stream, deadline)
    }
}

#[derive(Deserialize)]
struct WireVersion {
    #[serde(default)]
    pid: i32,
    #[serde(default)]
    deployment_id: String,
    #[serde(default)]
    management_protocol_version: String,
}

#[derive(Deserialize)]
struct WireHealth {
    #[serde(default)]
    status: String,
}

struct ParsedHeaders {
    status: u16,
    connection_close: bool,
    content_length: Option<usize>,
    chunked: bool,
    content_type: String,
}

fn parse_headers(bytes: &[u8]) -> Result<ParsedHeaders, WorkbenchError> {
    let text = std::str::from_utf8(bytes).map_err(|_| WorkbenchError::new(SafeKind::NotReady))?;
    let mut lines = text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| WorkbenchError::new(SafeKind::NotReady))?;
    let mut parts = status_line.split_whitespace();
    let version = parts
        .next()
        .ok_or_else(|| WorkbenchError::new(SafeKind::NotReady))?;
    if version != "HTTP/1.0" && version != "HTTP/1.1" {
        return Err(WorkbenchError::new(SafeKind::NotReady));
    }
    let status: u16 = parts
        .next()
        .ok_or_else(|| WorkbenchError::new(SafeKind::NotReady))?
        .parse()
        .map_err(|_| WorkbenchError::new(SafeKind::NotReady))?;
    let mut connection_close = version == "HTTP/1.0";
    let mut content_length = None;
    let mut chunked = false;
    let mut content_type = String::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("Connection") {
            connection_close = value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("close"));
        } else if name.eq_ignore_ascii_case("Content-Length") {
            content_length = Some(
                value
                    .parse()
                    .map_err(|_| WorkbenchError::new(SafeKind::NotReady))?,
            );
        } else if name.eq_ignore_ascii_case("Transfer-Encoding") {
            chunked = value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("chunked"));
        } else if name.eq_ignore_ascii_case("Content-Type") {
            content_type = value.to_ascii_lowercase();
        } else if name.eq_ignore_ascii_case("Upgrade") {
            return Err(WorkbenchError::new(SafeKind::Identity));
        }
    }
    Ok(ParsedHeaders {
        status,
        connection_close,
        content_length,
        chunked,
        content_type,
    })
}

fn sse_content(line: &str) -> Option<String> {
    let payload = line.strip_prefix("data:")?.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    value
        .pointer("/choices/0/delta/content")
        .and_then(|item| item.as_str())
        .map(ToOwned::to_owned)
}

fn is_loopback_authority(authority: &str) -> bool {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let Some((host, port)) = rest.split_once("]:") else {
            return false;
        };
        (host, port)
    } else {
        authority.rsplit_once(':').unwrap_or((authority, ""))
    };
    let Ok(port) = port.parse::<u16>() else {
        return false;
    };
    if port == 0 {
        return false;
    }
    host == "127.0.0.1"
        || host == "::1"
        || host.split('.').next().is_some_and(|first| first == "127")
}

fn valid_path(path: &str) -> bool {
    path.starts_with('/')
        && !path
            .bytes()
            .any(|byte| matches!(byte, b' ' | b'\r' | b'\n'))
}

fn is_redirect(status: u16) -> bool {
    (300..400).contains(&status)
}

fn remaining(deadline: Instant) -> Result<Duration, WorkbenchError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|value| *value > Duration::ZERO)
        .ok_or_else(|| WorkbenchError::new(SafeKind::Timeout))
}

fn apply_deadline(stream: &TcpStream, deadline: Instant) -> Result<(), WorkbenchError> {
    let timeout = remaining(deadline)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|_| WorkbenchError::new(SafeKind::Timeout))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|_| WorkbenchError::new(SafeKind::Timeout))?;
    Ok(())
}

fn map_io(error: std::io::Error) -> WorkbenchError {
    if error.kind() == std::io::ErrorKind::TimedOut
        || error.kind() == std::io::ErrorKind::WouldBlock
    {
        WorkbenchError::new(SafeKind::Timeout)
    } else {
        WorkbenchError::new(SafeKind::NotReady)
    }
}

pub fn generation_body(
    model: &str,
    prompt: &str,
    size: &str,
    reference_data_uri: Option<&str>,
) -> Vec<u8> {
    let mut value = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "n": 1,
        "size": size,
    });
    if let Some(uri) = reference_data_uri {
        value["image"] = serde_json::Value::String(uri.to_owned());
        value["images"] = serde_json::json!([uri]);
    }
    serde_json::to_vec(&value).unwrap_or_default()
}

pub fn decode_generation(response: &HttpResponse) -> Result<Vec<u8>, WorkbenchError> {
    if looks_like_image(&response.body) {
        return Ok(response.body.clone());
    }
    if !response.content_type.contains("json") && !response.body.starts_with(b"{") {
        return Err(WorkbenchError::new(SafeKind::ImageFailed));
    }
    let value: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|_| WorkbenchError::new(SafeKind::ImageFailed))?;
    let data = value.get("data").and_then(|item| item.as_array());
    let Some(items) = data else {
        return Err(WorkbenchError::new(SafeKind::ImageFailed));
    };
    if items.len() != 1 {
        return Err(WorkbenchError::new(SafeKind::ImageFailed));
    }
    if let Some(b64) = items[0].get("b64_json").and_then(|item| item.as_str()) {
        use base64::Engine;
        return base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|_| WorkbenchError::new(SafeKind::ImageFailed));
    }
    if items[0].get("url").is_some() {
        return Err(WorkbenchError::new(SafeKind::ImageFailed));
    }
    Err(WorkbenchError::new(SafeKind::ImageFailed))
}

fn looks_like_image(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || (bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff)
        || (bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_url_only_and_accepts_binary_or_b64() {
        let png = include_bytes!("../../../../internal/proxy/testdata/generation_binary.png");
        let binary = HttpResponse {
            status: 200,
            body: png.to_vec(),
            connection_close: false,
            content_type: "application/octet-stream".into(),
        };
        assert_eq!(decode_generation(&binary).unwrap(), png);
        let b64 = include_bytes!("../../../../internal/proxy/testdata/generation_b64_json.json");
        let json = HttpResponse {
            status: 200,
            body: b64.to_vec(),
            connection_close: false,
            content_type: "application/json".into(),
        };
        assert_eq!(decode_generation(&json).unwrap(), png);
        let url = include_bytes!("../../../../internal/proxy/testdata/generation_url_only.json");
        let only = HttpResponse {
            status: 200,
            body: url.to_vec(),
            connection_close: false,
            content_type: "application/json".into(),
        };
        assert!(decode_generation(&only).is_err());
    }

    #[test]
    fn loopback_only() {
        assert!(is_loopback_authority("127.0.0.1:19099"));
        assert!(!is_loopback_authority("10.66.0.2:19099"));
    }

    #[test]
    fn edit_body_includes_image_and_images() {
        let body = generation_body(
            "cx/gpt-5.5-image",
            "a lamp",
            "1024x1024",
            Some("data:image/png;base64,abc"),
        );
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["n"], 1);
        assert_eq!(value["image"], "data:image/png;base64,abc");
        assert_eq!(value["images"][0], "data:image/png;base64,abc");
        assert!(!value.to_string().contains("sk-"));
    }
}
