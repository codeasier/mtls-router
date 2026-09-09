use serde::Deserialize;

use super::error::{SafeKind, WorkbenchError};

pub const DEFAULT_CHAT_MODEL: &str = "gemini-3.8-flash";
pub const DEFAULT_IMAGE_MODEL: &str = "ag/gemini-3.1-flash-image";
pub const GPT_IMAGE_2: &str = "cx/gpt-5.5-image";
pub const SIZES: [&str; 3] = ["1024x1024", "1792x1024", "1024x1792"];
pub const MAX_CATALOG_BYTES: usize = 1 << 20;
const MAX_MODELS: usize = 1000;
const MAX_ID_BYTES: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ImageChoice {
    pub id: String,
    pub display_name: String,
    pub verified_edit: bool,
    pub allowed: bool,
}

pub fn parse_chat_models(body: &[u8]) -> Result<Vec<String>, WorkbenchError> {
    Ok(parse_ids(body, true)?
        .into_iter()
        .filter(|id| !id.contains('/'))
        .collect())
}

pub fn parse_image_models(body: &[u8]) -> Result<Vec<String>, WorkbenchError> {
    parse_ids(body, false)
}

fn parse_ids(body: &[u8], _keep_order: bool) -> Result<Vec<String>, WorkbenchError> {
    if body.len() > MAX_CATALOG_BYTES || std::str::from_utf8(body).is_err() {
        return Err(WorkbenchError::new(SafeKind::NotReady));
    }
    #[derive(Deserialize)]
    struct Catalog {
        data: Vec<Item>,
    }
    #[derive(Deserialize)]
    struct Item {
        id: String,
    }
    let catalog: Catalog =
        serde_json::from_slice(body).map_err(|_| WorkbenchError::new(SafeKind::NotReady))?;
    if catalog.data.len() > MAX_MODELS {
        return Err(WorkbenchError::new(SafeKind::NotReady));
    }
    let mut ids = Vec::new();
    for item in catalog.data {
        if !valid_id(&item.id) {
            return Err(WorkbenchError::new(SafeKind::NotReady));
        }
        if !ids.contains(&item.id) {
            ids.push(item.id);
        }
    }
    if ids.is_empty() {
        return Err(WorkbenchError::new(SafeKind::NotReady));
    }
    Ok(ids)
}

fn valid_id(id: &str) -> bool {
    if id.is_empty() || id.len() > MAX_ID_BYTES {
        return false;
    }
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let last = chars.next_back().unwrap_or(first);
    !first.is_whitespace() && !last.is_whitespace() && !id.chars().any(char::is_control)
}

pub fn image_id_visible(_id: &str) -> bool {
    true
}

pub fn select_chat_model(current: Option<&str>, catalog: &[String]) -> (Option<String>, bool) {
    if let Some(id) = current {
        let allowed = catalog.iter().any(|item| item == id);
        return (Some(id.to_owned()), allowed);
    }
    if catalog.iter().any(|item| item == DEFAULT_CHAT_MODEL) {
        return (Some(DEFAULT_CHAT_MODEL.to_owned()), true);
    }
    (None, false)
}

pub fn select_image_model(current: Option<&str>, catalog: &[String]) -> (Option<String>, bool) {
    if let Some(id) = current {
        let allowed = catalog.iter().any(|item| item == id);
        return (Some(id.to_owned()), allowed);
    }
    if catalog.iter().any(|item| item == DEFAULT_IMAGE_MODEL) {
        return (Some(DEFAULT_IMAGE_MODEL.to_owned()), true);
    }
    if let Some(first) = catalog.first() {
        return (Some(first.clone()), true);
    }
    (None, false)
}

pub fn grouped_image_choices(catalog: &[String], current: Option<&str>) -> Vec<ImageChoice> {
    let mut choices: Vec<ImageChoice> = catalog
        .iter()
        .map(|id| ImageChoice {
            id: id.clone(),
            display_name: id.clone(),
            verified_edit: id == GPT_IMAGE_2,
            allowed: true,
        })
        .collect();
    if let Some(id) = current {
        if !choices.iter().any(|choice| choice.id == id) {
            choices.insert(
                0,
                ImageChoice {
                    id: id.to_owned(),
                    display_name: id.to_owned(),
                    verified_edit: id == GPT_IMAGE_2,
                    allowed: catalog.iter().any(|item| item == id),
                },
            );
        }
    }
    choices
}

pub fn display_name_for(id: &str) -> String {
    id.to_owned()
}

pub fn verified_edit(id: &str) -> bool {
    id == GPT_IMAGE_2
}

pub fn valid_size(size: &str) -> bool {
    SIZES.contains(&size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_drops_slash_and_does_not_invent_fallback() {
        let body = include_bytes!("../../../../internal/proxy/testdata/models.json");
        let ids = parse_chat_models(body).expect("chat");
        assert!(ids.contains(&"gemini-3.8-flash".to_owned()));
        assert!(ids.iter().all(|id| !id.contains('/')));
        let (selected, ok) = select_chat_model(None, &ids);
        assert_eq!(selected.as_deref(), Some(DEFAULT_CHAT_MODEL));
        assert!(ok);
        let empty: Vec<String> = vec!["other-model".into()];
        let (selected, ok) = select_chat_model(None, &empty);
        assert_eq!(selected, None);
        assert!(!ok);
    }

    #[test]
    fn image_parses_upstream_models_without_mapping() {
        let body = include_bytes!("../../../../internal/proxy/testdata/models_image.json");
        let ids = parse_image_models(body).expect("image");
        assert!(ids.contains(&DEFAULT_IMAGE_MODEL.to_owned()));
        assert!(ids.contains(&GPT_IMAGE_2.to_owned()));
        assert!(ids.contains(&"cx/gpt-5.6-image".to_owned()));
        let choices = grouped_image_choices(&ids, Some("unknown/drifted-model"));
        assert!(choices
            .iter()
            .any(|c| c.display_name == DEFAULT_IMAGE_MODEL));
        assert!(choices.iter().any(|c| c.id == "cx/gpt-5.6-image"));
        let drifted = choices
            .iter()
            .find(|c| c.id == "unknown/drifted-model")
            .expect("kept");
        assert!(!drifted.allowed);
        assert_eq!(drifted.display_name, "unknown/drifted-model");
    }
}
