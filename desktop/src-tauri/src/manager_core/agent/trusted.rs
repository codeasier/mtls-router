use crate::manager_core::apikeyusage::Snapshot;
use crate::manager_core::errors::closed_error;
use crate::protocol::{ErrorCode, ProtocolError};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TrustedBinding {
    pub router_base_url: String,
    pub api_base_url: String,
    pub deployment_id: String,
    pub protocol_version: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TrustedModels {
    pub models: Vec<String>,
    pub binding: TrustedBinding,
}

pub trait TrustedCatalog: Send + Sync {
    fn fetch(&self, owner: &str, api_key: &str) -> Result<TrustedModels, ProtocolError>;
    fn fetch_usage(
        &self,
        _owner: &str,
        _period: &str,
        _api_key: &str,
    ) -> Result<Snapshot, ProtocolError> {
        Err(closed_error(
            ErrorCode::UsageUnavailable,
            "usage is unavailable",
        ))
    }
    fn revalidate(
        &self,
        owner: &str,
        api_key: &str,
        binding: &TrustedBinding,
    ) -> Result<Vec<String>, ProtocolError>;
}

#[derive(Clone, Debug, Default)]
pub struct StaticCatalog {
    pub models: TrustedModels,
    pub usage: Option<Snapshot>,
}

impl TrustedCatalog for StaticCatalog {
    fn fetch(&self, _owner: &str, _api_key: &str) -> Result<TrustedModels, ProtocolError> {
        Ok(self.models.clone())
    }

    fn fetch_usage(
        &self,
        _owner: &str,
        _period: &str,
        _api_key: &str,
    ) -> Result<Snapshot, ProtocolError> {
        self.usage
            .clone()
            .ok_or_else(|| closed_error(ErrorCode::UsageUnavailable, "usage is unavailable"))
    }

    fn revalidate(
        &self,
        _owner: &str,
        _api_key: &str,
        _binding: &TrustedBinding,
    ) -> Result<Vec<String>, ProtocolError> {
        Ok(self.models.models.clone())
    }
}
