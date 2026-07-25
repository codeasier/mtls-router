use crate::{
    error::Result,
    manager::ManagerClient,
    scheduler::PollScheduler,
    types::{RouterHealth, RouterStatus, RouterVersion},
};
use serde_json::json;

pub async fn start(manager: &ManagerClient, scheduler: &PollScheduler) -> Result<RouterStatus> {
    scheduler.request_refresh();
    match manager
        .call::<RouterStatus>("router.start", json!({ "owner": "desktop" }))
        .await
    {
        Ok(status) => {
            scheduler.set_status(status.clone()).await;
            Ok(status)
        }
        Err(error) if error.code == "ROUTER_DEGRADED" => {
            match manager
                .call::<RouterStatus>("router.status", json!({}))
                .await
            {
                Ok(status) if status.available() => {
                    scheduler.set_status(status.clone()).await;
                    Ok(status)
                }
                _ => {
                    scheduler.set_status_error(&error).await;
                    scheduler.request_refresh();
                    Err(error)
                }
            }
        }
        Err(error) => {
            match manager
                .call::<RouterStatus>("router.status", json!({}))
                .await
            {
                Ok(status) => scheduler.set_status(status).await,
                Err(_) => scheduler.set_status_error(&error).await,
            }
            scheduler.request_refresh();
            Err(error)
        }
    }
}

pub async fn stop(manager: &ManagerClient, scheduler: &PollScheduler) -> Result<RouterStatus> {
    scheduler.request_refresh();
    match manager.call::<RouterStatus>("router.stop", json!({})).await {
        Ok(status) => {
            scheduler.set_status(status.clone()).await;
            Ok(status)
        }
        Err(error) => {
            scheduler.set_status_error(&error).await;
            scheduler.request_refresh();
            Err(error)
        }
    }
}

pub async fn force_terminate_occupant(
    confirmation_token: String,
    manager: &ManagerClient,
    scheduler: &PollScheduler,
) -> Result<RouterStatus> {
    scheduler.request_status_refresh();
    let status = manager
        .call::<RouterStatus>(
            "router.force_terminate_occupant",
            json!({ "confirmation_token": confirmation_token }),
        )
        .await?;
    scheduler.set_status(status.clone()).await;
    Ok(status)
}

