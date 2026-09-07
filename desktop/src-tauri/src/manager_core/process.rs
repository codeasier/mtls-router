#![allow(dead_code)]

//! Process identity used by occupant inspection and later legacy migration.
//!
//! Matches Go `internal/manager/process`: PID + start time + executable, with
//! incomplete or unreadable identity classified as stale.

use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Identity {
    pub pid: i32,
    pub started_at: String,
    pub executable: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Genuine,
    Absent,
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SignalKind {
    Interrupt,
    Kill,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessError {
    NotFound,
    IdentityMismatch,
    PermissionDenied,
    Io,
}

impl ProcessError {
    fn from_io(error: &io::Error) -> Self {
        match error.raw_os_error() {
            Some(libc::ESRCH) => Self::NotFound,
            _ if error.kind() == io::ErrorKind::NotFound => Self::NotFound,
            _ if error.kind() == io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            _ => Self::Io,
        }
    }
}

pub fn inspect(pid: i32) -> Result<Identity, ProcessError> {
    if pid <= 0 {
        return Err(ProcessError::NotFound);
    }
    let (started_at, executable) = inspect_platform(pid)?;
    let executable = normalize_executable(&executable).map_err(|_| ProcessError::Io)?;
    Ok(Identity {
        pid,
        started_at,
        executable,
    })
}

pub fn normalize_executable(path: &str) -> Result<String, ProcessError> {
    let mut path = path.trim().to_owned();
    if cfg!(target_os = "linux") {
        path = path.strip_suffix(" (deleted)").unwrap_or(&path).to_owned();
    }
    if cfg!(windows) {
        if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
            path = format!(r"\\{rest}");
        } else if let Some(rest) = path.strip_prefix(r"\\?\") {
            path = rest.to_owned();
        }
    }
    if path.is_empty() {
        return Err(ProcessError::Io);
    }
    let absolute = absolute_path(Path::new(&path))?;
    let cleaned = clean_path(&absolute);
    let dir = cleaned.parent().unwrap_or(Path::new("."));
    let base = cleaned
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| cleaned.clone());
    let resolved = match std::fs::canonicalize(dir) {
        Ok(dir) => dir.join(base),
        Err(_) => cleaned,
    };
    Ok(clean_path(&resolved).to_string_lossy().into_owned())
}

pub fn validate(expected: &Identity, binary_path: &str) -> Result<Status, ProcessError> {
    if expected.pid <= 0
        || expected.started_at.is_empty()
        || expected.executable.is_empty()
        || binary_path.is_empty()
    {
        return Ok(Status::Stale);
    }
    let live = match inspect(expected.pid) {
        Ok(live) => live,
        Err(ProcessError::NotFound) => return Ok(Status::Absent),
        Err(error) => return Err(error),
    };
    let stored_executable = match normalize_executable(&expected.executable) {
        Ok(value) => value,
        Err(_) => return Ok(Status::Stale),
    };
    let stored_binary = match normalize_executable(binary_path) {
        Ok(value) => value,
        Err(_) => return Ok(Status::Stale),
    };
    if !same_start_identity(&expected.started_at, &live.started_at)
        || !same_executable(&stored_executable, &live.executable)
        || !same_executable(&stored_executable, &stored_binary)
    {
        return Ok(Status::Stale);
    }
    Ok(Status::Genuine)
}

pub fn same_identity(left: &Identity, right: &Identity) -> Result<bool, ProcessError> {
    if left.pid <= 0
        || right.pid <= 0
        || left.started_at.is_empty()
        || right.started_at.is_empty()
        || left.executable.is_empty()
        || right.executable.is_empty()
    {
        return Ok(false);
    }
    let left_executable = normalize_executable(&left.executable)?;
    let right_executable = normalize_executable(&right.executable)?;
    Ok(left.pid == right.pid
        && same_start_identity(&left.started_at, &right.started_at)
        && same_executable(&left_executable, &right_executable))
}

pub fn signal_identity(expected: &Identity) -> Result<(), ProcessError> {
    signal(expected, &expected.executable, SignalKind::Kill)
}

