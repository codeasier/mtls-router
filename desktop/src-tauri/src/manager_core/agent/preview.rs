use std::collections::HashMap;
use std::path::Path;

use serde_json::{Map, Value};

use super::detect::{decode_object, strip_jsonc};
use super::modelconfig::Agent;
use super::modelconfig::{CatalogClaims, TokenSigner};
use super::plan::{
    agent_name, different_path, managed_namespace, plan_file, revision_agents, revision_files,
    unique_kinds, unique_warnings, AgentPlan, BackupPlan, ConfigMode, FilePreview, FileRender,
    FileRevision, JournalScope, ManagedCollision, Operation, PlannedFile, Preview, WritePlan,
    BACKUP_WARNING, JSONC_MIGRATION_WARNING, JSONC_OVERRIDE_WARNING, REBUILD_WARNING,
    REVISION_CONTEXT_AGENT_FILE,
};
use super::recovery::{contains_recovery_reason, has_blocking_file_reason, path_writable};
use super::render::{
    assess_codex_merge, claude_owned_env_keys, codex_managed_config_keys, render_fragments,
    RenderInput,
};
use super::revision::read_keyed_revision;
use super::sidecar::LastAppliedState;
use super::toml::decode_toml;
use super::types::{Format, OperationError, RecoveryReason, State};
use crate::protocol::ErrorCode;

pub fn build_plan(
    signer: &TokenSigner,
    states: &[State],
    sidecar: LastAppliedState,
    sidecar_revision: FileRevision,
    sidecar_content: Vec<u8>,
    sidecar_mode: u32,
    agents: &[AgentPlan],
    input: RenderInput,
    catalog: CatalogClaims,
    canonical: Vec<u8>,
) -> Result<WritePlan, OperationError> {
    if input.config.claude.is_none()
        && input.config.opencode.is_none()
        && input.config.codex.is_none()
    {
        return Err(OperationError::new(
            ErrorCode::InvalidParams,
            "canonical model configuration is required",
        ));
    }
    let selected: Vec<Agent> = agents.iter().map(|item| item.kind).collect();
    let by_kind = ordered_states(states);
    let mut plan = WritePlan {
        agents: agents.to_vec(),
        selected,
        files: Vec::new(),
        revision_guards: Vec::new(),
        input: input.clone(),
        catalog,
        canonical,
        sidecar: sidecar.clone(),
        sidecar_revision,
        sidecar_content,
        sidecar_mode,
        drifted: Vec::new(),
        collisions: Vec::new(),
        requires_codex_auth_approval: false,
    };
    for item in agents {
        let state = by_kind.get(&item.kind).ok_or_else(|| {
            OperationError::new(
                ErrorCode::AgentNotFound,
                format!("{} is not detected", agent_name(item.kind)),
            )
        })?;
        if !state.detected {
            return Err(OperationError::new(
                ErrorCode::AgentNotFound,
                format!("{} is not detected", agent_name(item.kind)),
            ));
        }
        let mut files = if item.mode == ConfigMode::Rebuild {
            plan_rebuild(signer, state)?
        } else {
            if state.invalid {
                return Err(OperationError::new(
                    ErrorCode::ConfigInvalid,
                    format!("{} configuration is invalid", agent_name(item.kind)),
                ));
            }
            match item.kind {
                Agent::Claude => plan_claude(signer, state, &input, &sidecar)?,
                Agent::OpenCode => plan_opencode(signer, state, &input)?,
                Agent::Codex => plan_codex(signer, state, &input, &sidecar)?,
            }
        };
        for file in &mut files {
            file.mode = item.mode;
            file.scope = JournalScope::Agent;
            if file.role.is_empty() {
                file.role = "config".into();
            }
        }
        plan.files.extend(files);
    }
    inspect_ownership(signer, &mut plan)?;
    Ok(plan)
}

