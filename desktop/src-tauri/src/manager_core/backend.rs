use std::path::PathBuf;
use std::sync::Mutex;

use bytes::Bytes;
use http::{Request, StatusCode};
use http_body_util::{BodyExt, Empty};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

use crate::protocol::RouterOwner;
use crate::router_core::{RouterSupervisor, RuntimePhase, SupervisorConfig};

use super::errors::{lifecycle_error_from_supervisor, LifecycleError};
use super::lifecycle::Backend;
use super::types::{Classification, DiscoverySnapshot, HealthInfo, StartedRouter, VersionInfo};

type StartFn = Box<dyn FnMut(RouterOwner) -> Result<StartedRouter, LifecycleError> + Send>;
type ReclaimFn = Box<dyn FnMut() -> Result<StartedRouter, LifecycleError> + Send>;
type StopFn = Box<dyn FnMut() -> Result<(), LifecycleError> + Send>;

pub struct FakeBackend {
    start: Mutex<StartFn>,
    reclaim: Mutex<ReclaimFn>,
    stop: Mutex<StopFn>,
    status: Mutex<DiscoverySnapshot>,
    health: Mutex<DiscoverySnapshot>,
    recent: Mutex<String>,
    log_path: Mutex<Option<PathBuf>>,
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self {
            start: Mutex::new(Box::new(|_| {
                Err(LifecycleError::new(
                    crate::protocol::ErrorCode::RouterNotFound,
                ))
            })),
            reclaim: Mutex::new(Box::new(|| {
                Err(LifecycleError::new(
                    crate::protocol::ErrorCode::RouterStateStale,
                ))
            })),
            stop: Mutex::new(Box::new(|| Ok(()))),
            status: Mutex::new(DiscoverySnapshot::absent()),
            health: Mutex::new(DiscoverySnapshot::absent()),
            recent: Mutex::new(String::new()),
            log_path: Mutex::new(None),
        }
    }
}

impl FakeBackend {
    pub fn set_start<F>(&self, start: F)
    where
        F: FnMut(RouterOwner) -> Result<StartedRouter, LifecycleError> + Send + 'static,
    {
        *self.start.lock().expect("start lock") = Box::new(start);
    }

    pub fn set_reclaim<F>(&self, reclaim: F)
    where
        F: FnMut() -> Result<StartedRouter, LifecycleError> + Send + 'static,
    {
        *self.reclaim.lock().expect("reclaim lock") = Box::new(reclaim);
    }

    pub fn set_stop<F>(&self, stop: F)
    where
        F: FnMut() -> Result<(), LifecycleError> + Send + 'static,
    {
        *self.stop.lock().expect("stop lock") = Box::new(stop);
    }

    pub fn set_status(&self, snapshot: DiscoverySnapshot) {
        *self.status.lock().expect("status lock") = snapshot;
    }

    pub fn set_health(&self, snapshot: DiscoverySnapshot) {
        *self.health.lock().expect("health lock") = snapshot;
    }

    pub fn set_recent(&self, recent: impl Into<String>) {
        *self.recent.lock().expect("recent lock") = recent.into();
    }

    pub fn set_log_path(&self, path: Option<PathBuf>) {
        *self.log_path.lock().expect("log path lock") = path;
    }
}

impl Backend for FakeBackend {
    async fn start(&self, owner: RouterOwner) -> Result<StartedRouter, LifecycleError> {
        (self.start.lock().expect("start lock"))(owner)
    }

    async fn reclaim(&self) -> Result<StartedRouter, LifecycleError> {
        (self.reclaim.lock().expect("reclaim lock"))()
    }

    async fn stop(&self) -> Result<(), LifecycleError> {
        (self.stop.lock().expect("stop lock"))()
    }

    async fn discover_status(&self) -> DiscoverySnapshot {
        self.status.lock().expect("status lock").clone()
    }

    async fn discover_health(&self) -> DiscoverySnapshot {
        self.health.lock().expect("health lock").clone()
    }

    fn recent_output(&self) -> String {
        self.recent.lock().expect("recent lock").clone()
    }

    fn log_path(&self) -> Option<PathBuf> {
        self.log_path.lock().expect("log path lock").clone()
    }
}

