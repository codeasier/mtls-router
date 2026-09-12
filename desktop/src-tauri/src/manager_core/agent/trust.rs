use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD_NO_PAD;
use base64::Engine;
use serde::Deserialize;
use uuid::Uuid;

use super::files::{ensure_private_permissions, restrict_private, sync_directory, write_atomic};
use super::modelconfig::TokenSigner;
use super::types::OperationError;
use crate::protocol::ErrorCode;

pub const SIGNING_KEY_FILE_NAME: &str = "token-signing-key.json";
pub const SIDECAR_FILE_NAME: &str = "last-applied-model-config.json";
const MAX_SIGNING_KEY_SIZE: u64 = 4096;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SigningKeyFile {
    pub version: i64,
    pub generation: String,
    pub key: String,
}

pub fn load_or_create_signing_key(
    state_dir: &Path,
    journal_exists: bool,
    sidecar_exists: bool,
) -> Result<SigningKeyFile, OperationError> {
    let path = state_dir.join(SIGNING_KEY_FILE_NAME);
    match load_signing_key(&path) {
        Ok(key) => Ok(key),
        Err(error) if error.kind() != ErrorKind::NotFound => Err(trust_invalid()),
        Err(_) => {
            if journal_exists || sidecar_exists {
                return Err(trust_invalid());
            }
            fs::create_dir_all(state_dir).map_err(|_| trust_invalid())?;
            restrict_private(state_dir, true).map_err(|_| trust_invalid())?;
            let material = random_key().ok_or_else(trust_invalid)?;
            let generation = random_generation().ok_or_else(trust_invalid)?;
            let created = SigningKeyFile {
                version: 1,
                generation,
                key: STANDARD_NO_PAD.encode(material),
            };
            let mut content = serde_json::to_vec(&serde_json::json!({
                "version": created.version,
                "generation": created.generation,
                "key": created.key,
            }))
            .map_err(|_| trust_invalid())?;
            content.push(b'\n');
            let tmp_path =
                state_dir.join(format!(".token-signing-key-{}", Uuid::new_v4().simple()));
            {
                let mut tmp = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&tmp_path)
                    .map_err(|_| trust_invalid())?;
                restrict_private(&tmp_path, false).map_err(|_| trust_invalid())?;
                tmp.write_all(&content).map_err(|_| trust_invalid())?;
                tmp.sync_all().map_err(|_| trust_invalid())?;
            }
            match fs::hard_link(&tmp_path, &path) {
                Ok(()) => {
                    let _ = fs::remove_file(&tmp_path);
                    sync_directory(state_dir).map_err(|_| trust_invalid())?;
                    Ok(created)
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    let _ = fs::remove_file(&tmp_path);
                    load_signing_key(&path).map_err(|_| trust_invalid())
                }
                Err(_) => {
                    let _ = fs::remove_file(&tmp_path);
                    Err(trust_invalid())
                }
            }
        }
    }
}

pub fn load_signing_key(path: &Path) -> io::Result<SigningKeyFile> {
    let info = fs::metadata(path)?;
    if !info.is_file()
        || !ensure_private_permissions(path, false, unix_mode(&info))
        || info.len() == 0
        || info.len() > MAX_SIGNING_KEY_SIZE
    {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "unsafe signing key file",
        ));
    }
    let content = fs::read(path)?;
    let mut deserializer = serde_json::Deserializer::from_slice(&content);
    let key = SigningKeyFile::deserialize(&mut deserializer)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    deserializer
        .end()
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    let material = STANDARD_NO_PAD
        .decode(key.key.as_bytes())
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    if material.len() != 32 || key.version != 1 || key.generation.len() != 32 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "invalid signing key",
        ));
    }
    Ok(key)
}

pub fn signer_from_key(key: &SigningKeyFile) -> Result<TokenSigner, OperationError> {
    let material = STANDARD_NO_PAD
        .decode(key.key.as_bytes())
        .map_err(|_| trust_invalid())?;
    TokenSigner::new(&material, &key.generation).map_err(|_| trust_invalid())
}

pub fn persist_sidecar(path: &Path, content: &[u8]) -> Result<(), OperationError> {
    write_atomic(path, content, 0o600, true).map_err(|_| {
        OperationError::new(
            ErrorCode::WriteFailed,
            "could not persist Agent model state",
        )
    })
}

pub fn sidecar_path(state_dir: &Path) -> PathBuf {
    state_dir.join(SIDECAR_FILE_NAME)
}

fn unix_mode(info: &fs::Metadata) -> u32 {
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

fn random_key() -> Option<[u8; 32]> {
    let a = *Uuid::new_v4().as_bytes();
    let b = *Uuid::new_v4().as_bytes();
    let mut material = [0u8; 32];
    material[..16].copy_from_slice(&a);
    material[16..].copy_from_slice(&b);
    Some(material)
}

fn random_generation() -> Option<String> {
    Some(Uuid::new_v4().simple().to_string())
}

fn trust_invalid() -> OperationError {
    OperationError::new(
        ErrorCode::ModelStateInvalid,
        "Agent model trust state is invalid",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_key_round_trip_uses_standard_nopad() {
        let dir =
            std::env::temp_dir().join(format!("mtls-agent-trust-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&dir).unwrap();
        let key = load_or_create_signing_key(&dir, false, false).unwrap();
        assert_eq!(key.generation.len(), 32);
        let loaded = load_signing_key(&dir.join(SIGNING_KEY_FILE_NAME)).unwrap();
        assert_eq!(loaded.generation, key.generation);
        assert!(signer_from_key(&loaded).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn inherited_acl_signing_key_is_repaired_on_load() {
        let dir =
            std::env::temp_dir().join(format!("mtls-agent-trust-{}", Uuid::new_v4().simple()));
        let inherited_dir = std::env::temp_dir().join(format!(
            "mtls-agent-trust-inherit-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(&inherited_dir).unwrap();
        let created = load_or_create_signing_key(&dir, false, false).unwrap();
        let inherited = inherited_dir.join(SIGNING_KEY_FILE_NAME);
        fs::write(
            &inherited,
            fs::read(dir.join(SIGNING_KEY_FILE_NAME)).unwrap(),
        )
        .unwrap();
        assert!(!crate::windows_security::private_permissions_ok(&inherited));
        let loaded = load_signing_key(&inherited).unwrap();
        assert_eq!(loaded.generation, created.generation);
        assert!(crate::windows_security::private_permissions_ok(&inherited));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&inherited_dir);
    }
}
