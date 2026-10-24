use crate::{
    wire::{Identity, IssuedCertificate},
    IdentityError, Result, AUTH_PROTOCOL_VERSION,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use ring::{
    rand::{SecureRandom, SystemRandom},
    signature::{EcdsaKeyPair, ECDSA_P256_SHA256_ASN1_SIGNING},
};
use rustls::RootCertStore;
use rustls_pki_types::{pem::PemObject, CertificateDer, UnixTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, sync::Arc, time::Duration};
use x509_parser::{extensions::GeneralName, prelude::*};
use zeroize::Zeroizing;

pub fn digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
pub fn verifier() -> Result<Zeroizing<String>> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    SystemRandom::new()
        .fill(bytes.as_mut())
        .map_err(|_| IdentityError::Unavailable)?;
    Ok(Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes.as_ref())))
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceKey {
    private_key_pem: Zeroizing<String>,
    pub csr_pem: String,
    pub csr_digest: String,
    pub spki_digest: String,
}

impl fmt::Debug for DeviceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceKey").finish_non_exhaustive()
    }
}

impl DeviceKey {
    pub fn generate() -> Result<Self> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
            .map_err(|_| IdentityError::Unavailable)?;
        let params =
            CertificateParams::new(Vec::<String>::new()).map_err(|_| IdentityError::Unavailable)?;
        let csr = params
            .serialize_request(&key)
            .map_err(|_| IdentityError::Unavailable)?;
        Ok(Self {
            private_key_pem: Zeroizing::new(key.serialize_pem()),
            csr_pem: csr.pem().map_err(|_| IdentityError::Unavailable)?,
            csr_digest: digest(csr.der().as_ref()),
            spki_digest: digest(&key.public_key_der()),
        })
    }

    pub fn private_key_pem(&self) -> &str {
        &self.private_key_pem
    }

    pub fn validate(&self) -> Result<()> {
        if self.private_key_pem.len() > 8192 || self.csr_pem.len() > 8192 {
            return Err(IdentityError::InvalidState);
        }
        let key =
            KeyPair::from_pem(&self.private_key_pem).map_err(|_| IdentityError::InvalidState)?;
        let (tail, pem) = x509_parser::pem::parse_x509_pem(self.csr_pem.as_bytes())
            .map_err(|_| IdentityError::InvalidState)?;
        let (rest, csr) = X509CertificationRequest::from_der(&pem.contents)
            .map_err(|_| IdentityError::InvalidState)?;
        if !rest.is_empty()
            || !tail.iter().all(u8::is_ascii_whitespace)
            || !self
                .csr_pem
                .trim_start()
                .starts_with("-----BEGIN CERTIFICATE REQUEST-----")
            || csr.verify_signature().is_err()
            || key.algorithm() != &PKCS_ECDSA_P256_SHA256
            || pem.label != "CERTIFICATE REQUEST"
            || digest(&pem.contents) != self.csr_digest
            || digest(csr.certification_request_info.subject_pki.raw) != self.spki_digest
            || digest(&key.public_key_der()) != self.spki_digest
        {
            return Err(IdentityError::InvalidState);
        }
        Ok(())
    }

    pub fn sign(&self, message: &[u8]) -> Result<String> {
        let key =
            KeyPair::from_pem(&self.private_key_pem).map_err(|_| IdentityError::InvalidState)?;
        let der = Zeroizing::new(key.serialize_der());
        let rng = SystemRandom::new();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &der, &rng)
            .map_err(|_| IdentityError::InvalidState)?;
        let signature = pair
            .sign(&rng, message)
            .map_err(|_| IdentityError::Unavailable)?;
        Ok(URL_SAFE_NO_PAD.encode(signature.as_ref()))
    }
}

#[derive(Clone)]
pub struct IdentityTrust {
    root_pem: String,
}
impl fmt::Debug for IdentityTrust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdentityTrust").finish_non_exhaustive()
    }
}
impl IdentityTrust {
    pub fn new(root_pem: String) -> Result<Self> {
        let roots = certificates(&root_pem)?;
        if roots.len() != 1 {
            return Err(IdentityError::InvalidCertificate);
        }
        Ok(Self { root_pem })
    }

