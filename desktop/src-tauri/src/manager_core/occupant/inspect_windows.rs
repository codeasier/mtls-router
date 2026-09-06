#![cfg(windows)]

use super::super::process::{self, ProcessError};
use super::inspect::validate_address;
use super::service::CallContext;
use super::types::OccupantError;
use super::windows_target::{
    inspect_windows_target, select_tcp4_listener_owner, windows_pid, WindowsTargetDependencies,
};

pub fn inspect_native(
    ctx: &CallContext,
    listen_addr: &str,
) -> Result<super::types::Target, OccupantError> {
    inspect_windows_target(
        ctx,
        listen_addr,
        &WindowsTargetDependencies {
            inspect_pid_owner: Box::new(inspect_pid_owner_native),
            services_for_pid: Box::new(services_for_pid_native),
            process_session_id: Box::new(process_session_id_native),
            process_sid: Box::new(|pid| process_sid(pid)),
            current_sid: Box::new(current_user_native),
            inspect_process: Box::new(process::inspect),
            preflight_terminate: Box::new(preflight_terminate_pid_native),
        },
    )
}

pub fn inspect_pid_owner_native(
    ctx: &CallContext,
    listen_addr: &str,
) -> Result<i32, OccupantError> {
    if ctx.err() {
        return Err(OccupantError::IdentityUnavailable);
    }
    let (ip, port) = validate_address(listen_addr)?;
    let buffer = tcp_owner_table().map_err(|_| OccupantError::IdentityUnavailable)?;
    if ctx.err() {
        return Err(OccupantError::IdentityUnavailable);
    }
    select_tcp4_listener_owner(&buffer, ip.octets(), port as i32).map(|pid| pid as i32)
}

fn tcp_owner_table() -> Result<Vec<u8>, OccupantError> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, TCP_TABLE_OWNER_PID_ALL,
    };
    use windows_sys::Win32::Networking::WinSock::AF_INET;

    let mut size = 0u32;
    let first = unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            1,
            AF_INET as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    };
    if first != 122 || size < 4 {
        // 122 = ERROR_INSUFFICIENT_BUFFER
        return Err(OccupantError::IdentityUnavailable);
    }
    let mut buffer = vec![0u8; size as usize];
    let rc = unsafe {
        GetExtendedTcpTable(
            buffer.as_mut_ptr().cast(),
            &mut size,
            1,
            AF_INET as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    };
    if rc != 0 || size as usize > buffer.len() {
        return Err(OccupantError::IdentityUnavailable);
    }
    buffer.truncate(size as usize);
    Ok(buffer)
}

fn services_for_pid_native(pid: i32) -> Result<Vec<String>, OccupantError> {
    let _ = windows_pid(pid).map_err(|_| OccupantError::NotFound)?;
    // SCM enumeration stays fail-closed when the service manager is unavailable.
    match enumerate_services_for_pid(pid) {
        Ok(names) => Ok(names),
        Err(OccupantError::IdentityUnavailable) => Err(OccupantError::IdentityUnavailable),
        Err(error) => Err(error),
    }
}

fn enumerate_services_for_pid(_pid: i32) -> Result<Vec<String>, OccupantError> {
    // Keep the Windows native path present and fail closed when SCM APIs are
    // not linked in this build. Target classification is covered by
    // windows_target tests on all platforms.
    Err(OccupantError::IdentityUnavailable)
}

fn process_session_id_native(pid: i32) -> Result<u32, OccupantError> {
    let windows_pid = windows_pid(pid).map_err(|_| OccupantError::NotFound)?;
    let mut session_id = 0u32;
    let ok = unsafe {
        windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId(
            windows_pid,
            &mut session_id,
        )
    };
    if ok == 0 {
        return Err(OccupantError::IdentityUnavailable);
    }
    Ok(session_id)
}

fn process_sid(pid: i32) -> Result<String, OccupantError> {
    let _ = windows_pid(pid).map_err(|_| OccupantError::NotFound)?;
    Err(OccupantError::IdentityUnavailable)
}

pub fn current_user_native() -> Result<String, OccupantError> {
    Err(OccupantError::IdentityUnavailable)
}

fn preflight_terminate_pid_native(pid: i32) -> Result<(), OccupantError> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER},
        System::Threading::{OpenProcess, PROCESS_TERMINATE},
    };
    let windows_pid = windows_pid(pid).map_err(|_| OccupantError::NotFound)?;
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, windows_pid) };
    if handle.is_null() {
        return Err(map_windows_process_error(unsafe { GetLastError() }));
    }
    unsafe { CloseHandle(handle) };
    Ok(())
}

pub fn signal_pid_native(pid: i32) -> Result<(), OccupantError> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER},
        System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE},
    };
    let windows_pid = windows_pid(pid).map_err(|_| OccupantError::NotFound)?;
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, windows_pid) };
    if handle.is_null() {
        return Err(map_windows_process_error(unsafe { GetLastError() }));
    }
    let ok = unsafe { TerminateProcess(handle, 1) };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return Err(map_windows_process_error(unsafe { GetLastError() }));
    }
    Ok(())
}

fn map_windows_process_error(code: u32) -> OccupantError {
    use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER};
    if code == ERROR_ACCESS_DENIED {
        OccupantError::PermissionDenied
    } else if code == ERROR_INVALID_PARAMETER {
        OccupantError::NotFound
    } else {
        OccupantError::IdentityUnavailable
    }
}
