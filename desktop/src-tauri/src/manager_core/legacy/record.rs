//! Durable one-shot legacy migration attempt record.
//!
//! The record is written *before* the first termination signal so that a
//! crash, a relaunch, or an explicit retry within the same installation
//! generation can never signal a legacy router twice. It stores only
//! generation and process-instance fields: no paths, listen addresses,
//! credentials, or PEM material. This module is not the default sidecar
//! runtime path.

use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::super::process::SignalKind;
use super::super::state::{self, RouterState, StateError};
use super::identity::CurrentLineage;

/// Closed vocabulary; the strings are stable diagnostics, not error codes.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationOutcome {
    /// Written before signaling. A crash leaves this marker behind, and it
    /// blocks further termination just like an issued signal does.
    #[default]
    Pending,
    Stopped,
    StopTimeout,
    IdentityChanged,
    SignalRefused,
    PortReoccupied,
}

impl MigrationOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Stopped => "stopped",
            Self::StopTimeout => "stop_timeout",
            Self::IdentityChanged => "identity_changed",
            Self::SignalRefused => "signal_refused",
            Self::PortReoccupied => "port_reoccupied",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssuedSignal {
    Interrupt,
    Kill,
}

impl IssuedSignal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interrupt => "interrupt",
            Self::Kill => "kill",
        }
    }
}

