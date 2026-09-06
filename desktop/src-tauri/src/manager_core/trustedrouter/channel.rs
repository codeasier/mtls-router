use std::io;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::manager_core::apikeyusage::{self, Period, Snapshot};
use crate::manager_core::errors::closed_error;
use crate::manager_core::httpget::{self, HttpTransport};
use crate::manager_core::modelcatalog;
use crate::manager_core::process::{self, Identity, ProcessError, Status};
use crate::protocol::{ErrorCode, ProtocolError};

use super::listener::Listener;
use super::{Discovery, RemoteVersion};

pub const VERSION_BUDGET: Duration = Duration::from_secs(15);
const VERSION_BODY_LIMIT: usize = 64 << 10;
const DIAL_TIMEOUT: Duration = Duration::from_secs(5);

type DialFn = Arc<dyn Fn(&str) -> io::Result<TcpStream> + Send + Sync>;
type ValidateFn = Arc<dyn Fn(&Identity, &str) -> Result<Status, ProcessError> + Send + Sync>;

#[derive(Clone)]
pub struct Channel {
    pub simplify: bool,
    pub dial: Option<DialFn>,
    pub validate_process: Option<ValidateFn>,
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            simplify: false,
            dial: None,
            validate_process: None,
        }
    }
}

impl Channel {
    pub fn fetch(
        &self,
        listener: &Listener,
        trusted: &Discovery,
        api_key: &str,
        deadline: Instant,
    ) -> Result<Vec<String>, ProtocolError> {
        let mut transport = self.bind(listener, trusted, deadline)?;
        let url = format!("{}/models", listener.api_base_url);
        modelcatalog::Client::new(self.simplify)
            .fetch_with(
                &mut transport,
                deadline,
                modelcatalog::Request {
                    url,
                    api_key: api_key.to_owned(),
                },
            )
            .map_err(modelcatalog::CatalogError::into_protocol)
    }

    pub fn fetch_usage(
        &self,
        listener: &Listener,
        trusted: &Discovery,
        period: Period,
        api_key: &str,
        deadline: Instant,
    ) -> Result<Snapshot, ProtocolError> {
        let bind_deadline = min_deadline(deadline, VERSION_BUDGET);
        let mut transport = self.bind(listener, trusted, bind_deadline)?;
        let usage_deadline = min_deadline(deadline, apikeyusage::REQUEST_TIMEOUT);
        let url = format!("{}/usage", listener.api_base_url);
        apikeyusage::Client::new()
            .fetch_with(
                &mut transport,
                usage_deadline,
                apikeyusage::Request {
                    url,
                    period,
                    api_key: api_key.to_owned(),
                },
            )
            .map_err(apikeyusage::UsageError::into_protocol)
    }

    fn bind(
        &self,
        listener: &Listener,
        trusted: &Discovery,
        deadline: Instant,
    ) -> Result<BoundTransport, ProtocolError> {
        let mut transport = BoundTransport {
            listener: listener.clone(),
            stream: None,
            dialed: false,
            closed: false,
            dial: self.dial.clone(),
        };
        let response = transport
            .get("/version", None, deadline, VERSION_BODY_LIMIT + 1)
            .map_err(|_| discovery_failed())?;
        if response.status != 200 {
            return Err(discovery_failed());
        }
        if response.body.len() > VERSION_BODY_LIMIT {
            return Err(discovery_failed());
        }
        let remote: WireVersion =
            serde_json::from_slice(&response.body).map_err(|_| stale_catalog())?;
        if !version_matches(
            &RemoteVersion {
                pid: remote.pid,
                deployment_id: remote.deployment_id,
                management_protocol_version: remote.management_protocol_version,
            },
            trusted,
        ) {
            return Err(stale_catalog());
        }
        let identity = Identity {
            pid: trusted.state.pid,
            started_at: trusted.state.process_started_at.clone(),
            executable: trusted.state.process_executable.clone(),
        };
        let status = match &self.validate_process {
            Some(validate) => validate(&identity, &trusted.state.binary_path),
            None => process::validate(&identity, &trusted.state.binary_path),
        };
        if !matches!(status, Ok(Status::Genuine)) {
            return Err(stale_catalog());
        }
        Ok(transport)
    }
}

#[derive(Deserialize)]
struct WireVersion {
    #[serde(default)]
    pid: i32,
    #[serde(default)]
    deployment_id: String,
    #[serde(default)]
    management_protocol_version: String,
}

pub(super) struct BoundTransport {
    listener: Listener,
    stream: Option<TcpStream>,
    dialed: bool,
    closed: bool,
    dial: Option<DialFn>,
}

impl HttpTransport for BoundTransport {
    fn get(
        &mut self,
        path: &str,
        authorization: Option<&str>,
        deadline: Instant,
        max_body: usize,
    ) -> Result<httpget::HttpResponse, ()> {
        if self.closed || (self.dialed && self.stream.is_none()) {
            return Err(());
        }
        if self.stream.is_none() {
            if self.dialed {
                return Err(());
            }
            self.dialed = true;
            let stream = match &self.dial {
                Some(dial) => dial(&self.listener.authority).map_err(|_| ())?,
                None => {
                    let dial_deadline = min_deadline(deadline, DIAL_TIMEOUT);
                    httpget::connect(&self.listener.authority, dial_deadline)?
                }
            };
            self.stream = Some(stream);
        }
        let stream = self.stream.as_mut().ok_or(())?;
        let response = httpget::request(
            stream,
            &self.listener.authority,
            path,
            authorization,
            deadline,
            max_body,
        )?;
        if response.connection_close {
            self.closed = true;
            self.stream = None;
        }
        Ok(response)
    }
}

pub fn version_matches(remote: &RemoteVersion, trusted: &Discovery) -> bool {
    remote.pid > 0
        && remote.pid == trusted.state.pid
        && remote.deployment_id == trusted.state.deployment_id
        && remote.deployment_id == trusted.version.deployment_id
        && remote.management_protocol_version == trusted.state.management_protocol_version
        && remote.management_protocol_version == trusted.version.management_protocol_version
}

pub fn trusted_state_matches(
    found: &Discovery,
    listener: &Listener,
    deployment_id: &str,
    protocol_version: &str,
) -> bool {
    if !found.classification.is_trusted() {
        return false;
    }
    let value = &found.state;
    (value.owner == "cli" || value.owner == "desktop")
        && value.pid > 0
        && value.listen_addr == listener.router_base_url
        && found.listen_addr == listener.router_base_url
        && value.deployment_id == deployment_id
        && found.version.deployment_id == deployment_id
        && value.management_protocol_version == protocol_version
        && found.version.management_protocol_version == protocol_version
}

pub fn state_from_start(
    value: &super::StartedIdentity,
    listener: &Listener,
    deployment_id: &str,
    protocol_version: &str,
) -> bool {
    value.pid > 0
        && (value.owner == "cli" || value.owner == "desktop")
        && value.listen_addr == listener.router_base_url
        && value.deployment_id == deployment_id
        && value.management_protocol_version == protocol_version
}

pub fn discovery_failed() -> ProtocolError {
    closed_error(
        ErrorCode::ModelDiscoveryFailed,
        "model catalog request failed",
    )
}

pub fn stale_catalog() -> ProtocolError {
    closed_error(
        ErrorCode::ModelCatalogStale,
        "trusted router identity changed",
    )
}

pub fn min_deadline(parent: Instant, budget: Duration) -> Instant {
    let local = Instant::now() + budget;
    if parent < local {
        parent
    } else {
        local
    }
}
