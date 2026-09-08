use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use unicode_segmentation::UnicodeSegmentation;
use uuid::Uuid;

use super::error::{SafeKind, WorkbenchError};
use super::image::{validate_image, ImageFormat, ValidatedImage};
use super::orchestrate::{title_from, TITLE_GRAPHEMES};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

/// Writer schema this build emits for new files.
pub const SCHEMA_VERSION: u32 = 1;
/// Oldest reader that can safely use files this build writes.
/// Bump only for breaking changes. Additive fields keep this at 1 and raise
/// `SCHEMA_VERSION` so older builds still load known fields.
pub const MIN_READER_VERSION: u32 = 1;

fn default_min_reader() -> u32 {
    MIN_READER_VERSION
}

fn extra_map() -> Map<String, Value> {
    Map::new()
}

#[derive(Clone, Debug)]
pub struct WorkbenchPaths {
    root: PathBuf,
    index: PathBuf,
    assets: PathBuf,
}

impl WorkbenchPaths {
    pub fn from_data_dir(data_dir: impl AsRef<Path>) -> Self {
        let root = data_dir.as_ref().join("image-workbench");
        Self {
            index: root.join("index.json"),
            assets: root.join("assets"),
            root,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn asset_file(&self, id: &str, format: ImageFormat) -> PathBuf {
        self.assets.join(format!("{id}.{}", format.ext()))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Snapshot {
    pub version: u32,
    #[serde(default = "default_min_reader")]
    pub min_reader_version: u32,
    pub selected_conversation_id: Option<String>,
    pub conversations: Vec<Conversation>,
    pub messages: Vec<Message>,
    pub jobs: Vec<ImageJob>,
    pub assets: Vec<AssetMeta>,
    #[serde(flatten, default = "extra_map")]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub selected_chat_model: Option<String>,
    pub selected_image_model: Option<String>,
    pub selected_size: String,
    pub message_ids: Vec<String>,
    #[serde(flatten, default = "extra_map")]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub visible_text: String,
    pub created_at: String,
    pub reference_asset_id: Option<String>,
    pub phase: Option<String>,
    pub error_kind: Option<String>,
    pub job_ids: Vec<String>,
    #[serde(flatten, default = "extra_map")]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ImageJob {
    pub id: String,
    pub message_id: String,
    pub action: String,
    pub prompt: String,
    pub size: String,
    pub status: String,
    pub output_asset_id: Option<String>,
    pub error_kind: Option<String>,
    #[serde(flatten, default = "extra_map")]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AssetMeta {
    pub id: String,
    pub format: String,
    pub byte_len: u64,
    pub width: u32,
    pub height: u32,
    pub source: String,
    #[serde(flatten, default = "extra_map")]
    pub extra: Map<String, Value>,
}

pub struct WorkbenchStore {
    paths: WorkbenchPaths,
    inner: Mutex<StoreState>,
}

enum StoreState {
    Ready(Snapshot),
    Corrupt,
    Incompatible,
}

impl WorkbenchStore {
    pub fn open(paths: WorkbenchPaths) -> Result<Self, WorkbenchError> {
        prepare_dirs(&paths)?;
        let (state, may_sweep) = match fs::read(&paths.index) {
            Ok(bytes) => match load_snapshot(&bytes) {
                Load::Ready(mut snapshot) => {
                    interrupt_running(&mut snapshot);
                    let _ = write_snapshot(&paths.index, &snapshot);
                    (StoreState::Ready(snapshot), true)
                }
                Load::Incompatible => (StoreState::Incompatible, false),
                Load::Corrupt => (StoreState::Corrupt, false),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                (StoreState::Ready(empty_snapshot()), false)
            }
            Err(_) => (StoreState::Corrupt, false),
        };
        let store = Self {
            paths,
            inner: Mutex::new(state),
        };
        if may_sweep {
            if let Ok(Some(snapshot)) = store.inner.lock().map(|guard| guard.clone_ready()) {
                let _ = store.sweep_orphans(&snapshot);
            }
        }
        Ok(store)
    }

    pub fn snapshot(&self) -> Result<Snapshot, WorkbenchError> {
        match &*self
            .inner
            .lock()
            .map_err(|_| WorkbenchError::new(SafeKind::StoreCorrupt))?
        {
            StoreState::Ready(snapshot) => Ok(snapshot.clone()),
            StoreState::Corrupt => Err(WorkbenchError::new(SafeKind::StoreCorrupt)),
            StoreState::Incompatible => Err(WorkbenchError::new(SafeKind::StoreIncompatible)),
        }
    }

    pub fn is_corrupt(&self) -> bool {
        matches!(
            self.inner.lock().map(|g| matches!(*g, StoreState::Corrupt)),
            Ok(true)
        )
    }

    pub fn is_incompatible(&self) -> bool {
        matches!(
            self.inner
                .lock()
                .map(|g| matches!(*g, StoreState::Incompatible)),
            Ok(true)
        )
    }

    pub fn rebuild(&self) -> Result<Snapshot, WorkbenchError> {
        let snapshot = empty_snapshot();
        self.commit(&snapshot)?;
        let _ = fs::remove_dir_all(&self.paths.assets);
        prepare_dirs(&self.paths)?;
        *self
            .inner
            .lock()
            .map_err(|_| WorkbenchError::new(SafeKind::StoreCorrupt))? =
            StoreState::Ready(snapshot.clone());
        Ok(snapshot)
    }

    pub fn create_conversation(
        &self,
        chat: Option<String>,
        image: Option<String>,
        size: String,
    ) -> Result<Conversation, WorkbenchError> {
        let mut snapshot = self.snapshot()?;
        let now = now_rfc3339();
        let conversation = Conversation {
            id: new_id("c"),
            title: "新对话".to_owned(),
            created_at: now.clone(),
            updated_at: now,
            selected_chat_model: chat,
            selected_image_model: image,
            selected_size: size,
            message_ids: Vec::new(),
            extra: extra_map(),
        };
        snapshot.selected_conversation_id = Some(conversation.id.clone());
        snapshot.conversations.insert(0, conversation.clone());
        self.persist(snapshot)?;
        Ok(conversation)
    }

    pub fn select_conversation(&self, id: &str) -> Result<Snapshot, WorkbenchError> {
        let mut snapshot = self.snapshot()?;
        if !snapshot.conversations.iter().any(|item| item.id == id) {
            return Err(WorkbenchError::new(SafeKind::Unknown));
        }
        snapshot.selected_conversation_id = Some(id.to_owned());
        self.persist(snapshot)
    }

    pub fn delete_conversation(&self, id: &str) -> Result<Snapshot, WorkbenchError> {
        let mut snapshot = self.snapshot()?;
        snapshot.conversations.retain(|item| item.id != id);
        let removed_messages: Vec<String> = snapshot
            .messages
            .iter()
            .filter(|item| item.conversation_id == id)
            .map(|item| item.id.clone())
            .collect();
        snapshot
            .jobs
            .retain(|job| !removed_messages.iter().any(|mid| mid == &job.message_id));
        snapshot.messages.retain(|item| item.conversation_id != id);
        if snapshot.selected_conversation_id.as_deref() == Some(id) {
            snapshot.selected_conversation_id =
                snapshot.conversations.first().map(|item| item.id.clone());
        }
        let kept = referenced_assets(&snapshot);
        let removed_assets: Vec<AssetMeta> = snapshot
            .assets
            .iter()
            .filter(|asset| !kept.contains(&asset.id))
            .cloned()
            .collect();
        snapshot.assets.retain(|asset| kept.contains(&asset.id));
        self.persist(snapshot.clone())?;
        for asset in removed_assets {
            let _ = self.remove_asset_file(&asset);
        }
        Ok(snapshot)
    }

    pub fn set_options(
        &self,
        conversation_id: &str,
        chat: Option<Option<String>>,
        image: Option<Option<String>>,
        size: Option<String>,
    ) -> Result<Snapshot, WorkbenchError> {
        let mut snapshot = self.snapshot()?;
        let conversation = snapshot
            .conversations
            .iter_mut()
            .find(|item| item.id == conversation_id)
            .ok_or_else(|| WorkbenchError::new(SafeKind::Unknown))?;
        if let Some(value) = chat {
            conversation.selected_chat_model = value;
        }
        if let Some(value) = image {
            conversation.selected_image_model = value;
        }
        if let Some(value) = size {
            conversation.selected_size = value;
        }
        conversation.updated_at = now_rfc3339();
        self.persist(snapshot)
    }

    pub fn import_image(&self, bytes: &[u8], source: &str) -> Result<AssetMeta, WorkbenchError> {
        let validated = validate_image(bytes)?;
        self.persist_asset(validated, source)
    }

    pub fn persist_asset(
        &self,
        validated: ValidatedImage,
        source: &str,
    ) -> Result<AssetMeta, WorkbenchError> {
        let id = hex_sha256(&validated.bytes);
        let meta = AssetMeta {
            id: id.clone(),
            format: validated.format.ext().to_owned(),
            byte_len: validated.bytes.len() as u64,
            width: validated.width,
            height: validated.height,
            source: source.to_owned(),
            extra: extra_map(),
        };
        let path = self.paths.asset_file(&id, validated.format);
        if !path.exists() {
            write_atomic(&path, &validated.bytes)?;
        }
        let mut snapshot = self.snapshot()?;
        if !snapshot.assets.iter().any(|item| item.id == id) {
            snapshot.assets.push(meta.clone());
            self.persist(snapshot)?;
        }
        Ok(meta)
    }

    pub fn read_asset(&self, id: &str) -> Result<(AssetMeta, Vec<u8>), WorkbenchError> {
        if !is_sha256(id) {
            return Err(WorkbenchError::new(SafeKind::ImageInvalid));
        }
        let snapshot = self.snapshot()?;
        let meta = snapshot
            .assets
            .iter()
            .find(|item| item.id == id)
            .cloned()
            .ok_or_else(|| WorkbenchError::new(SafeKind::Unknown))?;
        let format = match meta.format.as_str() {
            "png" => ImageFormat::Png,
            "jpg" | "jpeg" => ImageFormat::Jpeg,
            "webp" => ImageFormat::Webp,
            _ => return Err(WorkbenchError::new(SafeKind::ImageInvalid)),
        };
        let bytes = fs::read(self.paths.asset_file(id, format))
            .map_err(|_| WorkbenchError::new(SafeKind::Unknown))?;
        if hex_sha256(&bytes) != id {
            return Err(WorkbenchError::new(SafeKind::ImageInvalid));
        }
        Ok((meta, bytes))
    }

    pub fn apply<F>(&self, mutate: F) -> Result<Snapshot, WorkbenchError>
    where
        F: FnOnce(&mut Snapshot),
    {
        let mut snapshot = self.snapshot()?;
        mutate(&mut snapshot);
        self.persist(snapshot)
    }

    pub fn title_for_first_user(text: &str) -> String {
        let title = title_from(text.trim());
        if title.is_empty() {
            "新对话".to_owned()
        } else {
            title.graphemes(true).take(TITLE_GRAPHEMES).collect()
        }
    }

    fn persist(&self, mut snapshot: Snapshot) -> Result<Snapshot, WorkbenchError> {
        if snapshot.version <= SCHEMA_VERSION {
            snapshot.version = SCHEMA_VERSION;
            snapshot.min_reader_version = MIN_READER_VERSION;
        }
        self.commit(&snapshot)?;
        *self
            .inner
            .lock()
            .map_err(|_| WorkbenchError::new(SafeKind::StoreCorrupt))? =
            StoreState::Ready(snapshot.clone());
        Ok(snapshot)
    }

    fn commit(&self, snapshot: &Snapshot) -> Result<(), WorkbenchError> {
        if snapshot.min_reader_version > SCHEMA_VERSION {
            return Err(WorkbenchError::new(SafeKind::StoreIncompatible));
        }
        if snapshot.version == 0 {
            return Err(WorkbenchError::new(SafeKind::StoreCorrupt));
        }
        write_snapshot(&self.paths.index, snapshot)
    }

    fn sweep_orphans(&self, snapshot: &Snapshot) -> Result<(), WorkbenchError> {
        let kept = retained_assets(snapshot);
        if let Ok(entries) = fs::read_dir(&self.paths.assets) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let id = name.split('.').next().unwrap_or("");
                if !kept.contains(id) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        Ok(())
    }

    fn remove_asset_file(&self, asset: &AssetMeta) -> Result<(), WorkbenchError> {
        let format = match asset.format.as_str() {
            "png" => ImageFormat::Png,
            "jpg" | "jpeg" => ImageFormat::Jpeg,
            "webp" => ImageFormat::Webp,
            _ => return Ok(()),
        };
        let _ = fs::remove_file(self.paths.asset_file(&asset.id, format));
        Ok(())
    }
}

impl StoreState {
    fn clone_ready(&self) -> Option<Snapshot> {
        match self {
            Self::Ready(snapshot) => Some(snapshot.clone()),
            Self::Corrupt | Self::Incompatible => None,
        }
    }
}

fn empty_snapshot() -> Snapshot {
    Snapshot {
        version: SCHEMA_VERSION,
        min_reader_version: MIN_READER_VERSION,
        selected_conversation_id: None,
        conversations: Vec::new(),
        messages: Vec::new(),
        jobs: Vec::new(),
        assets: Vec::new(),
        extra: extra_map(),
    }
}

enum Load {
    Ready(Snapshot),
    Incompatible,
    Corrupt,
}

fn load_snapshot(bytes: &[u8]) -> Load {
    let raw: Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(_) => return Load::Corrupt,
    };
    let version = raw.get("version").and_then(Value::as_u64).unwrap_or(0) as u32;
    let min_reader = raw
        .get("min_reader_version")
        .and_then(Value::as_u64)
        .unwrap_or(MIN_READER_VERSION as u64) as u32;
    if min_reader > SCHEMA_VERSION {
        return Load::Incompatible;
    }
    let mut snapshot: Snapshot = match serde_json::from_value(raw) {
        Ok(snapshot) => snapshot,
        Err(_) => return Load::Corrupt,
    };
    snapshot.version = version;
    snapshot.min_reader_version = if min_reader == 0 {
        MIN_READER_VERSION
    } else {
        min_reader
    };
    match upgrade_snapshot(snapshot) {
        Ok(snapshot) => Load::Ready(snapshot),
        Err(_) => Load::Corrupt,
    }
}

fn upgrade_snapshot(mut snapshot: Snapshot) -> Result<Snapshot, WorkbenchError> {
    if snapshot.version == 0 {
        snapshot.version = 1;
    }
    if snapshot.min_reader_version == 0 {
        snapshot.min_reader_version = MIN_READER_VERSION;
    }
    if snapshot.version < SCHEMA_VERSION {
        return Err(WorkbenchError::new(SafeKind::StoreCorrupt));
    }
    Ok(snapshot)
}

fn write_snapshot(path: &Path, snapshot: &Snapshot) -> Result<(), WorkbenchError> {
    let encoded = serde_json::to_vec_pretty(snapshot)
        .map_err(|_| WorkbenchError::new(SafeKind::StoreCorrupt))?;
    write_atomic(path, &encoded)
}

fn interrupt_running(snapshot: &mut Snapshot) {
    for message in &mut snapshot.messages {
        if message.phase.as_deref() == Some("running")
            || message.phase.as_deref() == Some("requesting")
            || message.phase.as_deref() == Some("thinking")
            || message.phase.as_deref() == Some("answering")
            || message.phase.as_deref() == Some("imaging")
        {
            message.phase = Some("interrupted".to_owned());
            message.error_kind = Some(SafeKind::Interrupted.code().to_owned());
        }
    }
    for job in &mut snapshot.jobs {
        if job.status == "running" {
            job.status = "interrupted".to_owned();
            job.error_kind = Some(SafeKind::Interrupted.code().to_owned());
        }
    }
}

fn referenced_assets(snapshot: &Snapshot) -> HashSet<String> {
    let mut ids = HashSet::new();
    for message in &snapshot.messages {
        if let Some(id) = &message.reference_asset_id {
            ids.insert(id.clone());
        }
    }
    for job in &snapshot.jobs {
        if let Some(id) = &job.output_asset_id {
            ids.insert(id.clone());
        }
    }
    ids
}

fn retained_assets(snapshot: &Snapshot) -> HashSet<String> {
    let mut ids = referenced_assets(snapshot);
    for asset in &snapshot.assets {
        ids.insert(asset.id.clone());
    }
    ids
}

fn prepare_dirs(paths: &WorkbenchPaths) -> Result<(), WorkbenchError> {
    fs::create_dir_all(&paths.root).map_err(|_| WorkbenchError::new(SafeKind::StoreCorrupt))?;
    fs::create_dir_all(&paths.assets).map_err(|_| WorkbenchError::new(SafeKind::StoreCorrupt))?;
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(&paths.root, fs::Permissions::from_mode(0o700));
        let _ = fs::set_permissions(&paths.assets, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn write_atomic(path: &Path, content: &[u8]) -> Result<(), WorkbenchError> {
    let parent = path
        .parent()
        .ok_or_else(|| WorkbenchError::new(SafeKind::StoreCorrupt))?;
    fs::create_dir_all(parent).map_err(|_| WorkbenchError::new(SafeKind::StoreCorrupt))?;
    let temporary = parent.join(format!(".workbench-{}.tmp", Uuid::new_v4()));
    let result = (|| -> io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary)?;
        file.write_all(content)?;
        file.sync_all()?;
        #[cfg(unix)]
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            OpenOptions::new().read(true).open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|_| WorkbenchError::new(SafeKind::StoreCorrupt))
}

pub fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4())
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> WorkbenchStore {
        let dir = std::env::temp_dir().join(format!("wb-{}", Uuid::new_v4()));
        WorkbenchStore::open(WorkbenchPaths::from_data_dir(dir)).expect("store")
    }

    fn message(
        id: &str,
        conversation_id: &str,
        reference: Option<&str>,
        phase: Option<&str>,
        job_ids: &[&str],
    ) -> Message {
        Message {
            id: id.into(),
            conversation_id: conversation_id.into(),
            role: "user".into(),
            visible_text: "hi".into(),
            created_at: now_rfc3339(),
            reference_asset_id: reference.map(str::to_owned),
            phase: phase.map(str::to_owned),
            error_kind: None,
            job_ids: job_ids.iter().map(|item| (*item).to_owned()).collect(),
            extra: extra_map(),
        }
    }

    #[test]
    fn future_breaking_schema_is_incompatible_and_rebuilds() {
        let dir = std::env::temp_dir().join(format!("wb-bad-{}", Uuid::new_v4()));
        let paths = WorkbenchPaths::from_data_dir(&dir);
        prepare_dirs(&paths).unwrap();
        fs::write(
            &paths.index,
            br#"{"version":99,"min_reader_version":99,"selected_conversation_id":null,"conversations":[],"messages":[],"jobs":[],"assets":[]}"#,
        )
        .unwrap();
        let store = WorkbenchStore::open(paths).unwrap();
        assert!(store.is_incompatible());
        assert!(!store.is_corrupt());
        let snapshot = store.rebuild().unwrap();
        assert_eq!(snapshot.version, 1);
        assert_eq!(snapshot.min_reader_version, 1);
    }

    #[test]
    fn additive_future_schema_loads_and_preserves_unknown_fields() {
        let dir = std::env::temp_dir().join(format!("wb-fwd-{}", Uuid::new_v4()));
        let paths = WorkbenchPaths::from_data_dir(&dir);
        prepare_dirs(&paths).unwrap();
        fs::write(
            &paths.index,
            br#"{"version":2,"min_reader_version":1,"selected_conversation_id":null,"conversations":[],"messages":[],"jobs":[],"assets":[],"theme":"dark"}"#,
        )
        .unwrap();
        let store = WorkbenchStore::open(paths.clone()).unwrap();
        let loaded = store.snapshot().unwrap();
        assert_eq!(loaded.version, 2);
        assert_eq!(
            loaded.extra.get("theme").and_then(Value::as_str),
            Some("dark")
        );
        store
            .create_conversation(None, None, "1024x1024".into())
            .unwrap();
        let written: Value = serde_json::from_slice(&fs::read(&paths.index).unwrap()).unwrap();
        assert_eq!(written["version"], 2);
        assert_eq!(written["theme"], "dark");
        assert_eq!(written["conversations"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn missing_index_does_not_delete_existing_assets() {
        let dir = std::env::temp_dir().join(format!("wb-miss-{}", Uuid::new_v4()));
        let paths = WorkbenchPaths::from_data_dir(&dir);
        prepare_dirs(&paths).unwrap();
        let leftover = paths.asset_file(&"a".repeat(64), ImageFormat::Png);
        fs::write(&leftover, b"png").unwrap();
        let store = WorkbenchStore::open(paths).unwrap();
        assert!(store.snapshot().unwrap().conversations.is_empty());
        assert!(leftover.exists());
    }

    #[test]
    fn reuses_duplicate_assets_and_cleans_deleted_refs() {
        let store = temp_store();
        let png = include_bytes!("../../../../internal/proxy/testdata/generation_binary.png");
        let first = store.import_image(png, "upload").unwrap();
        let second = store.import_image(png, "import").unwrap();
        assert_eq!(first.id, second.id);
        let conversation = store
            .create_conversation(None, None, "1024x1024".into())
            .unwrap();
        store
            .apply(|snapshot| {
                snapshot
                    .messages
                    .push(message("m1", &conversation.id, Some(&first.id), None, &[]));
            })
            .unwrap();
        store.delete_conversation(&conversation.id).unwrap();
        assert!(store.snapshot().unwrap().assets.is_empty());
    }

    #[test]
    fn reopen_keeps_indexed_assets_without_message_refs() {
        let dir = std::env::temp_dir().join(format!("wb-keep-{}", Uuid::new_v4()));
        let png = include_bytes!("../../../../internal/proxy/testdata/generation_binary.png");
        let first = WorkbenchStore::open(WorkbenchPaths::from_data_dir(&dir)).unwrap();
        let asset = first.import_image(png, "upload").unwrap();
        drop(first);
        let reopened = WorkbenchStore::open(WorkbenchPaths::from_data_dir(&dir)).unwrap();
        assert!(reopened
            .snapshot()
            .unwrap()
            .assets
            .iter()
            .any(|item| item.id == asset.id));
        let bytes = reopened.read_asset(&asset.id).unwrap();
        assert_eq!(bytes.1, png);
    }

    #[test]
    fn incompatible_index_does_not_sweep_assets() {
        let dir = std::env::temp_dir().join(format!("wb-keep-inc-{}", Uuid::new_v4()));
        let paths = WorkbenchPaths::from_data_dir(&dir);
        prepare_dirs(&paths).unwrap();
        let leftover = paths.asset_file(&"b".repeat(64), ImageFormat::Png);
        fs::write(&leftover, b"png").unwrap();
        fs::write(
            &paths.index,
            br#"{"version":99,"min_reader_version":99,"selected_conversation_id":null,"conversations":[],"messages":[],"jobs":[],"assets":[]}"#,
        )
        .unwrap();
        let store = WorkbenchStore::open(paths).unwrap();
        assert!(store.is_incompatible());
        assert!(leftover.exists());
    }

    #[test]
    fn current_files_without_min_reader_still_load() {
        let dir = std::env::temp_dir().join(format!("wb-old-{}", Uuid::new_v4()));
        let paths = WorkbenchPaths::from_data_dir(&dir);
        prepare_dirs(&paths).unwrap();
        fs::write(
            &paths.index,
            br#"{"version":1,"selected_conversation_id":null,"conversations":[],"messages":[],"jobs":[],"assets":[]}"#,
        )
        .unwrap();
        let store = WorkbenchStore::open(paths).unwrap();
        let loaded = store.snapshot().unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.min_reader_version, 1);
    }

    #[test]
    fn leftover_running_becomes_interrupted() {
        let dir = std::env::temp_dir().join(format!("wb-run-{}", Uuid::new_v4()));
        let paths = WorkbenchPaths::from_data_dir(&dir);
        prepare_dirs(&paths).unwrap();
        let snapshot = Snapshot {
            version: 1,
            min_reader_version: 1,
            selected_conversation_id: None,
            conversations: vec![],
            messages: vec![message("m", "c", None, Some("imaging"), &["j"])],
            jobs: vec![ImageJob {
                id: "j".into(),
                message_id: "m".into(),
                action: "generate".into(),
                prompt: "p".into(),
                size: "1024x1024".into(),
                status: "running".into(),
                output_asset_id: None,
                error_kind: None,
                extra: extra_map(),
            }],
            assets: vec![],
            extra: extra_map(),
        };
        write_atomic(&paths.index, &serde_json::to_vec(&snapshot).unwrap()).unwrap();
        let store = WorkbenchStore::open(paths).unwrap();
        let loaded = store.snapshot().unwrap();
        assert_eq!(loaded.jobs[0].status, "interrupted");
        assert_eq!(loaded.messages[0].phase.as_deref(), Some("interrupted"));
    }

    #[test]
    fn delete_does_not_drop_assets_still_referenced_elsewhere() {
        let store = temp_store();
        let png = include_bytes!("../../../../internal/proxy/testdata/generation_binary.png");
        let asset = store.import_image(png, "upload").unwrap();
        let first = store
            .create_conversation(None, None, "1024x1024".into())
            .unwrap();
        let second = store
            .create_conversation(None, None, "1024x1024".into())
            .unwrap();
        store
            .apply(|snapshot| {
                snapshot
                    .messages
                    .push(message("keep", &first.id, Some(&asset.id), None, &[]));
                snapshot
                    .messages
                    .push(message("drop", &second.id, Some(&asset.id), None, &[]));
            })
            .unwrap();
        store.delete_conversation(&second.id).unwrap();
        assert!(store
            .snapshot()
            .unwrap()
            .assets
            .iter()
            .any(|item| item.id == asset.id));
        store.delete_conversation(&first.id).unwrap();
        assert!(store.snapshot().unwrap().assets.is_empty());
    }

    #[test]
    fn oversized_import_does_not_write_bytes() {
        let store = temp_store();
        let huge = vec![0u8; crate::image_workbench::image::MAX_IMAGE_BYTES + 1];
        assert!(store.import_image(&huge, "import").is_err());
        assert!(store.snapshot().unwrap().assets.is_empty());
    }

    #[test]
    fn atomic_write_failure_leaves_no_index() {
        let dir = std::env::temp_dir().join(format!("wb-atomic-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("image-workbench");
        fs::write(&blocker, b"not-a-directory").unwrap();
        let paths = WorkbenchPaths::from_data_dir(&dir);
        assert!(write_atomic(&paths.index, b"{\"version\":1}").is_err());
        assert!(!paths.index.exists());
    }
}
