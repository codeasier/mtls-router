use crate::error::{CommandError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_CONFIG_SIZE: usize = 2 * 1024 * 1024;
pub const MAX_REFERENCED_MODELS_PER_AGENT: usize = 1000;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    pub version: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude: Option<ClaudeConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opencode: Option<OpenCodeConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex: Option<CodexConfig>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSelection {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<ClaudeContext>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum ClaudeContext {
    #[serde(rename = "1m")]
    OneMillion,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ClaudeRole {
    Inherit(InheritPrimary),
    Selection(ModelSelection),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InheritPrimary {
    pub inherit_primary: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeConfig {
    pub primary: ModelSelection,
    pub haiku: ClaudeRole,
    pub sonnet: ClaudeRole,
    pub opus: ClaudeRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<HashMap<String, String>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCodeConfig {
    pub default_model: String,
    pub models: HashMap<String, OpenCodeModel>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCodeModel {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<ModelLimit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modalities: Option<Modalities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interleaved: Option<Interleaved>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variants: Option<HashMap<String, Map<String, Value>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelLimit {
    pub context: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    pub output: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Modalities {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Interleaved {
    Enabled(bool),
    Field(InterleavedField),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InterleavedField {
    pub field: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodexConfig {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_compact_token_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Map<String, Value>>,
}

fn invalid(path: &str, rule: &str) -> CommandError {
    CommandError::invalid_params(format!("model config {rule} at {path}"))
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

pub fn valid_model_id(value: &str) -> bool {
    valid_text(value, 256) && value.trim_matches(char::is_whitespace) == value
}

fn selected_model(value: &ModelSelection, catalog: &HashSet<&str>, path: &str) -> Result<()> {
    if !valid_model_id(&value.model) || value.model.ends_with("[1m]") {
        return Err(invalid(path, "base_model"));
    }
    if !catalog.contains(value.model.as_str()) {
        return Err(invalid(path, "catalog_model"));
    }
    if value
        .name
        .as_ref()
        .is_some_and(|name| !valid_text(name, 16 * 1024))
    {
        return Err(invalid(path, "non_empty_name"));
    }
    Ok(())
}

pub fn validate(config: &ModelConfig, agents: &[String], catalog: &[String]) -> Result<()> {
    if config.version != 1 {
        return Err(invalid("/version", "const"));
    }
    let selected: HashSet<&str> = agents.iter().map(String::as_str).collect();
    if selected.contains("claude") != config.claude.is_some()
        || selected.contains("opencode") != config.opencode.is_some()
        || selected.contains("codex") != config.codex.is_some()
    {
        return Err(invalid("/", "agent_scope"));
    }
    let catalog: HashSet<&str> = catalog.iter().map(String::as_str).collect();
    if let Some(claude) = &config.claude {
        selected_model(&claude.primary, &catalog, "/claude/primary/model")?;
        for (name, role) in [
            ("haiku", &claude.haiku),
            ("sonnet", &claude.sonnet),
            ("opus", &claude.opus),
        ] {
            match role {
                ClaudeRole::Inherit(value) if value.inherit_primary => {}
                ClaudeRole::Selection(value) => {
                    selected_model(value, &catalog, &format!("/claude/{name}/model"))?
                }
                _ => return Err(invalid(&format!("/claude/{name}"), "one_of")),
            }
        }
        for (path, value) in [
            ("/claude/context_window", claude.context_window),
            ("/claude/max_output_tokens", claude.max_output_tokens),
        ] {
            if value.is_some_and(|value| value == 0 || value > MAX_SAFE_INTEGER) {
                return Err(invalid(path, "positive_integer"));
            }
        }
        if claude.context_window.is_some_and(|context| {
            claude
                .max_output_tokens
                .is_some_and(|output| output >= context)
        }) {
            return Err(invalid("/claude/max_output_tokens", "integer_relationship"));
        }
        if claude.context_window.is_some() {
            if claude.primary.context == Some(ClaudeContext::OneMillion) {
                return Err(invalid("/claude/primary/context", "context_conflict"));
            }
            for (name, role) in [
                ("haiku", &claude.haiku),
                ("sonnet", &claude.sonnet),
                ("opus", &claude.opus),
            ] {
                if matches!(
                    role,
                    ClaudeRole::Selection(ModelSelection {
                        context: Some(ClaudeContext::OneMillion),
                        ..
                    })
                ) {
                    return Err(invalid(
                        &format!("/claude/{name}/context"),
                        "context_conflict",
                    ));
                }
            }
        }
        if let Some(extra) = &claude.extra {
            let allowed = [
                "ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION",
                "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION",
                "ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION",
            ];
            if extra
                .iter()
                .any(|(key, value)| !allowed.contains(&key.as_str()) || value.len() > 16 * 1024)
            {
                return Err(invalid("/claude/extra", "allowlist"));
            }
        }
    }
    if let Some(opencode) = &config.opencode {
        if opencode.models.is_empty()
            || opencode.models.len() > MAX_REFERENCED_MODELS_PER_AGENT
            || !opencode.models.contains_key(&opencode.default_model)
        {
            return Err(invalid("/opencode/default_model", "selected_default"));
        }
        for (id, model) in &opencode.models {
            if !valid_model_id(id) || !catalog.contains(id.as_str()) {
                return Err(invalid("/opencode/models", "catalog_model"));
            }
            if model
                .name
                .as_ref()
                .is_some_and(|name| !valid_text(name, 16 * 1024))
            {
                return Err(invalid("/opencode/models/name", "non_empty_name"));
            }
            if let Some(limit) = &model.limit {
                if limit.context == 0
                    || limit.output == 0
                    || limit.context > MAX_SAFE_INTEGER
                    || limit.output > MAX_SAFE_INTEGER
                    || limit
                        .input
                        .is_some_and(|input| input == 0 || input > limit.context)
                {
                    return Err(invalid("/opencode/models/limit", "integer_relationship"));
                }
            }
            if let Some(modalities) = &model.modalities {
                for values in [&modalities.input, &modalities.output] {
                    let mut seen = HashSet::new();
                    if values.iter().any(|value| {
                        !matches!(value.as_str(), "text" | "audio" | "image" | "video" | "pdf")
                            || !seen.insert(value)
                    }) {
                        return Err(invalid("/opencode/models/modalities", "unique_enum"));
                    }
                }
            }
            if let Some(interleaved) = &model.interleaved {
                match interleaved {
                    Interleaved::Enabled(true) => {}
                    Interleaved::Field(field)
                        if matches!(
                            field.field.as_str(),
                            "reasoning" | "reasoning_content" | "reasoning_details"
                        ) => {}
                    _ => return Err(invalid("/opencode/models/interleaved", "enum")),
                }
            }
            if let Some(value) = &model.options {
                validate_extension(value, 0, "/opencode/models/options")?;
            }
            if let Some(variants) = &model.variants {
                for (name, value) in variants {
                    if !valid_text(name, 128) {
                        return Err(invalid("/opencode/models/variants", "key"));
                    }
                    validate_extension(value, 0, "/opencode/models/variants")?;
                }
            }
            if let Some(value) = &model.extra {
                let allowed = [
                    "family",
                    "release_date",
                    "cost",
                    "status",
                    "experimental",
                    "variants",
                ];
                if value.keys().any(|key| !allowed.contains(&key.as_str())) {
                    return Err(invalid("/opencode/models/extra", "allowlist"));
                }
                if model.variants.is_some() && value.contains_key("variants") {
                    return Err(invalid("/opencode/models/variants", "field_conflict"));
                }
                validate_extension(value, 0, "/opencode/models/extra")?;
            }
        }
    }
    if let Some(codex) = &config.codex {
        if !valid_model_id(&codex.model) || !catalog.contains(codex.model.as_str()) {
            return Err(invalid("/codex/model", "catalog_model"));
        }
        if codex.reasoning_effort.as_ref().is_some_and(|value| {
            value.is_empty()
                || value.len() > 64
                || !value.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || byte == b'_'
                        || byte == b'-'
                })
        }) {
            return Err(invalid("/codex/reasoning_effort", "token"));
        }
        if codex.reasoning_summary.as_ref().is_some_and(|value| {
            !matches!(value.as_str(), "auto" | "concise" | "detailed" | "none")
        }) {
            return Err(invalid("/codex/reasoning_summary", "enum"));
        }
        if codex
            .verbosity
            .as_ref()
            .is_some_and(|value| !matches!(value.as_str(), "low" | "medium" | "high"))
        {
            return Err(invalid("/codex/verbosity", "enum"));
        }
        if codex
            .context_window
            .is_some_and(|value| value == 0 || value > MAX_SAFE_INTEGER)
            || codex.auto_compact_token_limit.is_some_and(|value| {
                value == 0
                    || value > MAX_SAFE_INTEGER
                    || codex.context_window.is_some_and(|context| value > context)
            })
        {
            return Err(invalid("/codex/context_window", "integer_relationship"));
        }
        if let Some(extra) = &codex.extra {
            if extra.len() > 1
                || extra.iter().any(|(key, value)| {
                    key != "model_auto_compact_token_limit_scope"
                        || !matches!(value.as_str(), Some("total" | "body_after_prefix"))
                })
            {
                return Err(invalid("/codex/extra", "allowlist"));
            }
        }
    }
    if serde_json::to_vec(config)
        .map_err(|_| invalid("/", "encoding"))?
        .len()
        > MAX_CONFIG_SIZE
    {
        return Err(invalid("/", "max_size"));
    }
    Ok(())
}

fn validate_extension(value: &Map<String, Value>, depth: usize, path: &str) -> Result<()> {
    if depth >= 16 {
        return Err(invalid(path, "max_depth"));
    }
    for (key, value) in value {
        if key.len() > 128 || key.chars().any(char::is_control) {
            return Err(invalid(path, "key"));
        }
        let normalized: String = key
            .chars()
            .filter(|c| !matches!(c, '_' | '-' | '.'))
            .flat_map(char::to_lowercase)
            .collect();
        if [
            "credential",
            "apikey",
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
        ]
        .iter()
        .any(|word| normalized.contains(word))
        {
            return Err(invalid(path, "protected_path"));
        }
        validate_extension_value(value, depth + 1, path)?;
    }
    Ok(())
}

fn validate_extension_value(value: &Value, depth: usize, path: &str) -> Result<()> {
    match value {
        Value::Null => Err(invalid(path, "non_null")),
        Value::String(value) if value.len() > 16 * 1024 => Err(invalid(path, "max_length")),
        Value::Array(values) => {
            if depth >= 16 {
                return Err(invalid(path, "max_depth"));
            }
            if values.len() > 1024 {
                return Err(invalid(path, "max_items"));
            }
            for value in values {
                validate_extension_value(value, depth + 1, path)?;
            }
            Ok(())
        }
        Value::Object(value) => validate_extension(value, depth, path),
        _ => Ok(()),
    }
}

pub fn import_json(content: &str, agents: &[String], catalog: &[String]) -> Result<ModelConfig> {
    if content.is_empty() || content.len() > MAX_CONFIG_SIZE {
        return Err(invalid("/", "max_size"));
    }
    let config: ModelConfig = serde_json::from_str(content).map_err(|_| invalid("/", "json"))?;
    validate(&config, agents, catalog)?;
    Ok(config)
}

pub fn export_json(config: &ModelConfig, agents: &[String], catalog: &[String]) -> Result<String> {
    validate(config, agents, catalog)?;
    let value = serde_json::to_value(config).map_err(|_| invalid("/", "encoding"))?;
    canonical_json(&value)
}

fn canonical_json(value: &Value) -> Result<String> {
    match value {
        Value::Null | Value::Bool(_) | Value::String(_) => {
            serde_json::to_string(value).map_err(|_| invalid("/", "encoding"))
        }
        Value::Number(value) => canonical_number(value),
        Value::Array(values) => {
            let items = values
                .iter()
                .map(canonical_json)
                .collect::<Result<Vec<_>>>()?;
            Ok(format!("[{}]", items.join(",")))
        }
        Value::Object(values) => {
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.encode_utf16().cmp(right.encode_utf16()));
            let entries = entries
                .into_iter()
                .map(|(key, value)| {
                    Ok(format!(
                        "{}:{}",
                        serde_json::to_string(key).map_err(|_| invalid("/", "encoding"))?,
                        canonical_json(value)?
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(format!("{{{}}}", entries.join(",")))
        }
    }
}

fn canonical_number(value: &serde_json::Number) -> Result<String> {
    if value.is_i64() || value.is_u64() {
        return Ok(value.to_string());
    }
    let number = value.as_f64().ok_or_else(|| invalid("/", "encoding"))?;
    if !number.is_finite() {
        return Err(invalid("/", "encoding"));
    }
    if number == 0.0 {
        return Ok("0".into());
    }
    let absolute = number.abs();
    if (1e-6..1e21).contains(&absolute) {
        return Ok(number.to_string());
    }
    let scientific = format!("{number:e}");
    let (mantissa, exponent) = scientific
        .split_once('e')
        .ok_or_else(|| invalid("/", "encoding"))?;
    let exponent: i32 = exponent.parse().map_err(|_| invalid("/", "encoding"))?;
    Ok(format!(
        "{mantissa}e{}{exponent}",
        if exponent >= 0 { "+" } else { "" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal() -> ModelConfig {
        ModelConfig {
            version: 1,
            claude: Some(ClaudeConfig {
                primary: ModelSelection {
                    model: "m1".into(),
                    name: None,
                    context: None,
                },
                haiku: ClaudeRole::Inherit(InheritPrimary {
                    inherit_primary: true,
                }),
                sonnet: ClaudeRole::Inherit(InheritPrimary {
                    inherit_primary: true,
                }),
                opus: ClaudeRole::Inherit(InheritPrimary {
                    inherit_primary: true,
                }),
                context_window: None,
                max_output_tokens: None,
                extra: None,
            }),
            opencode: None,
            codex: None,
        }
    }

    #[test]
    fn validates_scope_catalog_and_protected_extensions() {
        assert!(validate(&minimal(), &["claude".into()], &["m1".into()]).is_ok());
        assert!(validate(&minimal(), &["codex".into()], &["m1".into()]).is_err());
        let mut config = minimal();
        config.claude.as_mut().unwrap().primary.model = "missing".into();
        assert!(validate(&config, &["claude".into()], &["m1".into()]).is_err());
        let mut extra = Map::new();
        extra.insert("api-key".into(), Value::String("value".into()));
        assert!(validate_extension(&extra, 0, "/extra").is_err());
    }

    #[test]
    fn focused_import_export_reject_credentials_and_unknown_fields() {
        let agents = vec!["claude".into()];
        let catalog = vec!["m1".into()];
        let exported = export_json(&minimal(), &agents, &catalog).unwrap();
        assert!(!exported.contains("api_key"));
        assert!(import_json(&exported, &agents, &catalog).is_ok());
        assert!(import_json(r#"{"version":1,"api_key":"secret"}"#, &agents, &catalog).is_err());
    }

    #[test]
    fn claude_context_is_strict_and_suffix_is_never_a_canonical_model() {
        let agents = vec!["claude".into()];
        let catalog = vec!["m1".into(), "m1[1m]".into()];
        let one_million = r#"{"version":1,"claude":{"primary":{"model":"m1","name":"Primary","context":"1m"},"haiku":{"inherit_primary":true},"sonnet":{"model":"m1","context":"1m"},"opus":{"inherit_primary":true}}}"#;
        assert!(import_json(one_million, &agents, &catalog).is_ok());
        for invalid_context in [r#""standard""#, r#""1M""#, r#""""#, "1", "true"] {
            let content = one_million.replace(r#""1m""#, invalid_context);
            assert!(import_json(&content, &agents, &catalog).is_err());
        }
        let suffixed = one_million.replace(r#""m1""#, r#""m1[1m]""#);
        assert!(import_json(&suffixed, &agents, &catalog).is_err());
    }

    #[test]
    fn claude_budgets_and_opencode_variants_round_trip() {
        let agents = vec!["claude".into(), "opencode".into()];
        let catalog = vec!["m1".into()];
        let content = r#"{"version":1,"claude":{"primary":{"model":"m1"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true},"context_window":353400,"max_output_tokens":100000},"opencode":{"default_model":"m1","models":{"m1":{"variants":{"medium":{"reasoningEffort":"medium"}}}}}}"#;
        let config = import_json(content, &agents, &catalog).unwrap();
        let claude = config.claude.as_ref().unwrap();
        assert_eq!(claude.context_window, Some(353400));
        assert_eq!(claude.max_output_tokens, Some(100000));
        assert_eq!(
            config.opencode.as_ref().unwrap().models["m1"]
                .variants
                .as_ref()
                .unwrap()["medium"]["reasoningEffort"],
            "medium"
        );
        assert!(export_json(&config, &agents, &catalog)
            .unwrap()
            .contains(r#""variants":{"medium":{"reasoningEffort":"medium"}}"#));
    }

    #[test]
    fn rejects_invalid_claude_budgets_and_variant_trees() {
        let claude_agents = vec!["claude".into()];
        let opencode_agents = vec!["opencode".into()];
        let catalog = vec!["m1".into()];
        let assert_rule = |content: &str, agents: &[String], rule: &str| {
            let error = import_json(content, agents, &catalog).unwrap_err();
            assert!(
                error.message.contains(rule),
                "expected {rule}, got {}",
                error.message
            );
        };

        let claude = r#"{"version":1,"claude":{"primary":{"model":"m1"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true},"context_window":100,"max_output_tokens":50}}"#;
        assert_rule(
            &claude.replace(r#""context_window":100"#, r#""context_window":0"#),
            &claude_agents,
            "positive_integer",
        );
        assert_rule(
            &claude.replace(
                r#""context_window":100"#,
                r#""context_window":9007199254740992"#,
            ),
            &claude_agents,
            "positive_integer",
        );
        assert_rule(
            &claude.replace(r#""max_output_tokens":50"#, r#""max_output_tokens":100"#),
            &claude_agents,
            "integer_relationship",
        );
        assert_rule(
            &claude.replace(
                r#""primary":{"model":"m1"}"#,
                r#""primary":{"model":"m1","context":"1m"}"#,
            ),
            &claude_agents,
            "context_conflict",
        );
        assert_rule(
            &claude.replace(
                r#""sonnet":{"inherit_primary":true}"#,
                r#""sonnet":{"model":"m1","context":"1m"}"#,
            ),
            &claude_agents,
            "context_conflict",
        );

        let opencode = r#"{"version":1,"opencode":{"default_model":"m1","models":{"m1":{"variants":{"high":{"reasoningEffort":"high"}}}}}}"#;
        assert_rule(
            &opencode.replace("reasoningEffort", "api_key"),
            &opencode_agents,
            "protected_path",
        );
        assert_rule(
            &opencode.replace(
                r#""reasoningEffort":"high""#,
                r#""safe":{"connection":"local"}"#,
            ),
            &opencode_agents,
            "protected_path",
        );
        assert_rule(
            &opencode.replace(
                r#"{"reasoningEffort":"high"}"#,
                &format!(r#"{{"values":{}0{}}}"#, "[".repeat(17), "]".repeat(17)),
            ),
            &opencode_agents,
            "max_depth",
        );
        assert_rule(
            &opencode.replace(
                r#""variants":{"high":{"reasoningEffort":"high"}}"#,
                r#""variants":{"high":{"reasoningEffort":"high"}},"extra":{"variants":{"low":{"reasoningEffort":"low"}}}"#,
            ),
            &opencode_agents,
            "field_conflict",
        );
        assert!(import_json(
            &opencode.replace(
                r#""variants":{"high":{"reasoningEffort":"high"}}"#,
                r#""extra":{"variants":{"high":{"reasoningEffort":"high"}}}"#,
            ),
            &opencode_agents,
            &catalog,
        )
        .is_ok());
        assert_rule(
            &opencode.replace(
                r#""variants":{"high":{"reasoningEffort":"high"}}"#,
                r#""extra":{"variants":{"high":[{"api_key":"secret"}]}}"#,
            ),
            &opencode_agents,
            "protected_path",
        );
        assert_rule(
            &opencode.replace(
                r#""variants":{"high":{"reasoningEffort":"high"}}"#,
                r#""extra":{"variants":[{"safe":{"connection":"local"}}]}"#,
            ),
            &opencode_agents,
            "protected_path",
        );
    }

    #[test]
    fn model_id_whitespace_and_referenced_model_bound_match_go() {
        for invalid in [" model", "model ", "\u{00a0}model", "model\u{3000}"] {
            let mut config = minimal();
            config.claude.as_mut().unwrap().primary.model = invalid.into();
            assert!(validate(&config, &["claude".into()], &[invalid.into()]).is_err());
        }
        let mut config = minimal();
        config.claude.as_mut().unwrap().primary.model = "model id".into();
        assert!(validate(&config, &["claude".into()], &["model id".into()]).is_ok());

        let models = (0..MAX_REFERENCED_MODELS_PER_AGENT)
            .map(|index| (format!("model-{index:04}"), OpenCodeModel::default()))
            .collect::<HashMap<_, _>>();
        let catalog = models.keys().cloned().collect::<Vec<_>>();
        let config = ModelConfig {
            version: 1,
            claude: None,
            opencode: Some(OpenCodeConfig {
                default_model: "model-0000".into(),
                models,
            }),
            codex: None,
        };
        assert!(validate(&config, &["opencode".into()], &catalog).is_ok());
        let mut overlarge = config;
        overlarge
            .opencode
            .as_mut()
            .unwrap()
            .models
            .insert("model-overflow".into(), OpenCodeModel::default());
        assert!(validate(&overlarge, &["opencode".into()], &catalog).is_err());
    }

    #[test]
    fn export_matches_shared_jcs_vectors() {
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../internal/manager/agent/modelconfig/testdata/jcs-vectors.json"
        ))
        .unwrap();
        for vector in vectors.as_array().unwrap() {
            let agents: Vec<String> = serde_json::from_value(vector["agents"].clone()).unwrap();
            let catalog: Vec<String> = serde_json::from_value(vector["catalog"].clone()).unwrap();
            let config = import_json(vector["input"].as_str().unwrap(), &agents, &catalog).unwrap();
            assert_eq!(
                export_json(&config, &agents, &catalog).unwrap(),
                vector["canonical"].as_str().unwrap(),
                "{}",
                vector["name"].as_str().unwrap()
            );
        }
    }

    #[test]
    fn generated_schema_matches_rust_interchange_fields() {
        let schema: Value = serde_json::from_str(include_str!(
            "../../../internal/manager/agent/modelconfig/schema/model-config-v1.schema.json"
        ))
        .unwrap();
        let sorted_keys = |value: &Value| {
            let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        };
        assert_eq!(
            sorted_keys(&schema["properties"]),
            ["claude", "codex", "opencode", "version"]
        );
        assert_eq!(
            sorted_keys(&schema["$defs"]["openCodeModel"]["properties"]),
            [
                "attachment",
                "extra",
                "interleaved",
                "limit",
                "modalities",
                "name",
                "options",
                "reasoning",
                "temperature",
                "tool_call",
                "variants",
            ]
        );
        assert_eq!(
            sorted_keys(&schema["properties"]["claude"]["properties"]),
            [
                "context_window",
                "extra",
                "haiku",
                "max_output_tokens",
                "opus",
                "primary",
                "sonnet",
            ]
        );
        assert_eq!(
            sorted_keys(&schema["$defs"]["model"]["properties"]),
            ["context", "model", "name"]
        );
        assert_eq!(
            sorted_keys(&schema["properties"]["codex"]["properties"]),
            [
                "auto_compact_token_limit",
                "context_window",
                "extra",
                "model",
                "reasoning_effort",
                "reasoning_summary",
                "verbosity",
            ]
        );
    }
}
