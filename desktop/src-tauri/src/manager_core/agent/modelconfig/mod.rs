//! Key-free Agent model config schema v1: decode, merge, JCS, and token MACs.
//!
//! Frozen against Go `internal/manager/agent/modelconfig`.

mod canonical;
mod decode;
mod merge;
mod token;
mod types;
mod validate;

pub use canonical::{
    canonical, canonical_value, config_to_value, marshal_jcs, JsonNumber, JsonValue,
};
pub use decode::{decode, decode_structural, decode_value, object_from_value};
pub use merge::deep_merge;
pub use token::{
    CatalogClaims, CleanupRevisionClaims, CleanupRevisionFile, RevisionAgent, RevisionClaims,
    RevisionFile, RevisionState, TokenError, TokenSigner, CANONICALIZATION_JCS1,
    MAX_CATALOG_TOKEN_SIZE, MAX_CLEANUP_REVISION_TOKEN_SIZE, MAX_REVISION_TOKEN_SIZE,
    TOKEN_VERSION,
};
pub use types::{
    Agent, ClaudeConfig, ClaudeContext, ClaudeRole, CodexConfig, Config, Interleaved, Limit,
    Modalities, Model, OpenCodeConfig, OpenCodeModelConfig, ValidationError, Version,
    MAX_CONFIG_SIZE, MAX_REFERENCED_MODELS_PER_AGENT, MAX_SAFE_INTEGER,
};

#[cfg(test)]
mod tests;
