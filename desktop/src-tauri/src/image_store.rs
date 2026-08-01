//! Image conversation storage: versioned JSON snapshots, content-addressed
//! image assets, atomic writes, orphan cleanup, and fail-closed reading.
//!
//! All data lives under `<data_dir>/image-conversations/` with:
//! - `index.json` - versioned conversation/message/asset metadata snapshot
//! - `assets/<sha256>.<ext>` - content-addressed image files

use crate::image_validation::{self, ImageFormat, ValidatedImage};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

// --- Schema ---

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version: u32,
    #[serde(default)]
    pub selected_conversation_id: String,
    pub conversations: Vec<Conversation>,
    pub messages: Vec<Message>,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub selected_model: String,
    pub message_ids: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub role: MessageRole,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub reference_asset_id: String,
    #[serde(default)]
    pub model_id: String,
    #[serde(default)]
    pub status: MessageStatus,
    #[serde(default)]
    pub output_asset_id: String,
    #[serde(default)]
    pub error_category: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub completed_at: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

impl Default for MessageStatus {
    fn default() -> Self {
        Self::Succeeded
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub id: String,
    pub format: String,
    pub bytes: usize,
    pub width: u32,
    pub height: u32,
    pub source: AssetSource,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssetSource {
    Generation,
    Upload,
}

// --- Errors ---

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    InvalidVersion(u32),
    Corrupted(String),
    IoError(String),
    AssetNotFound(String),
    ConversationNotFound(String),
    MessageNotFound(String),
    ValidationFailed(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidVersion(v) => write!(f, "unsupported snapshot version {v}"),
            Self::Corrupted(msg) => write!(f, "snapshot is corrupted: {msg}"),
            Self::IoError(msg) => write!(f, "io error: {msg}"),
            Self::AssetNotFound(id) => write!(f, "asset {id} not found"),
            Self::ConversationNotFound(id) => write!(f, "conversation {id} not found"),
            Self::MessageNotFound(id) => write!(f, "message {id} not found"),
            Self::ValidationFailed(msg) => write!(f, "validation failed: {msg}"),
        }
    }
}

impl std::error::Error for StoreError {}

// --- ImagePaths ---

/// Private paths for image conversation data, derived from DesktopPaths.data_dir.
/// Never exposed to the WebView.
pub struct ImagePaths {
    pub root: PathBuf,
    pub index: PathBuf,
    pub assets: PathBuf,
}

impl ImagePaths {
    pub fn from_data_dir(data_dir: &str) -> Self {
        let root = PathBuf::from(data_dir).join("image-conversations");
        Self {
            index: root.join("index.json"),
            assets: root.join("assets"),
            root,
        }
    }

    pub fn asset_path(&self, asset_id: &str, format: &str) -> PathBuf {
        self.assets.join(format!("{asset_id}.{format}"))
    }

    pub fn ensure_dirs(&self) -> Result<(), StoreError> {
        create_dir_secure(&self.root)?;
        create_dir_secure(&self.assets)?;
        Ok(())
    }
}

fn create_dir_secure(path: &Path) -> Result<(), StoreError> {
    if !path.exists() {
        std::fs::create_dir_all(path).map_err(|e| StoreError::IoError(e.to_string()))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(path, perms).map_err(|e| StoreError::IoError(e.to_string()))?;
    }
    Ok(())
}

// --- Store ---

pub struct ImageStore {
    paths: ImagePaths,
}

impl ImageStore {
    pub fn new(paths: ImagePaths) -> Self {
        Self { paths }
    }

    pub fn from_data_dir(data_dir: &str) -> Self {
        Self::new(ImagePaths::from_data_dir(data_dir))
    }

    pub fn reset(&self) -> Result<(), StoreError> {
        match std::fs::remove_dir_all(&self.paths.root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(StoreError::IoError(error.to_string())),
        }
        self.save(&empty_snapshot())
    }

    // --- Load / Save ---

