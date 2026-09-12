#![cfg(windows)]

use super::super::process;
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

fn enumerate_services_for_pid(pid: i32) -> Result<Vec<String>, OccupantError> {
    use windows_sys::Win32::{
        Foundation::{GetLastError, ERROR_MORE_DATA},
        System::Services::{
            CloseServiceHandle, EnumServicesStatusExW, OpenSCManagerW,
            ENUM_SERVICE_STATUS_PROCESSW, SC_ENUM_PROCESS_INFO, SC_MANAGER_ENUMERATE_SERVICE,
            SERVICE_STATE_ALL, SERVICE_WIN32,
        },
    };
    let manager = unsafe {
        OpenSCManagerW(
            std::ptr::null(),
            std::ptr::null(),
            SC_MANAGER_ENUMERATE_SERVICE,
        )
    };
    if manager.is_null() {
        return Err(OccupantError::IdentityUnavailable);
    }
    let result = (|| {
        let mut names = Vec::new();
        let mut resume = 0;
        // SCM caps a page at 256 KiB. Word alignment is required for its structs.
        let mut buffer = vec![0usize; 256 * 1024 / std::mem::size_of::<usize>()];
        for _ in 0..128 {
            let previous = resume;
            let mut needed = 0;
            let mut count = 0;
            let ok = unsafe {
                EnumServicesStatusExW(
                    manager,
                    SC_ENUM_PROCESS_INFO,
                    SERVICE_WIN32,
                    SERVICE_STATE_ALL,
                    buffer.as_mut_ptr().cast(),
                    256 * 1024,
                    &mut needed,
                    &mut count,
                    &mut resume,
                    std::ptr::null(),
                )
            };
            let error = if ok == 0 {
                unsafe { GetLastError() }
            } else {
                0
            };
            if error != 0 && error != ERROR_MORE_DATA {
                return Err(OccupantError::IdentityUnavailable);
            }
            if count as usize > 256 * 1024 / std::mem::size_of::<ENUM_SERVICE_STATUS_PROCESSW>() {
                return Err(OccupantError::IdentityUnavailable);
            }
            let entries = unsafe {
                std::slice::from_raw_parts(
                    buffer.as_ptr().cast::<ENUM_SERVICE_STATUS_PROCESSW>(),
                    count as usize,
                )
            };
            for entry in entries {
                if entry.ServiceStatusProcess.dwProcessId != pid as u32 {
                    continue;
                }
                if entry.lpServiceName.is_null() {
                    return Err(OccupantError::IdentityUnavailable);
                }
                let mut len = 0;
                while len <= 256 && unsafe { *entry.lpServiceName.add(len) } != 0 {
                    len += 1;
                }
                if len == 0 || len > 256 {
                    return Err(OccupantError::IdentityUnavailable);
                }
                names.push(
                    String::from_utf16(unsafe {
                        std::slice::from_raw_parts(entry.lpServiceName, len)
                    })
                    .map_err(|_| OccupantError::IdentityUnavailable)?,
                );
            }
            if ok != 0 {
                return Ok(names);
            }
            if resume == previous {
                return Err(OccupantError::IdentityUnavailable);
            }
        }
        Err(OccupantError::IdentityUnavailable)
    })();
    unsafe { CloseServiceHandle(manager) };
    result
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
    let pid = windows_pid(pid).map_err(|_| OccupantError::NotFound)?;
    crate::windows_security::process_sid(pid).map_err(|_| OccupantError::IdentityUnavailable)
}

pub fn current_user_native() -> Result<String, OccupantError> {
    crate::windows_security::current_sid().map_err(|_| OccupantError::IdentityUnavailable)
}

fn preflight_terminate_pid_native(pid: i32) -> Result<(), OccupantError> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError},
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
        Foundation::{CloseHandle, GetLastError},
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

#[cfg(test)]
mod tests {
    use super::super::types::{RecoveryReason, VerificationMode};
    use super::*;

    #[test]
    fn native_windows_scm_and_sid_resolve_own_listener() {
        let pid = std::process::id() as i32;
        assert!(services_for_pid_native(pid).unwrap().is_empty());
        assert_eq!(process_sid(pid).unwrap(), current_user_native().unwrap());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let target = inspect_native(&CallContext::unbounded(), &address).unwrap();
        assert_eq!(target.pid, pid);
        // Hosted CI can run in session zero; that must remain protected.
        if process_session_id_native(pid).unwrap() == 0 {
            assert_eq!(
                target.block_reason,
                Some(RecoveryReason::IdentityUnavailable)
            );
        } else {
            assert_eq!(target.mode, Some(VerificationMode::VerifiedIdentity));
            assert_eq!(target.block_reason, None);
            assert_eq!(target.identity.user_id, current_user_native().unwrap());
        }
    }
}
