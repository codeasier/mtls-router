#![allow(dead_code)]

//! Bounded per-key `/v1/usage` client and fail-closed parser.
//!
//! Frozen against Go `internal/manager/apikeyusage`.

mod client;
mod error;
mod parse;

pub use client::{Client, Request, MAX_BODY_BYTES, REQUEST_TIMEOUT};
pub use error::{code_of, UsageError};
pub use parse::{
    normalize_period, parse, Model, Period, Quota, QuotaUnit, Snapshot, Summary, MAX_MODELS,
};

#[cfg(test)]
mod tests;
