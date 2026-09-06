use std::path::Path;

use super::detect::{decode_object, strip_jsonc};
use super::lock::acquire_existing_transaction_lock;
use super::modelconfig::{
    canonical_value, decode_structural, Agent, CleanupRevisionClaims, Config, JsonNumber,
    JsonValue, TokenSigner,
};
use super::plan::{
    agent_name, cleanup_revision_file, AgentPlan, BackupPlan, ConfigMode, FilePreview, FileRender,
    FileRevision, JournalScope, Operation, PlannedFile, PlannedRevisionGuard, WritePlan,
};
use super::recovery::{contains_recovery_reason, inspect_recovery_target};
use super::render::{cleanup_claude, cleanup_codex_auth, cleanup_codex_config, cleanup_opencode};
use super::revision::read_keyed_revision;
use super::sidecar::{
    read_sidecar, sidecar_path, LastAppliedAgent, LastAppliedFile, LastAppliedState,
};
use super::types::{Format, OperationError, RecoveryReason};
use crate::protocol::ErrorCode;

#[derive(Clone, Debug)]
pub struct CleanupPreview {
    pub revision_token: String,
    pub agent: Agent,
    pub files: Vec<FilePreview>,
    pub removed_paths: Vec<String>,
    pub managed_config_drift: bool,
    pub state_change: Option<FilePreview>,
    pub state_backup: Option<FilePreview>,
}

pub struct CleanupPlan {
    pub write: WritePlan,
    pub removed_by_path: std::collections::HashMap<String, Vec<String>>,
    pub token_files: Vec<super::modelconfig::CleanupRevisionFile>,
    pub state_operation: Operation,
}

impl CleanupPlan {
    pub fn managed_config_drift(&self) -> bool {
        !self.write.drifted.is_empty()
    }
}

pub fn build_cleanup_plan(
    signer: &TokenSigner,
    state_dir: &Path,
    generation: &str,
    kind: Agent,
) -> Result<CleanupPlan, OperationError> {
    let (mut state, state_revision, state_content, state_mode) =
        read_sidecar(signer, state_dir, generation)?;
    let section = state.agents.get(&kind).cloned().ok_or_else(|| {
        OperationError::new(
            ErrorCode::AgentNotManaged,
            format!("{} is not managed by CodeasierRouter", agent_name(kind)),
        )
    })?;
    let saved = decode_cleanup_model_config(kind, &section.model_config)?;
    let mut plan = CleanupPlan {
        write: WritePlan {
            agents: vec![AgentPlan {
                kind,
                mode: ConfigMode::Merge,
            }],
            selected: vec![kind],
            files: Vec::new(),
            revision_guards: Vec::new(),
            input: Default::default(),
            catalog: super::modelconfig::CatalogClaims {
                version: 1,
                models: vec!["unused".into()],
                agents: vec![kind],
                owner: "desktop".into(),
                router_base_url: "http://127.0.0.1:1".into(),
                deployment_id: "cleanup".into(),
                protocol_version: crate::protocol::MANAGEMENT_PROTOCOL_VERSION.into(),
                simplify: false,
                canonicalization: String::new(),
                key_generation: String::new(),
            },
            canonical: b"{}".to_vec(),
            sidecar: state.clone(),
            sidecar_revision: state_revision.clone(),
            sidecar_content: state_content.clone(),
            sidecar_mode: state_mode,
            drifted: Vec::new(),
            collisions: Vec::new(),
            requires_codex_auth_approval: false,
        },
        removed_by_path: std::collections::HashMap::new(),
        token_files: Vec::new(),
        state_operation: Operation::Replace,
    };
    for recorded in &section.files {
        let (file, removed, drifted) = cleanup_file_plan(signer, kind, recorded, &section, &saved)?;
        if drifted {
            plan.write.drifted = vec![kind];
        }
        if !file.source_revision.exists {
            plan.token_files.push(cleanup_revision_file(&file, &[]));
            plan.write.revision_guards.push(PlannedRevisionGuard {
                path: file.source_path.clone(),
                revision: file.source_revision.clone(),
                scope: JournalScope::Agent,
            });
            continue;
        }
        if removed.is_empty() && file.operation != Operation::Delete {
            continue;
        }
        plan.removed_by_path
            .insert(file.target_path.clone(), removed.clone());
        plan.token_files
            .push(cleanup_revision_file(&file, &removed));
        plan.write.files.push(file);
    }
    state.agents.remove(&kind);
    plan.write.sidecar = state.clone();
    let sidecar = sidecar_path(state_dir).to_string_lossy().into_owned();
    if state.agents.is_empty() {
        plan.state_operation = Operation::Delete;
        plan.write.files.push(PlannedFile {
            agent: None,
            mode: ConfigMode::Merge,
            format: Format::Json,
            source_path: sidecar.clone(),
            target_path: sidecar.clone(),
            operation: Operation::Delete,
            contains_api_key: false,
            preserves: Vec::new(),
            warning: String::new(),
            source_revision: state_revision.clone(),
            target_revision: state_revision,
            source_content: state_content,
            source_mode: state_mode,
            target_mode: state_mode,
            backup_required: true,
            backup_source: sidecar.clone(),
            backup_path: String::new(),
            restore_from: sidecar,
            render: None,
            role: "state".into(),
            companion_exists: false,
            syntax_invalid: false,
            scope: JournalScope::ManagerState,
        });
        return Ok(plan);
    }
    let state_output = sidecar_canonical(&state)?;
    plan.state_operation = Operation::Replace;
    plan.write.files.push(PlannedFile {
        agent: None,
        mode: ConfigMode::Merge,
        format: Format::Json,
        source_path: sidecar.clone(),
        target_path: sidecar.clone(),
        operation: Operation::Replace,
        contains_api_key: false,
        preserves: Vec::new(),
        warning: String::new(),
        source_revision: state_revision.clone(),
        target_revision: state_revision,
        source_content: state_content,
        source_mode: state_mode,
        target_mode: state_mode,
        backup_required: true,
        backup_source: sidecar.clone(),
        backup_path: String::new(),
        restore_from: sidecar,
        render: Some(FileRender::Static(state_output)),
        role: "state".into(),
        companion_exists: false,
        syntax_invalid: false,
        scope: JournalScope::ManagerState,
    });
    Ok(plan)
}

