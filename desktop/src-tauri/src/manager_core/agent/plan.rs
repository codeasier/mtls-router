use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::modelconfig::Agent;
use super::modelconfig::{
    CatalogClaims, CleanupRevisionFile, RevisionAgent, RevisionFile, RevisionState,
};
use super::render::{
    merge_claude, merge_codex, merge_opencode, render_claude_fragment, render_codex_auth_fragment,
    render_codex_fragment, render_opencode_fragment, Fragment, RenderInput,
};
use super::sidecar::LastAppliedState;
use super::types::{Format, OperationError};
use crate::protocol::ErrorCode;

pub const JSONC_MIGRATION_WARNING: &str =
    "opencode.jsonc will be migrated to opencode.json; comments and formatting will not be preserved";
pub const JSONC_OVERRIDE_WARNING: &str =
    "OPENCODE_CONFIG JSONC will be normalized to strict JSON in place; comments and formatting will not be preserved";
pub const BACKUP_WARNING: &str =
    "Backups are sensitive recovery artifacts and may contain a previous API key";
pub const REBUILD_WARNING: &str =
    "Rebuild replaces the complete Agent configuration; unrelated settings, comments, and formatting will not be preserved";

pub const REVISION_CONTEXT_AGENT_FILE: &str = "agent-file";
pub const REVISION_CONTEXT_JOURNAL: &str = "journal-entry";
pub const REVISION_CONTEXT_BACKUP: &str = "backup-verification";
pub const REVISION_CONTEXT_SIDECAR: &str = "manager-state-sidecar";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    #[default]
    Create,
    Replace,
    Delete,
    Preserve,
    Backup,
}

impl Operation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Replace => "replace",
            Self::Delete => "delete",
            Self::Preserve => "preserve",
            Self::Backup => "backup",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigMode {
    Merge,
    Rebuild,
}

impl ConfigMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebuild => "rebuild",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "merge" => Some(Self::Merge),
            "rebuild" => Some(Self::Rebuild),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalScope {
    Agent,
    ManagerState,
}

impl JournalScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::ManagerState => "manager_state",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileRevision {
    pub exists: bool,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub size: i64,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub mode: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub digest: String,
}

impl FileRevision {
    pub fn token_value(&self) -> String {
        if self.exists {
            self.digest.clone()
        } else {
            "absent".into()
        }
    }

    pub fn claim(&self) -> RevisionState {
        RevisionState {
            exists: self.exists,
            size: self.size,
            mode: self.mode,
            digest: self.digest.clone(),
        }
    }
}