pub struct SupervisorBackend {
    supervisor: RouterSupervisor,
    identity: VersionInfo,
    listen: Mutex<Option<String>>,
    recent: Mutex<String>,
}

impl SupervisorBackend {
    pub fn new(config: SupervisorConfig) -> Self {
        let identity = VersionInfo {
            version: config.identity.version().to_owned(),
            deployment_id: config.identity.deployment_id().to_owned(),
            management_protocol_version: config.identity.management_protocol_version().to_owned(),
        };
        Self {
            supervisor: RouterSupervisor::spawn(config),
            identity,
            listen: Mutex::new(None),
            recent: Mutex::new(String::new()),
        }
    }

    pub fn supervisor(&self) -> &RouterSupervisor {
        &self.supervisor
    }

    pub async fn shutdown(self) -> Result<(), crate::router_core::SupervisorError> {
        self.supervisor.shutdown().await
    }

    fn owned_snapshot(&self, include_health: bool) -> DiscoverySnapshot {
        let status = self.supervisor.status();
        let listen = self.listen.lock().expect("listen lock").clone();
        if !status.bound {
            return DiscoverySnapshot::absent();
        }
        let classification = match status.phase {
            RuntimePhase::Degraded => Classification::Degraded,
            RuntimePhase::Running => Classification::DesktopOwned,
            _ => return DiscoverySnapshot::absent(),
        };
        DiscoverySnapshot {
            classification,
            owner: Some(RouterOwner::Desktop.as_str().to_owned()),
            listen_addr: listen,
            pid: None,
            version: Some(self.identity.clone()),
            state_version: None,
            health: include_health.then(|| HealthInfo {
                status: if classification == Classification::Degraded {
                    "degraded".to_owned()
                } else {
                    "ok".to_owned()
                },
            }),
            log_path: None,
        }
    }
}

impl Backend for SupervisorBackend {
    async fn start(&self, owner: RouterOwner) -> Result<StartedRouter, LifecycleError> {
        match self.supervisor.start().await {
            Ok(addr) => {
                *self.listen.lock().expect("listen lock") = Some(addr.clone());
                *self.recent.lock().expect("recent lock") = String::new();
                Ok(StartedRouter {
                    owner,
                    listen_addr: addr,
                    pid: None,
                })
            }
            Err(error) => {
                let mapped = lifecycle_error_from_supervisor(error);
                *self.recent.lock().expect("recent lock") = mapped.recent_output.clone();
                Err(mapped)
            }
        }
    }

    async fn reclaim(&self) -> Result<StartedRouter, LifecycleError> {
        Err(LifecycleError::new(
            crate::protocol::ErrorCode::RouterAlreadyRunning,
        ))
    }

    async fn stop(&self) -> Result<(), LifecycleError> {
        self.supervisor
            .stop()
            .await
            .map_err(lifecycle_error_from_supervisor)?;
        *self.listen.lock().expect("listen lock") = None;
        Ok(())
    }

    async fn discover_status(&self) -> DiscoverySnapshot {
        self.owned_snapshot(false)
    }

    async fn discover_health(&self) -> DiscoverySnapshot {
        let mut snapshot = self.owned_snapshot(true);
        let Some(addr) = snapshot.listen_addr.clone() else {
            return snapshot;
        };
        if let Some(status) = fetch_health_status(&addr).await {
            snapshot.health = Some(HealthInfo {
                status: status.clone(),
            });
            if status != "ok" {
                snapshot.classification = Classification::Degraded;
            }
        }
        snapshot
    }

    fn recent_output(&self) -> String {
        self.recent.lock().expect("recent lock").clone()
    }

    fn log_path(&self) -> Option<PathBuf> {
        None
    }
}

pub(super) async fn fetch_health_status(addr: &str) -> Option<String> {
    let stream = TcpStream::connect(addr).await.ok()?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .ok()?;
    tokio::spawn(connection);
    let response = sender
        .send_request(
            Request::builder()
                .method(http::Method::GET)
                .uri("/health")
                .body(Empty::<Bytes>::new())
                .ok()?,
        )
        .await
        .ok()?;
    if response.status() != StatusCode::OK {
        return None;
    }
    let body = response.into_body().collect().await.ok()?.to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).ok()?;
    value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}
