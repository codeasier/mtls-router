//! Cross-platform router process identity reading and validation.
//!
//! Mirrors Go `internal/manager/process` for the Rust image data plane.
//! Reads PID + start time + executable for any process and validates it
//! against expected identity and a managed binary path.
//!
//! This module is independent of `process_identity.rs`, which continues to
//! represent only the desktop parent process.

use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouterIdentity {
    pub pid: u32,
    pub started_at: String,
    pub executable: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouterProcessStatus {
    Genuine,
    Absent,
    Stale,
}

#[derive(Debug, thiserror::Error)]
pub enum RouterProcessError {
    #[error("process not found")]
    NotFound,
    #[error("process identity is malformed: {0}")]
    Malformed(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type RouterProcessResult<T> = Result<T, RouterProcessError>;

/// Inspect returns the current identity for `pid`.
pub fn inspect(pid: u32) -> RouterProcessResult<RouterIdentity> {
    if pid == 0 {
        return Err(RouterProcessError::NotFound);
    }
    let (started_at, executable) = inspect_platform(pid)?;
    let executable = normalize_executable(&executable)?;
    Ok(RouterIdentity {
        pid,
        started_at,
        executable,
    })
}

/// Validate checks PID, start identity, executable, and binary path.
/// Incomplete or inaccessible identity is stale.
pub fn validate(expected: &RouterIdentity, binary_path: &str) -> RouterProcessStatus {
    if expected.pid == 0
        || expected.started_at.is_empty()
        || expected.executable.is_empty()
        || binary_path.is_empty()
    {
        return RouterProcessStatus::Stale;
    }
    let live = match inspect(expected.pid) {
        Ok(id) => id,
        Err(RouterProcessError::NotFound) => return RouterProcessStatus::Absent,
        Err(_) => return RouterProcessStatus::Stale,
    };
    let stored_executable = match normalize_executable(&expected.executable) {
        Ok(p) => p,
        Err(_) => return RouterProcessStatus::Stale,
    };
    let stored_binary = match normalize_executable(binary_path) {
        Ok(p) => p,
        Err(_) => return RouterProcessStatus::Stale,
    };
    if !same_start_identity(&expected.started_at, &live.started_at)
        || !same_executable(&stored_executable, &live.executable)
        || !same_executable(&stored_executable, &stored_binary)
    {
        return RouterProcessStatus::Stale;
    }
    RouterProcessStatus::Genuine
}

/// NormalizeExecutable normalizes an executable identity path.
/// Linux's procfs suffix for a replaced running image is removed.
pub fn normalize_executable(path: &str) -> RouterProcessResult<String> {
    let path = path.trim().to_owned();
    #[cfg(target_os = "linux")]
    let path = path.trim_end_matches(" (deleted)").to_owned();
    #[cfg(windows)]
    let path = {
        let mut path = path;
        if path.starts_with(r"\\?\UNC\") {
            path = format!(r"\\{}", &path[r"\\?\UNC\".len()..]);
        } else if path.starts_with(r"\\?\") {
            path = path[r"\\?\".len()..].to_owned();
        }
        path
    };
    if path.is_empty() {
        return Err(RouterProcessError::Malformed(
            "empty executable path".into(),
        ));
    }
    let abs = Path::new(&path)
        .canonicalize()
        .or_else(|_| std::env::current_dir().map(|d| d.join(&path)))
        .map_err(RouterProcessError::Io)?;
    let cleaned = clean_path(&abs);
    Ok(cleaned)
}

fn clean_path(path: &Path) -> String {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    path.to_string_lossy().into_owned()
}

fn same_executable(left: &str, right: &str) -> bool {
    #[cfg(windows)]
    {
        left.eq_ignore_ascii_case(right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

// --- Platform-specific inspect and same_start_identity ---

#[cfg(target_os = "linux")]
fn inspect_platform(pid: u32) -> RouterProcessResult<(String, String)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            RouterProcessError::NotFound
        } else {
            RouterProcessError::Io(e)
        }
    })?;
    let close_paren = stat
        .rfind(") ")
        .ok_or_else(|| RouterProcessError::Malformed("invalid proc stat".into()))?;
    let fields: Vec<&str> = stat[close_paren + 2..].split_whitespace().collect();
    if fields.len() < 20 {
        return Err(RouterProcessError::Malformed("incomplete proc stat".into()));
    }
    let started_at = fields[19];
    started_at
        .parse::<u64>()
        .map_err(|_| RouterProcessError::Malformed("invalid proc start identity".into()))?;
    let executable = std::fs::read_link(format!("/proc/{pid}/exe")).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            RouterProcessError::NotFound
        } else {
            RouterProcessError::Io(e)
        }
    })?;
    Ok((
        started_at.to_owned(),
        executable.to_string_lossy().into_owned(),
    ))
}

#[cfg(target_os = "linux")]
fn same_start_identity(expected: &str, live: &str) -> bool {
    expected == live
}

#[cfg(target_os = "macos")]
fn inspect_platform(pid: u32) -> RouterProcessResult<(String, String)> {
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
        return Err(RouterProcessError::NotFound);
    }
    let info = unsafe { info.assume_init() };
    if info.pbi_pid == 0 {
        return Err(RouterProcessError::NotFound);
    }
    let started_at = format_timestamp(
        info.pbi_start_tvsec as i64,
        (info.pbi_start_tvusec * 1_000) as u32,
    )?;
    let mut buf = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let len = unsafe { libc::proc_pidpath(pid as i32, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if len <= 0 {
        return Err(RouterProcessError::Malformed(
            "empty process executable path".into(),
        ));
    }
    let executable = String::from_utf8_lossy(&buf[..len as usize]).into_owned();
    Ok((started_at, executable))
}

#[cfg(target_os = "macos")]
fn same_start_identity(expected: &str, live: &str) -> bool {
    use chrono::{DateTime, Utc};
    let live_time = match DateTime::parse_from_rfc3339(live) {
        Ok(t) => t.with_timezone(&Utc),
        Err(_) => return false,
    };
    if let Ok(expected_time) = DateTime::parse_from_rfc3339(expected) {
        return expected_time.with_timezone(&Utc) == live_time;
    }
    false
}

#[cfg(windows)]
fn inspect_platform(pid: u32) -> RouterProcessResult<(String, String)> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME},
        System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return Err(RouterProcessError::NotFound);
    }
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let ok = unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
    if ok == 0 {
        unsafe { CloseHandle(handle) };
        return Err(RouterProcessError::Malformed(
            "GetProcessTimes failed".into(),
        ));
    }
    let ticks = ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
    let unix_100ns = ticks
        .checked_sub(116_444_736_000_000_000)
        .ok_or_else(|| RouterProcessError::Malformed("invalid creation time".into()))?;
    let started_at = format_timestamp(
        (unix_100ns / 10_000_000) as i64,
        ((unix_100ns % 10_000_000) * 100) as u32,
    )?;
    use windows_sys::Win32::System::Threading::QueryFullProcessImageNameW;
    let mut buf = [0u16; 32768];
    let mut size = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size) };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return Err(RouterProcessError::Malformed(
            "QueryFullProcessImageName failed".into(),
        ));
    }
    let executable = String::from_utf16_lossy(&buf[..size as usize]);
    Ok((started_at, executable))
}

