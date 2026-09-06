use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};

use super::super::process::{self, Identity as ProcessIdentity, ProcessError};
use super::inspect::validate_address;
use super::service::CallContext;
use super::types::{
    valid_systemd_unit, Identity, OccupantError, RecoveryReason, Supervisor, SupervisorKind,
    SupervisorScope, Target, VerificationMode,
};

const MAX_PROC_CGROUP_SIZE: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemdCgroupState {
    Uncertain,
    Conclusive,
    Supervised,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemdCgroupResult {
    pub state: SystemdCgroupState,
    pub supervisor: Option<Supervisor>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcListener {
    pub inode: String,
    pub ip: IpAddr,
    pub port: u16,
}

#[cfg(target_os = "linux")]
pub fn inspect_native(ctx: &CallContext, listen_addr: &str) -> Result<Target, OccupantError> {
    inspect_linux(ctx, listen_addr, Path::new("/proc"), process::inspect)
}

pub fn inspect_linux(
    ctx: &CallContext,
    listen_addr: &str,
    proc_root: &Path,
    inspect_process: impl Fn(i32) -> Result<ProcessIdentity, ProcessError>,
) -> Result<Target, OccupantError> {
    let (ip, port) = validate_address(listen_addr)?;
    let listeners = read_proc_listeners(proc_root, IpAddr::V4(ip), port)
        .map_err(|_| OccupantError::IdentityUnavailable)?;
    if listeners.is_empty() {
        return Err(OccupantError::NotFound);
    }
    if listeners.len() != 1 {
        return Err(OccupantError::IdentityUnavailable);
    }
    let entries = std::fs::read_dir(proc_root).map_err(|_| OccupantError::IdentityUnavailable)?;
    let want = format!("socket:[{}]", listeners[0].inode);
    let mut owners = BTreeSet::new();
    for entry in entries {
        if ctx.err() {
            return Err(OccupantError::IdentityUnavailable);
        }
        let Ok(entry) = entry else {
            continue;
        };
        let pid = match entry.file_name().to_string_lossy().parse::<i32>() {
            Ok(pid) if pid > 0 && entry.path().is_dir() => pid,
            _ => continue,
        };
        let fd_dir = entry.path().join("fd");
        let Ok(fds) = std::fs::read_dir(fd_dir) else {
            continue;
        };
        for fd in fds.flatten() {
            if std::fs::read_link(fd.path())
                .ok()
                .and_then(|target| target.to_str().map(|value| value == want))
                == Some(true)
            {
                owners.insert(pid);
            }
        }
    }
    if owners.len() != 1 {
        return Err(OccupantError::IdentityUnavailable);
    }
    let pid = *owners.iter().next().expect("single owner");
    let user_id = match proc_effective_uid(proc_root, pid) {
        Ok(user_id) => user_id,
        Err(error) if error == OccupantError::NotFound => return Err(OccupantError::NotFound),
        Err(_) => return Err(OccupantError::IdentityUnavailable),
    };
    let identity = match inspect_process(pid) {
        Ok(identity) => identity,
        Err(ProcessError::NotFound) => return Err(OccupantError::NotFound),
        Err(_) => return Err(OccupantError::IdentityUnavailable),
    };
    let mut target = Target {
        mode: Some(VerificationMode::VerifiedIdentity),
        identity: Identity {
            listen_addr: listen_addr.to_owned(),
            network: "tcp4".into(),
            socket_id: listeners[0].inode.clone(),
            process: identity,
            user_id,
        },
        pid,
        listen_addr: listen_addr.to_owned(),
        supervisor: None,
        block_reason: None,
    };
    let cgroup = read_systemd_cgroup(&proc_root.join(pid.to_string()).join("cgroup"))?;
    match cgroup.state {
        SystemdCgroupState::Supervised => target.supervisor = cgroup.supervisor,
        SystemdCgroupState::Conclusive => {}
        SystemdCgroupState::Uncertain => {
            target.block_reason = Some(RecoveryReason::IdentityUnavailable)
        }
    }
    Ok(target)
}

pub fn read_systemd_cgroup(path: &Path) -> Result<SystemdCgroupResult, OccupantError> {
    match std::fs::read(path) {
        Ok(data) if data.len() > MAX_PROC_CGROUP_SIZE => Ok(SystemdCgroupResult {
            state: SystemdCgroupState::Uncertain,
            supervisor: None,
        }),
        Ok(data) => Ok(parse_systemd_cgroup(&data)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if std::fs::metadata(path.parent().unwrap_or(path)).is_err() {
                Err(OccupantError::NotFound)
            } else {
                Ok(SystemdCgroupResult {
                    state: SystemdCgroupState::Uncertain,
                    supervisor: None,
                })
            }
        }
        Err(_) => Ok(SystemdCgroupResult {
            state: SystemdCgroupState::Uncertain,
            supervisor: None,
        }),
    }
}

pub fn parse_systemd_cgroup(data: &[u8]) -> SystemdCgroupResult {
    if data.is_empty() || data.len() > MAX_PROC_CGROUP_SIZE || std::str::from_utf8(data).is_err() {
        return uncertain();
    }
    let mut text = String::from_utf8_lossy(data).into_owned();
    if text.ends_with('\n') {
        text.pop();
    }
    if text.is_empty() {
        return uncertain();
    }
    let mut candidates = BTreeMap::new();
    let mut seen_hierarchies = BTreeSet::new();
    let mut seen_controllers = BTreeSet::new();
    let mut unclassified = false;
    for line in text.split('\n') {
        let mut parts = line.splitn(3, ':');
        let (Some(hierarchy_text), Some(controllers_text), Some(path)) =
            (parts.next(), parts.next(), parts.next())
        else {
            return uncertain();
        };
        if hierarchy_text.is_empty() || has_control(controllers_text) || has_control(path) {
            return uncertain();
        }
        let Ok(hierarchy) = hierarchy_text.parse::<u64>() else {
            return uncertain();
        };
        let Some(controllers) = parse_cgroup_controllers(controllers_text) else {
            return uncertain();
        };
        if !valid_cgroup_path(path) {
            return uncertain();
        }
        if (hierarchy == 0) != controllers.is_empty() {
            return uncertain();
        }
        if !seen_hierarchies.insert(hierarchy) {
            return uncertain();
        }
        for controller in &controllers {
            if !seen_controllers.insert(controller.clone()) {
                return uncertain();
            }
        }
        let selected = if hierarchy == 0 {
            controllers.is_empty()
        } else {
            controllers.contains("name=systemd")
        };
        if !selected {
            continue;
        }
        let path_result = classify_systemd_cgroup_path(path);
        match path_result.state {
            SystemdCgroupState::Supervised => {
                let supervisor = path_result.supervisor.expect("supervised");
                let key = format!(
                    "{:?}\0{}",
                    supervisor.kind,
                    supervisor.identifiers.first().cloned().unwrap_or_default()
                );
                candidates.insert(key, supervisor);
            }
            SystemdCgroupState::Conclusive => unclassified = true,
            SystemdCgroupState::Uncertain => return uncertain(),
        }
    }
    if candidates.is_empty() {
        return SystemdCgroupResult {
            state: SystemdCgroupState::Conclusive,
            supervisor: None,
        };
    }
    if unclassified || candidates.len() != 1 {
        return uncertain();
    }
    let supervisor = candidates.into_values().next().expect("one candidate");
    if super::types::valid_supervisor(&supervisor) {
        SystemdCgroupResult {
            state: SystemdCgroupState::Supervised,
            supervisor: Some(supervisor),
        }
    } else {
        uncertain()
    }
}

fn classify_systemd_cgroup_path(path: &str) -> SystemdCgroupResult {
    if path == "/" {
        return SystemdCgroupResult {
            state: SystemdCgroupState::Conclusive,
            supervisor: None,
        };
    }
    let components: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if components.first() == Some(&"user.slice") {
        classify_user_service_path(&components)
    } else {
        classify_system_service_path(&components)
    }
}

fn classify_system_service_path(components: &[&str]) -> SystemdCgroupResult {
    let Some(service_indexes) = systemd_service_indexes(components, 0) else {
        return uncertain();
    };
    if !valid_systemd_slice_components(components) {
        return uncertain();
    }
    if service_indexes.is_empty() {
        return SystemdCgroupResult {
            state: SystemdCgroupState::Conclusive,
            supervisor: None,
        };
    }
    if service_indexes.len() != 1 {
        return uncertain();
    }
    let service_index = service_indexes[0];
    if service_index == 0 || !only_slice_ancestors(&components[..service_index]) {
        return uncertain();
    }
    supervised(
        SupervisorKind::SystemdSystem,
        SupervisorScope::System,
        components[service_index],
    )
}

fn classify_user_service_path(components: &[&str]) -> SystemdCgroupResult {
    let Some(service_indexes) = systemd_service_indexes(components, 2) else {
        return uncertain();
    };
    if !valid_systemd_slice_components(components) {
        return uncertain();
    }
    let Some(uid) = user_slice_uid(components) else {
        return if service_indexes.is_empty() {
            SystemdCgroupResult {
                state: SystemdCgroupState::Conclusive,
                supervisor: None,
            }
        } else {
            uncertain()
        };
    };
    let infrastructure = format!("user@{uid}.service");
    let has_infrastructure = components.get(2) == Some(&infrastructure.as_str());
    let actual_services: Vec<usize> = service_indexes
        .iter()
        .copied()
        .filter(|index| !(*index == 2 && components[*index] == infrastructure))
        .collect();
    if actual_services.is_empty() {
        return if service_indexes.is_empty() || has_infrastructure {
            SystemdCgroupResult {
                state: SystemdCgroupState::Conclusive,
                supervisor: None,
            }
        } else {
            uncertain()
        };
    }
    if !has_infrastructure || actual_services.len() != 1 {
        return uncertain();
    }
    let service_index = actual_services[0];
    if !only_slice_ancestors(&components[3..service_index]) {
        return uncertain();
    }
    supervised(
        SupervisorKind::SystemdUser,
        SupervisorScope::User,
        components[service_index],
    )
}

fn systemd_service_indexes(components: &[&str], start: usize) -> Option<Vec<usize>> {
    let mut indexes = Vec::new();
    for (index, component) in components.iter().enumerate().skip(start) {
        if !component.ends_with(".service") {
            continue;
        }
        if !valid_systemd_unit(component, ".service") {
            return None;
        }
        indexes.push(index);
    }
    Some(indexes)
}

fn supervised(kind: SupervisorKind, scope: SupervisorScope, unit: &str) -> SystemdCgroupResult {
    SystemdCgroupResult {
        state: SystemdCgroupState::Supervised,
        supervisor: Some(Supervisor {
            kind,
            scope,
            identifiers: vec![unit.to_owned()],
        }),
    }
}

fn only_slice_ancestors(components: &[&str]) -> bool {
    components
        .iter()
        .all(|component| valid_systemd_unit(component, ".slice"))
}

fn valid_systemd_slice_components(components: &[&str]) -> bool {
    components
        .iter()
        .all(|component| !component.ends_with(".slice") || valid_systemd_unit(component, ".slice"))
}

fn user_slice_uid(components: &[&str]) -> Option<String> {
    let component = components.get(1)?;
    let uid = component.strip_prefix("user-")?.strip_suffix(".slice")?;
    if uid.is_empty() || uid.parse::<u32>().is_err() {
        return None;
    }
    Some(uid.to_owned())
}

fn valid_cgroup_path(path: &str) -> bool {
    if path == "/" {
        return true;
    }
    if !path.starts_with('/') {
        return false;
    }
    path[1..].split('/').all(|component| {
        !component.is_empty()
            && component != "."
            && component != ".."
            && std::str::from_utf8(component.as_bytes()).is_ok()
            && !has_control(component)
    })
}

fn parse_cgroup_controllers(value: &str) -> Option<BTreeSet<String>> {
    let mut controllers = BTreeSet::new();
    if value.is_empty() {
        return Some(controllers);
    }
    for controller in value.split(',') {
        if controller.is_empty()
            || !valid_cgroup_controller(controller)
            || !controllers.insert(controller.to_owned())
        {
            return None;
        }
    }
    Some(controllers)
}

fn valid_cgroup_controller(value: &str) -> bool {
    value.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '_' | '.' | '-' | '=')
    })
}

