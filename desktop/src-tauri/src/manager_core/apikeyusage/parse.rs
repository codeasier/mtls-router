use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::Number;

use super::error::{invalid_period, response_invalid, UsageError};

pub const MAX_MODELS: usize = 64;
pub const MAX_QUOTAS: usize = 32;
const MAX_ID_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Period {
    OneHour,
    TwelveHours,
    TwentyFourHours,
    SevenDays,
    ThirtyDays,
    Today,
    ThisWeek,
    ThisMonth,
}

impl Period {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OneHour => "1h",
            Self::TwelveHours => "12h",
            Self::TwentyFourHours => "24h",
            Self::SevenDays => "7d",
            Self::ThirtyDays => "30d",
            Self::Today => "today",
            Self::ThisWeek => "this_week",
            Self::ThisMonth => "this_month",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "1h" => Some(Self::OneHour),
            "12h" => Some(Self::TwelveHours),
            "24h" => Some(Self::TwentyFourHours),
            "7d" => Some(Self::SevenDays),
            "30d" => Some(Self::ThirtyDays),
            "today" => Some(Self::Today),
            "this_week" => Some(Self::ThisWeek),
            "this_month" => Some(Self::ThisMonth),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetPeriod {
    Day,
    Week,
    Month,
}

impl BudgetPeriod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "day" => Some(Self::Day),
            "week" => Some(Self::Week),
            "month" => Some(Self::Month),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaUnit {
    Usd,
    Tokens,
    Requests,
}

impl QuotaUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usd => "usd",
            Self::Tokens => "tokens",
            Self::Requests => "requests",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "usd" => Some(Self::Usd),
            "tokens" => Some(Self::Tokens),
            "requests" => Some(Self::Requests),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub period: Period,
    pub as_of: String,
    pub summary: Summary,
    pub quota: Option<Quota>,
    pub quotas: Vec<ProviderQuota>,
    pub by_model: Vec<Model>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub requests: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cost: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Quota {
    pub used: f64,
    pub limit: Option<f64>,
    pub unit: QuotaUnit,
    pub resets_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderQuota {
    pub provider: String,
    pub period: BudgetPeriod,
    pub used: f64,
    pub limit: f64,
    pub unit: QuotaUnit,
    pub resets_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub model: String,
    pub requests: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cost: f64,
}

pub fn normalize_period(value: &str) -> Result<Period, UsageError> {
    match value.trim() {
        "" | "7d" => Ok(Period::SevenDays),
        other => Period::parse(other).ok_or_else(invalid_period),
    }
}

pub fn parse(body: &[u8], period: Period) -> Result<Snapshot, UsageError> {
    if std::str::from_utf8(body).is_err() {
        return Err(response_invalid());
    }
    let parsed: ParsedSnapshot = serde_json::from_slice(body).map_err(|_| response_invalid())?;
    if parsed.period != period {
        return Err(response_invalid());
    }
    Ok(Snapshot {
        period,
        as_of: parsed.as_of,
        summary: parsed.summary,
        quota: parsed.quota,
        quotas: parsed.quotas,
        by_model: parsed.by_model,
    })
}

struct ParsedSnapshot {
    period: Period,
    as_of: String,
    summary: Summary,
    quota: Option<Quota>,
    quotas: Vec<ProviderQuota>,
    by_model: Vec<Model>,
}

impl<'de> Deserialize<'de> for ParsedSnapshot {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(SnapshotVisitor)
    }
}

struct SnapshotVisitor;

impl<'de> Visitor<'de> for SnapshotVisitor {
    type Value = ParsedSnapshot;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("usage object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut seen = HashSet::new();
        let mut period = None;
        let mut as_of = String::new();
        let mut summary = None;
        let mut quota = None;
        let mut quotas = Vec::new();
        let mut by_model = None;
        while let Some(key) = map.next_key::<String>()? {
            if !seen_field(&mut seen, &key) {
                return Err(de::Error::custom("invalid field"));
            }
            match key.as_str() {
                "period" => {
                    let value = map.next_value::<String>()?;
                    period =
                        Some(Period::parse(&value).ok_or_else(|| de::Error::custom("period"))?);
                }
                "as_of" => {
                    let value = map.next_value::<String>()?;
                    if !valid_time(&value) {
                        return Err(de::Error::custom("as_of"));
                    }
                    as_of = value;
                }
                "summary" => summary = Some(map.next_value::<Summary>()?),
                "quota" => quota = map.next_value::<Option<Quota>>()?,
                "quotas" => {
                    quotas = map
                        .next_value::<Option<QuotaRows>>()?
                        .map(|rows| rows.0)
                        .unwrap_or_default();
                }
                "by_model" => by_model = Some(map.next_value::<ModelRows>()?.0),
                _ => {
                    let _: IgnoredAny = map.next_value()?;
                }
            }
        }
        if !seen.contains("period") || !seen.contains("summary") || !seen.contains("by_model") {
            return Err(de::Error::custom("missing required"));
        }
        Ok(ParsedSnapshot {
            period: period.ok_or_else(|| de::Error::custom("period"))?,
            as_of,
            summary: summary.ok_or_else(|| de::Error::custom("summary"))?,
            quota,
            quotas,
            by_model: by_model.ok_or_else(|| de::Error::custom("by_model"))?,
        })
    }
}

impl<'de> Deserialize<'de> for Summary {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let fields = ScalarObject::deserialize(deserializer)?.0;
        Ok(Summary {
            requests: required_count(&fields, "requests")?,
            prompt_tokens: required_count(&fields, "prompt_tokens")?,
            completion_tokens: required_count(&fields, "completion_tokens")?,
            cost: required_amount(&fields, "cost")?,
        })
    }
}

