use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::detect::strip_jsonc;
use super::files::private_permissions_ok;
use super::modelconfig::{
    canonical_value, config_to_value, decode_value, Agent, Config, JsonNumber, JsonValue,
    TokenSigner,
};
use super::plan::{
    clean_path, ConfigMode, FileRender, FileRevision, JournalScope, Operation, PlannedFile,
    WritePlan,
};
use super::render::{claude_owned_env_keys, codex_managed_config_keys};
use super::revision::{keyed_revision_for_content, read_keyed_revision};
use super::trust::SIDECAR_FILE_NAME;
use super::types::OperationError;
use crate::protocol::ErrorCode;

pub const MAX_SIDECAR_SIZE: usize = 2 << 20;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LastAppliedState {
    pub version: i64,
    pub key_generation: String,
    pub agents: HashMap<Agent, LastAppliedAgent>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LastAppliedAgent {
    pub model_config: JsonValue,
    pub files: Vec<LastAppliedFile>,
    pub owned_paths: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LastAppliedFile {
    pub role: String,
    pub path: String,
    pub revision_mac: String,
}

pub fn sidecar_path(state_dir: &Path) -> PathBuf {
    state_dir.join(SIDECAR_FILE_NAME)
}

pub fn empty_sidecar(generation: &str) -> LastAppliedState {
    LastAppliedState {
        version: 1,
        key_generation: generation.to_owned(),
        agents: HashMap::new(),
    }
}

pub fn read_sidecar(
    signer: &TokenSigner,
    state_dir: &Path,
    generation: &str,
) -> Result<(LastAppliedState, FileRevision, Vec<u8>, u32), OperationError> {
    let path = sidecar_path(state_dir);
    let (revision, content, mode) =
        read_keyed_revision(signer, &path, super::plan::REVISION_CONTEXT_SIDECAR).map_err(
            |_| {
                OperationError::new(
                    ErrorCode::ModelStateInvalid,
                    "Agent model state cannot be read",
                )
            },
        )?;
    if !revision.exists {
        return Ok((empty_sidecar(generation), revision, Vec::new(), 0o600));
    }
    let info = std::fs::metadata(&path).map_err(|_| sidecar_invalid())?;
    if !private_permissions_ok(&path, false, unix_mode(&info))
        || content.is_empty()
        || content.len() > MAX_SIDECAR_SIZE
    {
        return Err(sidecar_invalid());
    }
    let value = decode_value(&content).map_err(|_| sidecar_invalid())?;
    let state = parse_sidecar_state(&value)?;
    validate_sidecar_state(&state, generation)?;
    let canonical = sidecar_to_canonical(&state)?;
    if content != canonical {
        return Err(OperationError::new(
            ErrorCode::ModelStateInvalid,
            "Agent model state is not canonical",
        ));
    }
    Ok((state, revision, content, mode))
}

pub fn validate_sidecar_state(
    state: &LastAppliedState,
    generation: &str,
) -> Result<(), OperationError> {
    if state.version != 1 || state.key_generation != generation || state.agents.len() > 3 {
        return Err(sidecar_invalid());
    }
    for (kind, section) in &state.agents {
        if matches!(section.model_config, JsonValue::Null) || section.files.is_empty() {
            return Err(sidecar_invalid());
        }
        let mut seen = HashMap::new();
        for file in &section.files {
            if (file.role != "config" && file.role != "auth")
                || seen.contains_key(&file.role)
                || !Path::new(&file.path).is_absolute()
                || file.revision_mac.is_empty()
                || file.revision_mac.len() > 128
            {
                return Err(sidecar_invalid());
            }
            seen.insert(file.role.clone(), true);
        }
        let files_ok = match kind {
            Agent::Claude | Agent::OpenCode => section.files.len() == 1,
            Agent::Codex => {
                section.files.len() == 2 && seen.contains_key("config") && seen.contains_key("auth")
            }
        };
        if !files_ok {
            return Err(sidecar_invalid());
        }
        if !section.owned_paths.windows(2).all(|pair| pair[0] < pair[1])
            && section.owned_paths.len() > 1
        {
            return Err(sidecar_invalid());
        }
        for (index, path) in section.owned_paths.iter().enumerate() {
            if path.is_empty()
                || path.len() > 4096
                || path.trim() != path
                || (index > 0 && path == &section.owned_paths[index - 1])
            {
                return Err(sidecar_invalid());
            }
        }
    }
    Ok(())
}

pub fn sidecar_to_canonical(state: &LastAppliedState) -> Result<Vec<u8>, OperationError> {
    canonical_value(&sidecar_to_value(state)?).map_err(|_| sidecar_invalid())
}

pub fn append_sidecar_plan(
    signer: &TokenSigner,
    state_dir: &Path,
    generation: &str,
    mut plan: WritePlan,
    key: &str,
) -> Result<WritePlan, OperationError> {
    let mut state = plan.sidecar.clone();
    for kind in &plan.selected {
        let previous = state.agents.get(kind).cloned().unwrap_or_default();
        let (model_config, owned) = sidecar_section(
            &plan.input.config,
            *kind,
            &plan.files,
            plan.input.own_root_model,
            &previous,
        )?;
        let mut entry = LastAppliedAgent {
            model_config,
            files: Vec::new(),
            owned_paths: owned,
        };
        for file in plan
            .files
            .iter()
            .filter(|file| file.agent == Some(*kind) && file.scope == JournalScope::Agent)
        {
            let mut output = file.output(&plan.input, key).map_err(|_| {
                OperationError::new(
                    ErrorCode::WriteFailed,
                    "could not render an Agent configuration file",
                )
            })?;
            let mode = if file.target_revision.exists {
                file.target_mode
            } else {
                0o600
            };
            let revision = keyed_revision_for_content(
                signer,
                &output,
                mode,
                super::plan::REVISION_CONTEXT_AGENT_FILE,
                &file.target_path,
            )
            .map_err(|_| sidecar_create_failed())?;
            super::plan::zero_bytes(&mut output);
            entry.files.push(LastAppliedFile {
                role: file.role.clone(),
                path: file.target_path.clone(),
                revision_mac: revision.token_value(),
            });
        }
        state.agents.insert(*kind, entry);
    }
    state.version = 1;
    state.key_generation = generation.to_owned();
    let content = sidecar_to_canonical(&state)?;
    if content.len() > MAX_SIDECAR_SIZE {
        return Err(sidecar_create_failed());
    }
    let mode = if plan.sidecar_revision.exists {
        plan.sidecar_mode
    } else {
        0o600
    };
    let operation = if plan.sidecar_revision.exists {
        Operation::Replace
    } else {
        Operation::Create
    };
    let path = sidecar_path(state_dir).to_string_lossy().into_owned();
    plan.sidecar = state;
    plan.files.push(PlannedFile {
        agent: None,
        mode: ConfigMode::Merge,
        format: super::types::Format::Json,
        source_path: path.clone(),
        target_path: path.clone(),
        operation,
        contains_api_key: false,
        preserves: Vec::new(),
        warning: String::new(),
        source_revision: plan.sidecar_revision.clone(),
        target_revision: plan.sidecar_revision.clone(),
        source_content: plan.sidecar_content.clone(),
        source_mode: plan.sidecar_mode,
        target_mode: mode,
        backup_required: plan.sidecar_revision.exists,
        backup_source: path.clone(),
        backup_path: String::new(),
        restore_from: path,
        render: Some(FileRender::Static(content)),
        role: "state".into(),
        companion_exists: false,
        syntax_invalid: false,
        scope: JournalScope::ManagerState,
    });
    Ok(plan)
}

fn sidecar_section(
    config: &Config,
    kind: Agent,
    files: &[PlannedFile],
    own_root_model: bool,
    previous: &LastAppliedAgent,
) -> Result<(JsonValue, Vec<String>), OperationError> {
    let encoded = config_to_value(config).map_err(|_| sidecar_create_failed())?;
    let object = encoded.as_object().ok_or_else(sidecar_create_failed)?;
    let section = object
        .get(kind.as_str())
        .cloned()
        .ok_or_else(sidecar_create_failed)?;
    let mut owned = match kind {
        Agent::Claude => config
            .claude
            .as_ref()
            .map(claude_owned_env_keys)
            .unwrap_or_default()
            .into_iter()
            .map(|key| format!("env.{key}"))
            .collect(),
        Agent::OpenCode => {
            let mut paths = vec!["provider.mtls-router".to_owned()];
            if open_code_plan_owns_root_model(files, own_root_model, previous) {
                paths.push("model".into());
            }
            paths
        }
        Agent::Codex => {
            let mut paths = vec![
                "model_providers.mtls-router".to_owned(),
                "auth.auth_mode".to_owned(),
                "auth.OPENAI_API_KEY".to_owned(),
            ];
            if let Some(codex) = &config.codex {
                for key in codex_managed_config_keys(codex, "") {
                    if key != "model_providers" {
                        paths.push(key);
                    }
                }
            }
            paths
        }
    };
    owned.sort();
    owned.dedup();
    Ok((section, owned))
}

fn open_code_plan_owns_root_model(
    files: &[PlannedFile],
    own_root_model: bool,
    previous: &LastAppliedAgent,
) -> bool {
    for file in files {
        if file.agent != Some(Agent::OpenCode) || file.scope != JournalScope::Agent {
            continue;
        }
        if file.mode == ConfigMode::Rebuild || own_root_model || !file.source_revision.exists {
            return true;
        }
        let mut content = file.source_content.clone();
        if Path::new(&file.source_path)
            .extension()
            .and_then(|value| value.to_str())
            == Some("jsonc")
        {
            match strip_jsonc(&content) {
                Ok(stripped) => content = stripped,
                Err(()) => return false,
            }
        }
        if let Some(root) = super::detect::decode_object(&content) {
            let Some(model) = root.get("model") else {
                return true;
            };
            return model
                .as_str()
                .is_some_and(|value| value.starts_with("mtls-router/"))
                && previous_owns_opencode_model(previous, file);
        }
    }
    false
}

fn previous_owns_opencode_model(previous: &LastAppliedAgent, file: &PlannedFile) -> bool {
    if !previous.owned_paths.iter().any(|path| path == "model") {
        return false;
    }
    previous.files.iter().any(|recorded| {
        recorded.role == "config"
            && file.role == "config"
            && clean_path(&recorded.path) == clean_path(&file.source_path)
    })
}

fn sidecar_to_value(state: &LastAppliedState) -> Result<JsonValue, OperationError> {
    let mut agents = HashMap::new();
    for (kind, section) in &state.agents {
        let files = JsonValue::Array(
            section
                .files
                .iter()
                .map(|file| {
                    JsonValue::object([
                        ("role".into(), JsonValue::String(file.role.clone())),
                        ("path".into(), JsonValue::String(file.path.clone())),
                        (
                            "revision_mac".into(),
                            JsonValue::String(file.revision_mac.clone()),
                        ),
                    ])
                })
                .collect(),
        );
        agents.insert(
            kind.as_str().to_owned(),
            JsonValue::object([
                ("model_config".into(), section.model_config.clone()),
                ("files".into(), files),
                (
                    "owned_paths".into(),
                    JsonValue::Array(
                        section
                            .owned_paths
                            .iter()
                            .map(|path| JsonValue::String(path.clone()))
                            .collect(),
                    ),
                ),
            ]),
        );
    }
    Ok(JsonValue::object([
        (
            "version".into(),
            JsonValue::Number(JsonNumber(state.version.to_string())),
        ),
        (
            "key_generation".into(),
            JsonValue::String(state.key_generation.clone()),
        ),
        ("agents".into(), JsonValue::Object(agents)),
    ]))
}

fn parse_sidecar_state(value: &JsonValue) -> Result<LastAppliedState, OperationError> {
    let object = value.as_object().ok_or_else(sidecar_invalid)?;
    require_keys(object, &["version", "key_generation", "agents"])?;
    let version = object
        .get("version")
        .and_then(JsonValue::as_number)
        .and_then(|value| value.as_str().parse().ok())
        .ok_or_else(sidecar_invalid)?;
    let key_generation = object
        .get("key_generation")
        .and_then(JsonValue::as_str)
        .ok_or_else(sidecar_invalid)?
        .to_owned();
    let agents_value = object.get("agents").ok_or_else(sidecar_invalid)?;
    let agents_object = agents_value.as_object().ok_or_else(sidecar_invalid)?;
    let mut agents = HashMap::new();
    for (key, value) in agents_object {
        let kind = Agent::parse(key).ok_or_else(sidecar_invalid)?;
        agents.insert(kind, parse_agent_section(value)?);
    }
    Ok(LastAppliedState {
        version,
        key_generation,
        agents,
    })
}

fn parse_agent_section(value: &JsonValue) -> Result<LastAppliedAgent, OperationError> {
    let object = value.as_object().ok_or_else(sidecar_invalid)?;
    require_keys(object, &["model_config", "files", "owned_paths"])?;
    let files = object
        .get("files")
        .and_then(JsonValue::as_array)
        .ok_or_else(sidecar_invalid)?;
    let owned = object
        .get("owned_paths")
        .and_then(JsonValue::as_array)
        .ok_or_else(sidecar_invalid)?;
    Ok(LastAppliedAgent {
        model_config: object
            .get("model_config")
            .cloned()
            .ok_or_else(sidecar_invalid)?,
        files: files.iter().map(parse_file).collect::<Result<_, _>>()?,
        owned_paths: owned
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(sidecar_invalid)
            })
            .collect::<Result<_, _>>()?,
    })
}

fn parse_file(value: &JsonValue) -> Result<LastAppliedFile, OperationError> {
    let object = value.as_object().ok_or_else(sidecar_invalid)?;
    require_keys(object, &["role", "path", "revision_mac"])?;
    Ok(LastAppliedFile {
        role: required_string(object, "role")?,
        path: required_string(object, "path")?,
        revision_mac: required_string(object, "revision_mac")?,
    })
}

fn require_keys(object: &HashMap<String, JsonValue>, keys: &[&str]) -> Result<(), OperationError> {
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err(sidecar_invalid());
    }
    Ok(())
}

fn required_string(
    object: &HashMap<String, JsonValue>,
    key: &str,
) -> Result<String, OperationError> {
    object
        .get(key)
        .and_then(JsonValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(sidecar_invalid)
}

fn unix_mode(info: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        info.permissions().mode()
    }
    #[cfg(not(unix))]
    {
        0
    }
}

fn sidecar_invalid() -> OperationError {
    OperationError::new(ErrorCode::ModelStateInvalid, "Agent model state is invalid")
}

fn sidecar_create_failed() -> OperationError {
    OperationError::new(
        ErrorCode::ModelStateInvalid,
        "Agent model state could not be created",
    )
}
