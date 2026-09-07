use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use super::detect::{decode_object, exact_historical_codex, has_nonempty_json_string, raw_string};
use super::modelconfig::{
    canonical, deep_merge, Agent, ClaudeConfig, ClaudeRole, CodexConfig, Config, Interleaved,
    JsonValue, Model, OpenCodeConfig,
};
use super::toml::{encode_toml, toml_table};
use super::types::{
    is_managed_provider_display_name, Format, OperationError, State, MAX_RENDER_SIZE,
    PROVIDER_DISPLAY_NAME, REDACTED_API_KEY,
};
use super::url::{api_url, safe_display_path, valid_api_value};
use crate::protocol::ErrorCode;

const CLAUDE_UNCONDITIONAL_ENV_KEYS: [&str; 13] = [
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_CUSTOM_MODEL_OPTION",
    "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
    "ENABLE_TOOL_SEARCH",
    "DISABLE_AUTOUPDATER",
];

const CODEX_REQUIRED_ROOT_KEYS: [&str; 3] =
    ["model_provider", "model", "cli_auth_credentials_store"];
const CODEX_COMPETING_AUTH_KEYS: [&str; 5] = [
    "tokens",
    "last_refresh",
    "agent_identity",
    "personal_access_token",
    "bedrock_api_key",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fragment {
    pub agent: Agent,
    pub role: String,
    pub path: String,
    pub format: Format,
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderResult {
    pub model_config: Vec<u8>,
    pub fragments: Vec<Fragment>,
}

#[derive(Clone, Debug)]
pub struct RenderInput {
    pub config: Config,
    pub router_base_url: String,
    pub api_base_url: String,
    pub key: String,
    pub own_root_model: bool,
}

impl Default for RenderInput {
    fn default() -> Self {
        Self {
            config: Config {
                version: 1,
                claude: None,
                opencode: None,
                codex: None,
            },
            router_base_url: String::new(),
            api_base_url: String::new(),
            key: REDACTED_API_KEY.to_owned(),
            own_root_model: false,
        }
    }
}

pub fn render_fragments(
    selected: &[Agent],
    states: &HashMap<Agent, State>,
    input: &RenderInput,
) -> Result<Vec<Fragment>, OperationError> {
    let mut fragments = Vec::new();
    let mut total = canonical(&input.config)
        .map(|value| value.len())
        .unwrap_or(0);
    for kind in selected {
        let state = states.get(kind).ok_or_else(|| {
            OperationError::new(
                ErrorCode::ConfigInvalid,
                "Agent target paths are unavailable",
            )
        })?;
        if !safe_display_path(&state.path)
            || (!state.auth_path.is_empty() && !safe_display_path(&state.auth_path))
        {
            return Err(OperationError::new(
                ErrorCode::ConfigInvalid,
                "Agent target path contains unsafe characters",
            ));
        }
        let rendered = match kind {
            Agent::Claude => {
                let content = render_claude_fragment(
                    input.config.claude.as_ref().ok_or_else(render_failed)?,
                    &input.router_base_url,
                    &input.key,
                )?;
                vec![Fragment {
                    agent: *kind,
                    role: "config".into(),
                    path: state.path.clone(),
                    format: Format::Json,
                    content: String::from_utf8_lossy(&content).into_owned(),
                }]
            }
            Agent::OpenCode => {
                let content = render_opencode_fragment(
                    input.config.opencode.as_ref().ok_or_else(render_failed)?,
                    &input.api_base_url,
                    &input.key,
                )?;
                vec![Fragment {
                    agent: *kind,
                    role: "config".into(),
                    path: state.path.clone(),
                    format: Format::Json,
                    content: String::from_utf8_lossy(&content).into_owned(),
                }]
            }
            Agent::Codex => {
                let config_content = render_codex_fragment(
                    input.config.codex.as_ref().ok_or_else(render_failed)?,
                    &input.api_base_url,
                )?;
                let auth_content = render_codex_auth_fragment(&input.key, None)?;
                vec![
                    Fragment {
                        agent: *kind,
                        role: "config".into(),
                        path: state.path.clone(),
                        format: Format::Toml,
                        content: String::from_utf8_lossy(&config_content).into_owned(),
                    },
                    Fragment {
                        agent: *kind,
                        role: "auth".into(),
                        path: state.auth_path.clone(),
                        format: Format::Json,
                        content: String::from_utf8_lossy(&auth_content).into_owned(),
                    },
                ]
            }
        };
        for fragment in rendered {
            total += fragment.path.len() + fragment.content.len();
            if total > MAX_RENDER_SIZE {
                return Err(OperationError::new(
                    ErrorCode::ModelConfigInvalid,
                    "Agent render output exceeds the size limit",
                ));
            }
            fragments.push(fragment);
        }
    }
    Ok(fragments)
}

pub fn render_claude_fragment(
    config: &ClaudeConfig,
    router_base_url: &str,
    key: &str,
) -> Result<Vec<u8>, OperationError> {
    let env = claude_managed_env(config, router_base_url, key);
    marshal_indented_json(&json!({ "env": env }))
}

pub fn merge_claude(
    root: &Map<String, Value>,
    config: &ClaudeConfig,
    router_base_url: &str,
    key: &str,
    obsolete_owned_keys: &[String],
) -> Result<Vec<u8>, OperationError> {
    let mut result = root.clone();
    let mut env = match result.get("env") {
        Some(value) if !value.is_null() => value.as_object().cloned().ok_or_else(|| {
            OperationError::new(
                ErrorCode::ConfigInvalid,
                "Claude Code env setting is not an object",
            )
        })?,
        _ => Map::new(),
    };
    for key in CLAUDE_UNCONDITIONAL_ENV_KEYS {
        env.remove(key);
    }
    for key in claude_owned_env_keys(config) {
        env.remove(&key);
    }
    for key in obsolete_owned_keys {
        env.remove(key);
    }
    for (name, value) in claude_managed_env(config, router_base_url, key) {
        env.insert(name, Value::String(value));
    }
    result.insert("env".into(), Value::Object(env));
    marshal_object(&result)
}

pub fn claude_owned_env_keys(config: &ClaudeConfig) -> Vec<String> {
    let mut owned: Vec<String> = CLAUDE_UNCONDITIONAL_ENV_KEYS
        .iter()
        .map(|value| (*value).to_owned())
        .collect();
    if config.context_window.is_some() {
        owned.push("CLAUDE_CODE_MAX_CONTEXT_TOKENS".into());
    }
    if config.max_output_tokens.is_some() {
        owned.push("CLAUDE_CODE_MAX_OUTPUT_TOKENS".into());
    }
    if config.fable.is_some() {
        owned.push("ANTHROPIC_DEFAULT_FABLE_MODEL".into());
        owned.push("ANTHROPIC_DEFAULT_FABLE_MODEL_NAME".into());
    }
    owned.extend(config.extra.keys().cloned());
    owned
}

fn claude_managed_env(
    config: &ClaudeConfig,
    router_base_url: &str,
    key: &str,
) -> HashMap<String, String> {
    let mut result = HashMap::from([
        ("ANTHROPIC_BASE_URL".into(), router_base_url.to_owned()),
        ("ANTHROPIC_AUTH_TOKEN".into(), key.to_owned()),
        (
            "ANTHROPIC_MODEL".into(),
            claude_model_value(&config.primary),
        ),
        (
            "ANTHROPIC_CUSTOM_MODEL_OPTION".into(),
            claude_model_value(&config.primary),
        ),
        ("ENABLE_TOOL_SEARCH".into(), "true".into()),
        ("DISABLE_AUTOUPDATER".into(), "1".into()),
    ]);
    if let Some(name) = &config.primary.name {
        result.insert("ANTHROPIC_CUSTOM_MODEL_OPTION_NAME".into(), name.clone());
    }
    for (role, selection) in [
        ("HAIKU", &config.haiku),
        ("SONNET", &config.sonnet),
        ("OPUS", &config.opus),
    ] {
        let (model, name) = role_model(selection, &config.primary);
        result.insert(format!("ANTHROPIC_DEFAULT_{role}_MODEL"), model);
        if let Some(name) = name {
            result.insert(format!("ANTHROPIC_DEFAULT_{role}_MODEL_NAME"), name);
        }
    }
    if let Some(fable) = &config.fable {
        let (model, name) = role_model(fable, &config.primary);
        result.insert("ANTHROPIC_DEFAULT_FABLE_MODEL".into(), model);
        if let Some(name) = name {
            result.insert("ANTHROPIC_DEFAULT_FABLE_MODEL_NAME".into(), name);
        }
    }
    if let Some(window) = config.context_window {
        result.insert("CLAUDE_CODE_MAX_CONTEXT_TOKENS".into(), window.to_string());
    }
    if let Some(tokens) = config.max_output_tokens {
        result.insert("CLAUDE_CODE_MAX_OUTPUT_TOKENS".into(), tokens.to_string());
    }
    result.extend(config.extra.clone());
    result
}

fn role_model(role: &ClaudeRole, primary: &Model) -> (String, Option<String>) {
    if let Some(selection) = &role.selection {
        (claude_model_value(selection), selection.name.clone())
    } else {
        (claude_model_value(primary), primary.name.clone())
    }
}

fn claude_model_value(selection: &Model) -> String {
    if selection.context.is_some() {
        format!("{}[1m]", selection.model)
    } else {
        selection.model.clone()
    }
}

pub fn render_opencode_fragment(
    config: &OpenCodeConfig,
    api_base_url: &str,
    key: &str,
) -> Result<Vec<u8>, OperationError> {
    let provider = opencode_provider(config, api_base_url, key);
    marshal_indented_json(&json!({
        "model": format!("mtls-router/{}", config.default_model),
        "provider": { "mtls-router": provider },
    }))
}

pub fn merge_opencode(
    root: &Map<String, Value>,
    config: &OpenCodeConfig,
    api_base_url: &str,
    key: &str,
    own_root_model: bool,
) -> Result<Vec<u8>, OperationError> {
    let mut result = root.clone();
    let mut providers = match result.get("provider") {
        Some(value) if !value.is_null() => value.as_object().cloned().ok_or_else(|| {
            OperationError::new(
                ErrorCode::ConfigInvalid,
                "opencode provider field is not an object",
            )
        })?,
        _ => Map::new(),
    };
    providers.insert(
        "mtls-router".into(),
        opencode_provider(config, api_base_url, key),
    );
    result.insert("provider".into(), Value::Object(providers));
    if own_root_model || !result.contains_key("model") {
        result.insert(
            "model".into(),
            Value::String(format!("mtls-router/{}", config.default_model)),
        );
    }
    marshal_object(&result)
}

fn opencode_provider(config: &OpenCodeConfig, api_base_url: &str, key: &str) -> Value {
    let mut models = Map::new();
    for (id, model) in &config.models {
        let mut entry = Map::from_iter([(
            "name".into(),
            Value::String(model.name.clone().unwrap_or_else(|| id.clone())),
        )]);
        insert_bool(&mut entry, "reasoning", model.reasoning);
        insert_bool(&mut entry, "attachment", model.attachment);
        insert_bool(&mut entry, "tool_call", model.tool_call);
        insert_bool(&mut entry, "temperature", model.temperature);
        if let Some(limit) = &model.limit {
            let mut object = Map::from_iter([
                ("context".into(), json!(limit.context)),
                ("output".into(), json!(limit.output)),
            ]);
            if let Some(input) = limit.input {
                object.insert("input".into(), json!(input));
            }
            entry.insert("limit".into(), Value::Object(object));
        }
        if let Some(modalities) = &model.modalities {
            let mut object = Map::new();
            if !modalities.input.is_empty() {
                object.insert("input".into(), json!(modalities.input));
            }
            if !modalities.output.is_empty() {
                object.insert("output".into(), json!(modalities.output));
            }
            entry.insert("modalities".into(), Value::Object(object));
        }
        if let Some(interleaved) = &model.interleaved {
            entry.insert(
                "interleaved".into(),
                match interleaved {
                    Interleaved::Enabled => Value::Bool(true),
                    Interleaved::Field(field) => json!({ "field": field }),
                },
            );
        }
        if let Some(options) = &model.options {
            entry.insert(
                "options".into(),
                json_from_value(&JsonValue::Object(options.clone())),
            );
        }
        if let Some(variants) = &model.variants {
            let mut object = Map::new();
            for (name, options) in variants {
                object.insert(
                    name.clone(),
                    json_from_value(&JsonValue::Object(options.clone())),
                );
            }
            entry.insert("variants".into(), Value::Object(object));
        }
        let merged = if let Some(extra) = &model.extra {
            let extra_map = extra
                .iter()
                .map(|(key, value)| (key.clone(), json_from_model(value)))
                .collect();
            let overlay = entry
                .iter()
                .map(|(key, value)| (key.clone(), json_from_serde(value)))
                .collect();
            let merged = deep_merge(&extra_map, &overlay);
            json_from_value(&JsonValue::Object(merged))
        } else {
            Value::Object(entry)
        };
        models.insert(id.clone(), merged);
    }
    json!({
        "npm": "@ai-sdk/openai-compatible",
        "name": PROVIDER_DISPLAY_NAME,
        "options": { "baseURL": api_base_url, "apiKey": key },
        "models": models,
    })
}

pub fn render_codex_fragment(
    config: &CodexConfig,
    api_base_url: &str,
) -> Result<Vec<u8>, OperationError> {
    encode_toml(&codex_managed_table(config, api_base_url)).map_err(|_| render_failed())
}

pub fn render_codex_auth_fragment(
    key: &str,
    existing: Option<&Map<String, Value>>,
) -> Result<Vec<u8>, OperationError> {
    let mut result = existing.cloned().unwrap_or_default();
    for name in CODEX_COMPETING_AUTH_KEYS {
        result.remove(name);
    }
    result.insert("auth_mode".into(), json!("apikey"));
    result.insert("OPENAI_API_KEY".into(), json!(key));
    marshal_object(&result)
}

pub fn merge_codex(
    content: &[u8],
    config: &CodexConfig,
    api_base_url: &str,
    owned_optional: &[String],
    migrate_historical: bool,
) -> Result<Vec<u8>, OperationError> {
    let mut root = if content.is_empty() {
        toml::Table::new()
    } else {
        super::toml::decode_toml(content).ok_or_else(|| {
            OperationError::new(
                ErrorCode::ConfigInvalid,
                "Codex configuration is invalid TOML",
            )
        })?
    };
    for key in CODEX_REQUIRED_ROOT_KEYS {
        root.remove(key);
    }
    for key in owned_optional {
        root.remove(key);
    }
    if migrate_historical {
        root.remove("disable_response_storage");
    }
    let mut providers = root
        .get("model_providers")
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default();
    if migrate_historical {
        providers.remove("custom");
    }
    let mut managed = codex_managed_table(config, api_base_url);
    if let Some(toml::Value::Table(managed_providers)) = managed.remove("model_providers") {
        if let Some(provider) = managed_providers.get("mtls-router") {
            providers.insert("mtls-router".into(), provider.clone());
        }
    }
    root.insert("model_providers".into(), toml::Value::Table(providers));
    for (key, value) in managed {
        root.insert(key, value);
    }
    encode_toml(&root).map_err(|_| render_failed())
}

pub fn assess_codex_merge(
    config: &toml::Table,
    auth: &Map<String, Value>,
) -> Result<CodexMergeAssessment, OperationError> {
    if config
        .get("forced_login_method")
        .and_then(toml::Value::as_str)
        == Some("chatgpt")
    {
        return Err(OperationError::new(
            ErrorCode::CodexAuthUnsupported,
            "Codex policy requires ChatGPT authentication",
        ));
    }
    let mut assessment = CodexMergeAssessment {
        historical_migration: exact_historical_codex(config, auth),
        ..CodexMergeAssessment::default()
    };
    if let Some(provider) = toml_table(config, &["model_providers", "mtls-router"]) {
        let name = provider
            .get("name")
            .and_then(toml::Value::as_str)
            .unwrap_or("");
        assessment.managed_config_collision = !is_managed_provider_display_name(name)
            || provider.get("wire_api").and_then(toml::Value::as_str) != Some("responses")
            || provider
                .get("requires_openai_auth")
                .and_then(toml::Value::as_bool)
                != Some(true)
            || !provider
                .get("base_url")
                .and_then(toml::Value::as_str)
                .is_some_and(valid_api_value);
    }
    let mode = raw_string(auth.get("auth_mode"));
    assessment.auth_collision =
        mode.as_deref() != Some("apikey") || !has_nonempty_json_string(auth.get("OPENAI_API_KEY"));
    for key in CODEX_COMPETING_AUTH_KEYS {
        if auth.contains_key(key) {
            assessment.auth_collision = true;
        }
    }
    if let Some(store) = config
        .get("cli_auth_credentials_store")
        .and_then(toml::Value::as_str)
    {
        if store != "file" {
            assessment.auth_collision = true;
        }
    }
    assessment.requires_auth_approval =
        assessment.auth_collision || assessment.historical_migration;
    Ok(assessment)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CodexMergeAssessment {
    pub managed_config_collision: bool,
    pub auth_collision: bool,
    pub requires_auth_approval: bool,
    pub historical_migration: bool,
}

pub fn codex_managed_config_keys(config: &CodexConfig, api_base_url: &str) -> Vec<String> {
    codex_managed_table(config, api_base_url)
        .keys()
        .cloned()
        .collect()
}

fn codex_managed_table(config: &CodexConfig, api_base_url: &str) -> toml::Table {
    let mut provider = toml::Table::new();
    provider.insert(
        "name".into(),
        toml::Value::String(PROVIDER_DISPLAY_NAME.into()),
    );
    provider.insert("wire_api".into(), toml::Value::String("responses".into()));
    provider.insert("requires_openai_auth".into(), toml::Value::Boolean(true));
    provider.insert("base_url".into(), toml::Value::String(api_base_url.into()));
    let mut providers = toml::Table::new();
    providers.insert("mtls-router".into(), toml::Value::Table(provider));
    let mut result = toml::Table::new();
    result.insert(
        "model_provider".into(),
        toml::Value::String("mtls-router".into()),
    );
    result.insert("model".into(), toml::Value::String(config.model.clone()));
    result.insert(
        "cli_auth_credentials_store".into(),
        toml::Value::String("file".into()),
    );
    result.insert("model_providers".into(), toml::Value::Table(providers));
    if let Some(value) = &config.reasoning_effort {
        result.insert(
            "model_reasoning_effort".into(),
            toml::Value::String(value.clone()),
        );
    }
    if let Some(value) = &config.reasoning_summary {
        result.insert(
            "model_reasoning_summary".into(),
            toml::Value::String(value.clone()),
        );
    }
    if let Some(value) = &config.verbosity {
        result.insert("model_verbosity".into(), toml::Value::String(value.clone()));
    }
    if let Some(value) = config.context_window {
        result.insert("model_context_window".into(), toml::Value::Integer(value));
    }
    if let Some(value) = config.auto_compact_token_limit {
        result.insert(
            "model_auto_compact_token_limit".into(),
            toml::Value::Integer(value),
        );
    }
    for (key, value) in &config.extra {
        result.insert(key.clone(), toml_from_json(value));
    }
    result
}

fn toml_from_json(value: &JsonValue) -> toml::Value {
    match value {
        JsonValue::String(text) => toml::Value::String(text.clone()),
        JsonValue::Number(number) => number
            .as_str()
            .parse::<i64>()
            .map(toml::Value::Integer)
            .or_else(|_| number.as_str().parse::<f64>().map(toml::Value::Float))
            .unwrap_or_else(|_| toml::Value::String(number.as_str().to_owned())),
        JsonValue::Bool(flag) => toml::Value::Boolean(*flag),
        JsonValue::Array(items) => toml::Value::Array(items.iter().map(toml_from_json).collect()),
        JsonValue::Object(object) => toml::Value::Table(
            object
                .iter()
                .map(|(key, value)| (key.clone(), toml_from_json(value)))
                .collect(),
        ),
        JsonValue::Null => toml::Value::String(String::new()),
    }
}

#[derive(Clone, Debug, Default)]
pub struct CleanupTransform {
    pub content: Vec<u8>,
    pub delete: bool,
    pub removed_paths: Vec<String>,
}

pub fn cleanup_claude(
    root: &Map<String, Value>,
    owned_paths: &[String],
) -> Result<CleanupTransform, OperationError> {
    let mut result = root.clone();
    let Some(raw_env) = result.get("env").cloned() else {
        return Ok(new_cleanup_transform(
            marshal_cleanup_json(&Value::Object(result.clone()))?,
            result.is_empty(),
            Vec::new(),
        ));
    };
    let mut env = raw_env.as_object().cloned().ok_or_else(|| {
        OperationError::new(
            ErrorCode::ConfigInvalid,
            "Claude Code env setting is not an object",
        )
    })?;
    let mut removed = Vec::new();
    for path in owned_paths {
        if let Some(key) = path.strip_prefix("env.") {
            if env.remove(key).is_some() {
                removed.push(path.clone());
            }
        }
    }
    if env.is_empty() {
        result.remove("env");
    } else {
        result.insert("env".into(), Value::Object(env));
    }
    Ok(new_cleanup_transform(
        marshal_cleanup_json(&Value::Object(result.clone()))?,
        result.is_empty(),
        removed,
    ))
}

pub fn cleanup_opencode(
    root: &Map<String, Value>,
    owns_root_model: bool,
) -> Result<CleanupTransform, OperationError> {
    let mut result = root.clone();
    let mut removed = Vec::new();
    if let Some(raw) = result.get("provider").cloned() {
        let mut providers = raw.as_object().cloned().ok_or_else(|| {
            OperationError::new(
                ErrorCode::ConfigInvalid,
                "opencode provider field is not an object",
            )
        })?;
        if let Some(managed) = providers.get("mtls-router") {
            if !managed.is_object() {
                return Err(OperationError::new(
                    ErrorCode::ConfigInvalid,
                    "opencode managed provider is not an object",
                ));
            }
            providers.remove("mtls-router");
            removed.push("provider.mtls-router".into());
        }
        if providers.is_empty() {
            result.remove("provider");
        } else {
            result.insert("provider".into(), Value::Object(providers));
        }
    }
    if owns_root_model {
        if result
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(|value| value.starts_with("mtls-router/"))
        {
            result.remove("model");
            removed.push("model".into());
        }
    }
    Ok(new_cleanup_transform(
        marshal_cleanup_json(&Value::Object(result.clone()))?,
        result.is_empty(),
        removed,
    ))
}

pub fn cleanup_codex_config(
    content: &[u8],
    saved: &CodexConfig,
) -> Result<CleanupTransform, OperationError> {
    let mut root = super::toml::decode_toml(content).ok_or_else(|| {
        OperationError::new(
            ErrorCode::ConfigInvalid,
            "Codex configuration is invalid TOML",
        )
    })?;
    let managed = codex_managed_config_keys(saved, "");
    let mut removed = Vec::new();
    for key in CODEX_REQUIRED_ROOT_KEYS {
        if root.remove(key).is_some() {
            removed.push(key.to_owned());
        }
    }
    for key in &managed {
        if key == "model_providers" || CODEX_REQUIRED_ROOT_KEYS.contains(&key.as_str()) {
            continue;
        }
        if root.remove(key).is_some() {
            removed.push(key.clone());
        }
    }
    if let Some(value) = root.get("model_providers").cloned() {
        let mut providers = value.as_table().cloned().ok_or_else(|| {
            OperationError::new(
                ErrorCode::ConfigInvalid,
                "Codex model providers setting is not a table",
            )
        })?;
        if let Some(provider) = providers.get("mtls-router") {
            if !provider.is_table() {
                return Err(OperationError::new(
                    ErrorCode::ConfigInvalid,
                    "Codex managed provider is not a table",
                ));
            }
            providers.remove("mtls-router");
            removed.push("model_providers.mtls-router".into());
        }
        if providers.is_empty() {
            root.remove("model_providers");
        } else {
            root.insert("model_providers".into(), toml::Value::Table(providers));
        }
    }
    let output = encode_toml(&root).map_err(|_| render_failed())?;
    Ok(new_cleanup_transform(output, root.is_empty(), removed))
}

pub fn cleanup_codex_auth(root: &Map<String, Value>) -> Result<CleanupTransform, OperationError> {
    let mut result = root.clone();
    let mut removed = Vec::new();
    for key in ["auth_mode", "OPENAI_API_KEY"] {
        if result.remove(key).is_some() {
            removed.push(format!("auth.{key}"));
        }
    }
    Ok(new_cleanup_transform(
        marshal_cleanup_json(&Value::Object(result.clone()))?,
        result.is_empty(),
        removed,
    ))
}

fn new_cleanup_transform(
    content: Vec<u8>,
    empty: bool,
    mut removed: Vec<String>,
) -> CleanupTransform {
    removed.sort();
    removed.dedup();
    CleanupTransform {
        content: if empty { Vec::new() } else { content },
        delete: empty,
        removed_paths: removed,
    }
}

fn marshal_cleanup_json(value: &Value) -> Result<Vec<u8>, OperationError> {
    let mut encoded = serde_json::to_vec(value).map_err(|_| render_failed())?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn marshal_indented_json(value: &Value) -> Result<Vec<u8>, OperationError> {
    let mut encoded = serde_json::to_string_pretty(value).map_err(|_| render_failed())?;
    encoded.push('\n');
    Ok(encoded.into_bytes())
}

fn marshal_object(value: &Map<String, Value>) -> Result<Vec<u8>, OperationError> {
    marshal_indented_json(&Value::Object(value.clone()))
}

fn insert_bool(object: &mut Map<String, Value>, key: &str, value: Option<bool>) {
    if let Some(value) = value {
        object.insert(key.into(), Value::Bool(value));
    }
}

fn json_from_value(value: &JsonValue) -> Value {
    match value {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(flag) => Value::Bool(*flag),
        JsonValue::String(text) => Value::String(text.clone()),
        JsonValue::Number(number) => serde_json::from_str(number.as_str()).unwrap_or(Value::Null),
        JsonValue::Array(items) => Value::Array(items.iter().map(json_from_value).collect()),
        JsonValue::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), json_from_value(value)))
                .collect(),
        ),
    }
}

