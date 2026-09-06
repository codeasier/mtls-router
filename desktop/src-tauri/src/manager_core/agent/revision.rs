use std::fs;
use std::path::Path;
use std::time::SystemTime;

use super::detect::read_config;
use super::modelconfig::TokenSigner;
use super::plan::{
    FileRevision, JournalScope, REVISION_CONTEXT_AGENT_FILE, REVISION_CONTEXT_SIDECAR,
};
use super::types::{OperationError, MAX_CONFIG_SIZE};

pub fn keyed_revision_for_content(
    signer: &TokenSigner,
    content: &[u8],
    mode: u32,
    revision_context: &str,
    identity: &str,
) -> Result<FileRevision, OperationError> {
    let digest = signer
        .revision_mac(
            revision_context,
            &super::plan::clean_path(identity),
            content,
        )
        .map_err(|_| revision_invalid())?;
    Ok(FileRevision {
        exists: true,
        size: content.len() as i64,
        mode: mode & 0o777,
        digest,
    })
}

pub fn read_keyed_revision(
    signer: &TokenSigner,
    path: &Path,
    revision_context: &str,
) -> Result<(FileRevision, Vec<u8>, u32), OperationError> {
    let info = match fs::metadata(path) {
        Ok(info) => info,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((FileRevision::default(), Vec::new(), 0o600));
        }
        Err(_) => return Err(revision_invalid()),
    };
    if !info.is_file() {
        return Err(revision_invalid());
    }
    let content = read_config(path).map_err(|_| revision_invalid())?;
    let after = fs::metadata(path).map_err(|_| revision_invalid())?;
    if !same_file(&info, &after) || info.len() != after.len() || modified(&info) != modified(&after)
    {
        return Err(revision_invalid());
    }
    let mode = unix_perm(&after);
    let revision = keyed_revision_for_content(
        signer,
        &content,
        mode,
        revision_context,
        &path.to_string_lossy(),
    )?;
    Ok((revision, content, mode))
}

pub fn verify_keyed_revision(
    signer: &TokenSigner,
    path: &Path,
    expected: &FileRevision,
    revision_context: &str,
) -> Result<(), OperationError> {
    let (current, _, _) = read_keyed_revision(signer, path, revision_context)?;
    if &current != expected {
        return Err(revision_invalid());
    }
    Ok(())
}

pub fn revision_context_for(scope: JournalScope) -> &'static str {
    match scope {
        JournalScope::Agent => REVISION_CONTEXT_AGENT_FILE,
        JournalScope::ManagerState => REVISION_CONTEXT_SIDECAR,
    }
}

pub fn content_revision(content: &[u8], mode: u32) -> FileRevision {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(content);
    FileRevision {
        exists: true,
        size: content.len() as i64,
        mode: mode & 0o777,
        digest: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
    }
}

pub fn read_plain_revision(path: &Path) -> Result<(FileRevision, Vec<u8>, u32), OperationError> {
    let info = match fs::metadata(path) {
        Ok(info) => info,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((FileRevision::default(), Vec::new(), 0o600));
        }
        Err(_) => return Err(revision_invalid()),
    };
    if !info.is_file() {
        return Err(revision_invalid());
    }
    let content = read_config(path).map_err(|_| revision_invalid())?;
    let after = fs::metadata(path).map_err(|_| revision_invalid())?;
    if !same_file(&info, &after) || info.len() != after.len() || modified(&info) != modified(&after)
    {
        return Err(revision_invalid());
    }
    let mode = unix_perm(&after);
    Ok((content_revision(&content, mode), content, mode))
}

pub fn valid_journal_revision(revision: &FileRevision) -> bool {
    if !revision.exists {
        return *revision == FileRevision::default();
    }
    revision.size >= 0 && revision.size <= MAX_CONFIG_SIZE as i64 && !revision.digest.is_empty()
}

fn unix_perm(info: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        info.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        0o600
    }
}

fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        left.len() == right.len() && modified(left) == modified(right)
    }
}

fn modified(info: &fs::Metadata) -> Option<SystemTime> {
    info.modified().ok()
}

fn revision_invalid() -> OperationError {
    OperationError::new(
        crate::protocol::ErrorCode::ModelStateInvalid,
        "Agent revision state could not be created",
    )
}