fn is_zero_i64(value: &i64) -> bool {
    *value == 0
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

#[derive(Clone, Debug)]
pub enum FileRender {
    ClaudeMerge {
        root: Map<String, Value>,
        obsolete_owned_env: Vec<String>,
    },
    ClaudeFragment,
    OpenCodeMerge {
        root: Map<String, Value>,
        own_root_model: bool,
    },
    OpenCodeFragment,
    CodexMerge {
        content: Vec<u8>,
        obsolete_optional: Vec<String>,
        migrate_historical: bool,
    },
    CodexFragment,
    CodexAuth {
        root: Option<Map<String, Value>>,
    },
    CodexAuthFragment,
    Static(Vec<u8>),
}

impl FileRender {
    pub fn render(&self, input: &RenderInput, key: &str) -> Result<Vec<u8>, OperationError> {
        match self {
            Self::ClaudeMerge {
                root,
                obsolete_owned_env,
            } => merge_claude(
                root,
                input.config.claude.as_ref().ok_or_else(missing_section)?,
                &input.router_base_url,
                key,
                obsolete_owned_env,
            ),
            Self::ClaudeFragment => render_claude_fragment(
                input.config.claude.as_ref().ok_or_else(missing_section)?,
                &input.router_base_url,
                key,
            ),
            Self::OpenCodeMerge {
                root,
                own_root_model,
            } => merge_opencode(
                root,
                input.config.opencode.as_ref().ok_or_else(missing_section)?,
                &input.api_base_url,
                key,
                *own_root_model,
            ),
            Self::OpenCodeFragment => render_opencode_fragment(
                input.config.opencode.as_ref().ok_or_else(missing_section)?,
                &input.api_base_url,
                key,
            ),
            Self::CodexMerge {
                content,
                obsolete_optional,
                migrate_historical,
            } => merge_codex(
                content,
                input.config.codex.as_ref().ok_or_else(missing_section)?,
                &input.api_base_url,
                obsolete_optional,
                *migrate_historical,
            ),
            Self::CodexFragment => render_codex_fragment(
                input.config.codex.as_ref().ok_or_else(missing_section)?,
                &input.api_base_url,
            ),
            Self::CodexAuth { root } => render_codex_auth_fragment(key, root.as_ref()),
            Self::CodexAuthFragment => render_codex_auth_fragment(key, None),
            Self::Static(content) => Ok(content.clone()),
        }
    }
}

fn missing_section() -> OperationError {
    OperationError::new(
        ErrorCode::ModelConfigInvalid,
        "canonical model configuration is invalid",
    )
}

#[derive(Clone, Debug)]
pub struct PlannedFile {
    pub agent: Option<Agent>,
    pub mode: ConfigMode,
    pub format: Format,
    pub source_path: String,
    pub target_path: String,
    pub operation: Operation,
    pub contains_api_key: bool,
    pub preserves: Vec<String>,
    pub warning: String,
    pub source_revision: FileRevision,
    pub target_revision: FileRevision,
    pub source_content: Vec<u8>,
    pub source_mode: u32,
    pub target_mode: u32,
    pub backup_required: bool,
    pub backup_source: String,
    pub backup_path: String,
    pub restore_from: String,
    pub render: Option<FileRender>,
    pub role: String,
    pub companion_exists: bool,
    pub syntax_invalid: bool,
    pub scope: JournalScope,
}

impl PlannedFile {
    pub fn output(&self, input: &RenderInput, key: &str) -> Result<Vec<u8>, OperationError> {
        if self.operation == Operation::Delete {
            return Ok(Vec::new());
        }
        self.render
            .as_ref()
            .ok_or_else(|| {
                OperationError::new(
                    ErrorCode::WriteFailed,
                    "could not render an Agent configuration file",
                )
            })?
            .render(input, key)
    }
}

#[derive(Clone, Debug)]
pub struct PlannedRevisionGuard {
    pub path: String,
    pub revision: FileRevision,
    pub scope: JournalScope,
}

#[derive(Clone, Debug)]
pub struct AgentPlan {
    pub kind: Agent,
    pub mode: ConfigMode,
}

#[derive(Clone, Debug)]
pub struct WritePlan {
    pub agents: Vec<AgentPlan>,
    pub selected: Vec<Agent>,
    pub files: Vec<PlannedFile>,
    pub revision_guards: Vec<PlannedRevisionGuard>,
    pub input: RenderInput,
    pub catalog: CatalogClaims,
    pub canonical: Vec<u8>,
    pub sidecar: LastAppliedState,
    pub sidecar_revision: FileRevision,
    pub sidecar_content: Vec<u8>,
    pub sidecar_mode: u32,
    pub drifted: Vec<Agent>,
    pub collisions: Vec<ManagedCollision>,
    pub requires_codex_auth_approval: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct BackupPlan {
    pub required: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pattern: String,
    pub sensitive: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub warning: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FilePreview {
    pub role: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_path: String,
    pub format: Format,
    pub operation: Operation,
    pub operations: Vec<Operation>,
    pub contains_api_key: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preserves: Vec<String>,
    pub backup: BackupPlan,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub warning: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentPreview {
    pub agent: Agent,
    pub mode: ConfigMode,
    pub name: String,
    pub files: Vec<FilePreview>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ManagedCollision {
    pub agent: Agent,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub action: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preview {
    pub revision_token: String,
    pub agents: Vec<AgentPreview>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub model_config: serde_json::Value,
    pub fragments: Vec<FragmentPreview>,
    pub managed_config_drift: bool,
    pub drifted_agents: Vec<Agent>,
    pub managed_collisions: Vec<ManagedCollision>,
    pub requires_codex_auth_approval: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_change: Option<FilePreview>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_backup: Option<FilePreview>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FragmentPreview {
    pub agent: Agent,
    pub role: String,
    pub path: String,
    pub format: Format,
    pub content: String,
}

impl From<Fragment> for FragmentPreview {
    fn from(fragment: Fragment) -> Self {
        Self {
            agent: fragment.agent,
            role: fragment.role,
            path: fragment.path,
            format: fragment.format,
            content: fragment.content,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileWriteStatus {
    pub path: String,
    pub operation: Operation,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub backup_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rollback_backup_path: String,
    pub replaced: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restored: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentWriteStatus {
    pub agent: Agent,
    pub success: bool,
    pub files: Vec<FileWriteStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backups: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rollback_backups: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rolled_back: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<ErrorCode>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WriteResult {
    pub transaction_id: String,
    pub agents: Vec<AgentWriteStatus>,
    pub sensitive_files: bool,
    pub warning: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_change: Option<FileWriteStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_backup: Option<FileWriteStatus>,
}

#[derive(Clone, Debug)]
pub struct PreviewRequest {
    pub agents: Vec<Agent>,
    pub modes: HashMap<Agent, ConfigMode>,
    pub catalog_token: String,
    pub model_config: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct WriteRequest {
    pub agents: Vec<Agent>,
    pub modes: HashMap<Agent, ConfigMode>,
    pub approve_rebuild: Vec<Agent>,
    pub revision_token: String,
    pub api_key: String,
    pub catalog_token: String,
    pub model_config: Vec<u8>,
    pub approve_managed_overwrite: bool,
    pub approve_codex_auth_change: bool,
}

#[derive(Clone, Debug)]
pub struct CatalogBinding {
    pub owner: String,
    pub router_base_url: String,
    pub deployment_id: String,
    pub protocol_version: String,
    pub models: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JournalProgress {
    Pending,
    Replaced,
    Restored,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionJournal {
    pub version: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key_generation: String,
    pub transaction_id: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub committed: bool,
    pub entries: Vec<JournalEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<JournalScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<Agent>,
    pub target_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<Operation>,
    pub pre_revision: FileRevision,
    pub post_revision: FileRevision,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub backup_path: String,
    #[serde(default, skip_serializing_if = "revision_empty")]
    pub backup_revision: FileRevision,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub restore_from: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rollback_backup_path: String,
    pub target_mode: u32,
    pub progress: JournalProgress,
}

fn revision_empty(value: &FileRevision) -> bool {
    *value == FileRevision::default()
}

pub fn agent_name(kind: Agent) -> &'static str {
    match kind {
        Agent::Claude => "Claude Code",
        Agent::OpenCode => "opencode",
        Agent::Codex => "Codex",
    }
}

pub fn clean_path(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let mut out = PathBuf::new();
    let mut absolute = false;
    for component in Path::new(path).components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => {
                absolute = true;
                out.push(component);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() && !absolute {
                    out.push("..");
                }
            }
            Component::Normal(name) => out.push(name),
        }
    }
    if out.as_os_str().is_empty() {
        if absolute {
            "/".into()
        } else {
            ".".into()
        }
    } else {
        out.to_string_lossy().into_owned()
    }
}

pub fn clean_optional_path(path: &str) -> String {
    if path.is_empty() {
        String::new()
    } else {
        clean_path(path)
    }
}

pub fn backup_claim_path(file: &PlannedFile) -> String {
    if file.backup_required {
        clean_optional_path(&file.backup_source)
    } else {
        String::new()
    }
}

pub fn unique_kinds(values: &[Agent]) -> Vec<Agent> {
    Agent::all()
        .into_iter()
        .filter(|kind| values.contains(kind))
        .collect()
}

pub fn unique_warnings(values: Vec<String>) -> Vec<String> {
    let mut seen = HashMap::new();
    let mut result = Vec::new();
    for value in values {
        if !value.is_empty() && seen.insert(value.clone(), true).is_none() {
            result.push(value);
        }
    }
    result.sort();
    result
}

pub fn different_path(source: &str, target: &str) -> String {
    if source == target {
        String::new()
    } else {
        source.to_owned()
    }
}

pub fn managed_namespace(kind: Agent, role: &str) -> &'static str {
    match kind {
        Agent::Claude => "/env",
        Agent::OpenCode => "/provider/mtls-router",
        Agent::Codex if role == "auth" => "/auth",
        Agent::Codex => "model_providers.mtls-router",
    }
}

pub fn plan_file<'a>(files: &'a [PlannedFile], kind: Agent, role: &str) -> Option<&'a PlannedFile> {
    files
        .iter()
        .find(|file| file.agent == Some(kind) && file.role == role)
}

pub fn revision_files(plan: &WritePlan) -> Vec<RevisionFile> {
    plan.files
        .iter()
        .filter(|file| file.scope != JournalScope::ManagerState)
        .map(|file| RevisionFile {
            agent: file.agent.unwrap_or(Agent::Claude),
            role: file.role.clone(),
            format: file.format.as_str().to_owned(),
            source_path: clean_path(&file.source_path),
            target_path: clean_path(&file.target_path),
            operation: file.operation.as_str().to_owned(),
            backup_required: file.backup_required,
            backup_source: backup_claim_path(file),
            source_revision: file.source_revision.claim(),
            target_revision: file.target_revision.claim(),
            companion_exists: file.companion_exists,
        })
        .collect()
}

pub fn revision_agents(plan: &WritePlan) -> Vec<RevisionAgent> {
    plan.agents
        .iter()
        .map(|item| RevisionAgent {
            agent: item.kind,
            mode: item.mode.as_str().to_owned(),
        })
        .collect()
}

pub fn cleanup_revision_file(file: &PlannedFile, removed: &[String]) -> CleanupRevisionFile {
    CleanupRevisionFile {
        role: file.role.clone(),
        source_path: clean_path(&file.source_path),
        target_path: clean_path(&file.target_path),
        operation: file.operation.as_str().to_owned(),
        backup_required: file.backup_required,
        backup_source: file.backup_source.clone(),
        source_revision: file.source_revision.claim(),
        target_revision: file.target_revision.claim(),
        removed_paths: removed.to_vec(),
    }
}

pub fn zero_bytes(value: &mut [u8]) {
    for byte in value {
        *byte = 0;
    }
}

pub fn result_for_plan(transaction_id: String, plan: &WritePlan) -> WriteResult {
    let mut result = WriteResult {
        transaction_id,
        agents: Vec::new(),
        sensitive_files: true,
        warning: BACKUP_WARNING.to_owned(),
        state_change: None,
        state_backup: None,
    };
    for kind in &plan.selected {
        let files = plan
            .files
            .iter()
            .filter(|file| file.agent == Some(*kind))
            .map(|file| FileWriteStatus {
                path: file.target_path.clone(),
                operation: file.operation,
                ..FileWriteStatus::default()
            })
            .collect();
        result.agents.push(AgentWriteStatus {
            agent: *kind,
            success: false,
            files,
            changed: Vec::new(),
            backups: Vec::new(),
            rollback_backups: Vec::new(),
            rolled_back: false,
            error_code: None,
        });
    }
    if let Some(file) = plan
        .files
        .iter()
        .find(|file| file.scope == JournalScope::ManagerState)
    {
        result.state_change = Some(FileWriteStatus {
            path: file.target_path.clone(),
            operation: file.operation,
            ..FileWriteStatus::default()
        });
    }
    result
}
