use super::super::process::{self, Identity as ProcessIdentity, ProcessError};
use super::service::CallContext;
use super::types::{
    valid_supervisor, Identity, OccupantError, RecoveryReason, Supervisor, SupervisorKind,
    SupervisorScope, Target, VerificationMode,
};

pub const WINDOWS_TCP_ROW_OWNER_PID_SIZE: usize = 24;

pub fn windows_socket_id(listen_addr: &str, pid: i32) -> String {
    format!("tcp4:{listen_addr}:{pid}")
}

pub fn windows_pid(pid: i32) -> Result<u32, ProcessError> {
    if pid <= 0 || pid as u64 > u32::MAX as u64 {
        return Err(ProcessError::NotFound);
    }
    Ok(pid as u32)
}

pub struct WindowsTargetDependencies {
    pub inspect_pid_owner: Box<dyn Fn(&CallContext, &str) -> Result<i32, OccupantError>>,
    pub services_for_pid: Box<dyn Fn(i32) -> Result<Vec<String>, OccupantError>>,
    pub process_session_id: Box<dyn Fn(i32) -> Result<u32, OccupantError>>,
    pub process_sid: Box<dyn Fn(i32) -> Result<String, OccupantError>>,
    pub current_sid: Box<dyn Fn() -> Result<String, OccupantError>>,
    pub inspect_process: Box<dyn Fn(i32) -> Result<ProcessIdentity, ProcessError>>,
    pub preflight_terminate: Box<dyn Fn(i32) -> Result<(), OccupantError>>,
}

pub fn inspect_windows_target(
    ctx: &CallContext,
    listen_addr: &str,
    deps: &WindowsTargetDependencies,
) -> Result<Target, OccupantError> {
    if ctx.err() {
        return Err(OccupantError::IdentityUnavailable);
    }
    let pid = (deps.inspect_pid_owner)(ctx, listen_addr)?;
    if ctx.err() {
        return Err(OccupantError::IdentityUnavailable);
    }
    let mut degraded = Target {
        mode: Some(VerificationMode::WindowsPidOnly),
        pid,
        listen_addr: listen_addr.to_owned(),
        ..Target::default()
    };
    let services = match (deps.services_for_pid)(pid) {
        Ok(services) => services,
        Err(_) => {
            if ctx.err() {
                return Err(OccupantError::IdentityUnavailable);
            }
            degraded.block_reason = Some(RecoveryReason::IdentityUnavailable);
            return Ok(degraded);
        }
    };
    if ctx.err() {
        return Err(OccupantError::IdentityUnavailable);
    }
    if !services.is_empty() {
        match normalize_supervisor_identifiers(&services) {
            Some(identifiers) => {
                degraded.supervisor = Some(Supervisor {
                    kind: SupervisorKind::WindowsService,
                    scope: SupervisorScope::System,
                    identifiers,
                });
                degraded.block_reason = Some(RecoveryReason::ServiceManaged);
            }
            None => degraded.block_reason = Some(RecoveryReason::ServiceManaged),
        }
        return Ok(degraded);
    }
    let session_id = match (deps.process_session_id)(pid) {
        Ok(session_id) => session_id,
        Err(_) => {
            if ctx.err() {
                return Err(OccupantError::IdentityUnavailable);
            }
            degraded.block_reason = Some(RecoveryReason::IdentityUnavailable);
            return Ok(degraded);
        }
    };
    if ctx.err() {
        return Err(OccupantError::IdentityUnavailable);
    }
    if session_id == 0 {
        degraded.block_reason = Some(RecoveryReason::IdentityUnavailable);
        return Ok(degraded);
    }
    let process_sid = match (deps.process_sid)(pid) {
        Ok(sid) if !sid.is_empty() => sid,
        _ => {
            if ctx.err() {
                return Err(OccupantError::IdentityUnavailable);
            }
            return preflight_windows_target(ctx, degraded, &deps.preflight_terminate);
        }
    };
    if ctx.err() {
        return Err(OccupantError::IdentityUnavailable);
    }
    let current_sid = match (deps.current_sid)() {
        Ok(sid) if !sid.is_empty() => sid,
        _ => {
            if ctx.err() {
                return Err(OccupantError::IdentityUnavailable);
            }
            return preflight_windows_target(ctx, degraded, &deps.preflight_terminate);
        }
    };
    if ctx.err() {
        return Err(OccupantError::IdentityUnavailable);
    }
    if process_sid != current_sid {
        degraded.block_reason = Some(RecoveryReason::DifferentUser);
        return Ok(degraded);
    }
    let identity = match (deps.inspect_process)(pid) {
        Ok(identity)
            if identity.pid == pid
                && !identity.started_at.is_empty()
                && !identity.executable.is_empty() =>
        {
            identity
        }
        _ => {
            if ctx.err() {
                return Err(OccupantError::IdentityUnavailable);
            }
            return preflight_windows_target(ctx, degraded, &deps.preflight_terminate);
        }
    };
    if ctx.err() {
        return Err(OccupantError::IdentityUnavailable);
    }
    let target = Target {
        mode: Some(VerificationMode::VerifiedIdentity),
        identity: Identity {
            listen_addr: listen_addr.to_owned(),
            network: "tcp4".into(),
            socket_id: windows_socket_id(listen_addr, pid),
            process: identity,
            user_id: process_sid,
        },
        pid,
        listen_addr: listen_addr.to_owned(),
        supervisor: None,
        block_reason: None,
    };
    preflight_windows_target(ctx, target, &deps.preflight_terminate)
}