fn json_from_model(value: &JsonValue) -> JsonValue {
    value.clone()
}

fn json_from_serde(value: &Value) -> JsonValue {
    match value {
        Value::Null => JsonValue::Null,
        Value::Bool(flag) => JsonValue::Bool(*flag),
        Value::String(text) => JsonValue::String(text.clone()),
        Value::Number(number) => {
            JsonValue::Number(super::modelconfig::JsonNumber(number.to_string()))
        }
        Value::Array(items) => JsonValue::Array(items.iter().map(json_from_serde).collect()),
        Value::Object(object) => JsonValue::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), json_from_serde(value)))
                .collect(),
        ),
    }
}

fn render_failed() -> OperationError {
    OperationError::new(
        ErrorCode::ModelConfigInvalid,
        "Agent managed fragment could not be rendered",
    )
}

pub fn same_model_agents(selected: &[Agent], claims: &[Agent]) -> bool {
    selected == claims
}

pub fn api_base_url(router_base_url: &str) -> Result<String, OperationError> {
    api_url(router_base_url).map_err(|_| {
        OperationError::new(
            ErrorCode::ModelCatalogStale,
            "model catalog router address is invalid",
        )
    })
}

pub fn path_only_state(
    agent: Agent,
    path: PathBuf,
    auth_path: Option<PathBuf>,
    format: Format,
) -> State {
    State {
        agent,
        name: String::new(),
        detected: false,
        command: String::new(),
        path: path.to_string_lossy().into_owned(),
        auth_path: auth_path
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default(),
        format,
        exists: false,
        writable: false,
        configured: false,
        invalid: false,
        migratable: false,
        recovery: Default::default(),
        cleanup: Default::default(),
        path_overridden: false,
    }
}