fn plan_rebuild(signer: &TokenSigner, state: &State) -> Result<Vec<PlannedFile>, OperationError> {
    validate_rebuild_state(state)?;
    let mut files = Vec::new();
    for recovery in &state.recovery.files {
        let (revision, content, mode) = read_keyed_revision(
            signer,
            Path::new(&recovery.path),
            REVISION_CONTEXT_AGENT_FILE,
        )
        .map_err(|_| {
            OperationError::new(
                ErrorCode::ConfigInvalid,
                format!("{} recovery state is unavailable", agent_name(state.agent)),
            )
        })?;
        if revision.exists != recovery.exists {
            return Err(OperationError::new(
                ErrorCode::ConfigInvalid,
                format!("{} recovery state is unavailable", agent_name(state.agent)),
            ));
        }
        let operation = if revision.exists {
            Operation::Replace
        } else {
            Operation::Create
        };
        let render = match (state.agent, recovery.role.as_str()) {
            (Agent::Claude, _) => FileRender::ClaudeFragment,
            (Agent::OpenCode, _) => FileRender::OpenCodeFragment,
            (Agent::Codex, "config") => FileRender::CodexFragment,
            (Agent::Codex, _) => FileRender::CodexAuthFragment,
        };
        let target_mode = if revision.exists { mode } else { 0o600 };
        files.push(PlannedFile {
            agent: Some(state.agent),
            mode: ConfigMode::Rebuild,
            format: if state.agent == Agent::OpenCode {
                Format::Json
            } else {
                recovery.format
            },
            source_path: recovery.path.clone(),
            target_path: recovery.path.clone(),
            operation,
            contains_api_key: recovery.role == "auth" || state.agent != Agent::Codex,
            preserves: Vec::new(),
            warning: REBUILD_WARNING.to_owned(),
            source_revision: revision.clone(),
            target_revision: revision,
            source_content: content,
            source_mode: mode,
            target_mode,
            backup_required: recovery.exists,
            backup_source: recovery.path.clone(),
            backup_path: String::new(),
            restore_from: recovery.path.clone(),
            render: Some(render),
            role: recovery.role.clone(),
            companion_exists: false,
            syntax_invalid: contains_recovery_reason(
                &recovery.reasons,
                RecoveryReason::SyntaxInvalid,
            ),
            scope: JournalScope::Agent,
        });
    }
    if state.agent == Agent::Codex && files.len() == 2 {
        files[0].companion_exists = files[1].source_revision.exists;
        files[1].companion_exists = files[0].source_revision.exists;
    }
    Ok(files)
}

pub fn validate_rebuild_state(state: &State) -> Result<(), OperationError> {
    if !state.detected || !state.invalid || !state.recovery.eligible {
        return Err(OperationError::new(
            ErrorCode::ConfigInvalid,
            format!(
                "{} configuration is not eligible for rebuild",
                agent_name(state.agent)
            ),
        ));
    }
    let expected = match state.agent {
        Agent::Codex => vec![
            ("config", state.path.as_str(), Format::Toml),
            ("auth", state.auth_path.as_str(), Format::Json),
        ],
        _ => vec![("config", state.path.as_str(), state.format)],
    };
    if state.recovery.files.len() != expected.len() {
        return Err(rebuild_metadata_invalid(state.agent));
    }
    for reason in &state.recovery.reasons {
        if *reason != RecoveryReason::SyntaxInvalid {
            return Err(OperationError::new(
                ErrorCode::ConfigInvalid,
                format!("{} recovery metadata is unsafe", agent_name(state.agent)),
            ));
        }
    }
    let mut syntax_invalid = false;
    for (file, (role, path, format)) in state.recovery.files.iter().zip(expected) {
        if file.role != role
            || super::plan::clean_path(&file.path) != super::plan::clean_path(path)
            || file.format != format
        {
            return Err(rebuild_metadata_invalid(state.agent));
        }
        for reason in &file.reasons {
            if *reason == RecoveryReason::SyntaxInvalid {
                syntax_invalid = true;
            } else {
                return Err(OperationError::new(
                    ErrorCode::ConfigInvalid,
                    format!("{} recovery metadata is unsafe", agent_name(state.agent)),
                ));
            }
        }
    }
    if !syntax_invalid {
        return Err(OperationError::new(
            ErrorCode::ConfigInvalid,
            format!(
                "{} configuration is not syntax-invalid",
                agent_name(state.agent)
            ),
        ));
    }
    Ok(())
}

