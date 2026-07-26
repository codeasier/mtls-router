use crate::{
    error::CommandError,
    manager::ManagerClient,
    port_recovery::PortRecovery,
    types::{PollError, PollSnapshot, RouterHealth, RouterStatus},
};
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::{watch, RwLock};

type Observer = Arc<dyn Fn(PollSnapshot) + Send + Sync>;

struct PollDeadlines {
    status: Instant,
    health: Instant,
}

impl PollDeadlines {
    fn now() -> Self {
        Self {
            status: Instant::now(),
            health: Instant::now(),
        }
    }
}

#[derive(Clone)]
pub struct PollScheduler {
    manager: ManagerClient,
    snapshot: Arc<RwLock<PollSnapshot>>,
    port_recovery: Arc<RwLock<PortRecovery>>,
    visible: Arc<AtomicBool>,
    status_generation: Arc<AtomicU64>,
    health_generation: Arc<AtomicU64>,
    status_immediate: Arc<AtomicBool>,
    health_immediate: Arc<AtomicBool>,
    observer: Observer,
}

impl PollScheduler {
    #[cfg(test)]
    pub fn new(manager: ManagerClient) -> Self {
        Self::with_observer(manager, |_| {})
    }

    pub fn with_observer<F>(manager: ManagerClient, observer: F) -> Self
    where
        F: Fn(PollSnapshot) + Send + Sync + 'static,
    {
        Self {
            manager,
            snapshot: Arc::new(RwLock::new(PollSnapshot::default())),
            port_recovery: Arc::new(RwLock::new(PortRecovery::default())),
            visible: Arc::new(AtomicBool::new(true)),
            status_generation: Arc::new(AtomicU64::new(0)),
            health_generation: Arc::new(AtomicU64::new(0)),
            status_immediate: Arc::new(AtomicBool::new(true)),
            health_immediate: Arc::new(AtomicBool::new(true)),
            observer: Arc::new(observer),
        }
    }

    pub fn start(&self) {
        let scheduler = self.clone();
        tauri::async_runtime::spawn(async move { scheduler.run().await });
    }

    pub fn set_visible(&self, visible: bool) {
        self.visible.store(visible, Ordering::Release);
        self.request_refresh();
    }

    pub fn request_refresh(&self) {
        self.request_status_refresh();
        self.request_health_refresh();
    }

    pub fn request_status_refresh(&self) {
        self.status_generation.fetch_add(1, Ordering::AcqRel);
        self.status_immediate.store(true, Ordering::Release);
    }