    /// Loads the snapshot. Returns an empty snapshot if the file doesn't exist.
    /// Fails closed on unknown version, corrupted data, or incomplete relations.
    pub fn load(&self) -> Result<Snapshot, StoreError> {
        if !self.paths.index.exists() {
            return Ok(empty_snapshot());
        }
        let data =
            std::fs::read(&self.paths.index).map_err(|e| StoreError::IoError(e.to_string()))?;
        let snapshot: Snapshot =
            serde_json::from_slice(&data).map_err(|e| StoreError::Corrupted(e.to_string()))?;
        if snapshot.version != SCHEMA_VERSION {
            return Err(StoreError::InvalidVersion(snapshot.version));
        }
        validate_relations(&snapshot)?;
        self.validate_asset_files(&snapshot)?;
        Ok(snapshot)
    }

    /// Atomically writes the snapshot via temp file + rename.
    /// Temp files contain only no-key conversation metadata.
    pub fn save(&self, snapshot: &Snapshot) -> Result<(), StoreError> {
        if snapshot.version != SCHEMA_VERSION {
            return Err(StoreError::InvalidVersion(snapshot.version));
        }
        validate_relations(snapshot)?;
        self.validate_asset_files(snapshot)?;
        self.paths.ensure_dirs()?;
        let json = serde_json::to_vec_pretty(snapshot)
            .map_err(|e| StoreError::Corrupted(e.to_string()))?;
        write_atomic(&self.paths.index, &json)
    }

    fn validate_asset_files(&self, snapshot: &Snapshot) -> Result<(), StoreError> {
        for asset in &snapshot.assets {
            if !self.paths.asset_path(&asset.id, &asset.format).is_file() {
                return Err(StoreError::Corrupted(format!(
                    "asset {} file is missing",
                    asset.id
                )));
            }
        }
        Ok(())
    }

    // --- Conversation CRUD ---

    pub fn create_conversation(&self, snapshot: &mut Snapshot, model: &str) -> Conversation {
        let id = new_id();
        let now = now_rfc3339();
        let conv = Conversation {
            id: id.clone(),
            title: String::new(),
            created_at: now.clone(),
            updated_at: now,
            selected_model: model.to_owned(),
            message_ids: Vec::new(),
        };
        snapshot.conversations.push(conv.clone());
        snapshot.selected_conversation_id = id;
        conv
    }

    pub fn delete_conversation(
        &self,
        snapshot: &mut Snapshot,
        conversation_id: &str,
    ) -> Result<Vec<String>, StoreError> {
        let conv_idx = snapshot
            .conversations
            .iter()
            .position(|c| c.id == conversation_id)
            .ok_or_else(|| StoreError::ConversationNotFound(conversation_id.into()))?;
        let conv = snapshot.conversations.remove(conv_idx);
        // Collect asset IDs referenced by this conversation's messages BEFORE removal
        let candidate_asset_ids: Vec<String> = snapshot
            .messages
            .iter()
            .filter(|m| m.conversation_id == conversation_id)
            .flat_map(|m| {
                let mut ids = Vec::new();
                if !m.reference_asset_id.is_empty() {
                    ids.push(m.reference_asset_id.clone());
                }
                if !m.output_asset_id.is_empty() {
                    ids.push(m.output_asset_id.clone());
                }
                ids
            })
            .collect();
        // Remove messages belonging to this conversation
        snapshot
            .messages
            .retain(|m| m.conversation_id != conversation_id);
        // Update selected conversation
        if snapshot.selected_conversation_id == conversation_id {
            snapshot.selected_conversation_id = snapshot
                .conversations
                .first()
                .map(|c| c.id.clone())
                .unwrap_or_default();
        }
        // Check which candidate assets are still referenced by remaining messages
        let still_referenced = referenced_asset_ids(snapshot);
        let mut removed_asset_ids = Vec::new();
        for asset_id in &candidate_asset_ids {
            if !still_referenced.contains(asset_id) {
                snapshot.assets.retain(|a| &a.id != asset_id);
                removed_asset_ids.push(asset_id.clone());
            }
        }
        let _ = conv;
        removed_asset_ids.sort();
        removed_asset_ids.dedup();
        Ok(removed_asset_ids)
    }

    pub fn set_title_from_prompt(snapshot: &mut Snapshot, conversation_id: &str, prompt: &str) {
        if let Some(conv) = snapshot
            .conversations
            .iter_mut()
            .find(|c| c.id == conversation_id)
        {
            if conv.title.is_empty() {
                let title: String = prompt.chars().take(50).collect();
                conv.title = title;
                conv.updated_at = now_rfc3339();
            }
        }
    }

