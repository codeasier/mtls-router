use std::collections::HashMap;

use super::types::MAX_CONFIG_SIZE;
use super::url::api_url;

pub fn decode_toml(content: &[u8]) -> Option<toml::Table> {
    if content.len() > MAX_CONFIG_SIZE || std::str::from_utf8(content).is_err() {
        return None;
    }
    let text = std::str::from_utf8(content).ok()?;
    toml::from_str(text).ok()
}

pub fn encode_toml(value: &toml::Table) -> Result<Vec<u8>, String> {
    let encoded = toml::to_string(value).map_err(|error| error.to_string())?;
    if encoded.len() > super::types::MAX_RENDER_SIZE {
        return Err("TOML output exceeds limit".into());
    }
    Ok(encoded.into_bytes())
}

pub fn toml_string(value: Option<&toml::Value>) -> Option<String> {
    value
        .and_then(toml::Value::as_str)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

pub fn toml_table<'a>(root: &'a toml::Table, keys: &[&str]) -> Option<&'a toml::Table> {
    let mut current = root;
    for key in keys {
        current = current.get(*key)?.as_table()?;
    }
    Some(current)
}

pub fn loopback_router_url(value: Option<&toml::Value>, api: bool) -> bool {
    let Some(text) = value.and_then(toml::Value::as_str) else {
        return false;
    };
    let Ok(parsed) = api_url(text.strip_suffix("/v1").unwrap_or(text)) else {
        return false;
    };
    if api {
        parsed == text.trim_end_matches('/')
    } else {
        true
    }
}

pub fn table_from_map(map: HashMap<String, toml::Value>) -> toml::Table {
    map.into_iter().collect()
}
