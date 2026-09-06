use std::collections::HashMap;

use super::canonical::{JsonNumber, JsonValue};
use super::decode::pointer;
use super::types::{
    invalid, Agent, ClaudeConfig, ClaudeContext, ClaudeRole, CodexConfig, Config, Interleaved,
    Limit, Modalities, Model, OpenCodeConfig, OpenCodeModelConfig, ValidationError,
    MAX_REFERENCED_MODELS_PER_AGENT, MAX_SAFE_INTEGER, VERSION,
};

const PROTECTED_FRAGMENTS: [&str; 15] = [
    "apikey",
    "credential",
    "auth",
    "token",
    "secret",
    "password",
    "bearer",
    "header",
    "url",
    "endpoint",
    "provider",
    "connection",
    "transport",
    "proxy",
    "fetch",
];

pub fn parse_config(
    object: &HashMap<String, JsonValue>,
    selected: &[Agent],
    catalog: &[String],
    require_catalog: bool,
) -> Result<Config, ValidationError> {
    exact_keys(object, "", &["version", "claude", "opencode", "codex"])?;
    let version = integer(object.get("version")).ok_or_else(|| invalid("/version", "version"))?;
    if version != VERSION {
        return Err(invalid("/version", "version"));
    }
    let mut want = HashMap::new();
    for agent in selected {
        if !matches!(agent, Agent::Claude | Agent::OpenCode | Agent::Codex) {
            return Err(invalid("", "agent"));
        }
        if want.insert(*agent, true).is_some() {
            return Err(invalid("", "unique_agents"));
        }
    }
    if want.is_empty() {
        return Err(invalid("", "agents"));
    }
    for agent in Agent::all() {
        if object.contains_key(agent.as_str()) != want.contains_key(&agent) {
            return Err(invalid(format!("/{}", agent.as_str()), "section_scope"));
        }
    }
    let catalog: HashMap<String, bool> = catalog
        .iter()
        .filter(|id| valid_id(id))
        .map(|id| (id.clone(), true))
        .collect();
    let mut config = Config {
        version: VERSION,
        claude: None,
        opencode: None,
        codex: None,
    };
    if want.contains_key(&Agent::Claude) {
        config.claude = Some(parse_claude(
            as_object(object.get("claude")),
            &catalog,
            require_catalog,
        )?);
    }
    if want.contains_key(&Agent::OpenCode) {
        config.opencode = Some(parse_opencode(
            as_object(object.get("opencode")),
            &catalog,
            require_catalog,
        )?);
    }
    if want.contains_key(&Agent::Codex) {
        config.codex = Some(parse_codex(
            as_object(object.get("codex")),
            &catalog,
            require_catalog,
        )?);
    }
    Ok(config)
}

fn parse_claude(
    object: Option<&HashMap<String, JsonValue>>,
    catalog: &HashMap<String, bool>,
    require_catalog: bool,
) -> Result<ClaudeConfig, ValidationError> {
    let object = object.ok_or_else(|| invalid("/claude", "object"))?;
    exact_keys(
        object,
        "/claude",
        &[
            "primary",
            "fable",
            "haiku",
            "sonnet",
            "opus",
            "context_window",
            "max_output_tokens",
            "extra",
        ],
    )?;
    let primary = parse_model(
        as_object(object.get("primary")),
        "/claude/primary",
        catalog,
        require_catalog,
    )?;
    let mut config = ClaudeConfig {
        primary,
        fable: None,
        haiku: ClaudeRole::inherit(),
        sonnet: ClaudeRole::inherit(),
        opus: ClaudeRole::inherit(),
        context_window: None,
        max_output_tokens: None,
        extra: HashMap::new(),
    };
    if object.contains_key("fable") {
        config.fable = Some(parse_role(
            as_object(object.get("fable")),
            "/claude/fable",
            catalog,
            require_catalog,
        )?);
    }
    config.haiku = parse_role(
        as_object(object.get("haiku")),
        "/claude/haiku",
        catalog,
        require_catalog,
    )?;
    config.sonnet = parse_role(
        as_object(object.get("sonnet")),
        "/claude/sonnet",
        catalog,
        require_catalog,
    )?;
    config.opus = parse_role(
        as_object(object.get("opus")),
        "/claude/opus",
        catalog,
        require_catalog,
    )?;
    if object.contains_key("context_window") {
        config.context_window = Some(
            positive(object.get("context_window"))
                .ok_or_else(|| invalid("/claude/context_window", "positive_integer"))?,
        );
    }
    if object.contains_key("max_output_tokens") {
        config.max_output_tokens = Some(
            positive(object.get("max_output_tokens"))
                .ok_or_else(|| invalid("/claude/max_output_tokens", "positive_integer"))?,
        );
    }
    if let (Some(window), Some(output)) = (config.context_window, config.max_output_tokens) {
        if output >= window {
            return Err(invalid("/claude/max_output_tokens", "integer_relationship"));
        }
    }
    if config.context_window.is_some() {
        if config.primary.context.is_some() {
            return Err(invalid("/claude/primary/context", "context_conflict"));
        }
        for (key, role) in [
            ("haiku", &config.haiku),
            ("sonnet", &config.sonnet),
            ("opus", &config.opus),
        ] {
            if role
                .selection
                .as_ref()
                .and_then(|selection| selection.context)
                .is_some()
            {
                return Err(invalid(
                    format!("/claude/{key}/context"),
                    "context_conflict",
                ));
            }
        }
        if config
            .fable
            .as_ref()
            .and_then(|role| role.selection.as_ref())
            .and_then(|selection| selection.context)
            .is_some()
        {
            return Err(invalid("/claude/fable/context", "context_conflict"));
        }
    }
    if object.contains_key("extra") {
        let extra =
            as_object(object.get("extra")).ok_or_else(|| invalid("/claude/extra", "object"))?;
        let allowed = [
            "ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION",
            "ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION",
        ];
        for (key, value) in extra {
            let text = value
                .as_str()
                .ok_or_else(|| invalid(pointer("/claude/extra", key), "allowlist"))?;
            if !allowed.contains(&key.as_str()) || has_control(key) {
                return Err(invalid(pointer("/claude/extra", key), "allowlist"));
            }
            config.extra.insert(key.clone(), text.to_owned());
        }
    }
    Ok(config)
}

