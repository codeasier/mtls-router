use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};

use uuid::Uuid;

use super::detect::read_config;
use super::recovery::is_final_component_link;
use super::types::MAX_CONFIG_SIZE;

#[cfg(not(windows))]
use super::files_unix as platform;
#[cfg(windows)]
use super::files_windows as platform;

pub use platform::{
    apply_private_mode, apply_target_permissions, private_permissions_ok, replace_atomic,
    restrict_private, sync_directory,
};

/// Accept an already-private object, or tighten a current-user inherited ACL
/// left by older Windows builds whose restrict helper was a no-op.
pub fn ensure_private_permissions(path: &Path, directory: bool, mode: u32) -> bool {
    if private_permissions_ok(path, directory, mode) {
        return true;
    }
    if !cfg!(windows) {
        return false;
    }
    restrict_private(path, directory).is_ok() && private_permissions_ok(path, directory, mode)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackupStage {
    Permission,
    Write,
    Sync,
    Reopen,
    Read,
    Identity,
    Content,
}

#[derive(Debug)]
pub struct PostReplaceError {
    pub source: io::Error,
}

impl std::fmt::Display for PostReplaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(f)
    }
}

impl std::error::Error for PostReplaceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

pub fn replacement_occurred(error: &(dyn std::error::Error + 'static)) -> bool {
    error.downcast_ref::<PostReplaceError>().is_some()
}

pub fn write_atomic(
    path: &Path,
    content: &[u8],
    mode: u32,
    private: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "missing parent"))?;
    fs::create_dir_all(dir)?;
    let tmp_path = dir.join(format!(
        ".{}-{}",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("tmp"),
        Uuid::new_v4().simple()
    ));
    let written = (|| {
        {
            let mut tmp = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp_path)?;
            restrict_private(&tmp_path, false)?;
            tmp.write_all(content)?;
            tmp.sync_all()?;
        }
        if !private {
            apply_target_permissions(&tmp_path, path, mode)?;
        }
        replace_atomic(&tmp_path, path)?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    written?;
    if let Err(error) = sync_directory(dir) {
        return Err(Box::new(PostReplaceError { source: error }));
    }
    Ok(())
}

pub fn remove_target_and_sync(path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    fs::remove_file(path)?;
    if let Err(error) = sync_directory(path.parent().unwrap_or(path)) {
        return Err(Box::new(PostReplaceError { source: error }));
    }
    Ok(())
}

pub fn remove_and_sync(path: &Path) -> io::Result<()> {
    if let Err(error) = fs::remove_file(path) {
        if error.kind() != ErrorKind::NotFound {
            return Err(error);
        }
    }
    sync_directory(path.parent().unwrap_or(path))
}

pub fn create_private_backup(
    source_path: &Path,
    content: &[u8],
    source_mode: u32,
    label: &str,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    create_private_backup_with_hook(source_path, content, source_mode, label, None)
}

pub fn create_private_backup_with_hook(
    source_path: &Path,
    content: &[u8],
    source_mode: u32,
    label: &str,
    hook: Option<
        &dyn Fn(BackupStage, &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>>,
    >,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let dir = source_path
        .parent()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "missing parent"))?;
    let base = source_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("backup");
    for _ in 0..16 {
        let name = format!(
            "{base}.{label}-{}-{}",
            chrono::Utc::now().format("%Y%m%d-%H%M%S.%f"),
            Uuid::new_v4().simple()
        );
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
        if let Err(error) = finish_backup(&path, dir, content, source_mode, hook) {
            let _ = remove_and_sync(&path);
            return Err(error);
        }
        return Ok(path);
    }
    Err("could not allocate unique backup path".into())
}

fn finish_backup(
    path: &Path,
    dir: &Path,
    content: &[u8],
    source_mode: u32,
    hook: Option<
        &dyn Fn(BackupStage, &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>>,
    >,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(hook) = hook {
        hook(BackupStage::Permission, path)?;
    }
    restrict_private(path, false)?;
    if let Some(hook) = hook {
        hook(BackupStage::Write, path)?;
    }
    {
        let mut file = OpenOptions::new().write(true).open(path)?;
        file.write_all(content)?;
        if let Some(hook) = hook {
            hook(BackupStage::Sync, path)?;
        }
        file.sync_all()?;
    }
    apply_private_mode(path, source_mode)?;
    verify_private_backup(path, content, hook)?;
    sync_directory(dir)?;
    Ok(())
}

fn verify_private_backup(
    path: &Path,
    expected: &[u8],
    hook: Option<
        &dyn Fn(BackupStage, &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>>,
    >,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let before = fs::symlink_metadata(path)?;
    if is_final_component_link(path, &before) || !before.is_file() {
        return Err("backup identity is unsafe".into());
    }
    if let Some(hook) = hook {
        hook(BackupStage::Reopen, path)?;
    }
    let file = File::open(path)?;
    let after = file.metadata()?;
    if !after.is_file() || !same_file(&before, &after) {
        return Err("backup identity changed".into());
    }
    if let Some(hook) = hook {
        hook(BackupStage::Identity, path)?;
    }
    let path_after = fs::symlink_metadata(path)?;
    if is_final_component_link(path, &path_after)
        || !path_after.is_file()
        || !same_file(&after, &path_after)
    {
        return Err("backup path identity changed".into());
    }
    if let Some(hook) = hook {
        hook(BackupStage::Read, path)?;
    }
    let content = read_config(path).map_err(|_| "backup read-back failed")?;
    if content.len() > MAX_CONFIG_SIZE {
        return Err("backup read-back failed".into());
    }
    if let Some(hook) = hook {
        hook(BackupStage::Content, path)?;
    }
    if content.as_slice() != expected {
        return Err("backup content mismatch".into());
    }
    Ok(())
}

fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        left.len() == right.len() && left.modified().ok() == right.modified().ok()
    }
}
