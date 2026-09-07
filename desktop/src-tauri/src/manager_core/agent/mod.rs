//! Agent detection, model config, transactional writes, and cleanup.
//!
//! Frozen against Go `internal/manager/agent`. This module is not the default
//! sidecar runtime path.

pub mod modelconfig;

mod cleanup;
mod detect;
mod files;
#[cfg(not(windows))]
mod files_unix;
#[cfg(windows)]
mod files_windows;
mod journal;
mod lock;
#[cfg(not(windows))]
mod lock_unix;
#[cfg(windows)]
mod lock_windows;
mod models;
mod paths;
mod plan;
mod preview;
mod recovery;
#[cfg(windows)]
mod recovery_windows;
mod render;
mod revision;
mod service;
mod sidecar;
mod toml;
mod trust;
mod trusted;
mod types;
mod url;

pub use cleanup::CleanupPreview;
pub use detect::{strip_jsonc, Detector};
pub use models::{ModelsExisting, ModelsPreset, ModelsResult};
pub use paths::{claude_paths, codex_paths, opencode_paths, Paths};
pub use plan::{
    CatalogBinding, ConfigMode, FilePreview, Preview, PreviewRequest, WriteRequest, WriteResult,
};
pub use render::{render_fragments, Fragment, RenderInput, RenderResult};
pub use service::{
    isolated_service, normalize_selection, validate_refreshed_models, AgentService, Options,
};
pub use trusted::{StaticCatalog, TrustedBinding, TrustedCatalog, TrustedModels};
pub use types::{
    is_managed_provider_display_name, CleanupState, CleanupUnavailableReason, Format,
    OperationError, RecoveryFileState, RecoveryReason, RecoveryState, State, MAX_CONFIG_SIZE,
    MAX_RENDER_SIZE, PROVIDER_DISPLAY_NAME, REDACTED_API_KEY,
};
pub use url::{api_url, safe_display_path, valid_api_value, valid_router_base_url};