impl From<SignalKind> for IssuedSignal {
    fn from(kind: SignalKind) -> Self {
        match kind {
            SignalKind::Interrupt => Self::Interrupt,
            SignalKind::Kill => Self::Kill,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct MigrationRecord {
    #[serde(default)]
    pub installation_id: String,
    #[serde(default)]
    pub package_generation: i32,
    #[serde(default)]
    pub source_generation: i32,
    #[serde(default)]
    pub source_protocol: String,
    #[serde(default)]
    pub target_pid: i32,
    #[serde(default)]
    pub target_started_at: String,
    #[serde(default)]
    pub attempted_at: String,
    /// Only signals the host actually accepted. A refused signal is recorded
    /// through `outcome`, never here.
    #[serde(default)]
    pub signals: Vec<IssuedSignal>,
    #[serde(default)]
    pub outcome: MigrationOutcome,
}

impl MigrationRecord {
    pub fn pending(current: &CurrentLineage, target: &RouterState, now: DateTime<Utc>) -> Self {
        Self {
            installation_id: current.installation_id.clone(),
            package_generation: current.package_generation,
            source_generation: target.package_generation,
            source_protocol: target.management_protocol_version.clone(),
            target_pid: target.pid,
            target_started_at: target.process_started_at.clone(),
            attempted_at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
            signals: Vec::new(),
            outcome: MigrationOutcome::Pending,
        }
    }

    pub fn for_generation(&self, current: &CurrentLineage) -> bool {
        self.installation_id == current.installation_id
            && self.package_generation == current.package_generation
    }

    /// True once this generation may already have terminated a router: either
    /// a signal was accepted or the attempt never reached a final outcome.
    pub fn blocks_termination(&self) -> bool {
        self.outcome == MigrationOutcome::Pending || !self.signals.is_empty()
    }

    pub fn issued(&mut self, kind: SignalKind) {
        self.signals.push(kind.into());
    }

    pub fn diagnostic_line(&self) -> String {
        let signals = if self.signals.is_empty() {
            "none".to_owned()
        } else {
            self.signals
                .iter()
                .map(|signal| signal.as_str())
                .collect::<Vec<_>>()
                .join(",")
        };
        format!(
            "legacy_migration generation={} source_generation={} source_protocol={} pid={} signals={} outcome={} attempted_at={}",
            self.package_generation,
            self.source_generation,
            self.source_protocol,
            self.target_pid,
            signals,
            self.outcome.as_str(),
            self.attempted_at
        )
    }
}

pub const CORRUPT_RECORD_LINE: &str = "legacy_migration record=corrupt";

/// Missing is `Ok(None)`. Unreadable or malformed records are errors so the
/// caller fails closed instead of assuming no signal was ever issued.
pub fn read(path: &Path) -> Result<Option<MigrationRecord>, StateError> {
    match state::read_json::<MigrationRecord>(path) {
        Ok(record) => Ok(Some(record)),
        Err(StateError::NotFound) => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn write(path: &Path, record: &MigrationRecord) -> Result<(), StateError> {
    state::write_json(path, record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager_core::process::Identity as ProcessIdentity;
    use std::fs;
    use std::path::PathBuf;

    fn current() -> CurrentLineage {
        CurrentLineage {
            session_id: "s".into(),
            installation_id: "install-a".into(),
            package_generation: 2,
            deployment_id: "d".into(),
            management_protocol_version: "4".into(),
            manager: ProcessIdentity::default(),
        }
    }

    fn target() -> RouterState {
        RouterState {
            pid: 41018,
            listen_addr: "127.0.0.1:19099".into(),
            binary_path: "/Users/someone/Applications/mtls-router".into(),
            process_started_at: "2026-09-06T00:00:00.000000000Z".into(),
            process_executable: "/Users/someone/Applications/mtls-router".into(),
            owner: "desktop".into(),
            management_protocol_version: "1".into(),
            ..RouterState::default()
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mtls-legacy-record-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join("legacy-migration.json")
    }

    #[test]
    fn pending_record_round_trips_and_omits_paths() {
        let now = DateTime::parse_from_rfc3339("2026-09-07T01:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut record = MigrationRecord::pending(&current(), &target(), now);
        record.issued(SignalKind::Interrupt);
        record.issued(SignalKind::Kill);
        record.outcome = MigrationOutcome::StopTimeout;

        let path = temp_path("roundtrip");
        write(&path, &record).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("Applications"), "{raw}");
        assert!(!raw.contains("19099"), "{raw}");
        assert_eq!(read(&path).unwrap(), Some(record.clone()));
        assert_eq!(
            record.diagnostic_line(),
            "legacy_migration generation=2 source_generation=0 source_protocol=1 pid=41018 signals=interrupt,kill outcome=stop_timeout attempted_at=2026-09-07T01:30:00Z"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn blocking_rule_covers_pending_and_issued_signals_only() {
        let mut record = MigrationRecord::pending(&current(), &target(), Utc::now());
        assert!(record.blocks_termination(), "pending marker blocks");

        record.outcome = MigrationOutcome::SignalRefused;
        assert!(
            !record.blocks_termination(),
            "refused without issue is free"
        );

        record.outcome = MigrationOutcome::IdentityChanged;
        assert!(!record.blocks_termination());

        record.issued(SignalKind::Interrupt);
        assert!(record.blocks_termination(), "an accepted signal blocks");
        record.outcome = MigrationOutcome::Stopped;
        assert!(record.blocks_termination());
        assert_eq!(
            record.diagnostic_line().split(' ').nth(5),
            Some("signals=interrupt")
        );
    }

    #[test]
    fn other_installation_or_generation_never_matches() {
        let record = MigrationRecord::pending(&current(), &target(), Utc::now());
        assert!(record.for_generation(&current()));
        let mut newer = current();
        newer.package_generation = 3;
        assert!(!record.for_generation(&newer));
        let mut other = current();
        other.installation_id = "install-b".into();
        assert!(!record.for_generation(&other));
    }

    #[test]
    fn missing_is_none_and_corrupt_is_an_error() {
        let path = temp_path("missing");
        assert_eq!(read(&path), Ok(None));
        fs::write(&path, b"{\"outcome\":\"pending\"").unwrap();
        assert_eq!(read(&path), Err(StateError::Corrupt));
        fs::write(&path, b"{}").unwrap();
        let sparse = read(&path).unwrap().unwrap();
        assert_eq!(sparse.outcome, MigrationOutcome::Pending);
        assert!(!sparse.for_generation(&current()));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
