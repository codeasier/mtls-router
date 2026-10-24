//! Assembles the embedded desktop runtime from compile-time identity and the
//! router credentials embedded by `build.rs`.
//!
//! This is the only place that reads the embedded client certificate, key,
//! CA, and upstream URL; they go straight into the router-core supervisor and
//! never into protocol params, logs, or state files.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::error::{CommandError, Result};
use crate::installation::InstallationOwnership;
use crate::manager_core::{
    current_identity, load_embedded_preset, normalize_listener, CurrentLineage, EmbeddedBackend,
    EmbeddedConfig, Paths, ProductionRuntime, SessionConfig,
};
use crate::protocol::MANAGEMENT_PROTOCOL_VERSION;
use crate::router_core::{
    BuildInfo, RouterDefaults, RouterEnv, RouterFlags, SupervisorConfig, SupervisorLimits,
    TlsMaterials, DEFAULT_TIMEOUT,
};

const CLIENT_AUTH_CA_PEM: &str = include_str!(concat!(
    env!("MTLS_ROUTER_TRUST_DIR"),
    "/client-auth-ca.pem"
));
const UPSTREAM_CA_PEM: &str =
    include_str!(concat!(env!("MTLS_ROUTER_TRUST_DIR"), "/upstream-ca.pem"));

pub const UPSTREAM_URL: &str = env!("MTLS_UPSTREAM_URL");

/// Router start must outlive the upstream probe (Go `-timeout 10s`) plus
/// listener binding; stop must outlive graceful shutdown.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);

fn init_failed(message: &'static str) -> CommandError {
    CommandError::new("MANAGER_INIT_FAILED", message)
}

pub fn build_info() -> BuildInfo {
    BuildInfo::new(
        env!("MTLS_MANAGER_VERSION"),
        option_env!("MTLS_ROUTER_COMMIT").unwrap_or("unknown"),
        option_env!("MTLS_ROUTER_BUILD_DATE").unwrap_or("unknown"),
        env!("MTLS_DEPLOYMENT_ID"),
    )
    .with_personal_auth()
}

pub fn identity_store(data_dir: &Path) -> Result<router_identity::IdentityStore> {
    let trust = router_identity::IdentityTrust::new(CLIENT_AUTH_CA_PEM.into())
        .map_err(|e| CommandError::new(e.code(), "public certificate trust is unavailable"))?;
    router_identity::IdentityStore::for_runtime(data_dir, trust)
        .map_err(|e| CommandError::new(e.code(), "private certificate storage is unavailable"))
}

pub fn identity_client(data_dir: &Path) -> Result<router_identity::NativeClient> {
    router_identity::NativeClient::new(identity_store(data_dir)?, UPSTREAM_CA_PEM.into())
        .map_err(|e| CommandError::new(e.code(), "certificate service is unavailable"))
}

/// Same listen/TLS/probe configuration the desktop passed to the Go router:
/// loopback listen, TLS 1.2 floor, 10s probe timeout, debug off, foreground.
/// `request_timeout` is a separate supervisor isolation budget, not Go
/// `-timeout` / `MTLS_TIMEOUT`.
pub fn supervisor_config_with_listen(
    listen_addr: &str,
    data_dir: &Path,
) -> Result<SupervisorConfig> {
    Ok(SupervisorConfig {
        defaults: RouterDefaults {
            listen_addr: listen_addr.to_owned(),
            upstream_url: UPSTREAM_URL.to_owned(),
            tls_min: "tls1.2".to_owned(),
            timeout: DEFAULT_TIMEOUT,
            debug: false,
            backend: false,
            log_path: String::new(),
        },
        env: RouterEnv::default(),
        flags: RouterFlags::default(),
        materials: TlsMaterials::from_identity(identity_store(data_dir)?, UPSTREAM_CA_PEM),
        identity: build_info(),
        limits: SupervisorLimits {
            command_timeout: COMMAND_TIMEOUT,
            ..SupervisorLimits::default()
        },
        logger: None,
    })
}