fn parse_model(
    object: Option<&HashMap<String, JsonValue>>,
    path: &str,
    catalog: &HashMap<String, bool>,
    require_catalog: bool,
) -> Result<Model, ValidationError> {
    let object = object.ok_or_else(|| invalid(path, "object"))?;
    exact_keys(object, path, &["model", "name", "context"])?;
    let id = object
        .get("model")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| invalid(format!("{path}/model"), "model_id"))?;
    if !valid_id(id) || id.ends_with("[1m]") {
        return Err(invalid(format!("{path}/model"), "model_id"));
    }
    if require_catalog && !catalog.contains_key(id) {
        return Err(invalid(format!("{path}/model"), "catalog"));
    }
    let mut model = Model {
        model: id.to_owned(),
        name: None,
        context: None,
    };
    if object.contains_key("name") {
        let name = object
            .get("name")
            .and_then(JsonValue::as_str)
            .filter(|value| valid_name(value))
            .ok_or_else(|| invalid(format!("{path}/name"), "name"))?;
        model.name = Some(name.to_owned());
    }
    if object.contains_key("context") {
        let context = object
            .get("context")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| invalid(format!("{path}/context"), "enum"))?;
        if context != ClaudeContext::OneMillion.as_str() {
            return Err(invalid(format!("{path}/context"), "enum"));
        }
        model.context = Some(ClaudeContext::OneMillion);
    }
    Ok(model)
}

fn parse_role(
    object: Option<&HashMap<String, JsonValue>>,
    path: &str,
    catalog: &HashMap<String, bool>,
    require_catalog: bool,
) -> Result<ClaudeRole, ValidationError> {
    let object = object.ok_or_else(|| invalid(path, "object"))?;
    if object.len() == 1 {
        if let Some(true) = object.get("inherit_primary").and_then(JsonValue::as_bool) {
            return Ok(ClaudeRole::inherit());
        }
    }
    Ok(ClaudeRole::selected(parse_model(
        Some(object),
        path,
        catalog,
        require_catalog,
    )?))
}

fn parse_opencode(
    object: Option<&HashMap<String, JsonValue>>,
    catalog: &HashMap<String, bool>,
    require_catalog: bool,
) -> Result<OpenCodeConfig, ValidationError> {
    let object = object.ok_or_else(|| invalid("/opencode", "object"))?;
    exact_keys(object, "/opencode", &["default_model", "models"])?;
    let default_model = object
        .get("default_model")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| invalid("/opencode/default_model", "string"))?
        .to_owned();
    let models = as_object(object.get("models")).cloned().unwrap_or_default();
    if models.is_empty() {
        return Err(invalid("/opencode/models", "min_properties"));
    }
    if models.len() > MAX_REFERENCED_MODELS_PER_AGENT {
        return Err(invalid("/opencode/models", "max_properties"));
    }
    let mut parsed = HashMap::new();
    for (id, raw) in &models {
        let path = pointer("/opencode/models", id);
        if !valid_id(id) || (require_catalog && !catalog.contains_key(id)) {
            return Err(invalid(path, "catalog"));
        }
        parsed.insert(
            id.clone(),
            parse_opencode_model(as_object(Some(raw)), &path)?,
        );
    }
    if !parsed.contains_key(&default_model) {
        return Err(invalid("/opencode/default_model", "selected_model"));
    }
    Ok(OpenCodeConfig {
        default_model,
        models: parsed,
    })
}

