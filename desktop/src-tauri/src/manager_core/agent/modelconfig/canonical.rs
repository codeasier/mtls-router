use std::collections::HashMap;

use super::types::{invalid, ClaudeRole, Config, Interleaved, ValidationError};

#[derive(Clone, Debug, PartialEq)]
pub struct JsonNumber(pub String);

impl JsonNumber {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum JsonValue {
    #[default]
    Null,
    Bool(bool),
    String(String),
    Number(JsonNumber),
    Array(Vec<JsonValue>),
    Object(HashMap<String, JsonValue>),
}

impl JsonValue {
    pub fn object(pairs: impl IntoIterator<Item = (String, JsonValue)>) -> Self {
        Self::Object(pairs.into_iter().collect())
    }

    pub fn as_object(&self) -> Option<&HashMap<String, JsonValue>> {
        match self {
            Self::Object(object) => Some(object),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            Self::Array(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<&JsonNumber> {
        match self {
            Self::Number(value) => Some(value),
            _ => None,
        }
    }
}

pub fn canonical(config: &Config) -> Result<Vec<u8>, ValidationError> {
    marshal_jcs(&config_to_value(config)?)
}

pub fn canonical_value(value: &JsonValue) -> Result<Vec<u8>, ValidationError> {
    marshal_jcs(value)
}

pub fn marshal_jcs(value: &JsonValue) -> Result<Vec<u8>, ValidationError> {
    let mut out = Vec::new();
    append_jcs(&mut out, value)?;
    Ok(out)
}

fn append_jcs(out: &mut Vec<u8>, value: &JsonValue) -> Result<(), ValidationError> {
    match value {
        JsonValue::Null => out.extend_from_slice(b"null"),
        JsonValue::Bool(true) => out.extend_from_slice(b"true"),
        JsonValue::Bool(false) => out.extend_from_slice(b"false"),
        JsonValue::String(value) => append_string(out, value),
        JsonValue::Number(number) => {
            let parsed = parse_finite_f64(number.as_str())?;
            out.extend_from_slice(format_number(parsed).as_bytes());
        }
        JsonValue::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                append_jcs(out, item)?;
            }
            out.push(b']');
        }
        JsonValue::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort_by(|left, right| {
                if utf16_less(left, right) {
                    std::cmp::Ordering::Less
                } else if utf16_less(right, left) {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            });
            out.push(b'{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                append_string(out, key);
                out.push(b':');
                append_jcs(out, &object[key.as_str()])?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

fn parse_finite_f64(value: &str) -> Result<f64, ValidationError> {
    let parsed = value.parse::<f64>().map_err(|_| invalid("", "number"))?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(invalid("", "number"))
    }
}

pub fn format_number(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let absolute = value.abs();
    if absolute >= 1e21 || absolute < 1e-6 {
        let formatted = format!("{value:e}");
        let Some(index) = formatted.rfind('e') else {
            return formatted;
        };
        let mantissa = &formatted[..index];
        let mut exponent = &formatted[index + 1..];
        let mut sign = "+";
        if let Some(rest) = exponent.strip_prefix('+') {
            exponent = rest;
        } else if let Some(rest) = exponent.strip_prefix('-') {
            sign = "-";
            exponent = rest;
        }
        while exponent.len() > 1 && exponent.starts_with('0') {
            exponent = &exponent[1..];
        }
        format!("{mantissa}e{sign}{exponent}")
    } else {
        let mut formatted = format!("{value}");
        if formatted.ends_with(".0") && !formatted.contains('e') {
            formatted.truncate(formatted.len() - 2);
        }
        formatted
    }
}

fn append_string(out: &mut Vec<u8>, value: &str) {
    out.push(b'"');
    for ch in value.chars() {
        match ch {
            '"' | '\\' => {
                out.push(b'\\');
                out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
            }
            '\u{0008}' => out.extend_from_slice(br"\b"),
            '\t' => out.extend_from_slice(br"\t"),
            '\n' => out.extend_from_slice(br"\n"),
            '\u{000c}' => out.extend_from_slice(br"\f"),
            '\r' => out.extend_from_slice(br"\r"),
            ch if (ch as u32) < 0x20 => {
                out.extend_from_slice(br"\u00");
                out.push(hex_digit((ch as u32) >> 4));
                out.push(hex_digit((ch as u32) & 15));
            }
            ch => out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes()),
        }
    }
    out.push(b'"');
}

fn hex_digit(value: u32) -> u8 {
    if value < 10 {
        b'0' + value as u8
    } else {
        b'a' + (value as u8 - 10)
    }
}

fn utf16_less(left: &str, right: &str) -> bool {
    let left_units: Vec<u16> = left.encode_utf16().collect();
    let right_units: Vec<u16> = right.encode_utf16().collect();
    for (a, b) in left_units.iter().zip(right_units.iter()) {
        if a != b {
            return a < b;
        }
    }
    left_units.len() < right_units.len()
}

pub fn config_to_value(config: &Config) -> Result<JsonValue, ValidationError> {
    let mut object = HashMap::new();
    object.insert(
        "version".to_owned(),
        JsonValue::Number(JsonNumber("1".into())),
    );
    if let Some(claude) = &config.claude {
        object.insert("claude".to_owned(), claude_to_value(claude)?);
    }
    if let Some(opencode) = &config.opencode {
        object.insert("opencode".to_owned(), opencode_to_value(opencode));
    }
    if let Some(codex) = &config.codex {
        object.insert("codex".to_owned(), codex_to_value(codex));
    }
    Ok(JsonValue::Object(object))
}

fn claude_to_value(config: &super::types::ClaudeConfig) -> Result<JsonValue, ValidationError> {
    let mut object = HashMap::new();
    object.insert("primary".to_owned(), model_to_value(&config.primary));
    if let Some(fable) = &config.fable {
        object.insert("fable".to_owned(), role_to_value(fable)?);
    }
    object.insert("haiku".to_owned(), role_to_value(&config.haiku)?);
    object.insert("sonnet".to_owned(), role_to_value(&config.sonnet)?);
    object.insert("opus".to_owned(), role_to_value(&config.opus)?);
    if let Some(window) = config.context_window {
        object.insert(
            "context_window".to_owned(),
            JsonValue::Number(JsonNumber(window.to_string())),
        );
    }
    if let Some(tokens) = config.max_output_tokens {
        object.insert(
            "max_output_tokens".to_owned(),
            JsonValue::Number(JsonNumber(tokens.to_string())),
        );
    }
    if !config.extra.is_empty() {
        object.insert(
            "extra".to_owned(),
            JsonValue::Object(
                config
                    .extra
                    .iter()
                    .map(|(key, value)| (key.clone(), JsonValue::String(value.clone())))
                    .collect(),
            ),
        );
    }
    Ok(JsonValue::Object(object))
}

fn role_to_value(role: &ClaudeRole) -> Result<JsonValue, ValidationError> {
    if role.inherit_primary && role.selection.is_none() {
        return Ok(JsonValue::object([(
            "inherit_primary".to_owned(),
            JsonValue::Bool(true),
        )]));
    }
    if !role.inherit_primary {
        if let Some(selection) = &role.selection {
            return Ok(model_to_value(selection));
        }
    }
    Err(invalid("", "json_value"))
}

fn model_to_value(model: &super::types::Model) -> JsonValue {
    let mut object = HashMap::new();
    object.insert("model".to_owned(), JsonValue::String(model.model.clone()));
    if let Some(name) = &model.name {
        object.insert("name".to_owned(), JsonValue::String(name.clone()));
    }
    if let Some(context) = model.context {
        object.insert(
            "context".to_owned(),
            JsonValue::String(context.as_str().to_owned()),
        );
    }
    JsonValue::Object(object)
}

fn opencode_to_value(config: &super::types::OpenCodeConfig) -> JsonValue {
    let mut models = HashMap::new();
    for (id, model) in &config.models {
        models.insert(id.clone(), open_code_model_to_value(model));
    }
    JsonValue::object([
        (
            "default_model".to_owned(),
            JsonValue::String(config.default_model.clone()),
        ),
        ("models".to_owned(), JsonValue::Object(models)),
    ])
}

fn open_code_model_to_value(model: &super::types::OpenCodeModelConfig) -> JsonValue {
    let mut object = HashMap::new();
    if let Some(name) = &model.name {
        object.insert("name".to_owned(), JsonValue::String(name.clone()));
    }
    insert_bool(&mut object, "reasoning", model.reasoning);
    insert_bool(&mut object, "attachment", model.attachment);
    insert_bool(&mut object, "tool_call", model.tool_call);
    insert_bool(&mut object, "temperature", model.temperature);
    if let Some(limit) = &model.limit {
        let mut limit_object = HashMap::new();
        limit_object.insert(
            "context".to_owned(),
            JsonValue::Number(JsonNumber(limit.context.to_string())),
        );
        if let Some(input) = limit.input {
            limit_object.insert(
                "input".to_owned(),
                JsonValue::Number(JsonNumber(input.to_string())),
            );
        }
        limit_object.insert(
            "output".to_owned(),
            JsonValue::Number(JsonNumber(limit.output.to_string())),
        );
        object.insert("limit".to_owned(), JsonValue::Object(limit_object));
    }
    if let Some(modalities) = &model.modalities {
        let mut modalities_object = HashMap::new();
        if !modalities.input.is_empty() {
            modalities_object.insert(
                "input".to_owned(),
                JsonValue::Array(
                    modalities
                        .input
                        .iter()
                        .map(|value| JsonValue::String(value.clone()))
                        .collect(),
                ),
            );
        }
        if !modalities.output.is_empty() {
            modalities_object.insert(
                "output".to_owned(),
                JsonValue::Array(
                    modalities
                        .output
                        .iter()
                        .map(|value| JsonValue::String(value.clone()))
                        .collect(),
                ),
            );
        }
        object.insert(
            "modalities".to_owned(),
            JsonValue::Object(modalities_object),
        );
    }
    if let Some(interleaved) = &model.interleaved {
        object.insert(
            "interleaved".to_owned(),
            match interleaved {
                Interleaved::Enabled => JsonValue::Bool(true),
                Interleaved::Field(field) => {
                    JsonValue::object([("field".to_owned(), JsonValue::String(field.clone()))])
                }
            },
        );
    }
    if let Some(options) = &model.options {
        object.insert("options".to_owned(), JsonValue::Object(options.clone()));
    }
    if let Some(variants) = &model.variants {
        object.insert(
            "variants".to_owned(),
            JsonValue::Object(
                variants
                    .iter()
                    .map(|(name, options)| (name.clone(), JsonValue::Object(options.clone())))
                    .collect(),
            ),
        );
    }
    if let Some(extra) = &model.extra {
        object.insert("extra".to_owned(), JsonValue::Object(extra.clone()));
    }
    JsonValue::Object(object)
}

fn insert_bool(object: &mut HashMap<String, JsonValue>, key: &str, value: Option<bool>) {
    if let Some(value) = value {
        object.insert(key.to_owned(), JsonValue::Bool(value));
    }
}

fn codex_to_value(config: &super::types::CodexConfig) -> JsonValue {
    let mut object = HashMap::new();
    object.insert("model".to_owned(), JsonValue::String(config.model.clone()));
    if let Some(value) = &config.reasoning_effort {
        object.insert(
            "reasoning_effort".to_owned(),
            JsonValue::String(value.clone()),
        );
    }
    if let Some(value) = &config.reasoning_summary {
        object.insert(
            "reasoning_summary".to_owned(),
            JsonValue::String(value.clone()),
        );
    }
    if let Some(value) = &config.verbosity {
        object.insert("verbosity".to_owned(), JsonValue::String(value.clone()));
    }
    if let Some(value) = config.context_window {
        object.insert(
            "context_window".to_owned(),
            JsonValue::Number(JsonNumber(value.to_string())),
        );
    }
    if let Some(value) = config.auto_compact_token_limit {
        object.insert(
            "auto_compact_token_limit".to_owned(),
            JsonValue::Number(JsonNumber(value.to_string())),
        );
    }
    if !config.extra.is_empty() {
        object.insert("extra".to_owned(), JsonValue::Object(config.extra.clone()));
    }
    JsonValue::Object(object)
}