pub fn production_runtime(
    session_id: String,
    ownership: &InstallationOwnership,
    desktop_data_dir: &Path,
) -> Result<Arc<ProductionRuntime>> {
    let paths = Paths::resolve_for_desktop_dir(desktop_data_dir)
        .map_err(|_| init_failed("desktop paths are unavailable"))?;
    let own =
        current_identity().map_err(|_| init_failed("desktop process identity is unavailable"))?;
    let configured = crate::listen_override::load_configured(desktop_data_dir)?;
    let listener = normalize_listener(&configured.listen_addr)
        .map_err(|_| init_failed("router listen address is invalid"))?;
    let package_generation = i32::try_from(ownership.package_generation)
        .map_err(|_| init_failed("installation generation is invalid"))?;
    let current = CurrentLineage {
        session_id: session_id.clone(),
        installation_id: ownership.installation_id.clone(),
        package_generation,
        deployment_id: env!("MTLS_DEPLOYMENT_ID").to_owned(),
        management_protocol_version: MANAGEMENT_PROTOCOL_VERSION.to_owned(),
        manager: own.clone(),
    };
    let preset = load_embedded_preset(env!("MTLS_AGENT_MODEL_PRESET_BASE64"))
        .map_err(|()| init_failed("embedded Agent model preset is invalid"))?;
    let backend = Arc::new(EmbeddedBackend::new(
        EmbeddedConfig {
            current: current.clone(),
            listener,
            desktop_state_path: paths.desktop_state_file.clone(),
            cli_state_path: paths.cli_state_file.clone(),
            log_base: paths.desktop_log_file.clone(),
            version: env!("MTLS_MANAGER_VERSION").to_owned(),
        },
        supervisor_config_with_listen(&configured.listen_addr, desktop_data_dir)?,
    ));
    Ok(Arc::new(ProductionRuntime {
        session: SessionConfig {
            listen_addr: configured.listen_addr.clone(),
            desktop_session: session_id,
            parent: own.clone(),
            manager_identity: own,
            paths,
            isolate_agent: false,
        },
        current,
        backend,
        preset,
        simplify: env!("MTLS_SIMPLIFY") == "true",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager_core::DEFAULT_LISTEN;
    use crate::router_core::{parse_pem_certificates, DEFAULT_REQUEST_TIMEOUT};

    #[test]
    fn embedded_credentials_parse_and_never_reach_protocol_surfaces() {
        assert!(!parse_pem_certificates(CLIENT_AUTH_CA_PEM.as_bytes())
            .unwrap()
            .is_empty());
        assert!(!parse_pem_certificates(UPSTREAM_CA_PEM.as_bytes())
            .unwrap()
            .is_empty());
        let directory =
            std::env::temp_dir().join(format!("router-public-trust-{}", uuid::Uuid::new_v4()));
        let config = supervisor_config_with_listen(DEFAULT_LISTEN, &directory).unwrap();
        assert!(config.materials.is_personal());
        let _ = std::fs::remove_dir_all(directory);
        assert_eq!(config.defaults.listen_addr, "127.0.0.1:19099");
        assert_eq!(config.defaults.tls_min, "tls1.2");
        assert_eq!(config.defaults.timeout, DEFAULT_TIMEOUT);
        assert_eq!(config.limits.request_timeout, DEFAULT_REQUEST_TIMEOUT);
        assert_ne!(config.limits.request_timeout, DEFAULT_TIMEOUT);
        assert!(!config.defaults.debug && !config.defaults.backend);
        assert!(config.limits.command_timeout > DEFAULT_TIMEOUT);
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("PRIVATE KEY"), "{rendered}");
        assert!(!rendered.contains(UPSTREAM_URL), "{rendered}");
        let info = crate::manager_core::info();
        assert_eq!(build_info().version(), info.version);
        assert_eq!(build_info().deployment_id(), info.deployment_id);
    }

    #[test]
    fn production_runtime_uses_current_process_as_manager_and_parent() {
        let ownership = InstallationOwnership {
            installation_id: "11111111-1111-4111-8111-111111111111".into(),
            ..InstallationOwnership::current()
        };
        let dir =
            std::env::temp_dir().join(format!("mtls-runtime-{}", uuid::Uuid::new_v4().simple()));
        let runtime = production_runtime("session-under-test".into(), &ownership, &dir).unwrap();
        let own = current_identity().unwrap();
        assert_eq!(
            runtime.session.paths.desktop_state_file,
            dir.join("desktop-state.json")
        );
        assert_eq!(
            runtime.backend.config().desktop_state_path,
            dir.join("desktop-state.json")
        );
        assert_eq!(
            runtime.backend.config().log_base,
            dir.join("mtls-router.log")
        );
        assert_eq!(runtime.current.manager, own);
        assert_eq!(runtime.session.parent, own);
        assert_eq!(runtime.session.manager_identity, own);
        assert_eq!(
            runtime.current.package_generation,
            crate::installation::PACKAGE_GENERATION as i32
        );
        assert_eq!(runtime.current.session_id, "session-under-test");
        assert!(!runtime.session.isolate_agent);
        assert_eq!(
            runtime.backend.config().listener.authority,
            "127.0.0.1:19099"
        );
        assert!(!runtime.backend.supervisor().status().bound);
    }

    #[test]
    fn production_runtime_uses_persisted_loopback_override_as_sole_listener() {
        let ownership = InstallationOwnership {
            installation_id: "11111111-1111-4111-8111-111111111111".into(),
            ..InstallationOwnership::current()
        };
        let dir =
            std::env::temp_dir().join(format!("mtls-runtime-{}", uuid::Uuid::new_v4().simple()));
        crate::listen_override::store_override(&dir, "127.0.0.1:19100").unwrap();
        let runtime = production_runtime("session-under-test".into(), &ownership, &dir).unwrap();
        assert_eq!(runtime.session.listen_addr, "127.0.0.1:19100");
        assert_eq!(
            runtime.backend.config().listener.authority,
            "127.0.0.1:19100"
        );
        assert!(!runtime.backend.supervisor().status().bound);
    }

    #[test]
    fn production_runtime_fails_closed_when_listen_override_is_corrupt() {
        let ownership = InstallationOwnership {
            installation_id: "11111111-1111-4111-8111-111111111111".into(),
            ..InstallationOwnership::current()
        };
        let dir =
            std::env::temp_dir().join(format!("mtls-runtime-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("listen-override.json"), b"{not-json").unwrap();
        match production_runtime("session-under-test".into(), &ownership, &dir) {
            Ok(_) => panic!("corrupt listen override must fail closed"),
            Err(error) => assert_eq!(error.code, "LISTEN_OVERRIDE_INVALID"),
        }
    }
}
