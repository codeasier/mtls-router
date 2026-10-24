use crate::redaction::public_upstream_origin;
use http::Uri;
use std::fmt;
use std::time::Duration;

use super::{StartupError, StartupReason, TlsMinVersion, DEFAULT_LISTEN_ADDR, DEFAULT_TLS_MIN};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct RouterDefaults {
    pub listen_addr: String,
    pub upstream_url: String,
    pub tls_min: String,
    pub timeout: Duration,
    pub debug: bool,
    pub backend: bool,
    pub log_path: String,
}

impl Default for RouterDefaults {
    fn default() -> Self {
        Self {
            listen_addr: DEFAULT_LISTEN_ADDR.to_owned(),
            upstream_url: String::new(),
            tls_min: DEFAULT_TLS_MIN.to_owned(),
            timeout: DEFAULT_TIMEOUT,
            debug: false,
            backend: false,
            log_path: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RouterEnv {
    pub listen_addr: Option<String>,
    pub upstream_url: Option<String>,
    pub tls_min: Option<String>,
    pub timeout: Option<String>,
    pub debug: Option<String>,
    pub backend: Option<String>,
    pub log_path: Option<String>,
}

impl RouterEnv {
    pub fn from_os() -> Self {
        Self {
            listen_addr: nonempty_var("MTLS_LISTEN_ADDR"),
            upstream_url: nonempty_var("MTLS_UPSTREAM_URL"),
            tls_min: nonempty_var("MTLS_TLS_MIN"),
            timeout: nonempty_var("MTLS_TIMEOUT"),
            debug: nonempty_var("MTLS_DEBUG"),
            backend: nonempty_var("MTLS_BACKEND"),
            log_path: nonempty_var("MTLS_LOG"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RouterFlags {
    pub listen_addr: Option<String>,
    pub upstream_url: Option<String>,
    pub tls_min: Option<String>,
    pub timeout: Option<String>,
    pub debug: Option<bool>,
    pub backend: Option<bool>,
    pub log_path: Option<String>,
}

#[derive(Clone)]
pub struct RouterConfig {
    listen_addr: String,
    upstream_url: String,
    tls_min: TlsMinVersion,
    tls_min_raw: String,
    timeout: Duration,
    debug: bool,
    backend: bool,
    log_path: String,
}

impl RouterConfig {
    pub fn listen_addr(&self) -> &str {
        &self.listen_addr
    }

    pub fn upstream_url(&self) -> &str {
        &self.upstream_url
    }

    pub fn tls_min(&self) -> TlsMinVersion {
        self.tls_min
    }

    pub fn tls_min_raw(&self) -> &str {
        &self.tls_min_raw
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn debug(&self) -> bool {
        self.debug
    }

    pub fn backend(&self) -> bool {
        self.backend
    }

    pub fn log_path(&self) -> &str {
        &self.log_path
    }

    pub fn probe_timeout(&self) -> Duration {
        if self.timeout.is_zero() {
            Duration::from_secs(5)
        } else {
            self.timeout
        }
    }
}

impl fmt::Debug for RouterConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RouterConfig")
            .field("listen_addr", &self.listen_addr)
            .field(
                "upstream_origin",
                &public_upstream_origin(&self.upstream_url),
            )
            .field("tls_min", &self.tls_min_raw)
            .field("timeout", &self.timeout)
            .field("debug", &self.debug)
            .field("backend", &self.backend)
            .field("log_path", &self.log_path)
            .finish()
    }
}

pub fn load_router_config(
    defaults: RouterDefaults,
    env: &RouterEnv,
    flags: &RouterFlags,
) -> Result<RouterConfig, StartupError> {
    let mut listen_addr = defaults.listen_addr;
    let mut upstream_url = defaults.upstream_url;
    let mut tls_min_raw = defaults.tls_min;
    let mut timeout = defaults.timeout;
    let mut debug = defaults.debug;
    let mut backend = defaults.backend;
    let mut log_path = defaults.log_path;

    if let Some(value) = nonempty(env.listen_addr.as_deref()) {
        listen_addr = value.to_owned();
    }
    if let Some(value) = nonempty(env.upstream_url.as_deref()) {
        upstream_url = value.to_owned();
    }
    if let Some(value) = nonempty(env.tls_min.as_deref()) {
        tls_min_raw = value.to_owned();
    }
    if let Some(value) = nonempty(env.timeout.as_deref()) {
        if let Some(parsed) = parse_go_duration(value) {
            timeout = parsed;
        }
    }
    if let Some(value) = nonempty(env.debug.as_deref()) {
        if let Some(parsed) = parse_go_bool(value) {
            debug = parsed;
        }
    }
    if let Some(value) = nonempty(env.backend.as_deref()) {
        if let Some(parsed) = parse_go_bool(value) {
            backend = parsed;
        }
    }
    if let Some(value) = nonempty(env.log_path.as_deref()) {
        log_path = value.to_owned();
    }

    if let Some(value) = flags.listen_addr.as_deref() {
        listen_addr = value.to_owned();
    }
    if let Some(value) = flags.upstream_url.as_deref() {
        upstream_url = value.to_owned();
    }
    if let Some(value) = flags.tls_min.as_deref() {
        tls_min_raw = value.to_owned();
    }
    if let Some(value) = flags.timeout.as_deref() {
        timeout = parse_go_duration(value)
            .ok_or_else(|| StartupError::new(StartupReason::ConfigInvalid))?;
    }
    if let Some(value) = flags.debug {
        debug = value;
    }
    if let Some(value) = flags.backend {
        backend = value;
    }
    if let Some(value) = flags.log_path.as_deref() {
        log_path = value.to_owned();
    }

    validate_upstream_url(&upstream_url)?;
    let tls_min = TlsMinVersion::parse(&tls_min_raw)?;
    Ok(RouterConfig {
        listen_addr,
        upstream_url,
        tls_min,
        tls_min_raw,
        timeout,
        debug,
        backend,
        log_path,
    })
}

pub(super) fn validate_upstream_url(raw: &str) -> Result<Uri, StartupError> {
    if raw.is_empty() {
        return Err(StartupError::new(StartupReason::ConfigInvalid));
    }
    let uri = raw
        .parse::<Uri>()
        .map_err(|_| StartupError::new(StartupReason::ConfigInvalid))?;
    if uri.scheme_str() != Some("https") {
        return Err(StartupError::new(StartupReason::ConfigInvalid));
    }
    match uri.host() {
        Some(host) if !host.is_empty() => Ok(uri),
        _ => Err(StartupError::new(StartupReason::ConfigInvalid)),
    }
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| if value.is_empty() { None } else { Some(value) })
}

fn nonempty_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .and_then(|value| if value.is_empty() { None } else { Some(value) })
}

fn parse_go_bool(value: &str) -> Option<bool> {
    match value {
        "1" | "t" | "T" | "TRUE" | "true" | "True" => Some(true),
        "0" | "f" | "F" | "FALSE" | "false" | "False" => Some(false),
        _ => None,
    }
}

fn parse_go_duration(input: &str) -> Option<Duration> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let (negative, rest) = if let Some(rest) = input.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = input.strip_prefix('+') {
        (false, rest)
    } else {
        (false, input)
    };
    if negative {
        return None;
    }
    if rest == "0" {
        return Some(Duration::ZERO);
    }
    let mut remaining = rest;
    let mut total_nanos: u128 = 0;
    let mut parsed_any = false;
    while !remaining.is_empty() {
        let (value, after_number) = parse_go_number(remaining)?;
        let (unit_nanos, after_unit) = parse_go_unit(after_number)?;
        let increment = (value * unit_nanos as f64).round() as u128;
        total_nanos = total_nanos.checked_add(increment)?;
        remaining = after_unit;
        parsed_any = true;
    }
    if !parsed_any {
        return None;
    }
    if total_nanos > u64::MAX as u128 {
        return None;
    }
    Some(Duration::from_nanos(total_nanos as u64))
}

fn parse_go_number(input: &str) -> Option<(f64, &str)> {
    let bytes = input.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
    }
    if index == 0 {
        return None;
    }
    let number = input[..index].parse().ok()?;
    Some((number, &input[index..]))
}

fn parse_go_unit(input: &str) -> Option<(u64, &str)> {
    const NS: u64 = 1;
    const US: u64 = 1_000;
    const MS: u64 = 1_000_000;
    const S: u64 = 1_000_000_000;
    const M: u64 = 60 * S;
    const H: u64 = 60 * M;
    if let Some(rest) = input.strip_prefix("ms") {
        return Some((MS, rest));
    }
    if let Some(rest) = input.strip_prefix("ns") {
        return Some((NS, rest));
    }
    if let Some(rest) = input.strip_prefix("us") {
        return Some((US, rest));
    }
    if let Some(rest) = input.strip_prefix("µs") {
        return Some((US, rest));
    }
    if let Some(rest) = input.strip_prefix("μs") {
        return Some((US, rest));
    }
    if let Some(rest) = input.strip_prefix('s') {
        return Some((S, rest));
    }
    if let Some(rest) = input.strip_prefix('m') {
        return Some((M, rest));
    }
    if let Some(rest) = input.strip_prefix('h') {
        return Some((H, rest));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn defaults() -> RouterDefaults {
        RouterDefaults {
            listen_addr: "127.0.0.1:1".into(),
            upstream_url: "https://default.example".into(),
            tls_min: "tls1.2".into(),
            timeout: Duration::from_secs(1),
            debug: false,
            backend: false,
            log_path: String::new(),
        }
    }

    #[test]
    fn load_precedence_matches_go_flag_over_env_over_default() {
        let env = RouterEnv {
            listen_addr: Some("127.0.0.1:2".into()),
            upstream_url: Some("https://env.example".into()),
            tls_min: Some("tls1.3".into()),
            timeout: Some("3s".into()),
            debug: Some("true".into()),
            backend: None,
            log_path: None,
        };
        let flags = RouterFlags {
            listen_addr: Some("127.0.0.1:3".into()),
            upstream_url: Some("https://flag.example".into()),
            tls_min: Some("tls1.2".into()),
            timeout: Some("4s".into()),
            debug: Some(false),
            backend: None,
            log_path: None,
        };
        let config = load_router_config(defaults(), &env, &flags).expect("valid config");
        assert_eq!(config.listen_addr(), "127.0.0.1:3");
        assert_eq!(config.upstream_url(), "https://flag.example");
        assert_eq!(config.tls_min_raw(), "tls1.2");
        assert_eq!(config.tls_min(), TlsMinVersion::Tls12);
        assert_eq!(config.timeout(), Duration::from_secs(4));
        assert!(!config.debug());
    }

    #[test]
    fn load_uses_env_when_flags_are_absent() {
        let env = RouterEnv {
            listen_addr: Some("127.0.0.1:2".into()),
            upstream_url: Some("https://env.example".into()),
            tls_min: Some("tls1.3".into()),
            timeout: Some("3s".into()),
            debug: Some("true".into()),
            backend: Some("true".into()),
            log_path: Some("/tmp/env-router.log".into()),
        };
        let config = load_router_config(defaults(), &env, &RouterFlags::default()).expect("valid");
        assert_eq!(config.listen_addr(), "127.0.0.1:2");
        assert_eq!(config.upstream_url(), "https://env.example");
        assert_eq!(config.tls_min(), TlsMinVersion::Tls13);
        assert_eq!(config.timeout(), Duration::from_secs(3));
        assert!(config.debug());
        assert!(config.backend());
        assert_eq!(config.log_path(), "/tmp/env-router.log");
    }

    #[test]
    fn load_ignores_invalid_env_timeout_and_bool_but_rejects_invalid_flag_timeout() {
        let env = RouterEnv {
            timeout: Some("nope".into()),
            debug: Some("maybe".into()),
            backend: Some("maybe".into()),
            ..RouterEnv::default()
        };
        let config = load_router_config(defaults(), &env, &RouterFlags::default()).expect("valid");
        assert_eq!(config.timeout(), Duration::from_secs(1));
        assert!(!config.debug());
        assert!(!config.backend());

        let flags = RouterFlags {
            timeout: Some("nope".into()),
            ..RouterFlags::default()
        };
        let error = load_router_config(defaults(), &RouterEnv::default(), &flags).unwrap_err();
        assert_eq!(error.reason(), StartupReason::ConfigInvalid);
        assert_eq!(error.to_string(), "config_invalid");
    }

    #[test]
    fn load_rejects_missing_http_and_invalid_upstream() {
        for upstream in ["", "://bad", "http://example.test", "https://"] {
            let mut values = defaults();
            values.upstream_url = upstream.into();
            let error = load_router_config(values, &RouterEnv::default(), &RouterFlags::default())
                .unwrap_err();
            assert_eq!(error.reason(), StartupReason::ConfigInvalid, "{upstream}");
            assert_eq!(error.to_string(), "config_invalid");
            if !upstream.is_empty() {
                assert!(!error.to_string().contains(upstream));
            }
        }
    }

    #[test]
    fn load_accepts_https_upstream_with_userinfo_path_and_query() {
        const UPSTREAM: &str = "https://user:pass@example.test/base?key=value#fragment";
        let mut values = defaults();
        values.upstream_url = UPSTREAM.into();
        let config = load_router_config(values, &RouterEnv::default(), &RouterFlags::default())
            .expect("valid");
        assert_eq!(config.upstream_url(), UPSTREAM);
        let debug = format!("{config:?}");
        assert!(!debug.contains("user:pass"));
        assert!(!debug.contains("key=value"));
        assert!(debug.contains("https://example.test"));
    }

    #[test]
    fn load_tls_min_validation_matches_go() {
        for version in ["", "tls1.2", "tls1.3"] {
            let mut values = defaults();
            values.tls_min = version.into();
            let config = load_router_config(values, &RouterEnv::default(), &RouterFlags::default())
                .expect(version);
            if version == "tls1.3" {
                assert_eq!(config.tls_min(), TlsMinVersion::Tls13);
            } else {
                assert_eq!(config.tls_min(), TlsMinVersion::Tls12);
            }
        }
        let mut values = defaults();
        values.tls_min = "tls1.1".into();
        let error =
            load_router_config(values, &RouterEnv::default(), &RouterFlags::default()).unwrap_err();
        assert_eq!(error.reason(), StartupReason::ConfigInvalid);
    }

    #[test]
    fn load_parses_backend_and_log_flags_over_env() {
        let env = RouterEnv {
            backend: Some("false".into()),
            log_path: Some("/tmp/env-router.log".into()),
            ..RouterEnv::default()
        };
        let flags = RouterFlags {
            backend: Some(true),
            log_path: Some("/tmp/flag-router.log".into()),
            ..RouterFlags::default()
        };
        let config = load_router_config(defaults(), &env, &flags).expect("valid");
        assert!(config.backend());
        assert_eq!(config.log_path(), "/tmp/flag-router.log");
    }

    #[test]
    fn parse_go_duration_covers_flag_and_compound_forms() {
        assert_eq!(parse_go_duration("3s"), Some(Duration::from_secs(3)));
        assert_eq!(parse_go_duration("4s"), Some(Duration::from_secs(4)));
        assert_eq!(parse_go_duration("10s"), Some(Duration::from_secs(10)));
        assert_eq!(parse_go_duration("1.5s"), Some(Duration::from_millis(1500)));
        assert_eq!(
            parse_go_duration("2h45m"),
            Some(Duration::from_secs(2 * 3600 + 45 * 60))
        );
        assert_eq!(parse_go_duration("0"), Some(Duration::ZERO));
        assert_eq!(parse_go_duration("nope"), None);
        assert_eq!(parse_go_duration("-1s"), None);
    }
}
