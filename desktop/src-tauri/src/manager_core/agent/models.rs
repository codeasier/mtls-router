use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};

use super::detect::{decode_object, read_config, strip_jsonc};
use super::modelconfig::TokenSigner;
use super::modelconfig::{
    canonical, decode, decode_structural, Agent, ClaudeConfig, ClaudeContext, ClaudeRole,
    CodexConfig, Config, Model, OpenCodeConfig, MAX_SAFE_INTEGER,
};
use super::plan::REVISION_CONTEXT_AGENT_FILE;
use super::revision::read_keyed_revision;
use super::sidecar::{read_sidecar, LastAppliedAgent};
use super::toml::decode_toml;
use super::types::{Format, OperationError, State};
use crate::protocol::ErrorCode;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelsExisting {
    pub model_config: Vec<u8>,
    pub unavailable_models: HashMap<String, Vec<String>>,
    pub drifted_agents: Vec<String>,
}

impl ModelsExisting {
    pub fn empty() -> Self {
        Self {
            model_config: b"{}".to_vec(),
            unavailable_models: HashMap::new(),
            drifted_agents: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelsPreset {
    pub model_config: Vec<u8>,
    pub unavailable_agents: HashMap<String, Vec<String>>,
}

impl ModelsPreset {
    pub fn empty() -> Self {
        Self {
            model_config: b"{}".to_vec(),
            unavailable_agents: HashMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelsResult {
    pub catalog_token: String,
    pub existing: ModelsExisting,
    pub preset: ModelsPreset,
}

pub fn discover_existing(
    signer: &TokenSigner,
    state_dir: &std::path::Path,
    generation: &str,
    selected: &[Agent],
    catalog: &[String],
    states: &[State],
) -> Result<ModelsExisting, OperationError> {
    let mut existing = ModelsExisting::empty();
    let by_kind: HashMap<_, _> = states.iter().map(|state| (state.agent, state)).collect();
    let available: HashSet<&str> = catalog.iter().map(String::as_str).collect();
    let mut config = Config {
        version: 1,
        claude: None,
        opencode: None,
        codex: None,
    };
    let (last_applied, _, _, _) = read_sidecar(signer, state_dir, generation)?;
    for kind in selected {
        let Some(state) = by_kind.get(kind) else {
            continue;
        };
        if !state.exists || state.invalid {
            continue;
        }
        if let Some(applied) = last_applied.agents.get(kind) {
            if applied_revisions_match(signer, applied) {
                if let Some((section, ids)) = decode_applied_section(*kind, &applied.model_config) {
                    let missing = unavailable(&ids, &available);
                    if !missing.is_empty() {
                        existing
                            .unavailable_models
                            .insert(kind.as_str().to_owned(), missing);
                    } else {
                        set_config_section(&mut config, *kind, section);
                    }
                    continue;
                }
            }
        }
        let Some((section, ids)) = typed_current_section(*kind, state) else {
            continue;
        };
        let missing = unavailable(&ids, &available);
        if !missing.is_empty() {
            existing
                .unavailable_models
                .insert(kind.as_str().to_owned(), missing);
        } else {
            set_config_section(&mut config, *kind, section);
        }
        existing.drifted_agents.push(kind.as_str().to_owned());
    }
    if config.claude.is_some() || config.opencode.is_some() || config.codex.is_some() {
        existing.model_config = canonical(&config).map_err(|_| {
            OperationError::new(ErrorCode::ConfigInvalid, "Agent model prefill is invalid")
        })?;
    }
    Ok(existing)
}

pub fn discover_preset(
    preset: Option<&Config>,
    selected: &[Agent],
    catalog: &[String],
) -> Result<ModelsPreset, OperationError> {
    let mut result = ModelsPreset::empty();
    let Some(preset) = preset else {
        return Ok(result);
    };
    let available: HashSet<&str> = catalog.iter().map(String::as_str).collect();
    let mut validated = Config {
        version: 1,
        claude: None,
        opencode: None,
        codex: None,
    };
    for kind in selected {
        match kind {
            Agent::Claude => {
                if let Some(section) = &preset.claude {
                    let ids = preset_model_ids(*kind, section);
                    let missing = unavailable(&ids, &available);
                    if !missing.is_empty() {
                        result
                            .unavailable_agents
                            .insert(kind.as_str().to_owned(), missing);
                    } else {
                        validated.claude = Some(section.clone());
                    }
                }
            }
            Agent::OpenCode => {
                if let Some(section) = &preset.opencode {
                    let ids = preset_model_ids(*kind, section);
                    let missing = unavailable(&ids, &available);
                    if !missing.is_empty() {
                        result
                            .unavailable_agents
                            .insert(kind.as_str().to_owned(), missing);
                    } else {
                        validated.opencode = Some(section.clone());
                    }
                }
            }
            Agent::Codex => {
                if let Some(section) = &preset.codex {
                    let ids = preset_model_ids(*kind, section);
                    let missing = unavailable(&ids, &available);
                    if !missing.is_empty() {
                        result
                            .unavailable_agents
                            .insert(kind.as_str().to_owned(), missing);
                    } else {
                        validated.codex = Some(section.clone());
                    }
                }
            }
        }
    }
    if validated.claude.is_some() || validated.opencode.is_some() || validated.codex.is_some() {
        result.model_config = canonical(&validated).map_err(|_| {
            OperationError::new(
                ErrorCode::ModelStateInvalid,
                "Agent model preset state is invalid",
            )
        })?;
    }
    Ok(result)
}

enum Section {
    Claude(ClaudeConfig),
    OpenCode(OpenCodeConfig),
    Codex(CodexConfig),
}

fn set_config_section(config: &mut Config, kind: Agent, section: Section) {
    match (kind, section) {
        (Agent::Claude, Section::Claude(value)) => config.claude = Some(value),
        (Agent::OpenCode, Section::OpenCode(value)) => config.opencode = Some(value),
        (Agent::Codex, Section::Codex(value)) => config.codex = Some(value),
        _ => {}
    }
}

fn preset_model_ids(kind: Agent, section: &dyn std::any::Any) -> Vec<String> {
    match kind {
        Agent::Claude => section
            .downcast_ref::<ClaudeConfig>()
            .map(claude_model_ids)
            .unwrap_or_default(),
        Agent::OpenCode => section
            .downcast_ref::<OpenCodeConfig>()
            .map(|config| {
                let mut ids: Vec<String> = config.models.keys().cloned().collect();
                ids.sort();
                ids.dedup();
                ids
            })
            .unwrap_or_default(),
        Agent::Codex => section
            .downcast_ref::<CodexConfig>()
            .map(|config| vec![config.model.clone()])
            .unwrap_or_default(),
    }
}

fn claude_model_ids(config: &ClaudeConfig) -> Vec<String> {
    let mut ids = vec![config.primary.model.clone()];
    for role in [&config.haiku, &config.sonnet, &config.opus] {
        if let Some(selection) = &role.selection {
            ids.push(selection.model.clone());
        }
    }
    if let Some(fable) = &config.fable {
        if let Some(selection) = &fable.selection {
            ids.push(selection.model.clone());
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

fn applied_revisions_match(signer: &TokenSigner, section: &LastAppliedAgent) -> bool {
    section.files.iter().all(|file| {
        read_keyed_revision(
            signer,
            std::path::Path::new(&file.path),
            REVISION_CONTEXT_AGENT_FILE,
        )
        .ok()
        .is_some_and(|(revision, _, _)| revision.token_value() == file.revision_mac)
    })
}

fn decode_applied_section(
    kind: Agent,
    section: &super::modelconfig::JsonValue,
) -> Option<(Section, Vec<String>)> {
    let document = super::modelconfig::JsonValue::object([
        (
            "version".into(),
            super::modelconfig::JsonValue::Number(super::modelconfig::JsonNumber("1".into())),
        ),
        (kind.as_str().to_owned(), section.clone()),
    ]);
    let raw = super::modelconfig::canonical_value(&document).ok()?;
    let structural = decode_structural(&raw).ok()?;
    match kind {
        Agent::Claude => {
            let section = structural.claude?;
            let ids = claude_model_ids(&section);
            Some((Section::Claude(section), ids))
        }
        Agent::OpenCode => {
            let section = structural.opencode?;
            let mut ids: Vec<String> = section.models.keys().cloned().collect();
            ids.sort();
            ids.dedup();
            Some((Section::OpenCode(section), ids))
        }
        Agent::Codex => {
            let section = structural.codex?;
            let ids = vec![section.model.clone()];
            Some((Section::Codex(section), ids))
        }
    }
}

fn typed_current_section(kind: Agent, state: &State) -> Option<(Section, Vec<String>)> {
    match kind {
        Agent::Claude => current_claude(&state.path),
        Agent::OpenCode => current_opencode(&state.path, state.format),
        Agent::Codex => current_codex(&state.path),
    }
}

fn current_claude(path: &str) -> Option<(Section, Vec<String>)> {
    let content = read_config(std::path::Path::new(path)).ok()?;
    let root = decode_object(&content)?;
    let env = root.get("env").and_then(Value::as_object)?;
    let primary = project_claude_selection(env.get("ANTHROPIC_MODEL").and_then(Value::as_str)?)?;
    let mut config = ClaudeConfig {
        primary: primary.clone(),
        fable: None,
        haiku: ClaudeRole::inherit(),
        sonnet: ClaudeRole::inherit(),
        opus: ClaudeRole::inherit(),
        context_window: None,
        max_output_tokens: None,
        extra: HashMap::new(),
    };
    if let Some(name) = env
        .get("ANTHROPIC_CUSTOM_MODEL_OPTION_NAME")
        .and_then(Value::as_str)
    {
        config.primary.name = Some(name.to_owned());
    }
    let mut ids = vec![config.primary.model.clone()];
    for (key, target) in [
        ("HAIKU", &mut config.haiku),
        ("SONNET", &mut config.sonnet),
        ("OPUS", &mut config.opus),
    ] {
        let id = env
            .get(&format!("ANTHROPIC_DEFAULT_{key}_MODEL"))
            .and_then(Value::as_str)?;
        let mut selection = project_claude_selection(id)?;
        if let Some(name) = env
            .get(&format!("ANTHROPIC_DEFAULT_{key}_MODEL_NAME"))
            .and_then(Value::as_str)
        {
            selection.name = Some(name.to_owned());
        }
        ids.push(selection.model.clone());
        *target = if same_claude_selection(&selection, &config.primary) {
            ClaudeRole::inherit()
        } else {
            ClaudeRole::selected(selection)
        };
    }
    if let Some(raw) = env.get("ANTHROPIC_DEFAULT_FABLE_MODEL") {
        let id = raw.as_str()?;
        let mut selection = project_claude_selection(id)?;
        if let Some(raw_name) = env.get("ANTHROPIC_DEFAULT_FABLE_MODEL_NAME") {
            selection.name = Some(raw_name.as_str()?.to_owned());
        }
        ids.push(selection.model.clone());
        config.fable = Some(if same_claude_selection(&selection, &config.primary) {
            ClaudeRole::inherit()
        } else {
            ClaudeRole::selected(selection)
        });
    }
    config.context_window = raw_positive_int_string(env.get("CLAUDE_CODE_MAX_CONTEXT_TOKENS"))?;
    config.max_output_tokens = raw_positive_int_string(env.get("CLAUDE_CODE_MAX_OUTPUT_TOKENS"))?;
    let document =
        serde_json::to_vec(&json!({"version": 1, "claude": claude_document(&config)})).ok()?;
    let decoded = decode_structural(&document).ok()?;
    Some((Section::Claude(decoded.claude?), ids))
}

fn claude_document(config: &ClaudeConfig) -> Value {
    // Re-decode through structural JSON so inherit/name/context match the schema.
    json!({
        "primary": {
            "model": config.primary.model,
            "name": config.primary.name,
            "context": config.primary.context.map(|_| "1m"),
        },
        "haiku": role_document(&config.haiku),
        "sonnet": role_document(&config.sonnet),
        "opus": role_document(&config.opus),
        "fable": config.fable.as_ref().map(role_document),
        "context_window": config.context_window,
        "max_output_tokens": config.max_output_tokens,
    })
}

fn role_document(role: &ClaudeRole) -> Value {
    if role.inherit_primary {
        json!({"inherit_primary": true})
    } else if let Some(selection) = &role.selection {
        json!({
            "model": selection.model,
            "name": selection.name,
            "context": selection.context.map(|_| "1m"),
        })
    } else {
        json!({"inherit_primary": true})
    }
}

fn current_opencode(path: &str, format: Format) -> Option<(Section, Vec<String>)> {
    let mut content = read_config(std::path::Path::new(path)).ok()?;
    if format == Format::Jsonc {
        content = strip_jsonc(&content).ok()?;
    }
    let root = decode_object(&content)?;
    let default_ref = root.get("model").and_then(Value::as_str)?;
    if !default_ref.starts_with("mtls-router/") {
        return None;
    }
    let providers = root.get("provider").and_then(Value::as_object)?;
    let provider = providers.get("mtls-router").and_then(Value::as_object)?;
    let models = provider.get("models").and_then(Value::as_object)?;
    if models.is_empty() {
        return None;
    }
    let default_model = default_ref.trim_start_matches("mtls-router/");
    let mut projected = Map::new();
    let mut ids = Vec::new();
    for (id, raw) in models {
        let object = raw.as_object()?;
        let mut entry = Map::new();
        for key in [
            "name",
            "reasoning",
            "attachment",
            "tool_call",
            "temperature",
            "limit",
            "modalities",
            "interleaved",
            "options",
        ] {
            if let Some(value) = object.get(key) {
                entry.insert(key.to_owned(), value.clone());
            }
        }
        if let Some(variants) = object.get("variants") {
            if let Some(typed) = variants.as_object() {
                if typed.values().all(Value::is_object) {
                    entry.insert("variants".into(), variants.clone());
                } else {
                    entry.insert("extra".into(), json!({"variants": variants}));
                }
            } else {
                entry.insert("extra".into(), json!({"variants": variants}));
            }
        }
        projected.insert(id.clone(), Value::Object(entry));
        ids.push(id.clone());
    }
    if !projected.contains_key(default_model) {
        return None;
    }
    let document = serde_json::to_vec(&json!({
        "version": 1,
        "opencode": {"default_model": default_model, "models": projected},
    }))
    .ok()?;
    let decoded = decode(&document, &[Agent::OpenCode], &ids).ok()?;
    Some((Section::OpenCode(decoded.opencode?), ids))
}

fn current_codex(path: &str) -> Option<(Section, Vec<String>)> {
    let content = read_config(std::path::Path::new(path)).ok()?;
    let values = decode_toml(&content)?;
    if values.get("model_provider").and_then(toml::Value::as_str) != Some("mtls-router") {
        return None;
    }
    let id = values
        .get("model")
        .and_then(toml::Value::as_str)?
        .to_owned();
    if id.is_empty() {
        return None;
    }
    let mut section = json!({"model": id});
    for (toml_key, config_key) in [
        ("model_reasoning_effort", "reasoning_effort"),
        ("model_reasoning_summary", "reasoning_summary"),
        ("model_verbosity", "verbosity"),
    ] {
        if let Some(value) = values.get(toml_key).and_then(toml::Value::as_str) {
            section
                .as_object_mut()?
                .insert(config_key.to_owned(), json!(value));
        }
    }
    for (toml_key, config_key) in [
        ("model_context_window", "context_window"),
        ("model_auto_compact_token_limit", "auto_compact_token_limit"),
    ] {
        if let Some(value) = values.get(toml_key).and_then(toml::Value::as_integer) {
            section
                .as_object_mut()?
                .insert(config_key.to_owned(), json!(value));
        }
    }
    if let Some(value) = values
        .get("model_auto_compact_token_limit_scope")
        .and_then(toml::Value::as_str)
    {
        section.as_object_mut()?.insert(
            "extra".into(),
            json!({"model_auto_compact_token_limit_scope": value}),
        );
    }
    let document = serde_json::to_vec(&json!({"version": 1, "codex": section})).ok()?;
    let decoded = decode(&document, &[Agent::Codex], &[id.clone()]).ok()?;
    Some((Section::Codex(decoded.codex?), vec![id]))
}

fn raw_positive_int_string(value: Option<&Value>) -> Option<Option<i64>> {
    let Some(value) = value else {
        return Some(None);
    };
    let text = value.as_str()?;
    if text.is_empty() {
        return None;
    }
    let parsed = text.parse::<i64>().ok()?;
    if parsed <= 0 || parsed > MAX_SAFE_INTEGER || parsed.to_string() != text {
        return None;
    }
    Some(Some(parsed))
}

fn trim_claude_context_suffix(value: &str) -> (String, bool) {
    for suffix in ["[1m]", "[1M]"] {
        if let Some(stripped) = value.strip_suffix(suffix) {
            return (stripped.to_owned(), true);
        }
    }
    (value.to_owned(), false)
}

fn has_claude_context_suffix(value: &str) -> bool {
    value.contains("[1m]") || value.contains("[1M]")
}

fn project_claude_selection(value: &str) -> Option<Model> {
    let (base, has_context) = trim_claude_context_suffix(value);
    if has_context && (base.is_empty() || has_claude_context_suffix(&base)) {
        return None;
    }
    if base.is_empty() {
        return None;
    }
    Some(Model {
        model: base,
        name: None,
        context: has_context.then_some(ClaudeContext::OneMillion),
    })
}

fn same_claude_selection(left: &Model, right: &Model) -> bool {
    left.model == right.model && left.name == right.name && left.context == right.context
}

fn unavailable(ids: &[String], available: &HashSet<&str>) -> Vec<String> {
    let mut missing: Vec<String> = ids
        .iter()
        .filter(|id| !available.contains(id.as_str()))
        .cloned()
        .collect();
    missing.sort();
    missing.dedup();
    missing
}
