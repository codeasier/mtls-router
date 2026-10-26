use crate::{
    diagnostic_snapshot::DiagnosticSnapshot,
    error::{CommandError, Result},
};
use chrono::{DateTime, Local};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;
use zip::{write::FileOptions, CompressionMethod, ZipWriter};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub trait SaveDialog: Send + Sync {
    fn choose_zip_path(&self, default_name: &str) -> Option<PathBuf>;
}

pub struct NativeSaveDialog;

impl SaveDialog for NativeSaveDialog {
    fn choose_zip_path(&self, default_name: &str) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("ZIP", &["zip"])
            .set_file_name(default_name);
        if let Some(downloads) = dirs::download_dir() {
            dialog = dialog.set_directory(downloads);
        }
        dialog.save_file()
    }
}

pub fn default_bundle_name(now: DateTime<Local>) -> String {
    format!(
        "codeasier-router-diagnostics-{}.zip",
        now.format("%Y%m%d-%H%M%S")
    )
}

pub fn write_support_bundle(
    snapshot: &DiagnosticSnapshot,
    log_directory: &Path,
    dest: &Path,
) -> std::io::Result<()> {
    if dest.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination path is empty",
        ));
    }

    let parent = dest.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination path has no parent",
        )
    })?;
    fs::create_dir_all(parent)?;

    let tmp = parent.join(format!(".bundle-{}.tmp", Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(&tmp)?;
        let mut zip = ZipWriter::new(file);
        let zip_options = FileOptions::default().compression_method(CompressionMethod::Deflated);

        zip.start_file("last-diagnostics.json", zip_options)?;
        zip.write_all(&serde_json::to_vec_pretty(snapshot).map_err(io::Error::other)?)?;

        if log_directory.is_dir() {
            if let Ok(canonical_root) = fs::canonicalize(log_directory) {
                add_log_files(&mut zip, zip_options, &canonical_root, log_directory)?;
            }
        }

        let finished = zip.finish()?;
        finished.sync_all()?;
        #[cfg(unix)]
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
        fs::rename(&tmp, dest)?;
        #[cfg(unix)]
        fs::set_permissions(dest, fs::Permissions::from_mode(0o600))?;
        Ok(())
    })();

    if result.is_err() {
        cleanup_failed_export(&tmp);
    }
    result
}

/// On a failed export, only delete the temporary zip. Never unlink `dest`:
/// a pre-existing user-chosen path must survive an incomplete write.
fn cleanup_failed_export(tmp: &Path) {
    let _ = fs::remove_file(tmp);
}

