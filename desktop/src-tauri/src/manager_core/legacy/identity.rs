//! Legacy installation/generation identity records.
//!
//! Frozen against Go `internal/manager/lifecycle` and `internal/manager/discovery`
//! installation-lineage predicates, plus the release goldens in
//! `internal/manager/testdata`. This module only parses identity and does not
//! load secrets. Live process validate/stop lives in `stop`. It is not the
//! default sidecar runtime path.

use std::path::Path;

use serde::Deserialize;

use crate::protocol::is_legacy_lineage_version;

use super::super::process::Identity as ProcessIdentity;
use super::super::state::{self, RouterState, StateError};

/// Current package installation/generation used to classify a recorded router.
///
/// Sidecar hashes, API keys, and TLS material are not part of this record and
/// never authorize protocol 1/3 migration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CurrentLineage {
    pub session_id: String,
    pub installation_id: String,
    pub package_generation: i32,
    pub deployment_id: String,
    pub management_protocol_version: String,
    pub manager: ProcessIdentity,
}

impl CurrentLineage {
    pub const DEFAULT_PACKAGE_GENERATION: i32 = 1;

    /// Matches Go `lifecycle.New` defaults for generation and protocol.
    pub fn with_lifecycle_defaults(mut self) -> Self {
        if self.package_generation <= 0 {
            self.package_generation = Self::DEFAULT_PACKAGE_GENERATION;
        }
        if self.management_protocol_version.is_empty() {
            self.management_protocol_version =
                crate::protocol::MANAGEMENT_PROTOCOL_VERSION.to_owned();
        }
        self
    }
}

/// Installation, generation, and process identity taken from a recorded
/// router state. Credentials, PEM material, and sidecar hashes are omitted.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordedIdentity {
    pub owner: String,
    pub listen_addr: String,
    pub binary_path: String,
    pub log_path: String,
    pub desktop_session_id: String,
    pub installation_id: String,
    pub package_generation: i32,
    pub deployment_id: String,
    pub management_protocol_version: String,
    pub manager_version: String,
    pub router_version: String,
    pub router: ProcessIdentity,
    pub previous_manager: ProcessIdentity,
}

