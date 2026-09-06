use std::time::{Duration, Instant};

use crate::manager_core::httpget;

use super::error::{auth_failed, discovery_failed, response_invalid, CatalogError};
use super::parse::parse;

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
pub const MAX_BODY_BYTES: usize = 1 << 20;

#[derive(Clone, Debug)]
pub struct Request {
    pub url: String,
    pub api_key: String,
}

pub struct Client {
    timeout: Duration,
    simplify: bool,
}

impl Client {
    pub fn new(simplify: bool) -> Self {
        Self {
            timeout: REQUEST_TIMEOUT,
            simplify,
        }
    }

    #[cfg(test)]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn fetch(&self, request: Request) -> Result<Vec<String>, CatalogError> {
        let deadline = Instant::now() + self.timeout;
        self.fetch_by(deadline, request)
    }

    pub fn fetch_by(
        &self,
        deadline: Instant,
        request: Request,
    ) -> Result<Vec<String>, CatalogError> {
        let authority = httpget::parse_exact_http_url(&request.url, "/v1/models")
            .map_err(|_| discovery_failed())?;
        let mut transport = httpget::DirectTransport { authority };
        self.fetch_with(&mut transport, deadline, request)
    }

    pub fn fetch_with(
        &self,
        transport: &mut dyn httpget::HttpTransport,
        deadline: Instant,
        request: Request,
    ) -> Result<Vec<String>, CatalogError> {
        httpget::parse_exact_http_url(&request.url, "/v1/models")
            .map_err(|_| discovery_failed())?;
        let response = transport
            .get(
                "/v1/models",
                Some(&format!("Bearer {}", request.api_key)),
                deadline,
                MAX_BODY_BYTES + 1,
            )
            .map_err(|_| discovery_failed())?;
        match response.status {
            401 | 403 => return Err(auth_failed()),
            200 => {}
            _ => return Err(discovery_failed()),
        }
        if response.body.len() > MAX_BODY_BYTES {
            return Err(response_invalid());
        }
        parse(&response.body, self.simplify)
    }
}