fn add_log_files(
    zip: &mut ZipWriter<File>,
    options: FileOptions,
    canonical_root: &Path,
    current: &Path,
) -> std::io::Result<()> {
    let entries = match fs::read_dir(current) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path
            .strip_prefix(canonical_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if skip_packaged_log_name(&relative) {
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            add_log_files(zip, options, canonical_root, &path)?;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }

        let canonical_file = match fs::canonicalize(&path) {
            Ok(path) => path,
            Err(_) => continue,
        };
        if !canonical_file.starts_with(canonical_root) {
            continue;
        }
        let relative = match canonical_file.strip_prefix(canonical_root) {
            Ok(relative) => relative,
            Err(_) => continue,
        };
        if relative
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            continue;
        }

        let relative_unix = relative
            .components()
            .filter_map(|component| match component {
                Component::Normal(part) => Some(part.to_string_lossy()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        if relative_unix.is_empty() || skip_packaged_log_name(&relative_unix) {
            continue;
        }

        let body = match fs::read(&path) {
            Ok(body) => body,
            Err(_) => continue,
        };
        let name = format!("mtls-router-logs/{relative_unix}");
        let Ok(text) = std::str::from_utf8(&body) else {
            continue;
        };
        let sanitized = crate::redaction::sanitize_text(text);
        zip.start_file(name, options)?;
        zip.write_all(sanitized.as_bytes())?;
    }
    Ok(())
}

fn skip_packaged_log_name(relative_unix: &str) -> bool {
    if relative_unix.split('/').any(|part| {
        matches!(
            part.to_ascii_lowercase().as_str(),
            "identity" | "router-identity" | "enroll-state" | "journal"
        )
    }) {
        return true;
    }
    let name = relative_unix
        .rsplit('/')
        .next()
        .unwrap_or(relative_unix)
        .to_ascii_lowercase();
    name.starts_with(".identity-stage-")
        || name == "credentials.json"
        || [
            ".zip", ".tmp", ".pem", ".key", ".csr", ".p8", ".p12", ".pfx", ".der", ".sqlite3",
            ".bak",
        ]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

pub fn export_support_bundle(
    snapshot: &DiagnosticSnapshot,
    log_directory: &Path,
    dialog: &dyn SaveDialog,
) -> Result<PathBuf> {
    let default_name = default_bundle_name(Local::now());
    let Some(dest) = dialog.choose_zip_path(&default_name) else {
        return Err(CommandError::recoverable(
            "DIALOG_CANCELLED",
            "export cancelled",
        ));
    };
    if dest.as_os_str().is_empty() {
        return Err(CommandError::new(
            "INVALID_PATH",
            "export destination is empty",
        ));
    }
    write_support_bundle(snapshot, log_directory, &dest)
        .map_err(|_| CommandError::new("EXPORT_FAILED", "cannot write support bundle"))?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic_snapshot::SCHEMA_VERSION;
    use std::{
        fs,
        io::{Cursor, Read},
    };
    use zip::ZipArchive;

    struct ScriptedDialog(Option<PathBuf>);

    impl SaveDialog for ScriptedDialog {
        fn choose_zip_path(&self, _: &str) -> Option<PathBuf> {
            self.0.clone()
        }
    }

    fn minimal_snapshot() -> DiagnosticSnapshot {
        DiagnosticSnapshot {
            schema_version: SCHEMA_VERSION,
            captured_at: "2026-09-03T10:00:00Z".into(),
            classification: "healthy".into(),
            desktop: "0.1.0".into(),
            manager: "0.1.0".into(),
            management_protocol: "1".into(),
            deployment_id: "test".into(),
            target: "test-triple".into(),
            router_state: None,
            owner: None,
            listen_addr: None,
            pid: None,
            health_status: None,
            health_checked_at: None,
            health_stale: true,
            status_error_code: None,
            status_error_stage: None,
            health_error_code: None,
            manager_stage: None,
            manager_code: None,
            last_error: None,
            os_error: None,
            listen_refusal: None,
            recorded_state: None,
            recorded_pid: None,
            recorded_started_at: None,
            recorded_executable: None,
        }
    }

    #[test]
    fn cancel_returns_dialog_cancelled() {
        let err = export_support_bundle(
            &minimal_snapshot(),
            Path::new("/tmp"),
            &ScriptedDialog(None),
        )
        .unwrap_err();
        assert_eq!(err.code, "DIALOG_CANCELLED");
    }

    #[test]
    fn identity_material_and_native_proofs_never_enter_support_archive() {
        let root = std::env::temp_dir().join(format!("router-support-identity-{}", Uuid::new_v4()));
        let logs = root.join("logs");
        fs::create_dir_all(logs.join("identity")).unwrap();
        fs::write(logs.join("identity/state.json"), b"private-state-canary").unwrap();
        fs::write(logs.join("accidental.key"), b"private-key-canary").unwrap();
        fs::write(
            logs.join(".identity-stage-interrupted"),
            b"private-stage-canary",
        )
        .unwrap();
        fs::write(logs.join("session.log"),b"verifier=native-proof-canary oauth_code=oauth-code-canary token=oauth-token-canary api_key=business-key-canary\nsafe log\n").unwrap();
        let dest = root.join("support.zip");
        write_support_bundle(&minimal_snapshot(), &logs, &dest).unwrap();
        let mut archive = ZipArchive::new(File::open(dest).unwrap()).unwrap();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).unwrap();
            assert!(!file.name().contains("identity"));
            assert!(!file.name().ends_with(".key"));
            let mut body = String::new();
            file.read_to_string(&mut body).unwrap();
            assert!(!body.contains("canary"));
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zip_contains_snapshot_and_logs_skips_escape() {
        let root =
            std::env::temp_dir().join(format!("mtls-router-support-bundle-{}", Uuid::new_v4()));
        let log_dir = root.join("logs");
        let day_dir = log_dir.join("2026-09-03");
        fs::create_dir_all(&day_dir).unwrap();
        fs::write(day_dir.join("session.log"), b"safe log").unwrap();

        let credentials = root.join("credentials.json");
        fs::write(&credentials, b"secret").unwrap();

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&credentials, log_dir.join("escape.link")).unwrap();
        }

        let dest = root.join("out.zip");
        write_support_bundle(&minimal_snapshot(), &log_dir, &dest).unwrap();

        let bytes = fs::read(&dest).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut names = Vec::new();
        for i in 0..archive.len() {
            names.push(archive.by_index(i).unwrap().name().to_owned());
        }
        assert!(names.contains(&"last-diagnostics.json".to_owned()));
        assert!(names.contains(&"mtls-router-logs/2026-09-03/session.log".to_owned()));
        assert!(!names.iter().any(|name| name.contains("credentials")));
        assert!(!names.iter().any(|name| name.contains("escape")));

        {
            let mut file = archive
                .by_name("mtls-router-logs/2026-09-03/session.log")
                .unwrap();
            let mut body = String::new();
            file.read_to_string(&mut body).unwrap();
            assert_eq!(body, "safe log");
        }

        fs::write(
            log_dir.join("app.log"),
            b"{\"event\":\"update.verify_failed\",\"token\":\"sk-supersecretvalue\"}\n",
        )
        .unwrap();
        let redacted_dest = root.join("redacted.zip");
        write_support_bundle(&minimal_snapshot(), &log_dir, &redacted_dest).unwrap();
        let redacted_bytes = fs::read(&redacted_dest).unwrap();
        let mut redacted = ZipArchive::new(Cursor::new(redacted_bytes)).unwrap();
        let mut app_log = String::new();
        redacted
            .by_name("mtls-router-logs/app.log")
            .unwrap()
            .read_to_string(&mut app_log)
            .unwrap();
        assert!(!app_log.contains("sk-supersecretvalue"));
        assert!(app_log.contains("update.verify_failed"));
        {
            let mut file = archive.by_name("last-diagnostics.json").unwrap();
            let mut body = String::new();
            file.read_to_string(&mut body).unwrap();
            let parsed: DiagnosticSnapshot = serde_json::from_str(&body).unwrap();
            assert_eq!(parsed.classification, "healthy");
        }

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zip_keeps_latched_state_reconcile_and_redacted_recorded_identity() {
        let root = std::env::temp_dir().join(format!(
            "mtls-router-support-bundle-reconcile-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let dest = root.join("out.zip");
        let mut snapshot = minimal_snapshot();
        snapshot.classification = "start_failed".into();
        snapshot.router_state = Some("start_failed".into());
        snapshot.owner = Some("desktop".into());
        snapshot.manager_stage = Some("state_reconcile".into());
        snapshot.manager_code = Some("ROUTER_STATE_STALE".into());
        snapshot.last_error = Some("stage=state_reconcile code=ROUTER_STATE_STALE".into());
        snapshot.recorded_state = Some("readable".into());
        snapshot.recorded_pid = Some(42018);
        snapshot.recorded_started_at = Some("2026-09-14T08:59:00.000000000Z".into());
        snapshot.recorded_executable = Some("mtls-router-desktop".into());
        write_support_bundle(&snapshot, &root.join("no-logs"), &dest).unwrap();

        let bytes = fs::read(&dest).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut file = archive.by_name("last-diagnostics.json").unwrap();
        let mut body = String::new();
        file.read_to_string(&mut body).unwrap();
        let parsed: DiagnosticSnapshot = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed.manager_stage.as_deref(), Some("state_reconcile"));
        assert_eq!(parsed.manager_code.as_deref(), Some("ROUTER_STATE_STALE"));
        assert_eq!(
            parsed.last_error.as_deref(),
            Some("stage=state_reconcile code=ROUTER_STATE_STALE")
        );
        assert_eq!(parsed.recorded_pid, Some(42018));
        assert_eq!(
            parsed.recorded_executable.as_deref(),
            Some("mtls-router-desktop")
        );
        assert!(!body.contains("Users"));
        assert!(!body.contains("Library"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_log_dir_still_writes_snapshot_only() {
        let root = std::env::temp_dir().join(format!(
            "mtls-router-support-bundle-missing-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let missing_logs = root.join("no-such-logs");
        let dest = root.join("out.zip");
        write_support_bundle(&minimal_snapshot(), &missing_logs, &dest).unwrap();

        let bytes = fs::read(&dest).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 1);
        assert_eq!(archive.by_index(0).unwrap().name(), "last-diagnostics.json");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn default_bundle_name_uses_local_timestamp() {
        let now = chrono::TimeZone::with_ymd_and_hms(&Local, 2026, 9, 3, 14, 5, 6).unwrap();
        assert_eq!(
            default_bundle_name(now),
            "codeasier-router-diagnostics-20260903-140506.zip"
        );
    }

    #[test]
    fn cleanup_failed_export_only_removes_tmp_not_dest() {
        let root = std::env::temp_dir().join(format!(
            "mtls-router-support-bundle-cleanup-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let dest = root.join("existing.zip");
        let tmp = root.join(".bundle-test.tmp");
        fs::write(&dest, b"KEEP-SENTINEL").unwrap();
        fs::write(&tmp, b"partial").unwrap();

        cleanup_failed_export(&tmp);

        assert!(!tmp.exists());
        assert_eq!(fs::read(&dest).unwrap(), b"KEEP-SENTINEL");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zip_skips_nested_zip_and_tmp_even_when_dest_is_inside_logs() {
        let root = std::env::temp_dir().join(format!(
            "mtls-router-support-bundle-skip-{}",
            Uuid::new_v4()
        ));
        let log_dir = root.join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        fs::write(log_dir.join("session.log"), b"keep").unwrap();
        fs::write(log_dir.join("old.zip"), b"previous-bundle").unwrap();
        fs::write(log_dir.join(".bundle-leftover.tmp"), b"partial").unwrap();

        let dest = log_dir.join("export.zip");
        write_support_bundle(&minimal_snapshot(), &log_dir, &dest).unwrap();

        let bytes = fs::read(&dest).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut names = Vec::new();
        for i in 0..archive.len() {
            names.push(archive.by_index(i).unwrap().name().to_owned());
        }
        assert!(names.contains(&"last-diagnostics.json".to_owned()));
        assert!(names.contains(&"mtls-router-logs/session.log".to_owned()));
        assert!(!names.iter().any(|name| name.ends_with(".zip")));
        assert!(!names.iter().any(|name| name.ends_with(".tmp")));
        assert!(!names.iter().any(|name| name.contains("old.zip")));
        assert!(!names.iter().any(|name| name.contains("leftover")));

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn bundle_temp_and_dest_are_owner_readable_only() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "mtls-router-support-bundle-mode-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let dest = root.join("out.zip");
        write_support_bundle(&minimal_snapshot(), root.join("no-logs").as_path(), &dest).unwrap();

        let mode = fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let _ = fs::remove_dir_all(root);
    }
}
