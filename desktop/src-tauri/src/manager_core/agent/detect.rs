use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Map, Value};

use super::modelconfig::{self, Agent, JsonNumber, JsonValue};
use super::paths::{claude_paths, codex_paths, opencode_paths, Paths};
use super::recovery::{
    finalize_recovery, has_blocking_file_reason, has_legacy_invalid_file_reason,
    inspect_recovery_target, path_writable,
};
use super::toml::{decode_toml, loopback_router_url, toml_string, toml_table};
use super::types::{
    append_reason, is_managed_provider_display_name, CleanupState, Format, RecoveryReason, State,
    MAX_CONFIG_SIZE,
};
use super::url::{valid_api_value, valid_router_base_url};

#[derive(Clone, Debug)]
pub struct Detector {
    pub home_dir: Option<PathBuf>,
    pub getenv: fn(&str) -> String,
}

impl Default for Detector {
    fn default() -> Self {
        Self {
            home_dir: None,
            getenv: |key| std::env::var(key).unwrap_or_default(),
        }
    }
}

impl Detector {
    pub fn isolated(home: impl Into<PathBuf>) -> Self {
        Self {
            home_dir: Some(home.into()),
            getenv: |_| String::new(),
        }
    }

    pub fn detect(&self) -> Result<Vec<State>, String> {
        let home = match &self.home_dir {
            Some(home) => home.clone(),
            None => dirs::home_dir().ok_or_else(|| "resolve user home directory".to_owned())?,
        };
        let getenv = self.getenv;
        let claude = claude_paths(&home, &getenv("CLAUDE_CONFIG_DIR"));
        let opencode_override = getenv("OPENCODE_CONFIG");
        let opencode = opencode_paths(&home, &opencode_override);
        let codex = codex_paths(&home, &getenv("CODEX_HOME"));
        let claude_state = inspect_json_state(
            Agent::Claude,
            "Claude Code",
            &claude,
            Format::Json,
            inspect_claude,
        );
        let opencode_format = if opencode
            .config_path
            .extension()
            .and_then(|value| value.to_str())
            == Some("jsonc")
        {
            Format::Jsonc
        } else {
            Format::Json
        };
        let mut opencode_state = inspect_json_state(
            Agent::OpenCode,
            "opencode",
            &opencode,
            opencode_format,
            inspect_opencode,
        );
        opencode_state.path_overridden = !opencode_override.is_empty();
        let codex_state = inspect_codex(&codex);
        Ok(vec![claude_state, opencode_state, codex_state])
    }

    pub fn target_states(&self) -> Result<HashMap<Agent, State>, String> {
        let home = match &self.home_dir {
            Some(home) => home.clone(),
            None => dirs::home_dir().ok_or_else(|| "resolve user home directory".to_owned())?,
        };
        let getenv = self.getenv;
        let claude = claude_paths(&home, &getenv("CLAUDE_CONFIG_DIR"));
        let opencode = opencode_paths(&home, &getenv("OPENCODE_CONFIG"));
        let codex = codex_paths(&home, &getenv("CODEX_HOME"));
        let format = if opencode
            .config_path
            .extension()
            .and_then(|value| value.to_str())
            == Some("jsonc")
        {
            Format::Jsonc
        } else {
            Format::Json
        };
        Ok(HashMap::from([
            (
                Agent::Claude,
                path_state(Agent::Claude, claude.config_path, None, Format::Json),
            ),
            (
                Agent::OpenCode,
                path_state(Agent::OpenCode, opencode.config_path, None, format),
            ),
            (
                Agent::Codex,
                path_state(
                    Agent::Codex,
                    codex.config_path,
                    codex.auth_path,
                    Format::Toml,
                ),
            ),
        ]))
    }
}

fn path_state(agent: Agent, path: PathBuf, auth_path: Option<PathBuf>, format: Format) -> State {
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
        cleanup: CleanupState::default(),
        path_overridden: false,
    }
}

