#![allow(dead_code)]

//! Shared redaction contract for the future embedded manager, router-core,
//! supervisor, diagnostics, and panic containment.
//!
//! Surfaces use field allowlists. Residual free text is sanitized with the
//! same replacement rules as Go `internal/manager/app.sanitizeText`. This
//! module is not wired into the current sidecar runtime path.

use crate::protocol::{ErrorCode, ErrorDetails, ProtocolError};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::Duration;

pub const REDACTED_PEM: &str = "[REDACTED PEM]";
pub const REDACTED: &str = "[REDACTED]";
pub const REDACTED_KEY: &str = "[REDACTED KEY]";
pub const REDACTED_BODY: &str = "[REDACTED BODY]";
pub const MAX_DIAGNOSTICS_SIZE: usize = 16 * 1024;

struct Patterns {
    pem: Regex,
    auth_header: Regex,
    key: Regex,
    sk: Regex,
    url_userinfo: Regex,
    url_query: Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| Patterns {
        pem: Regex::new(r"(?s)-----BEGIN [^-\r\n]+-----.*?(?:-----END [^-\r\n]+-----|$)")
            .expect("pem pattern"),
        auth_header: Regex::new(
            r"(?i)((?:authorization|proxy-authorization|authentication)\s*[:=]\s*)[^\r\n]*",
        )
        .expect("auth header pattern"),
        key: Regex::new(
            r#"(?i)((?:api[_-]?key|auth(?:entication|orization)?|token|secret|password)["']?\s*[:=]\s*["']?)([^"'\s,}]+)"#,
        )
        .expect("key pattern"),
        sk: Regex::new(r"\bsk-[A-Za-z0-9_-]{8,}\b").expect("sk pattern"),
        url_userinfo: Regex::new(r#"(?i)(https?://)[^/\s?#"']+@"#).expect("url userinfo pattern"),
        url_query: Regex::new(r#"(?i)(https?://[^\s?"']+)\?[^\s"']+"#).expect("url query pattern"),
    })
}

pub fn sanitize_text(value: &str) -> String {
    let patterns = patterns();
    let value = patterns.pem.replace_all(value, REDACTED_PEM);
    let value = patterns
        .auth_header
        .replace_all(&value, format!("${{1}}{REDACTED}"));
    // Userinfo must run before sk-/key replacements. Those replacements insert
    // spaces (`[REDACTED KEY]`) that would otherwise break URL matching.
    let value = patterns
        .url_userinfo
        .replace_all(&value, format!("${{1}}{REDACTED}@"));
    let value = patterns
        .key
        .replace_all(&value, format!("${{1}}{REDACTED}"));
    let value = patterns.sk.replace_all(&value, REDACTED_KEY);
    patterns
        .url_query
        .replace_all(&value, format!("${{1}}?{REDACTED}"))
        .into_owned()
}

pub fn sanitize_public_text(value: &str) -> String {
    redact_json_literals(&sanitize_text(value))
}

fn redact_json_literals(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    let bytes = value.as_bytes();
    while index < bytes.len() {
        if bytes[index] == b'{' || bytes[index] == b'[' {
            if let Some(end) = find_json_literal_end(value, index) {
                let candidate = &value[index..=end];
                if serde_json::from_str::<Value>(candidate).is_ok() {
                    output.push_str(REDACTED_BODY);
                    index = end + 1;
                    continue;
                }
            }
        }
        let next = value[index..]
            .chars()
            .next()
            .map(|ch| ch.len_utf8())
            .unwrap_or(1);
        output.push_str(&value[index..index + next]);
        index += next;
    }
    output
}

fn find_json_literal_end(input: &str, start: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escape = false;
    for (offset, &byte) in bytes[start..].iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            if byte == b'\\' {
                escape = true;
                continue;
            }
            if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => stack.push(byte),
            b'}' | b']' => {
                let open = stack.pop()?;
                if (open == b'{' && byte != b'}') || (open == b'[' && byte != b']') {
                    return None;
                }
                if stack.is_empty() {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn bound_text(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let end = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|&index| index <= limit)
        .last()
        .unwrap_or(0);
    format!("{}[truncated]", &value[..end])
}

pub fn request_path(raw_target: &str) -> String {
    let without_fragment = raw_target
        .split_once('#')
        .map(|(path, _)| path)
        .unwrap_or(raw_target);
    let without_query = without_fragment
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(without_fragment);
    if let Some(scheme_end) = without_query.find("://") {
        let after_scheme = &without_query[scheme_end + 3..];
        after_scheme
            .find('/')
            .map(|index| after_scheme[index..].to_owned())
            .unwrap_or_else(|| "/".to_owned())
    } else if without_query.is_empty() {
        "/".to_owned()
    } else {
        without_query.to_owned()
    }
}

pub fn public_upstream_origin(raw: &str) -> Option<String> {
    let url = raw.trim();
    let scheme_end = url.find("://")?;
    let scheme = &url[..scheme_end];
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let rest = &url[scheme_end + 3..];
    let hostport = rest
        .split_once(['/', '?', '#'])
        .map(|(host, _)| host)
        .unwrap_or(rest);
    let host = hostport
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(hostport);
    if host.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host}"))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AccessLogRecord {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub bytes: u64,
    pub latency: u64,
}

impl AccessLogRecord {
    pub fn new(
        method: impl Into<String>,
        raw_target: &str,
        status: u16,
        bytes: u64,
        latency: Duration,
    ) -> Self {
        Self {
            method: method.into(),
            path: request_path(raw_target),
            status,
            bytes,
            latency: u64::try_from(latency.as_millis()).unwrap_or(u64::MAX),
        }
    }
}

pub fn proxy_error_json(status: u16) -> Value {
    json!({ "error": closed_status_text(status) })
}

fn closed_status_text(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        502 => "Bad Gateway",
        504 => "Gateway Timeout",
        _ => "Bad Gateway",
    }
}

pub fn sanitize_protocol_error(
    code: ErrorCode,
    message: &str,
    details: Option<ErrorDetails>,
) -> ProtocolError {
    ProtocolError {
        code,
        message: sanitize_public_text(message),
        details,
    }
}

pub fn sanitize_diagnostic_summary(summary: &str) -> String {
    bound_text(&sanitize_public_text(summary), MAX_DIAGNOSTICS_SIZE)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainedFailureKind {
    RouterPanic,
    RequestTimeout,
    ListenFailed,
    ShutdownTimeout,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContainedFailure {
    pub kind: ContainedFailureKind,
    pub code: ErrorCode,
}

pub fn contain_panic(_payload: &str) -> ContainedFailure {
    ContainedFailure {
        kind: ContainedFailureKind::RouterPanic,
        code: ErrorCode::RouterDegraded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager_diagnostics::{
        ManagerDiagnosticRing, STAGE_HANDSHAKE, STAGE_SPAWN, STAGE_UNEXPECTED_EXIT,
    };
    use crate::protocol::Response;
    use serde_json::Map;

    const API_KEY: &str = "sk-test-canary-key-1a2b3c4d";
    const REQUEST_BODY: &str = r#"{"messages":[{"content":"body-canary-7g8h9i"}]}"#;
    const PEM_BODY: &str = "private-pem-canary";
    const USERINFO_USER: &str = "auth-user-canary";
    const USERINFO_PASSWORD: &str = "sk-password-canary";
    const UPSTREAM_PATH: &str = "private-path-canary";
    const QUERY_KEY: &str = "sk-query-canary";
    const BEARER: &str = "Bearer auth-canary-4d5e6f";

    fn pem_block() -> String {
        format!("-----BEGIN PRIVATE KEY-----\n{PEM_BODY}\n-----END PRIVATE KEY-----")
    }

    fn full_upstream_url() -> String {
        format!(
            "https://{USERINFO_USER}:{USERINFO_PASSWORD}@upstream.example:8443/{UPSTREAM_PATH}?api_key={QUERY_KEY}"
        )
    }

    fn credential_canaries() -> Vec<&'static str> {
        vec![
            API_KEY,
            PEM_BODY,
            USERINFO_USER,
            USERINFO_PASSWORD,
            QUERY_KEY,
            "auth-canary-4d5e6f",
            "BEGIN PRIVATE KEY",
            "END PRIVATE KEY",
        ]
    }

    fn assert_no_credential_leaks(surface: &str, encoded: &str) {
        for canary in credential_canaries() {
            assert!(
                !encoded.contains(canary),
                "{surface} leaked {canary:?}: {encoded}"
            );
        }
        assert!(
            !encoded.contains(&full_upstream_url()),
            "{surface} leaked the full upstream URL: {encoded}"
        );
    }

    fn assert_no_public_leaks(surface: &str, encoded: &str) {
        assert_no_credential_leaks(surface, encoded);
        assert!(
            !encoded.contains("body-canary-7g8h9i"),
            "{surface} leaked request-body canary: {encoded}"
        );
        assert!(
            !encoded.contains(REQUEST_BODY),
            "{surface} leaked the request body: {encoded}"
        );
    }

    #[test]
    fn sanitize_text_matches_go_url_userinfo_contract() {
        let cases = [
            (
                "http://alice:secret@example.test:8080/v1",
                "http://[REDACTED]@example.test:8080/v1",
            ),
            (
                "https://alice:secret@example.test:8443/v1?token=query-secret",
                "https://[REDACTED]@example.test:8443/v1?[REDACTED]",
            ),
            (
                "http://alice@example.test/v1",
                "http://[REDACTED]@example.test/v1",
            ),
            (
                "https://alice%40example:secret%2Fvalue@example.test/v1",
                "https://[REDACTED]@example.test/v1",
            ),
            (
                "HtTpS://alice:secret@example.test:9443/v1?token=query-secret",
                "HtTpS://[REDACTED]@example.test:9443/v1?[REDACTED]",
            ),
            (
                "https://example.test/users/alice@example.test",
                "https://example.test/users/alice@example.test",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(sanitize_text(input), want, "input={input}");
        }
    }

    #[test]
    fn sanitize_text_redacts_api_keys_pem_and_auth_headers() {
        let input = format!(
            "ok line\napi_key={API_KEY}\nAuthorization: {BEARER}\nGET {}\n{}\n{REQUEST_BODY}\n",
            full_upstream_url(),
            pem_block()
        );
        let sanitized = sanitize_text(&input);
        assert_no_credential_leaks("sanitize_text", &sanitized);
        assert!(sanitized.contains(REDACTED_PEM));
        assert!(sanitized.contains(REDACTED) || sanitized.contains(REDACTED_KEY));
        assert!(sanitized.contains("ok line"));
        let public = sanitize_public_text(&input);
        assert_no_public_leaks("sanitize_public_text", &public);
        assert!(public.contains(REDACTED_BODY));
    }

    #[test]
    fn access_log_only_allows_method_path_status_bytes_latency() {
        let record = AccessLogRecord::new(
            "POST",
            &format!("/v1/chat?api_key={QUERY_KEY}"),
            502,
            12,
            Duration::from_millis(3),
        );
        assert_eq!(record.path, "/v1/chat");
        let encoded = serde_json::to_value(&record).unwrap();
        let object = encoded.as_object().expect("access log object");
        let mut keys: Vec<_> = object.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["bytes", "latency", "method", "path", "status"]);
        assert_eq!(
            serde_json::to_string(&record).unwrap(),
            r#"{"method":"POST","path":"/v1/chat","status":502,"bytes":12,"latency":3}"#
        );
        assert_eq!(object.get("method").unwrap(), "POST");
        assert_eq!(object.get("path").unwrap(), "/v1/chat");
        assert_eq!(object.get("status").unwrap(), 502);
        assert_eq!(object.get("bytes").unwrap(), 12);
        assert_eq!(object.get("latency").unwrap(), 3);
        assert_no_public_leaks("access_log", &encoded.to_string());
        assert!(serde_json::from_value::<AccessLogRecord>(json!({
            "method": "POST",
            "path": "/v1/chat",
            "status": 502,
            "bytes": 12,
            "latency": 3,
            "api_key": API_KEY,
            "request_body": REQUEST_BODY,
        }))
        .is_err());
    }

    #[test]
    fn protocol_error_and_proxy_json_omit_secrets_and_upstream_details() {
        let error = sanitize_protocol_error(
            ErrorCode::RouterStartFailed,
            &format!(
                "probe failed: {} pem={} body={REQUEST_BODY} authorization={BEARER}",
                full_upstream_url(),
                pem_block()
            ),
            Some(ErrorDetails {
                path: "/owner".into(),
                rule: "required".into(),
            }),
        );
        let response = Response {
            id: Some("request-1".into()),
            result: None,
            error: Some(error),
        };
        let encoded = serde_json::to_string(&response).unwrap();
        assert!(encoded.contains("ROUTER_START_FAILED"));
        assert!(encoded.contains("\"path\":\"/owner\""));
        assert_no_public_leaks("protocol_error", &encoded);

        let proxy_body = proxy_error_json(502);
        assert_eq!(proxy_body, json!({"error":"Bad Gateway"}));
        assert_no_public_leaks("proxy_error_json", &proxy_body.to_string());
        let object = proxy_body.as_object().expect("proxy error object");
        assert_eq!(object.keys().collect::<Vec<_>>(), vec!["error"]);
    }

    #[test]
    fn listening_origin_drops_userinfo_path_and_query() {
        let origin = public_upstream_origin(&full_upstream_url()).expect("origin");
        assert_eq!(origin, "https://upstream.example:8443");
        assert_no_public_leaks("upstream_origin", &origin);
        assert!(!origin.contains(UPSTREAM_PATH));
    }

    #[test]
    fn diagnostic_summary_and_ring_omit_sensitive_material() {
        let summary = sanitize_diagnostic_summary(&format!(
            "listen={}\napi_key={API_KEY}\n{}\n{REQUEST_BODY}\n",
            full_upstream_url(),
            pem_block()
        ));
        assert_no_public_leaks("diagnostic_summary", &summary);
        assert!(summary.contains("listen=https://"));
        assert!(summary.contains(&format!("?{REDACTED}")));

        let ring = ManagerDiagnosticRing::default();
        ring.ingest_stderr(
            format!(
                "raw path /tmp/private api_key={API_KEY} {REQUEST_BODY} {}\n",
                pem_block()
            )
            .as_bytes(),
        );
        assert!(ring.last().is_none());
        ring.ingest_stderr(
            br#"{"schema_version":1,"kind":"manager_bootstrap_failure","stage":"handshake","code":"MANAGER_BOOTSTRAP_FAILED"}
"#,
        );
        ring.record(STAGE_SPAWN, format!("not a code {API_KEY}"));
        let events = ring.snapshot();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].stage, STAGE_HANDSHAKE);
        assert_eq!(events[0].code, "MANAGER_BOOTSTRAP_FAILED");
        assert_eq!(events[1].stage, STAGE_SPAWN);
        assert_eq!(events[1].code, "MANAGER_FAILED");
        let encoded = serde_json::to_string(&events).unwrap();
        assert_no_public_leaks("manager_diagnostics", &encoded);
        assert_ne!(events[1].stage, STAGE_UNEXPECTED_EXIT);
    }

    #[test]
    fn panic_containment_omits_payload() {
        let payload = format!(
            "router task panicked: {} {} {REQUEST_BODY} {API_KEY}",
            full_upstream_url(),
            pem_block()
        );
        let failure = contain_panic(&payload);
        assert_eq!(failure.kind, ContainedFailureKind::RouterPanic);
        assert_eq!(failure.code, ErrorCode::RouterDegraded);
        let encoded = serde_json::to_string(&failure).unwrap();
        assert_eq!(
            encoded,
            r#"{"kind":"router_panic","code":"ROUTER_DEGRADED"}"#
        );
        assert_no_public_leaks("panic_containment", &encoded);
        assert!(!encoded.contains(&payload));
    }

    #[test]
    fn cross_component_redaction_matrix() {
        let payload = format!(
            "api_key={API_KEY}\nAuthorization: {BEARER}\n{REQUEST_BODY}\n{}\nGET {}\n",
            pem_block(),
            full_upstream_url()
        );
        let surfaces = [
            ("sanitize_text", sanitize_text(&payload)),
            (
                "access_log",
                serde_json::to_string(&AccessLogRecord::new(
                    "POST",
                    &format!("/v1/messages?api_key={QUERY_KEY}"),
                    200,
                    0,
                    Duration::from_millis(1),
                ))
                .unwrap(),
            ),
            (
                "protocol_error",
                serde_json::to_string(&sanitize_protocol_error(
                    ErrorCode::InvalidParams,
                    &payload,
                    None,
                ))
                .unwrap(),
            ),
            ("proxy_error", proxy_error_json(400).to_string()),
            (
                "upstream_origin",
                public_upstream_origin(&full_upstream_url()).unwrap(),
            ),
            ("diagnostics", sanitize_diagnostic_summary(&payload)),
            (
                "panic",
                serde_json::to_string(&contain_panic(&payload)).unwrap(),
            ),
        ];
        for (name, encoded) in &surfaces {
            if *name == "sanitize_text" {
                assert_no_credential_leaks(name, encoded);
            } else {
                assert_no_public_leaks(name, encoded);
            }
        }

        let access: Map<String, Value> = serde_json::from_str(&surfaces[1].1).unwrap();
        let mut keys: Vec<_> = access.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["bytes", "latency", "method", "path", "status"]);
    }
}
