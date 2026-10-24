//! Shared HTTP/1 streaming router; no Tauri, WebKit or subprocess dependency.
pub use router_contract::{protocol, redaction};
pub mod router_core;
pub use router_core::*;
