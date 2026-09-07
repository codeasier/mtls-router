use std::collections::HashMap;

use super::canonical::{JsonNumber, JsonValue};
use super::types::{invalid, Agent, Config, ValidationError, MAX_CONFIG_SIZE};
use super::validate::parse_config;

pub fn decode(
    data: &[u8],
    selected: &[Agent],
    catalog: &[String],
) -> Result<Config, ValidationError> {
    let object = decode_config_object(data)?;
    parse_config(&object, selected, catalog, true)
}

pub fn decode_structural(data: &[u8]) -> Result<Config, ValidationError> {
    let object = decode_config_object(data)?;
    let selected: Vec<Agent> = Agent::all()
        .into_iter()
        .filter(|agent| object.contains_key(agent.as_str()))
        .collect();
    parse_config(&object, &selected, &[], false)
}

pub fn object_from_value(value: &JsonValue) -> Option<HashMap<String, JsonValue>> {
    value.as_object().cloned()
}

fn decode_config_object(data: &[u8]) -> Result<HashMap<String, JsonValue>, ValidationError> {
    if data.len() > MAX_CONFIG_SIZE {
        return Err(invalid("", "size"));
    }
    if !std::str::from_utf8(data).is_ok() {
        return Err(invalid("", "utf8"));
    }
    if !valid_unicode_escapes(data) {
        return Err(invalid("", "unicode"));
    }
    match decode_value(data)? {
        JsonValue::Object(object) => Ok(object),
        _ => Err(invalid("", "object")),
    }
}

pub fn decode_value(data: &[u8]) -> Result<JsonValue, ValidationError> {
    let mut parser = Parser { data, index: 0 };
    parser.skip_ws();
    let value = parser.read_value("")?;
    parser.skip_ws();
    if parser.index != parser.data.len() {
        return Err(invalid("", "trailing_json"));
    }
    Ok(value)
}

fn valid_unicode_escapes(data: &[u8]) -> bool {
    let mut in_string = false;
    let mut index = 0;
    while index < data.len() {
        if data[index] == b'"' {
            in_string = !in_string;
            index += 1;
            continue;
        }
        if !in_string || data[index] != b'\\' {
            index += 1;
            continue;
        }
        index += 1;
        if index >= data.len() {
            return false;
        }
        if data[index] != b'u' {
            index += 1;
            continue;
        }
        let Some(first) = hex_quad(data, index + 1) else {
            return false;
        };
        index += 5;
        if (0xd800..=0xdbff).contains(&first) {
            if index + 6 >= data.len() || data[index + 1] != b'\\' || data[index + 2] != b'u' {
                return false;
            }
            let Some(second) = hex_quad(data, index + 3) else {
                return false;
            };
            if !(0xdc00..=0xdfff).contains(&second) {
                return false;
            }
            index += 6;
        } else if (0xdc00..=0xdfff).contains(&first) {
            return false;
        }
    }
    true
}

fn hex_quad(data: &[u8], start: usize) -> Option<u16> {
    if start + 4 > data.len() {
        return None;
    }
    let mut value = 0u16;
    for byte in &data[start..start + 4] {
        value <<= 4;
        value += match byte {
            b'0'..=b'9' => u16::from(byte - b'0'),
            b'a'..=b'f' => u16::from(byte - b'a') + 10,
            b'A'..=b'F' => u16::from(byte - b'A') + 10,
            _ => return None,
        };
    }
    Some(value)
}

