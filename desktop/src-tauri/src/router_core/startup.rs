use crate::redaction::public_upstream_origin;
use std::fmt;
use std::io;

use super::{
    load_router_config, MtlsTransport, Prober, RouterConfig, RouterDefaults, RouterEnv,
    RouterFlags, TlsMaterials,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupReason {
    ArgumentsInvalid,
    ConfigInvalid,
    BackendStartFailed,
    LogOpenFailed,
    TlsMaterialInvalid,
    ProbeSetupFailed,
    UpstreamProbeFailed,
    ListenFailed,
    ShutdownFailed,
    RouterFailure,
}

impl StartupReason {
    pub const ALL: [Self; 10] = [
        Self::ArgumentsInvalid,
        Self::ConfigInvalid,
        Self::BackendStartFailed,
        Self::LogOpenFailed,
        Self::TlsMaterialInvalid,
        Self::ProbeSetupFailed,
        Self::UpstreamProbeFailed,
        Self::ListenFailed,
        Self::ShutdownFailed,
        Self::RouterFailure,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ArgumentsInvalid => "arguments_invalid",
            Self::ConfigInvalid => "config_invalid",
            Self::BackendStartFailed => "backend_start_failed",
            Self::LogOpenFailed => "log_open_failed",
            Self::TlsMaterialInvalid => "tls_material_invalid",
            Self::ProbeSetupFailed => "probe_setup_failed",
            Self::UpstreamProbeFailed => "upstream_probe_failed",
            Self::ListenFailed => "listen_failed",
            Self::ShutdownFailed => "shutdown_failed",
            Self::RouterFailure => "router_failure",
        }
    }
}

impl fmt::Display for StartupReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Closed listen-refusal class. Keeps `listen_failed` as the reason while
/// distinguishing a reserved/denied bind from address-in-use without
/// exposing paths or transport text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListenRefusal {
    AccessDenied,
    AddressInUse,
    Failed,
}

impl ListenRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AccessDenied => "access_denied",
            Self::AddressInUse => "address_in_use",
            Self::Failed => "failed",
        }
    }

    pub fn from_os_error(code: Option<u64>) -> Self {
        match code {
            // WSAEACCES / EACCES: typical Hyper-V/WinNAT excluded range.
            Some(10013 | 13) => Self::AccessDenied,
            // WSAEADDRINUSE / Linux EADDRINUSE / macOS EADDRINUSE.
            Some(10048 | 98 | 48) => Self::AddressInUse,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug)]
pub struct StartupError {
    reason: StartupReason,
    os_error: Option<u64>,
}

impl StartupError {
    pub fn new(reason: StartupReason) -> Self {
        Self {
            reason,
            os_error: None,
        }
    }

    pub fn from_io(reason: StartupReason, error: &io::Error) -> Self {
        Self {
            reason,
            os_error: error
                .raw_os_error()
                .and_then(|code| u64::try_from(code).ok())
                .filter(|code| *code != 0),
        }
    }

    pub fn with_os_error(mut self, code: u64) -> Self {
        if code != 0 {
            self.os_error = Some(code);
        }
        self
    }

    pub fn reason(&self) -> StartupReason {
        self.reason
    }

    pub fn os_error(&self) -> Option<u64> {
        self.os_error
    }

    pub fn listen_refusal(&self) -> Option<ListenRefusal> {
        (self.reason == StartupReason::ListenFailed)
            .then(|| ListenRefusal::from_os_error(self.os_error))
    }
}

impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason.as_str())
    }
}

impl std::error::Error for StartupError {}

pub struct PreparedRouter {
    config: RouterConfig,
    transport: MtlsTransport,
}

impl fmt::Debug for PreparedRouter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedRouter")
            .field("listen_addr", &self.config.listen_addr())
            .field("upstream_origin", &self.public_upstream_origin())
            .field("bound", &self.is_bound())
            .finish()
    }
}

impl PreparedRouter {
    pub fn config(&self) -> &RouterConfig {
        &self.config
    }

    pub fn transport(&self) -> &MtlsTransport {
        &self.transport
    }

    pub fn is_bound(&self) -> bool {
        false
    }

    pub fn public_upstream_origin(&self) -> Option<String> {
        public_upstream_origin(self.config.upstream_url())
    }
}