#[cfg(windows)]
fn same_start_identity(expected: &str, live: &str) -> bool {
    use chrono::{DateTime, Utc};
    let Ok(expected_time) = DateTime::parse_from_rfc3339(expected) else {
        return false;
    };
    let Ok(live_time) = DateTime::parse_from_rfc3339(live) else {
        return false;
    };
    expected_time.with_timezone(&Utc) == live_time.with_timezone(&Utc)
}

#[cfg(any(target_os = "macos", windows))]
fn format_timestamp(seconds: i64, nanos: u32) -> RouterProcessResult<String> {
    use chrono::{DateTime, SecondsFormat, Utc};
    let value = DateTime::<Utc>::from_timestamp(seconds, nanos)
        .ok_or_else(|| RouterProcessError::Malformed("invalid timestamp".into()))?;
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
    fn inspect_current_process() {
        let pid = std::process::id();
        let identity = inspect(pid).expect("inspect current process");
        assert_eq!(identity.pid, pid);
        assert!(!identity.started_at.is_empty());
        assert!(!identity.executable.is_empty());
    }

    #[test]
    fn inspect_nonexistent_pid() {
        let result = inspect(u32::MAX);
        assert!(matches!(result, Err(RouterProcessError::NotFound)));
    }

    #[test]
    fn validate_current_process_is_genuine() {
        let pid = std::process::id();
        let identity = inspect(pid).expect("inspect");
        let status = validate(&identity, &identity.executable);
        assert_eq!(status, RouterProcessStatus::Genuine);
    }

    #[test]
    fn validate_rejects_zero_pid() {
        let identity = RouterIdentity {
            pid: 0,
            started_at: "123".into(),
            executable: "/path".into(),
        };
        assert_eq!(validate(&identity, "/path"), RouterProcessStatus::Stale);
    }

    #[test]
    fn validate_rejects_empty_fields() {
        let identity = RouterIdentity {
            pid: 1,
            started_at: "".into(),
            executable: "/path".into(),
        };
        assert_eq!(validate(&identity, "/path"), RouterProcessStatus::Stale);
    }

    #[test]
    fn validate_rejects_mismatched_binary() {
        let pid = std::process::id();
        let identity = inspect(pid).expect("inspect");
        let status = validate(&identity, "/nonexistent/binary");
        assert_eq!(status, RouterProcessStatus::Stale);
    }

    #[test]
    fn validate_rejects_wrong_pid() {
        let pid = std::process::id();
        let identity = inspect(pid).expect("inspect");
        let wrong = RouterIdentity {
            pid,
            started_at: "0".into(),
            executable: identity.executable.clone(),
        };
        assert_eq!(
            validate(&wrong, &identity.executable),
            RouterProcessStatus::Stale
        );
    }

    #[test]
    fn normalize_executable_strips_deleted_suffix() {
        #[cfg(target_os = "linux")]
        {
            let result = normalize_executable("/usr/bin/foo (deleted)").unwrap_or_default();
            assert!(!result.ends_with(" (deleted)"));
        }
    }

    #[test]
    fn same_executable_platform_behavior() {
        #[cfg(windows)]
        {
            assert!(same_executable("C:\\Foo.exe", "c:\\foo.exe"));
        }
        #[cfg(not(windows))]
        {
            assert!(same_executable("/foo", "/foo"));
            assert!(!same_executable("/foo", "/Foo"));
        }
    }
}
