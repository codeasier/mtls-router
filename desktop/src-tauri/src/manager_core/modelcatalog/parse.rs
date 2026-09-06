use std::collections::BTreeSet;
use std::fmt;

use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;

use super::error::{catalog_empty, response_invalid, CatalogError};

pub const MAX_MODELS: usize = 1000;
const MAX_ID_BYTES: usize = 256;

pub fn parse(body: &[u8], simplify: bool) -> Result<Vec<String>, CatalogError> {
    if std::str::from_utf8(body).is_err() {
        return Err(response_invalid());
    }
    let parsed: CatalogRoot = serde_json::from_slice(body).map_err(|_| response_invalid())?;
    let mut models: Vec<String> = parsed.models.into_iter().collect();
    models.sort();
    if simplify {
        models.retain(|model| !model.contains('/'));
        models.shrink_to_fit();
    }
    if models.is_empty() {
        return Err(catalog_empty());
    }
    Ok(models)
}

pub(super) fn valid_id(id: &str) -> bool {
    if id.is_empty() || id.len() > MAX_ID_BYTES {
        return false;
    }
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let last = chars.next_back().unwrap_or(first);
    if first.is_whitespace() || last.is_whitespace() {
        return false;
    }
    id.chars().all(|ch| !ch.is_control())
}

struct CatalogRoot {
    models: BTreeSet<String>,
}

impl<'de> Deserialize<'de> for CatalogRoot {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(CatalogRootVisitor)
    }
}

struct CatalogRootVisitor;

impl<'de> Visitor<'de> for CatalogRootVisitor {
    type Value = CatalogRoot;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("catalog object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut models = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == "data" {
                if models.is_some() {
                    return Err(de::Error::custom("duplicate data"));
                }
                models = Some(map.next_value::<CatalogData>()?.0);
            } else {
                let _: IgnoredAny = map.next_value()?;
            }
        }
        Ok(CatalogRoot {
            models: models.ok_or_else(|| de::Error::custom("missing data"))?,
        })
    }
}

struct CatalogData(BTreeSet<String>);

impl<'de> Deserialize<'de> for CatalogData {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_seq(CatalogDataVisitor)
    }
}

struct CatalogDataVisitor;

impl<'de> Visitor<'de> for CatalogDataVisitor {
    type Value = CatalogData;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("catalog data array")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut models = BTreeSet::new();
        while let Some(item) = seq.next_element::<CatalogItem>()? {
            models.insert(item.0);
            if models.len() > MAX_MODELS {
                return Err(de::Error::custom("too many models"));
            }
        }
        Ok(CatalogData(models))
    }
}

struct CatalogItem(String);

impl<'de> Deserialize<'de> for CatalogItem {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(CatalogItemVisitor)
    }
}

struct CatalogItemVisitor;

impl<'de> Visitor<'de> for CatalogItemVisitor {
    type Value = CatalogItem;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("catalog item")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut id = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == "id" {
                if id.is_some() {
                    return Err(de::Error::custom("duplicate id"));
                }
                let value = map.next_value::<String>()?;
                if !valid_id(&value) {
                    return Err(de::Error::custom("invalid id"));
                }
                id = Some(value);
            } else {
                let _: IgnoredAny = map.next_value()?;
            }
        }
        Ok(CatalogItem(
            id.ok_or_else(|| de::Error::custom("missing id"))?,
        ))
    }
}