fn rebuild_metadata_invalid(agent: Agent) -> OperationError {
    OperationError::new(
        ErrorCode::ConfigInvalid,
        format!("{} recovery metadata is invalid", agent_name(agent)),
    )
}

fn plan_claude(
    signer: &TokenSigner,
    state: &State,
    input: &RenderInput,
    sidecar: &LastAppliedState,
) -> Result<Vec<PlannedFile>, OperationError> {
    ensure_writable(&state.path)?;
    let (revision, content, mode) =
        read_keyed_revision(signer, Path::new(&state.path), REVISION_CONTEXT_AGENT_FILE).map_err(
            |_| {
                OperationError::new(
                    ErrorCode::ConfigInvalid,
                    "Claude Code configuration cannot be read",
                )
            },
        )?;
    let root = if revision.exists {
        decode_object(&content).ok_or_else(|| {
            OperationError::new(
                ErrorCode::ConfigInvalid,
                "Claude Code configuration is not a JSON object",
            )
        })?
    } else {
        Map::new()
    };
    let operation = if revision.exists {
        Operation::Replace
    } else {
        Operation::Create
    };
    Ok(vec![PlannedFile {
        agent: Some(Agent::Claude),
        mode: ConfigMode::Merge,
        format: Format::Json,
        source_path: state.path.clone(),
        target_path: state.path.clone(),
        operation,
        contains_api_key: true,
        preserves: vec!["all non-env top-level fields".into()],
        warning: String::new(),
        source_revision: revision.clone(),
        target_revision: revision.clone(),
        source_content: content,
        source_mode: mode,
        target_mode: mode,
        backup_required: revision.exists,
        backup_source: state.path.clone(),
        backup_path: String::new(),
        restore_from: state.path.clone(),
        render: Some(FileRender::ClaudeMerge {
            root,
            obsolete_owned_env: obsolete_claude_env(sidecar, input),
        }),
        role: "config".into(),
        companion_exists: false,
        syntax_invalid: false,
        scope: JournalScope::Agent,
    }])
}

