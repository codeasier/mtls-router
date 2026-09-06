use std::net::Ipv4Addr;

use super::super::process::{self, ProcessError};
use super::inspect::validate_address;
use super::service::CallContext;
use super::types::{Identity, OccupantError, Target, VerificationMode};

const DARWIN_SOCKET_INFO_SIZE: usize = 792;
const IPPROTO_TCP: i32 = 6;
const AF_INET: i32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DarwinTcp4Record {
    pub socket_id: u64,
    pub ip: [u8; 4],
    pub port: i32,
    pub state: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DarwinListenerMatch {
    Rejected,
    Exact,
    Wildcard,
}

pub fn decode_darwin_tcp4_record(info: &[u8]) -> Option<DarwinTcp4Record> {
    if info.len() < 348
        || i32::from_le_bytes(info[180..184].try_into().ok()?) != IPPROTO_TCP
        || i32::from_le_bytes(info[184..188].try_into().ok()?) != AF_INET
        || i32::from_le_bytes(info[256..260].try_into().ok()?) != 2
    {
        return None;
    }
    Some(DarwinTcp4Record {
        socket_id: u64::from_le_bytes(info[160..168].try_into().ok()?),
        ip: info[324..328].try_into().ok()?,
        port: u16::from_be_bytes(info[268..270].try_into().ok()?) as i32,
        state: i32::from_le_bytes(info[344..348].try_into().ok()?),
    })
}

pub fn match_darwin_tcp4_listener(
    record: DarwinTcp4Record,
    ip: Ipv4Addr,
    port: u16,
) -> DarwinListenerMatch {
    let record_ip = Ipv4Addr::from(record.ip);
    if record.port == port as i32 && record_ip.is_unspecified() {
        return DarwinListenerMatch::Wildcard;
    }
    if record.port == port as i32 && record_ip == ip && record.state == 1 {
        return DarwinListenerMatch::Exact;
    }
    DarwinListenerMatch::Rejected
}

#[cfg(target_os = "macos")]
pub fn inspect_native(ctx: &CallContext, listen_addr: &str) -> Result<Target, OccupantError> {
    let identity = inspect_darwin(ctx, listen_addr)?;
    Ok(Target {
        mode: Some(VerificationMode::VerifiedIdentity),
        pid: identity.process.pid,
        listen_addr: identity.listen_addr.clone(),
        identity,
        supervisor: None,
        block_reason: None,
    })
}

#[cfg(target_os = "macos")]
fn inspect_darwin(ctx: &CallContext, listen_addr: &str) -> Result<Identity, OccupantError> {
    let (ip, port) = validate_address(listen_addr)?;
    let pids = darwin_pids().map_err(|_| OccupantError::IdentityUnavailable)?;
    let mut matches = Vec::new();
    for pid in pids {
        if ctx.err() {
            return Err(OccupantError::IdentityUnavailable);
        }
        let Ok(fds) = darwin_fds(pid) else {
            continue;
        };
        for fd in fds {
            let Ok(info) = darwin_socket(pid, fd) else {
                continue;
            };
            let Some(record) = decode_darwin_tcp4_record(&info) else {
                continue;
            };
            match match_darwin_tcp4_listener(record, ip, port) {
                DarwinListenerMatch::Wildcard => return Err(OccupantError::IdentityUnavailable),
                DarwinListenerMatch::Exact => {}
                DarwinListenerMatch::Rejected => continue,
            }
            let process = process::inspect(pid).map_err(|_| OccupantError::IdentityUnavailable)?;
            let user_id = process::process_uid(pid).map_err(|error| {
                if error == ProcessError::NotFound {
                    OccupantError::IdentityUnavailable
                } else {
                    OccupantError::IdentityUnavailable
                }
            })?;
            if record.socket_id == 0 {
                return Err(OccupantError::IdentityUnavailable);
            }
            matches.push(Identity {
                listen_addr: listen_addr.to_owned(),
                network: "tcp4".into(),
                socket_id: format!("{:x}", record.socket_id),
                process,
                user_id,
            });
        }
    }
    match matches.len() {
        0 => Err(OccupantError::NotFound),
        1 => Ok(matches.remove(0)),
        _ => Err(OccupantError::IdentityUnavailable),
    }
}

#[cfg(target_os = "macos")]
const PROC_PIDFD_SOCKET_INFO: libc::c_int = 3;

#[cfg(target_os = "macos")]
fn darwin_pids() -> Result<Vec<i32>, OccupantError> {
    let mut pids = vec![0i32; 16 * 1024];
    let bytes =
        unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * 4) as i32) };
    if bytes <= 0 || bytes as usize > pids.len() * 4 {
        return Err(OccupantError::IdentityUnavailable);
    }
    let count = bytes as usize / 4;
    Ok(pids[..count]
        .iter()
        .copied()
        .filter(|pid| *pid > 0)
        .collect())
}

