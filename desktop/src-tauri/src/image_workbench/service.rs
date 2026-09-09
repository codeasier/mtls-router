use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use serde::Serialize;
use serde_json::Value;

use super::catalog::{grouped_image_choices, select_chat_model, select_image_model, valid_size};
use super::channel::{
    decode_generation, generation_body, BoundSession, ChannelHooks, TrustTarget, BODY_BUDGET,
    CATALOG_LIMIT, GENERATION_LIMIT,
};
use super::error::{SafeKind, WorkbenchError};
use super::image::validate_image;
use super::orchestrate::{
    bounded_text, decide_jobs, extract_image_payload, fallback_edit_text, rebuild_chat_messages,
    HistoryTurn, RebuildInput,
};
use super::store::{
    new_id, AssetMeta, Conversation, ImageJob, Message, Snapshot, WorkbenchPaths, WorkbenchStore,
};

pub trait EventSink: Send + Sync {
    fn emit(&self, event: &str, payload: Value);
}

pub trait FilePicker: Send + Sync {
    fn pick(&self) -> Option<PathBuf>;
    fn save(&self, name: &str) -> Option<PathBuf>;
}

pub struct NativePicker;

impl FilePicker for NativePicker {
    fn pick(&self) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
            .pick_file()
    }

    fn save(&self, name: &str) -> Option<PathBuf> {
        rfd::FileDialog::new().set_file_name(name).save_file()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Readiness {
    pub ready: bool,
    pub has_credential: bool,
    pub router_trusted: bool,
    pub health_ok: bool,
    pub store_corrupt: bool,
    pub store_incompatible: bool,
    pub chat_models: Vec<String>,
    pub image_models: Vec<super::catalog::ImageChoice>,
    pub can_submit: bool,
    pub reason: Option<String>,
    pub busy: bool,
}

#[derive(Clone)]
struct CatalogCache {
    chat: Vec<String>,
    image: Vec<String>,
}

struct Operation {
    id: String,
    conversation_id: String,
    cancel: Arc<AtomicBool>,
}

pub struct Workbench {
    store: WorkbenchStore,
    hooks: ChannelHooks,
    picker: Arc<dyn FilePicker>,
    catalogs: Mutex<Option<CatalogCache>>,
    operation: Mutex<Option<Operation>>,
    language: Mutex<String>,
}

impl Workbench {
    pub fn open(paths: WorkbenchPaths) -> Result<Self, WorkbenchError> {
        Ok(Self {
            store: WorkbenchStore::open(paths)?,
            hooks: ChannelHooks::default(),
            picker: Arc::new(NativePicker),
            catalogs: Mutex::new(None),
            operation: Mutex::new(None),
            language: Mutex::new("zh-CN".into()),
        })
    }

    pub fn with_hooks(mut self, hooks: ChannelHooks) -> Self {
        self.hooks = hooks;
        self
    }

    pub fn store(&self) -> &WorkbenchStore {
        &self.store
    }

    pub fn set_language(&self, language: &str) {
        if let Ok(mut slot) = self.language.lock() {
            *slot = language.to_owned();
        }
    }

    pub fn english(&self) -> bool {
        self.language
            .lock()
            .map(|value| value.as_str() == "en")
            .unwrap_or(false)
    }

    pub fn invalidate_catalogs(&self) {
        if let Ok(mut slot) = self.catalogs.lock() {
            *slot = None;
        }
    }

    pub fn readiness_snapshot(&self, has_credential: bool, router_trusted: bool) -> Readiness {
        let store_corrupt = self.store.is_corrupt();
        let store_incompatible = self.store.is_incompatible();
        let store_blocked = store_corrupt || store_incompatible;
        let catalogs = self.catalogs.lock().ok().and_then(|slot| slot.clone());
        let health_ok = catalogs.is_some();
        let chat = catalogs
            .as_ref()
            .map(|item| item.chat.clone())
            .unwrap_or_default();
        let image = catalogs
            .as_ref()
            .map(|item| item.image.clone())
            .unwrap_or_default();
        let snapshot = self.store.snapshot().ok();
        let selected = snapshot.as_ref().and_then(|item| {
            item.selected_conversation_id
                .as_ref()
                .and_then(|id| item.conversations.iter().find(|c| &c.id == id))
        });
        let chat_ok = selected
            .and_then(|item| item.selected_chat_model.as_deref())
            .map(|id| chat.iter().any(|item| item == id))
            .unwrap_or(false);
        let image_ok = selected
            .and_then(|item| item.selected_image_model.as_deref())
            .map(|id| image.iter().any(|item| item == id))
            .unwrap_or(false);
        let busy = self
            .operation
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(|_| true))
            .unwrap_or(false);
        let ready =
            has_credential && router_trusted && health_ok && !store_blocked && chat_ok && image_ok;
        Readiness {
            ready,
            has_credential,
            router_trusted,
            health_ok,
            store_corrupt,
            store_incompatible,
            chat_models: chat.clone(),
            image_models: grouped_image_choices(
                &image,
                selected.and_then(|item| item.selected_image_model.as_deref()),
            ),
            can_submit: ready && !busy,
            reason: if store_incompatible {
                Some(SafeKind::StoreIncompatible.code().into())
            } else if store_corrupt {
                Some(SafeKind::StoreCorrupt.code().into())
            } else if !has_credential {
                Some("WORKBENCH_NO_CREDENTIAL".into())
            } else if !router_trusted {
                Some("WORKBENCH_ROUTER".into())
            } else if !health_ok {
                Some("WORKBENCH_CATALOG".into())
            } else if !chat_ok || !image_ok {
                Some("WORKBENCH_MODEL_MISSING".into())
            } else if busy {
                Some(SafeKind::Busy.code().into())
            } else {
                None
            },
            busy,
        }
    }

    pub fn refresh_catalogs(
        &self,
        target: &TrustTarget,
        api_key: &str,
    ) -> Result<Readiness, WorkbenchError> {
        let cancel = Arc::new(AtomicBool::new(false));
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut session = BoundSession::connect(target, &self.hooks, deadline, cancel)?;
        let authorization = format!("Bearer {api_key}");
        let chat = session.get("/v1/models", &authorization, deadline, CATALOG_LIMIT)?;
        if chat.status != 200 {
            return Err(WorkbenchError::new(SafeKind::NotReady));
        }
        let chat_ids = super::catalog::parse_chat_models(&chat.body)?;
        if chat.connection_close {
            return Err(WorkbenchError::new(SafeKind::Redial));
        }
        let image = session.get("/v1/models/image", &authorization, deadline, CATALOG_LIMIT)?;
        if image.status != 200 {
            return Err(WorkbenchError::new(SafeKind::NotReady));
        }
        let image_ids = super::catalog::parse_image_models(&image.body)?;
        *self
            .catalogs
            .lock()
            .map_err(|_| WorkbenchError::new(SafeKind::Unknown))? = Some(CatalogCache {
            chat: chat_ids,
            image: image_ids,
        });
        Ok(self.readiness_snapshot(true, true))
    }

    pub fn pick_reference(&self) -> Result<AssetMeta, WorkbenchError> {
        let path = self
            .picker
            .pick()
            .ok_or_else(|| WorkbenchError::new(SafeKind::Cancelled))?;
        let bytes = std::fs::read(path).map_err(|_| WorkbenchError::new(SafeKind::ImageInvalid))?;
        self.store.import_image(&bytes, "upload")
    }

    pub fn import_bytes(&self, bytes: &[u8]) -> Result<AssetMeta, WorkbenchError> {
        self.store.import_image(bytes, "import")
    }

    pub fn save_asset(&self, id: &str) -> Result<(), WorkbenchError> {
        let (meta, bytes) = self.store.read_asset(id)?;
        let name = format!("{}.{}", &meta.id[..8], meta.format);
        let path = self
            .picker
            .save(&name)
            .ok_or_else(|| WorkbenchError::new(SafeKind::Cancelled))?;
        std::fs::write(path, bytes).map_err(|_| WorkbenchError::new(SafeKind::Unknown))
    }

    pub fn cancel(&self) {
        if let Ok(slot) = self.operation.lock() {
            if let Some(operation) = slot.as_ref() {
                operation.cancel.store(true, Ordering::SeqCst);
            }
        }
    }

    pub fn interrupt_on_exit(&self) {
        self.cancel();
        let _ = self.store.apply(|snapshot| {
            for message in &mut snapshot.messages {
                if message
                    .phase
                    .as_deref()
                    .is_some_and(|phase| !matches!(phase, "interrupted" | ""))
                    && message.error_kind.is_none()
                    && snapshot
                        .jobs
                        .iter()
                        .any(|job| message.job_ids.contains(&job.id) && job.status == "running")
                {
                    message.phase = Some("interrupted".into());
                    message.error_kind = Some(SafeKind::Interrupted.code().into());
                }
            }
            for job in &mut snapshot.jobs {
                if job.status == "running" {
                    job.status = "interrupted".into();
                    job.error_kind = Some(SafeKind::Interrupted.code().into());
                }
            }
        });
    }

    pub fn send(
        &self,
        conversation_id: &str,
        mut text: String,
        force_image: bool,
        reference_asset_id: Option<String>,
        target: &TrustTarget,
        api_key: &str,
        sink: &dyn EventSink,
    ) -> Result<Snapshot, WorkbenchError> {
        let operation_id = new_id("op");
        let cancel = self.begin_operation(&operation_id, conversation_id)?;
        let result = self.send_inner(
            &operation_id,
            conversation_id,
            &mut text,
            force_image,
            reference_asset_id,
            target,
            api_key,
            sink,
            cancel,
        );
        self.end_operation();
        result
    }

    pub fn regenerate(
        &self,
        conversation_id: &str,
        job_id: &str,
        reference_asset_id: Option<String>,
        target: &TrustTarget,
        api_key: &str,
        sink: &dyn EventSink,
    ) -> Result<Snapshot, WorkbenchError> {
        let operation_id = new_id("op");
        let cancel = self.begin_operation(&operation_id, conversation_id)?;
        let result = self.regenerate_inner(
            &operation_id,
            conversation_id,
            job_id,
            reference_asset_id,
            target,
            api_key,
            sink,
            cancel,
        );
        self.end_operation();
        result
    }

    fn send_inner(
        &self,
        operation_id: &str,
        conversation_id: &str,
        text: &mut String,
        force_image: bool,
        reference_asset_id: Option<String>,
        target: &TrustTarget,
        api_key: &str,
        sink: &dyn EventSink,
        cancel: Arc<AtomicBool>,
    ) -> Result<Snapshot, WorkbenchError> {
        *text = text.trim().to_owned();
        if text.is_empty() {
            if reference_asset_id.is_some() {
                *text = fallback_edit_text(self.english()).to_owned();
            } else {
                return Err(WorkbenchError::new(SafeKind::InputEmpty));
            }
        }
        bounded_text(text)?;
        let snapshot = self.store.snapshot()?;
        let conversation = snapshot
            .conversations
            .iter()
            .find(|item| item.id == conversation_id)
            .cloned()
            .ok_or_else(|| WorkbenchError::new(SafeKind::Unknown))?;
        let catalogs = self
            .catalogs
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
            .ok_or_else(|| WorkbenchError::new(SafeKind::NotReady))?;
        let (chat_model, chat_ok) =
            select_chat_model(conversation.selected_chat_model.as_deref(), &catalogs.chat);
        let (image_model, image_ok) = select_image_model(
            conversation.selected_image_model.as_deref(),
            &catalogs.image,
        );
        if !chat_ok || !image_ok {
            return Err(WorkbenchError::new(SafeKind::NotReady));
        }
        let chat_model = chat_model.ok_or_else(|| WorkbenchError::new(SafeKind::NotReady))?;
        let image_model = image_model.ok_or_else(|| WorkbenchError::new(SafeKind::NotReady))?;
        if !valid_size(&conversation.selected_size) {
            return Err(WorkbenchError::new(SafeKind::NotReady));
        }
        let user_id = new_id("m");
        let assistant_id = new_id("m");
        let first = conversation.message_ids.is_empty();
        self.store.apply(|snapshot| {
            if let Some(item) = snapshot
                .conversations
                .iter_mut()
                .find(|item| item.id == conversation_id)
            {
                if first {
                    item.title = WorkbenchStore::title_for_first_user(text);
                }
                item.message_ids.push(user_id.clone());
                item.message_ids.push(assistant_id.clone());
                item.updated_at = chrono::Utc::now().to_rfc3339();
            }
            snapshot.messages.push(Message {
                id: user_id.clone(),
                conversation_id: conversation_id.to_owned(),
                role: "user".into(),
                visible_text: text.clone(),
                created_at: chrono::Utc::now().to_rfc3339(),
                reference_asset_id: reference_asset_id.clone(),
                phase: None,
                error_kind: None,
                job_ids: vec![],
                extra: serde_json::Map::new(),
            });
            snapshot.messages.push(Message {
                id: assistant_id.clone(),
                conversation_id: conversation_id.to_owned(),
                role: "assistant".into(),
                visible_text: String::new(),
                created_at: chrono::Utc::now().to_rfc3339(),
                reference_asset_id: None,
                phase: Some("requesting".into()),
                error_kind: None,
                job_ids: vec![],
                extra: serde_json::Map::new(),
            });
        })?;
        emit_phase(
            sink,
            operation_id,
            conversation_id,
            &assistant_id,
            "requesting",
        );
        let history = history_turns(&self.store.snapshot()?, conversation_id);
        let last_prompt = last_success_prompt(&self.store.snapshot()?, conversation_id);
        let messages = rebuild_chat_messages(RebuildInput {
            history: &history,
            last_image_prompt: last_prompt.as_deref(),
            force_image,
            explicit_reference: reference_asset_id.is_some(),
            size: &conversation.selected_size,
        })?;
        let body = serde_json::json!({
            "model": chat_model,
            "messages": messages,
            "stream": true
        });
        let body = serde_json::to_vec(&body).unwrap_or_default();
        let authorization = format!("Bearer {api_key}");
        let deadline = Instant::now() + BODY_BUDGET;
        let mut session = BoundSession::connect(target, &self.hooks, deadline, cancel.clone())?;
        emit_phase(
            sink,
            operation_id,
            conversation_id,
            &assistant_id,
            "thinking",
        );
        let raw = match session.post_sse(
            "/v1/chat/completions",
            &authorization,
            &body,
            deadline,
            |partial| {
                let visible = extract_image_payload(partial, &conversation.selected_size).display;
                sink.emit(
                    super::ipc::CHAT_DELTA_EVENT,
                    serde_json::json!({
                        "operation_id": operation_id,
                        "conversation_id": conversation_id,
                        "message_id": assistant_id,
                        "visible_text": visible,
                    }),
                );
            },
        ) {
            Ok(raw) => raw,
            Err(error) => {
                self.fail_assistant(
                    &assistant_id,
                    error.kind,
                    sink,
                    operation_id,
                    conversation_id,
                )?;
                return Err(error);
            }
        };
        let extracted = extract_image_payload(&raw, &conversation.selected_size);
        let last_image = last_success_asset(&self.store.snapshot()?, conversation_id);
        let jobs = match decide_jobs(
            &extracted.specs,
            text,
            force_image,
            reference_asset_id.as_deref(),
            last_image.as_deref(),
            &conversation.selected_size,
            self.english(),
        ) {
            Ok(jobs) => jobs,
            Err(error) => {
                self.fail_assistant(
                    &assistant_id,
                    error.kind,
                    sink,
                    operation_id,
                    conversation_id,
                )?;
                return Err(error);
            }
        };
        let mut job_ids = Vec::new();
        self.store.apply(|snapshot| {
            if let Some(message) = snapshot
                .messages
                .iter_mut()
                .find(|item| item.id == assistant_id)
            {
                message.visible_text = extracted.display.clone();
                message.phase = if jobs.is_empty() {
                    None
                } else {
                    Some("imaging".into())
                };
            }
            for decision in &jobs {
                let job = ImageJob {
                    id: new_id("j"),
                    message_id: assistant_id.clone(),
                    action: decision.action.clone(),
                    prompt: decision.prompt.clone(),
                    size: decision.size.clone(),
                    status: "running".into(),
                    output_asset_id: None,
                    error_kind: None,
                    extra: serde_json::Map::new(),
                };
                job_ids.push(job.id.clone());
                if let Some(message) = snapshot
                    .messages
                    .iter_mut()
                    .find(|item| item.id == assistant_id)
                {
                    message.job_ids.push(job.id.clone());
                }
                snapshot.jobs.push(job);
            }
        })?;
        if !jobs.is_empty() {
            emit_phase(
                sink,
                operation_id,
                conversation_id,
                &assistant_id,
                "imaging",
            );
        }
        for (decision, job_id) in jobs.iter().zip(job_ids.iter()) {
            if cancel.load(Ordering::SeqCst) {
                self.finish_job(
                    job_id,
                    "cancelled",
                    None,
                    Some(SafeKind::Cancelled),
                    sink,
                    operation_id,
                    conversation_id,
                    &assistant_id,
                )?;
                continue;
            }
            if decision.action == "edit" && decision.reference_asset_id.is_none() {
                self.finish_job(
                    job_id,
                    "failed",
                    None,
                    Some(SafeKind::NoReference),
                    sink,
                    operation_id,
                    conversation_id,
                    &assistant_id,
                )?;
                continue;
            }
            let data_uri = if decision.action == "edit" {
                let id = decision.reference_asset_id.as_ref().unwrap();
                let (meta, bytes) = self.store.read_asset(id)?;
                Some(data_uri(&meta.format, &bytes))
            } else {
                None
            };
            let gen_body = generation_body(
                &image_model,
                &decision.prompt,
                &decision.size,
                data_uri.as_deref(),
            );
            let deadline = Instant::now() + BODY_BUDGET;
            let outcome = (|| {
                let mut session =
                    BoundSession::connect(target, &self.hooks, deadline, cancel.clone())?;
                let response = session.post(
                    "/v1/images/generations?response_format=binary",
                    &authorization,
                    &gen_body,
                    deadline,
                    GENERATION_LIMIT,
                )?;
                if response.status != 200 {
                    return Err(WorkbenchError::new(SafeKind::ImageFailed));
                }
                let bytes = decode_generation(&response)?;
                let validated = validate_image(&bytes)?;
                self.store.persist_asset(validated, "generate")
            })();
            match outcome {
                Ok(asset) => {
                    self.finish_job(
                        job_id,
                        "succeeded",
                        Some(asset.id),
                        None,
                        sink,
                        operation_id,
                        conversation_id,
                        &assistant_id,
                    )?;
                }
                Err(error) => {
                    let kind = if cancel.load(Ordering::SeqCst) {
                        SafeKind::Cancelled
                    } else {
                        error.kind
                    };
                    let status = if kind == SafeKind::Cancelled {
                        "cancelled"
                    } else {
                        "failed"
                    };
                    self.finish_job(
                        job_id,
                        status,
                        None,
                        Some(kind),
                        sink,
                        operation_id,
                        conversation_id,
                        &assistant_id,
                    )?;
                }
            }
        }
        let snapshot = self.store.apply(|snapshot| {
            if let Some(message) = snapshot
                .messages
                .iter_mut()
                .find(|item| item.id == assistant_id)
            {
                message.phase = None;
            }
        })?;
        sink.emit(
            super::ipc::OPERATION_DONE_EVENT,
            serde_json::json!({
                "operation_id": operation_id,
                "conversation_id": conversation_id,
            }),
        );
        Ok(snapshot)
    }

    fn regenerate_inner(
        &self,
        operation_id: &str,
        conversation_id: &str,
        job_id: &str,
        reference_asset_id: Option<String>,
        target: &TrustTarget,
        api_key: &str,
        sink: &dyn EventSink,
        cancel: Arc<AtomicBool>,
    ) -> Result<Snapshot, WorkbenchError> {
        let snapshot = self.store.snapshot()?;
        let job = snapshot
            .jobs
            .iter()
            .find(|item| item.id == job_id)
            .cloned()
            .ok_or_else(|| WorkbenchError::new(SafeKind::Unknown))?;
        let conversation = snapshot
            .conversations
            .iter()
            .find(|item| item.id == conversation_id)
            .cloned()
            .ok_or_else(|| WorkbenchError::new(SafeKind::Unknown))?;
        let image_model = conversation
            .selected_image_model
            .ok_or_else(|| WorkbenchError::new(SafeKind::NotReady))?;
        let last_image = last_success_asset(&snapshot, conversation_id);
        let reference = if job.action == "edit" {
            reference_asset_id.or(last_image)
        } else {
            None
        };
        if job.action == "edit" && reference.is_none() {
            return Err(WorkbenchError::new(SafeKind::NoReference));
        }
        self.store.apply(|snapshot| {
            if let Some(item) = snapshot.jobs.iter_mut().find(|item| item.id == job_id) {
                item.status = "running".into();
                item.error_kind = None;
                item.output_asset_id = None;
            }
        })?;
        emit_phase(
            sink,
            operation_id,
            conversation_id,
            &job.message_id,
            "imaging",
        );
        let data_uri = if let Some(id) = &reference {
            let (meta, bytes) = self.store.read_asset(id)?;
            Some(data_uri(&meta.format, &bytes))
        } else {
            None
        };
        let body = generation_body(&image_model, &job.prompt, &job.size, data_uri.as_deref());
        let authorization = format!("Bearer {api_key}");
        let deadline = Instant::now() + BODY_BUDGET;
        let outcome = (|| {
            let mut session = BoundSession::connect(target, &self.hooks, deadline, cancel.clone())?;
            let response = session.post(
                "/v1/images/generations?response_format=binary",
                &authorization,
                &body,
                deadline,
                GENERATION_LIMIT,
            )?;
            if response.status != 200 {
                return Err(WorkbenchError::new(SafeKind::ImageFailed));
            }
            let bytes = decode_generation(&response)?;
            let validated = validate_image(&bytes)?;
            self.store.persist_asset(validated, "generate")
        })();
        match outcome {
            Ok(asset) => {
                self.finish_job(
                    job_id,
                    "succeeded",
                    Some(asset.id),
                    None,
                    sink,
                    operation_id,
                    conversation_id,
                    &job.message_id,
                )?;
            }
            Err(error) => {
                self.finish_job(
                    job_id,
                    "failed",
                    None,
                    Some(error.kind),
                    sink,
                    operation_id,
                    conversation_id,
                    &job.message_id,
                )?;
                return Err(error);
            }
        }
        let snapshot = self.store.snapshot()?;
        sink.emit(
            super::ipc::OPERATION_DONE_EVENT,
            serde_json::json!({
                "operation_id": operation_id,
                "conversation_id": conversation_id,
            }),
        );
        Ok(snapshot)
    }

    fn begin_operation(
        &self,
        id: &str,
        conversation_id: &str,
    ) -> Result<Arc<AtomicBool>, WorkbenchError> {
        let mut slot = self
            .operation
            .lock()
            .map_err(|_| WorkbenchError::new(SafeKind::Unknown))?;
        if slot.is_some() {
            return Err(WorkbenchError::new(SafeKind::Busy));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *slot = Some(Operation {
            id: id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            cancel: cancel.clone(),
        });
        Ok(cancel)
    }

    fn end_operation(&self) {
        if let Ok(mut slot) = self.operation.lock() {
            *slot = None;
        }
    }

    fn fail_assistant(
        &self,
        assistant_id: &str,
        kind: SafeKind,
        sink: &dyn EventSink,
        operation_id: &str,
        conversation_id: &str,
    ) -> Result<(), WorkbenchError> {
        self.store.apply(|snapshot| {
            if let Some(message) = snapshot
                .messages
                .iter_mut()
                .find(|item| item.id == assistant_id)
            {
                message.phase = None;
                message.error_kind = Some(kind.code().into());
            }
        })?;
        sink.emit(
            super::ipc::OPERATION_DONE_EVENT,
            serde_json::json!({
                "operation_id": operation_id,
                "conversation_id": conversation_id,
            }),
        );
        Ok(())
    }

    fn finish_job(
        &self,
        job_id: &str,
        status: &str,
        asset: Option<String>,
        kind: Option<SafeKind>,
        sink: &dyn EventSink,
        operation_id: &str,
        conversation_id: &str,
        message_id: &str,
    ) -> Result<(), WorkbenchError> {
        self.store.apply(|snapshot| {
            if let Some(job) = snapshot.jobs.iter_mut().find(|item| item.id == job_id) {
                job.status = status.to_owned();
                job.output_asset_id = asset.clone();
                job.error_kind = kind.map(|value| value.code().to_owned());
            }
        })?;
        sink.emit(
            super::ipc::IMAGE_STATUS_EVENT,
            serde_json::json!({
                "operation_id": operation_id,
                "conversation_id": conversation_id,
                "message_id": message_id,
                "job_id": job_id,
                "status": status,
                "output_asset_id": asset,
                "error_kind": kind.map(|value| value.code()),
            }),
        );
        Ok(())
    }
}

fn history_turns(snapshot: &Snapshot, conversation_id: &str) -> Vec<HistoryTurn> {
    snapshot
        .messages
        .iter()
        .filter(|item| item.conversation_id == conversation_id)
        .map(|item| HistoryTurn {
            role: item.role.clone(),
            visible_text: item.visible_text.clone(),
            generated_prompts: snapshot
                .jobs
                .iter()
                .filter(|job| item.job_ids.contains(&job.id) && job.status == "succeeded")
                .map(|job| job.prompt.clone())
                .collect(),
            reference_hint: item.reference_asset_id.as_ref().map(|_| String::new()),
        })
        .collect()
}

fn last_success_asset(snapshot: &Snapshot, conversation_id: &str) -> Option<String> {
    for message in snapshot.messages.iter().rev() {
        if message.conversation_id != conversation_id {
            continue;
        }
        for job_id in message.job_ids.iter().rev() {
            if let Some(job) = snapshot.jobs.iter().find(|item| &item.id == job_id) {
                if job.status == "succeeded" {
                    return job.output_asset_id.clone();
                }
            }
        }
    }
    None
}

fn last_success_prompt(snapshot: &Snapshot, conversation_id: &str) -> Option<String> {
    for message in snapshot.messages.iter().rev() {
        if message.conversation_id != conversation_id {
            continue;
        }
        for job_id in message.job_ids.iter().rev() {
            if let Some(job) = snapshot.jobs.iter().find(|item| &item.id == job_id) {
                if job.status == "succeeded" {
                    return Some(job.prompt.clone());
                }
            }
        }
    }
    None
}

fn data_uri(format: &str, bytes: &[u8]) -> String {
    let mime = match format {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    };
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn emit_phase(
    sink: &dyn EventSink,
    operation_id: &str,
    conversation_id: &str,
    message_id: &str,
    phase: &str,
) {
    sink.emit(
        super::ipc::PHASE_EVENT,
        serde_json::json!({
            "operation_id": operation_id,
            "conversation_id": conversation_id,
            "message_id": message_id,
            "phase": phase,
        }),
    );
}

pub fn conversation_view(snapshot: &Snapshot) -> Vec<Conversation> {
    snapshot.conversations.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: &str, conversation_id: &str, job_ids: &[&str]) -> Message {
        Message {
            id: id.into(),
            conversation_id: conversation_id.into(),
            role: "assistant".into(),
            visible_text: "ok".into(),
            created_at: "2026-09-08T10:00:00Z".into(),
            reference_asset_id: None,
            phase: None,
            error_kind: None,
            job_ids: job_ids.iter().map(|item| (*item).to_owned()).collect(),
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn implicit_edit_reference_does_not_cross_conversations() {
        let snapshot = Snapshot {
            version: 1,
            min_reader_version: 1,
            selected_conversation_id: Some("b".into()),
            conversations: vec![],
            messages: vec![message("m-a", "a", &["j-a"]), message("m-b", "b", &[])],
            jobs: vec![ImageJob {
                id: "j-a".into(),
                message_id: "m-a".into(),
                action: "generate".into(),
                prompt: "other session".into(),
                size: "1024x1024".into(),
                status: "succeeded".into(),
                output_asset_id: Some("asset-a".into()),
                error_kind: None,
                extra: serde_json::Map::new(),
            }],
            assets: vec![],
            extra: serde_json::Map::new(),
        };
        assert_eq!(
            last_success_asset(&snapshot, "a").as_deref(),
            Some("asset-a")
        );
        assert_eq!(last_success_asset(&snapshot, "b"), None);
        let jobs = decide_jobs(
            &[],
            "改成蓝色的天空",
            false,
            None,
            last_success_asset(&snapshot, "b").as_deref(),
            "1024x1024",
            false,
        )
        .unwrap();
        assert!(jobs
            .iter()
            .all(|job| job.action != "edit" && job.reference_asset_id.is_none()));
    }
}
