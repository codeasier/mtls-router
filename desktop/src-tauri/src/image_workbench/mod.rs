#![allow(unused_imports)]

//! Local chat-and-image workbench data plane.
//!
//! WebView talks to this module through narrow IPC. API keys stay in Rust and
//! only travel on a single non-redialable loopback HTTP/1.1 connection after
//! `/version`, process identity, and `/health` succeed.

mod catalog;
mod channel;
mod error;
pub(crate) mod image;
pub mod ipc;
mod orchestrate;
pub(crate) mod scheme;
mod service;
mod store;

pub use catalog::{
    parse_chat_models, parse_image_models, select_chat_model, select_image_model, ImageChoice,
    DEFAULT_CHAT_MODEL, DEFAULT_IMAGE_MODEL, GPT_IMAGE_2, SIZES,
};
pub use error::{SafeKind, WorkbenchError};
pub use image::MAX_IMAGE_BYTES;
pub use image::{validate_image, ValidatedImage};
pub use ipc::{
    WorkbenchCommands, CHAT_DELTA_EVENT, IMAGE_STATUS_EVENT, OPERATION_DONE_EVENT, PHASE_EVENT,
};
pub use orchestrate::{
    decide_jobs, extract_image_payload, fallback_edit_text, rebuild_chat_messages, ImageSpec,
    JobDecision, RebuildInput, SYSTEM_PROMPT,
};
pub use scheme::{asset_id_from_uri, not_found, serve_asset_response};
pub use service::Workbench;
pub use store::{
    AssetMeta, Conversation, ImageJob, Message, Snapshot, WorkbenchPaths, WorkbenchStore,
};

#[cfg(test)]
mod tests;
