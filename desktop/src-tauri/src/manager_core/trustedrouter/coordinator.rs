use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::manager_core::agent::{TrustedCatalog, TrustedModels};
use crate::manager_core::apikeyusage::{Period, Snapshot};
use crate::manager_core::errors::{closed_error, invalid_params};
use crate::protocol::{ErrorCode, ProtocolError, RouterOwner};

use super::channel::{
    min_deadline, stale_catalog, state_from_start, trusted_state_matches, Channel,
};
use super::listener::Listener;
use super::{Binding, Discovery, StartedIdentity};

pub const USAGE_ESTABLISH_BUDGET: Duration = Duration::from_secs(20);

#[derive(Debug)]
pub struct UsageResult {
    pub snapshot: Snapshot,
    pub binding: Binding,
}

#[derive(Debug)]
pub struct CatalogResult {
    pub models: Vec<String>,
    pub binding: Binding,
}

#[derive(Debug)]
pub struct StartOutcome {
    pub state: StartedIdentity,
    pub error: Option<ProtocolError>,
}

pub trait TrustedLifecycle: Send + Sync {
    fn start(&self, owner: RouterOwner, deadline: Instant) -> StartOutcome;
}

type DiscoverFn = Arc<dyn Fn() -> Discovery + Send + Sync>;
type FlagFn = Arc<dyn Fn() -> bool + Send + Sync>;

pub struct Coordinator {
    pub listener: Listener,
    pub deployment_id: String,
    pub protocol_version: String,
    pub discover: DiscoverFn,
    pub lifecycle: Arc<dyn TrustedLifecycle>,
    pub channel: Channel,
    pub desktop_eligible: FlagFn,
    pub absent_start_ok: FlagFn,
}

impl Coordinator {
    pub fn fetch(
        &self,
        owner: RouterOwner,
        api_key: &str,
        deadline: Instant,
    ) -> Result<CatalogResult, ProtocolError> {
        let found = self.establish(owner, deadline)?;
        let models = self
            .channel
            .fetch(&self.listener, &found, api_key, deadline)?;
        Ok(CatalogResult {
            models,
            binding: self.binding(),
        })
    }

    pub fn fetch_usage(
        &self,
        owner: RouterOwner,
        period: Period,
        api_key: &str,
        deadline: Instant,
    ) -> Result<UsageResult, ProtocolError> {
        let found = self.establish(owner, min_deadline(deadline, USAGE_ESTABLISH_BUDGET))?;
        let snapshot =
            self.channel
                .fetch_usage(&self.listener, &found, period, api_key, deadline)?;
        Ok(UsageResult {
            snapshot,
            binding: self.binding(),
        })
    }

    pub fn revalidate(
        &self,
        owner: RouterOwner,
        api_key: &str,
        binding: &Binding,
        deadline: Instant,
    ) -> Result<Vec<String>, ProtocolError> {
        if binding.router_base_url != self.listener.router_base_url
            || binding.api_base_url != self.listener.api_base_url
            || binding.deployment_id != self.deployment_id
            || binding.protocol_version != self.protocol_version
        {
            return Err(stale_catalog());
        }
        let result = self.fetch(owner, api_key, deadline)?;
        if result.binding != *binding {
            return Err(stale_catalog());
        }
        Ok(result.models)
    }

    fn binding(&self) -> Binding {
        Binding {
            router_base_url: self.listener.router_base_url.clone(),
            api_base_url: self.listener.api_base_url.clone(),
            deployment_id: self.deployment_id.clone(),
            protocol_version: self.protocol_version.clone(),
        }
    }

    fn establish(&self, owner: RouterOwner, deadline: Instant) -> Result<Discovery, ProtocolError> {
        if owner == RouterOwner::Desktop && !(self.desktop_eligible)() {
            return Err(invalid_params(
                "desktop owner requires a verified desktop session",
            ));
        }
        let mut found = (self.discover)();
        if found.classification == crate::manager_core::types::Classification::Absent {
            if !(self.absent_start_ok)() {
                return Err(closed_error(
                    ErrorCode::RouterStateStale,
                    "router restart requires explicit lifecycle recovery",
                ));
            }
            let started = self.lifecycle.start(owner, deadline);
            if let Some(error) = started.error {
                if error.code != ErrorCode::RouterDegraded {
                    return Err(lifecycle_protocol_error(error.code));
                }
            }
            if !state_from_start(
                &started.state,
                &self.listener,
                &self.deployment_id,
                &self.protocol_version,
            ) {
                return Err(stale_catalog());
            }
            found = (self.discover)();
        }
        if !trusted_state_matches(
            &found,
            &self.listener,
            &self.deployment_id,
            &self.protocol_version,
        ) {
            return Err(match found.classification {
                crate::manager_core::types::Classification::Stale => {
                    closed_error(ErrorCode::RouterStateStale, "router state is stale")
                }
                crate::manager_core::types::Classification::LegacyManaged => closed_error(
                    ErrorCode::RouterLegacyManaged,
                    "legacy desktop router requires explicit migration",
                ),
                crate::manager_core::types::Classification::UnknownOccupant => {
                    closed_error(ErrorCode::PortOccupied, "router port is occupied")
                }
                crate::manager_core::types::Classification::Absent => {
                    closed_error(ErrorCode::RouterNotFound, "router was not found")
                }
                _ => stale_catalog(),
            });
        }
        Ok(found)
    }
}

impl TrustedCatalog for Coordinator {
    fn fetch(&self, owner: &str, api_key: &str) -> Result<TrustedModels, ProtocolError> {
        let owner = parse_owner(owner)?;
        let result = Coordinator::fetch(
            self,
            owner,
            api_key,
            Instant::now() + Duration::from_secs(30),
        )?;
        Ok(TrustedModels {
            models: result.models,
            binding: result.binding.into(),
        })
    }

    fn fetch_usage(
        &self,
        owner: &str,
        period: &str,
        api_key: &str,
    ) -> Result<Snapshot, ProtocolError> {
        let owner = parse_owner(owner)?;
        let period = crate::manager_core::apikeyusage::normalize_period(period)
            .map_err(|error| invalid_params(error.message()))?;
        let result = Coordinator::fetch_usage(
            self,
            owner,
            period,
            api_key,
            Instant::now() + Duration::from_secs(60),
        )?;
        Ok(result.snapshot)
    }

    fn revalidate(
        &self,
        owner: &str,
        api_key: &str,
        binding: &crate::manager_core::agent::TrustedBinding,
    ) -> Result<Vec<String>, ProtocolError> {
        let owner = parse_owner(owner)?;
        Coordinator::revalidate(
            self,
            owner,
            api_key,
            &Binding::from(binding),
            Instant::now() + Duration::from_secs(30),
        )
    }
}

fn parse_owner(owner: &str) -> Result<RouterOwner, ProtocolError> {
    match owner {
        "desktop" => Ok(RouterOwner::Desktop),
        "cli" => Ok(RouterOwner::Cli),
        _ => Err(invalid_params("owner must be cli or desktop")),
    }
}

fn lifecycle_protocol_error(code: ErrorCode) -> ProtocolError {
    closed_error(code, "router could not be safely started")
}
