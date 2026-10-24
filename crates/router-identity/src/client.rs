use crate::{
    store::{ActiveIdentity, PendingRenewal},
    wire::*,
    IdentityError, IdentityStore, Result, AUTH_PROTOCOL_VERSION, ENROLL_ORIGIN, RELAY_ORIGIN,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Route {
    Start,
    Poll,
    Cancel,
    Recover,
    Revoke,
    Renew,
}
impl Route {
    fn path(self) -> &'static str {
        match self {
            Self::Start => "/enroll/native/start",
            Self::Poll => "/enroll/native/poll",
            Self::Cancel => "/enroll/native/cancel",
            Self::Recover => "/enroll/renew/recover",
            Self::Revoke => "/enroll/device/revoke",
            Self::Renew => "/enroll/renew",
        }
    }
    fn origin(self) -> &'static str {
        if matches!(self, Self::Renew) {
            RELAY_ORIGIN
        } else {
            ENROLL_ORIGIN
        }
    }
}
pub(crate) struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}
type WireFuture<'a> = Pin<Box<dyn Future<Output = Result<Reply>> + Send + 'a>>;
pub(crate) trait WireBackend: Send + Sync {
    fn post(
        &self,
        route: Route,
        body: Zeroizing<Vec<u8>>,
        identity: Option<ActiveIdentity>,
    ) -> WireFuture<'_>;
}
struct HttpsBackend {
    public: reqwest::Client,
    server_roots: String,
    #[cfg(test)]
    origins: Option<(String, String)>,
}