fn has_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}

fn uncertain() -> SystemdCgroupResult {
    SystemdCgroupResult {
        state: SystemdCgroupState::Uncertain,
        supervisor: None,
    }
}

pub fn read_proc_listeners(
    root: &Path,
    ip: IpAddr,
    port: u16,
) -> Result<Vec<ProcListener>, String> {
    let mut matches = Vec::new();
    for name in ["tcp", "tcp6"] {
        let path = root.join("net").join(name);
        let data = match std::fs::read_to_string(&path) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        for (index, line) in data.lines().enumerate() {
            if index == 0 {
                continue;
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 {
                return Err("malformed proc TCP row".into());
            }
            if fields[3] != "0A" {
                continue;
            }
            let Some((address, port_text)) = fields[1].split_once(':') else {
                return Err("malformed proc TCP address".into());
            };
            let decoded_port = u16::from_str_radix(port_text, 16)
                .map_err(|_| "invalid proc TCP port".to_owned())?;
            let decoded_ip =
                decode_proc_ip(address).map_err(|_| "invalid proc TCP address".to_owned())?;
            if decoded_port == port && decoded_ip.is_unspecified() {
                return Err("wildcard listener is ambiguous".into());
            }
            if decoded_port == port && decoded_ip == ip {
                matches.push(ProcListener {
                    inode: fields[9].to_owned(),
                    ip: decoded_ip,
                    port,
                });
            }
        }
    }
    Ok(matches)
}

fn decode_proc_ip(value: &str) -> Result<IpAddr, ()> {
    let mut data = hex_decode(value).ok_or(())?;
    if data.len() != 4 && data.len() != 16 {
        return Err(());
    }
    for chunk in data.chunks_exact_mut(4) {
        chunk.swap(0, 3);
        chunk.swap(1, 2);
    }
    match data.len() {
        4 => Ok(IpAddr::V4(Ipv4Addr::new(
            data[0], data[1], data[2], data[3],
        ))),
        16 => {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&data);
            Ok(IpAddr::V6(std::net::Ipv6Addr::from(octets)))
        }
        _ => Err(()),
    }
}

