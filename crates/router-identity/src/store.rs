use crate::{
    crypto::{self, DeviceKey, IdentityTrust},
    platform,
    wire::{IssuedCertificate, NativeStart, NativeStarted},
    IdentityError, Result, AUTH_PROTOCOL_VERSION,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    path::{Path, PathBuf},
};
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PendingIdentity {
    pub(crate) key: DeviceKey,
    pub(crate) verifier: Zeroizing<String>,
    pub device_id: String,
    pub device_kind: String,
    pub device_label: String,
    pub created_at: i64,
    pub started: Option<NativeStarted>,
}
impl PendingIdentity {
    pub fn request(&self) -> NativeStart<'_> {
        NativeStart {
            version: AUTH_PROTOCOL_VERSION,
            csr_pem: &self.key.csr_pem,
            device_id: &self.device_id,
            device_kind: &self.device_kind,
            device_label: &self.device_label,
            challenge: crypto::challenge(&self.verifier),
            challenge_method: "S256",
        }
    }

    fn validate(&self) -> Result<()> {
        self.key.validate()?;
        let id = Uuid::parse_str(&self.device_id).map_err(|_| IdentityError::InvalidState)?;
        let verifier = URL_SAFE_NO_PAD
            .decode(self.verifier.as_bytes())
            .map_err(|_| IdentityError::InvalidState)?;
        if id.get_version_num() != 4
            || !matches!(self.device_kind.as_str(), "desktop" | "linux_cli")
            || verifier.len() != 32
            || self.verifier.len() != 43
            || self.device_label.is_empty()
            || self.device_label.len() > 128
            || self.device_label.chars().any(char::is_control)
        {
            return Err(IdentityError::InvalidState);
        }
        if let Some(started) = &self.started {
            self.validate_started(started)?;
        }
        Ok(())
    }

    fn validate_started(&self, started: &NativeStarted) -> Result<()> {
        let transaction = URL_SAFE_NO_PAD
            .decode(&started.transaction)
            .map_err(|_| IdentityError::InvalidCertificate)?;
        let short = started.short_code.as_bytes();
        if transaction.len() != 32
            || started.transaction.len() != 43
            || short.len() != 9
            || short[4] != b'-'
            || short
                .iter()
                .enumerate()
                .any(|(i, c)| i != 4 && !c.is_ascii_uppercase() && !(b'2'..=b'7').contains(c))
            || started.version != AUTH_PROTOCOL_VERSION
            || started.spki_digest != self.key.spki_digest
            || started.expires_in != 600
            || started.interval != 5
            || started.verification_uri
                != format!(
                    "{}/enroll/native/verify?transaction={}",
                    crate::ENROLL_ORIGIN,
                    started.transaction
                )
        {
            return Err(IdentityError::InvalidCertificate);
        }
        Ok(())
    }
}
impl fmt::Debug for PendingIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingIdentity").finish_non_exhaustive()
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveIdentity {
    pub(crate) key: DeviceKey,
    pub certificate: IssuedCertificate,
}
impl ActiveIdentity {
    pub fn private_key_pem(&self) -> &str {
        self.key.private_key_pem()
    }
}
impl fmt::Debug for ActiveIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActiveIdentity").finish_non_exhaustive()
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingRenewal {
    pub operation_id: String,
    pub key: DeviceKey,
    pub created_at: i64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RevokeTodo {
    pub active: ActiveIdentity,
    pub renewal: Option<PendingRenewal>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    schema_version: u32,
    pub generation: u64,
    pub(crate) active: Option<ActiveIdentity>,
    pub(crate) pending: Option<PendingIdentity>,
    pub(crate) renewal: Option<PendingRenewal>,
    pub(crate) revocations: Vec<RevokeTodo>,
    pub(crate) cancellations: Vec<PendingIdentity>,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            schema_version: 1,
            generation: 0,
            active: None,
            pending: None,
            renewal: None,
            revocations: Vec::new(),
            cancellations: Vec::new(),
            last_error: None,
        }
    }
}
impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdentityState")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct IdentityStore {
    root: PathBuf,
    trust: IdentityTrust,
}
impl fmt::Debug for IdentityStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdentityStore").finish_non_exhaustive()
    }
}
impl IdentityStore {
    pub fn for_runtime(data_dir: &Path, trust: IdentityTrust) -> Result<Self> {
        let root = data_dir.join("identity");
        platform::directory(&root)?;
        Ok(Self { root, trust })
    }
    pub fn open(data_dir: &Path, trust: IdentityTrust) -> Result<Self> {
        let root = data_dir.join("identity");
        platform::directory(&root)?;
        let store = Self { root, trust };
        let _ = store.read()?;
        Ok(store)
    }
    fn load(&self) -> Result<State> {
        let state = match platform::read(&self.root.join("state.json"))? {
            Some(data) => {
                serde_json::from_slice::<State>(&data).map_err(|_| IdentityError::InvalidState)?
            }
            None => State::default(),
        };
        self.validate(&state)?;
        Ok(state)
    }
    fn validate(&self, state: &State) -> Result<()> {
        if state.schema_version != 1
            || state.revocations.len() > 32
            || state.cancellations.len() > 32
        {
            return Err(IdentityError::InvalidState);
        }
        if state.last_error.as_deref().is_some_and(|code| {
            !matches!(
                code,
                "IDENTITY_STORAGE_UNAVAILABLE"
                    | "IDENTITY_STATE_INVALID"
                    | "IDENTITY_BUSY"
                    | "CERT_OPERATION_CONFLICT"
                    | "CLIENT_CERT_REJECTED"
                    | "CERT_SERVICE_UNAVAILABLE"
                    | "IDENTITY_NETWORK_UNAVAILABLE"
                    | "CERT_REAUTH_REQUIRED"
                    | "ORG_MEMBERSHIP_REQUIRED"
            )
        }) {
            return Err(IdentityError::InvalidState);
        }
        let validate_active = |active: &ActiveIdentity| {
            let at = active
                .certificate
                .not_before
                .max(active.certificate.not_after.saturating_sub(1));
            self.trust
                .validate(&active.certificate, &active.key, None, at)
        };
        if let Some(v) = &state.active {
            validate_active(v)?;
        }
        if let Some(v) = &state.pending {
            v.validate()?;
        }
        if let Some(v) = &state.renewal {
            v.key.validate()?;
            if state.active.is_none()
                || Uuid::parse_str(&v.operation_id)
                    .map(|v| v.get_version_num() != 4)
                    .unwrap_or(true)
                || state
                    .active
                    .as_ref()
                    .is_some_and(|a| a.key.spki_digest == v.key.spki_digest)
            {
                return Err(IdentityError::InvalidState);
            }
        }
        for v in &state.revocations {
            validate_active(&v.active)?;
            if let Some(p) = &v.renewal {
                p.key.validate()?;
            }
        }
        for v in &state.cancellations {
            v.validate()?;
        }
        Ok(())
    }
    pub fn read(&self) -> Result<State> {
        let _lock = platform::Lock::acquire(&self.root.join("identity.lock"))?;
        self.load()
    }
    pub(crate) fn change<T>(&self, operation: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        let _lock = platform::Lock::acquire(&self.root.join("identity.lock"))?;
        let mut state = self.load()?;
        let original =
            Zeroizing::new(serde_json::to_vec(&state).map_err(|_| IdentityError::InvalidState)?);
        let result = operation(&mut state)?;
        self.validate(&state)?;
        let data =
            Zeroizing::new(serde_json::to_vec(&state).map_err(|_| IdentityError::InvalidState)?);
        if data.len() > 512 * 1024 {
            return Err(IdentityError::InvalidState);
        }
        if *data == *original {
            return Ok(result);
        }
        platform::atomic_write(&self.root.join("state.json"), &data)?;
        Ok(result)
    }
    pub fn begin_login(&self, kind: &str, label: &str, now: i64) -> Result<(u64, PendingIdentity)> {
        if !matches!(kind, "desktop" | "linux_cli")
            || label.is_empty()
            || label.len() > 128
            || label.chars().any(char::is_control)
        {
            return Err(IdentityError::InvalidState);
        }
        let pending = PendingIdentity {
            key: DeviceKey::generate()?,
            verifier: crypto::verifier()?,
            device_id: Uuid::new_v4().to_string(),
            device_kind: kind.into(),
            device_label: label.into(),
            created_at: now,
            started: None,
        };
        self.change(|state| {
            retire(state)?;
            state.generation = state
                .generation
                .checked_add(1)
                .ok_or(IdentityError::InvalidState)?;
            state.pending = Some(pending.clone());
            state.last_error = None;
            Ok((state.generation, pending))
        })
    }
    pub fn record_started(&self, generation: u64, started: NativeStarted) -> Result<()> {
        self.change(|state| {
            if state.generation != generation {
                return Err(IdentityError::Conflict);
            }
            let pending = state.pending.as_mut().ok_or(IdentityError::Conflict)?;
            pending.validate_started(&started)?;
            if pending
                .started
                .as_ref()
                .is_some_and(|previous| previous.transaction != started.transaction)
            {
                return Err(IdentityError::Conflict);
            }
            pending.started = Some(started);
            Ok(())
        })
    }
    pub fn activate_login(
        &self,
        generation: u64,
        certificate: IssuedCertificate,
        now: i64,
    ) -> Result<()> {
        self.change(|state| {
            if state.generation != generation {
                return Err(IdentityError::Conflict);
            }
            let pending = state.pending.as_ref().ok_or(IdentityError::Conflict)?;
            if certificate.identity.device_id != pending.device_id {
                return Err(IdentityError::InvalidCertificate);
            }
            self.trust.validate(&certificate, &pending.key, None, now)?;
            state.active = Some(ActiveIdentity {
                key: pending.key.clone(),
                certificate,
            });
            state.pending = None;
            state.last_error = None;
            Ok(())
        })
    }
    pub fn snapshot(&self, now: i64) -> Result<Option<(u64, ActiveIdentity)>> {
        let state = self.read()?;
        let Some(active) = state.active else {
            return Ok(None);
        };
        if active.certificate.not_after <= now {
            return Err(IdentityError::ReauthRequired);
        }
        self.trust
            .validate(&active.certificate, &active.key, None, now)?;
        Ok(Some((state.generation, active)))
    }

