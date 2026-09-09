#![allow(dead_code, unused_imports)]

//! Embedded router-core.
//!
//! This module locks the HTTP/TLS stack and implements configuration
//! precedence, mTLS transport, the startup probe, closed startup reason
//! codes, exact `/version` and `/health` handlers, a streaming reverse
//! proxy, and a dedicated supervisor runtime. Supervisor request timeout
//! and concurrency limits isolate the control plane; they are not the
//! frozen Go HTTP contract and do not apply to exact `/version`/`/health`.

mod config;
mod connect;
mod handlers;
mod probe;
mod proxy;
mod stack;
mod startup;
mod supervisor;
mod tls;

pub use config::{
    load_router_config, RouterConfig, RouterDefaults, RouterEnv, RouterFlags, DEFAULT_TIMEOUT,
};
pub use handlers::{dispatch_management, handle_health, handle_version, BuildInfo, InfoProvider};
pub use probe::Prober;
pub use proxy::{
    normalize_sse_headers, rewrite_upstream_uri, serve_request, ProxyErrorKind, ProxyLog,
    ProxyLogger, ReverseProxy,
};
pub use stack::{
    is_exact_management_route, local_http_request, parse_pem_certificates, parse_pem_private_key,
    LockedHttpTlsStack, LockedHttpTlsVersions, PemParseError, CRYPTO_BACKEND, DEFAULT_LISTEN_ADDR,
    DEFAULT_TLS_MIN, HEALTH_PATH, LOCKED_HTTP_TLS_VERSIONS, VERSION_PATH,
};
pub use startup::{prepare_startup, ListenRefusal, PreparedRouter, StartupError, StartupReason};
pub use supervisor::{
    RouterSupervisor, RuntimePhase, RuntimeStatus, SupervisorConfig, SupervisorError,
    SupervisorLimits, DEFAULT_MAX_CONCURRENT, DEFAULT_REQUEST_TIMEOUT,
};
pub use tls::{MtlsTransport, TlsMaterials, TlsMinVersion};

#[cfg(test)]
mod parity;
#[cfg(test)]
pub(crate) mod test_support;