fn builder() -> reqwest::ClientBuilder {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .tls_backend_rustls()
        .https_only(true)
        .no_proxy()
        .http1_only()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .pool_max_idle_per_host(0)
}
impl WireBackend for HttpsBackend {
    fn post(
        &self,
        route: Route,
        body: Zeroizing<Vec<u8>>,
        identity: Option<ActiveIdentity>,
    ) -> WireFuture<'_> {
        Box::pin(async move {
            let client = if matches!(route, Route::Renew) {
                let identity = identity.ok_or(IdentityError::ReauthRequired)?;
                let material = Zeroizing::new(format!(
                    "{}{}{}",
                    identity.certificate.certificate_pem,
                    identity.certificate.chain_pem,
                    identity.private_key_pem()
                ));
                let tls_identity = reqwest::Identity::from_pem(material.as_bytes())
                    .map_err(|_| IdentityError::InvalidCertificate)?;
                let roots = reqwest::Certificate::from_pem_bundle(self.server_roots.as_bytes())
                    .map_err(|_| IdentityError::InvalidCertificate)?;
                if roots.is_empty() {
                    return Err(IdentityError::InvalidCertificate);
                }
                builder()
                    .tls_certs_only(roots)
                    .identity(tls_identity)
                    .build()
                    .map_err(|_| IdentityError::Unavailable)?
            } else {
                self.public.clone()
            };
            let origin = route.origin();
            #[cfg(test)]
            let origin = self
                .origins
                .as_ref()
                .map(|v| {
                    if matches!(route, Route::Renew) {
                        v.1.as_str()
                    } else {
                        v.0.as_str()
                    }
                })
                .unwrap_or(origin);
            let mut response = client
                .post(format!("{}{}", origin, route.path()))
                .header("Content-Type", "application/json")
                .body(body.to_vec())
                .send()
                .await
                .map_err(|_| IdentityError::Unavailable)?;
            if response.content_length().is_some_and(|len| len > 65536) {
                return Err(IdentityError::Unavailable);
            }
            let status = response.status().as_u16();
            let mut data = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| IdentityError::Unavailable)?
            {
                if data.len() + chunk.len() > 65536 {
                    return Err(IdentityError::Unavailable);
                }
                data.extend_from_slice(&chunk);
            }
            Ok(Reply { status, body: data })
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ConnectionSummary {
    pub generation: u64,
    pub phase: String,
    pub user_id: Option<String>,
    pub device_id: Option<String>,
    pub expires_at: Option<i64>,
    pub revocation_pending: bool,
    pub error_code: Option<String>,
}

#[derive(Clone)]
pub struct NativeClient {
    store: IdentityStore,
    wire: Arc<dyn WireBackend>,
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
}
impl std::fmt::Debug for NativeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeClient").finish_non_exhaustive()
    }
}
impl NativeClient {
    #[cfg(test)]
    pub(crate) fn with_backend(
        store: IdentityStore,
        wire: Arc<dyn WireBackend>,
        now: Arc<dyn Fn() -> i64 + Send + Sync>,
    ) -> Self {
        Self { store, wire, now }
    }
    pub fn new(store: IdentityStore, relay_server_roots: String) -> Result<Self> {
        let public = builder().build().map_err(|_| IdentityError::Unavailable)?;
        Ok(Self {
            store,
            wire: Arc::new(HttpsBackend {
                public,
                server_roots: relay_server_roots,
                #[cfg(test)]
                origins: None,
            }),
            now: Arc::new(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0)
            }),
        })
    }
    pub fn store(&self) -> &IdentityStore {
        &self.store
    }

    #[cfg(test)]
    pub(crate) fn https_fixture(
        store: IdentityStore,
        server_roots: String,
        enroll: String,
        relay: String,
    ) -> Self {
        let parsed = reqwest::Url::parse(&enroll).unwrap();
        let address = std::net::SocketAddr::from(([127, 0, 0, 1], parsed.port().unwrap()));
        let roots = reqwest::Certificate::from_pem_bundle(server_roots.as_bytes()).unwrap();
        let public = builder()
            .tls_certs_only(roots)
            .resolve("enroll.codeasier.top", address)
            .build()
            .unwrap();
        Self {
            store,
            wire: Arc::new(HttpsBackend {
                public,
                server_roots,
                origins: Some((enroll, relay)),
            }),
            now: Arc::new(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64
            }),
        }
    }
    async fn note_error(&self, generation: u64, error: IdentityError) {
        let _ = self
            .disk(move |s| {
                s.change(|state| {
                    if state.generation != generation {
                        return Err(IdentityError::Conflict);
                    }
                    if state.pending.is_none() && state.renewal.is_none() {
                        return Ok(());
                    }
                    state.last_error = Some(error.code().into());
                    Ok(())
                })
            })
            .await;
    }
    async fn disk<T: Send + 'static>(
        &self,
        operation: impl FnOnce(IdentityStore) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || operation(store))
            .await
            .map_err(|_| IdentityError::Storage)?
    }
    pub async fn status(&self) -> Result<ConnectionSummary> {
        let state = self.disk(|s| s.read()).await?;
        let now = (self.now)();
        let active = state.active.as_ref();
        let phase = if state.last_error.as_deref() == Some("CLIENT_CERT_REJECTED") {
            "rejected"
        } else if state.last_error.as_deref() == Some("CERT_REAUTH_REQUIRED") {
            "reauth_required"
        } else if state.renewal.is_some() {
            "renewing"
        } else if state
            .pending
            .as_ref()
            .is_some_and(|pending| pending.started.is_some())
        {
            "authorizing"
        } else if active.is_some_and(|v| v.certificate.not_after <= now) {
            "expired"
        } else if active.is_some() {
            "connected"
        } else if !state.revocations.is_empty() || !state.cancellations.is_empty() {
            "revocation_pending"
        } else {
            "disconnected"
        };
        Ok(ConnectionSummary {
            generation: state.generation,
            phase: phase.into(),
            user_id: active.map(|v| v.certificate.identity.user_id.clone()),
            device_id: active.map(|v| v.certificate.identity.device_id.clone()),
            expires_at: active.map(|v| v.certificate.not_after),
            revocation_pending: !state.revocations.is_empty() || !state.cancellations.is_empty(),
            error_code: state.last_error,
        })
    }
    async fn send<T: Serialize, V: DeserializeOwned>(
        &self,
        route: Route,
        request: &T,
        identity: Option<ActiveIdentity>,
    ) -> Result<V> {
        let body =
            Zeroizing::new(serde_json::to_vec(request).map_err(|_| IdentityError::InvalidState)?);
        let reply = self.wire.post(route, body, identity).await?;
        if reply.status != 200 && reply.status != 201 {
            let error: serde_json::Value =
                serde_json::from_slice(&reply.body).map_err(|_| IdentityError::Unavailable)?;
            return Err(
                match error.get("code").and_then(|v| v.as_str()).unwrap_or("") {
                    "CERT_OPERATION_CONFLICT" => IdentityError::Conflict,
                    "CERT_REAUTH_REQUIRED"
                    | "ENROLL_EXPIRED"
                    | "ENROLL_DENIED"
                    | "ORG_MEMBERSHIP_REQUIRED" => IdentityError::ReauthRequired,
                    "CLIENT_CERT_REJECTED" | "ENROLL_CANCELLED" => IdentityError::Rejected,
                    "ENROLL_SLOW_DOWN" | "ENROLL_CAPACITY_REACHED" => IdentityError::Busy,
                    _ => IdentityError::Unavailable,
                },
            );
        }
        serde_json::from_slice(&reply.body).map_err(|_| IdentityError::InvalidCertificate)
    }
    pub async fn connect(&self, kind: &str, label: &str) -> Result<NativeStarted> {
        let kind = kind.to_owned();
        let label = label.to_owned();
        let now = (self.now)();
        let (generation, pending) = self
            .disk(move |s| s.begin_login(&kind, &label, now))
            .await?;
        let started: NativeStarted = match self.send(Route::Start, &pending.request(), None).await {
            Ok(v) => v,
            Err(err) => {
                self.note_error(generation, err).await;
                return Err(err);
            }
        };
        let result = started.clone();
        self.disk(move |s| s.record_started(generation, started))
            .await?;
        Ok(result)
    }
    pub async fn poll(&self) -> Result<ConnectionSummary> {
        let state = self.disk(|s| s.read()).await?;
        let pending = state.pending.ok_or(IdentityError::Conflict)?;
        let started = pending.started.as_ref().ok_or(IdentityError::Conflict)?;
        let response: Result<NativeResult> = self
            .send(
                Route::Poll,
                &NativeProof {
                    transaction: &started.transaction,
                    verifier: &pending.verifier,
                },
                None,
            )
            .await;
        let result = match response {
            Ok(v) => v,
            Err(err) => {
                self.note_error(state.generation, err).await;
                return Err(err);
            }
        };
        if result.interval != 5 || result.status != "issued" && result.certificate.is_some() {
            return Err(IdentityError::InvalidCertificate);
        }
        if result.status == "issued" {
            let certificate = result
                .certificate
                .ok_or(IdentityError::InvalidCertificate)?;
            let now = (self.now)();
            self.disk(move |s| s.activate_login(state.generation, certificate, now))
                .await?;
        } else if !matches!(
            result.status.as_str(),
            "pending" | "authorizing" | "verifying" | "verified" | "approved"
        ) {
            return Err(IdentityError::Rejected);
        }
        self.status().await
    }
    pub async fn logout(&self) -> Result<ConnectionSummary> {
        self.disk(|s| s.disable_local()).await?;
        // Local disablement has already committed even if the network is down.
        let _ = tokio::time::timeout(Duration::from_secs(2), self.flush_deactivations()).await;
        self.status().await
    }
    pub async fn renew(&self) -> Result<ConnectionSummary> {
        let now = (self.now)();
        let (generation, active, pending) = self.disk(move |s| s.begin_renewal(now)).await?;
        let result = self.recover(&active, &pending).await;
        let certificate_result = match result {
            Ok(value) => Ok(value),
            Err(IdentityError::Conflict) if active.certificate.not_after > now => {
                let request = renewal_request(&active, &pending, false, now)?;
                self.send(Route::Renew, &request, Some(active.clone()))
                    .await
            }
            Err(err) => Err(err),
        };
        let certificate = match certificate_result {
            Ok(v) => v,
            Err(err) => {
                self.note_error(generation, err).await;
                return Err(err);
            }
        };
        let activated_at = (self.now)();
        self.disk(move |s| {
            s.activate_renewal(generation, &pending.operation_id, certificate, activated_at)
        })
        .await?;
        self.status().await
    }
    async fn recover(
        &self,
        active: &ActiveIdentity,
        pending: &PendingRenewal,
    ) -> Result<IssuedCertificate> {
        let request = renewal_request(active, pending, true, (self.now)())?;
        self.send(Route::Recover, &request, None).await
    }
    pub async fn flush_deactivations(&self) -> Result<()> {
        let now = (self.now)();
        self.disk(move |s| {
            s.change(|state| {
                state.revocations.retain(|v| {
                    now < v
                        .active
                        .certificate
                        .not_after
                        .saturating_add(if v.renewal.is_some() { 30 * 86400 } else { 0 })
                });
                state
                    .cancellations
                    .retain(|v| now < v.created_at.saturating_add(600 + 30 * 86400));
                Ok(())
            })
        })
        .await?;
        let state = self.disk(|s| s.read()).await?;
        for pending in state.cancellations {
            let Some(started) = &pending.started else {
                continue;
            };
            let response: Result<NativeResult> = self
                .send(
                    Route::Cancel,
                    &NativeProof {
                        transaction: &started.transaction,
                        verifier: &pending.verifier,
                    },
                    None,
                )
                .await;
            let Ok(result) = response else { continue };
            if matches!(result.status.as_str(), "cancelled" | "expired" | "revoked") {
                let id = started.transaction.clone();
                self.disk(move |s| {
                    s.change(|state| {
                        state
                            .cancellations
                            .retain(|p| p.started.as_ref().map_or(true, |v| v.transaction != id));
                        Ok(())
                    })
                })
                .await?;
            }
        }
        for todo in state.revocations {
            let mut target = todo.active.clone();
            if let Some(pending) = &todo.renewal {
                if let Ok(certificate) = self.recover(&todo.active, pending).await {
                    self.store.validate_renewal_result(
                        &certificate,
                        &pending.key,
                        &todo.active,
                        (self.now)(),
                    )?;
                    target = ActiveIdentity {
                        key: pending.key.clone(),
                        certificate,
                    };
                }
            }
            let now = (self.now)();
            let request = revoke_request(&target, now)?;
            let result: DeviceRevokeResult = match self.send(Route::Revoke, &request, None).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            if result.status == "revoked" {
                let issuer = todo.active.certificate.issuer_id;
                let serial = todo.active.certificate.serial;
                self.disk(move |s| {
                    s.change(|state| {
                        state.revocations.retain(|v| {
                            v.active.certificate.issuer_id != issuer
                                || v.active.certificate.serial != serial
                        });
                        Ok(())
                    })
                })
                .await?;
            }
        }
        Ok(())
    }

    pub async fn run_maintenance(&self, mut stop: tokio::sync::watch::Receiver<bool>) {
        let mut failures = 0u32;
        loop {
            if *stop.borrow() {
                break;
            }
            let work = async {
                let _ =
                    tokio::time::timeout(Duration::from_secs(2), self.flush_deactivations()).await;
                let state = self.disk(|s| s.read()).await?;
                if state.pending.as_ref().is_some_and(|p| p.started.is_some()) {
                    self.poll().await?;
                    return Ok::<_, IdentityError>(5u64);
                }
                if state.renewal.is_some()
                    || state
                        .active
                        .as_ref()
                        .is_some_and(|v| v.certificate.not_after <= (self.now)() + 7 * 86400)
                {
                    self.renew().await?;
                }
                Ok(30)
            };
            let result = tokio::select! {_ = stop.changed()=>break,value=work=>value};
            let seconds = match result {
                Ok(seconds) => {
                    failures = 0;
                    seconds
                }
                Err(_) => {
                    failures = (failures + 1).min(6);
                    (5u64 << failures).min(300)
                }
            };
            let mut random = [0u8; 2];
            let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut random);
            let delay = Duration::from_secs(seconds)
                + Duration::from_millis(u16::from_le_bytes(random) as u64 % 1000);
            tokio::select! {_ = stop.changed()=>break,_=tokio::time::sleep(delay)=>{}}
        }
    }
}