/// Validates complete identity immediately before signaling. There is no
/// PID-only signaling API.
pub fn signal(
    expected: &Identity,
    binary_path: &str,
    kind: SignalKind,
) -> Result<(), ProcessError> {
    match validate(expected, binary_path)? {
        Status::Genuine => signal_process(expected.pid, kind),
        _ => Err(ProcessError::IdentityMismatch),
    }
}

fn same_executable(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf, ProcessError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().map_err(|_| ProcessError::Io)?;
    Ok(cwd.join(path))
}

fn clean_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

#[cfg(target_os = "linux")]
fn inspect_platform(pid: i32) -> Result<(String, String), ProcessError> {
    let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(value) => value,
        Err(error) => return Err(ProcessError::from_io(&error)),
    };
    let end = stat.rfind(") ").ok_or(ProcessError::Io)?;
    let started_at = stat[end + 2..]
        .split_whitespace()
        .nth(19)
        .filter(|value| value.parse::<u64>().is_ok())
        .ok_or(ProcessError::Io)?
        .to_owned();
    let executable = match std::fs::read_link(format!("/proc/{pid}/exe")) {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(error) => return Err(ProcessError::from_io(&error)),
    };
    Ok((started_at, executable))
}

#[cfg(target_os = "linux")]
fn same_start_identity(expected: &str, live: &str) -> bool {
    expected == live
}

#[cfg(target_os = "macos")]
fn inspect_platform(pid: i32) -> Result<(String, String), ProcessError> {
    let info = proc_bsdinfo(pid)?;
    let executable = procargs2(pid)?;
    let start = chrono::DateTime::<chrono::Utc>::from_timestamp(
        info.pbi_start_tvsec as i64,
        (info.pbi_start_tvusec * 1000) as u32,
    )
    .ok_or(ProcessError::Io)?;
    Ok((
        start.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        executable,
    ))
}

#[cfg(target_os = "macos")]
fn same_start_identity(expected: &str, live: &str) -> bool {
    let Ok(live_time) = chrono::DateTime::parse_from_rfc3339(live) else {
        return false;
    };
    if let Ok(expected_time) = chrono::DateTime::parse_from_rfc3339(expected) {
        return expected_time == live_time;
    }
    chrono::NaiveDateTime::parse_from_str(expected, "%a %b %e %H:%M:%S %Y")
        .ok()
        .and_then(|naive| naive.and_local_timezone(chrono::Local).single())
        .is_some_and(|legacy| legacy.timestamp() == live_time.timestamp())
}

#[cfg(target_os = "macos")]
fn proc_bsdinfo(pid: i32) -> Result<libc::proc_bsdinfo, ProcessError> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if read != size {
        return Err(ProcessError::from_io(&io::Error::last_os_error()));
    }
    let info = unsafe { info.assume_init() };
    if info.pbi_pid == 0 {
        return Err(ProcessError::NotFound);
    }
    Ok(info)
}

#[cfg(target_os = "macos")]
const KERN_PROCARGS2: libc::c_int = 49;

