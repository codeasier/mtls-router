use crate::{
    diagnostic_snapshot::DiagnosticSnapshot,
    error::{CommandError, Result},
};
use chrono::{DateTime, Local};
use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;
use zip::{write::FileOptions, CompressionMethod, ZipWriter};

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
        let file = File::create(&tmp)?;
        let mut zip = ZipWriter::new(file);
        let options = FileOptions::default().compression_method(CompressionMethod::Deflated);

        zip.start_file("last-diagnostics.json", options)?;
        zip.write_all(&serde_json::to_vec_pretty(snapshot).map_err(io::Error::other)?)?;

        if log_directory.is_dir() {
            let canonical_root = fs::canonicalize(log_directory)?;
            add_log_files(&mut zip, options, &canonical_root, log_directory)?;
        }

        let finished = zip.finish()?;
        finished.sync_all()?;
        fs::rename(&tmp, dest)?;
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
        if relative_unix.is_empty() {
            continue;
        }

        let body = match fs::read(&path) {
            Ok(body) => body,
            Err(_) => continue,
        };
        let name = format!("mtls-router-logs/{relative_unix}");
        zip.start_file(name, options)?;
        zip.write_all(&body)?;
    }
    Ok(())
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
            router: String::new(),
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
}