    pub fn select_conversation(
        snapshot: &mut Snapshot,
        conversation_id: &str,
    ) -> Result<(), StoreError> {
        if !snapshot
            .conversations
            .iter()
            .any(|c| c.id == conversation_id)
        {
            return Err(StoreError::ConversationNotFound(conversation_id.into()));
        }
        snapshot.selected_conversation_id = conversation_id.to_owned();
        Ok(())
    }

    pub fn set_conversation_model(
        snapshot: &mut Snapshot,
        conversation_id: &str,
        model: &str,
    ) -> Result<(), StoreError> {
        let conversation = snapshot
            .conversations
            .iter_mut()
            .find(|conversation| conversation.id == conversation_id)
            .ok_or_else(|| StoreError::ConversationNotFound(conversation_id.into()))?;
        conversation.selected_model = model.to_owned();
        conversation.updated_at = now_rfc3339();
        Ok(())
    }

    // --- Messages ---

    pub fn add_user_message(
        &self,
        snapshot: &mut Snapshot,
        conversation_id: &str,
        prompt: &str,
        reference_asset_id: Option<&str>,
    ) -> Result<Message, StoreError> {
        let id = new_id();
        let now = now_rfc3339();
        let msg = Message {
            id: id.clone(),
            conversation_id: conversation_id.to_owned(),
            role: MessageRole::User,
            prompt: prompt.to_owned(),
            reference_asset_id: reference_asset_id.unwrap_or("").to_owned(),
            model_id: String::new(),
            status: MessageStatus::Succeeded,
            output_asset_id: String::new(),
            error_category: String::new(),
            created_at: now,
            completed_at: String::new(),
        };
        if let Some(conv) = snapshot
            .conversations
            .iter_mut()
            .find(|c| c.id == conversation_id)
        {
            conv.message_ids.push(id.clone());
            conv.updated_at = now_rfc3339();
        } else {
            return Err(StoreError::ConversationNotFound(conversation_id.into()));
        }
        Self::set_title_from_prompt(snapshot, conversation_id, prompt);
        snapshot.messages.push(msg.clone());
        Ok(msg)
    }

    pub fn add_running_assistant(
        &self,
        snapshot: &mut Snapshot,
        conversation_id: &str,
        model_id: &str,
    ) -> Result<Message, StoreError> {
        let id = new_id();
        let now = now_rfc3339();
        let msg = Message {
            id: id.clone(),
            conversation_id: conversation_id.to_owned(),
            role: MessageRole::Assistant,
            prompt: String::new(),
            reference_asset_id: String::new(),
            model_id: model_id.to_owned(),
            status: MessageStatus::Running,
            output_asset_id: String::new(),
            error_category: String::new(),
            created_at: now,
            completed_at: String::new(),
        };
        if let Some(conv) = snapshot
            .conversations
            .iter_mut()
            .find(|c| c.id == conversation_id)
        {
            conv.message_ids.push(id.clone());
            conv.updated_at = now_rfc3339();
        } else {
            return Err(StoreError::ConversationNotFound(conversation_id.into()));
        }
        snapshot.messages.push(msg.clone());
        Ok(msg)
    }

    pub fn complete_assistant_message(
        &self,
        snapshot: &mut Snapshot,
        message_id: &str,
        status: MessageStatus,
        output_asset_id: Option<&str>,
        error_category: Option<&str>,
    ) -> Result<(), StoreError> {
        let msg = snapshot
            .messages
            .iter_mut()
            .find(|m| m.id == message_id)
            .ok_or_else(|| StoreError::MessageNotFound(message_id.into()))?;
        msg.status = status;
        msg.output_asset_id = output_asset_id.unwrap_or("").to_owned();
        msg.error_category = error_category.unwrap_or("").to_owned();
        msg.completed_at = now_rfc3339();
        Ok(())
    }

    /// On startup, transitions all `running` messages to `interrupted`.
    pub fn finalize_leftover_running(&self, snapshot: &mut Snapshot) -> bool {
        let now = now_rfc3339();
        let mut changed = false;
        for msg in &mut snapshot.messages {
            if msg.status == MessageStatus::Running {
                msg.status = MessageStatus::Interrupted;
                msg.error_category = "interrupted".into();
                msg.completed_at = now.clone();
                changed = true;
            }
        }
        changed
    }