pub async fn prepare_startup(
    defaults: RouterDefaults,
    env: &RouterEnv,
    flags: &RouterFlags,
    materials: &TlsMaterials,
) -> Result<PreparedRouter, StartupError> {
    let config = load_router_config(defaults, env, flags)?;
    if config.backend() {
        return Err(StartupError::new(StartupReason::BackendStartFailed));
    }
    let transport = MtlsTransport::new(materials, config.tls_min())?;
    let prober = Prober::new(&config, &transport)?;
    prober.probe().await?;
    Ok(PreparedRouter { config, transport })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_reasons_are_the_closed_go_set() {
        let encoded: Vec<_> = StartupReason::ALL
            .into_iter()
            .map(StartupReason::as_str)
            .collect();
        assert_eq!(
            encoded,
            [
                "arguments_invalid",
                "config_invalid",
                "backend_start_failed",
                "log_open_failed",
                "tls_material_invalid",
                "probe_setup_failed",
                "upstream_probe_failed",
                "listen_failed",
                "shutdown_failed",
                "router_failure",
            ]
        );
        for reason in StartupReason::ALL {
            let error = StartupError::new(reason);
            assert_eq!(error.to_string(), reason.as_str());
            assert!(!error.to_string().contains("https://"));
            assert!(!error.to_string().contains("BEGIN"));
            assert!(!error.to_string().contains("api_key"));
        }
    }

    #[tokio::test]
    async fn prepare_startup_probes_before_binding_and_keeps_closed_reasons() {
        use crate::router_core::test_support::{start_mtls_server, MtlsServerOptions};
        use crate::router_core::{RouterDefaults, RouterEnv, RouterFlags};
        use tokio::net::TcpStream;

        let reserved = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("free listen addr");
        let listen_addr = reserved.local_addr().expect("listen addr").to_string();
        drop(reserved);

        let server = start_mtls_server(MtlsServerOptions::default()).await;
        let mut defaults = RouterDefaults::default();
        defaults.listen_addr = listen_addr.clone();
        defaults.upstream_url = server.url.clone();
        let prepared = prepare_startup(
            defaults.clone(),
            &RouterEnv::default(),
            &RouterFlags::default(),
            &server.certs.materials(),
        )
        .await
        .expect("prepared");
        assert!(!prepared.is_bound());
        assert!(TcpStream::connect(&listen_addr).await.is_err());
        assert_eq!(
            prepared.public_upstream_origin().as_deref(),
            Some(server.url.as_str())
        );

        let mut backend_flags = RouterFlags::default();
        backend_flags.backend = Some(true);
        let error = prepare_startup(
            defaults.clone(),
            &RouterEnv::default(),
            &backend_flags,
            &server.certs.materials(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.reason(), StartupReason::BackendStartFailed);
        assert_eq!(error.to_string(), "backend_start_failed");

        let mut failed = start_mtls_server(MtlsServerOptions {
            status: 503,
            ..MtlsServerOptions::default()
        })
        .await;
        defaults.upstream_url = format!("{}/private-path?api_key=sk-startup-canary", failed.url);
        let error = prepare_startup(
            defaults,
            &RouterEnv::default(),
            &RouterFlags::default(),
            &failed.certs.materials(),
        )
        .await
        .unwrap_err();
        failed.shutdown();
        assert_eq!(error.reason(), StartupReason::UpstreamProbeFailed);
        assert_eq!(error.to_string(), "upstream_probe_failed");
        assert!(!error.to_string().contains("sk-startup-canary"));
        assert!(!error.to_string().contains("private-path"));
        assert!(TcpStream::connect(&listen_addr).await.is_err());
    }

    #[test]
    fn listen_refusal_keeps_closed_reason_and_classifies_os_errors() {
        let reserved = StartupError::new(StartupReason::ListenFailed).with_os_error(10013);
        assert_eq!(reserved.to_string(), "listen_failed");
        assert_eq!(reserved.os_error(), Some(10013));
        assert_eq!(reserved.listen_refusal(), Some(ListenRefusal::AccessDenied));
        assert_eq!(
            StartupError::new(StartupReason::ListenFailed)
                .with_os_error(13)
                .listen_refusal(),
            Some(ListenRefusal::AccessDenied)
        );
        assert_eq!(
            StartupError::new(StartupReason::ListenFailed)
                .with_os_error(10048)
                .listen_refusal(),
            Some(ListenRefusal::AddressInUse)
        );
        assert_eq!(
            StartupError::new(StartupReason::ListenFailed)
                .with_os_error(98)
                .listen_refusal(),
            Some(ListenRefusal::AddressInUse)
        );
        assert_eq!(
            StartupError::new(StartupReason::ListenFailed)
                .with_os_error(48)
                .listen_refusal(),
            Some(ListenRefusal::AddressInUse)
        );
        assert_eq!(
            StartupError::new(StartupReason::ListenFailed).listen_refusal(),
            Some(ListenRefusal::Failed)
        );
        assert_eq!(
            StartupError::from_io(
                StartupReason::ListenFailed,
                &io::Error::from_raw_os_error(10013),
            )
            .os_error(),
            Some(10013)
        );
        assert_eq!(
            StartupError::new(StartupReason::UpstreamProbeFailed)
                .with_os_error(10013)
                .listen_refusal(),
            None
        );
        assert!(!reserved.to_string().contains("10013"));
        assert!(!reserved.to_string().contains("https://"));
    }
}
