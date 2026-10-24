use rustls::{ClientConfig, RootCertStore};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_rustls::TlsConnector;
use zeroize::Zeroizing;

use super::{
    parse_pem_certificates, parse_pem_private_key, StartupError, StartupReason, DEFAULT_TLS_MIN,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TlsMinVersion {
    Tls12,
    Tls13,
}

impl TlsMinVersion {
    pub fn parse(value: &str) -> Result<Self, StartupError> {
        match value {
            "" | "tls1.2" => Ok(Self::Tls12),
            "tls1.3" => Ok(Self::Tls13),
            _ => Err(StartupError::new(StartupReason::ConfigInvalid)),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tls12 => "tls1.2",
            Self::Tls13 => "tls1.3",
        }
    }

    pub(crate) fn protocol_versions(self) -> &'static [&'static rustls::SupportedProtocolVersion] {
        match self {
            Self::Tls12 => TLS12_AND_13,
            Self::Tls13 => TLS13_ONLY,
        }
    }
}

const TLS12_AND_13: &[&rustls::SupportedProtocolVersion] =
    &[&rustls::version::TLS12, &rustls::version::TLS13];
const TLS13_ONLY: &[&rustls::SupportedProtocolVersion] = &[&rustls::version::TLS13];

impl Default for TlsMinVersion {
    fn default() -> Self {
        Self::parse(DEFAULT_TLS_MIN).expect("default TLS min is valid")
    }
}

pub struct TlsMaterials {
    client_cert_pem: String,
    client_key_pem: Zeroizing<String>,
    upstream_ca_pem: String,
    identity_store: Option<router_identity::IdentityStore>,
}

impl TlsMaterials {
    pub fn new(
        client_cert_pem: impl Into<String>,
        client_key_pem: impl Into<String>,
        upstream_ca_pem: impl Into<String>,
    ) -> Self {
        Self {
            client_cert_pem: client_cert_pem.into(),
            client_key_pem: Zeroizing::new(client_key_pem.into()),
            upstream_ca_pem: upstream_ca_pem.into(),
            identity_store: None,
        }
    }

    pub fn client_cert_pem(&self) -> &str {
        &self.client_cert_pem
    }

    pub fn from_identity(
        store: router_identity::IdentityStore,
        upstream_ca_pem: impl Into<String>,
    ) -> Self {
        Self {
            client_cert_pem: String::new(),
            client_key_pem: Zeroizing::new(String::new()),
            upstream_ca_pem: upstream_ca_pem.into(),
            identity_store: Some(store),
        }
    }

    pub fn is_personal(&self) -> bool {
        self.identity_store.is_some()
    }

    pub fn client_key_pem(&self) -> &str {
        &self.client_key_pem
    }

    pub fn upstream_ca_pem(&self) -> &str {
        &self.upstream_ca_pem
    }
}

impl fmt::Debug for TlsMaterials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsMaterials").finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct MtlsTransport {
    client_config: Arc<ClientConfig>,
    connector: TlsConnector,
    dynamic: Option<Arc<DynamicIdentity>>,
}

struct DynamicIdentity {
    store: router_identity::IdentityStore,
    server_roots: String,
    tls_min: TlsMinVersion,
    cache: Mutex<Option<(u64, String, MtlsTransport)>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CertificateState {
    Required,
    Expired,
    Rejected,
    Unavailable,
}
impl CertificateState {
    pub fn code(self) -> &'static str {
        match self {
            Self::Required => "CLIENT_CERT_REQUIRED",
            Self::Expired => "CLIENT_CERT_EXPIRED",
            Self::Rejected => "CLIENT_CERT_REJECTED",
            Self::Unavailable => "CERT_SERVICE_UNAVAILABLE",
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::Required => "client certificate required",
            Self::Expired => "client certificate expired",
            Self::Rejected => "client certificate rejected",
            Self::Unavailable => "certificate service unavailable",
        }
    }
}

impl MtlsTransport {
    pub fn new(materials: &TlsMaterials, tls_min: TlsMinVersion) -> Result<Self, StartupError> {
        if (materials.identity_store.is_none()
            && (materials.client_cert_pem.is_empty() || materials.client_key_pem.is_empty()))
            || materials.upstream_ca_pem.is_empty()
        {
            return Err(StartupError::new(StartupReason::TlsMaterialInvalid));
        }
        let mut roots = RootCertStore::empty();
        for certificate in parse_pem_certificates(materials.upstream_ca_pem.as_bytes())
            .map_err(|_| StartupError::new(StartupReason::TlsMaterialInvalid))?
        {
            roots
                .add(certificate)
                .map_err(|_| StartupError::new(StartupReason::TlsMaterialInvalid))?;
        }
        if roots.is_empty() {
            return Err(StartupError::new(StartupReason::TlsMaterialInvalid));
        }
        let builder =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(tls_min.protocol_versions())
                .map_err(|_| StartupError::new(StartupReason::TlsMaterialInvalid))?
                .with_root_certificates(roots);
        let dynamic = materials.identity_store.as_ref().map(|store| {
            Arc::new(DynamicIdentity {
                store: store.clone(),
                server_roots: materials.upstream_ca_pem.clone(),
                tls_min,
                cache: Mutex::new(None),
            })
        });
        let client_config = if dynamic.is_some() {
            builder.with_no_client_auth()
        } else {
            let certificates = parse_pem_certificates(materials.client_cert_pem.as_bytes())
                .map_err(|_| StartupError::new(StartupReason::TlsMaterialInvalid))?;
            let key = parse_pem_private_key(materials.client_key_pem.as_bytes())
                .map_err(|_| StartupError::new(StartupReason::TlsMaterialInvalid))?;
            builder
                .with_client_auth_cert(certificates, key)
                .map_err(|_| StartupError::new(StartupReason::TlsMaterialInvalid))?
        };
        let client_config = Arc::new(client_config);
        let connector = TlsConnector::from(client_config.clone());
        Ok(Self {
            client_config,
            connector,
            dynamic,
        })
    }