fn preflight_windows_target(
    ctx: &CallContext,
    mut target: Target,
    preflight: &dyn Fn(i32) -> Result<(), OccupantError>,
) -> Result<Target, OccupantError> {
    let err = preflight(target.pid);
    if ctx.err() {
        return Err(OccupantError::IdentityUnavailable);
    }
    match err {
        Ok(()) => Ok(target),
        Err(OccupantError::PermissionDenied) => {
            target.block_reason = Some(RecoveryReason::InsufficientPrivilege);
            Ok(target)
        }
        Err(_) => {
            target.block_reason = Some(RecoveryReason::IdentityUnavailable);
            Ok(target)
        }
    }
}

pub fn normalize_supervisor_identifiers(values: &[String]) -> Option<Vec<String>> {
    if values.is_empty() || values.len() > super::types::MAX_SUPERVISOR_IDENTIFIERS {
        return None;
    }
    let mut result: Vec<String> = values
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    result.sort();
    let supervisor = Supervisor {
        kind: SupervisorKind::WindowsService,
        scope: SupervisorScope::System,
        identifiers: result.clone(),
    };
    valid_supervisor(&supervisor).then_some(result)
}

pub fn select_tcp4_listener_owner(
    buffer: &[u8],
    ip: [u8; 4],
    port: i32,
) -> Result<u32, OccupantError> {
    if buffer.len() < 4 || port <= 0 || port > 65535 {
        return Err(OccupantError::IdentityUnavailable);
    }
    let count = u32::from_le_bytes(buffer[..4].try_into().unwrap()) as u64;
    const ROW_OFFSET: u64 = 4;
    if count > (buffer.len() as u64 - ROW_OFFSET) / WINDOWS_TCP_ROW_OWNER_PID_SIZE as u64 {
        return Err(OccupantError::IdentityUnavailable);
    }
    let row_end = ROW_OFFSET + count * WINDOWS_TCP_ROW_OWNER_PID_SIZE as u64;
    let wanted = u32::from_le_bytes(ip);
    let mut owner = 0u32;
    let mut offset = ROW_OFFSET as usize;
    while offset < row_end as usize {
        let row = &buffer[offset..offset + WINDOWS_TCP_ROW_OWNER_PID_SIZE];
        if u32::from_le_bytes(row[0..4].try_into().unwrap()) != 2 {
            offset += WINDOWS_TCP_ROW_OWNER_PID_SIZE;
            continue;
        }
        let local_port = u16::from_be_bytes(row[8..10].try_into().unwrap()) as i32;
        if row[10] != 0
            || row[11] != 0
            || u32::from_le_bytes(row[12..16].try_into().unwrap()) != 0
            || u32::from_le_bytes(row[16..20].try_into().unwrap()) != 0
            || u32::from_le_bytes(row[20..24].try_into().unwrap()) == 0
        {
            return Err(OccupantError::IdentityUnavailable);
        }
        let local_address = u32::from_le_bytes(row[4..8].try_into().unwrap());
        if local_port == port && local_address == 0 {
            return Err(OccupantError::IdentityUnavailable);
        }
        if local_port != port || local_address != wanted {
            offset += WINDOWS_TCP_ROW_OWNER_PID_SIZE;
            continue;
        }
        if owner != 0 {
            return Err(OccupantError::IdentityUnavailable);
        }
        owner = u32::from_le_bytes(row[20..24].try_into().unwrap());
        offset += WINDOWS_TCP_ROW_OWNER_PID_SIZE;
    }
    if owner == 0 {
        return Err(OccupantError::NotFound);
    }
    Ok(owner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_windows_process(pid: i32) -> ProcessIdentity {
        ProcessIdentity {
            pid,
            started_at: "start".into(),
            executable: r"C:\router.exe".into(),
        }
    }

    fn deps(
        owner: Result<i32, OccupantError>,
        services: Result<Vec<String>, OccupantError>,
        session: Result<u32, OccupantError>,
        process_sid: Result<String, OccupantError>,
        current_sid: Result<String, OccupantError>,
        process: Result<ProcessIdentity, ProcessError>,
        preflight: Result<(), OccupantError>,
    ) -> WindowsTargetDependencies {
        WindowsTargetDependencies {
            inspect_pid_owner: Box::new(move |_, _| owner.clone()),
            services_for_pid: Box::new(move |_| services.clone()),
            process_session_id: Box::new(move |_| session.clone()),
            process_sid: Box::new(move |_| process_sid.clone()),
            current_sid: Box::new(move || current_sid.clone()),
            inspect_process: Box::new(move |_| process.clone()),
            preflight_terminate: Box::new(move |_| preflight.clone()),
        }
    }

    #[test]
    fn inspect_windows_target_complete_first() {
        let target = inspect_windows_target(
            &CallContext::unbounded(),
            "127.0.0.1:19099",
            &deps(
                Ok(42),
                Ok(Vec::new()),
                Ok(1),
                Ok("S-1-5-21-1".into()),
                Ok("S-1-5-21-1".into()),
                Ok(complete_windows_process(42)),
                Ok(()),
            ),
        )
        .unwrap();
        assert_eq!(target.mode, Some(VerificationMode::VerifiedIdentity));
        assert_eq!(target.pid, 42);
        assert_eq!(target.identity.user_id, "S-1-5-21-1");
        assert_eq!(target.identity.socket_id, "tcp4:127.0.0.1:19099:42");
    }

    #[test]
    fn inspect_windows_target_classifies_service_owner() {
        let target = inspect_windows_target(
            &CallContext::unbounded(),
            "127.0.0.1:19099",
            &deps(
                Ok(42),
                Ok(vec!["Beta".into(), "Alpha".into(), "Beta".into()]),
                Ok(1),
                Ok("S-1-5-18".into()),
                Ok("S-1-5-21-1".into()),
                Ok(complete_windows_process(42)),
                Ok(()),
            ),
        )
        .unwrap();
        assert_eq!(target.mode, Some(VerificationMode::WindowsPidOnly));
        assert_eq!(target.block_reason, Some(RecoveryReason::ServiceManaged));
        assert_eq!(
            target.supervisor.unwrap().identifiers,
            ["Alpha".to_owned(), "Beta".to_owned()]
        );
    }

    #[test]
    fn inspect_windows_target_does_not_degrade_owner_errors() {
        for error in [OccupantError::NotFound, OccupantError::IdentityUnavailable] {
            let err = inspect_windows_target(
                &CallContext::unbounded(),
                "127.0.0.1:19099",
                &deps(
                    Err(error),
                    Ok(Vec::new()),
                    Ok(1),
                    Ok(String::new()),
                    Ok(String::new()),
                    Ok(complete_windows_process(42)),
                    Ok(()),
                ),
            )
            .unwrap_err();
            assert_eq!(err, error);
        }
    }

    #[test]
    fn windows_pid_range() {
        assert_eq!(windows_pid(-1), Err(ProcessError::NotFound));
        assert_eq!(windows_pid(0), Err(ProcessError::NotFound));
        assert_eq!(windows_pid(42), Ok(42));
    }

    #[test]
    fn select_tcp4_listener_owner_picks_exact_listen_row() {
        let buffer = windows_tcp_table(&[
            tcp_row(5, [127, 0, 0, 1], 19099, [127, 0, 0, 2], 19100, 10),
            tcp_row(2, [127, 0, 0, 2], 19099, [0, 0, 0, 0], 0, 11),
            tcp_row(2, [127, 0, 0, 1], 19099, [0, 0, 0, 0], 0, 42),
            tcp_row(2, [127, 0, 0, 1], 19100, [0, 0, 0, 0], 0, 12),
        ]);
        assert_eq!(
            select_tcp4_listener_owner(&buffer, [127, 0, 0, 1], 19099).unwrap(),
            42
        );
    }

    #[test]
    fn select_tcp4_listener_owner_fails_closed_on_wildcard_and_duplicates() {
        let exact = tcp_row(2, [127, 0, 0, 1], 19099, [0, 0, 0, 0], 0, 42);
        assert_eq!(
            select_tcp4_listener_owner(
                &windows_tcp_table(&[tcp_row(2, [0, 0, 0, 0], 19099, [0, 0, 0, 0], 0, 42)]),
                [127, 0, 0, 1],
                19099
            ),
            Err(OccupantError::IdentityUnavailable)
        );
        assert_eq!(
            select_tcp4_listener_owner(&windows_tcp_table(&[exact, exact]), [127, 0, 0, 1], 19099),
            Err(OccupantError::IdentityUnavailable)
        );
        assert_eq!(
            select_tcp4_listener_owner(
                &windows_tcp_table(&[tcp_row(2, [127, 0, 0, 2], 19099, [0, 0, 0, 0], 0, 11)]),
                [127, 0, 0, 1],
                19099
            ),
            Err(OccupantError::NotFound)
        );
    }

    fn tcp_row(
        state: u32,
        local_ip: [u8; 4],
        local_port: u16,
        remote_ip: [u8; 4],
        remote_port: u32,
        pid: u32,
    ) -> [u8; WINDOWS_TCP_ROW_OWNER_PID_SIZE] {
        let mut row = [0u8; WINDOWS_TCP_ROW_OWNER_PID_SIZE];
        row[0..4].copy_from_slice(&state.to_le_bytes());
        row[4..8].copy_from_slice(&local_ip);
        row[8..10].copy_from_slice(&local_port.to_be_bytes());
        row[12..16].copy_from_slice(&remote_ip);
        row[16..20].copy_from_slice(&remote_port.to_le_bytes());
        row[20..24].copy_from_slice(&pid.to_le_bytes());
        row
    }

    fn windows_tcp_table(rows: &[[u8; WINDOWS_TCP_ROW_OWNER_PID_SIZE]]) -> Vec<u8> {
        let mut buffer = vec![0u8; 4 + rows.len() * WINDOWS_TCP_ROW_OWNER_PID_SIZE];
        buffer[..4].copy_from_slice(&(rows.len() as u32).to_le_bytes());
        for (index, row) in rows.iter().enumerate() {
            let start = 4 + index * WINDOWS_TCP_ROW_OWNER_PID_SIZE;
            buffer[start..start + WINDOWS_TCP_ROW_OWNER_PID_SIZE].copy_from_slice(row);
        }
        buffer
    }
}