    pub fn prune_unreferenced_assets(&self, snapshot: &mut Snapshot) -> bool {
        let referenced = referenced_asset_ids(snapshot);
        let previous_len = snapshot.assets.len();
        snapshot
            .assets
            .retain(|asset| referenced.contains(&asset.id));
        snapshot.assets.len() != previous_len
    }

    // --- Assets ---

    /// Saves image bytes as a content-addressed asset. Returns the asset metadata.
    pub fn save_asset(
        &self,
        snapshot: &mut Snapshot,
        data: &[u8],
        validated: &ValidatedImage,
        source: AssetSource,
    ) -> Result<Asset, StoreError> {
        self.paths.ensure_dirs()?;
        let id = sha256_hex(data);
        // Check for existing asset (dedup)
        if let Some(existing) = snapshot.assets.iter().find(|a| a.id == id) {
            return Ok(existing.clone());
        }
        let format = match validated.format {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Webp => "webp",
        };
        let path = self.paths.asset_path(&id, format);
        write_atomic(&path, data)?;
        let asset = Asset {
            id: id.clone(),
            format: format.to_owned(),
            bytes: validated.byte_len,
            width: validated.width,
            height: validated.height,
            source,
        };
        snapshot.assets.push(asset.clone());
        Ok(asset)
    }

    /// Reads asset bytes by ID.
    pub fn read_asset(&self, snapshot: &Snapshot, asset_id: &str) -> Result<Vec<u8>, StoreError> {
        let asset = snapshot
            .assets
            .iter()
            .find(|a| a.id == asset_id)
            .ok_or_else(|| StoreError::AssetNotFound(asset_id.into()))?;
        let path = self.paths.asset_path(&asset.id, &asset.format);
        std::fs::read(&path).map_err(|e| StoreError::IoError(e.to_string()))
    }

    /// Returns the asset path for a given asset ID (for the custom URI handler).
    pub fn asset_path(&self, snapshot: &Snapshot, asset_id: &str) -> Result<PathBuf, StoreError> {
        let asset = snapshot
            .assets
            .iter()
            .find(|a| a.id == asset_id)
            .ok_or_else(|| StoreError::AssetNotFound(asset_id.into()))?;
        Ok(self.paths.asset_path(&asset.id, &asset.format))
    }

    /// Returns the MIME type for an asset.
    pub fn asset_mime(snapshot: &Snapshot, asset_id: &str) -> Result<&'static str, StoreError> {
        let asset = snapshot
            .assets
            .iter()
            .find(|a| a.id == asset_id)
            .ok_or_else(|| StoreError::AssetNotFound(asset_id.into()))?;
        Ok(match asset.format.as_str() {
            "png" => "image/png",
            "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            _ => "application/octet-stream",
        })
    }

    fn remove_asset_file(&self, asset_id: &str) -> Result<(), StoreError> {
        // Try all known extensions
        for ext in &["png", "jpeg", "webp"] {
            let path = self.paths.asset_path(asset_id, ext);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(StoreError::IoError(error.to_string())),
            }
        }
        Ok(())
    }

    pub fn remove_asset_files(&self, asset_ids: &[String]) -> Result<(), StoreError> {
        for asset_id in asset_ids {
            self.remove_asset_file(asset_id)?;
        }
        Ok(())
    }

    /// Cleans up orphan asset files (files in assets/ not referenced by any message).
    pub fn cleanup_orphans(&self, snapshot: &Snapshot) -> Result<usize, StoreError> {
        let referenced = referenced_asset_ids(snapshot);
        let mut cleaned = 0;
        if !self.paths.assets.exists() {
            return Ok(0);
        }
        let entries = std::fs::read_dir(&self.paths.assets)
            .map_err(|e| StoreError::IoError(e.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|e| StoreError::IoError(e.to_string()))?;
            let path = entry.path();
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if !referenced.contains(stem) {
                    std::fs::remove_file(&path)
                        .map_err(|error| StoreError::IoError(error.to_string()))?;
                    cleaned += 1;
                }
            }
        }
        Ok(cleaned)
    }

    // --- File import (task 3.5) ---

    /// Imports a single file as an asset. Validates format, size, and pixels
    /// before copying. Does not return absolute paths or raw bytes.
    pub fn import_file(
        &self,
        snapshot: &mut Snapshot,
        file_path: &Path,
    ) -> Result<Asset, StoreError> {
        let mut file = std::fs::File::open(file_path)
            .map_err(|error| StoreError::IoError(error.to_string()))?;
        let metadata = file
            .metadata()
            .map_err(|error| StoreError::IoError(error.to_string()))?;
        if !metadata.is_file() || metadata.len() > crate::image_limits::MAX_IMAGE_BYTES as u64 {
            return Err(StoreError::ValidationFailed(
                "selected file exceeds the image size limit".into(),
            ));
        }
        let mut data = Vec::with_capacity(metadata.len() as usize);
        std::io::Read::by_ref(&mut file)
            .take(crate::image_limits::MAX_IMAGE_BYTES as u64 + 1)
            .read_to_end(&mut data)
            .map_err(|error| StoreError::IoError(error.to_string()))?;
        if data.len() > crate::image_limits::MAX_IMAGE_BYTES {
            return Err(StoreError::ValidationFailed(
                "selected file exceeds the image size limit".into(),
            ));
        }
        let validated = image_validation::validate_image_bytes(&data)
            .map_err(|e| StoreError::ValidationFailed(e.to_string()))?;
        self.save_asset(snapshot, &data, &validated, AssetSource::Upload)
    }
}