    pub fn client_config(&self) -> &ClientConfig {
        self.client_config.as_ref()
    }

    pub fn connector(&self) -> &TlsConnector {
        &self.connector
    }

    pub fn is_personal(&self) -> bool {
        self.dynamic.is_some()
    }

    // Only local private-state I/O happens here, before sending any upstream
    // request. Each request captures one immutable TLS generation. Existing
    // streams retain their prior Arc; a later request cannot reuse a disabled
    // identity even when another process changed the same directory.
    pub(super) async fn connector_for_request(
        &self,
    ) -> Result<(TlsConnector, Option<(u64, String)>), CertificateState> {
        let Some(dynamic) = &self.dynamic else {
            return Ok((self.connector.clone(), None));
        };
        let store = dynamic.store.clone();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| CertificateState::Unavailable)?
            .as_secs() as i64;
        let snapshot = tokio::task::spawn_blocking(move || store.snapshot(now))
            .await
            .map_err(|_| CertificateState::Unavailable)?;
        let (generation, identity) = snapshot
            .map_err(|error| match error {
                router_identity::IdentityError::ReauthRequired => CertificateState::Expired,
                router_identity::IdentityError::InvalidCertificate
                | router_identity::IdentityError::Rejected => CertificateState::Rejected,
                _ => CertificateState::Unavailable,
            })?
            .ok_or(CertificateState::Required)?;
        let mut cache = dynamic
            .cache
            .lock()
            .map_err(|_| CertificateState::Unavailable)?;
        if let Some((current, serial, transport)) = &*cache {
            if *current == generation && *serial == identity.certificate.serial {
                return Ok((
                    transport.connector.clone(),
                    Some((generation, identity.certificate.serial.clone())),
                ));
            }
        }
        let materials = TlsMaterials::new(
            format!(
                "{}{}",
                identity.certificate.certificate_pem, identity.certificate.chain_pem
            ),
            identity.private_key_pem(),
            &dynamic.server_roots,
        );
        let transport =
            Self::new(&materials, dynamic.tls_min).map_err(|_| CertificateState::Rejected)?;
        let connector = transport.connector.clone();
        let serial = identity.certificate.serial;
        *cache = Some((generation, serial.clone(), transport));
        Ok((connector, Some((generation, serial))))
    }

    pub(super) async fn feedback(&self, tag: Option<(u64, String)>, rejected: bool) {
        if let (Some(dynamic), Some((generation, serial))) = (&self.dynamic, tag) {
            let store = dynamic.store.clone();
            let _ = tokio::task::spawn_blocking(move || {
                store.certificate_feedback(generation, &serial, rejected)
            })
            .await;
        }
    }
}

impl fmt::Debug for MtlsTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MtlsTransport").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router_core::test_support::TestCerts;

    #[test]
    fn missing_or_invalid_materials_use_closed_tls_reason() {
        let certs = TestCerts::generate();
        for materials in [
            TlsMaterials::new("", certs.client_key.clone(), certs.ca.clone()),
            TlsMaterials::new(certs.client_cert.clone(), "", certs.ca.clone()),
            TlsMaterials::new(certs.client_cert.clone(), certs.client_key.clone(), ""),
            TlsMaterials::new("not-pem", certs.client_key.clone(), certs.ca.clone()),
        ] {
            let error = MtlsTransport::new(&materials, TlsMinVersion::Tls12).unwrap_err();
            assert_eq!(error.reason(), StartupReason::TlsMaterialInvalid);
            assert_eq!(error.to_string(), "tls_material_invalid");
            assert!(!error.to_string().contains("BEGIN"));
            assert!(!error.to_string().contains("PRIVATE"));
        }
    }

    #[test]
    fn materials_debug_omits_pem() {
        let certs = TestCerts::generate();
        let materials = certs.materials();
        let encoded = format!("{materials:?}");
        assert!(!encoded.contains("BEGIN"));
        assert!(!encoded.contains("PRIVATE"));
        assert!(!encoded.contains(&certs.client_cert));
        assert!(!encoded.contains(&certs.client_key));
        assert!(!encoded.contains(&certs.ca));
    }

    #[test]
    fn transport_accepts_tls12_and_tls13_minimums() {
        let materials = TestCerts::generate().materials();
        MtlsTransport::new(&materials, TlsMinVersion::Tls12).expect("tls1.2");
        MtlsTransport::new(&materials, TlsMinVersion::Tls13).expect("tls1.3");
    }
}