fn hex_decode(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

fn proc_effective_uid(root: &Path, pid: i32) -> Result<String, OccupantError> {
    let data =
        std::fs::read_to_string(root.join(pid.to_string()).join("status")).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                OccupantError::NotFound
            } else {
                OccupantError::IdentityUnavailable
            }
        })?;
    for line in data.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            let fields: Vec<&str> = rest.split_whitespace().collect();
            if fields.len() >= 2 && fields[1].parse::<u32>().is_ok() {
                return Ok(fields[1].to_owned());
            }
        }
    }
    Err(OccupantError::IdentityUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supervised_system(unit: &str) -> SystemdCgroupResult {
        supervised(SupervisorKind::SystemdSystem, SupervisorScope::System, unit)
    }

    fn supervised_user(unit: &str) -> SystemdCgroupResult {
        supervised(SupervisorKind::SystemdUser, SupervisorScope::User, unit)
    }

    #[test]
    fn parse_systemd_cgroup_table() {
        let cases: &[(&str, &str, SystemdCgroupState, Option<Supervisor>)] = &[
            (
                "cgroup v2 user service",
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/demo.service\n",
                SystemdCgroupState::Supervised,
                supervised_user("demo.service").supervisor,
            ),
            (
                "cgroup v2 system service",
                "0::/system.slice/mtls-router.service\n",
                SystemdCgroupState::Supervised,
                supervised_system("mtls-router.service").supervisor,
            ),
            (
                "cgroup v1 system service",
                "1:name=systemd:/system.slice/legacy.service\n",
                SystemdCgroupState::Supervised,
                supervised_system("legacy.service").supervisor,
            ),
            (
                "scope is conclusive",
                "0::/system.slice/docker-abc.scope\n",
                SystemdCgroupState::Conclusive,
                None,
            ),
            (
                "user manager itself is conclusive",
                "0::/user.slice/user-1000.slice/user@1000.service\n",
                SystemdCgroupState::Conclusive,
                None,
            ),
            (
                "root is conclusive",
                "0::/\n",
                SystemdCgroupState::Conclusive,
                None,
            ),
            (
                "traversal",
                "0::/system.slice/../evil.service\n",
                SystemdCgroupState::Uncertain,
                None,
            ),
            (
                "multiple system services",
                "0::/system.slice/outer.service/inner.service/child\n",
                SystemdCgroupState::Uncertain,
                None,
            ),
            (
                "space in unit",
                "0::/system.slice/demo worker.service\n",
                SystemdCgroupState::Uncertain,
                None,
            ),
            (
                "ambiguous units",
                "0::/system.slice/alpha.service\n1:name=systemd:/system.slice/beta.service\n",
                SystemdCgroupState::Uncertain,
                None,
            ),
        ];
        for (name, input, state, supervisor) in cases {
            let got = parse_systemd_cgroup(input.as_bytes());
            assert_eq!(got.state, *state, "{name}");
            assert_eq!(got.supervisor, *supervisor, "{name}");
        }
    }

    const PROC_TCP_HEADER: &str =
        "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n";

    fn proc_tcp_row(local_address: &str, inode: &str) -> String {
        format!(
            "   0: {local_address} 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 {inode} 1 0000000000000000 100 0 0 10 0\n"
        )
    }

    fn write_proc_net(files: &[(&str, String)]) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("mtls-occupant-proc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("net")).unwrap();
        for (name, content) in files {
            std::fs::write(root.join("net").join(name), content).unwrap();
        }
        root
    }

    #[test]
    fn read_proc_listeners_exact_loopback() {
        let root = write_proc_net(&[(
            "tcp",
            format!(
                "{PROC_TCP_HEADER}{}{}",
                proc_tcp_row("0100007F:4A9B", "101"),
                proc_tcp_row("0200007F:4A9B", "102")
            ),
        )]);
        let listeners =
            read_proc_listeners(&root, IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 19099).unwrap();
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].inode, "101");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn read_proc_listeners_rejects_wildcard() {
        let root = write_proc_net(&[(
            "tcp",
            format!(
                "{}{}",
                PROC_TCP_HEADER,
                proc_tcp_row("00000000:4A9B", "401")
            ),
        )]);
        let error =
            read_proc_listeners(&root, IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 19099).unwrap_err();
        assert_eq!(error, "wildcard listener is ambiguous");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn inspect_linux_correlates_socket_owner() {
        let root = write_proc_net(&[(
            "tcp",
            format!(
                "{}{}",
                PROC_TCP_HEADER,
                proc_tcp_row("0100007F:4A9B", "601")
            ),
        )]);
        let pid_dir = root.join("4242");
        std::fs::create_dir_all(pid_dir.join("fd")).unwrap();
        std::fs::write(
            pid_dir.join("status"),
            b"Name:\ttest\nUid:\t1000\t1001\t1002\t1003\n",
        )
        .unwrap();
        std::os::unix::fs::symlink("socket:[601]", pid_dir.join("fd").join("7")).unwrap();
        std::fs::write(
            pid_dir.join("cgroup"),
            b"0::/codeasier.slice/router.slice/demo.service/delegated/child\n",
        )
        .unwrap();
        let target = inspect_linux(&CallContext::unbounded(), "127.0.0.1:19099", &root, |_| {
            Ok(ProcessIdentity {
                pid: 4242,
                started_at: "123".into(),
                executable: "/tmp/listener".into(),
            })
        })
        .unwrap();
        assert_eq!(target.identity.socket_id, "601");
        assert_eq!(target.identity.user_id, "1001");
        assert_eq!(
            target
                .supervisor
                .as_ref()
                .map(|value| value.identifiers.clone()),
            Some(vec!["demo.service".to_owned()])
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