#[cfg(target_os = "macos")]
fn darwin_fds(pid: i32) -> Result<Vec<i32>, OccupantError> {
    let mut buffer = vec![0u8; 64 * 1024];
    let length = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDLISTFDS,
            0,
            buffer.as_mut_ptr().cast(),
            buffer.len() as i32,
        )
    };
    if length <= 0 || length as usize % 8 != 0 {
        return Err(OccupantError::IdentityUnavailable);
    }
    Ok(buffer[..length as usize]
        .chunks_exact(8)
        .filter(|chunk| {
            u32::from_le_bytes(chunk[4..8].try_into().unwrap()) == libc::PROX_FDTYPE_SOCKET as u32
        })
        .map(|chunk| i32::from_le_bytes(chunk[0..4].try_into().unwrap()))
        .collect())
}

#[cfg(target_os = "macos")]
fn darwin_socket(pid: i32, fd: i32) -> Result<Vec<u8>, OccupantError> {
    let mut buffer = vec![0u8; DARWIN_SOCKET_INFO_SIZE];
    let length = unsafe {
        libc::proc_pidfdinfo(
            pid,
            fd,
            PROC_PIDFD_SOCKET_INFO,
            buffer.as_mut_ptr().cast(),
            buffer.len() as i32,
        )
    };
    if length < 372 {
        return Err(OccupantError::IdentityUnavailable);
    }
    buffer.truncate(length as usize);
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn darwin_tcp4_test_record() -> Vec<u8> {
        let mut info = vec![0u8; 348];
        info[160..168].copy_from_slice(&0x1020304050607080u64.to_le_bytes());
        info[180..184].copy_from_slice(&(IPPROTO_TCP as u32).to_le_bytes());
        info[184..188].copy_from_slice(&(AF_INET as u32).to_le_bytes());
        info[256..260].copy_from_slice(&2u32.to_le_bytes());
        info[268..270].copy_from_slice(&20128u16.to_be_bytes());
        info[324..328].copy_from_slice(&Ipv4Addr::new(127, 0, 0, 1).octets());
        info[344..348].copy_from_slice(&1u32.to_le_bytes());
        info
    }

    #[test]
    fn decode_darwin_tcp4_loopback_listener() {
        let record = decode_darwin_tcp4_record(&darwin_tcp4_test_record()).expect("decode");
        assert_eq!(record.socket_id, 0x1020304050607080);
        assert_eq!(record.ip, [127, 0, 0, 1]);
        assert_eq!(record.port, 20128);
        assert_eq!(record.state, 1);
        assert_eq!(
            match_darwin_tcp4_listener(record, Ipv4Addr::new(127, 0, 0, 1), 20128),
            DarwinListenerMatch::Exact
        );
    }

    #[test]
    fn darwin_tcp4_record_rejection() {
        assert!(decode_darwin_tcp4_record(&vec![0; 347]).is_none());

        let mut unknown = darwin_tcp4_test_record();
        unknown[180..184].copy_from_slice(&17u32.to_le_bytes());
        assert!(decode_darwin_tcp4_record(&unknown).is_none());

        let mut wildcard = darwin_tcp4_test_record();
        wildcard[324..328].copy_from_slice(&[0, 0, 0, 0]);
        let record = decode_darwin_tcp4_record(&wildcard).expect("wildcard");
        assert_eq!(
            match_darwin_tcp4_listener(record, Ipv4Addr::new(127, 0, 0, 1), 20128),
            DarwinListenerMatch::Wildcard
        );

        let mut non_listener = darwin_tcp4_test_record();
        non_listener[344..348].copy_from_slice(&4u32.to_le_bytes());
        let record = decode_darwin_tcp4_record(&non_listener).expect("connected");
        assert_eq!(
            match_darwin_tcp4_listener(record, Ipv4Addr::new(127, 0, 0, 1), 20128),
            DarwinListenerMatch::Rejected
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_inspect_own_loopback_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let target = inspect_native(&CallContext::unbounded(), &addr).expect("inspect");
        assert_eq!(target.pid, std::process::id() as i32);
        assert_eq!(target.mode, Some(VerificationMode::VerifiedIdentity));
        assert_eq!(target.listen_addr, addr);
        assert!(!target.identity.socket_id.is_empty());
        drop(listener);
    }
}
