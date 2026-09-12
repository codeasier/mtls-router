use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, SecondsFormat, Utc};

use super::super::process::{self, Identity as ProcessIdentity, ProcessError, Status};
use super::super::types::Classification;
use super::inspect::{inspect_native, supports_pid_only_native};
use super::types::{
    recovery_for_reason, released_result, valid_supervisor, Identity, Inspection, OccupantError,
    Recovery, RecoveryAction, RecoveryReason, Target, TerminateResult, VerificationMode,
};

pub const TOKEN_LIFETIME: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct OccupantConfig {
    pub listen_addr: String,
    pub desktop_pid: i32,
    pub manager_identity: ProcessIdentity,
    pub is_protected: Option<fn(&Identity) -> bool>,
    pub is_protected_pid: Option<fn(i32) -> bool>,
    pub state_paths: Vec<PathBuf>,
    pub release_timeout: Duration,
    pub poll_interval: Duration,
}

impl Default for OccupantConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:19099".into(),
            desktop_pid: 0,
            manager_identity: ProcessIdentity::default(),
            is_protected: None,
            is_protected_pid: None,
            state_paths: Vec::new(),
            release_timeout: Duration::from_secs(2),
            poll_interval: Duration::from_millis(50),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CallContext {
    pub deadline: Option<Instant>,
    pub cancelled: Arc<AtomicBool>,
}