#[cfg(target_os = "macos")]
fn procargs2(pid: i32) -> Result<String, ProcessError> {
    let mut name = [libc::CTL_KERN, KERN_PROCARGS2, pid];
    let mut size = 0usize;
    let probe = unsafe {
        libc::sysctl(
            name.as_mut_ptr(),
            name.len() as u32,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if probe != 0 || size <= 4 {
        return Err(ProcessError::from_io(&io::Error::last_os_error()));
    }
    let mut buffer = vec![0u8; size];
    let rc = unsafe {
        libc::sysctl(
            name.as_mut_ptr(),
            name.len() as u32,
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return Err(ProcessError::from_io(&io::Error::last_os_error()));
    }
    buffer.truncate(size);
    if buffer.len() <= 4 || u32::from_le_bytes(buffer[..4].try_into().unwrap()) == 0 {
        return Err(ProcessError::Io);
    }
    let executable = buffer[4..]
        .split(|byte| *byte == 0)
        .next()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .filter(|value| !value.is_empty())
        .ok_or(ProcessError::Io)?;
    Ok(executable.to_owned())
}

#[cfg(target_os = "macos")]
pub(crate) fn process_uid(pid: i32) -> Result<String, ProcessError> {
    Ok(proc_bsdinfo(pid)?.pbi_uid.to_string())
}

#[cfg(windows)]
fn inspect_platform(pid: i32) -> Result<(String, String), ProcessError> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_GEN_FAILURE, ERROR_INVALID_PARAMETER},
        System::Threading::{
            GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
    };

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32) };
    if handle.is_null() {
        let error = unsafe { GetLastError() };
        return Err(
            if error == ERROR_INVALID_PARAMETER || error == ERROR_GEN_FAILURE {
                ProcessError::NotFound
            } else {
                ProcessError::Io
            },
        );
    }
    let mut creation = windows_sys::Win32::Foundation::FILETIME::default();
    let mut exit = windows_sys::Win32::Foundation::FILETIME::default();
    let mut kernel = windows_sys::Win32::Foundation::FILETIME::default();
    let mut user = windows_sys::Win32::Foundation::FILETIME::default();
    let times_ok =
        unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
    let mut name = vec![0u16; 32768];
    let mut name_len = name.len() as u32;
    let name_ok =
        unsafe { QueryFullProcessImageNameW(handle, 0, name.as_mut_ptr(), &mut name_len) };
    unsafe { CloseHandle(handle) };
    if times_ok == 0 || name_ok == 0 {
        let error = unsafe { GetLastError() };
        return Err(if error == ERROR_GEN_FAILURE {
            ProcessError::NotFound
        } else {
            ProcessError::Io
        });
    }
    let ticks = ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
    let unix_100ns = ticks
        .checked_sub(116_444_736_000_000_000)
        .ok_or(ProcessError::Io)?;
    let start = chrono::DateTime::<chrono::Utc>::from_timestamp(
        (unix_100ns / 10_000_000) as i64,
        ((unix_100ns % 10_000_000) * 100) as u32,
    )
    .ok_or(ProcessError::Io)?;
    let executable = String::from_utf16_lossy(&name[..name_len as usize]);
    Ok((
        start.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        executable,
    ))
}

#[cfg(windows)]
fn same_start_identity(expected: &str, live: &str) -> bool {
    let (Ok(expected_time), Ok(live_time)) = (
        chrono::DateTime::parse_from_rfc3339(expected),
        chrono::DateTime::parse_from_rfc3339(live),
    ) else {
        return false;
    };
    expected_time == live_time
}

#[cfg(unix)]
fn signal_process(pid: i32, kind: SignalKind) -> Result<(), ProcessError> {
    let sig = match kind {
        SignalKind::Interrupt => libc::SIGINT,
        SignalKind::Kill => libc::SIGKILL,
    };
    let rc = unsafe { libc::kill(pid, sig) };
    if rc == 0 {
        return Ok(());
    }
    Err(ProcessError::from_io(&io::Error::last_os_error()))
}

#[cfg(windows)]
fn signal_process(pid: i32, kind: SignalKind) -> Result<(), ProcessError> {
    if kind == SignalKind::Interrupt {
        return signal_interrupt_windows(pid);
    }
    signal_kill_windows(pid)
}