// --- Helpers ---

fn empty_snapshot() -> Snapshot {
    Snapshot {
        version: SCHEMA_VERSION,
        selected_conversation_id: String::new(),
        conversations: Vec::new(),
        messages: Vec::new(),
        assets: Vec::new(),
    }
}

fn validate_relations(snapshot: &Snapshot) -> Result<(), StoreError> {
    let conv_ids: HashSet<&str> = snapshot
        .conversations
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    let msg_ids: HashSet<&str> = snapshot.messages.iter().map(|m| m.id.as_str()).collect();
    let asset_ids: HashSet<&str> = snapshot.assets.iter().map(|a| a.id.as_str()).collect();
    if conv_ids.len() != snapshot.conversations.len()
        || msg_ids.len() != snapshot.messages.len()
        || asset_ids.len() != snapshot.assets.len()
    {
        return Err(StoreError::Corrupted("duplicate IDs".into()));
    }
    if !snapshot.selected_conversation_id.is_empty()
        && !conv_ids.contains(snapshot.selected_conversation_id.as_str())
    {
        return Err(StoreError::Corrupted(
            "selected conversation does not exist".into(),
        ));
    }
    for asset in &snapshot.assets {
        if asset.id.len() != 64
            || !asset
                .id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !matches!(asset.format.as_str(), "png" | "jpeg" | "webp")
        {
            return Err(StoreError::Corrupted("invalid asset metadata".into()));
        }
    }
    for msg in &snapshot.messages {
        if !conv_ids.contains(msg.conversation_id.as_str()) {
            return Err(StoreError::Corrupted(format!(
                "message {} references unknown conversation {}",
                msg.id, msg.conversation_id
            )));
        }
        if !msg.reference_asset_id.is_empty()
            && !asset_ids.contains(msg.reference_asset_id.as_str())
        {
            return Err(StoreError::Corrupted(format!(
                "message {} references unknown asset {}",
                msg.id, msg.reference_asset_id
            )));
        }
        if !msg.output_asset_id.is_empty() && !asset_ids.contains(msg.output_asset_id.as_str()) {
            return Err(StoreError::Corrupted(format!(
                "message {} references unknown output asset {}",
                msg.id, msg.output_asset_id
            )));
        }
    }
    for conv in &snapshot.conversations {
        let mut listed = HashSet::new();
        for mid in &conv.message_ids {
            if !listed.insert(mid.as_str()) {
                return Err(StoreError::Corrupted(format!(
                    "conversation {} contains duplicate messages",
                    conv.id
                )));
            }
            let Some(message) = snapshot.messages.iter().find(|message| message.id == *mid) else {
                return Err(StoreError::Corrupted(format!(
                    "conversation {} references unknown message {}",
                    conv.id, mid
                )));
            };
            if message.conversation_id != conv.id {
                return Err(StoreError::Corrupted(format!(
                    "conversation {} contains a message from another conversation",
                    conv.id
                )));
            }
        }
    }
    let listed_messages: HashSet<&str> = snapshot
        .conversations
        .iter()
        .flat_map(|conversation| conversation.message_ids.iter().map(String::as_str))
        .collect();
    if listed_messages.len() != snapshot.messages.len() {
        return Err(StoreError::Corrupted(
            "one or more messages are not listed by their conversation".into(),
        ));
    }
    Ok(())
}