impl<'de> Deserialize<'de> for Quota {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let fields = ScalarObject::deserialize(deserializer)?.0;
        let used = required_amount(&fields, "used")?;
        let unit = match fields.get("unit") {
            Some(Scalar::String(value)) => {
                QuotaUnit::parse(value).ok_or_else(|| de::Error::custom("unit"))?
            }
            _ => return Err(de::Error::custom("unit")),
        };
        let mut quota = Quota {
            used,
            limit: None,
            unit,
            resets_at: String::new(),
        };
        if let Some(limit) = fields.get("limit") {
            if !matches!(limit, Scalar::Null) {
                quota.limit = Some(decode_amount(limit)?);
            }
        }
        if let Some(resets) = fields.get("resets_at") {
            if !matches!(resets, Scalar::Null) {
                let Scalar::String(value) = resets else {
                    return Err(de::Error::custom("resets_at"));
                };
                if !valid_time(value) {
                    return Err(de::Error::custom("resets_at"));
                }
                quota.resets_at = value.clone();
            }
        }
        Ok(quota)
    }
}

impl<'de> Deserialize<'de> for ProviderQuota {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let fields = ScalarObject::deserialize(deserializer)?.0;
        let provider = match fields.get("provider") {
            Some(Scalar::String(value)) if valid_id(value) => value.clone(),
            _ => return Err(de::Error::custom("provider")),
        };
        let period = match fields.get("period") {
            Some(Scalar::String(value)) => {
                BudgetPeriod::parse(value).ok_or_else(|| de::Error::custom("period"))?
            }
            _ => return Err(de::Error::custom("period")),
        };
        let used = required_amount(&fields, "used")?;
        let limit = required_positive_amount(&fields, "limit")?;
        let unit = match fields.get("unit") {
            Some(Scalar::String(value)) if value == "usd" => QuotaUnit::Usd,
            _ => return Err(de::Error::custom("unit")),
        };
        let resets_at = match fields.get("resets_at") {
            Some(Scalar::String(value)) if valid_time(value) => value.clone(),
            _ => return Err(de::Error::custom("resets_at")),
        };
        Ok(ProviderQuota {
            provider,
            period,
            used,
            limit,
            unit,
            resets_at,
        })
    }
}

impl<'de> Deserialize<'de> for Model {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let fields = ScalarObject::deserialize(deserializer)?.0;
        let model = match fields.get("model") {
            Some(Scalar::String(value)) if valid_id(value) => value.clone(),
            _ => return Err(de::Error::custom("model")),
        };
        Ok(Model {
            model,
            requests: required_count(&fields, "requests")?,
            prompt_tokens: required_count(&fields, "prompt_tokens")?,
            completion_tokens: required_count(&fields, "completion_tokens")?,
            cost: required_amount(&fields, "cost")?,
        })
    }
}

struct ModelRows(Vec<Model>);

impl<'de> Deserialize<'de> for ModelRows {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ModelsVisitor;
        impl<'de> Visitor<'de> for ModelsVisitor {
            type Value = ModelRows;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("model rows")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut models = Vec::new();
                while let Some(model) = seq.next_element::<Model>()? {
                    models.push(model);
                    if models.len() > MAX_MODELS {
                        return Err(de::Error::custom("too many models"));
                    }
                }
                Ok(ModelRows(models))
            }
        }
        deserializer.deserialize_seq(ModelsVisitor)
    }
}

struct QuotaRows(Vec<ProviderQuota>);