    pub fn request_health_refresh(&self) {
        self.health_generation.fetch_add(1, Ordering::AcqRel);
        self.health_immediate.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn status_generation(&self) -> u64 {
        self.status_generation.load(Ordering::Acquire)
    }

    pub async fn snapshot(&self) -> PollSnapshot {
        self.snapshot.read().await.clone()
    }

    pub async fn set_status(&self, status: RouterStatus) {
        self.apply_status(status, None).await;
        self.request_refresh();
    }

    pub async fn begin_release_observation(&self, manager_session_epoch: u64) -> bool {
        let mut recovery = self.port_recovery.write().await;
        if self.manager.session_epoch() != manager_session_epoch {
            return false;
        }
        recovery.begin(manager_session_epoch, Instant::now());
        let mut snapshot = self.snapshot.write().await;
        if snapshot.release_observation.take().is_some() {
            self.publish(&mut snapshot);
        }
        drop(snapshot);
        drop(recovery);
        self.request_status_refresh();
        true
    }

    pub async fn cancel_release_observation(&self) {
        let mut recovery = self.port_recovery.write().await;
        recovery.cancel();
        let mut snapshot = self.snapshot.write().await;
        if snapshot.release_observation.take().is_some() {
            self.publish(&mut snapshot);
        }
    }

    pub async fn set_status_error(&self, error: &CommandError) {
        let mut recovery = self.port_recovery.write().await;
        recovery.cancel();
        let mut snapshot = self.snapshot.write().await;
        snapshot.release_observation = None;
        snapshot.status_error = Some(PollError::new(&error.code));
        self.publish(&mut snapshot);
    }

    pub async fn set_health(&self, health: RouterHealth) {
        self.apply_health(health).await;
    }

    pub async fn set_health_error(&self, error: &CommandError) {
        let mut snapshot = self.snapshot.write().await;
        snapshot.health_error = Some(PollError::new(&error.code));
        self.publish(&mut snapshot);
    }

    async fn run(self) {
        let mut deadlines = PollDeadlines::now();
        let mut activity = self.manager.subscribe_activity();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                changed = activity.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
            }
            self.run_due(&mut deadlines, &mut activity).await;
        }
    }

    async fn run_due(
        &self,
        deadlines: &mut PollDeadlines,
        activity: &mut watch::Receiver<crate::manager::ManagerActivity>,
    ) {
        let now = Instant::now();
        self.mark_health_stale().await;
        self.cancel_for_manager_epoch_change().await;
        let activity_generation = activity.borrow_and_update().generation;
        let status_due =
            self.status_immediate.swap(false, Ordering::AcqRel) || now >= deadlines.status;
        let mut health_due =
            self.health_immediate.swap(false, Ordering::AcqRel) || now >= deadlines.health;
        if self.manager.is_busy() {
            self.defer(status_due, health_due);
            return;
        }

        let was_available = self.available().await;
        if status_due {
            if !self.poll_status(activity_generation).await {
                self.defer(true, health_due);
                return;
            }
            deadlines.status = Instant::now() + self.status_interval();
        }

        let available = self.available().await;
        health_due = health_due || !was_available;
        if available && health_due {
            if !self.poll_health(activity_generation).await {
                self.defer(false, true);
                return;
            }
            deadlines.health = Instant::now() + self.health_interval();
        } else if !available {
            self.clear_health().await;
            deadlines.health = Instant::now();
        }
    }

    fn defer(&self, status: bool, health: bool) {
        if status {
            self.status_immediate.store(true, Ordering::Release);
        }
        if health {
            self.health_immediate.store(true, Ordering::Release);
        }
    }

    async fn poll_status(&self, activity_generation: u64) -> bool {
        let generation = self.status_generation.load(Ordering::Acquire);
        let Some(result) = self
            .manager
            .poll_with_session_epoch::<RouterStatus>(
                "router.status",
                json!({}),
                activity_generation,
            )
            .await
        else {
            return false;
        };
        let current = self.status_generation.load(Ordering::Acquire) == generation
            && self.manager.activity().generation == activity_generation;
        match result {
            Ok((status, session_epoch)) if current => {
                self.apply_status(status, Some(session_epoch)).await;
            }
            Err(error) if current => {
                self.set_status_error(&error).await;
            }
            _ => {}
        }
        current
    }

    async fn poll_health(&self, activity_generation: u64) -> bool {
        let generation = self.health_generation.load(Ordering::Acquire);
        let Some(result) = self
            .manager
            .poll::<RouterHealth>("router.health", json!({}), activity_generation)
            .await
        else {
            return false;
        };
        let current = self.health_generation.load(Ordering::Acquire) == generation
            && self.manager.activity().generation == activity_generation;
        match result {
            Ok(health) if current => {
                self.apply_health(health).await;
            }
            Err(error) if current => {
                self.set_health_error(&error).await;
            }
            _ => {}
        }
        current
    }

    async fn apply_status(&self, status: RouterStatus, session_epoch: Option<u64>) {
        let mut recovery = self.port_recovery.write().await;
        if let Some(session_epoch) = session_epoch {
            recovery.observe(&status, session_epoch, Instant::now());
        } else if status.available() {
            recovery.cancel();
        }
        let mut snapshot = self.snapshot.write().await;
        if !status.available() {
            snapshot.health = None;
            snapshot.health_stale = false;
            snapshot.health_error = None;
        }
        snapshot.status = Some(status);
        snapshot.status_error = None;
        snapshot.release_observation = recovery.projection();
        self.publish(&mut snapshot);
    }

    async fn apply_health(&self, health: RouterHealth) {
        let mut snapshot = self.snapshot.write().await;
        snapshot.health_stale = health_is_stale(&health);
        snapshot.health = Some(health);
        snapshot.health_error = None;
        self.publish(&mut snapshot);
    }

    async fn clear_health(&self) {
        let mut snapshot = self.snapshot.write().await;
        if snapshot.health.is_some() || snapshot.health_error.is_some() {
            snapshot.health = None;
            snapshot.health_stale = false;
            snapshot.health_error = None;
            self.publish(&mut snapshot);
        }
    }

    async fn available(&self) -> bool {
        self.snapshot
            .read()
            .await
            .status
            .as_ref()
            .is_some_and(RouterStatus::available)
    }

    async fn mark_health_stale(&self) {
        let mut snapshot = self.snapshot.write().await;
        if !snapshot.health_stale && snapshot.health.as_ref().is_some_and(health_is_stale) {
            snapshot.health_stale = true;
            self.publish(&mut snapshot);
        }
    }

    async fn cancel_for_manager_epoch_change(&self) {
        let mut recovery = self.port_recovery.write().await;
        let had_projection = recovery.projection().is_some();
        if !recovery.cancel_if_epoch_changed(self.manager.session_epoch()) {
            return;
        }
        if !had_projection {
            return;
        }
        let mut snapshot = self.snapshot.write().await;
        snapshot.release_observation = None;
        self.publish(&mut snapshot);
    }

    fn publish(&self, snapshot: &mut PollSnapshot) {
        snapshot.revision += 1;
        (self.observer)(snapshot.clone());
    }

    fn status_interval(&self) -> Duration {
        if self.visible.load(Ordering::Acquire) {
            Duration::from_secs(2)
        } else {
            Duration::from_secs(10)
        }
    }

    fn health_interval(&self) -> Duration {
        if self.visible.load(Ordering::Acquire) {
            Duration::from_secs(10)
        } else {
            Duration::from_secs(30)
        }
    }
}

