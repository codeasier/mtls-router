use rustls::{ClientConfig, RootCertStore};
use std::fmt;
use std::sync::Arc;
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
        }
    }

    pub fn client_cert_pem(&self) -> &str {
        &self.client_cert_pem
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
}

impl MtlsTransport {
    pub fn new(materials: &TlsMaterials, tls_min: TlsMinVersion) -> Result<Self, StartupError> {
        if materials.client_cert_pem.is_empty()
            || materials.client_key_pem.is_empty()
            || materials.upstream_ca_pem.is_empty()
        {
            return Err(StartupError::new(StartupReason::TlsMaterialInvalid));
        }
        let certificates = parse_pem_certificates(materials.client_cert_pem.as_bytes())
            .map_err(|_| StartupError::new(StartupReason::TlsMaterialInvalid))?;
        let key = parse_pem_private_key(materials.client_key_pem.as_bytes())
            .map_err(|_| StartupError::new(StartupReason::TlsMaterialInvalid))?;
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
        let client_config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(tls_min.protocol_versions())
                .map_err(|_| StartupError::new(StartupReason::TlsMaterialInvalid))?
                .with_root_certificates(roots)
                .with_client_auth_cert(certificates, key)
                .map_err(|_| StartupError::new(StartupReason::TlsMaterialInvalid))?;
        let client_config = Arc::new(client_config);
        let connector = TlsConnector::from(client_config.clone());
        Ok(Self {
            client_config,
            connector,
        })
    }

    pub fn client_config(&self) -> &ClientConfig {
        self.client_config.as_ref()
    }

    pub fn connector(&self) -> &TlsConnector {
        &self.connector
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