pub(crate) fn renewal_request<'a>(
    active: &'a ActiveIdentity,
    pending: &'a PendingRenewal,
    recover: bool,
    now: i64,
) -> Result<RenewalRequest<'a>> {
    let route = if recover {
        Route::Recover
    } else {
        Route::Renew
    };
    let purpose = if recover { "recover" } else { "renew" };
    let message = format!(
        "codeasier-router-renew-v1\n{purpose}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{now}\n",
        route.origin(),
        route.path(),
        AUTH_PROTOCOL_VERSION,
        active.certificate.issuer_id,
        active.certificate.serial,
        pending.operation_id,
        pending.key.csr_digest
    );
    Ok(RenewalRequest {
        version: AUTH_PROTOCOL_VERSION,
        issuer_id: &active.certificate.issuer_id,
        serial: &active.certificate.serial,
        operation_id: &pending.operation_id,
        csr_pem: &pending.key.csr_pem,
        timestamp: now,
        signature: active.key.sign(message.as_bytes())?,
    })
}
fn revoke_request(active: &ActiveIdentity, now: i64) -> Result<DeviceRevokeRequest<'_>> {
    let message=format!("codeasier-router-revoke-v1\n{ENROLL_ORIGIN}\n/enroll/device/revoke\n{AUTH_PROTOCOL_VERSION}\n{}\n{}\n{now}\n",active.certificate.issuer_id,active.certificate.serial);
    Ok(DeviceRevokeRequest {
        version: AUTH_PROTOCOL_VERSION,
        issuer_id: &active.certificate.issuer_id,
        serial: &active.certificate.serial,
        timestamp: now,
        signature: active.key.sign(message.as_bytes())?,
    })
}
