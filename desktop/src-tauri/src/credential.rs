use crate::types::CredentialSummary;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use thiserror::Error;
use tokio::sync::Mutex;
use zeroize::{Zeroize, Zeroizing};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

// Keep in sync with the early UI validation in desktop/src/ApiKeysPage.tsx.
pub const MAX_KEY_BYTES: usize = 16 * 1024;
const SCHEMA_VERSION: u32 = 1;
const LOCK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("credential not found")]
    NotFound,
    #[error("credential file is malformed: {0}")]
    InvalidFormat(String),
    #[error("credential io error: {0}")]
    Io(#[from] io::Error),
    #[error("credential lock timeout")]
    LockTimeout,
}

#[derive(Debug)]
pub struct CredentialStore {
    path: PathBuf,
    inner: Mutex<()>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OnDisk {
    version: u32,
    saved_at: String,
    key: Zeroizing<String>,
}

impl CredentialStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            inner: Mutex::new(()),
        }
    }

    pub async fn read_summary(&self) -> Result<CredentialSummary, CredentialError> {
        let _guard = tokio::time::timeout(LOCK_TIMEOUT, self.inner.lock())
            .await
            .map_err(|_| CredentialError::LockTimeout)?;
        let (saved_at, key) = read(&self.path)?;
        let summary = CredentialSummary {
            present: true,
            fingerprint: fingerprint(&key),
            saved_at: Some(saved_at),
        };
        Ok(summary)
    }

    pub async fn write(
        &self,
        key: Zeroizing<String>,
    ) -> Result<CredentialSummary, CredentialError> {
        validate_key(&key)?;
        let _guard = tokio::time::timeout(LOCK_TIMEOUT, self.inner.lock())
            .await
            .map_err(|_| CredentialError::LockTimeout)?;

        let saved_at = Utc::now().to_rfc3339();
        let summary = CredentialSummary {
            present: true,
            fingerprint: fingerprint(&key),
            saved_at: Some(saved_at.clone()),
        };
        let mut disk = OnDisk {
            version: SCHEMA_VERSION,
            saved_at,
            key,
        };
        let encoded = serde_json::to_vec_pretty(&disk)
            .map(Zeroizing::new)
            .map_err(|error| CredentialError::InvalidFormat(error.to_string()));
        // The serialized buffer now owns its copy; clear the temporary field immediately.
        disk.key.zeroize();
        let encoded = encoded?;
        write_atomic(&self.path, &encoded)?;
        Ok(summary)
    }

    pub async fn delete(&self) -> Result<(), CredentialError> {
        let _guard = tokio::time::timeout(LOCK_TIMEOUT, self.inner.lock())
            .await
            .map_err(|_| CredentialError::LockTimeout)?;
        match fs::remove_file(&self.path) {
            Ok(()) => {
                sync_parent(&self.path)?;
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn use_(&self) -> Result<Zeroizing<String>, CredentialError> {
        let _guard = tokio::time::timeout(LOCK_TIMEOUT, self.inner.lock())
            .await
            .map_err(|_| CredentialError::LockTimeout)?;
        let (_, key) = read(&self.path)?;
        Ok(key)
    }
}

fn read(path: &Path) -> Result<(String, Zeroizing<String>), CredentialError> {
    let content = match fs::read(path) {
        Ok(content) => Zeroizing::new(content),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(CredentialError::NotFound);
        }
        Err(error) => return Err(error.into()),
    };
    parse(&content)
}

fn parse(content: &[u8]) -> Result<(String, Zeroizing<String>), CredentialError> {
    let content = content.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(content);
    let disk: OnDisk = serde_json::from_slice(content)
        .map_err(|error| CredentialError::InvalidFormat(error.to_string()))?;
    if disk.version != SCHEMA_VERSION {
        return Err(CredentialError::InvalidFormat(
            "unsupported schema version".into(),
        ));
    }
    validate_key(&disk.key)?;
    DateTime::parse_from_rfc3339(&disk.saved_at)
        .map_err(|_| CredentialError::InvalidFormat("saved_at is not RFC3339".into()))?;
    Ok((disk.saved_at, disk.key))
}

fn validate_key(key: &str) -> Result<(), CredentialError> {
    if key.is_empty() || key.len() > MAX_KEY_BYTES {
        return Err(CredentialError::InvalidFormat(
            "key length is invalid".into(),
        ));
    }
    Ok(())
}

fn fingerprint(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    let encoded = base32(&digest);
    encoded[encoded.len() - 4..].to_owned()
}

fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut output = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let mut buffer = 0_u16;
    let mut bits = 0_u8;
    for byte in bytes {
        buffer = (buffer << 8) | u16::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            output.push(ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        output.push(ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    output
}

fn write_atomic(path: &Path, content: &[u8]) -> Result<(), CredentialError> {
    write_atomic_with(path, content, replace_file)
}

fn write_atomic_with(
    path: &Path,
    content: &[u8],
    replace: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<(), CredentialError> {
    let parent = path
        .parent()
        .ok_or_else(|| CredentialError::InvalidFormat("credential path has no parent".into()))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".credentials-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary)?;
        file.write_all(content)?;
        file.sync_all()?;
        #[cfg(unix)]
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
        replace(&temporary, path)?;
        #[cfg(unix)]
        {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            OpenOptions::new().read(true).open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result.map_err(CredentialError::Io)
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // Same-volume MoveFileExW replaces the directory entry without an unlink gap.
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        OpenOptions::new().read(true).open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct TempDir(PathBuf);

    impl TempDir {
        fn credential(&self) -> PathBuf {
            self.0.join("credentials.json")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir(name: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "mtls-router-credential-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    #[tokio::test]
    async fn summary_when_file_missing() {
        let dir = temp_dir("missing");
        let error = CredentialStore::new(dir.credential())
            .read_summary()
            .await
            .unwrap_err();
        assert!(matches!(error, CredentialError::NotFound));
    }

    #[tokio::test]
    async fn malformed_files_are_rejected() {
        for (name, content) in [
            ("empty", b"{}".as_slice()),
            ("bad-json", b"{".as_slice()),
            (
                "wrong-version",
                br#"{"version":2,"saved_at":"2026-07-26T00:00:00Z","key":"x"}"#,
            ),
            (
                "missing-key",
                br#"{"version":1,"saved_at":"2026-07-26T00:00:00Z"}"#,
            ),
            (
                "extra-field",
                br#"{"version":1,"saved_at":"2026-07-26T00:00:00Z","key":"x","extra":true}"#,
            ),
        ] {
            let dir = temp_dir(name);
            fs::write(dir.credential(), content).unwrap();
            let error = CredentialStore::new(dir.credential())
                .read_summary()
                .await
                .unwrap_err();
            assert!(matches!(error, CredentialError::InvalidFormat(_)), "{name}");
        }
    }

    #[tokio::test]
    async fn bom_is_accepted() {
        let dir = temp_dir("bom");
        fs::write(
            dir.credential(),
            b"\xEF\xBB\xBF{\"version\":1,\"saved_at\":\"2026-07-26T00:00:00Z\",\"key\":\"fixture\"}",
        )
        .unwrap();
        assert!(
            CredentialStore::new(dir.credential())
                .read_summary()
                .await
                .unwrap()
                .present
        );
    }

    #[tokio::test]
    async fn write_then_summary_and_use() {
        let dir = temp_dir("write");
        let path = dir.credential();
        let store = CredentialStore::new(path.clone());
        let written = store
            .write(Zeroizing::new("fixture-key".into()))
            .await
            .unwrap();
        assert!(written.present);
        assert_eq!(written.fingerprint.len(), 4);
        assert_eq!(store.read_summary().await.unwrap(), written);
        assert_eq!(store.use_().await.unwrap().as_str(), "fixture-key");
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[tokio::test]
    async fn use_returns_key_then_zeroize_drop() {
        let dir = temp_dir("use-zeroizing");
        let store = CredentialStore::new(dir.credential());
        store
            .write(Zeroizing::new("fixture-key".into()))
            .await
            .unwrap();

        let key: Zeroizing<String> = store.use_().await.unwrap();
        assert_eq!(key.as_str(), "fixture-key");
        assert!(std::mem::needs_drop::<Zeroizing<String>>());
        drop(key);
    }

    #[tokio::test]
    async fn write_overwrites_existing_key() {
        let dir = temp_dir("overwrite");
        let store = CredentialStore::new(dir.credential());
        store.write(Zeroizing::new("first".into())).await.unwrap();
        store.write(Zeroizing::new("second".into())).await.unwrap();
        assert_eq!(store.use_().await.unwrap().as_str(), "second");
    }

    #[test]
    fn failed_replace_preserves_existing_key() {
        let dir = temp_dir("replace-failure");
        let path = dir.credential();
        fs::write(&path, b"old-credential").unwrap();

        let error = write_atomic_with(&path, b"new-credential", |_, _| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected replacement failure",
            ))
        })
        .unwrap_err();

        assert!(matches!(error, CredentialError::Io(_)));
        assert_eq!(fs::read(path).unwrap(), b"old-credential");
    }

    #[tokio::test]
    async fn invalid_key_lengths_are_rejected() {
        let dir = temp_dir("length");
        let store = CredentialStore::new(dir.credential());
        for key in [String::new(), "x".repeat(MAX_KEY_BYTES + 1)] {
            let error = store.write(Zeroizing::new(key)).await.unwrap_err();
            assert!(matches!(error, CredentialError::InvalidFormat(_)));
        }
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let dir = temp_dir("delete");
        let store = CredentialStore::new(dir.credential());
        store.write(Zeroizing::new("fixture".into())).await.unwrap();
        store.delete().await.unwrap();
        store.delete().await.unwrap();
        assert!(matches!(
            store.read_summary().await,
            Err(CredentialError::NotFound)
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_save_and_use_does_not_tear_reads() {
        let dir = temp_dir("concurrent");
        let store = Arc::new(CredentialStore::new(dir.credential()));
        store
            .write(Zeroizing::new("key-initial".into()))
            .await
            .unwrap();
        let writer = {
            let store = store.clone();
            tokio::spawn(async move {
                for index in 0..30 {
                    store
                        .write(Zeroizing::new(format!("key-{index}")))
                        .await
                        .unwrap();
                }
            })
        };
        let reader = {
            let store = store.clone();
            tokio::spawn(async move {
                for _ in 0..100 {
                    assert!(store.use_().await.unwrap().starts_with("key-"));
                }
            })
        };
        writer.await.unwrap();
        reader.await.unwrap();
    }
}
