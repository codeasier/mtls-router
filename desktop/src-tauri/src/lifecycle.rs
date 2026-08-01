use std::{
    future::Future,
    sync::{Mutex, MutexGuard},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum QuitState {
    #[default]
    Idle,
    AwaitingConfirmation,
    WaitingForLifecycle,
    Exiting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuitAction {
    None,
    RequestConfirmation,
    WaitForLifecycle,
    ExecuteQuit,
}

#[derive(Debug, Eq, PartialEq)]
pub struct OperationOutput<T> {
    pub value: T,
    pub quit_action: QuitAction,
}

#[derive(Default)]
pub struct LifecycleState {
    inner: Mutex<LifecycleInner>,
}

#[derive(Default)]
struct LifecycleInner {
    operation_active: bool,
    draft_dirty: bool,
    quit: QuitState,
}

impl LifecycleState {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn set_draft_dirty(&self, dirty: bool) {
        self.lock().draft_dirty = dirty;
    }

    fn try_begin_operation(&self) -> bool {
        let mut inner = self.lock();
        if inner.operation_active || inner.quit != QuitState::Idle {
            return false;
        }
        inner.operation_active = true;
        true
    }

    pub async fn run_operation<F, T>(&self, operation: F) -> Option<OperationOutput<T>>
    where
        F: Future<Output = T>,
    {
        if !self.try_begin_operation() {
            return None;
        }
        let value = operation.await;
        Some(OperationOutput {
            value,
            quit_action: self.finish_operation(),
        })
    }

    fn finish_operation(&self) -> QuitAction {
        let mut inner = self.lock();
        inner.operation_active = false;
        if inner.quit == QuitState::WaitingForLifecycle {
            inner.quit = QuitState::Exiting;
            return QuitAction::ExecuteQuit;
        }
        QuitAction::None
    }

    pub fn request_quit(&self, webview_exists: bool) -> QuitAction {
        let mut inner = self.lock();
        if !webview_exists {
            inner.draft_dirty = false;
            if inner.quit == QuitState::AwaitingConfirmation {
                inner.quit = QuitState::Idle;
            }
        }
        match inner.quit {
            QuitState::Exiting | QuitState::WaitingForLifecycle => return QuitAction::None,
            QuitState::AwaitingConfirmation => return QuitAction::RequestConfirmation,
            QuitState::Idle => {}
        }
        if inner.draft_dirty {
            inner.quit = QuitState::AwaitingConfirmation;
            return QuitAction::RequestConfirmation;
        }
        if inner.operation_active {
            inner.quit = QuitState::WaitingForLifecycle;
            return QuitAction::WaitForLifecycle;
        }
        inner.quit = QuitState::Exiting;
        QuitAction::ExecuteQuit
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn resolve_quit(&self, confirmed: bool) -> QuitAction {
        let mut inner = self.lock();
        if inner.quit != QuitState::AwaitingConfirmation {
            return QuitAction::None;
        }
        if !confirmed {
            inner.quit = QuitState::Idle;
            return QuitAction::None;
        }
        if inner.operation_active {
            inner.quit = QuitState::WaitingForLifecycle;
            return QuitAction::WaitForLifecycle;
        }
        inner.quit = QuitState::Exiting;
        QuitAction::ExecuteQuit
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn reset_after_emit_failure(&self) {
        let mut inner = self.lock();
        if inner.quit == QuitState::AwaitingConfirmation {
            inner.quit = QuitState::Idle;
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_exiting(&self) -> bool {
        self.lock().quit == QuitState::Exiting
    }

    pub fn try_begin_quit_operation(&self) -> bool {
        let mut inner = self.lock();
        if inner.operation_active || inner.quit != QuitState::Exiting {
            return false;
        }
        inner.operation_active = true;
        true
    }

    pub fn prepare_restart(&self) -> bool {
        let mut inner = self.lock();
        if inner.operation_active || inner.quit != QuitState::Idle {
            return false;
        }
        inner.draft_dirty = false;
        inner.quit = QuitState::Exiting;
        true
    }

    fn lock(&self) -> MutexGuard<'_, LifecycleInner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_quit_waits_for_confirmation_and_active_lifecycle() {
        tauri::async_runtime::block_on(async {
            let state = LifecycleState::default();
            let output = state
                .run_operation(async {
                    state.set_draft_dirty(true);

                    assert_eq!(state.request_quit(true), QuitAction::RequestConfirmation);
                    assert_eq!(state.resolve_quit(true), QuitAction::WaitForLifecycle);
                })
                .await
                .unwrap();
            assert_eq!(output.quit_action, QuitAction::ExecuteQuit);
            assert!(state.is_exiting());
        });
    }

    #[test]
    fn only_one_mutating_operation_can_run() {
        tauri::async_runtime::block_on(async {
            let state = LifecycleState::default();

            let output = state
                .run_operation(async { state.run_operation(async {}).await.is_none() })
                .await
                .unwrap();
            assert!(output.value);
            assert_eq!(output.quit_action, QuitAction::None);
            assert!(state.run_operation(async {}).await.is_some());
        });
    }

    #[test]
    fn clean_quit_executes_immediately_and_ignores_recursion() {
        tauri::async_runtime::block_on(async {
            let state = LifecycleState::default();

            assert_eq!(state.request_quit(true), QuitAction::ExecuteQuit);
            assert!(state.is_exiting());
            assert_eq!(state.request_quit(true), QuitAction::None);
            assert!(state.run_operation(async {}).await.is_none());
        });
    }

    #[test]
    fn cancelling_confirmation_restores_idle_state() {
        tauri::async_runtime::block_on(async {
            let state = LifecycleState::default();
            state.set_draft_dirty(true);

            assert_eq!(state.request_quit(true), QuitAction::RequestConfirmation);
            assert_eq!(state.resolve_quit(false), QuitAction::None);
            assert!(state.run_operation(async {}).await.is_some());
        });
    }

    #[test]
    fn duplicate_confirmation_request_refocuses_and_retries_delivery() {
        let state = LifecycleState::default();
        state.set_draft_dirty(true);

        assert_eq!(state.request_quit(true), QuitAction::RequestConfirmation);
        assert_eq!(state.request_quit(true), QuitAction::RequestConfirmation);
    }

    #[test]
    fn missing_webview_clears_stale_dirty_state() {
        let state = LifecycleState::default();
        state.set_draft_dirty(true);

        assert_eq!(state.request_quit(false), QuitAction::ExecuteQuit);
    }

    #[test]
    fn webview_disappearing_during_confirmation_clears_stale_dirty_state() {
        let state = LifecycleState::default();
        state.set_draft_dirty(true);
        assert_eq!(state.request_quit(true), QuitAction::RequestConfirmation);

        assert_eq!(state.request_quit(false), QuitAction::ExecuteQuit);
        assert!(state.is_exiting());
    }

    #[test]
    fn emit_failure_allows_confirmation_to_be_retried() {
        let state = LifecycleState::default();
        state.set_draft_dirty(true);
        assert_eq!(state.request_quit(true), QuitAction::RequestConfirmation);

        state.reset_after_emit_failure();

        assert_eq!(state.request_quit(true), QuitAction::RequestConfirmation);
    }

    #[test]
    fn clean_quit_waits_for_active_lifecycle() {
        tauri::async_runtime::block_on(async {
            let state = LifecycleState::default();
            let output = state
                .run_operation(async {
                    assert_eq!(state.request_quit(true), QuitAction::WaitForLifecycle);
                })
                .await
                .unwrap();

            assert_eq!(output.quit_action, QuitAction::ExecuteQuit);
        });
    }

    #[test]
    fn quit_stop_uses_the_same_operation_gate() {
        tauri::async_runtime::block_on(async {
            let state = LifecycleState::default();
            assert_eq!(state.request_quit(true), QuitAction::ExecuteQuit);

            assert!(state.try_begin_quit_operation());
            assert!(!state.try_begin_quit_operation());
            assert!(state.run_operation(async {}).await.is_none());
        });
    }

    #[test]
    fn approved_restart_is_allowed_only_after_the_update_operation_finishes() {
        tauri::async_runtime::block_on(async {
            let state = LifecycleState::default();
            state.set_draft_dirty(true);

            let output = state
                .run_operation(async { assert!(!state.prepare_restart()) })
                .await
                .unwrap();
            assert_eq!(output.quit_action, QuitAction::None);

            assert!(state.prepare_restart());
            assert!(state.is_exiting());
            assert!(!state.prepare_restart());
            assert_eq!(state.request_quit(true), QuitAction::None);
        });
    }

    #[test]
    fn shared_runner_releases_gate_after_success_and_failure() {
        tauri::async_runtime::block_on(async {
            let state = LifecycleState::default();
            let success = state
                .run_operation(async {
                    assert!(state.run_operation(async {}).await.is_none());
                    Ok::<_, &'static str>("started")
                })
                .await
                .unwrap();
            assert_eq!(success.value, Ok("started"));
            assert_eq!(success.quit_action, QuitAction::None);

            let failure = state
                .run_operation(async { Err::<(), _>("stop failed") })
                .await
                .unwrap();
            assert_eq!(failure.value, Err("stop failed"));
            assert_eq!(failure.quit_action, QuitAction::None);

            assert!(state.run_operation(async {}).await.is_some());
        });
    }

    #[test]
    fn shared_runner_resumes_waiting_quit_after_error() {
        tauri::async_runtime::block_on(async {
            let state = LifecycleState::default();

            let output = state
                .run_operation(async {
                    assert_eq!(state.request_quit(true), QuitAction::WaitForLifecycle);
                    Err::<(), _>("start failed")
                })
                .await
                .unwrap();

            assert_eq!(output.value, Err("start failed"));
            assert_eq!(output.quit_action, QuitAction::ExecuteQuit);
        });
    }
}
