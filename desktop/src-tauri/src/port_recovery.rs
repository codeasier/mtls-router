use crate::types::RouterStatus;
use serde::Serialize;
use std::time::{Duration, Instant};

const RELEASE_OBSERVATION_DURATION: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseObservationState {
    Observing,
    Released,
    Reoccupied,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReleaseObservation {
    pub state: ReleaseObservationState,
}

#[derive(Clone, Debug)]
struct ActiveRecovery {
    manager_session_epoch: u64,
    started: Instant,
    state: ReleaseObservationState,
    confirmed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct PortRecovery {
    active: Option<ActiveRecovery>,
}

impl PortRecovery {
    pub fn begin(&mut self, manager_session_epoch: u64, started: Instant) {
        self.active = Some(ActiveRecovery {
            manager_session_epoch,
            started,
            state: ReleaseObservationState::Observing,
            confirmed: false,
        });
    }

    pub fn observe(
        &mut self,
        status: &RouterStatus,
        manager_session_epoch: u64,
        now: Instant,
    ) -> Option<ReleaseObservationState> {
        let active = self.active.as_mut()?;
        if active.manager_session_epoch != manager_session_epoch {
            self.active = None;
            return None;
        }

        match status.state.as_str() {
            "absent" => {
                active.confirmed = true;
                if active.state == ReleaseObservationState::Observing
                    && now.saturating_duration_since(active.started) >= RELEASE_OBSERVATION_DURATION
                {
                    active.state = ReleaseObservationState::Released;
                }
                Some(active.state)
            }
            "unknown_occupant" => {
                active.confirmed = true;
                active.state = ReleaseObservationState::Reoccupied;
                Some(active.state)
            }
            _ => {
                self.active = None;
                None
            }
        }
    }

    pub fn cancel(&mut self) {
        self.active = None;
    }

    pub fn cancel_if_epoch_changed(&mut self, manager_session_epoch: u64) -> bool {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.manager_session_epoch != manager_session_epoch)
        {
            self.active = None;
            return true;
        }
        false
    }

    pub fn projection(&self) -> Option<ReleaseObservation> {
        self.active
            .as_ref()
            .filter(|active| active.confirmed)
            .map(|active| ReleaseObservation {
                state: active.state,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::RouterStatus;
    use std::time::{Duration, Instant};

    fn status(state: &str) -> RouterStatus {
        RouterStatus {
            state: state.to_owned(),
            ..RouterStatus::default()
        }
    }

    #[test]
    fn only_absence_advances_the_ten_second_release_window() {
        let started = Instant::now();
        let mut recovery = PortRecovery::default();
        recovery.begin(7, started);
        assert_eq!(recovery.projection(), None);

        assert_eq!(
            recovery.observe(&status("absent"), 7, started + Duration::from_secs(2)),
            Some(ReleaseObservationState::Observing)
        );
        assert_eq!(
            recovery.observe(&status("absent"), 7, started + Duration::from_secs(10)),
            Some(ReleaseObservationState::Released)
        );
        assert_eq!(
            recovery.observe(&status("absent"), 7, started + Duration::from_secs(11)),
            Some(ReleaseObservationState::Released)
        );
    }

    #[test]
    fn reports_reoccupation_and_keeps_terminal_states_monotonic() {
        let started = Instant::now();
        let mut recovery = PortRecovery::default();
        recovery.begin(7, started);

        assert_eq!(
            recovery.observe(
                &status("unknown_occupant"),
                7,
                started + Duration::from_secs(4)
            ),
            Some(ReleaseObservationState::Reoccupied)
        );
        assert_eq!(
            recovery.observe(&status("absent"), 7, started + Duration::from_secs(10)),
            Some(ReleaseObservationState::Reoccupied)
        );
    }

    #[test]
    fn non_absent_statuses_cancel_without_reusing_elapsed_time() {
        let started = Instant::now();

        for state in [
            "desktop_owned",
            "external_compatible",
            "degraded",
            "stale",
            "start_failed",
            "stopping",
            "unexpected",
        ] {
            let mut recovery = PortRecovery::default();
            recovery.begin(7, started);
            assert_eq!(
                recovery.observe(&status("absent"), 7, started + Duration::from_secs(9)),
                Some(ReleaseObservationState::Observing),
                "state {state}"
            );

            assert_eq!(
                recovery.observe(&status(state), 7, started + Duration::from_secs(10)),
                None,
                "state {state}"
            );
            assert_eq!(recovery.projection(), None, "state {state}");
            assert_eq!(
                recovery.observe(&status("absent"), 7, started + Duration::from_secs(11)),
                None,
                "state {state}"
            );
        }
    }

    #[test]
    fn unknown_occupant_replaces_a_released_projection() {
        let started = Instant::now();
        let mut recovery = PortRecovery::default();
        recovery.begin(7, started);

        assert_eq!(
            recovery.observe(&status("absent"), 7, started + Duration::from_secs(10)),
            Some(ReleaseObservationState::Released)
        );
        assert_eq!(
            recovery.observe(
                &status("unknown_occupant"),
                7,
                started + Duration::from_secs(11)
            ),
            Some(ReleaseObservationState::Reoccupied)
        );
    }

    #[test]
    fn clears_on_epoch_change_available_router_or_explicit_cancel() {
        let started = Instant::now();
        let mut recovery = PortRecovery::default();

        recovery.begin(7, started);
        assert!(recovery.cancel_if_epoch_changed(8));
        assert_eq!(recovery.projection(), None);

        recovery.begin(8, started);
        assert_eq!(
            recovery.observe(
                &status("desktop_owned"),
                8,
                started + Duration::from_secs(1)
            ),
            None
        );

        recovery.begin(8, started);
        recovery.cancel();
        assert_eq!(recovery.projection(), None);
    }
}
