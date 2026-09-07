use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::protocol::ErrorCode;

use super::files::{private_permissions_ok, restrict_private};
use super::recovery::is_final_component_link;
use super::types::OperationError;

const LOCK_FILE_NAME: &str = "agent-operation.lock";
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const LOCK_POLL: Duration = Duration::from_millis(25);

#[cfg(not(windows))]
use super::lock_unix::{try_lock_file, unlock_file};
#[cfg(windows)]
use super::lock_windows::{try_lock_file, unlock_file};

pub struct TransactionLock {
    file: Option<File>,
}

impl TransactionLock {
    pub fn release(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = unlock_file(&file);
        }
    }
}

impl Drop for TransactionLock {
    fn drop(&mut self) {
        self.release();
    }
}

pub fn acquire_transaction_lock(
    state_dir: &Path,
    deadline: Option<Instant>,
) -> Result<TransactionLock, OperationError> {
    std::fs::create_dir_all(state_dir).map_err(|_| write_failed())?;
    restrict_private(state_dir, true).map_err(|_| write_failed())?;
    let path = state_dir.join(LOCK_FILE_NAME);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&path)
        .map_err(|_| write_failed())?;
    restrict_private(&path, false).map_err(|_| write_failed())?;
    acquire_opened(file, deadline)
}

pub fn acquire_existing_transaction_lock(
    state_dir: &Path,
    deadline: Option<Instant>,
) -> Result<TransactionLock, OperationError> {
    let dir_info = std::fs::symlink_metadata(state_dir).map_err(|_| invalid_dir())?;
    if is_final_component_link(state_dir, &dir_info)
        || !dir_info.is_dir()
        || !private_permissions_ok(state_dir, true, unix_mode(&dir_info))
    {
        return Err(invalid_dir());
    }
    let path = state_dir.join(LOCK_FILE_NAME);
    let path_info = std::fs::symlink_metadata(&path).map_err(|_| invalid_lock())?;
    if is_final_component_link(&path, &path_info)
        || !path_info.is_file()
        || !private_permissions_ok(&path, false, unix_mode(&path_info))
    {
        return Err(invalid_lock());
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|_| invalid_lock())?;
    let opened = file.metadata().map_err(|_| identity_changed())?;
    if !opened.is_file() || !same_inode(&path_info, &opened) {
        return Err(identity_changed());
    }
    let path_after = std::fs::symlink_metadata(&path).map_err(|_| path_changed())?;
    if is_final_component_link(&path, &path_after)
        || !same_inode(&opened, &path_after)
        || !private_permissions_ok(&path, false, unix_mode(&path_after))
    {
        return Err(path_changed());
    }
    acquire_opened(file, deadline)
}

fn acquire_opened(
    file: File,
    deadline: Option<Instant>,
) -> Result<TransactionLock, OperationError> {
    let stop = Instant::now() + LOCK_TIMEOUT;
    loop {
        match try_lock_file(&file) {
            Ok(true) => return Ok(TransactionLock { file: Some(file) }),
            Ok(false) => {}
            Err(_) => return Err(write_failed()),
        }
        if deadline.is_some_and(|value| Instant::now() >= value) || Instant::now() >= stop {
            return Err(OperationError::new(
                ErrorCode::AgentOperationBusy,
                "Another Agent operation is in progress",
            ));
        }
        std::thread::sleep(LOCK_POLL);
    }
}

fn unix_mode(info: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        info.permissions().mode()
    }
    #[cfg(not(unix))]
    {
        let _ = info;
        0
    }
}

fn same_inode(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
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

fn write_failed() -> OperationError {
    OperationError::new(
        ErrorCode::WriteFailed,
        "could not prepare Agent transaction state",
    )
}

fn invalid_dir() -> OperationError {
    OperationError::new(
        ErrorCode::ModelStateInvalid,
        "invalid transaction state directory",
    )
}

fn invalid_lock() -> OperationError {
    OperationError::new(ErrorCode::ModelStateInvalid, "invalid transaction lock")
}

fn identity_changed() -> OperationError {
    OperationError::new(
        ErrorCode::ModelStateInvalid,
        "transaction lock identity changed",
    )
}

fn path_changed() -> OperationError {
    OperationError::new(
        ErrorCode::ModelStateInvalid,
        "transaction lock path identity changed",
    )
}