fn parse_opencode_model(
    object: Option<&HashMap<String, JsonValue>>,
    path: &str,
) -> Result<OpenCodeModelConfig, ValidationError> {
    let object = object.ok_or_else(|| invalid(path, "object"))?;
    exact_keys(
        object,
        path,
        &[
            "name",
            "reasoning",
            "attachment",
            "tool_call",
            "temperature",
            "limit",
            "modalities",
            "interleaved",
            "options",
            "variants",
            "extra",
        ],
    )?;
    let mut model = OpenCodeModelConfig::default();
    if object.contains_key("name") {
        let name = object
            .get("name")
            .and_then(JsonValue::as_str)
            .filter(|value| valid_name(value))
            .ok_or_else(|| invalid(format!("{path}/name"), "name"))?;
        model.name = Some(name.to_owned());
    }
    for (key, slot) in [
        ("reasoning", &mut model.reasoning),
        ("attachment", &mut model.attachment),
        ("tool_call", &mut model.tool_call),
        ("temperature", &mut model.temperature),
    ] {
        if object.contains_key(key) {
            *slot = Some(
                object
                    .get(key)
                    .and_then(JsonValue::as_bool)
                    .ok_or_else(|| invalid(format!("{path}/{key}"), "boolean"))?,
            );
        }
    }
    if object.contains_key("limit") {
        model.limit = Some(parse_limit(
            as_object(object.get("limit")),
            &format!("{path}/limit"),
        )?);
    }
    if object.contains_key("modalities") {
        model.modalities = Some(parse_modalities(
            as_object(object.get("modalities")),
            &format!("{path}/modalities"),
        )?);
    }
    if let Some(interleaved) = object.get("interleaved") {
        model.interleaved = Some(parse_interleaved(interleaved, path)?);
    }
    if object.contains_key("options") {
        let options = as_object(object.get("options"))
            .ok_or_else(|| invalid(format!("{path}/options"), "object"))?;
        validate_tree(
            &JsonValue::Object(options.clone()),
            &format!("{path}/options"),
            0,
            None,
        )?;
        model.options = Some(map_from_object(options));
    }
    if object.contains_key("variants") {
        let variants = as_object(object.get("variants"))
            .ok_or_else(|| invalid(format!("{path}/variants"), "object"))?;
        let mut parsed = HashMap::new();
        for (name, raw) in variants {
            let variant_path = pointer(&format!("{path}/variants"), name);
            if name.is_empty() || name.len() > 128 || has_control(name) {
                return Err(invalid(variant_path, "key"));
            }
            let options =
                as_object(Some(raw)).ok_or_else(|| invalid(variant_path.clone(), "object"))?;
            validate_tree(&JsonValue::Object(options.clone()), &variant_path, 0, None)?;
            parsed.insert(name.clone(), map_from_object(options));
        }
        model.variants = Some(parsed);
    }
    if object.contains_key("extra") {
        let extra = as_object(object.get("extra"))
            .ok_or_else(|| invalid(format!("{path}/extra"), "object"))?;
        if model.variants.is_some() && extra.contains_key("variants") {
            return Err(invalid(format!("{path}/variants"), "field_conflict"));
        }
        let allowed = [
            "family",
            "release_date",
            "cost",
            "status",
            "experimental",
            "variants",
        ];
        validate_tree(
            &JsonValue::Object(extra.clone()),
            &format!("{path}/extra"),
            0,
            Some(&allowed),
        )?;
        model.extra = Some(map_from_object(extra));
    }
    Ok(model)
}

fn parse_interleaved(value: &JsonValue, path: &str) -> Result<Interleaved, ValidationError> {
    match value {
        JsonValue::Bool(true) => Ok(Interleaved::Enabled),
        JsonValue::Bool(false) => Err(invalid(format!("{path}/interleaved"), "enum")),
        JsonValue::Object(object) => {
            exact_keys(object, &format!("{path}/interleaved"), &["field"])?;
            let field = object
                .get("field")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| invalid(format!("{path}/interleaved/field"), "enum"))?;
            if !matches!(
                field,
                "reasoning" | "reasoning_content" | "reasoning_details"
            ) {
                return Err(invalid(format!("{path}/interleaved/field"), "enum"));
            }
            Ok(Interleaved::Field(field.to_owned()))
        }
        _ => Err(invalid(format!("{path}/interleaved"), "shape")),
    }
}

