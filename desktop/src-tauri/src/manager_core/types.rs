use std::path::PathBuf;

use crate::protocol::RouterOwner;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Classification {
    DesktopOwned,
    ExternalCompatible,
    Degraded,
    Stale,
    LegacyManaged,
    Absent,
    UnknownOccupant,
}

impl Classification {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DesktopOwned => "desktop_owned",
            Self::ExternalCompatible => "external_compatible",
            Self::Degraded => "degraded",
            Self::Stale => "stale",
            Self::LegacyManaged => "legacy_managed",
            Self::Absent => "absent",
            Self::UnknownOccupant => "unknown_occupant",
        }
    }

    pub fn is_trusted(self) -> bool {
        matches!(
            self,
            Self::DesktopOwned | Self::ExternalCompatible | Self::Degraded
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VersionInfo {
    pub version: String,
    pub deployment_id: String,
    pub management_protocol_version: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HealthInfo {
    pub status: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoverySnapshot {
    pub classification: Classification,
    pub owner: Option<String>,
    pub listen_addr: Option<String>,
    pub pid: Option<u32>,
    pub version: Option<VersionInfo>,
    pub state_version: Option<VersionInfo>,
    pub health: Option<HealthInfo>,
    pub log_path: Option<PathBuf>,
}

impl DiscoverySnapshot {
    pub fn absent() -> Self {
        Self {
            classification: Classification::Absent,
            owner: None,
            listen_addr: None,
            pid: None,
            version: None,
            state_version: None,
            health: None,
            log_path: None,
        }
    }

    pub fn resolved_version(&self) -> Option<&VersionInfo> {
        match self.version.as_ref() {
            Some(version) if !version.version.is_empty() => Some(version),
            _ => self.state_version.as_ref(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartedRouter {
    pub owner: RouterOwner,
    pub listen_addr: String,
    pub pid: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentLine {
    pub agent: String,
    pub detected: bool,
    pub exists: bool,
    pub writable: bool,
    pub configured: bool,
    pub invalid: bool,
}