fn inspect_json_state(
    kind: Agent,
    name: &str,
    paths: &Paths,
    format: Format,
    inspect: fn(&Map<String, Value>) -> (bool, bool),
) -> State {
    let mut state = base_state(kind, name, paths, format);
    let file = &mut state.recovery.files[0];
    if !file.reasons.is_empty() {
        state.invalid = has_legacy_invalid_file_reason(&file.reasons);
        finalize_recovery(&mut state);
        return state;
    }
    if !state.exists {
        finalize_recovery(&mut state);
        return state;
    }
    let content = match read_recovery_config(&paths.config_path) {
        Ok(content) => content,
        Err(reason) => {
            state.invalid = true;
            append_reason(&mut state.recovery.files[0].reasons, reason);
            finalize_recovery(&mut state);
            return state;
        }
    };
    let content = if format == Format::Jsonc {
        match strip_jsonc(&content) {
            Ok(content) => content,
            Err(()) => {
                state.invalid = true;
                append_reason(
                    &mut state.recovery.files[0].reasons,
                    RecoveryReason::SyntaxInvalid,
                );
                finalize_recovery(&mut state);
                return state;
            }
        }
    } else {
        content
    };
    let root = match decode_recovery_object(&content) {
        Ok(root) => root,
        Err(reason) => {
            state.invalid = true;
            append_reason(&mut state.recovery.files[0].reasons, reason);
            finalize_recovery(&mut state);
            return state;
        }
    };
    let (configured, invalid) = inspect(&root);
    state.configured = configured;
    state.invalid = invalid;
    if state.invalid {
        append_reason(
            &mut state.recovery.files[0].reasons,
            RecoveryReason::UnsupportedStructure,
        );
    } else if kind == Agent::Claude {
        if let Some(env) = root.get("env") {
            if !env.is_object() {
                append_reason(
                    &mut state.recovery.files[0].reasons,
                    RecoveryReason::UnsupportedStructure,
                );
            }
        }
    }
    finalize_recovery(&mut state);
    state
}

fn base_state(kind: Agent, name: &str, paths: &Paths, format: Format) -> State {
    let config_file = inspect_recovery_target("config", &paths.config_path, format);
    let exists = config_file.exists;
    let mut writable = path_writable(&paths.config_path);
    let mut files = vec![config_file];
    if let Some(auth_path) = &paths.auth_path {
        writable = writable && path_writable(auth_path);
        files.push(inspect_recovery_target("auth", auth_path, Format::Json));
    }
    State {
        agent: kind,
        name: name.to_owned(),
        detected: true,
        command: String::new(),
        path: paths.config_path.to_string_lossy().into_owned(),
        auth_path: paths
            .auth_path
            .as_ref()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default(),
        format,
        exists,
        writable,
        configured: false,
        invalid: false,
        migratable: false,
        recovery: super::types::RecoveryState {
            eligible: false,
            reasons: Vec::new(),
            files,
        },
        cleanup: CleanupState::default(),
        path_overridden: false,
    }
}

fn inspect_claude(root: &Map<String, Value>) -> (bool, bool) {
    let Some(env) = root.get("env").and_then(Value::as_object) else {
        return (false, false);
    };
    let Some(base) = raw_string(env.get("ANTHROPIC_BASE_URL")) else {
        return (false, false);
    };
    if !valid_router_base_url(&base) || !has_nonempty_json_string(env.get("ANTHROPIC_AUTH_TOKEN")) {
        return (false, false);
    }
    for key in [
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
    ] {
        if !has_nonempty_json_string(env.get(key)) {
            return (false, false);
        }
    }
    (true, false)
}

fn inspect_opencode(root: &Map<String, Value>) -> (bool, bool) {
    let Some(provider_raw) = root.get("provider") else {
        return (false, false);
    };
    if provider_raw.is_null() {
        return (false, false);
    }
    let Some(providers) = provider_raw.as_object() else {
        return (false, true);
    };
    let Some(router) = providers.get("mtls-router").and_then(Value::as_object) else {
        return (false, false);
    };
    let Some(options) = router.get("options").and_then(Value::as_object) else {
        return (false, false);
    };
    let Some(base) = raw_string(options.get("baseURL")) else {
        return (false, false);
    };
    if !valid_api_value(&base) || !has_nonempty_json_string(options.get("apiKey")) {
        return (false, false);
    }
    let Some(name) = raw_string(router.get("name")) else {
        return (false, false);
    };
    if !json_string_equals(router.get("npm"), "@ai-sdk/openai-compatible")
        || !is_managed_provider_display_name(&name)
    {
        return (false, false);
    }
    let Some(models) = router.get("models").and_then(Value::as_object) else {
        return (false, false);
    };
    if models.is_empty() {
        return (false, false);
    }
    let Some(root_model) = raw_string(root.get("model")) else {
        return (false, false);
    };
    let Some(id) = root_model.strip_prefix("mtls-router/") else {
        return (false, false);
    };
    (models.contains_key(id), false)
}