impl RecordedIdentity {
    pub fn from_state(value: &RouterState) -> Self {
        Self {
            owner: value.owner.clone(),
            listen_addr: value.listen_addr.clone(),
            binary_path: value.binary_path.clone(),
            log_path: value.log_path.clone(),
            desktop_session_id: value.desktop_session_id.clone(),
            installation_id: value.installation_id.clone(),
            package_generation: value.package_generation,
            deployment_id: value.deployment_id.clone(),
            management_protocol_version: value.management_protocol_version.clone(),
            manager_version: value.manager_version.clone(),
            router_version: value.router_version.clone(),
            router: router_identity(value),
            previous_manager: manager_identity(value),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct InstallationLineageFile {
    #[serde(default)]
    installation_id: String,
    #[serde(default)]
    package_generation: i32,
    #[serde(default)]
    deployment_id: String,
    #[serde(default)]
    management_protocol_version: String,
}

pub fn router_identity(value: &RouterState) -> ProcessIdentity {
    ProcessIdentity {
        pid: value.pid,
        started_at: value.process_started_at.clone(),
        executable: value.process_executable.clone(),
    }
}

pub fn manager_identity(value: &RouterState) -> ProcessIdentity {
    ProcessIdentity {
        pid: value.manager_pid,
        started_at: value.manager_process_started_at.clone(),
        executable: value.manager_process_executable.clone(),
    }
}

pub fn complete_process(value: &ProcessIdentity) -> bool {
    value.pid > 0 && !value.started_at.is_empty() && !value.executable.is_empty()
}

pub fn manager_matches(value: &RouterState, identity: &ProcessIdentity) -> bool {
    value.manager_pid == identity.pid
        && value.manager_process_started_at == identity.started_at
        && value.manager_process_executable == identity.executable
}

/// Go `lifecycle.completeDesktopState`.
pub fn complete_desktop_state(value: &RouterState) -> bool {
    value.pid > 0
        && !value.listen_addr.is_empty()
        && !value.binary_path.is_empty()
        && !value.log_path.is_empty()
        && !value.process_started_at.is_empty()
        && !value.process_executable.is_empty()
        && !value.desktop_session_id.is_empty()
        && !value.installation_id.is_empty()
        && value.manager_pid > 0
        && !value.manager_process_started_at.is_empty()
        && !value.manager_process_executable.is_empty()
        && !value.manager_version.is_empty()
        && !value.router_version.is_empty()
        && !value.deployment_id.is_empty()
        && !value.management_protocol_version.is_empty()
}

pub fn installation_matches(current: &CurrentLineage, value: &RouterState) -> bool {
    !current.installation_id.is_empty() && value.installation_id == current.installation_id
}

pub fn generation_compatible(current: &CurrentLineage, value: &RouterState) -> bool {
    value.deployment_id == current.deployment_id
        && value.management_protocol_version == current.management_protocol_version
        && value.package_generation > 0
        && current.package_generation > 0
        && value.package_generation == current.package_generation
}

/// Go `lifecycle.supportedLegacySource` / discovery `supportedLegacyLineage`
/// installation half: no current installation fields, protocol 1 or 3 only.
pub fn supported_legacy_source(value: &RouterState) -> bool {
    if value.package_generation != 0 || !value.installation_id.is_empty() {
        return false;
    }
    if !is_legacy_lineage_version(&value.management_protocol_version) {
        return false;
    }
    complete_process(&router_identity(value)) && complete_process(&manager_identity(value))
}

pub fn correlated_installation(current: &CurrentLineage, value: &RouterState) -> bool {
    if value.installation_id.is_empty() {
        return supported_legacy_source(value);
    }
    value.package_generation > 0 && installation_matches(current, value)
}

pub fn current_owned(current: &CurrentLineage, value: &RouterState) -> bool {
    value.owner == "desktop"
        && complete_process(&router_identity(value))
        && value.desktop_session_id == current.session_id
        && manager_matches(value, &current.manager)
        && installation_matches(current, value)
        && generation_compatible(current, value)
}

/// Static half of Go `reclaimable`.
pub fn record_allows_reclaim(current: &CurrentLineage, value: &RouterState) -> bool {
    value.owner == "desktop"
        && complete_desktop_state(value)
        && installation_matches(current, value)
        && generation_compatible(current, value)
}

/// Static half of Go `migratable`.
pub fn record_allows_migration(current: &CurrentLineage, value: &RouterState) -> bool {
    if value.owner != "desktop"
        || !complete_process(&router_identity(value))
        || value.binary_path.is_empty()
        || !correlated_installation(current, value)
    {
        return false;
    }
    if generation_compatible(current, value) && !value.installation_id.is_empty() {
        return false;
    }
    true
}

pub fn parse_record(path: impl AsRef<Path>) -> Result<RecordedIdentity, StateError> {
    state::read(path).map(|value| RecordedIdentity::from_state(&value))
}

pub fn parse_current_installation(path: impl AsRef<Path>) -> Result<CurrentLineage, StateError> {
    let bytes = std::fs::read(path.as_ref()).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => StateError::NotFound,
        std::io::ErrorKind::PermissionDenied => StateError::PermissionDenied,
        _ => StateError::Io,
    })?;
    parse_current_installation_bytes(&bytes)
}

pub fn parse_current_installation_bytes(bytes: &[u8]) -> Result<CurrentLineage, StateError> {
    let bytes = strip_bom(bytes);
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let value = InstallationLineageFile::deserialize(&mut de).map_err(|_| StateError::Corrupt)?;
    de.end().map_err(|_| StateError::Corrupt)?;
    Ok(CurrentLineage {
        installation_id: value.installation_id,
        package_generation: value.package_generation,
        deployment_id: value.deployment_id,
        management_protocol_version: value.management_protocol_version,
        ..CurrentLineage::default()
    })
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    const RELEASE_V018: &str = "desktop-state-v0.1.8.json";
    const RELEASE_V020: &str = "desktop-state-v0.2.0.json";

    fn release_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../internal/manager/testdata")
            .join(name)
    }

    fn load_release(name: &str) -> RouterState {
        let value = state::read(release_path(name)).expect("release golden");
        assert!(
            value.installation_id.is_empty() && value.package_generation == 0,
            "release fixture unexpectedly contains current installation lineage: {value:?}"
        );
        value
    }

    fn current_v4() -> CurrentLineage {
        CurrentLineage {
            session_id: "current-session".into(),
            installation_id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".into(),
            package_generation: 1,
            deployment_id: "current-production".into(),
            management_protocol_version: "4".into(),
            manager: ProcessIdentity {
                pid: 9,
                started_at: "2026-09-06T00:00:00.000000000Z".into(),
                executable: "/manager".into(),
            },
        }
    }

