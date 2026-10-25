use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, Duration as ChronoDuration, NaiveDate, SecondsFormat, Utc};

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
        let (start, end) = window(period, Utc::now());
        // `period` keeps the legacy jump window; jump2 only honors start/end.
        let path = format!(
            "/v1/usage?period={}&start={}&end={}",
            period.as_str(),
            start.to_rfc3339_opts(SecondsFormat::Secs, true),
            end.to_rfc3339_opts(SecondsFormat::Secs, true)
        );
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

/// Calendar windows start at 08:00 Beijing time, which is 00:00 UTC.
pub(super) fn window(period: Period, now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let today = now.date_naive();
    let start = match period {
        Period::OneHour => now - ChronoDuration::hours(1),
        Period::TwelveHours => now - ChronoDuration::hours(12),
        Period::TwentyFourHours => now - ChronoDuration::hours(24),
        Period::SevenDays => now - ChronoDuration::days(7),
        Period::ThirtyDays => now - ChronoDuration::days(30),
        Period::Today => midnight(today),
        Period::ThisWeek => {
            let days = i64::from(now.weekday().num_days_from_monday());
            midnight(today - ChronoDuration::days(days))
        }
        Period::ThisMonth => midnight(today.with_day(1).unwrap_or(today)),
    };
    (start, now)
}

fn midnight(date: NaiveDate) -> DateTime<Utc> {
    date.and_hms_opt(0, 0, 0)
        .expect("midnight is valid")
        .and_utc()
}