struct Parser<'a> {
    data: &'a [u8],
    index: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.index < self.data.len()
            && matches!(self.data[self.index], b' ' | b'\t' | b'\n' | b'\r')
        {
            self.index += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.index).copied()
    }

    fn read_value(&mut self, path: &str) -> Result<JsonValue, ValidationError> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.read_object(path),
            Some(b'[') => self.read_array(path),
            Some(b'"') => self.read_string(path).map(JsonValue::String),
            Some(b't') => self.read_literal(path, b"true", JsonValue::Bool(true)),
            Some(b'f') => self.read_literal(path, b"false", JsonValue::Bool(false)),
            Some(b'n') => self.read_literal(path, b"null", JsonValue::Null),
            Some(b'-' | b'0'..=b'9') => self.read_number(path),
            _ => Err(invalid(path, "json")),
        }
    }

    fn read_literal(
        &mut self,
        path: &str,
        expected: &[u8],
        value: JsonValue,
    ) -> Result<JsonValue, ValidationError> {
        if self.data[self.index..].starts_with(expected) {
            self.index += expected.len();
            Ok(value)
        } else {
            Err(invalid(path, "json"))
        }
    }

    fn read_object(&mut self, path: &str) -> Result<JsonValue, ValidationError> {
        self.index += 1;
        let mut object = HashMap::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b'}') {
                self.index += 1;
                return Ok(JsonValue::Object(object));
            }
            if object.is_empty() && self.peek() != Some(b'"') {
                return Err(invalid(path, "json"));
            }
            if !object.is_empty() {
                if self.peek() != Some(b',') {
                    return Err(invalid(path, "json"));
                }
                self.index += 1;
                self.skip_ws();
                if self.peek() == Some(b'}') {
                    return Err(invalid(path, "json"));
                }
            }
            let key = self.read_string(path)?;
            let child = pointer(path, &key);
            if object.contains_key(&key) {
                return Err(invalid(child, "duplicate_key"));
            }
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(invalid(path, "json"));
            }
            self.index += 1;
            let value = self.read_value(&child)?;
            object.insert(key, value);
        }
    }

    fn read_array(&mut self, path: &str) -> Result<JsonValue, ValidationError> {
        self.index += 1;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.index += 1;
                return Ok(JsonValue::Array(items));
            }
            if !items.is_empty() {
                if self.peek() != Some(b',') {
                    return Err(invalid(path, "json"));
                }
                self.index += 1;
                self.skip_ws();
                if self.peek() == Some(b']') {
                    return Err(invalid(path, "json"));
                }
            }
            if items.len() >= 1024 {
                return Err(invalid(path, "array_size"));
            }
            let child = format!("{path}/{}", items.len());
            items.push(self.read_value(&child)?);
        }
    }

    fn read_string(&mut self, path: &str) -> Result<String, ValidationError> {
        if self.peek() != Some(b'"') {
            return Err(invalid(path, "json"));
        }
        self.index += 1;
        let mut out = String::new();
        while self.index < self.data.len() {
            let byte = self.data[self.index];
            if byte == b'"' {
                self.index += 1;
                if out.len() > 16 << 10 {
                    return Err(invalid(path, "string_size"));
                }
                return Ok(out);
            }
            if byte == b'\\' {
                self.index += 1;
                let escaped = self.peek().ok_or_else(|| invalid(path, "json"))?;
                self.index += 1;
                match escaped {
                    b'"' | b'\\' | b'/' => out.push(escaped as char),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let first =
                            hex_quad(self.data, self.index).ok_or_else(|| invalid(path, "json"))?;
                        self.index += 4;
                        if (0xd800..=0xdbff).contains(&first) {
                            if self.index + 6 > self.data.len()
                                || self.data[self.index] != b'\\'
                                || self.data[self.index + 1] != b'u'
                            {
                                return Err(invalid(path, "json"));
                            }
                            self.index += 2;
                            let second = hex_quad(self.data, self.index)
                                .ok_or_else(|| invalid(path, "json"))?;
                            self.index += 4;
                            let combined = 0x10000
                                + (((first as u32) - 0xd800) << 10)
                                + ((second as u32) - 0xdc00);
                            out.push(
                                char::from_u32(combined).ok_or_else(|| invalid(path, "json"))?,
                            );
                        } else {
                            out.push(
                                char::from_u32(first as u32)
                                    .ok_or_else(|| invalid(path, "json"))?,
                            );
                        }
                    }
                    _ => return Err(invalid(path, "json")),
                }
                continue;
            }
            if byte < 0x20 {
                return Err(invalid(path, "json"));
            }
            let width = utf8_width(byte).ok_or_else(|| invalid(path, "json"))?;
            if self.index + width > self.data.len() {
                return Err(invalid(path, "json"));
            }
            let slice = &self.data[self.index..self.index + width];
            let ch = std::str::from_utf8(slice)
                .map_err(|_| invalid(path, "json"))?
                .chars()
                .next()
                .ok_or_else(|| invalid(path, "json"))?;
            out.push(ch);
            self.index += width;
        }
        Err(invalid(path, "json"))
    }

    fn read_number(&mut self, path: &str) -> Result<JsonValue, ValidationError> {
        let start = self.index;
        if self.peek() == Some(b'-') {
            self.index += 1;
        }
        if self.peek() == Some(b'0') {
            self.index += 1;
        } else if matches!(self.peek(), Some(b'1'..=b'9')) {
            self.index += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.index += 1;
            }
        } else {
            return Err(invalid(path, "json"));
        }
        if self.peek() == Some(b'.') {
            self.index += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(invalid(path, "json"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.index += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.index += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.index += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(invalid(path, "json"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.index += 1;
            }
        }
        let raw = std::str::from_utf8(&self.data[start..self.index])
            .map_err(|_| invalid(path, "json"))?;
        let parsed = raw.parse::<f64>().map_err(|_| invalid(path, "number"))?;
        if !parsed.is_finite() {
            return Err(invalid(path, "number"));
        }
        Ok(JsonValue::Number(JsonNumber(raw.to_owned())))
    }
}

fn utf8_width(byte: u8) -> Option<usize> {
    match byte {
        0x00..=0x7f => Some(1),
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

pub fn pointer(base: &str, key: &str) -> String {
    let escaped = key.replace('~', "~0").replace('/', "~1");
    format!("{base}/{escaped}")
}