    pub fn certificate_feedback(
        &self,
        generation: u64,
        serial: &str,
        rejected: bool,
    ) -> Result<()> {
        self.change(|state| {
            if state.generation != generation
                || !state
                    .active
                    .as_ref()
                    .is_some_and(|v| v.certificate.serial == serial)
            {
                return Err(IdentityError::Conflict);
            }
            if rejected {
                state.last_error = Some("CLIENT_CERT_REJECTED".into())
            } else if state.last_error.as_deref() == Some("CLIENT_CERT_REJECTED") {
                state.last_error = None
            }
            Ok(())
        })
    }
    pub fn disable_local(&self) -> Result<u64> {
        self.change(|state| {
            retire(state)?;
            state.generation = state
                .generation
                .checked_add(1)
                .ok_or(IdentityError::InvalidState)?;
            state.last_error = None;
            Ok(state.generation)
        })
    }

    pub(crate) fn begin_renewal(&self, now: i64) -> Result<(u64, ActiveIdentity, PendingRenewal)> {
        self.change(|state| {
            let active = state.active.as_ref().ok_or(IdentityError::ReauthRequired)?;
            if state.renewal.is_none() {
                if active.certificate.not_after <= now {
                    return Err(IdentityError::ReauthRequired);
                }
                if active.certificate.not_after > now + 7 * 86400 {
                    return Err(IdentityError::Conflict);
                }
                state.renewal = Some(PendingRenewal {
                    operation_id: Uuid::new_v4().to_string(),
                    key: DeviceKey::generate()?,
                    created_at: now,
                });
            }
            Ok((
                state.generation,
                active.clone(),
                state.renewal.clone().unwrap(),
            ))
        })
    }

