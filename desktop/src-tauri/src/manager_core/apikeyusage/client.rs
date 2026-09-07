use std::time::{Duration, Instant};

use crate::manager_core::httpget::{self, HttpTransport};

use super::error::{auth_failed, request_failed, response_invalid, unavailable, UsageError};
use super::parse::{normalize_period, parse, Period, Snapshot};

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(25);
pub const MAX_BODY_BYTES: usize = 1 << 20;

#[derive(Clone, Debug)]
pub struct Request {
    pub url: String,
    pub period: Period,
    pub api_key: String,
}

pub struct Client {
    timeout: Duration,
}

impl Client {
    pub fn new() -> Self {
        Self {
            timeout: REQUEST_TIMEOUT,
        }
    }

    #[cfg(test)]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn fetch(&self, request: Request) -> Result<Snapshot, UsageError> {
        self.exchange(None, Instant::now() + self.timeout, request)
    }

    pub fn fetch_with(
        &self,
        transport: &mut dyn httpget::HttpTransport,
        deadline: Instant,
        request: Request,
    ) -> Result<Snapshot, UsageError> {
        self.exchange(Some(transport), deadline, request)
    }

    fn exchange(
        &self,
        transport: Option<&mut dyn httpget::HttpTransport>,
        deadline: Instant,
        request: Request,
    ) -> Result<Snapshot, UsageError> {
        let period = normalize_period(request.period.as_str())?;
        let authority = httpget::parse_exact_http_url(&request.url, "/v1/usage")
            .map_err(|_| request_failed())?;
        let path = format!("/v1/usage?period={}", period.as_str());
        let authorization = format!("Bearer {}", request.api_key);
        let response = match transport {
            Some(bound) => bound.get(&path, Some(&authorization), deadline, MAX_BODY_BYTES + 1),
            None => httpget::DirectTransport { authority }.get(
                &path,
                Some(&authorization),
                deadline,
                MAX_BODY_BYTES + 1,
            ),
        }
        .map_err(|_| request_failed())?;
        match response.status {
            401 | 403 => return Err(auth_failed()),
            404 | 405 | 501 => return Err(unavailable()),
            200 => {}
            _ => return Err(request_failed()),
        }
        if response.body.len() > MAX_BODY_BYTES {
            return Err(response_invalid());
        }
        parse(&response.body, period)
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}