fn parse_limit(
    object: Option<&HashMap<String, JsonValue>>,
    path: &str,
) -> Result<Limit, ValidationError> {
    let object = object.ok_or_else(|| invalid(path, "object"))?;
    exact_keys(object, path, &["context", "input", "output"])?;
    let context = positive(object.get("context"))
        .ok_or_else(|| invalid(format!("{path}/context"), "positive_integer"))?;
    let output = positive(object.get("output"))
        .ok_or_else(|| invalid(format!("{path}/output"), "positive_integer"))?;
    let mut limit = Limit {
        context,
        input: None,
        output,
    };
    if object.contains_key("input") {
        let input = positive(object.get("input"))
            .ok_or_else(|| invalid(format!("{path}/input"), "positive_integer"))?;
        if input > context {
            return Err(invalid(format!("{path}/input"), "maximum_context"));
        }
        limit.input = Some(input);
    }
    Ok(limit)
}

fn parse_modalities(
    object: Option<&HashMap<String, JsonValue>>,
    path: &str,
) -> Result<Modalities, ValidationError> {
    let object = object.ok_or_else(|| invalid(path, "object"))?;
    exact_keys(object, path, &["input", "output"])?;
    let mut modalities = Modalities::default();
    for (key, dest) in [
        ("input", &mut modalities.input),
        ("output", &mut modalities.output),
    ] {
        if let Some(value) = object.get(key) {
            let array = value
                .as_array()
                .ok_or_else(|| invalid(format!("{path}/{key}"), "array"))?;
            let mut seen = HashMap::new();
            for (index, item) in array.iter().enumerate() {
                let text = item
                    .as_str()
                    .ok_or_else(|| invalid(format!("{path}/{key}/{index}"), "enum"))?;
                if !matches!(text, "text" | "audio" | "image" | "video" | "pdf") {
                    return Err(invalid(format!("{path}/{key}/{index}"), "enum"));
                }
                if seen.insert(text.to_owned(), true).is_some() {
                    return Err(invalid(format!("{path}/{key}"), "unique"));
                }
                dest.push(text.to_owned());
            }
        }
    }
    Ok(modalities)
}

fn parse_codex(
    object: Option<&HashMap<String, JsonValue>>,
    catalog: &HashMap<String, bool>,
    require_catalog: bool,
) -> Result<CodexConfig, ValidationError> {
    let object = object.ok_or_else(|| invalid("/codex", "object"))?;
    exact_keys(
        object,
        "/codex",
        &[
            "model",
            "reasoning_effort",
            "reasoning_summary",
            "verbosity",
            "context_window",
            "auto_compact_token_limit",
            "extra",
        ],
    )?;
    let id = object
        .get("model")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| invalid("/codex/model", "catalog"))?;
    if !valid_id(id) || (require_catalog && !catalog.contains_key(id)) {
        return Err(invalid("/codex/model", "catalog"));
    }
    let mut config = CodexConfig {
        model: id.to_owned(),
        reasoning_effort: None,
        reasoning_summary: None,
        verbosity: None,
        context_window: None,
        auto_compact_token_limit: None,
        extra: HashMap::new(),
    };
    if object.contains_key("reasoning_effort") {
        let value = object
            .get("reasoning_effort")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| invalid("/codex/reasoning_effort", "lowercase_token"))?;
        if value.is_empty()
            || value.len() > 64
            || value != value.to_ascii_lowercase()
            || !is_token(value)
        {
            return Err(invalid("/codex/reasoning_effort", "lowercase_token"));
        }
        config.reasoning_effort = Some(value.to_owned());
    }
    if object.contains_key("reasoning_summary") {
        let value = object
            .get("reasoning_summary")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| invalid("/codex/reasoning_summary", "enum"))?;
        if !matches!(value, "auto" | "concise" | "detailed" | "none") {
            return Err(invalid("/codex/reasoning_summary", "enum"));
        }
        config.reasoning_summary = Some(value.to_owned());
    }
    if object.contains_key("verbosity") {
        let value = object
            .get("verbosity")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| invalid("/codex/verbosity", "enum"))?;
        if !matches!(value, "low" | "medium" | "high") {
            return Err(invalid("/codex/verbosity", "enum"));
        }
        config.verbosity = Some(value.to_owned());
    }
    if object.contains_key("context_window") {
        config.context_window = Some(
            positive(object.get("context_window"))
                .ok_or_else(|| invalid("/codex/context_window", "positive_integer"))?,
        );
    }
    if object.contains_key("auto_compact_token_limit") {
        let value = positive(object.get("auto_compact_token_limit"))
            .ok_or_else(|| invalid("/codex/auto_compact_token_limit", "positive_integer"))?;
        if config.context_window.is_some_and(|window| value > window) {
            return Err(invalid(
                "/codex/auto_compact_token_limit",
                "maximum_context",
            ));
        }
        config.auto_compact_token_limit = Some(value);
    }
    if object.contains_key("extra") {
        let extra =
            as_object(object.get("extra")).ok_or_else(|| invalid("/codex/extra", "object"))?;
        exact_keys(
            extra,
            "/codex/extra",
            &["model_auto_compact_token_limit_scope"],
        )?;
        if let Some(raw) = extra.get("model_auto_compact_token_limit_scope") {
            let value = raw.as_str().ok_or_else(|| {
                invalid("/codex/extra/model_auto_compact_token_limit_scope", "enum")
            })?;
            if !matches!(value, "total" | "body_after_prefix") {
                return Err(invalid(
                    "/codex/extra/model_auto_compact_token_limit_scope",
                    "enum",
                ));
            }
        }
        config.extra = map_from_object(extra);
    }
    Ok(config)
}

