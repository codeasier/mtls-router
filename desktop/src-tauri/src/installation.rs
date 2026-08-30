use crate::error::{CommandError, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use uuid::Uuid;

pub const SCHEMA_VERSION: u32 = 1;
pub const PACKAGE_GENERATION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallationOwnership {
    pub schema_version: u32,
    pub installation_id: String,
    pub package_generation: u32,
    pub deployment_id: String,
    pub management_protocol_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub manager_sidecar_sha256: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub router_sidecar_sha256: String,
}

impl InstallationOwnership {
    pub fn current(sidecar_hashes: (&str, &str)) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            installation_id: String::new(),
            package_generation: PACKAGE_GENERATION,
            deployment_id: env!("MTLS_DEPLOYMENT_ID").to_owned(),
            management_protocol_version: env!("MTLS_MANAGEMENT_PROTOCOL_VERSION").to_owned(),
            manager_sidecar_sha256: sidecar_hashes.0.to_owned(),
            router_sidecar_sha256: sidecar_hashes.1.to_owned(),
        }
    }
}

pub fn load_or_create(
    data_dir: &str,
    sidecar_hashes: (&str, &str),
) -> Result<InstallationOwnership> {
    let path = Path::new(data_dir).join("installation.json");
    let current = InstallationOwnership::current(sidecar_hashes);
    match fs::read(&path) {
        Ok(bytes) => {
            let existing: InstallationOwnership = serde_json::from_slice(&bytes).map_err(|_| {
                CommandError::new(
                    "INSTALLATION_INVALID",
                    "desktop installation metadata is corrupt",
                )
            })?;
            validate(&existing)?;
            let mut updated = existing.clone();
            updated.package_generation = current.package_generation;
            updated.deployment_id = current.deployment_id;
            updated.management_protocol_version = current.management_protocol_version;
            updated.manager_sidecar_sha256 = current.manager_sidecar_sha256;
            updated.router_sidecar_sha256 = current.router_sidecar_sha256;
            validate(&updated)?;
            if updated != existing {
                write_atomic(&path, &updated)?;
            }
            Ok(updated)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut created = current;
            created.installation_id = Uuid::new_v4().to_string();
            validate(&created)?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|_| {
                    CommandError::new(
                        "INSTALLATION_INVALID",
                        "cannot create installation directory",
                    )
                })?;
            }
            write_atomic(&path, &created)?;
            Ok(created)
        }
        Err(_) => Err(CommandError::new(
            "INSTALLATION_INVALID",
            "desktop installation metadata is unreadable",
        )),
    }
}

fn validate(value: &InstallationOwnership) -> Result<()> {
    if value.schema_version != SCHEMA_VERSION {
        return Err(CommandError::new(
            "INSTALLATION_INVALID",
            "desktop installation metadata is incompatible",
        ));
    }
    if Uuid::parse_str(&value.installation_id).is_err() {
        return Err(CommandError::new(
            "INSTALLATION_INVALID",
            "desktop installation identity is invalid",
        ));
    }
    if value.package_generation < 1
        || value.deployment_id.trim().is_empty()
        || value.management_protocol_version.trim().is_empty()
    {
        return Err(CommandError::new(
            "INSTALLATION_INVALID",
            "desktop installation metadata is corrupt",
        ));
    }
    for hash in [&value.manager_sidecar_sha256, &value.router_sidecar_sha256] {
        if !hash.is_empty() && !valid_sha256(hash) {
            return Err(CommandError::new(
                "INSTALLATION_INVALID",
                "desktop installation metadata is corrupt",
            ));
        }
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn write_atomic(path: &Path, value: &InstallationOwnership) -> Result<()> {
    let encoded = serde_json::to_vec_pretty(value).map_err(|_| {
        CommandError::new(
            "INSTALLATION_INVALID",
            "cannot encode installation metadata",
        )
    })?;
    let parent = path.parent().ok_or_else(|| {
        CommandError::new(
            "INSTALLATION_INVALID",
            "cannot persist installation metadata",
        )
    })?;
    let tmp = parent.join(format!(".installation-{}.tmp", Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&tmp)?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        #[cfg(unix)]
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
        fs::rename(&tmp, path)?;
        #[cfg(unix)]
        {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            OpenOptions::new().read(true).open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|_| {
        CommandError::new(
            "INSTALLATION_INVALID",
            "cannot persist installation metadata",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn hashes() -> (&'static str, &'static str) {
        (
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
    }

    #[test]
    fn load_or_create_keeps_stable_id_across_hash_updates() {
        let dir = tempfile();
        let first = load_or_create(&dir, hashes()).unwrap();
        let updated = load_or_create(
            &dir,
            (
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                hashes().1,
            ),
        )
        .unwrap();
        assert_eq!(first.installation_id, updated.installation_id);
        assert_eq!(
            updated.manager_sidecar_sha256,
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        );
    }

    #[test]
    fn load_or_create_rejects_newer_schema() {
        let dir = tempfile();
        fs::write(
            Path::new(&dir).join("installation.json"),
            r#"{"schema_version":2,"installation_id":"11111111-1111-4111-8111-111111111111","package_generation":1,"deployment_id":"prod","management_protocol_version":"4"}"#,
        )
        .unwrap();
        assert_eq!(
            load_or_create(&dir, hashes()).unwrap_err().code,
            "INSTALLATION_INVALID"
        );
    }

    fn tempfile() -> String {
        let path = std::env::temp_dir().join(format!("mtls-install-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path.to_string_lossy().into_owned()
    }
}
