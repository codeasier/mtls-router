use crate::error::{CommandError, Result};
#[cfg(any(target_os = "macos", windows))]
use chrono::{DateTime, SecondsFormat, Utc};
use std::{env, fs, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub started_at: String,
    pub executable: PathBuf,
}

pub fn current() -> Result<ProcessIdentity> {
    let pid = std::process::id();
    let executable = env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|_| CommandError::new("SIDECAR_INVALID", "cannot identify desktop executable"))?;
    let started_at = started_at(pid)?;
    if started_at.is_empty() || executable.to_string_lossy().contains(['\r', '\n']) {
        return Err(CommandError::new(
            "SIDECAR_INVALID",
            "desktop process identity is invalid",
        ));
    }
    Ok(ProcessIdentity {
        pid,
        started_at,
        executable,
    })
}

#[cfg(target_os = "linux")]
fn started_at(pid: u32) -> Result<String> {
    let value = fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|_| CommandError::new("SIDECAR_INVALID", "cannot read desktop start identity"))?;
    let end = value.rfind(") ").ok_or_else(|| {
        CommandError::new("SIDECAR_INVALID", "desktop start identity is malformed")
    })?;
    value[end + 2..]
        .split_whitespace()
        .nth(19)
        .filter(|value| value.parse::<u64>().is_ok())
        .map(str::to_owned)
        .ok_or_else(|| CommandError::new("SIDECAR_INVALID", "desktop start identity is incomplete"))
}

#[cfg(target_os = "macos")]
fn started_at(pid: u32) -> Result<String> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if read != size {
        return Err(CommandError::new(
            "SIDECAR_INVALID",
            "cannot read desktop start identity",
        ));
    }
    let info = unsafe { info.assume_init() };
    format_timestamp(
        info.pbi_start_tvsec as i64,
        (info.pbi_start_tvusec * 1_000) as u32,
    )
}

#[cfg(windows)]
fn started_at(pid: u32) -> Result<String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME},
        System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return Err(CommandError::new(
            "SIDECAR_INVALID",
            "cannot open desktop process identity",
        ));
    }
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let ok = unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return Err(CommandError::new(
            "SIDECAR_INVALID",
            "cannot read desktop start identity",
        ));
    }
    let ticks = ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
    let unix_100ns = ticks.checked_sub(116_444_736_000_000_000).ok_or_else(|| {
        CommandError::new("SIDECAR_INVALID", "desktop start identity is malformed")
    })?;
    format_timestamp(
        (unix_100ns / 10_000_000) as i64,
        ((unix_100ns % 10_000_000) * 100) as u32,
    )
}

#[cfg(any(target_os = "macos", windows))]
fn format_timestamp(seconds: i64, nanos: u32) -> Result<String> {
    let value = DateTime::<Utc>::from_timestamp(seconds, nanos).ok_or_else(|| {
        CommandError::new("SIDECAR_INVALID", "desktop start identity is malformed")
    })?;
    let precision = if nanos == 0 {
        SecondsFormat::Secs
    } else {
        SecondsFormat::Nanos
    };
    Ok(value.to_rfc3339_opts(precision, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_identity_is_complete() {
        let identity = current().expect("identity");
        assert_eq!(identity.pid, std::process::id());
        assert!(!identity.started_at.is_empty());
        assert!(identity.executable.is_absolute());
    }
}