impl CallContext {
    pub fn unbounded() -> Self {
        Self {
            deadline: None,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_deadline(deadline: Instant) -> Self {
        Self {
            deadline: Some(deadline),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn err(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
            || self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
    }
}

type DiscoverFn = Box<dyn Fn(&CallContext) -> Classification + Send + Sync>;
type InspectFn = Box<dyn Fn(&CallContext, &str) -> Result<Target, OccupantError> + Send + Sync>;
type InspectPidFn = Box<dyn Fn(&CallContext, &str) -> Result<i32, OccupantError> + Send + Sync>;
type SignalPidFn = Box<dyn Fn(i32) -> Result<(), OccupantError> + Send + Sync>;
type CurrentUserFn = Box<dyn Fn() -> Result<String, OccupantError> + Send + Sync>;
type SameProcessFn =
    Box<dyn Fn(&ProcessIdentity, &ProcessIdentity) -> Result<bool, OccupantError> + Send + Sync>;
type ValidateFn =
    Box<dyn Fn(&ProcessIdentity, &str) -> Result<Status, OccupantError> + Send + Sync>;
type SignalFn = Box<dyn Fn(&ProcessIdentity) -> Result<(), OccupantError> + Send + Sync>;
type DialFn = Box<dyn Fn(&CallContext, &str, &str) -> Result<(), OccupantError> + Send + Sync>;
type RandomFn = Box<dyn Fn(&mut [u8]) -> Result<(), OccupantError> + Send + Sync>;
type NowFn = Box<dyn Fn() -> DateTime<Utc> + Send + Sync>;
type SleepFn = Box<dyn Fn(&CallContext, Duration) -> Result<(), OccupantError> + Send + Sync>;

pub struct OccupantDependencies {
    pub discover: Option<DiscoverFn>,
    pub inspect: Option<InspectFn>,
    pub supports_pid_only: Option<Box<dyn Fn() -> bool + Send + Sync>>,
    pub inspect_pid_owner: Option<InspectPidFn>,
    pub signal_pid: Option<SignalPidFn>,
    pub current_user: Option<CurrentUserFn>,
    pub same_process: Option<SameProcessFn>,
    pub validate: Option<ValidateFn>,
    pub signal: Option<SignalFn>,
    pub dial: Option<DialFn>,
    pub random: Option<RandomFn>,
    pub now: Option<NowFn>,
    pub sleep: Option<SleepFn>,
}

impl Default for OccupantDependencies {
    fn default() -> Self {
        Self {
            discover: None,
            inspect: None,
            supports_pid_only: None,
            inspect_pid_owner: None,
            signal_pid: None,
            current_user: None,
            same_process: None,
            validate: None,
            signal: None,
            dial: None,
            random: None,
            now: None,
            sleep: None,
        }
    }
}

struct TokenRecord {
    value: String,
    expires_at: DateTime<Utc>,
    target: Target,
}

pub struct OccupantService {
    config: OccupantConfig,
    deps: ResolvedDependencies,
    token: Mutex<Option<TokenRecord>>,
}

struct ResolvedDependencies {
    discover: Option<DiscoverFn>,
    inspect: InspectFn,
    supports_pid_only: Box<dyn Fn() -> bool + Send + Sync>,
    inspect_pid_owner: InspectPidFn,
    has_inspect_pid_owner: bool,
    signal_pid: SignalPidFn,
    has_signal_pid: bool,
    current_user: CurrentUserFn,
    same_process: SameProcessFn,
    validate: ValidateFn,
    signal: SignalFn,
    dial: DialFn,
    random: RandomFn,
    now: NowFn,
    sleep: SleepFn,
}

impl OccupantService {
    pub fn new(mut config: OccupantConfig, deps: OccupantDependencies) -> Self {
        if config.release_timeout.is_zero() {
            config.release_timeout = Duration::from_secs(2);
        }
        if config.poll_interval.is_zero() {
            config.poll_interval = Duration::from_millis(50);
        }
        Self {
            config,
            deps: resolve_dependencies(deps),
            token: Mutex::new(None),
        }
    }

    pub fn inspect(&self, ctx: &CallContext) -> Result<Inspection, OccupantError> {
        let classification = self.discover(ctx)?;
        self.inspect_for(ctx, classification)
    }

    pub fn inspect_for(
        &self,
        ctx: &CallContext,
        classification: Classification,
    ) -> Result<Inspection, OccupantError> {
        let mut token = self.token.lock().expect("occupant token");
        *token = None;
        self.require_unknown_value(ctx, classification)?;
        let mut target = (self.deps.inspect)(ctx, &self.config.listen_addr)?;
        let (mut inspection, forceable) = self.classify_target(&mut target)?;
        if !forceable {
            return Ok(inspection);
        }
        let record = self.mint_token(&mut token, target)?;
        inspection.recovery = Recovery {
            action: RecoveryAction::ForceTerminate,
            reason: None,
        };
        inspection.confirmation_token = Some(record.value.clone());
        inspection.expires_at = Some(format_expiry(record.expires_at));
        Ok(inspection)
    }

    pub fn force_terminate(
        &self,
        ctx: &CallContext,
        token: &str,
    ) -> Result<TerminateResult, OccupantError> {
        self.force_terminate_inner(ctx, token, None)
    }

    pub fn force_terminate_for(
        &self,
        ctx: &CallContext,
        classification: Classification,
        token: &str,
    ) -> Result<TerminateResult, OccupantError> {
        self.force_terminate_inner(ctx, token, Some(classification))
    }

    fn force_terminate_inner(
        &self,
        ctx: &CallContext,
        token: &str,
        classification: Option<Classification>,
    ) -> Result<TerminateResult, OccupantError> {
        let mut guard = self.token.lock().expect("occupant token");
        let record = guard.take();
        let Some(record) = record else {
            return Err(OccupantError::ConfirmationExpired);
        };
        if token.is_empty() || token != record.value || (self.deps.now)() >= record.expires_at {
            return Err(OccupantError::ConfirmationExpired);
        }
        let reserved = reserve_release_window(ctx, self.config.release_timeout);
        let found = match classification {
            Some(value) => value,
            None => self.discover(&reserved)?,
        };
        if let Err(error) = self.require_unknown_value(&reserved, found) {
            if record.target.mode == Some(VerificationMode::WindowsPidOnly)
                && error == OccupantError::NotFound
            {
                return Err(OccupantError::Changed);
            }
            return Err(error);
        }
        match record.target.mode {
            Some(VerificationMode::VerifiedIdentity) => {
                self.force_terminate_verified(&reserved, ctx, record.target)
            }
            Some(VerificationMode::WindowsPidOnly) => {
                self.force_terminate_pid_only(&reserved, ctx, record.target)
            }
            None => Err(OccupantError::Changed),
        }
    }

    fn mint_token<'a>(
        &self,
        slot: &'a mut Option<TokenRecord>,
        target: Target,
    ) -> Result<&'a TokenRecord, OccupantError> {
        let mut random = [0u8; 32];
        (self.deps.random)(&mut random)?;
        let now = (self.deps.now)();
        *slot = Some(TokenRecord {
            value: URL_SAFE_NO_PAD.encode(random),
            expires_at: now + chrono::Duration::seconds(30),
            target,
        });
        Ok(slot.as_ref().expect("minted token"))
    }

    fn classify_target(&self, target: &mut Target) -> Result<(Inspection, bool), OccupantError> {
        let invalid_supervisor = target
            .supervisor
            .as_ref()
            .is_some_and(|supervisor| !valid_supervisor(supervisor));
        if invalid_supervisor {
            target.supervisor = None;
        }
        let mut inspection = Inspection {
            pid: target.pid,
            verification_mode: target.mode,
            listen_addr: target.listen_addr.clone(),
            supervisor: target.supervisor.clone(),
            recovery: Recovery::default(),
            ..Inspection::default()
        };
        let identity = target.identity.clone();
        let valid_verified_identity = target.mode == Some(VerificationMode::VerifiedIdentity)
            && identity.listen_addr == self.config.listen_addr
            && identity.network == "tcp4"
            && !identity.socket_id.is_empty()
            && identity.process.pid > 0
            && !identity.process.started_at.is_empty()
            && !identity.process.executable.is_empty()
            && !identity.user_id.is_empty()
            && target.pid == identity.process.pid
            && target.listen_addr == identity.listen_addr;
        if valid_verified_identity {
            inspection.process_name = Path::new(&identity.process.executable)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            inspection.executable = Some(identity.process.executable.clone());
        }
        if self.is_protected_target(target) {
            inspection.supervisor = None;
            inspection.recovery = Recovery {
                action: RecoveryAction::Unavailable,
                reason: Some(RecoveryReason::ProtectedProcess),
            };
            return Ok((inspection, false));
        }
        if target.mode == Some(VerificationMode::VerifiedIdentity) {
            if !valid_verified_identity {
                inspection.supervisor = None;
                inspection.recovery = Recovery {
                    action: RecoveryAction::Unavailable,
                    reason: Some(RecoveryReason::IdentityUnavailable),
                };
                return Ok((inspection, false));
            }
            match (self.deps.current_user)() {
                Ok(user_id) if !user_id.is_empty() && identity.user_id == user_id => {}
                Ok(user_id) if !user_id.is_empty() => {
                    inspection.process_name = None;
                    inspection.executable = None;
                    inspection.supervisor = None;
                    inspection.recovery = Recovery {
                        action: RecoveryAction::ManualStopRequired,
                        reason: Some(RecoveryReason::DifferentUser),
                    };
                    return Ok((inspection, false));
                }
                _ => {
                    inspection.supervisor = None;
                    inspection.recovery = Recovery {
                        action: RecoveryAction::Unavailable,
                        reason: Some(RecoveryReason::IdentityUnavailable),
                    };
                    return Ok((inspection, false));
                }
            }
        }
        if invalid_supervisor {
            inspection.recovery = Recovery {
                action: RecoveryAction::Unavailable,
                reason: Some(RecoveryReason::IdentityUnavailable),
            };
            return Ok((inspection, false));
        }
        if target.supervisor.is_some() {
            inspection.recovery = Recovery {
                action: RecoveryAction::ManualStopRequired,
                reason: Some(RecoveryReason::ServiceManaged),
            };
            return Ok((inspection, false));
        }
        if let Some(reason) = target.block_reason {
            let recovery = recovery_for_reason(reason).ok_or(OccupantError::IdentityUnavailable)?;
            if recovery.reason == Some(RecoveryReason::DifferentUser) {
                inspection.process_name = None;
                inspection.executable = None;
            }
            inspection.recovery = recovery;
            return Ok((inspection, false));
        }
        match target.mode {
            Some(VerificationMode::VerifiedIdentity) => Ok((inspection, true)),
            Some(VerificationMode::WindowsPidOnly) => {
                if !(self.deps.supports_pid_only)()
                    || !self.deps.has_inspect_pid_owner
                    || !self.deps.has_signal_pid
                    || target.listen_addr != self.config.listen_addr
                    || target.pid <= 0
                {
                    inspection.recovery = Recovery {
                        action: RecoveryAction::Unavailable,
                        reason: Some(RecoveryReason::IdentityUnavailable),
                    };
                    return Ok((inspection, false));
                }
                Ok((inspection, true))
            }
            None => {
                inspection.recovery = Recovery {
                    action: RecoveryAction::Unavailable,
                    reason: Some(RecoveryReason::IdentityUnavailable),
                };
                Ok((inspection, false))
            }
        }
    }

    fn force_terminate_verified(
        &self,
        pre_signal: &CallContext,
        ctx: &CallContext,
        target: Target,
    ) -> Result<TerminateResult, OccupantError> {
        let live = match self.inspect_verified_eligible(pre_signal) {
            Ok(live) => live,
            Err(OccupantError::NotFound) => return Err(OccupantError::Changed),
            Err(error) => return Err(error),
        };
        let same = same_identity(&target.identity, &live.identity, |left, right| {
            (self.deps.same_process)(left, right)
        })?;
        if !same {
            return Err(OccupantError::Changed);
        }
        if live.block_reason == Some(RecoveryReason::InsufficientPrivilege) {
            return Err(OccupantError::PermissionDenied);
        }
        match (self.deps.validate)(&live.identity.process, &live.identity.process.executable) {
            Ok(Status::Genuine) => {}
            _ => return Err(OccupantError::Changed),
        }
        match (self.deps.signal)(&live.identity.process) {
            Ok(()) => {}
            Err(OccupantError::Changed) | Err(OccupantError::NotFound) => {
                return Err(OccupantError::Changed)
            }
            Err(OccupantError::PermissionDenied) => return Err(OccupantError::PermissionDenied),
            Err(_) => return Err(OccupantError::TerminationFailed),
        }
        self.wait_released(ctx, &live.identity)
    }

    fn force_terminate_pid_only(
        &self,
        pre_signal: &CallContext,
        ctx: &CallContext,
        target: Target,
    ) -> Result<TerminateResult, OccupantError> {
        let live_pid = match (self.deps.inspect_pid_owner)(pre_signal, &target.listen_addr) {
            Ok(pid) if pid == target.pid => pid,
            _ => return Err(OccupantError::Changed),
        };
        if self.is_protected_pid(live_pid) {
            return Err(OccupantError::Protected);
        }
        if pre_signal.err() {
            return Err(OccupantError::Changed);
        }
        match (self.deps.signal_pid)(live_pid) {
            Ok(()) => {}
            Err(OccupantError::NotFound) => return Err(OccupantError::Changed),
            Err(OccupantError::PermissionDenied) => return Err(OccupantError::PermissionDenied),
            Err(_) => return Err(OccupantError::TerminationFailed),
        }
        self.wait_pid_released(ctx, &target)
    }

    fn inspect_verified_eligible(&self, ctx: &CallContext) -> Result<Target, OccupantError> {
        let target = (self.deps.inspect)(ctx, &self.config.listen_addr)?;
        if target.mode != Some(VerificationMode::VerifiedIdentity) {
            return Err(OccupantError::Changed);
        }
        self.validate_verified_target(target)
    }

    fn validate_verified_target(&self, target: Target) -> Result<Target, OccupantError> {
        if target.supervisor.is_some() {
            return Err(OccupantError::Changed);
        }
        match target.block_reason {
            None | Some(RecoveryReason::InsufficientPrivilege) => {}
            Some(RecoveryReason::ProtectedProcess) => return Err(OccupantError::Protected),
            Some(_) => return Err(OccupantError::Changed),
        }
        let identity = &target.identity;
        if identity.listen_addr != self.config.listen_addr
            || identity.network != "tcp4"
            || identity.socket_id.is_empty()
            || identity.process.pid <= 0
            || identity.process.started_at.is_empty()
            || identity.process.executable.is_empty()
            || identity.user_id.is_empty()
        {
            return Err(OccupantError::IdentityUnavailable);
        }
        if target.pid != identity.process.pid || target.listen_addr != identity.listen_addr {
            return Err(OccupantError::IdentityUnavailable);
        }
        let user_id = (self.deps.current_user)()?;
        if user_id.is_empty() {
            return Err(OccupantError::IdentityUnavailable);
        }
        if identity.user_id != user_id {
            return Err(OccupantError::NotOwned);
        }
        if identity.process.pid == self.config.desktop_pid
            || identity.process.pid == self.config.manager_identity.pid
            || self.state_protects_identity(identity)
            || self
                .config
                .is_protected
                .is_some_and(|is_protected| is_protected(identity))
        {
            return Err(OccupantError::Protected);
        }
        Ok(target)
    }

    fn is_protected_pid(&self, pid: i32) -> bool {
        pid > 0
            && (pid == self.config.desktop_pid
                || pid == self.config.manager_identity.pid
                || super::super::state::protects_pid(&self.config.state_paths, pid)
                || self
                    .config
                    .is_protected_pid
                    .is_some_and(|is_protected| is_protected(pid)))
    }

    fn is_protected_target(&self, target: &Target) -> bool {
        if target.block_reason == Some(RecoveryReason::ProtectedProcess)
            || self.is_protected_pid(target.pid)
        {
            return true;
        }
        if target.mode != Some(VerificationMode::VerifiedIdentity) {
            return false;
        }
        self.is_protected_pid(target.identity.process.pid)
            || self.state_protects_identity(&target.identity)
            || self
                .config
                .is_protected
                .is_some_and(|is_protected| is_protected(&target.identity))
    }

    fn state_protects_identity(&self, identity: &Identity) -> bool {
        self.config.state_paths.iter().any(|path| {
            let Ok(value) = super::super::state::read(path) else {
                return false;
            };
            let managed = ProcessIdentity {
                pid: value.pid,
                started_at: value.process_started_at,
                executable: value.process_executable,
            };
            process::same_identity(&identity.process, &managed).unwrap_or(false)
        })
    }

    fn discover(&self, ctx: &CallContext) -> Result<Classification, OccupantError> {
        let Some(discover) = &self.deps.discover else {
            return Err(OccupantError::IdentityUnavailable);
        };
        let found = discover(ctx);
        if ctx.err() {
            return Err(OccupantError::IdentityUnavailable);
        }
        Ok(found)
    }

    fn require_unknown(&self, ctx: &CallContext) -> Result<(), OccupantError> {
        let found = self.discover(ctx)?;
        self.require_unknown_value(ctx, found)
    }

    fn require_unknown_value(
        &self,
        ctx: &CallContext,
        found: Classification,
    ) -> Result<(), OccupantError> {
        if ctx.err() {
            return Err(OccupantError::IdentityUnavailable);
        }
        if found != Classification::UnknownOccupant {
            return if found == Classification::Absent {
                Err(OccupantError::NotFound)
            } else {
                Err(OccupantError::Protected)
            };
        }
        Ok(())
    }

    fn wait_released(
        &self,
        parent: &CallContext,
        identity: &Identity,
    ) -> Result<TerminateResult, OccupantError> {
        let deadline = Instant::now() + self.config.release_timeout;
        let ctx = CallContext {
            deadline: Some(merge_deadline(parent.deadline, deadline)),
            cancelled: parent.cancelled.clone(),
        };
        loop {
            let status = (self.deps.validate)(&identity.process, &identity.process.executable)
                .unwrap_or(Status::Stale);
            if status == Status::Stale {
                return Err(OccupantError::Changed);
            }
            if status == Status::Absent {
                if (self.deps.dial)(&ctx, "tcp4", &identity.listen_addr).is_err() {
                    return Ok(released_result());
                }
                if let Ok(replacement) = (self.deps.inspect)(&ctx, &identity.listen_addr) {
                    let same = same_identity(&identity, &replacement.identity, |left, right| {
                        (self.deps.same_process)(left, right)
                    })
                    .unwrap_or(false);
                    if !same {
                        return Err(OccupantError::Changed);
                    }
                }
            }
            if (self.deps.sleep)(&ctx, self.config.poll_interval).is_err() {
                return match (self.deps.validate)(&identity.process, &identity.process.executable)
                    .unwrap_or(Status::Stale)
                {
                    Status::Stale => Err(OccupantError::Changed),
                    Status::Genuine => Err(OccupantError::TerminationFailed),
                    Status::Absent => Err(OccupantError::PortReleaseTimeout),
                };
            }
        }
    }

    fn wait_pid_released(
        &self,
        parent: &CallContext,
        target: &Target,
    ) -> Result<TerminateResult, OccupantError> {
        let deadline = Instant::now() + self.config.release_timeout;
        let ctx = CallContext {
            deadline: Some(merge_deadline(parent.deadline, deadline)),
            cancelled: parent.cancelled.clone(),
        };
        loop {
            match (self.deps.inspect_pid_owner)(&ctx, &target.listen_addr) {
                Err(OccupantError::NotFound) => return Ok(released_result()),
                Ok(pid) if pid == target.pid => {}
                _ => return Err(OccupantError::Changed),
            }
            if (self.deps.sleep)(&ctx, self.config.poll_interval).is_err() {
                let final_ctx = CallContext {
                    deadline: Some(Instant::now() + self.config.poll_interval),
                    cancelled: Arc::new(AtomicBool::new(false)),
                };
                return match (self.deps.inspect_pid_owner)(&final_ctx, &target.listen_addr) {
                    Err(OccupantError::NotFound) => Ok(released_result()),
                    Ok(pid) if pid == target.pid => Err(OccupantError::PortReleaseTimeout),
                    _ => Err(OccupantError::Changed),
                };
            }
        }
    }
}

fn resolve_dependencies(deps: OccupantDependencies) -> ResolvedDependencies {
    let native_windows = cfg!(windows) && deps.inspect.is_none();
    ResolvedDependencies {
        discover: deps.discover,
        inspect: deps
            .inspect
            .unwrap_or_else(|| Box::new(|ctx, addr| inspect_native(ctx, addr))),
        supports_pid_only: deps
            .supports_pid_only
            .unwrap_or_else(|| Box::new(supports_pid_only_native)),
        has_inspect_pid_owner: deps.inspect_pid_owner.is_some() || native_windows,
        inspect_pid_owner: deps.inspect_pid_owner.unwrap_or_else(|| {
            Box::new(|ctx, addr| super::inspect::inspect_pid_owner_native(ctx, addr))
        }),
        has_signal_pid: deps.signal_pid.is_some() || native_windows,
        signal_pid: deps
            .signal_pid
            .unwrap_or_else(|| Box::new(super::inspect::signal_pid_native)),
        current_user: deps
            .current_user
            .unwrap_or_else(|| Box::new(super::inspect::current_user_native)),
        same_process: deps.same_process.unwrap_or_else(|| {
            Box::new(|left, right| {
                process::same_identity(left, right).map_err(|_| OccupantError::IdentityUnavailable)
            })
        }),
        validate: deps.validate.unwrap_or_else(|| {
            Box::new(|identity, binary| {
                process::validate(identity, binary).map_err(|_| OccupantError::IdentityUnavailable)
            })
        }),
        signal: deps.signal.unwrap_or_else(|| {
            Box::new(|identity| match process::signal_identity(identity) {
                Ok(()) => Ok(()),
                Err(ProcessError::NotFound) | Err(ProcessError::IdentityMismatch) => {
                    Err(OccupantError::Changed)
                }
                Err(ProcessError::PermissionDenied) => Err(OccupantError::PermissionDenied),
                Err(ProcessError::Io) => Err(OccupantError::TerminationFailed),
            })
        }),
        dial: deps
            .dial
            .unwrap_or_else(|| Box::new(|_ctx, _network, addr| default_dial(addr))),
        random: deps.random.unwrap_or_else(|| Box::new(default_random)),
        now: deps.now.unwrap_or_else(|| Box::new(Utc::now)),
        sleep: deps.sleep.unwrap_or_else(|| Box::new(default_sleep)),
    }
}

fn default_random(buffer: &mut [u8]) -> Result<(), OccupantError> {
    getrandom_fill(buffer)
}

fn getrandom_fill(buffer: &mut [u8]) -> Result<(), OccupantError> {
    // uuid is already a locked dependency and uses getrandom internally.
    let mut offset = 0;
    while offset < buffer.len() {
        let bytes = uuid::Uuid::new_v4();
        let chunk = bytes.as_bytes();
        let take = chunk.len().min(buffer.len() - offset);
        buffer[offset..offset + take].copy_from_slice(&chunk[..take]);
        offset += take;
    }
    Ok(())
}

fn default_dial(addr: &str) -> Result<(), OccupantError> {
    match std::net::TcpStream::connect_timeout(
        &addr
            .parse()
            .map_err(|_| OccupantError::IdentityUnavailable)?,
        Duration::from_millis(50),
    ) {
        Ok(_) => Ok(()),
        Err(_) => Err(OccupantError::NotFound),
    }
}

fn default_sleep(ctx: &CallContext, delay: Duration) -> Result<(), OccupantError> {
    let remaining = ctx
        .deadline
        .map(|deadline| deadline.saturating_duration_since(Instant::now()))
        .unwrap_or(delay);
    if ctx.err() || remaining.is_zero() {
        return Err(OccupantError::PortReleaseTimeout);
    }
    std::thread::sleep(delay.min(remaining));
    if ctx.err() {
        return Err(OccupantError::PortReleaseTimeout);
    }
    Ok(())
}

fn reserve_release_window(parent: &CallContext, release_timeout: Duration) -> CallContext {
    CallContext {
        deadline: parent.deadline.map(|deadline| {
            deadline
                .checked_sub(release_timeout)
                .unwrap_or_else(Instant::now)
        }),
        cancelled: parent.cancelled.clone(),
    }
}

fn merge_deadline(parent: Option<Instant>, child: Instant) -> Instant {
    match parent {
        Some(parent) if parent < child => parent,
        _ => child,
    }
}

fn same_identity(
    left: &Identity,
    right: &Identity,
    same_process: impl Fn(&ProcessIdentity, &ProcessIdentity) -> Result<bool, OccupantError>,
) -> Result<bool, OccupantError> {
    if left.listen_addr != right.listen_addr
        || left.network != right.network
        || left.socket_id != right.socket_id
        || left.user_id != right.user_id
    {
        return Ok(false);
    }
    same_process(&left.process, &right.process)
}

fn format_expiry(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn map_signal_error(error: OccupantError) -> OccupantError {
    match error {
        OccupantError::NotFound | OccupantError::Changed => OccupantError::Changed,
        OccupantError::PermissionDenied => OccupantError::PermissionDenied,
        _ => OccupantError::TerminationFailed,
    }
}

pub fn permission_from_io(error: &std::io::Error) -> OccupantError {
    if error.kind() == ErrorKind::PermissionDenied {
        OccupantError::PermissionDenied
    } else {
        OccupantError::TerminationFailed
    }
}

pub fn fill_random_from_reader(
    reader: &mut impl Read,
    buffer: &mut [u8],
) -> Result<(), OccupantError> {
    reader
        .read_exact(buffer)
        .map_err(|_| OccupantError::IdentityUnavailable)
}