#[cfg(test)]
mod tests {
    use super::super::detect::decode_object;
    use super::super::modelconfig::{ClaudeContext, ClaudeRole, Model};
    use super::*;

    fn sample_claude() -> ClaudeConfig {
        ClaudeConfig {
            primary: Model {
                model: "model-primary".into(),
                name: Some("Primary".into()),
                context: None,
            },
            fable: None,
            haiku: ClaudeRole::inherit(),
            sonnet: ClaudeRole::selected(Model {
                model: "model-sonnet".into(),
                name: None,
                context: Some(ClaudeContext::OneMillion),
            }),
            opus: ClaudeRole::inherit(),
            context_window: None,
            max_output_tokens: None,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn claude_merge_preserves_unrelated_values() {
        let root = decode_object(
            br#"{"theme":"dark","env":{"OTHER":{"nested":true},"ANTHROPIC_CUSTOM_MODEL_OPTION_NAME":"stale","UNOWNED":"keep"}}"#,
        )
        .unwrap();
        let content = merge_claude(
            &root,
            &sample_claude(),
            "http://127.0.0.1:19443",
            "secret",
            &[],
        )
        .unwrap();
        let merged = decode_object(&content).unwrap();
        assert_eq!(merged["theme"], json!("dark"));
        let env = merged["env"].as_object().unwrap();
        assert_eq!(env["UNOWNED"], json!("keep"));
        assert!(env.get("OTHER").is_some());
        assert_eq!(env["ANTHROPIC_AUTH_TOKEN"], json!("secret"));
        assert_eq!(
            env["ANTHROPIC_DEFAULT_SONNET_MODEL"],
            json!("model-sonnet[1m]")
        );
    }

    #[test]
    fn fragments_are_redacted_and_file_independent() {
        let config = Config {
            version: 1,
            claude: Some(sample_claude()),
            opencode: None,
            codex: None,
        };
        let input = RenderInput {
            config,
            router_base_url: "http://127.0.0.1:19443".into(),
            api_base_url: "http://127.0.0.1:19443/v1".into(),
            key: REDACTED_API_KEY.into(),
            own_root_model: true,
        };
        let mut states = HashMap::new();
        // Must be absolute on every platform: `safe_display_path` rejects a
        // Unix-style root on Windows.
        states.insert(
            Agent::Claude,
            path_only_state(
                Agent::Claude,
                std::env::temp_dir().join(".claude").join("settings.json"),
                None,
                Format::Json,
            ),
        );
        let fragments = render_fragments(&[Agent::Claude], &states, &input).unwrap();
        assert_eq!(fragments.len(), 1);
        assert!(fragments[0].content.contains(REDACTED_API_KEY));
        assert!(fragments[0].content.contains("http://127.0.0.1:19443"));
        assert!(!fragments[0].content.contains("secret-canary"));
    }
}