fn sidecar_canonical(state: &LastAppliedState) -> Result<Vec<u8>, OperationError> {
    super::sidecar::sidecar_to_canonical(state).and_then(|content| {
        if content.len() > super::sidecar::MAX_SIDECAR_SIZE {
            Err(OperationError::new(
                ErrorCode::ModelStateInvalid,
                "Agent model state could not be updated",
            ))
        } else {
            Ok(content)
        }
    })
}

fn decode_cleanup_model_config(kind: Agent, raw: &JsonValue) -> Result<Config, OperationError> {
    let document = canonical_value(&JsonValue::object([
        ("version".into(), JsonValue::Number(JsonNumber("1".into()))),
        (kind.as_str().to_owned(), raw.clone()),
    ]))
    .map_err(|_| {
        OperationError::new(
            ErrorCode::ModelStateInvalid,
            "saved Agent model state is invalid",
        )
    })?;
    decode_structural(&document).map_err(|_| {
        OperationError::new(
            ErrorCode::ModelStateInvalid,
            "saved Agent model state is invalid",
        )
    })
}

fn cleanup_file_plan(
    signer: &TokenSigner,
    kind: Agent,
    recorded: &LastAppliedFile,
    section: &LastAppliedAgent,
    saved: &Config,
) -> Result<(PlannedFile, Vec<String>, bool), OperationError> {
    let format = cleanup_file_format(kind, recorded);
    let safety = inspect_recovery_target(&recorded.role, Path::new(&recorded.path), format);
    if !safety.exists && !contains_recovery_reason(&safety.reasons, RecoveryReason::Unreadable) {
        return Ok((
            PlannedFile {
                agent: Some(kind),
                mode: ConfigMode::Merge,
                format,
                source_path: recorded.path.clone(),
                target_path: recorded.path.clone(),
                operation: Operation::Delete,
                contains_api_key: false,
                preserves: Vec::new(),
                warning: String::new(),
                source_revision: FileRevision::default(),
                target_revision: FileRevision::default(),
                source_content: Vec::new(),
                source_mode: 0,
                target_mode: 0,
                backup_required: false,
                backup_source: String::new(),
                backup_path: String::new(),
                restore_from: String::new(),
                render: None,
                role: recorded.role.clone(),
                companion_exists: false,
                syntax_invalid: false,
                scope: JournalScope::Agent,
            },
            Vec::new(),
            true,
        ));
    }
    if !safety.reasons.is_empty() {
        if contains_recovery_reason(&safety.reasons, RecoveryReason::NotWritable)
            || contains_recovery_reason(&safety.reasons, RecoveryReason::ParentUnavailable)
        {
            return Err(OperationError::new(
                ErrorCode::ConfigNotWritable,
                format!("{} managed configuration is not writable", agent_name(kind)),
            ));
        }
        return Err(OperationError::new(
            ErrorCode::ConfigInvalid,
            format!("{} managed configuration path is unsafe", agent_name(kind)),
        ));
    }
    let (revision, content, mode) = read_keyed_revision(
        signer,
        Path::new(&recorded.path),
        super::plan::REVISION_CONTEXT_AGENT_FILE,
    )
    .map_err(|_| {
        OperationError::new(
            ErrorCode::ConfigInvalid,
            format!("{} managed configuration cannot be read", agent_name(kind)),
        )
    })?;
    if !revision.exists {
        return Ok((
            PlannedFile {
                agent: Some(kind),
                mode: ConfigMode::Merge,
                format,
                source_path: recorded.path.clone(),
                target_path: recorded.path.clone(),
                operation: Operation::Delete,
                contains_api_key: false,
                preserves: Vec::new(),
                warning: String::new(),
                source_revision: revision.clone(),
                target_revision: revision,
                source_content: Vec::new(),
                source_mode: 0,
                target_mode: 0,
                backup_required: false,
                backup_source: String::new(),
                backup_path: String::new(),
                restore_from: String::new(),
                render: None,
                role: recorded.role.clone(),
                companion_exists: false,
                syntax_invalid: false,
                scope: JournalScope::Agent,
            },
            Vec::new(),
            true,
        ));
    }
    super::preview::ensure_writable(&recorded.path)?;
    let transform = match kind {
        Agent::Claude => {
            let root = decode_object(&content).ok_or_else(|| {
                OperationError::new(
                    ErrorCode::ConfigInvalid,
                    "Claude Code configuration is not a JSON object",
                )
            })?;
            cleanup_claude(&root, &section.owned_paths)?
        }
        Agent::OpenCode => {
            let parsed = if format == Format::Jsonc {
                strip_jsonc(&content).map_err(|_| {
                    OperationError::new(
                        ErrorCode::ConfigInvalid,
                        "opencode configuration is invalid JSONC",
                    )
                })?
            } else {
                content.clone()
            };
            let root = decode_object(&parsed).ok_or_else(|| {
                OperationError::new(
                    ErrorCode::ConfigInvalid,
                    "opencode configuration is not a JSON object",
                )
            })?;
            cleanup_opencode(
                &root,
                section.owned_paths.iter().any(|path| path == "model"),
            )?
        }
        Agent::Codex if recorded.role == "config" => {
            let saved = saved.codex.as_ref().ok_or_else(|| {
                OperationError::new(
                    ErrorCode::ConfigInvalid,
                    "saved Codex configuration is invalid",
                )
            })?;
            cleanup_codex_config(&content, saved)?
        }
        Agent::Codex => {
            let root = decode_object(&content).ok_or_else(|| {
                OperationError::new(
                    ErrorCode::ConfigInvalid,
                    "Codex auth configuration is not a JSON object",
                )
            })?;
            cleanup_codex_auth(&root)?
        }
    };
    let operation = if transform.delete {
        Operation::Delete
    } else {
        Operation::Replace
    };
    let mut file = PlannedFile {
        agent: Some(kind),
        mode: ConfigMode::Merge,
        role: recorded.role.clone(),
        format,
        source_path: recorded.path.clone(),
        target_path: recorded.path.clone(),
        operation,
        contains_api_key: kind != Agent::Codex || recorded.role == "auth",
        preserves: Vec::new(),
        warning: String::new(),
        source_revision: revision.clone(),
        target_revision: revision.clone(),
        source_content: content,
        source_mode: mode,
        target_mode: mode,
        backup_required: true,
        backup_source: recorded.path.clone(),
        backup_path: String::new(),
        restore_from: recorded.path.clone(),
        render: None,
        companion_exists: false,
        syntax_invalid: false,
        scope: JournalScope::Agent,
    };
    if operation == Operation::Replace {
        file.render = Some(FileRender::Static(transform.content));
    }
    Ok((
        file,
        transform.removed_paths,
        revision.token_value() != recorded.revision_mac,
    ))
}

