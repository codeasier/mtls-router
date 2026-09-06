use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use super::service::CallContext;
use super::types::{OccupantError, Target};

pub fn validate_address(value: &str) -> Result<(Ipv4Addr, u16), OccupantError> {
    let addr: SocketAddr = value
        .parse()
        .map_err(|_| OccupantError::IdentityUnavailable)?;
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_loopback() && addr.port() > 0 => Ok((ip, addr.port())),
        _ => Err(OccupantError::IdentityUnavailable),
    }
}

#[cfg(target_os = "macos")]
pub fn inspect_native(ctx: &CallContext, listen_addr: &str) -> Result<Target, OccupantError> {
    super::inspect_darwin::inspect_native(ctx, listen_addr)
}

#[cfg(target_os = "linux")]
pub fn inspect_native(ctx: &CallContext, listen_addr: &str) -> Result<Target, OccupantError> {
    super::inspect_linux::inspect_native(ctx, listen_addr)
}

#[cfg(windows)]
pub fn inspect_native(ctx: &CallContext, listen_addr: &str) -> Result<Target, OccupantError> {
    super::inspect_windows::inspect_native(ctx, listen_addr)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn inspect_native(_ctx: &CallContext, _listen_addr: &str) -> Result<Target, OccupantError> {
    Err(OccupantError::IdentityUnavailable)
}

#[cfg(windows)]
pub fn supports_pid_only_native() -> bool {
    true
}

#[cfg(not(windows))]
pub fn supports_pid_only_native() -> bool {
    false
}

#[cfg(windows)]
pub fn inspect_pid_owner_native(
    ctx: &CallContext,
    listen_addr: &str,
) -> Result<i32, OccupantError> {
    super::inspect_windows::inspect_pid_owner_native(ctx, listen_addr)
}

#[cfg(not(windows))]
pub fn inspect_pid_owner_native(
    _ctx: &CallContext,
    _listen_addr: &str,
) -> Result<i32, OccupantError> {
    Err(OccupantError::IdentityUnavailable)
}

#[cfg(windows)]
pub fn signal_pid_native(pid: i32) -> Result<(), OccupantError> {
    super::inspect_windows::signal_pid_native(pid)
}

#[cfg(not(windows))]
pub fn signal_pid_native(_pid: i32) -> Result<(), OccupantError> {
    Err(OccupantError::IdentityUnavailable)
}

#[cfg(unix)]
pub fn current_user_native() -> Result<String, OccupantError> {
    Ok(unsafe { libc::geteuid() }.to_string())
}

#[cfg(windows)]
pub fn current_user_native() -> Result<String, OccupantError> {
    super::inspect_windows::current_user_native()
}

#[cfg(not(any(unix, windows)))]
pub fn current_user_native() -> Result<String, OccupantError> {
    Err(OccupantError::IdentityUnavailable)
}