pub async fn first_launch(
    manager: &ManagerClient,
    scheduler: &PollScheduler,
) -> Result<RouterStatus> {
    let mut status: RouterStatus = manager.call("router.status", json!({})).await?;
    if status.state == "absent" {
        status = start(manager, scheduler).await?;
    }
    scheduler.set_status(status.clone()).await;
    if status.available() {
        let _: RouterVersion = manager.call("router.version", json!({})).await?;
        let health: RouterHealth = manager.call("router.health", json!({})).await?;
        scheduler.set_health(health).await;
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{TransportChild, TransportEvent, TransportFactory, TransportSession};
    use serde_json::Value;
    use std::sync::{Arc, Mutex, OnceLock};
    use tokio::sync::mpsc;

    struct Child {
        calls: Arc<Mutex<Vec<String>>>,
        sender: mpsc::Sender<TransportEvent>,
        initial: &'static str,
        start_error: Option<&'static str>,
    }

    impl TransportChild for Child {
        fn write(&mut self, bytes: &[u8]) -> Result<()> {
            let request: Value = serde_json::from_slice(bytes).unwrap();
            let method = request["method"].as_str().unwrap().to_owned();
            self.calls.lock().unwrap().push(method.clone());
            let state = if method == "router.start" {
                "desktop_owned"
            } else {
                self.initial
            };
            if let ("router.start", Some(code)) = (method.as_str(), self.start_error) {
                let response = serde_json::to_vec(&json!({
                    "id": request["id"],
                    "error": { "code": code, "message": "router start rejected" }
                }))
                .unwrap();
                self.sender
                    .try_send(TransportEvent::Stdout(response))
                    .unwrap();
                return Ok(());
            }
            let result = match method.as_str() {
                "manager.info" => json!({
                    "version": env!("MTLS_MANAGER_VERSION"), "commit": "test", "build_date": "test",
                    "target": env!("MTLS_MANAGER_TARGET"), "deployment_id": env!("MTLS_DEPLOYMENT_ID"),
                    "management_protocol_version": env!("MTLS_MANAGEMENT_PROTOCOL_VERSION")
                }),
                "router.status" | "router.start" => {
                    json!({ "state": state, "owner": if matches!(state, "desktop_owned" | "degraded") { "desktop" } else { "external" } })
                }
                "router.force_terminate_occupant" => json!({ "state": "absent" }),
                "router.version" => {
                    json!({ "version": "router", "deployment_id": env!("MTLS_DEPLOYMENT_ID"), "management_protocol_version": env!("MTLS_MANAGEMENT_PROTOCOL_VERSION") })
                }
                "router.health" => json!({ "status": "ok", "checked_at": "2026-07-12T00:00:00Z" }),
                _ => json!({}),
            };
            let response =
                serde_json::to_vec(&json!({ "id": request["id"], "result": result })).unwrap();
            self.sender
                .try_send(TransportEvent::Stdout(response))
                .unwrap();
            Ok(())
        }

        fn kill(self: Box<Self>) {}
    }

    struct Factory {
        calls: Arc<Mutex<Vec<String>>>,
        initial: &'static str,
        start_error: Option<&'static str>,
    }

    impl TransportFactory for Factory {
        fn spawn(&self) -> Result<TransportSession> {
            let (sender, events) = mpsc::channel(16);
            Ok(TransportSession {
                child: Box::new(Child {
                    calls: self.calls.clone(),
                    sender,
                    initial: self.initial,
                    start_error: self.start_error,
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
    fn absent_router_starts_then_checks_version_and_health() {
        runtime().block_on(async {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let manager = ManagerClient::new(Arc::new(Factory {
                calls: calls.clone(),
                initial: "absent",
                start_error: None,
            }));
            let scheduler = PollScheduler::new(manager.clone());
            let status = first_launch(&manager, &scheduler).await.unwrap();
            assert_eq!(status.state, "desktop_owned");
            assert_eq!(
                &calls.lock().unwrap()[1..],
                [
                    "router.status",
                    "router.start",
                    "router.version",
                    "router.health"
                ]
            );
        });
    }

    #[test]
    fn external_router_is_reused_and_unknown_occupant_is_never_started() {
        runtime().block_on(async {
            for (state, expected) in [
                (
                    "external_compatible",
                    vec!["router.status", "router.version", "router.health"],
                ),
                ("unknown_occupant", vec!["router.status"]),
            ] {
                let calls = Arc::new(Mutex::new(Vec::new()));
                let manager = ManagerClient::new(Arc::new(Factory {
                    calls: calls.clone(),
                    initial: state,
                    start_error: None,
                }));
                let scheduler = PollScheduler::new(manager.clone());
                first_launch(&manager, &scheduler).await.unwrap();
                assert_eq!(&calls.lock().unwrap()[1..], expected);
            }
        });
    }

    #[test]
    fn degraded_start_is_reconciled_as_running_instead_of_failed() {
        runtime().block_on(async {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let manager = ManagerClient::new(Arc::new(Factory {
                calls: calls.clone(),
                initial: "degraded",
                start_error: Some("ROUTER_DEGRADED"),
            }));
            let scheduler = PollScheduler::new(manager.clone());

            let status = start(&manager, &scheduler).await.unwrap();

            assert_eq!(status.state, "degraded");
            assert_eq!(status.owner.as_deref(), Some("desktop"));
            assert_eq!(
                &calls.lock().unwrap()[1..],
                ["router.start", "router.status"]
            );
            assert_eq!(scheduler.snapshot().await.status, Some(status));
        });
    }

    #[test]
    fn failed_start_reconciles_latched_status() {
        runtime().block_on(async {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let manager = ManagerClient::new(Arc::new(Factory {
                calls: calls.clone(),
                initial: "start_failed",
                start_error: Some("ROUTER_START_FAILED"),
            }));
            let scheduler = PollScheduler::new(manager.clone());

            let error = start(&manager, &scheduler).await.unwrap_err();

            assert_eq!(error.code, "ROUTER_START_FAILED");
            assert_eq!(
                &calls.lock().unwrap()[1..],
                ["router.start", "router.status"]
            );
            assert_eq!(
                scheduler
                    .snapshot()
                    .await
                    .status
                    .as_ref()
                    .map(|status| status.state.as_str()),
                Some("start_failed")
            );
        });
    }

    #[test]
    fn force_termination_reconciles_status_without_starting_router() {
        runtime().block_on(async {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let manager = ManagerClient::new(Arc::new(Factory {
                calls: calls.clone(),
                initial: "absent",
                start_error: None,
            }));
            let scheduler = PollScheduler::new(manager.clone());

            let status =
                force_terminate_occupant("single-use-token".to_owned(), &manager, &scheduler)
                    .await
                    .unwrap();

            assert_eq!(status.state, "absent");
            assert_eq!(
                &calls.lock().unwrap()[1..],
                ["router.force_terminate_occupant"]
            );
            assert_eq!(scheduler.snapshot().await.status, Some(status));
        });
    }
}