fn cleanup_file_format(kind: Agent, recorded: &LastAppliedFile) -> Format {
    if kind == Agent::Codex && recorded.role == "config" {
        return Format::Toml;
    }
    if kind == Agent::OpenCode
        && Path::new(&recorded.path)
            .extension()
            .and_then(|value| value.to_str())
            == Some("jsonc")
    {
        return Format::Jsonc;
    }
    Format::Json
}

pub fn cleanup_token_for_plan(
    signer: &TokenSigner,
    plan: &CleanupPlan,
) -> Result<String, OperationError> {
    signer
        .sign_cleanup_revision(CleanupRevisionClaims {
            agent: plan.write.selected[0],
            files: plan.token_files.clone(),
            state_operation: plan.state_operation.as_str().to_owned(),
            state_revision: plan.write.sidecar_revision.claim(),
            managed_config_drift: plan.managed_config_drift(),
        })
        .map_err(|_| {
            OperationError::new(
                ErrorCode::ModelStateInvalid,
                "Agent cleanup revision token could not be created",
            )
        })
}

pub fn cleanup_preview_for_plan(
    signer: &TokenSigner,
    state_dir: &Path,
    plan: &CleanupPlan,
) -> Result<CleanupPreview, OperationError> {
    let token = cleanup_token_for_plan(signer, plan)?;
    let sidecar = sidecar_path(state_dir).to_string_lossy().into_owned();
    let mut preview = CleanupPreview {
        revision_token: token,
        agent: plan.write.selected[0],
        files: Vec::new(),
        removed_paths: Vec::new(),
        managed_config_drift: plan.managed_config_drift(),
        state_change: Some(FilePreview {
            role: "state".into(),
            path: sidecar.clone(),
            source_path: String::new(),
            format: Format::Json,
            operation: plan.state_operation,
            operations: vec![plan.state_operation],
            contains_api_key: false,
            preserves: Vec::new(),
            backup: cleanup_backup_plan(&sidecar, false),
            warning: String::new(),
        }),
        state_backup: Some(FilePreview {
            role: "state".into(),
            path: sidecar.clone(),
            source_path: String::new(),
            format: Format::Json,
            operation: Operation::Backup,
            operations: vec![Operation::Backup],
            contains_api_key: false,
            preserves: Vec::new(),
            backup: cleanup_backup_plan(&sidecar, false),
            warning: String::new(),
        }),
    };
    for file in &plan.write.files {
        if file.scope == JournalScope::ManagerState {
            continue;
        }
        let removed = plan
            .removed_by_path
            .get(&file.target_path)
            .cloned()
            .unwrap_or_default();
        preview.removed_paths.extend(removed);
        preview.files.push(FilePreview {
            role: file.role.clone(),
            path: file.target_path.clone(),
            source_path: String::new(),
            format: file.format,
            operation: file.operation,
            operations: vec![file.operation],
            contains_api_key: file.contains_api_key,
            preserves: Vec::new(),
            backup: cleanup_backup_plan(&file.backup_source, true),
            warning: String::new(),
        });
    }
    preview.removed_paths.sort();
    preview.removed_paths.dedup();
    Ok(preview)
}

fn cleanup_backup_plan(path: &str, sensitive: bool) -> BackupPlan {
    BackupPlan {
        required: true,
        sensitive,
        pattern: format!("{path}.bak-<timestamp>-<random>"),
        warning: String::new(),
    }
}

pub fn ensure_cleanup_state_exists(state_dir: &Path, kind: Agent) -> Result<(), OperationError> {
    match std::fs::metadata(sidecar_path(state_dir)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(OperationError::new(
            ErrorCode::AgentNotManaged,
            format!("{} is not managed by CodeasierRouter", agent_name(kind)),
        )),
        Err(_) => Err(OperationError::new(
            ErrorCode::ModelStateInvalid,
            "Agent model state cannot be read",
        )),
        Ok(_) => Ok(()),
    }
}

pub fn acquire_cleanup_preview_lock(
    state_dir: &Path,
    deadline: Option<std::time::Instant>,
) -> Result<super::lock::TransactionLock, OperationError> {
    acquire_existing_transaction_lock(state_dir, deadline).map_err(|error| {
        if error.code == ErrorCode::AgentOperationBusy {
            error
        } else {
            OperationError::new(
                ErrorCode::ModelStateInvalid,
                "Agent cleanup coordination state is invalid",
            )
        }
    })
}
