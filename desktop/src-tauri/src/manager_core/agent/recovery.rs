use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::Path;

use super::types::{append_reason, Format, RecoveryFileState, RecoveryReason, State};

#[cfg(windows)]
pub(crate) use super::recovery_windows::is_final_component_link;

#[cfg(not(windows))]
pub(crate) fn is_final_component_link(_path: &Path, metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

pub fn inspect_recovery_target(role: &str, path: &Path, format: Format) -> RecoveryFileState {
    let mut file = RecoveryFileState {
        role: role.to_owned(),
        path: path.to_string_lossy().into_owned(),
        format,
        exists: false,
        reasons: Vec::new(),
    };
    match fs::symlink_metadata(path) {
        Ok(info) => {
            file.exists = true;
            if is_final_component_link(path, &info) {
                append_reason(&mut file.reasons, RecoveryReason::Linked);
            } else if !info.file_type().is_file() {
                append_reason(&mut file.reasons, RecoveryReason::NonRegular);
            } else if !path_writable(path) {
                append_reason(&mut file.reasons, RecoveryReason::NotWritable);
            }
        }
        Err(error) if error.kind() != ErrorKind::NotFound => {
            append_reason(&mut file.reasons, RecoveryReason::Unreadable);
        }
        Err(_) => {}
    }
    let parent = path.parent().unwrap_or(path);
    let parent_ok = fs::metadata(parent)
        .ok()
        .is_some_and(|info| info.is_dir() && parent_writable(&info));
    if !parent_ok {
        append_reason(&mut file.reasons, RecoveryReason::ParentUnavailable);
    }
    file
}

pub fn finalize_recovery(state: &mut State) {
    let mut reasons = Vec::new();
    let mut has_syntax_invalid = false;
    let mut blocked = false;
    for file in &state.recovery.files {
        for reason in &file.reasons {
            append_reason(&mut reasons, *reason);
            if *reason == RecoveryReason::SyntaxInvalid {
                has_syntax_invalid = true;
            } else {
                blocked = true;
            }
        }
    }
    state.recovery.reasons = reasons;
    state.recovery.eligible = has_syntax_invalid && !blocked;
}

pub fn contains_recovery_reason(reasons: &[RecoveryReason], want: RecoveryReason) -> bool {
    reasons.contains(&want)
}

pub fn has_blocking_file_reason(reasons: &[RecoveryReason]) -> bool {
    reasons
        .iter()
        .any(|reason| *reason != RecoveryReason::SyntaxInvalid)
}

pub fn has_legacy_invalid_file_reason(reasons: &[RecoveryReason]) -> bool {
    reasons.iter().any(|reason| {
        matches!(
            reason,
            RecoveryReason::SyntaxInvalid
                | RecoveryReason::Unreadable
                | RecoveryReason::Oversized
                | RecoveryReason::NonRegular
        )
    })
}

pub fn path_writable(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(info) => {
            if !info.is_file() {
                return false;
            }
            OpenOptions::new().write(true).open(path).is_ok()
        }
        Err(error) if error.kind() != ErrorKind::NotFound => false,
        Err(_) => {
            let mut dir = match path.parent() {
                Some(parent) => parent.to_path_buf(),
                None => return false,
            };
            loop {
                match fs::metadata(&dir) {
                    Ok(info) => return info.is_dir() && dir_owner_or_group_writable(&info),
                    Err(error) if error.kind() != ErrorKind::NotFound => return false,
                    Err(_) => {
                        let Some(parent) = dir.parent() else {
                            return false;
                        };
                        if parent == dir {
                            return false;
                        }
                        dir = parent.to_path_buf();
                    }
                }
            }
        }
    }
}

#[cfg(unix)]
fn parent_writable(info: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    info.permissions().mode() & 0o200 != 0
}

#[cfg(not(unix))]
fn parent_writable(info: &std::fs::Metadata) -> bool {
    !info.permissions().readonly()
}

#[cfg(unix)]
fn dir_owner_or_group_writable(info: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    info.permissions().mode() & 0o222 != 0
}

#[cfg(not(unix))]
fn dir_owner_or_group_writable(info: &std::fs::Metadata) -> bool {
    !info.permissions().readonly()
}
