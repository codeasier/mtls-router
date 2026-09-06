use std::collections::HashMap;

use super::canonical::JsonValue;

pub fn deep_merge(
    base: &HashMap<String, JsonValue>,
    overlay: &HashMap<String, JsonValue>,
) -> HashMap<String, JsonValue> {
    let mut result = clone_map(base);
    for (key, value) in overlay {
        if let (JsonValue::Object(right), Some(JsonValue::Object(left))) = (value, result.get(key))
        {
            result.insert(key.clone(), JsonValue::Object(deep_merge(left, right)));
        } else {
            result.insert(key.clone(), clone_value(value));
        }
    }
    result
}

fn clone_map(value: &HashMap<String, JsonValue>) -> HashMap<String, JsonValue> {
    value
        .iter()
        .map(|(key, child)| (key.clone(), clone_value(child)))
        .collect()
}

fn clone_value(value: &JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(object) => JsonValue::Object(clone_map(object)),
        JsonValue::Array(items) => JsonValue::Array(items.iter().map(clone_value).collect()),
        other => other.clone(),
    }
}