fn inspect_codex(paths: &Paths) -> State {
    let mut state = base_state(Agent::Codex, "Codex", paths, Format::Toml);
    let mut auth_configured = false;
    let mut auth_root = Map::new();
    if state.recovery.files[1].exists && !has_blocking_file_reason(&state.recovery.files[1].reasons)
    {
        match read_recovery_config(paths.auth_path.as_ref().expect("codex auth")) {
            Ok(content) => match decode_recovery_object(&content) {
                Ok(root) => {
                    auth_configured = json_string_equals(root.get("auth_mode"), "apikey")
                        && has_nonempty_json_string(root.get("OPENAI_API_KEY"));
                    auth_root = root;
                }
                Err(reason) => {
                    state.invalid = true;
                    append_reason(&mut state.recovery.files[1].reasons, reason);
                }
            },
            Err(reason) => {
                state.invalid = true;
                append_reason(&mut state.recovery.files[1].reasons, reason);
            }
        }
    }
    if !state.exists {
        state.invalid = state.invalid
            || has_legacy_invalid_file_reason(&state.recovery.files[0].reasons)
            || has_legacy_invalid_file_reason(&state.recovery.files[1].reasons);
        finalize_recovery(&mut state);
        return state;
    }
    if has_blocking_file_reason(&state.recovery.files[0].reasons) {
        state.invalid = has_legacy_invalid_file_reason(&state.recovery.files[0].reasons);
        finalize_recovery(&mut state);
        return state;
    }
    let content = match read_recovery_config(&paths.config_path) {
        Ok(content) => content,
        Err(reason) => {
            state.invalid = true;
            append_reason(&mut state.recovery.files[0].reasons, reason);
            finalize_recovery(&mut state);
            return state;
        }
    };
    let Some(values) = decode_toml(&content) else {
        state.invalid = true;
        append_reason(
            &mut state.recovery.files[0].reasons,
            RecoveryReason::SyntaxInvalid,
        );
        finalize_recovery(&mut state);
        return state;
    };
    state.migratable = exact_historical_codex(&values, &auth_root);
    let provider = toml_table(&values, &["model_providers", "mtls-router"]);
    let model = toml_string(values.get("model"));
    let store = toml_string(values.get("cli_auth_credentials_store"));
    let provider_name = provider
        .and_then(|table| table.get("name"))
        .and_then(toml::Value::as_str);
    let provider_ok = provider.is_some_and(|table| {
        matches!(provider_name, Some(name) if is_managed_provider_display_name(name))
            && table.get("wire_api").and_then(toml::Value::as_str) == Some("responses")
            && table
                .get("requires_openai_auth")
                .and_then(toml::Value::as_bool)
                == Some(true)
            && table
                .get("base_url")
                .and_then(toml::Value::as_str)
                .is_some_and(valid_api_value)
    });
    state.configured = values.get("model_provider").and_then(toml::Value::as_str)
        == Some("mtls-router")
        && model.as_ref().is_some_and(|value| !value.is_empty())
        && store.as_deref() == Some("file")
        && provider_ok
        && auth_configured
        && valid_codex_model_settings(&values);
    state.invalid = state.invalid
        || has_legacy_invalid_file_reason(&state.recovery.files[0].reasons)
        || has_legacy_invalid_file_reason(&state.recovery.files[1].reasons);
    finalize_recovery(&mut state);
    state
}

pub(crate) fn exact_historical_codex(config: &toml::Table, auth: &Map<String, Value>) -> bool {
    let Some(provider) = toml_table(config, &["model_providers", "custom"]) else {
        return false;
    };
    if provider.len() != 4 {
        return false;
    }
    let name = provider.get("name").and_then(toml::Value::as_str);
    let wire = provider.get("wire_api").and_then(toml::Value::as_str);
    let requires = provider
        .get("requires_openai_auth")
        .and_then(toml::Value::as_bool);
    let base_ok = loopback_router_url(provider.get("base_url"), true);
    let model_provider = config.get("model_provider").and_then(toml::Value::as_str);
    let model = config.get("model").and_then(toml::Value::as_str);
    let disable_storage = config
        .get("disable_response_storage")
        .and_then(toml::Value::as_bool);
    let auth_mode = raw_string(auth.get("auth_mode"));
    name == Some("9router")
        && wire == Some("responses")
        && requires == Some(true)
        && base_ok
        && model_provider == Some("custom")
        && model == Some("gpt-5.5")
        && disable_storage == Some(true)
        && has_nonempty_json_string(auth.get("OPENAI_API_KEY"))
        && auth_mode.as_deref().is_none_or(|value| value == "apikey")
}

fn valid_codex_model_settings(values: &toml::Table) -> bool {
    let mut section = HashMap::new();
    if let Some(model) = values.get("model") {
        section.insert("model".to_owned(), toml_value_to_json(model));
    }
    for (key, canonical) in [
        ("model_reasoning_effort", "reasoning_effort"),
        ("model_reasoning_summary", "reasoning_summary"),
        ("model_verbosity", "verbosity"),
        ("model_context_window", "context_window"),
        ("model_auto_compact_token_limit", "auto_compact_token_limit"),
    ] {
        if let Some(value) = values.get(key) {
            section.insert(canonical.to_owned(), toml_value_to_json(value));
        }
    }
    if let Some(value) = values.get("model_auto_compact_token_limit_scope") {
        let mut extra = HashMap::new();
        extra.insert(
            "model_auto_compact_token_limit_scope".to_owned(),
            toml_value_to_json(value),
        );
        section.insert("extra".to_owned(), JsonValue::Object(extra));
    }
    let mut root = HashMap::new();
    root.insert(
        "version".to_owned(),
        JsonValue::Number(JsonNumber("1".into())),
    );
    root.insert("codex".to_owned(), JsonValue::Object(section));
    let Ok(doc) = modelconfig::marshal_jcs(&JsonValue::Object(root)) else {
        return false;
    };
    let Some(model) = values.get("model").and_then(toml::Value::as_str) else {
        return false;
    };
    modelconfig::decode(&doc, &[Agent::Codex], &[model.to_owned()]).is_ok()
}