#[cfg(windows)]
fn signal_interrupt_windows(pid: i32) -> Result<(), ProcessError> {
    use windows_sys::Win32::System::Console::GenerateConsoleCtrlEvent;
    const CTRL_BREAK_EVENT: u32 = 1;
    let ok = unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid as u32) };
    if ok == 0 {
        return Err(ProcessError::from_io(&io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(windows)]
fn signal_kill_windows(pid: i32) -> Result<(), ProcessError> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER},
        System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE},
    };

    let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid as u32) };
    if handle.is_null() {
        let error = unsafe { GetLastError() };
        return Err(if error == ERROR_INVALID_PARAMETER {
            ProcessError::NotFound
        } else if error == ERROR_ACCESS_DENIED {
            ProcessError::PermissionDenied
        } else {
            ProcessError::Io
        });
    }
    let ok = unsafe { TerminateProcess(handle, 1) };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return Err(ProcessError::Io);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_and_validate_current_process() {
        let identity = inspect(std::process::id() as i32).expect("inspect current");
        let executable = std::env::current_exe()
            .expect("current exe")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            validate(&identity, &executable).expect("validate"),
            Status::Genuine
        );
        assert!(same_identity(&identity, &identity).expect("same"));
    }

    #[test]
    fn validate_rejects_incomplete_and_mismatched_identity() {
        let identity = inspect(std::process::id() as i32).expect("inspect current");
        let executable = std::env::current_exe()
            .expect("current exe")
            .to_string_lossy()
            .into_owned();
        let cases = [
            Identity {
                pid: identity.pid,
                started_at: String::new(),
                executable: String::new(),
            },
            Identity {
                pid: identity.pid,
                started_at: format!("{}-reused", identity.started_at),
                executable: identity.executable.clone(),
            },
            Identity {
                pid: identity.pid,
                started_at: identity.started_at.clone(),
                executable: std::env::temp_dir()
                    .join("other")
                    .to_string_lossy()
                    .into_owned(),
            },
        ];
        for expected in cases {
            assert_eq!(
                validate(&expected, &executable).expect("validate"),
                Status::Stale,
                "{expected:?}"
            );
        }
    }

    #[test]
    fn validate_missing_process_is_absent() {
        assert_eq!(
            validate(
                &Identity {
                    pid: (1 << 30) - 1,
                    started_at: "missing".into(),
                    executable: "/missing".into(),
                },
                "/missing"
            )
            .expect("validate"),
            Status::Absent
        );
    }

    #[test]
    fn same_identity_requires_every_field() {
        let identity = inspect(std::process::id() as i32).expect("inspect current");
        let changed = [
            Identity {
                pid: identity.pid + 1,
                started_at: identity.started_at.clone(),
                executable: identity.executable.clone(),
            },
            Identity {
                pid: identity.pid,
                started_at: format!("{}-changed", identity.started_at),
                executable: identity.executable.clone(),
            },
            Identity {
                pid: identity.pid,
                started_at: identity.started_at.clone(),
                executable: std::env::temp_dir()
                    .join("other")
                    .to_string_lossy()
                    .into_owned(),
            },
            Identity {
                pid: 0,
                started_at: String::new(),
                executable: String::new(),
            },
        ];
        for value in changed {
            assert!(
                !same_identity(&identity, &value).expect("compare"),
                "{value:?}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn darwin_same_start_identity_rfc3339_nano_precision() {
        assert!(same_start_identity(
            "2026-08-04T16:11:31.349861Z",
            "2026-08-04T16:11:31.349861Z"
        ));
        assert!(same_start_identity(
            "2026-08-04T16:11:31.349860000Z",
            "2026-08-04T16:11:31.34986Z"
        ));
        assert!(same_start_identity(
            "2026-08-04T16:11:31Z",
            "2026-08-04T16:11:31.000000Z"
        ));
        assert!(!same_start_identity(
            "2026-08-04T16:11:31.349861Z",
            "2026-08-04T16:11:32.349861Z"
        ));
        assert!(!same_start_identity(
            "2026-08-04T16:11:31.349861Z",
            "12345678"
        ));
    }

    #[test]
    fn signal_never_trusts_pid_alone() {
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("spawn helper");
        let pid = child.id() as i32;
        let identity = inspect(pid).expect("inspect helper");
        let mut forged = identity.clone();
        forged.started_at.push_str("-reused");
        assert_eq!(
            signal_identity(&forged),
            Err(ProcessError::IdentityMismatch)
        );
        assert!(inspect(pid).is_ok(), "forged signal stopped process");
        signal_identity(&identity).expect("genuine signal");
        let _ = child.wait();
    }
}