    fn installation_aware(pid: i32) -> RouterState {
        RouterState {
            pid,
            listen_addr: "http://127.0.0.1:19099".into(),
            binary_path: "/router".into(),
            log_path: "/router.log".into(),
            process_started_at: "router-start".into(),
            process_executable: "/router".into(),
            owner: "desktop".into(),
            desktop_session_id: "previous-session".into(),
            installation_id: current_v4().installation_id,
            package_generation: 1,
            manager_pid: 6,
            manager_process_started_at: "manager-old".into(),
            manager_process_executable: "/manager".into(),
            manager_version: "v1".into(),
            router_version: "v1".into(),
            deployment_id: current_v4().deployment_id,
            management_protocol_version: current_v4().management_protocol_version,
            ..RouterState::default()
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mtls-legacy-identity-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn release_goldens_are_supported_legacy_sources() {
        let current = current_v4();
        for (name, protocol, pid, manager_pid) in [
            (RELEASE_V018, "1", 41018, 41017),
            (RELEASE_V020, "3", 42018, 42017),
        ] {
            let value = load_release(name);
            let record = RecordedIdentity::from_state(&value);
            assert_eq!(record.management_protocol_version, protocol, "{name}");
            assert_eq!(record.router.pid, pid, "{name}");
            assert_eq!(record.previous_manager.pid, manager_pid, "{name}");
            assert_eq!(record.owner, "desktop", "{name}");
            assert!(record.installation_id.is_empty(), "{name}");
            assert_eq!(record.package_generation, 0, "{name}");
            assert!(supported_legacy_source(&value), "{name}");
            assert!(correlated_installation(&current, &value), "{name}");
            assert!(!generation_compatible(&current, &value), "{name}");
            assert!(!current_owned(&current, &value), "{name}");
            assert!(!record_allows_reclaim(&current, &value), "{name}");
            assert!(record_allows_migration(&current, &value), "{name}");
            assert!(!format!("{record:?}").contains("BEGIN"), "{name}");
        }
    }

    #[test]
    fn parse_record_reads_release_golden_without_secrets() {
        let record = parse_record(release_path(RELEASE_V018)).unwrap();
        assert_eq!(record.management_protocol_version, "1");
        assert_eq!(
            record.router.executable,
            r"C:\Program Files\CodeasierRouter\mtls-router.exe"
        );
    }

    #[test]
    fn parse_drops_secret_fields_and_does_not_read_secret_files() {
        let dir = temp_dir("secrets");
        fs::create_dir_all(dir.join("secrets")).unwrap();
        fs::write(
            dir.join("secrets").join("client.key"),
            "SECRET-KEY-MATERIAL",
        )
        .unwrap();
        fs::write(
            dir.join("credentials.json"),
            r#"{"api_key":"sk-live-secret"}"#,
        )
        .unwrap();

        let mut extra = serde_json::to_value(load_release(RELEASE_V020)).unwrap();
        extra["api_key"] = serde_json::json!("sk-state-secret");
        extra["client_key"] = serde_json::json!("-----BEGIN PRIVATE KEY-----");
        extra["upstream"] = serde_json::json!("https://secret.example");
        extra["credentials_path"] =
            serde_json::json!(dir.join("credentials.json").to_string_lossy());
        let path = dir.join("desktop-state.json");
        fs::write(&path, serde_json::to_vec(&extra).unwrap()).unwrap();

        let record = parse_record(&path).unwrap();
        let rendered = format!("{record:?}");
        assert_eq!(record.management_protocol_version, "3");
        assert!(record_allows_migration(
            &current_v4(),
            &state::read(&path).unwrap()
        ));
        assert!(!rendered.contains("sk-"));
        assert!(!rendered.contains("BEGIN PRIVATE"));
        assert!(!rendered.contains("secret.example"));
        assert!(!rendered.contains("credentials.json"));
        assert_eq!(
            fs::read_to_string(dir.join("secrets").join("client.key")).unwrap(),
            "SECRET-KEY-MATERIAL"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn installation_hashes_do_not_authorize_legacy_lineage() {
        let current = parse_current_installation_bytes(
            br#"{
                "schema_version": 1,
                "installation_id": "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
                "package_generation": 1,
                "deployment_id": "current-production",
                "management_protocol_version": "4",
                "manager_sidecar_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "router_sidecar_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "api_key": "sk-install-secret"
            }"#,
        )
        .unwrap();
        assert_eq!(current.installation_id, current_v4().installation_id);
        assert_eq!(current.package_generation, 1);
        assert!(!format!("{current:?}").contains("sk-"));
        assert!(!format!("{current:?}").contains(&"a".repeat(64)));

        let legacy = load_release(RELEASE_V018);
        assert!(supported_legacy_source(&legacy));
        assert!(record_allows_migration(&current, &legacy));
        assert!(!generation_compatible(&current, &legacy));
    }

    #[test]
    fn installation_aware_generation_zero_cannot_reclaim_or_migrate() {
        let current = current_v4();
        let mut value = installation_aware(101);
        value.package_generation = 0;
        assert!(!correlated_installation(&current, &value));
        assert!(!supported_legacy_source(&value));
        assert!(!record_allows_reclaim(&current, &value));
        assert!(!record_allows_migration(&current, &value));
    }

    #[test]
    fn compatible_generation_is_reclaimable_not_migratable() {
        let current = current_v4();
        let value = installation_aware(101);
        assert!(correlated_installation(&current, &value));
        assert!(generation_compatible(&current, &value));
        assert!(record_allows_reclaim(&current, &value));
        assert!(!record_allows_migration(&current, &value));
        assert!(!current_owned(&current, &value));

        let mut owned = value.clone();
        owned.desktop_session_id = current.session_id.clone();
        owned.manager_pid = current.manager.pid;
        owned.manager_process_started_at = current.manager.started_at.clone();
        owned.manager_process_executable = current.manager.executable.clone();
        assert!(current_owned(&current, &owned));
        assert!(!record_allows_migration(&current, &owned));
    }

    #[test]
    fn same_install_incompatible_generation_is_migratable_record() {
        let current = current_v4();
        let mut value = installation_aware(101);
        value.package_generation = 2;
        assert!(correlated_installation(&current, &value));
        assert!(!generation_compatible(&current, &value));
        assert!(!record_allows_reclaim(&current, &value));
        assert!(record_allows_migration(&current, &value));
    }

    #[test]
    fn incomplete_or_foreign_records_fail_closed() {
        let current = current_v4();
        let mut pid_only = load_release(RELEASE_V018);
        pid_only.process_started_at.clear();
        pid_only.process_executable.clear();
        pid_only.binary_path.clear();
        assert!(!supported_legacy_source(&pid_only));
        assert!(!record_allows_migration(&current, &pid_only));

        let mut missing_manager = load_release(RELEASE_V020);
        missing_manager.manager_process_started_at.clear();
        assert!(!supported_legacy_source(&missing_manager));
        assert!(!record_allows_migration(&current, &missing_manager));

        let mut protocol_two = load_release(RELEASE_V018);
        protocol_two.management_protocol_version = "2".into();
        assert!(!supported_legacy_source(&protocol_two));
        assert!(!record_allows_migration(&current, &protocol_two));

        let mut other_install = installation_aware(101);
        other_install.installation_id = "ffffffff-bbbb-4ccc-8ddd-eeeeeeeeeeee".into();
        other_install.management_protocol_version = "3".into();
        assert!(!correlated_installation(&current, &other_install));
        assert!(!record_allows_migration(&current, &other_install));

        let mut cli = load_release(RELEASE_V018);
        cli.owner = "cli".into();
        assert!(supported_legacy_source(&cli));
        assert!(!record_allows_migration(&current, &cli));
    }

    #[test]
    fn current_lineage_defaults_match_go_new() {
        let got = CurrentLineage::default().with_lifecycle_defaults();
        assert_eq!(
            got.package_generation,
            CurrentLineage::DEFAULT_PACKAGE_GENERATION
        );
        assert_eq!(
            got.management_protocol_version,
            crate::protocol::MANAGEMENT_PROTOCOL_VERSION
        );
    }

    #[test]
    fn parse_current_installation_rejects_trailing_json() {
        assert_eq!(
            parse_current_installation_bytes(br#"{"installation_id":"a"}{"installation_id":"b"}"#)
                .unwrap_err(),
            StateError::Corrupt
        );
        let missing = std::env::temp_dir().join(format!(
            "mtls-legacy-missing-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        assert_eq!(
            parse_current_installation(&missing).unwrap_err(),
            StateError::NotFound
        );
    }
}
