use crate::error::CommandError;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

pub const RING_CAPACITY: usize = 8;
const STDERR_RECORD_LIMIT: usize = 4 * 1024;
const BOOTSTRAP_SCHEMA_VERSION: u32 = 1;
const BOOTSTRAP_KIND: &str = "manager_bootstrap_failure";

pub const STAGE_SIDECAR_RESOLUTION: &str = "sidecar_resolution";
pub const STAGE_SPAWN: &str = "spawn";
pub const STAGE_HANDSHAKE: &str = "handshake";
pub const STAGE_PROTOCOL_PARSE: &str = "protocol_parse";
pub const STAGE_WATCHDOG_TIMEOUT: &str = "watchdog_timeout";
pub const STAGE_UNEXPECTED_EXIT: &str = "unexpected_exit";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerDiagnostic {
    pub stage: String,
    pub code: String,
}

#[derive(Default)]
struct DiagnosticState {
    events: VecDeque<ManagerDiagnostic>,
    stderr_buffer: Vec<u8>,
    stderr_overflowed: bool,
}

#[derive(Clone, Default)]
pub struct ManagerDiagnosticRing {
    state: Arc<Mutex<DiagnosticState>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapRecord {
    schema_version: u32,
    kind: String,
    stage: String,
    code: String,
}

impl ManagerDiagnosticRing {
    pub fn record(&self, stage: impl Into<String>, code: impl Into<String>) {
        let event = ManagerDiagnostic {
            stage: sanitize_stage(stage.into()),
            code: sanitize_code(code.into()),
        };
        let mut state = self.state.lock().unwrap();
        if state.events.len() == RING_CAPACITY {
            state.events.pop_front();
        }
        state.events.push_back(event.clone());
        eprintln!(
            "CodeasierRouter manager: {}",
            format!("stage={} code={}", event.stage, event.code)
        );
    }

    pub fn record_error(&self, stage: &'static str, error: &CommandError) {
        self.record(stage, error.code.clone());
    }

    // Manager stderr is untrusted. Only one closed, bounded JSON record format
    // is accepted; all raw text, paths, and malformed payloads are discarded.
    pub fn ingest_stderr(&self, bytes: &[u8]) -> bool {
        let mut completed = Vec::new();
        {
            let mut state = self.state.lock().unwrap();
            for &byte in bytes {
                if byte == b'\n' {
                    if !state.stderr_overflowed && !state.stderr_buffer.is_empty() {
                        completed.push(std::mem::take(&mut state.stderr_buffer));
                    } else {
                        state.stderr_buffer.clear();
                    }
                    state.stderr_overflowed = false;
                    continue;
                }
                if state.stderr_overflowed {
                    continue;
                }
                if state.stderr_buffer.len() == STDERR_RECORD_LIMIT {
                    state.stderr_buffer.clear();
                    state.stderr_overflowed = true;
                    continue;
                }
                state.stderr_buffer.push(byte);
            }
        }
        let mut accepted = false;
        for line in completed {
            let Ok(record) = serde_json::from_slice::<BootstrapRecord>(&line) else {
                continue;
            };
            if record.schema_version != BOOTSTRAP_SCHEMA_VERSION
                || record.kind != BOOTSTRAP_KIND
                || !known_stage(&record.stage)
            {
                continue;
            }
            self.record(record.stage, record.code);
            accepted = true;
        }
        accepted
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> Vec<ManagerDiagnostic> {
        self.state.lock().unwrap().events.iter().cloned().collect()
    }

    pub fn last(&self) -> Option<ManagerDiagnostic> {
        self.state.lock().unwrap().events.back().cloned()
    }
}

pub fn stage_for_code(code: &str) -> &'static str {
    match code {
        "SIDECAR_MISSING" | "SIDECAR_INVALID" | "INSTALLATION_INVALID" => STAGE_SIDECAR_RESOLUTION,
        "INVALID_RESPONSE" => STAGE_PROTOCOL_PARSE,
        "OPERATION_TIMEOUT" => STAGE_WATCHDOG_TIMEOUT,
        _ => STAGE_UNEXPECTED_EXIT,
    }
}

fn known_stage(stage: &str) -> bool {
    matches!(
        stage,
        STAGE_SIDECAR_RESOLUTION
            | STAGE_SPAWN
            | STAGE_HANDSHAKE
            | STAGE_PROTOCOL_PARSE
            | STAGE_WATCHDOG_TIMEOUT
            | STAGE_UNEXPECTED_EXIT
    )
}

fn sanitize_stage(stage: String) -> String {
    if known_stage(&stage) {
        stage
    } else {
        STAGE_UNEXPECTED_EXIT.to_owned()
    }
}

fn sanitize_code(code: String) -> String {
    if code.len() <= 64
        && code
            .bytes()
            .all(|value| value.is_ascii_uppercase() || value.is_ascii_digit() || value == b'_')
    {
        code
    } else {
        "MANAGER_FAILED".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_is_bounded_and_omits_payloads() {
        let ring = ManagerDiagnosticRing::default();
        for index in 0..12 {
            ring.record(STAGE_SPAWN, format!("CODE_{index}"));
        }
        let events = ring.snapshot();
        assert_eq!(events.len(), RING_CAPACITY);
        assert_eq!(events[0].code, "CODE_4");
        assert!(events.iter().all(|event| event.stage == STAGE_SPAWN));
    }

    #[test]
    fn rejects_untrusted_error_codes() {
        let ring = ManagerDiagnosticRing::default();
        ring.record(STAGE_HANDSHAKE, "not a code /tmp/secret");
        assert_eq!(ring.last().unwrap().code, "MANAGER_FAILED");
    }

    #[test]
    fn accepts_only_chunked_closed_bootstrap_records() {
        let ring = ManagerDiagnosticRing::default();
        ring.ingest_stderr(
            br#"{"schema_version":1,"kind":"manager_bootstrap_failure","stage":"hand"#,
        );
        assert!(ring.last().is_none());
        ring.ingest_stderr(
            br#"shake","code":"MANAGER_BOOTSTRAP_FAILED"}
"#,
        );
        assert_eq!(
            ring.last(),
            Some(ManagerDiagnostic {
                stage: STAGE_HANDSHAKE.to_owned(),
                code: "MANAGER_BOOTSTRAP_FAILED".to_owned(),
            })
        );

        ring.ingest_stderr(b"raw path /tmp/private and api_key=secret\n");
        assert_eq!(ring.snapshot().len(), 1);
        ring.ingest_stderr(
            br#"{"schema_version":1,"kind":"manager_bootstrap_failure","stage":"unknown","code":"BAD"}
"#,
        );
        assert_eq!(ring.snapshot().len(), 1);
    }

    #[test]
    fn discards_oversized_stderr_record() {
        let ring = ManagerDiagnosticRing::default();
        ring.ingest_stderr(&vec![b'x'; STDERR_RECORD_LIMIT + 100]);
        ring.ingest_stderr(b"\n");
        assert!(ring.last().is_none());
    }
}