fn toml_value_to_json(value: &toml::Value) -> JsonValue {
    match value {
        toml::Value::String(text) => JsonValue::String(text.clone()),
        toml::Value::Integer(number) => JsonValue::Number(JsonNumber(number.to_string())),
        toml::Value::Float(number) => JsonValue::Number(JsonNumber(number.to_string())),
        toml::Value::Boolean(flag) => JsonValue::Bool(*flag),
        toml::Value::Array(items) => {
            JsonValue::Array(items.iter().map(toml_value_to_json).collect())
        }
        toml::Value::Table(table) => JsonValue::Object(
            table
                .iter()
                .map(|(key, value)| (key.clone(), toml_value_to_json(value)))
                .collect(),
        ),
        toml::Value::Datetime(value) => JsonValue::String(value.to_string()),
    }
}

fn decode_recovery_object(content: &[u8]) -> Result<Map<String, Value>, RecoveryReason> {
    let content = content.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(content);
    let value = decode_json_value(content).ok_or(RecoveryReason::SyntaxInvalid)?;
    if !value.is_object() {
        return Err(RecoveryReason::UnsupportedStructure);
    }
    decode_object(content).ok_or(RecoveryReason::UnsupportedStructure)
}

pub(crate) fn decode_object(content: &[u8]) -> Option<Map<String, Value>> {
    let content = content.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(content);
    let value = decode_json_value(content)?;
    value.as_object().cloned()
}

fn decode_json_value(content: &[u8]) -> Option<Value> {
    let mut deserializer = serde_json::Deserializer::from_slice(content);
    let value = Value::deserialize(&mut deserializer).ok()?;
    deserializer.end().ok()?;
    Some(value)
}

pub(crate) fn raw_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(ToOwned::to_owned)
}

pub(crate) fn json_string_equals(value: Option<&Value>, want: &str) -> bool {
    value.and_then(Value::as_str) == Some(want)
}

pub(crate) fn has_nonempty_json_string(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty())
}

fn read_recovery_config(path: &Path) -> Result<Vec<u8>, RecoveryReason> {
    match read_config(path) {
        Ok(content) => Ok(content),
        Err(error) if error.kind() == ErrorKind::InvalidData => Err(RecoveryReason::Oversized),
        Err(_) => Err(RecoveryReason::Unreadable),
    }
}

pub(crate) fn read_config(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut content = Vec::new();
    file.take(MAX_CONFIG_SIZE as u64 + 1)
        .read_to_end(&mut content)?;
    if content.len() > MAX_CONFIG_SIZE {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "configuration file is too large",
        ));
    }
    Ok(content)
}

pub fn strip_jsonc(src: &[u8]) -> Result<Vec<u8>, ()> {
    let mut out = Vec::with_capacity(src.len());
    let mut in_string = false;
    let mut escape = false;
    let mut line_comment = false;
    let mut block_comment = false;
    let mut index = 0;
    while index < src.len() {
        let ch = src[index];
        let next = src.get(index + 1).copied();
        if line_comment {
            if ch == b'\n' {
                line_comment = false;
                out.push(ch);
            }
            index += 1;
        } else if block_comment {
            if ch == b'*' && next == Some(b'/') {
                block_comment = false;
                index += 2;
            } else {
                index += 1;
            }
        } else if in_string {
            out.push(ch);
            if escape {
                escape = false;
            } else if ch == b'\\' {
                escape = true;
            } else if ch == b'"' {
                in_string = false;
            }
            index += 1;
        } else if ch == b'"' {
            in_string = true;
            out.push(ch);
            index += 1;
        } else if ch == b'/' && next == Some(b'/') {
            line_comment = true;
            index += 2;
        } else if ch == b'/' && next == Some(b'*') {
            block_comment = true;
            index += 2;
        } else if ch == b',' {
            let mut look = index + 1;
            while look < src.len() && matches!(src[look], b' ' | b'\t' | b'\r' | b'\n') {
                look += 1;
            }
            if look < src.len() && matches!(src[look], b']' | b'}') {
                index += 1;
                continue;
            }
            out.push(ch);
            index += 1;
        } else {
            out.push(ch);
            index += 1;
        }
    }
    if in_string || block_comment {
        return Err(());
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