fn plan_opencode(
    signer: &TokenSigner,
    state: &State,
    input: &RenderInput,
) -> Result<Vec<PlannedFile>, OperationError> {
    let mut source_path = state.path.clone();
    let mut target_path = state.path.clone();
    let mut format = state.format;
    let is_jsonc = Path::new(&source_path)
        .extension()
        .and_then(|value| value.to_str())
        == Some("jsonc");
    if is_jsonc {
        format = Format::Json;
        if !state.path_overridden {
            let parent = Path::new(&source_path)
                .parent()
                .map(|path| path.to_path_buf())
                .unwrap_or_else(|| Path::new(&source_path).to_path_buf());
            target_path = parent.join("opencode.json").to_string_lossy().into_owned();
            match std::fs::symlink_metadata(&source_path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    match std::fs::metadata(&target_path) {
                        Ok(info) if info.is_file() => source_path = target_path.clone(),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(_) | Ok(_) => {
                            return Err(OperationError::new(
                                ErrorCode::ConfigInvalid,
                                "opencode target cannot be inspected",
                            ));
                        }
                    }
                }
                Err(_) => {
                    return Err(OperationError::new(
                        ErrorCode::ConfigInvalid,
                        "opencode JSONC source cannot be inspected",
                    ));
                }
                Ok(_) => match std::fs::symlink_metadata(&target_path) {
                    Ok(_) => {
                        return Err(OperationError::new(
                            ErrorCode::ConfigInvalid,
                            "opencode JSONC migration target already exists",
                        ));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => {
                        return Err(OperationError::new(
                            ErrorCode::ConfigInvalid,
                            "opencode JSONC migration target already exists",
                        ));
                    }
                },
            }
        }
    }
    ensure_writable(&source_path)?;
    ensure_writable(&target_path)?;
    let (source_revision, source_content, source_mode) =
        read_keyed_revision(signer, Path::new(&source_path), REVISION_CONTEXT_AGENT_FILE).map_err(
            |_| {
                OperationError::new(
                    ErrorCode::ConfigInvalid,
                    "opencode configuration cannot be read",
                )
            },
        )?;
    let mut parsed = source_content.clone();
    if is_jsonc && source_revision.exists {
        parsed = strip_jsonc(&source_content).map_err(|_| {
            OperationError::new(
                ErrorCode::ConfigInvalid,
                "opencode JSONC configuration is invalid",
            )
        })?;
    }
    let root = if source_revision.exists {
        let root = decode_object(&parsed).ok_or_else(|| {
            OperationError::new(
                ErrorCode::ConfigInvalid,
                "opencode configuration is not a JSON object",
            )
        })?;
        if let Some(provider) = root.get("provider") {
            if !provider.is_null() && !provider.is_object() {
                return Err(OperationError::new(
                    ErrorCode::ConfigInvalid,
                    "opencode provider field is not an object",
                ));
            }
        }
        root
    } else {
        Map::new()
    };
    let (target_revision, _, target_mode) =
        read_keyed_revision(signer, Path::new(&target_path), REVISION_CONTEXT_AGENT_FILE).map_err(
            |_| OperationError::new(ErrorCode::ConfigInvalid, "opencode target cannot be read"),
        )?;
    let operation = if target_revision.exists {
        Operation::Replace
    } else {
        Operation::Create
    };
    let warning = if is_jsonc && source_revision.exists {
        if state.path_overridden {
            JSONC_OVERRIDE_WARNING.to_owned()
        } else if source_path != target_path {
            JSONC_MIGRATION_WARNING.to_owned()
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    Ok(vec![PlannedFile {
        agent: Some(Agent::OpenCode),
        mode: ConfigMode::Merge,
        format,
        source_path: source_path.clone(),
        target_path,
        operation,
        contains_api_key: true,
        preserves: vec![
            "all root fields other than provider.mtls-router".into(),
            "all other providers".into(),
        ],
        warning,
        source_revision: source_revision.clone(),
        target_revision,
        source_content,
        source_mode,
        target_mode,
        backup_required: source_revision.exists,
        backup_source: source_path.clone(),
        backup_path: String::new(),
        restore_from: source_path,
        render: Some(FileRender::OpenCodeMerge {
            root,
            own_root_model: input.own_root_model,
        }),
        role: "config".into(),
        companion_exists: false,
        syntax_invalid: false,
        scope: JournalScope::Agent,
    }])
}

fn plan_codex(
    signer: &TokenSigner,
    state: &State,
    input: &RenderInput,
    sidecar: &LastAppliedState,
) -> Result<Vec<PlannedFile>, OperationError> {
    ensure_writable(&state.path)?;
    ensure_writable(&state.auth_path)?;
    let (config_revision, config_content, config_mode) =
        read_keyed_revision(signer, Path::new(&state.path), REVISION_CONTEXT_AGENT_FILE).map_err(
            |_| {
                OperationError::new(
                    ErrorCode::ConfigInvalid,
                    "Codex configuration cannot be read",
                )
            },
        )?;
    let codex_root = if config_revision.exists {
        decode_toml(&config_content).ok_or_else(|| {
            OperationError::new(
                ErrorCode::ConfigInvalid,
                "Codex configuration is invalid TOML",
            )
        })?
    } else {
        toml::Table::new()
    };
    let (auth_revision, auth_content, auth_mode) = read_keyed_revision(
        signer,
        Path::new(&state.auth_path),
        REVISION_CONTEXT_AGENT_FILE,
    )
    .map_err(|_| {
        OperationError::new(
            ErrorCode::ConfigInvalid,
            "Codex auth configuration cannot be read",
        )
    })?;
    if auth_revision.exists && decode_object(&auth_content).is_none() {
        return Err(OperationError::new(
            ErrorCode::ConfigInvalid,
            "Codex auth configuration is invalid JSON",
        ));
    }
    let auth_root = decode_object(&auth_content);
    let assessment = assess_codex_merge(&codex_root, auth_root.as_ref().unwrap_or(&Map::new()))?;
    let config_operation = if config_revision.exists {
        Operation::Replace
    } else {
        Operation::Create
    };
    let auth_operation = if auth_revision.exists {
        Operation::Replace
    } else {
        Operation::Create
    };
    let config = PlannedFile {
        agent: Some(Agent::Codex),
        mode: ConfigMode::Merge,
        format: Format::Toml,
        source_path: state.path.clone(),
        target_path: state.path.clone(),
        operation: config_operation,
        contains_api_key: false,
        preserves: vec!["unrelated root keys and sections".into()],
        warning: String::new(),
        source_revision: config_revision.clone(),
        target_revision: config_revision.clone(),
        source_content: config_content.clone(),
        source_mode: config_mode,
        target_mode: config_mode,
        backup_required: config_revision.exists,
        backup_source: state.path.clone(),
        backup_path: String::new(),
        restore_from: state.path.clone(),
        render: Some(FileRender::CodexMerge {
            content: config_content,
            obsolete_optional: obsolete_codex_optional(sidecar, input),
            migrate_historical: assessment.historical_migration,
        }),
        role: "config".into(),
        companion_exists: auth_revision.exists,
        syntax_invalid: false,
        scope: JournalScope::Agent,
    };
    let auth = PlannedFile {
        agent: Some(Agent::Codex),
        mode: ConfigMode::Merge,
        format: Format::Json,
        source_path: state.auth_path.clone(),
        target_path: state.auth_path.clone(),
        operation: auth_operation,
        contains_api_key: true,
        preserves: Vec::new(),
        warning: "Codex authentication changes to file-backed API-key login and requires separate approval"
            .into(),
        source_revision: auth_revision.clone(),
        target_revision: auth_revision.clone(),
        source_content: auth_content,
        source_mode: auth_mode,
        target_mode: auth_mode,
        backup_required: auth_revision.exists,
        backup_source: state.auth_path.clone(),
        backup_path: String::new(),
        restore_from: state.auth_path.clone(),
        render: Some(FileRender::CodexAuth { root: auth_root }),
        role: "auth".into(),
        companion_exists: config_revision.exists,
        syntax_invalid: false,
        scope: JournalScope::Agent,
    };
    Ok(vec![config, auth])
}

fn obsolete_claude_env(sidecar: &LastAppliedState, input: &RenderInput) -> Vec<String> {
    let Some(previous) = sidecar.agents.get(&Agent::Claude) else {
        return Vec::new();
    };
    let Some(config) = &input.config.claude else {
        return Vec::new();
    };
    let current: HashMap<_, _> = claude_owned_env_keys(config)
        .into_iter()
        .map(|key| (format!("env.{key}"), true))
        .collect();
    previous
        .owned_paths
        .iter()
        .filter_map(|path| {
            path.strip_prefix("env.")
                .filter(|_| !current.contains_key(path))
                .map(ToOwned::to_owned)
        })
        .collect()
}

fn obsolete_codex_optional(sidecar: &LastAppliedState, input: &RenderInput) -> Vec<String> {
    let Some(previous) = sidecar.agents.get(&Agent::Codex) else {
        return Vec::new();
    };
    let Some(config) = &input.config.codex else {
        return Vec::new();
    };
    let current = codex_managed_config_keys(config, &input.api_base_url);
    previous
        .owned_paths
        .iter()
        .filter(|path| path.starts_with("model_") && !current.iter().any(|key| key == *path))
        .cloned()
        .collect()
}

fn inspect_ownership(signer: &TokenSigner, plan: &mut WritePlan) -> Result<(), OperationError> {
    for item in plan.agents.clone() {
        if item.mode == ConfigMode::Rebuild {
            continue;
        }
        let previous = plan.sidecar.agents.get(&item.kind).cloned();
        if let Some(previous) = &previous {
            for recorded in &previous.files {
                let current = read_keyed_revision(
                    signer,
                    Path::new(&recorded.path),
                    REVISION_CONTEXT_AGENT_FILE,
                );
                if current
                    .map(|(revision, _, _)| revision.token_value() != recorded.revision_mac)
                    .unwrap_or(true)
                {
                    plan.drifted.push(item.kind);
                    break;
                }
            }
            if item.kind != Agent::Claude {
                continue;
            }
        }
        let previous_owned = previous
            .as_ref()
            .map(|section| section.owned_paths.clone())
            .unwrap_or_default();
        for file in &plan.files {
            if file.agent != Some(item.kind) || !file.source_revision.exists {
                continue;
            }
            let mut collision = false;
            match item.kind {
                Agent::Claude => {
                    if let (Some(root), Some(config)) = (
                        decode_object(&file.source_content),
                        plan.input.config.claude.as_ref(),
                    ) {
                        let env = root
                            .get("env")
                            .and_then(Value::as_object)
                            .cloned()
                            .unwrap_or_default();
                        for key in claude_owned_env_keys(config) {
                            if env.contains_key(&key)
                                && !previous_owned
                                    .iter()
                                    .any(|path| path == &format!("env.{key}"))
                            {
                                plan.collisions.push(ManagedCollision {
                                    agent: item.kind,
                                    path: format!("/env/{key}"),
                                    kind: "fixed_managed_path".into(),
                                    action: "replace".into(),
                                });
                                collision = true;
                            }
                        }
                    }
                }
                Agent::OpenCode => {
                    let mut content = file.source_content.clone();
                    if file.format == Format::Jsonc {
                        if let Ok(stripped) = strip_jsonc(&content) {
                            content = stripped;
                        }
                    }
                    if let Some(root) = decode_object(&content) {
                        let model = root.contains_key("model");
                        let provider = root
                            .get("provider")
                            .and_then(Value::as_object)
                            .is_some_and(|value| value.contains_key("mtls-router"));
                        collision = model || provider;
                    }
                }
                Agent::Codex if file.role == "config" => {
                    if let Some(root) = decode_toml(&file.source_content) {
                        if let Ok(assessment) = assess_codex_merge(&root, &Map::new()) {
                            collision = assessment.managed_config_collision;
                        }
                    }
                }
                Agent::Codex => {}
            }
            if collision {
                if item.kind != Agent::Claude {
                    plan.collisions.push(ManagedCollision {
                        agent: item.kind,
                        path: managed_namespace(item.kind, &file.role).to_owned(),
                        kind: "fixed_managed_path".into(),
                        action: "replace".into(),
                    });
                }
                plan.drifted.push(item.kind);
                break;
            }
        }
    }
    if let Some(auth) = plan_file(&plan.files, Agent::Codex, "auth") {
        let config = plan_file(&plan.files, Agent::Codex, "config");
        let config_root = config
            .and_then(|file| decode_toml(&file.source_content))
            .unwrap_or_default();
        let auth_root = decode_object(&auth.source_content).unwrap_or_default();
        if let Ok(assessment) = assess_codex_merge(&config_root, &auth_root) {
            plan.requires_codex_auth_approval = assessment.requires_auth_approval;
        }
    }
    plan.drifted = unique_kinds(&plan.drifted);
    Ok(())
}

pub fn token_for_plan(signer: &TokenSigner, plan: &WritePlan) -> Result<String, OperationError> {
    signer
        .sign_revision(super::modelconfig::RevisionClaims {
            version: 0,
            agent_plans: revision_agents(plan),
            canonical_config: plan.canonical.clone(),
            catalog_identity: TokenSigner::catalog_identity(&plan.catalog),
            sidecar_revision: plan.sidecar_revision.claim(),
            router_base_url: plan.catalog.router_base_url.clone(),
            deployment_id: plan.catalog.deployment_id.clone(),
            protocol_version: plan.catalog.protocol_version.clone(),
            canonicalization: String::new(),
            key_generation: String::new(),
            managed_drift: !plan.drifted.is_empty(),
            requires_codex_auth_approval: plan.requires_codex_auth_approval,
            drifted_agents: plan.drifted.clone(),
            files: revision_files(plan),
        })
        .map_err(|_| {
            OperationError::new(
                ErrorCode::ModelStateInvalid,
                "Agent revision token could not be created",
            )
        })
}

pub fn preview_for_plan(
    signer: &TokenSigner,
    detector: &super::detect::Detector,
    sidecar_path: &str,
    plan: &WritePlan,
) -> Result<Preview, OperationError> {
    let token = token_for_plan(signer, plan)?;
    let states = detector.target_states().map_err(|_| {
        OperationError::new(
            ErrorCode::ConfigInvalid,
            "Agent target paths are unavailable",
        )
    })?;
    let mut fragment_input = plan.input.clone();
    fragment_input.key = super::types::REDACTED_API_KEY.to_owned();
    let fragments = render_fragments(&plan.selected, &states, &fragment_input)?;
    let state_operation = if plan.sidecar_revision.exists {
        Operation::Replace
    } else {
        Operation::Create
    };
    let mut state_change = FilePreview {
        role: String::new(),
        path: sidecar_path.to_owned(),
        source_path: String::new(),
        format: Format::Json,
        operation: state_operation,
        operations: vec![state_operation],
        contains_api_key: false,
        preserves: Vec::new(),
        backup: BackupPlan {
            required: plan.sidecar_revision.exists,
            pattern: String::new(),
            sensitive: plan.sidecar_revision.exists,
            warning: String::new(),
        },
        warning: String::new(),
    };
    if state_change.backup.required {
        state_change.backup.pattern = format!("{sidecar_path}.bak-<timestamp>-<random>");
        state_change.backup.warning = BACKUP_WARNING.to_owned();
    }
    let model_config = serde_json::from_slice(&plan.canonical).unwrap_or(serde_json::json!({}));
    let mut preview = Preview {
        revision_token: token,
        agents: Vec::new(),
        warnings: Vec::new(),
        model_config,
        fragments: fragments.into_iter().map(Into::into).collect(),
        managed_config_drift: !plan.drifted.is_empty(),
        drifted_agents: plan.drifted.clone(),
        managed_collisions: plan.collisions.clone(),
        requires_codex_auth_approval: plan.requires_codex_auth_approval,
        state_change: Some(state_change),
        state_backup: None,
    };
    for item in &plan.agents {
        let mut agent_preview = super::plan::AgentPreview {
            agent: item.kind,
            mode: item.mode,
            name: agent_name(item.kind).to_owned(),
            files: Vec::new(),
            warnings: Vec::new(),
        };
        for file in plan
            .files
            .iter()
            .filter(|file| file.agent == Some(item.kind))
        {
            let mut backup = BackupPlan {
                required: file.backup_required,
                pattern: String::new(),
                sensitive: file.backup_required,
                warning: String::new(),
            };
            if backup.required {
                backup.pattern = format!("{}.bak-<timestamp>-<random>", file.backup_source);
                backup.warning = BACKUP_WARNING.to_owned();
            }
            let mut operations = vec![file.operation];
            if !file.preserves.is_empty() {
                operations.push(Operation::Preserve);
            }
            agent_preview.files.push(FilePreview {
                role: file.role.clone(),
                path: file.target_path.clone(),
                source_path: different_path(&file.source_path, &file.target_path),
                format: file.format,
                operation: file.operation,
                operations,
                contains_api_key: file.contains_api_key,
                preserves: file.preserves.clone(),
                backup: backup.clone(),
                warning: file.warning.clone(),
            });
            if !file.warning.is_empty() {
                agent_preview.warnings.push(file.warning.clone());
                preview.warnings.push(file.warning.clone());
            }
            if backup.required {
                agent_preview.warnings.push(BACKUP_WARNING.to_owned());
                preview.warnings.push(BACKUP_WARNING.to_owned());
            }
        }
        agent_preview.warnings = unique_warnings(agent_preview.warnings);
        preview.agents.push(agent_preview);
    }
    preview.warnings = unique_warnings(preview.warnings);
    Ok(preview)
}

pub fn ordered_states(states: &[State]) -> HashMap<Agent, State> {
    states
        .iter()
        .map(|state| (state.agent, state.clone()))
        .collect()
}

pub fn ensure_writable(path: &str) -> Result<(), OperationError> {
    if !path_writable(Path::new(path)) {
        return Err(OperationError::new(
            ErrorCode::ConfigNotWritable,
            "Agent configuration path is not writable",
        ));
    }
    Ok(())
}

pub fn rebuild_files_eligible(files: &[super::types::RecoveryFileState]) -> bool {
    let mut has_syntax_invalid = false;
    for file in files {
        for reason in &file.reasons {
            if *reason == RecoveryReason::SyntaxInvalid {
                has_syntax_invalid = true;
            } else {
                return false;
            }
        }
    }
    has_syntax_invalid
}

pub fn without_recovery_reason(
    values: &[RecoveryReason],
    remove: RecoveryReason,
) -> Vec<RecoveryReason> {
    values
        .iter()
        .copied()
        .filter(|value| *value != remove)
        .collect()
}
