#![allow(dead_code)]

//! Bounded `/v1/models` client and fail-closed parser.
//!
//! Frozen against Go `internal/manager/modelcatalog`.

mod client;
mod error;
mod parse;
mod simplify;

pub use client::{Client, Request, MAX_BODY_BYTES, REQUEST_TIMEOUT};
pub use error::{code_of, CatalogError};
pub use parse::{parse, MAX_MODELS};
#[cfg(test)]
pub use simplify::set_simplify_for_test;
pub use simplify::{parse_simplify, parse_simplify_value, SIMPLIFY_DEFAULT};

#[cfg(test)]
mod tests;