    pub fn validate(
        &self,
        value: &IssuedCertificate,
        key: &DeviceKey,
        expected: Option<&Identity>,
        now: i64,
    ) -> Result<()> {
        key.validate()?;
        if now < 0
            || value.certificate_pem.len() > 16384
            || value.chain_pem.len() > 32768
            || value.identity.version != AUTH_PROTOCOL_VERSION
            || !stable_id(&value.identity.user_id)
            || !stable_id(&value.identity.organization_id)
            || value.identity.policy_epoch < 1
            || value.not_before > now
            || value.not_after <= now
            || value.not_after - now > 30 * 24 * 3600
            || expected.is_some_and(|id| id != &value.identity)
        {
            return Err(IdentityError::InvalidCertificate);
        }
        let leafs = certificates(&value.certificate_pem)?;
        let chain = certificates(&value.chain_pem)?;
        let pinned = certificates(&self.root_pem)?;
        if leafs.len() != 1
            || chain.len() != 2
            || chain.last() != pinned.first()
            || digest(chain[0].as_ref()) != value.issuer_id
        {
            return Err(IdentityError::InvalidCertificate);
        }
        let (rest, cert) = X509Certificate::from_der(leafs[0].as_ref())
            .map_err(|_| IdentityError::InvalidCertificate)?;
        let usage = cert
            .extended_key_usage()
            .map_err(|_| IdentityError::InvalidCertificate)?
            .ok_or(IdentityError::InvalidCertificate)?;
        let key_usage = cert
            .key_usage()
            .map_err(|_| IdentityError::InvalidCertificate)?
            .ok_or(IdentityError::InvalidCertificate)?;
        let serial = cert
            .raw_serial()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if !rest.is_empty()
            || cert.is_ca()
            || !client_only(&usage.value)
            || key_usage.value.flags != 1
            || cert
                .basic_constraints()
                .map_err(|_| IdentityError::InvalidCertificate)?
                .is_none()
            || cert.validity().not_before.timestamp() != value.not_before
            || cert.validity().not_after.timestamp() != value.not_after
            || serial.trim_start_matches('0') != value.serial
            || digest(cert.public_key().raw) != value.spki_digest
            || value.spki_digest != key.spki_digest
        {
            return Err(IdentityError::InvalidCertificate);
        }
        let (_, issuer) = X509Certificate::from_der(chain[0].as_ref())
            .map_err(|_| IdentityError::InvalidCertificate)?;
        let (_, root) = X509Certificate::from_der(pinned[0].as_ref())
            .map_err(|_| IdentityError::InvalidCertificate)?;
        let constraint = issuer
            .basic_constraints()
            .map_err(|_| IdentityError::InvalidCertificate)?
            .ok_or(IdentityError::InvalidCertificate)?;
        let issuer_usage = issuer
            .extended_key_usage()
            .map_err(|_| IdentityError::InvalidCertificate)?
            .ok_or(IdentityError::InvalidCertificate)?;
        let issuer_keys = issuer
            .key_usage()
            .map_err(|_| IdentityError::InvalidCertificate)?
            .ok_or(IdentityError::InvalidCertificate)?;
        if !constraint.value.ca
            || constraint.value.path_len_constraint != Some(0)
            || !client_only(&issuer_usage.value)
            || issuer_keys.value.flags != 96
            || value.not_after > issuer.validity().not_after.timestamp()
            || value.not_after > root.validity().not_after.timestamp()
        {
            return Err(IdentityError::InvalidCertificate);
        }
        let san = cert
            .subject_alternative_name()
            .map_err(|_| IdentityError::InvalidCertificate)?
            .ok_or(IdentityError::InvalidCertificate)?;
        if san.value.general_names.len() != 1 {
            return Err(IdentityError::InvalidCertificate);
        }
        let GeneralName::URI(uri) = san.value.general_names[0] else {
            return Err(IdentityError::InvalidCertificate);
        };
        let claims = uri
            .strip_prefix("urn:codeasier:router:identity:v1:")
            .ok_or(IdentityError::InvalidCertificate)?;
        let claims = URL_SAFE_NO_PAD
            .decode(claims)
            .map_err(|_| IdentityError::InvalidCertificate)?;
        let identity: Identity =
            serde_json::from_slice(&claims).map_err(|_| IdentityError::InvalidCertificate)?;
        if identity != value.identity {
            return Err(IdentityError::InvalidCertificate);
        }
        let mut roots = RootCertStore::empty();
        roots
            .add(pinned[0].clone())
            .map_err(|_| IdentityError::InvalidCertificate)?;
        let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
            Arc::new(roots),
            Arc::new(rustls::crypto::ring::default_provider()),
        )
        .build()
        .map_err(|_| IdentityError::InvalidCertificate)?;
        verifier
            .verify_client_cert(
                &leafs[0],
                &chain[..1],
                UnixTime::since_unix_epoch(Duration::from_secs(now as u64)),
            )
            .map_err(|_| IdentityError::InvalidCertificate)?;
        Ok(())
    }
}

fn stable_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn client_only(value: &x509_parser::extensions::ExtendedKeyUsage<'_>) -> bool {
    value.client_auth
        && !value.any
        && !value.server_auth
        && !value.code_signing
        && !value.email_protection
        && !value.time_stamping
        && !value.ocsp_signing
        && value.other.is_empty()
}

fn certificates(value: &str) -> Result<Vec<CertificateDer<'static>>> {
    if value.len() > 32768 {
        return Err(IdentityError::InvalidCertificate);
    }
    CertificateDer::pem_slice_iter(value.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| IdentityError::InvalidCertificate)
}