impl<'de> Deserialize<'de> for QuotaRows {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct QuotasVisitor;
        impl<'de> Visitor<'de> for QuotasVisitor {
            type Value = QuotaRows;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("provider quotas")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut quotas = Vec::new();
                while let Some(quota) = seq.next_element::<ProviderQuota>()? {
                    quotas.push(quota);
                    if quotas.len() > MAX_QUOTAS {
                        return Err(de::Error::custom("too many quotas"));
                    }
                }
                Ok(QuotaRows(quotas))
            }
        }
        deserializer.deserialize_seq(QuotasVisitor)
    }
}

struct ScalarObject(HashMap<String, Scalar>);

impl<'de> Deserialize<'de> for ScalarObject {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = ScalarObject;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("scalar object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut seen = HashSet::new();
                let mut fields = HashMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !seen_field(&mut seen, &key) {
                        return Err(de::Error::custom("invalid field"));
                    }
                    fields.insert(key, map.next_value::<Scalar>()?);
                }
                Ok(ScalarObject(fields))
            }
        }
        deserializer.deserialize_map(ObjectVisitor)
    }
}

enum Scalar {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
}

impl<'de> Deserialize<'de> for Scalar {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ScalarVisitor;
        impl<'de> Visitor<'de> for ScalarVisitor {
            type Value = Scalar;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("scalar")
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Scalar::Null)
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Scalar::Null)
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(Scalar::Bool(value))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(Scalar::Number(value.into()))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(Scalar::Number(value.into()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Number::from_f64(value)
                    .map(Scalar::Number)
                    .ok_or_else(|| de::Error::custom("number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(Scalar::String(value.to_owned()))
            }
        }
        deserializer.deserialize_any(ScalarVisitor)
    }
}

fn seen_field(seen: &mut HashSet<String>, key: &str) -> bool {
    if key.is_empty() || sensitive_field(key) || !seen.insert(key.to_owned()) {
        return false;
    }
    true
}

fn sensitive_field(name: &str) -> bool {
    let normalized: String = name
        .chars()
        .filter(|ch| *ch != '_' && *ch != '-')
        .flat_map(|ch| ch.to_lowercase())
        .collect();
    matches!(
        normalized.as_str(),
        "apikey"
            | "key"
            | "token"
            | "authorization"
            | "secret"
            | "password"
            | "credential"
            | "bearer"
            | "accesstoken"
            | "clientsecret"
    )
}

fn required_count<E: de::Error>(fields: &HashMap<String, Scalar>, key: &str) -> Result<i64, E> {
    match fields.get(key) {
        Some(Scalar::Number(number)) => decode_count(number),
        _ => Err(de::Error::custom(key)),
    }
}

fn required_amount<E: de::Error>(fields: &HashMap<String, Scalar>, key: &str) -> Result<f64, E> {
    match fields.get(key) {
        Some(value) => decode_amount(value),
        None => Err(de::Error::custom(key)),
    }
}

fn required_positive_amount<E: de::Error>(
    fields: &HashMap<String, Scalar>,
    key: &str,
) -> Result<f64, E> {
    let amount = required_amount(fields, key)?;
    if amount <= 0.0 {
        return Err(de::Error::custom(key));
    }
    Ok(amount)
}

fn decode_count<E: de::Error>(number: &Number) -> Result<i64, E> {
    let text = number.to_string();
    if text.contains(['.', 'e', 'E']) {
        return Err(de::Error::custom("count"));
    }
    let count = number.as_i64().ok_or_else(|| de::Error::custom("count"))?;
    if count < 0 {
        return Err(de::Error::custom("count"));
    }
    Ok(count)
}

fn decode_amount<E: de::Error>(value: &Scalar) -> Result<f64, E> {
    let Scalar::Number(number) = value else {
        return Err(de::Error::custom("amount"));
    };
    let text = number.to_string();
    if text.eq_ignore_ascii_case("nan")
        || text.eq_ignore_ascii_case("inf")
        || text.eq_ignore_ascii_case("+inf")
        || text.eq_ignore_ascii_case("-inf")
    {
        return Err(de::Error::custom("amount"));
    }
    let amount = number.as_f64().ok_or_else(|| de::Error::custom("amount"))?;
    if amount < 0.0 || !amount.is_finite() {
        return Err(de::Error::custom("amount"));
    }
    Ok(amount)
}

fn valid_time(value: &str) -> bool {
    !value.is_empty() && chrono::DateTime::parse_from_rfc3339(value).is_ok()
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
    if first.is_whitespace() || last.is_whitespace() {
        return false;
    }
    id.chars().all(|ch| !ch.is_control())
}