    pub(crate) fn activate_renewal(
        &self,
        generation: u64,
        operation: &str,
        certificate: IssuedCertificate,
        now: i64,
    ) -> Result<()> {
        self.change(|state| {
            if state.generation != generation {
                return Err(IdentityError::Conflict);
            }
            let active = state.active.as_ref().ok_or(IdentityError::Conflict)?;
            let pending = state.renewal.as_ref().ok_or(IdentityError::Conflict)?;
            if pending.operation_id != operation
                || certificate.issuer_id == active.certificate.issuer_id
                    && certificate.serial == active.certificate.serial
                || certificate.not_after <= active.certificate.not_after
            {
                return Err(IdentityError::InvalidCertificate);
            }
            self.trust.validate(
                &certificate,
                &pending.key,
                Some(&active.certificate.identity),
                now,
            )?;
            state.active = Some(ActiveIdentity {
                key: pending.key.clone(),
                certificate,
            });
            state.renewal = None;
            state.last_error = None;
            state.generation = state
                .generation
                .checked_add(1)
                .ok_or(IdentityError::InvalidState)?;
            Ok(())
        })
    }

    pub(crate) fn validate_renewal_result(
        &self,
        certificate: &IssuedCertificate,
        key: &DeviceKey,
        original: &ActiveIdentity,
        now: i64,
    ) -> Result<()> {
        self.trust
            .validate(certificate, key, Some(&original.certificate.identity), now)
    }
}

fn retire(state: &mut State) -> Result<()> {
    if state.active.is_some() && state.revocations.len() >= 32
        || state.pending.is_some() && state.cancellations.len() >= 32
    {
        return Err(IdentityError::Busy);
    }
    if let Some(active) = state.active.take() {
        state.revocations.push(RevokeTodo {
            active,
            renewal: state.renewal.take(),
        });
    }
    if let Some(pending) = state.pending.take() {
        if pending.started.is_some() {
            state.cancellations.push(pending)
        }
    }
    state.renewal = None;
    Ok(())
}
