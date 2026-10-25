#![allow(dead_code)]

//! Bounded per-key `/v1/usage` client and fail-closed parser.
//!
//! Mirrors Go `internal/manager/apikeyusage`, and additionally accepts the
//! jump2 relay `recorded_usage` shape (`total`/`byModel`/window object) that
//! the per-user mTLS relay serves. Windows are sent as `start`/`end` because
//! jump2 ignores the legacy `period` token; it is still sent for jump.

mod client;
mod error;
mod parse;

pub use client::{Client, Request, MAX_BODY_BYTES, REQUEST_TIMEOUT};
pub use error::{code_of, UsageError};
pub use parse::{
    normalize_period, parse, BudgetPeriod, Model, Period, ProviderQuota, Quota, QuotaUnit,
    Snapshot, Summary, MAX_MODELS, MAX_QUOTAS,
};

#[cfg(test)]
mod tests;
