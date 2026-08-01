//! `/v1/models/image` strict parser and two immutable model presets.
//!
//! The parser limits response size, preserves `/` in model IDs, and returns
//! the exact intersection of presets and the authenticated catalog without
//! inference or fallback.

use crate::image_limits::{CATALOG_BODY_LIMIT, CATALOG_MAX_ENTRIES, CATALOG_MAX_ID_LEN};
use serde::Deserialize;

/// The two immutable preset models for image conversations.
pub static PRESET_MODELS: &[PresetModel] = &[
    PresetModel {
        id: "cx/gpt-5.5-image",
        display_name: "GPT 5.5 Image",
        supports_generation: true,
        supports_edit: true,
    },
    PresetModel {
        id: "ag/gemini-3.1-flash-image",
        display_name: "Gemini 3.1 Flash Image",
        supports_generation: true,
        supports_edit: true,
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetModel {
    pub id: &'static str,
    pub display_name: &'static str,
    pub supports_generation: bool,
    pub supports_edit: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogError {
    BodyTooLarge,
    InvalidJson,
    NotList,
    TooManyEntries,
    IdTooLong,
    IdMissingSlash,
    EmptyId,
    InvalidEntry,
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BodyTooLarge => write!(f, "catalog response exceeds size limit"),
            Self::InvalidJson => write!(f, "catalog response is not valid JSON"),
            Self::NotList => write!(f, "catalog response object is not \"list\""),
            Self::TooManyEntries => write!(f, "catalog has too many entries"),
            Self::IdTooLong => write!(f, "catalog model ID exceeds length limit"),
            Self::IdMissingSlash => write!(f, "catalog model ID does not contain slash"),
            Self::EmptyId => write!(f, "catalog model ID is empty"),
            Self::InvalidEntry => write!(f, "catalog entry is not a model"),
        }
    }
}

impl std::error::Error for CatalogError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntry {
    pub id: String,
    pub owned_by: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalog {
    object: String,
    data: Vec<RawCatalogEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalogEntry {
    id: String,
    object: String,
    #[serde(default)]
    owned_by: String,
}

/// Parse a raw `/v1/models/image` response body into catalog entries.
/// Enforces size, count, and ID format constraints.
pub fn parse_catalog(body: &[u8]) -> Result<Vec<CatalogEntry>, CatalogError> {
    if body.len() > CATALOG_BODY_LIMIT {
        return Err(CatalogError::BodyTooLarge);
    }
    let raw: RawCatalog = serde_json::from_slice(body).map_err(|_| CatalogError::InvalidJson)?;
    if raw.object != "list" {
        return Err(CatalogError::NotList);
    }
    if raw.data.len() > CATALOG_MAX_ENTRIES {
        return Err(CatalogError::TooManyEntries);
    }
    let mut entries = Vec::with_capacity(raw.data.len());
    for item in raw.data {
        if item.object != "model" {
            return Err(CatalogError::InvalidEntry);
        }
        if item.id.is_empty() {
            return Err(CatalogError::EmptyId);
        }
        if item.id.len() > CATALOG_MAX_ID_LEN {
            return Err(CatalogError::IdTooLong);
        }
        if !item.id.contains('/') {
            return Err(CatalogError::IdMissingSlash);
        }
        entries.push(CatalogEntry {
            id: item.id,
            owned_by: item.owned_by,
        });
    }
    Ok(entries)
}

/// Returns the exact intersection of preset models and the catalog.
/// Presets not in the catalog are excluded. No inference or fallback.
pub fn available_presets(catalog: &[CatalogEntry]) -> Vec<&'static PresetModel> {
    let catalog_ids: std::collections::HashSet<&str> =
        catalog.iter().map(|e| e.id.as_str()).collect();
    PRESET_MODELS
        .iter()
        .filter(|p| catalog_ids.contains(p.id))
        .collect()
}

/// Checks whether a specific model ID is a preset and is present in the catalog.
pub fn is_preset_available(catalog: &[CatalogEntry], model_id: &str) -> bool {
    if !PRESET_MODELS.iter().any(|p| p.id == model_id) {
        return false;
    }
    catalog.iter().any(|e| e.id == model_id)
}

/// Finds a preset model by ID. Returns None for non-preset IDs.
pub fn find_preset(model_id: &str) -> Option<&'static PresetModel> {
    PRESET_MODELS.iter().find(|p| p.id == model_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_json(ids: &[&str]) -> Vec<u8> {
        let data: Vec<_> = ids
            .iter()
            .map(|id| {
                let owned_by = id.split('/').next().unwrap_or("");
                serde_json::json!({"id": id, "object": "model", "owned_by": owned_by})
            })
            .collect();
        serde_json::json!({"object": "list", "data": data})
            .to_string()
            .into_bytes()
    }

    #[test]
    fn parses_valid_catalog() {
        let body = catalog_json(&[
            "cx/gpt-5.5-image",
            "ag/gemini-3.1-flash-image",
            "cx/gpt-5.4-image",
        ]);
        let entries = parse_catalog(&body).unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().any(|e| e.id == "cx/gpt-5.5-image"));
        assert!(entries.iter().any(|e| e.id == "ag/gemini-3.1-flash-image"));
    }

    #[test]
    fn rejects_non_list_object() {
        let body = br#"{"object":"page","data":[]}"#;
        assert_eq!(parse_catalog(body), Err(CatalogError::NotList));
    }

    #[test]
    fn rejects_id_without_slash() {
        let body =
            br#"{"object":"list","data":[{"id":"no-slash","object":"model","owned_by":"x"}]}"#;
        assert_eq!(parse_catalog(body), Err(CatalogError::IdMissingSlash));
    }

    #[test]
    fn rejects_empty_id() {
        let body = br#"{"object":"list","data":[{"id":"","object":"model","owned_by":"x"}]}"#;
        assert_eq!(parse_catalog(body), Err(CatalogError::EmptyId));
    }

    #[test]
    fn rejects_invalid_json() {
        assert_eq!(parse_catalog(b"not json"), Err(CatalogError::InvalidJson));
    }

    #[test]
    fn available_presets_returns_exact_intersection() {
        let body = catalog_json(&["cx/gpt-5.5-image", "cx/gpt-5.4-image"]);
        let entries = parse_catalog(&body).unwrap();
        let available = available_presets(&entries);
        assert_eq!(available.len(), 1);
        assert_eq!(available[0].id, "cx/gpt-5.5-image");
    }

    #[test]
    fn available_presets_empty_when_no_match() {
        let body = catalog_json(&["cx/gpt-5.6-image", "ag/gemini-3.2-flash-image"]);
        let entries = parse_catalog(&body).unwrap();
        let available = available_presets(&entries);
        assert!(available.is_empty());
    }

    #[test]
    fn available_presets_all_when_both_present() {
        let body = catalog_json(&["cx/gpt-5.5-image", "ag/gemini-3.1-flash-image"]);
        let entries = parse_catalog(&body).unwrap();
        let available = available_presets(&entries);
        assert_eq!(available.len(), 2);
    }

    #[test]
    fn id_drift_fails_closed() {
        let drifted = [
            "cx/gpt-5.6-image",
            "ag/gemini-3.2-flash-image",
            "cx/gpt-5.5-image-v2",
        ];
        for id in &drifted {
            assert!(find_preset(id).is_none(), "{} should not be a preset", id);
        }
    }

    #[test]
    fn is_preset_available_checks_both_preset_and_catalog() {
        let body = catalog_json(&["cx/gpt-5.5-image"]);
        let entries = parse_catalog(&body).unwrap();
        assert!(is_preset_available(&entries, "cx/gpt-5.5-image"));
        assert!(!is_preset_available(&entries, "ag/gemini-3.1-flash-image"));
        assert!(!is_preset_available(&entries, "cx/gpt-5.4-image"));
    }
}
