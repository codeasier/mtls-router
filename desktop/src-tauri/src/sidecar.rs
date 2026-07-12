use crate::error::{CommandError, Result};
use object::{Architecture, BinaryFormat, Object, ObjectKind};
use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidecarPaths {
    pub manager: PathBuf,
    pub router: PathBuf,
}

impl SidecarPaths {
    pub fn resolve() -> Result<Self> {
        let directory = env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(PathBuf::from))
            .ok_or_else(|| missing("cannot locate packaged sidecars"))?;
        let suffix = if cfg!(windows) { ".exe" } else { "" };
        Ok(Self {
            manager: directory.join(format!("mtls-router-manager{suffix}")),
            router: directory.join(format!("mtls-router{suffix}")),
        })
    }

    pub fn validate(&self) -> Result<()> {
        validate_one(&self.manager, env!("MTLS_MANAGER_SHA256"), "manager")?;
        validate_one(&self.router, env!("MTLS_ROUTER_SHA256"), "router")
    }
}

fn validate_one(path: &PathBuf, expected_hash: &str, label: &str) -> Result<()> {
    let metadata =
        fs::metadata(path).map_err(|_| missing(format!("packaged {label} is missing")))?;
    if !metadata.is_file() {
        return Err(invalid(format!("packaged {label} is not a file")));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(invalid(format!("packaged {label} is not executable")));
        }
    }
    let bytes = fs::read(path).map_err(|_| invalid(format!("packaged {label} cannot be read")))?;
    let actual_hash = format!("{:x}", Sha256::digest(&bytes));
    if actual_hash != expected_hash {
        return Err(invalid(format!(
            "packaged {label} failed integrity validation"
        )));
    }
    validate_native(&bytes, label)
}

fn validate_native(bytes: &[u8], label: &str) -> Result<()> {
    let file = object::File::parse(bytes)
        .map_err(|_| invalid(format!("packaged {label} is not a native executable")))?;
    let architecture = match env!("MTLS_TARGET_TRIPLE").split('-').next() {
        Some("aarch64") => Architecture::Aarch64,
        Some("x86_64") => Architecture::X86_64,
        _ => return Err(invalid("desktop target architecture is unsupported")),
    };
    let format = if cfg!(target_os = "macos") {
        BinaryFormat::MachO
    } else if cfg!(windows) {
        BinaryFormat::Coff
    } else {
        BinaryFormat::Elf
    };
    if file.architecture() != architecture
        || file.format() != format
        || file.kind() != ObjectKind::Executable
    {
        return Err(invalid(format!(
            "packaged {label} does not match desktop target {}",
            env!("MTLS_TARGET_TRIPLE")
        )));
    }
    Ok(())
}

fn missing(message: impl Into<String>) -> CommandError {
    CommandError::new(
        "SIDECAR_MISSING",
        format!("{}; reinstall the application", message.into()),
    )
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(
        "SIDECAR_INVALID",
        format!("{}; reinstall the application", message.into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_sidecars_fail_closed() {
        let paths = SidecarPaths {
            manager: PathBuf::from("/definitely/missing/manager"),
            router: PathBuf::from("/definitely/missing/router"),
        };
        assert_eq!(paths.validate().unwrap_err().code, "SIDECAR_MISSING");
    }

    #[test]
    fn altered_executable_fails_integrity_before_execution() {
        let path = env::temp_dir().join(format!("mtls-sidecar-test-{}", uuid::Uuid::new_v4()));
        fs::write(&path, b"not an executable").expect("write fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("permissions");
        }
        let error = validate_one(&path, "00", "fixture").unwrap_err();
        let _ = fs::remove_file(path);
        assert_eq!(error.code, "SIDECAR_INVALID");
    }
}
