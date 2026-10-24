//! Synthetic in-memory authority shared by native-client and transport tests.
use crate::{
    crypto::digest,
    wire::{Identity, IssuedCertificate},
    DeviceKey, IdentityTrust, AUTH_PROTOCOL_VERSION,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose, SanType, SerialNumber,
};
use time::OffsetDateTime;

pub struct TestAuthority {
    pub root: Certificate,
    pub issuer: Certificate,
    issuer_key: KeyPair,
    pub trust: IdentityTrust,
    pub server_certificate: String,
    pub server_key: String,
}
impl Default for TestAuthority {
    fn default() -> Self {
        Self::new()
    }
}
impl TestAuthority {
    pub fn issue_pending(
        &self,
        pending: &crate::PendingIdentity,
        now: i64,
        serial: u64,
    ) -> IssuedCertificate {
        self.issue_at(&pending.key, &pending.device_id, now, serial)
    }
    pub fn new() -> Self {
        let root_key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params
            .distinguished_name
            .push(DnType::CommonName, "synthetic-offline-root");
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(1));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let root = params.self_signed(&root_key).unwrap();
        let issuer_key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params
            .distinguished_name
            .push(DnType::CommonName, "synthetic-client-issuer");
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let issuer = params.signed_by(&issuer_key, &root, &root_key).unwrap();
        let server_key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec![
            "127.0.0.1".into(),
            "relay.codeasier.top".into(),
            "enroll.codeasier.top".into(),
        ])
        .unwrap();
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let server = params.signed_by(&server_key, &root, &root_key).unwrap();
        let trust = IdentityTrust::new(root.pem()).unwrap();
        Self {
            root,
            issuer,
            issuer_key,
            trust,
            server_certificate: server.pem(),
            server_key: server_key.serialize_pem(),
        }
    }
    pub fn issue(&self, key: &DeviceKey, device: &str) -> IssuedCertificate {
        self.issue_at(key, device, 1_792_670_400, 42)
    }
    pub fn issue_at(
        &self,
        key: &DeviceKey,
        device: &str,
        now: i64,
        serial: u64,
    ) -> IssuedCertificate {
        self.issue_for(key, device, "user-fixture-42", now, serial)
    }
    pub fn issue_for(
        &self,
        key: &DeviceKey,
        device: &str,
        user: &str,
        now: i64,
        serial: u64,
    ) -> IssuedCertificate {
        let key_pair = KeyPair::from_pem(key.private_key_pem()).unwrap();
        let identity = Identity {
            version: AUTH_PROTOCOL_VERSION.into(),
            organization_id: "org-fixture-7".into(),
            policy_epoch: 3,
            user_id: user.into(),
            device_id: device.into(),
        };
        let uri = format!(
            "urn:codeasier:router:identity:v1:{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&identity).unwrap())
        );
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::ExplicitNoCa;
        params.serial_number = Some(SerialNumber::from(serial));
        params.not_before = OffsetDateTime::from_unix_timestamp(now - 300).unwrap();
        params.not_after = OffsetDateTime::from_unix_timestamp(now + 30 * 86400).unwrap();
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        params.subject_alt_names = vec![SanType::URI(uri.try_into().unwrap())];
        let leaf = params
            .signed_by(&key_pair, &self.issuer, &self.issuer_key)
            .unwrap();
        IssuedCertificate {
            issuer_id: digest(self.issuer.der().as_ref()),
            serial: format!("{serial:x}"),
            certificate_pem: leaf.pem(),
            chain_pem: format!("{}{}", self.issuer.pem(), self.root.pem()),
            identity,
            not_before: now - 300,
            not_after: now + 30 * 86400,
            spki_digest: key.spki_digest.clone(),
        }
    }
}
