use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub version: String,
    pub organization_id: String,
    pub policy_epoch: i64,
    pub user_id: String,
    pub device_id: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IssuedCertificate {
    pub issuer_id: String,
    pub serial: String,
    pub certificate_pem: String,
    pub chain_pem: String,
    pub identity: Identity,
    pub not_before: i64,
    pub not_after: i64,
    pub spki_digest: String,
}

#[derive(Serialize)]
pub struct NativeStart<'a> {
    pub version: &'a str,
    pub csr_pem: &'a str,
    pub device_id: &'a str,
    pub device_kind: &'a str,
    pub device_label: &'a str,
    pub challenge: String,
    pub challenge_method: &'a str,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeStarted {
    pub version: String,
    pub transaction: String,
    pub verification_uri: String,
    pub short_code: String,
    pub expires_in: i64,
    pub interval: i64,
    pub spki_digest: String,
}

#[derive(Serialize)]
pub struct NativeProof<'a> {
    pub transaction: &'a str,
    pub verifier: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeResult {
    pub status: String,
    #[serde(default)]
    pub interval: i64,
    pub certificate: Option<IssuedCertificate>,
}

#[derive(Serialize)]
pub struct RenewalRequest<'a> {
    pub version: &'a str,
    pub issuer_id: &'a str,
    pub serial: &'a str,
    pub operation_id: &'a str,
    pub csr_pem: &'a str,
    pub timestamp: i64,
    pub signature: String,
}

#[derive(Serialize)]
pub struct DeviceRevokeRequest<'a> {
    pub version: &'a str,
    pub issuer_id: &'a str,
    pub serial: &'a str,
    pub timestamp: i64,
    pub signature: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceRevokeResult {
    pub status: String,
}