fn validate_tree(
    value: &JsonValue,
    path: &str,
    depth: usize,
    top_allow: Option<&[&str]>,
) -> Result<(), ValidationError> {
    match value {
        JsonValue::Null => Err(invalid(path, "null")),
        JsonValue::String(text) if text.len() > 16 << 10 => Err(invalid(path, "string_size")),
        JsonValue::String(_) | JsonValue::Bool(_) => Ok(()),
        JsonValue::Number(number) => {
            number
                .as_str()
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or_else(|| invalid(path, "number"))?;
            Ok(())
        }
        JsonValue::Array(items) => {
            if depth >= 16 {
                return Err(invalid(path, "depth"));
            }
            if items.len() > 1024 {
                return Err(invalid(path, "array_size"));
            }
            for (index, item) in items.iter().enumerate() {
                validate_tree(item, &format!("{path}/{index}"), depth + 1, None)?;
            }
            Ok(())
        }
        JsonValue::Object(object) => {
            if depth >= 16 {
                return Err(invalid(path, "depth"));
            }
            for (key, child) in object {
                let child_path = pointer(path, key);
                if key.len() > 128 || has_control(key) {
                    return Err(invalid(child_path, "key"));
                }
                if let Some(allow) = top_allow {
                    if !allow.contains(&key.as_str()) {
                        return Err(invalid(child_path, "allowlist"));
                    }
                }
                if protected_key(key) {
                    return Err(invalid(child_path, "protected_path"));
                }
                validate_tree(child, &child_path, depth + 1, None)?;
            }
            Ok(())
        }
    }
}

fn exact_keys(
    object: &HashMap<String, JsonValue>,
    path: &str,
    keys: &[&str],
) -> Result<(), ValidationError> {
    for key in object.keys() {
        if !keys.contains(&key.as_str()) {
            return Err(invalid(pointer(path, key), "unknown_field"));
        }
    }
    Ok(())
}

fn integer(value: Option<&JsonValue>) -> Option<i64> {
    let number = value?.as_number()?;
    let parsed = number.as_str().parse::<i64>().ok()?;
    (parsed >= 0 && parsed <= MAX_SAFE_INTEGER).then_some(parsed)
}

fn positive(value: Option<&JsonValue>) -> Option<i64> {
    integer(value).filter(|value| *value > 0)
}

fn as_object(value: Option<&JsonValue>) -> Option<&HashMap<String, JsonValue>> {
    value.and_then(JsonValue::as_object)
}

fn map_from_object(object: &HashMap<String, JsonValue>) -> HashMap<String, JsonValue> {
    object.clone()
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !has_control(value) && value.trim() == value
}

fn valid_name(value: &str) -> bool {
    !value.is_empty() && !has_control(value)
}

fn has_control(value: &str) -> bool {
    value.chars().any(|ch| ch.is_control())
}

fn protected_key(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase().replace(['_', '-', '.'], "");
    PROTECTED_FRAGMENTS
        .iter()
        .any(|fragment| normalized.contains(fragment))
}

fn is_token(value: &str) -> bool {
    value
        .chars()
        .all(|ch| matches!(ch, 'a'..='z' | '0'..='9' | '_' | '-'))
}
