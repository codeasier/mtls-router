//! GUI-independent protocol, process identity and safe diagnostic contracts.
pub mod cli_state;
pub mod error;
pub mod manager_diagnostics;
pub mod process;
pub mod protocol;
pub mod redaction;
#[cfg(windows)]
pub mod windows_security;