fn referenced_asset_ids(snapshot: &Snapshot) -> HashSet<String> {
    let mut ids = HashSet::new();
    for msg in &snapshot.messages {
        if !msg.reference_asset_id.is_empty() {
            ids.insert(msg.reference_asset_id.clone());
        }
        if !msg.output_asset_id.is_empty() {
            ids.insert(msg.output_asset_id.clone());
        }
    }
    ids
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<(), StoreError> {
    let dir = path
        .parent()
        .ok_or_else(|| StoreError::IoError("no parent dir".into()))?;
    let temp = dir.join(format!(
        ".{}-{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("image"),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp)
            .map_err(|error| StoreError::IoError(error.to_string()))?;
        file.write_all(data)
            .map_err(|error| StoreError::IoError(error.to_string()))?;
        file.sync_all()
            .map_err(|error| StoreError::IoError(error.to_string()))?;
        replace_file(&temp, path).map_err(|error| StoreError::IoError(error.to_string()))?;
        #[cfg(unix)]
        OpenOptions::new()
            .read(true)
            .open(dir)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| StoreError::IoError(error.to_string()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    let source = wide(source);
    let destination = wide(destination);
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    result.iter().map(|b| format!("{b:02x}")).collect()
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (ImageStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = ImageStore::from_data_dir(dir.path().to_str().unwrap());
        store.paths.ensure_dirs().unwrap();
        (store, dir)
    }

    fn make_png(width: u32, height: u32) -> (Vec<u8>, ValidatedImage) {
        let mut data = vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
            b'D', b'R',
        ];
        data.extend(width.to_be_bytes());
        data.extend(height.to_be_bytes());
        data.extend(&[8, 2, 0, 0, 0]);
        let validated = image_validation::validate_image_bytes(&data).unwrap();
        (data, validated)
    }

    #[test]
    fn load_returns_empty_when_no_file() {
        let (store, _dir) = temp_store();
        let snap = store.load().unwrap();
        assert!(snap.conversations.is_empty());
        assert_eq!(snap.version, SCHEMA_VERSION);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let (store, _dir) = temp_store();
        let mut snap = store.load().unwrap();
        store.create_conversation(&mut snap, "cx/gpt-5.5-image");
        store.save(&snap).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.conversations.len(), 1);
        assert_eq!(loaded.conversations[0].selected_model, "cx/gpt-5.5-image");
    }

    #[test]
    fn rejects_unknown_version() {
        let (store, _dir) = temp_store();
        let bad = r#"{"version":99,"selected_conversation_id":"","conversations":[],"messages":[],"assets":[]}"#;
        std::fs::write(&store.paths.index, bad).unwrap();
        assert_eq!(store.load(), Err(StoreError::InvalidVersion(99)));
    }

    #[test]
    fn rejects_corrupted_json() {
        let (store, _dir) = temp_store();
        std::fs::write(&store.paths.index, b"not json").unwrap();
        assert!(matches!(store.load(), Err(StoreError::Corrupted(_))));
    }

    #[test]
    fn explicit_reset_replaces_corrupt_data() {
        let (store, _dir) = temp_store();
        std::fs::write(&store.paths.index, b"not json").unwrap();
        store.reset().unwrap();
        assert_eq!(store.load().unwrap(), empty_snapshot());
    }

    #[test]
    fn rejects_incomplete_relations() {
        let (store, _dir) = temp_store();
        let bad = r#"{"version":1,"selected_conversation_id":"","conversations":[],"messages":[{"id":"m1","conversation_id":"nonexistent","role":"user","prompt":"","reference_asset_id":"","model_id":"","status":"succeeded","output_asset_id":"","error_category":"","created_at":"","completed_at":""}],"assets":[]}"#;
        std::fs::write(&store.paths.index, bad).unwrap();
        assert!(matches!(store.load(), Err(StoreError::Corrupted(_))));
    }

    #[test]
    fn create_and_delete_conversation() {
        let (store, _dir) = temp_store();
        let mut snap = store.load().unwrap();
        let conv = store.create_conversation(&mut snap, "cx/gpt-5.5-image");
        assert_eq!(snap.conversations.len(), 1);
        assert_eq!(snap.selected_conversation_id, conv.id);
        store.delete_conversation(&mut snap, &conv.id).unwrap();
        assert!(snap.conversations.is_empty());
        assert!(snap.selected_conversation_id.is_empty());
    }

    #[test]
    fn title_from_first_prompt() {
        let (store, _dir) = temp_store();
        let mut snap = store.load().unwrap();
        let conv = store.create_conversation(&mut snap, "cx/gpt-5.5-image");
        store
            .add_user_message(
                &mut snap,
                &conv.id,
                "A beautiful sunset over mountains",
                None,
            )
            .unwrap();
        assert_eq!(
            snap.conversations[0].title,
            "A beautiful sunset over mountains"
        );
    }

    #[test]
    fn add_running_then_complete() {
        let (store, _dir) = temp_store();
        let mut snap = store.load().unwrap();
        let conv = store.create_conversation(&mut snap, "cx/gpt-5.5-image");
        let msg = store
            .add_running_assistant(&mut snap, &conv.id, "cx/gpt-5.5-image")
            .unwrap();
        assert_eq!(msg.status, MessageStatus::Running);
        store
            .complete_assistant_message(
                &mut snap,
                &msg.id,
                MessageStatus::Succeeded,
                Some("asset-1"),
                None,
            )
            .unwrap();
        let updated = snap.messages.iter().find(|m| m.id == msg.id).unwrap();
        assert_eq!(updated.status, MessageStatus::Succeeded);
        assert_eq!(updated.output_asset_id, "asset-1");
    }

    #[test]
    fn finalize_leftover_running() {
        let (store, _dir) = temp_store();
        let mut snap = store.load().unwrap();
        let conv = store.create_conversation(&mut snap, "cx/gpt-5.5-image");
        store
            .add_running_assistant(&mut snap, &conv.id, "cx/gpt-5.5-image")
            .unwrap();
        store.finalize_leftover_running(&mut snap);
        assert!(snap
            .messages
            .iter()
            .all(|m| m.status != MessageStatus::Running));
        assert!(snap
            .messages
            .iter()
            .all(|m| m.status == MessageStatus::Interrupted));
    }

    #[test]
    fn save_and_dedup_asset() {
        let (store, _dir) = temp_store();
        let mut snap = store.load().unwrap();
        let (data, validated) = make_png(2, 2);
        let asset1 = store
            .save_asset(&mut snap, &data, &validated, AssetSource::Generation)
            .unwrap();
        let asset2 = store
            .save_asset(&mut snap, &data, &validated, AssetSource::Generation)
            .unwrap();
        assert_eq!(asset1.id, asset2.id);
        assert_eq!(snap.assets.len(), 1);
    }

    #[test]
    fn delete_conversation_cleans_unreferenced_assets() {
        let (store, _dir) = temp_store();
        let mut snap = store.load().unwrap();
        let conv = store.create_conversation(&mut snap, "cx/gpt-5.5-image");
        let (data, validated) = make_png(2, 2);
        let asset = store
            .save_asset(&mut snap, &data, &validated, AssetSource::Generation)
            .unwrap();
        store
            .add_user_message(&mut snap, &conv.id, "test", None)
            .unwrap();
        let asst_msg = store
            .add_running_assistant(&mut snap, &conv.id, "cx/gpt-5.5-image")
            .unwrap();
        store
            .complete_assistant_message(
                &mut snap,
                &asst_msg.id,
                MessageStatus::Succeeded,
                Some(&asset.id),
                None,
            )
            .unwrap();
        assert_eq!(snap.assets.len(), 1);
        let removed = store.delete_conversation(&mut snap, &conv.id).unwrap();
        assert!(snap.assets.is_empty());
        assert!(store.paths.asset_path(&asset.id, "png").exists());
        store.save(&snap).unwrap();
        store.remove_asset_files(&removed).unwrap();
        assert!(!store.paths.asset_path(&asset.id, "png").exists());
    }

    #[test]
    fn cleanup_orphans_removes_unreferenced_files() {
        let (store, _dir) = temp_store();
        let snap = store.load().unwrap();
        // Write an orphan file directly
        let orphan_path = store.paths.asset_path("orphan123", "png");
        std::fs::write(&orphan_path, b"fake").unwrap();
        assert!(orphan_path.exists());
        let cleaned = store.cleanup_orphans(&snap).unwrap();
        assert_eq!(cleaned, 1);
        assert!(!orphan_path.exists());
    }

    #[test]
    fn cross_conversation_asset_not_cleaned_prematurely() {
        let (store, _dir) = temp_store();
        let mut snap = store.load().unwrap();
        let conv1 = store.create_conversation(&mut snap, "cx/gpt-5.5-image");
        let conv2 = store.create_conversation(&mut snap, "cx/gpt-5.5-image");
        let (data, validated) = make_png(2, 2);
        let asset = store
            .save_asset(&mut snap, &data, &validated, AssetSource::Generation)
            .unwrap();
        // Both conversations reference the same asset
        let asst1 = store
            .add_running_assistant(&mut snap, &conv1.id, "cx/gpt-5.5-image")
            .unwrap();
        store
            .complete_assistant_message(
                &mut snap,
                &asst1.id,
                MessageStatus::Succeeded,
                Some(&asset.id),
                None,
            )
            .unwrap();
        let asst2 = store
            .add_running_assistant(&mut snap, &conv2.id, "cx/gpt-5.5-image")
            .unwrap();
        store
            .complete_assistant_message(
                &mut snap,
                &asst2.id,
                MessageStatus::Succeeded,
                Some(&asset.id),
                None,
            )
            .unwrap();
        // Delete conv1 - asset should still exist because conv2 references it
        let removed = store.delete_conversation(&mut snap, &conv1.id).unwrap();
        store.remove_asset_files(&removed).unwrap();
        assert_eq!(snap.assets.len(), 1);
        assert!(store.paths.asset_path(&asset.id, "png").exists());
        // Delete conv2 - now asset should be cleaned
        let removed = store.delete_conversation(&mut snap, &conv2.id).unwrap();
        store.remove_asset_files(&removed).unwrap();
        assert!(snap.assets.is_empty());
    }

    #[test]
    fn temp_files_contain_no_secrets() {
        let (store, _dir) = temp_store();
        let mut snap = store.load().unwrap();
        let conv = store.create_conversation(&mut snap, "cx/gpt-5.5-image");
        store
            .add_user_message(&mut snap, &conv.id, "prompt-with-no-key", None)
            .unwrap();
        store.save(&snap).unwrap();
        let data = std::fs::read(&store.paths.index).unwrap();
        let text = String::from_utf8(data).unwrap();
        assert!(!text.contains("api_key"));
        assert!(!text.contains("Authorization"));
        assert!(!text.contains("Bearer"));
    }

    #[test]
    fn import_file_validates_before_saving() {
        let (store, _dir) = temp_store();
        let mut snap = store.load().unwrap();
        let (data, _) = make_png(2, 2);
        let temp_file = store.paths.root.join("test-input.png");
        std::fs::write(&temp_file, &data).unwrap();
        let asset = store.import_file(&mut snap, &temp_file).unwrap();
        assert_eq!(asset.source, AssetSource::Upload);
        assert_eq!(asset.format, "png");
        let _ = std::fs::remove_file(&temp_file);
    }

    #[test]
    fn import_rejects_invalid_image() {
        let (store, _dir) = temp_store();
        let mut snap = store.load().unwrap();
        let temp_file = store.paths.root.join("not-image.txt");
        std::fs::write(&temp_file, b"not an image").unwrap();
        assert!(store.import_file(&mut snap, &temp_file).is_err());
        let _ = std::fs::remove_file(&temp_file);
    }

    #[test]
    fn import_rejects_oversized_file_before_reading_it() {
        let (store, _dir) = temp_store();
        let mut snap = store.load().unwrap();
        let temp_file = store.paths.root.join("oversized.png");
        let file = std::fs::File::create(&temp_file).unwrap();
        file.set_len(crate::image_limits::MAX_IMAGE_BYTES as u64 + 1)
            .unwrap();
        drop(file);
        assert!(matches!(
            store.import_file(&mut snap, &temp_file),
            Err(StoreError::ValidationFailed(_))
        ));
    }
}