fn health_is_stale(health: &RouterHealth) -> bool {
    chrono::DateTime::parse_from_rfc3339(&health.checked_at)
        .map(|checked_at| {
            chrono::Utc::now()
                .signed_duration_since(checked_at)
                .num_milliseconds()
                > 30_000
        })
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        error::Result,
        manager::{TransportChild, TransportEvent, TransportFactory, TransportSession},
        port_recovery::ReleaseObservationState,
    };
    use serde_json::Value;
    use std::sync::{atomic::AtomicUsize, Mutex, OnceLock};
    use tokio::sync::{mpsc, Semaphore};

    struct ControlledChild {
        calls: Arc<Mutex<Vec<String>>>,
        responder: mpsc::Sender<TransportEvent>,
        operation: &'static str,
        started: Arc<Semaphore>,
        release: Arc<Semaphore>,
    }

    impl TransportChild for ControlledChild {
        fn write(&mut self, bytes: &[u8]) -> Result<()> {
            let request: Value = serde_json::from_slice(bytes).unwrap();
            let method = request["method"].as_str().unwrap().to_owned();
            self.calls.lock().unwrap().push(method.clone());
            let responder = self.responder.clone();
            let operation = self.operation;
            let started = self.started.clone();
            let release = self.release.clone();
            tauri::async_runtime::spawn(async move {
                if method == operation {
                    started.add_permits(1);
                    release.acquire().await.unwrap().forget();
                }
                let result = match method.as_str() {
                    "manager.info" => json!({
                        "version": env!("MTLS_MANAGER_VERSION"), "commit": "test", "build_date": "test",
                        "target": env!("MTLS_MANAGER_TARGET"), "deployment_id": env!("MTLS_DEPLOYMENT_ID"),
                        "management_protocol_version": env!("MTLS_MANAGEMENT_PROTOCOL_VERSION")
                    }),
                    "router.status" => json!({ "state": "desktop_owned", "owner": "desktop" }),
                    "router.health" => json!({
                        "status": "ok", "checked_at": chrono::Utc::now().to_rfc3339()
                    }),
                    _ => json!({}),
                };
                let response = serde_json::to_vec(&json!({
                    "id": request["id"], "result": result
                }))
                .unwrap();
                responder
                    .send(TransportEvent::Stdout(response))
                    .await
                    .unwrap();
            });
            Ok(())
        }

        fn kill(self: Box<Self>) {}
    }

    struct ControlledFactory {
        calls: Arc<Mutex<Vec<String>>>,
        operation: &'static str,
        started: Arc<Semaphore>,
        release: Arc<Semaphore>,
    }

    impl TransportFactory for ControlledFactory {
        fn spawn(&self) -> Result<TransportSession> {
            let (responder, events) = mpsc::channel(16);
            Ok(TransportSession {
                child: Box::new(ControlledChild {
                    calls: self.calls.clone(),
                    responder,
                    operation: self.operation,
                    started: self.started.clone(),
                    release: self.release.clone(),
                }),
                events,
            })
        }
    }

    struct FailedRecoveryChild {
        responder: mpsc::Sender<TransportEvent>,
        replacement: bool,
    }

    impl TransportChild for FailedRecoveryChild {
        fn write(&mut self, bytes: &[u8]) -> Result<()> {
            let request: Value = serde_json::from_slice(bytes).unwrap();
            let method = request["method"].as_str().unwrap();
            let event = if method != "manager.info" {
                TransportEvent::Terminated
            } else {
                let target = if self.replacement {
                    "wrong/target"
                } else {
                    env!("MTLS_MANAGER_TARGET")
                };
                TransportEvent::Stdout(
                    serde_json::to_vec(&json!({"id":request["id"],"result":{
                        "version":env!("MTLS_MANAGER_VERSION"), "commit":"test", "build_date":"test",
                        "target":target, "deployment_id":env!("MTLS_DEPLOYMENT_ID"),
                        "management_protocol_version":env!("MTLS_MANAGEMENT_PROTOCOL_VERSION")
                    }}))
                    .unwrap(),
                )
            };
            self.responder.try_send(event).unwrap();
            Ok(())
        }

        fn kill(self: Box<Self>) {}
    }

    struct FailedRecoveryFactory {
        spawns: AtomicUsize,
    }

    impl TransportFactory for FailedRecoveryFactory {
        fn spawn(&self) -> Result<TransportSession> {
            let replacement = self.spawns.fetch_add(1, Ordering::AcqRel) > 0;
            let (responder, events) = mpsc::channel(8);
            Ok(TransportSession {
                child: Box::new(FailedRecoveryChild {
                    responder,
                    replacement,
                }),
                events,
            })
        }
    }

    fn runtime() -> &'static tokio::runtime::Runtime {
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().unwrap())
    }

    #[test]
    fn polling_intervals_follow_visibility() {
        // Manager behavior is covered through the transport tests; this test
        // verifies the scheduler policy independently of wall-clock polling.
        let scheduler = PollScheduler {
            manager: ManagerClient::failed(crate::error::CommandError::manager_failed()),
            snapshot: Arc::new(RwLock::new(PollSnapshot::default())),
            port_recovery: Arc::new(RwLock::new(PortRecovery::default())),
            visible: Arc::new(AtomicBool::new(true)),
            status_generation: Arc::new(AtomicU64::new(0)),
            health_generation: Arc::new(AtomicU64::new(0)),
            status_immediate: Arc::new(AtomicBool::new(false)),
            health_immediate: Arc::new(AtomicBool::new(false)),
            observer: Arc::new(|_| {}),
        };
        assert_eq!(scheduler.status_interval(), Duration::from_secs(2));
        assert_eq!(scheduler.health_interval(), Duration::from_secs(10));
        scheduler.set_visible(false);
        assert_eq!(scheduler.status_interval(), Duration::from_secs(10));
        assert_eq!(scheduler.health_interval(), Duration::from_secs(30));
    }

    #[test]
    fn mutations_publish_revisioned_snapshots_and_preserve_degraded_ownership() {
        let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = observed.clone();
        let scheduler = PollScheduler::with_observer(
            ManagerClient::failed(crate::error::CommandError::manager_failed()),
            move |snapshot| captured.lock().unwrap().push(snapshot),
        );

        tauri::async_runtime::block_on(async {
            scheduler
                .set_status(RouterStatus {
                    state: "degraded".to_owned(),
                    owner: Some("desktop".to_owned()),
                    ..RouterStatus::default()
                })
                .await;
            scheduler
                .set_health(RouterHealth {
                    status: "degraded".to_owned(),
                    checked_at: "2026-07-12T00:00:00Z".to_owned(),
                })
                .await;
        });

        let observed = observed.lock().unwrap();
        assert_eq!(observed.len(), 2);
        assert_eq!(observed[0].revision, 1);
        assert_eq!(observed[1].revision, 2);
        assert_eq!(
            observed[1].status.as_ref().unwrap().owner.as_deref(),
            Some("desktop")
        );
        assert_eq!(observed[1].health.as_ref().unwrap().status, "degraded");
    }

    #[test]
    fn release_observation_is_projected_and_cleared_through_revisions() {
        let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = observed.clone();
        let scheduler = PollScheduler::with_observer(
            ManagerClient::failed(crate::error::CommandError::manager_failed()),
            move |snapshot| captured.lock().unwrap().push(snapshot),
        );

        runtime().block_on(async {
            scheduler.begin_release_observation(0).await;
            assert_eq!(scheduler.snapshot().await.release_observation, None);
            assert!(observed.lock().unwrap().is_empty());

            scheduler
                .apply_status(
                    RouterStatus {
                        state: "absent".into(),
                        ..RouterStatus::default()
                    },
                    Some(0),
                )
                .await;
            assert_eq!(
                scheduler
                    .snapshot()
                    .await
                    .release_observation
                    .unwrap()
                    .state,
                crate::port_recovery::ReleaseObservationState::Observing
            );
            let serialized = serde_json::to_value(scheduler.snapshot().await).unwrap();
            assert_eq!(
                serialized["release_observation"],
                json!({ "state": "observing" })
            );
            assert!(!serialized["release_observation"]
                .to_string()
                .contains("4242"));

            scheduler
                .apply_status(
                    RouterStatus {
                        state: "unknown_occupant".into(),
                        ..RouterStatus::default()
                    },
                    Some(0),
                )
                .await;
            assert_eq!(
                scheduler
                    .snapshot()
                    .await
                    .release_observation
                    .unwrap()
                    .state,
                crate::port_recovery::ReleaseObservationState::Reoccupied
            );

            scheduler.cancel_release_observation().await;
            assert_eq!(scheduler.snapshot().await.release_observation, None);
        });

        let observed = observed.lock().unwrap();
        assert_eq!(observed.len(), 3);
        assert_eq!(
            observed
                .iter()
                .map(|snapshot| snapshot.revision)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[test]
    fn available_router_cancels_release_observation() {
        let scheduler = PollScheduler::new(ManagerClient::failed(
            crate::error::CommandError::manager_failed(),
        ));

        runtime().block_on(async {
            scheduler.begin_release_observation(0).await;
            scheduler
                .apply_status(
                    RouterStatus {
                        state: "external_compatible".into(),
                        owner: Some("external".into()),
                        ..RouterStatus::default()
                    },
                    None,
                )
                .await;

            assert_eq!(scheduler.snapshot().await.release_observation, None);
        });
    }

    #[test]
    fn non_absent_statuses_clear_projection_and_cannot_inherit_elapsed_time() {
        for state in [
            "desktop_owned",
            "external_compatible",
            "degraded",
            "stale",
            "start_failed",
            "stopping",
            "unexpected",
        ] {
            let scheduler = PollScheduler::new(ManagerClient::failed(
                crate::error::CommandError::manager_failed(),
            ));

            runtime().block_on(async {
                scheduler
                    .port_recovery
                    .write()
                    .await
                    .begin(0, Instant::now() - Duration::from_secs(9));
                scheduler
                    .apply_status(
                        RouterStatus {
                            state: "absent".into(),
                            ..RouterStatus::default()
                        },
                        Some(0),
                    )
                    .await;
                assert_eq!(
                    scheduler
                        .snapshot()
                        .await
                        .release_observation
                        .unwrap()
                        .state,
                    ReleaseObservationState::Observing,
                    "state {state}"
                );

                scheduler
                    .apply_status(
                        RouterStatus {
                            state: state.into(),
                            ..RouterStatus::default()
                        },
                        Some(0),
                    )
                    .await;
                assert_eq!(
                    scheduler.snapshot().await.release_observation,
                    None,
                    "state {state}"
                );

                scheduler
                    .apply_status(
                        RouterStatus {
                            state: "absent".into(),
                            ..RouterStatus::default()
                        },
                        Some(0),
                    )
                    .await;
                assert_eq!(
                    scheduler.snapshot().await.release_observation,
                    None,
                    "state {state}"
                );
            });
        }
    }

    #[test]
    fn stale_epoch_cannot_begin_release_observation() {
        let manager = ManagerClient::failed(crate::error::CommandError::manager_failed());
        let scheduler = PollScheduler::new(manager.clone());

        runtime().block_on(async {
            let inspected_epoch = manager.session_epoch();
            manager.invalidate_session_for_test();

            assert!(!scheduler.begin_release_observation(inspected_epoch).await);
            assert_eq!(scheduler.snapshot().await.release_observation, None);
        });
    }

    #[test]
    fn failed_manager_recovery_epoch_cancels_release_observation() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let captured = observed.clone();
        let manager = ManagerClient::new(Arc::new(FailedRecoveryFactory {
            spawns: AtomicUsize::new(0),
        }));
        let scheduler = PollScheduler::with_observer(manager.clone(), move |snapshot| {
            captured.lock().unwrap().push(snapshot)
        });

        runtime().block_on(async {
            assert!(scheduler.begin_release_observation(0).await);
            let error = manager
                .call::<Value>("router.logs", json!({ "limit": 1 }))
                .await
                .unwrap_err();
            assert_eq!(error.code, "SIDECAR_INVALID");
            assert_eq!(manager.session_epoch(), 1);

            scheduler.cancel_for_manager_epoch_change().await;
            assert_eq!(scheduler.snapshot().await.release_observation, None);
            assert_eq!(scheduler.snapshot().await.revision, 0);
            assert!(observed.lock().unwrap().is_empty());
        });
    }

    #[test]
    fn poll_errors_retain_cached_status_and_health() {
        let scheduler = PollScheduler::new(ManagerClient::failed(
            crate::error::CommandError::manager_failed(),
        ));
        let status = RouterStatus {
            state: "desktop_owned".to_owned(),
            owner: Some("desktop".to_owned()),
            ..RouterStatus::default()
        };
        let health = RouterHealth {
            status: "ok".to_owned(),
            checked_at: chrono::Utc::now().to_rfc3339(),
        };

        runtime().block_on(async {
            scheduler.apply_status(status.clone(), None).await;
            scheduler.apply_health(health.clone()).await;
            scheduler
                .set_status_error(&CommandError::manager_failed())
                .await;
            scheduler
                .set_health_error(&CommandError::manager_failed())
                .await;

            let snapshot = scheduler.snapshot().await;
            assert_eq!(snapshot.status, Some(status));
            assert_eq!(snapshot.health, Some(health));
            assert!(snapshot.status_error.is_some());
            assert!(snapshot.health_error.is_some());
        });
    }

    #[test]
    fn status_errors_cancel_active_and_terminal_release_observations() {
        for (elapsed, expected_state) in [
            (Duration::ZERO, ReleaseObservationState::Observing),
            (Duration::from_secs(10), ReleaseObservationState::Released),
        ] {
            let observed = Arc::new(Mutex::new(Vec::new()));
            let captured = observed.clone();
            let scheduler = PollScheduler::with_observer(
                ManagerClient::failed(crate::error::CommandError::manager_failed()),
                move |snapshot| captured.lock().unwrap().push(snapshot),
            );

            runtime().block_on(async {
                scheduler
                    .port_recovery
                    .write()
                    .await
                    .begin(0, Instant::now() - elapsed);
                scheduler
                    .apply_status(
                        RouterStatus {
                            state: "absent".into(),
                            ..RouterStatus::default()
                        },
                        Some(0),
                    )
                    .await;
                assert_eq!(
                    scheduler
                        .snapshot()
                        .await
                        .release_observation
                        .unwrap()
                        .state,
                    expected_state
                );

                scheduler
                    .set_status_error(&CommandError::manager_failed())
                    .await;

                let snapshot = scheduler.snapshot().await;
                assert_eq!(snapshot.release_observation, None);
                assert!(snapshot.status_error.is_some());
                assert_eq!(scheduler.port_recovery.read().await.projection(), None);
            });

            let observed = observed.lock().unwrap();
            assert_eq!(observed.last().unwrap().release_observation, None);
            assert!(observed.last().unwrap().status_error.is_some());
        }
    }

    #[test]
    fn health_older_than_thirty_seconds_is_stale() {
        assert!(health_is_stale(&RouterHealth {
            status: "ok".to_owned(),
            checked_at: "2026-01-01T00:00:00Z".to_owned(),
        }));
        assert!(health_is_stale(&RouterHealth {
            status: "ok".to_owned(),
            checked_at: "invalid".to_owned(),
        }));
    }

    #[test]
    fn due_tick_waits_for_every_manager_operation_then_refreshes_once() {
        runtime().block_on(async {
            for operation in [
                "agent.detect",
                "router.logs",
                "diagnostics.collect",
                "router.version",
            ] {
                let calls = Arc::new(Mutex::new(Vec::new()));
                let started = Arc::new(Semaphore::new(0));
                let release = Arc::new(Semaphore::new(0));
                let manager = ManagerClient::new(Arc::new(ControlledFactory {
                    calls: calls.clone(),
                    operation,
                    started: started.clone(),
                    release: release.clone(),
                }));
                let scheduler = PollScheduler::new(manager.clone());
                scheduler
                    .apply_status(
                        RouterStatus {
                            state: "desktop_owned".to_owned(),
                            owner: Some("desktop".to_owned()),
                            ..RouterStatus::default()
                        },
                        None,
                    )
                    .await;
                let mut activity = manager.subscribe_activity();
                let manager_call = {
                    let manager = manager.clone();
                    tauri::async_runtime::spawn(async move {
                        manager.call::<Value>(operation, json!({})).await.unwrap()
                    })
                };

                started.acquire().await.unwrap().forget();
                activity.changed().await.unwrap();
                assert!(activity.borrow_and_update().active > 0);
                let mut deadlines = PollDeadlines::now();
                scheduler.run_due(&mut deadlines, &mut activity).await;
                assert_eq!(&calls.lock().unwrap()[1..], [operation]);

                release.add_permits(1);
                manager_call.await.unwrap();
                activity.changed().await.unwrap();
                scheduler.run_due(&mut deadlines, &mut activity).await;
                assert_eq!(
                    &calls.lock().unwrap()[1..],
                    [operation, "router.status", "router.health"]
                );

                let generation = manager.activity().generation;
                scheduler.run_due(&mut deadlines, &mut activity).await;
                assert_eq!(manager.activity().generation, generation);
                assert_eq!(calls.lock().unwrap().len(), 4);
            }
        });
    }
}
