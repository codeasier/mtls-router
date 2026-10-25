use crate::manager_core::httpget::{TestRequest, TestResponse, TestServer};
use crate::protocol::ErrorCode;

use super::client::{window, Client, Request, MAX_BODY_BYTES};
use super::error::code_of;
use super::parse::{
    normalize_period, parse, BudgetPeriod, Period, ProviderQuota, QuotaUnit, Summary, MAX_MODELS,
    MAX_QUOTAS,
};

const USAGE_KEY: &str = "usage-key-canary-4419";

#[test]
fn parse_accepts_bounded_per_key_usage() {
    let snapshot = parse(
        br#"{
		"period":"7d",
		"as_of":"2026-08-28T00:00:00Z",
		"summary":{"requests":12,"prompt_tokens":100,"completion_tokens":20,"cost":1.5},
		"quota":{"used":1.5,"limit":100,"unit":"usd","resets_at":"2026-09-01T00:00:00Z"},
		"by_model":[{"model":"claude-sonnet","requests":12,"prompt_tokens":100,"completion_tokens":20,"cost":1.5}]
	}"#,
        Period::SevenDays,
    )
    .expect("parse");
    assert_eq!(snapshot.period, Period::SevenDays);
    assert_eq!(snapshot.as_of, "2026-08-28T00:00:00Z");
    assert_eq!(
        snapshot.summary,
        Summary {
            requests: 12,
            prompt_tokens: 100,
            completion_tokens: 20,
            cost: 1.5,
        }
    );
    let quota = snapshot.quota.expect("quota");
    assert_eq!(quota.used, 1.5);
    assert_eq!(quota.limit, Some(100.0));
    assert_eq!(quota.unit, QuotaUnit::Usd);
    assert_eq!(quota.resets_at, "2026-09-01T00:00:00Z");
    assert_eq!(snapshot.by_model.len(), 1);
    assert_eq!(snapshot.by_model[0].model, "claude-sonnet");
    assert!(snapshot.quotas.is_empty());
}

#[test]
fn parse_accepts_jump2_recorded_usage_schema() {
    let snapshot = parse(
        br#"{
		"scope":"recorded_usage",
		"coverage":["chat"],
		"apiKey":{"id":"08394fa2-8d20-40f3-a48d-666bc8ac208c","name":"key-canary"},
		"period":{"start":"2026-10-18T05:18:00.367Z","end":"2026-10-25T05:18:00.367Z","timezone":"UTC","days":7},
		"total":{"requests":1488,"promptTokens":180707282,"completionTokens":1649117,"cachedTokens":168799620,"cost":39.470644},
		"byModel":[{"model":"gpt-6.1-sol","requests":721,"promptTokens":93731145,"completionTokens":765463,"cachedTokens":88607104,"cost":37.564463}],
		"byDay":[{"date":"2026-10-18","requests":84,"cost":4.9}],
		"truncated":false,
		"currency":"USD",
		"limits":{"maxDays":31,"ratePerMinute":60,"cacheTtlSeconds":120}
	}"#,
        Period::SevenDays,
    )
    .expect("parse");
    assert_eq!(snapshot.period, Period::SevenDays);
    assert_eq!(snapshot.as_of, "2026-10-25T05:18:00.367Z");
    assert_eq!(
        snapshot.summary,
        Summary {
            requests: 1488,
            prompt_tokens: 180707282,
            completion_tokens: 1649117,
            cost: 39.470644,
        }
    );
    assert_eq!(snapshot.by_model.len(), 1);
    assert_eq!(snapshot.by_model[0].model, "gpt-6.1-sol");
    assert_eq!(snapshot.by_model[0].prompt_tokens, 93731145);
    assert_eq!(snapshot.by_model[0].completion_tokens, 765463);
    assert!(snapshot.quota.is_none());
    assert!(snapshot.quotas.is_empty());
    let exposed = format!("{snapshot:?}");
    assert!(!exposed.contains("08394fa2-8d20-40f3-a48d-666bc8ac208c"));
    assert!(!exposed.contains("key-canary"));
}

#[test]
fn parse_rejects_unsafe_jump2_key_metadata() {
    for metadata in [
        r#""metadata-secret-canary""#,
        "null",
        "[]",
        r#"{"id":"key-id","secret":"metadata-secret-canary"}"#,
        r#"{"id":"key-id","api_key":"metadata-secret-canary"}"#,
        r#"{"id":"key-id","access-token":"metadata-secret-canary"}"#,
        r#"{"id":"key-id","id":"other-id"}"#,
        r#"{"id":"key-id"},"apiKey":{"id":"other-id"}"#,
    ] {
        let body = format!(
            r#"{{"apiKey":{metadata},"period":{{"start":"2026-10-18T05:00:00Z","end":"2026-10-25T05:00:00Z"}},"total":{{"requests":0,"promptTokens":0,"completionTokens":0,"cost":0}},"byModel":[]}}"#
        );
        let error = parse(body.as_bytes(), Period::SevenDays).expect_err("unsafe metadata");
        assert_eq!(code_of(&error), ErrorCode::UsageResponseInvalid);
        assert!(!error.to_string().contains("metadata-secret-canary"));
    }
}

#[test]
fn parse_accepts_jump2_window_without_token_echo() {
    let snapshot = parse(
        br#"{
		"period":{"start":"2026-10-24T16:00:00Z","end":"2026-10-25T05:00:00Z","timezone":"UTC","days":1},
		"total":{"requests":7,"promptTokens":70,"completionTokens":7,"cachedTokens":0,"cost":0.07},
		"byModel":[]
	}"#,
        Period::Today,
    )
    .expect("parse");
    assert_eq!(snapshot.period, Period::Today);
    assert_eq!(snapshot.as_of, "2026-10-25T05:00:00Z");
    assert_eq!(snapshot.summary.requests, 7);
}

#[test]
fn parse_rejects_mixed_schemas_and_secret_fields() {
    for body in [
        br#"{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"total":{"requests":0,"promptTokens":0,"completionTokens":0,"cost":0},"by_model":[]}"#.as_slice(),
        br#"{"period":"7d","key":"canary","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"by_model":[]}"#.as_slice(),
        br#"{"total":{"requests":0,"promptTokens":0,"completionTokens":0,"cost":0},"byModel":[]}"#.as_slice(),
    ] {
        let error = parse(body, Period::SevenDays).expect_err("reject");
        assert_eq!(code_of(&error), ErrorCode::UsageResponseInvalid);
    }
}

#[test]
fn parse_allows_unknown_safe_fields_and_unlimited_quota() {
    let snapshot = parse(
        br#"{
		"period":"today",
		"extra":"ok",
		"summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0,"note":"ok"},
		"quota":{"used":0,"limit":null,"unit":"tokens"},
		"by_model":[]
	}"#,
        Period::Today,
    )
    .expect("parse");
    let quota = snapshot.quota.expect("quota");
    assert_eq!(quota.limit, None);
    assert_eq!(quota.unit, QuotaUnit::Tokens);
    assert!(snapshot.quotas.is_empty());
    assert!(snapshot.by_model.is_empty());
}

#[test]
fn parse_treats_missing_or_empty_quotas_as_absent() {
    for body in [
        br#"{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"by_model":[]}"#.as_slice(),
        br#"{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":[],"by_model":[]}"#.as_slice(),
        br#"{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":null,"by_model":[]}"#.as_slice(),
    ] {
        let snapshot = parse(body, Period::SevenDays).expect("parse");
        assert!(snapshot.quotas.is_empty());
    }
}

#[test]
fn parse_accepts_provider_quotas_without_synthesizing_overview() {
    let snapshot = parse(
        br#"{
		"period":"1h",
		"summary":{"requests":2,"prompt_tokens":4,"completion_tokens":1,"cost":0.4},
		"quotas":[
			{"provider":"*","period":"week","used":1.25,"limit":10,"unit":"usd","resets_at":"2026-09-14T00:00:00Z"},
			{"provider":"codex","period":"day","used":0.4,"limit":2,"unit":"usd","resets_at":"2026-09-08T00:00:00Z"}
		],
		"by_model":[]
	}"#,
        Period::OneHour,
    )
    .expect("parse");
    assert!(snapshot.quota.is_none());
    assert_eq!(
        snapshot.quotas,
        vec![
            ProviderQuota {
                provider: "*".into(),
                period: BudgetPeriod::Week,
                used: 1.25,
                limit: 10.0,
                unit: QuotaUnit::Usd,
                resets_at: "2026-09-14T00:00:00Z".into(),
            },
            ProviderQuota {
                provider: "codex".into(),
                period: BudgetPeriod::Day,
                used: 0.4,
                limit: 2.0,
                unit: QuotaUnit::Usd,
                resets_at: "2026-09-08T00:00:00Z".into(),
            },
        ]
    );
}

#[test]
fn parse_rejects_unsafe_or_unbounded_bodies() {
    let too_many = format!(
        r#"{{"period":"7d","summary":{{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0}},"by_model":[{}{{"model":"z","requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0}}]}}"#,
        r#"{"model":"m","requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"#
            .repeat(MAX_MODELS + 1)
    );
    let too_many_quotas = format!(
        r#"{{"period":"7d","summary":{{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0}},"quotas":[{}{{"provider":"z","period":"day","used":0,"limit":1,"unit":"usd","resets_at":"2026-09-08T00:00:00Z"}}],"by_model":[]}}"#,
        r#"{"provider":"p","period":"day","used":0,"limit":1,"unit":"usd","resets_at":"2026-09-08T00:00:00Z"},"#
            .repeat(MAX_QUOTAS + 1)
    );
    let cases = [
        (
            "period mismatch",
            r#"{"period":"today","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"by_model":[]}"#,
            Period::SevenDays,
        ),
        (
            "missing summary",
            r#"{"period":"7d","by_model":[]}"#,
            Period::SevenDays,
        ),
        (
            "missing by_model",
            r#"{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0}}"#,
            Period::SevenDays,
        ),
        (
            "negative requests",
            r#"{"period":"7d","summary":{"requests":-1,"prompt_tokens":0,"completion_tokens":0,"cost":0},"by_model":[]}"#,
            Period::SevenDays,
        ),
        (
            "fractional tokens",
            r#"{"period":"7d","summary":{"requests":1,"prompt_tokens":1.5,"completion_tokens":0,"cost":0},"by_model":[]}"#,
            Period::SevenDays,
        ),
        (
            "api_key field",
            r#"{"period":"7d","api_key":"secret","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"by_model":[]}"#,
            Period::SevenDays,
        ),
        (
            "token field",
            r#"{"period":"7d","token":"secret","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"by_model":[]}"#,
            Period::SevenDays,
        ),
        ("too many models", too_many.as_str(), Period::SevenDays),
        (
            "invalid model id",
            r#"{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"by_model":[{"model":" m","requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0}]}"#,
            Period::SevenDays,
        ),
        ("not object", r#"[]"#, Period::SevenDays),
        (
            "quota bad unit",
            r#"{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quota":{"used":0,"unit":"hours"},"by_model":[]}"#,
            Period::SevenDays,
        ),
        (
            "quotas not array",
            r#"{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":{},"by_model":[]}"#,
            Period::SevenDays,
        ),
        (
            "quotas unit tokens",
            r#"{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":[{"provider":"*","period":"day","used":0,"limit":1,"unit":"tokens","resets_at":"2026-09-08T00:00:00Z"}],"by_model":[]}"#,
            Period::SevenDays,
        ),
        (
            "quotas null limit",
            r#"{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":[{"provider":"*","period":"day","used":0,"limit":null,"unit":"usd","resets_at":"2026-09-08T00:00:00Z"}],"by_model":[]}"#,
            Period::SevenDays,
        ),
        (
            "quotas zero limit",
            r#"{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":[{"provider":"*","period":"day","used":0,"limit":0,"unit":"usd","resets_at":"2026-09-08T00:00:00Z"}],"by_model":[]}"#,
            Period::SevenDays,
        ),
        (
            "quotas usage period token",
            r#"{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":[{"provider":"*","period":"today","used":0,"limit":1,"unit":"usd","resets_at":"2026-09-08T00:00:00Z"}],"by_model":[]}"#,
            Period::SevenDays,
        ),
        (
            "too many quotas",
            too_many_quotas.as_str(),
            Period::SevenDays,
        ),
    ];
    for (name, body, period) in cases {
        let error = parse(body.as_bytes(), period).expect_err(name);
        assert_eq!(code_of(&error), ErrorCode::UsageResponseInvalid, "{name}");
        assert!(!error.to_string().contains("secret"));
    }
}

#[test]
fn normalize_period_defaults_and_rejects() {
    assert_eq!(normalize_period("").expect("default"), Period::SevenDays);
    for want in [
        Period::OneHour,
        Period::TwelveHours,
        Period::TwentyFourHours,
        Period::SevenDays,
        Period::ThirtyDays,
        Period::Today,
        Period::ThisWeek,
        Period::ThisMonth,
    ] {
        assert_eq!(normalize_period(want.as_str()).expect(want.as_str()), want);
    }
    for invalid in ["week", "month", "1d"] {
        assert_eq!(
            code_of(&normalize_period(invalid).expect_err(invalid)),
            ErrorCode::InvalidParams
        );
    }
}

#[test]
fn windows_cover_every_selected_period() {
    let now = "2026-10-25T05:18:00.367Z".parse().unwrap();
    for (period, expected_start) in [
        (Period::OneHour, "2026-10-25T04:18:00.367Z"),
        (Period::TwelveHours, "2026-10-24T17:18:00.367Z"),
        (Period::TwentyFourHours, "2026-10-24T05:18:00.367Z"),
        (Period::SevenDays, "2026-10-18T05:18:00.367Z"),
        (Period::ThirtyDays, "2026-09-25T05:18:00.367Z"),
        (Period::Today, "2026-10-25T00:00:00Z"),
        (Period::ThisWeek, "2026-10-19T00:00:00Z"),
        (Period::ThisMonth, "2026-10-01T00:00:00Z"),
    ] {
        let (start, end) = window(period, now);
        assert_eq!(
            start,
            chrono::DateTime::parse_from_rfc3339(expected_start).unwrap(),
            "{period:?}"
        );
        assert_eq!(end, now, "{period:?}");
    }
}

#[test]
fn calendar_windows_keep_utc_boundaries_at_rollovers() {
    for (now, period, expected_start) in [
        // Before Beijing 08:00 the previous UTC day/week/month still applies.
        (
            "2026-11-01T23:59:59Z",
            Period::Today,
            "2026-11-01T00:00:00Z",
        ),
        (
            "2026-11-01T23:59:59Z",
            Period::ThisWeek,
            "2026-10-26T00:00:00Z",
        ),
        (
            "2026-10-31T23:59:59Z",
            Period::ThisMonth,
            "2026-10-01T00:00:00Z",
        ),
        (
            "2026-11-02T00:00:00Z",
            Period::ThisWeek,
            "2026-11-02T00:00:00Z",
        ),
        (
            "2026-11-01T00:00:00Z",
            Period::ThisMonth,
            "2026-11-01T00:00:00Z",
        ),
        (
            "2027-01-01T12:00:00Z",
            Period::ThisWeek,
            "2026-12-28T00:00:00Z",
        ),
        (
            "2027-01-01T12:00:00Z",
            Period::ThisMonth,
            "2027-01-01T00:00:00Z",
        ),
        (
            "2024-03-01T00:00:00Z",
            Period::TwentyFourHours,
            "2024-02-29T00:00:00Z",
        ),
        (
            "2024-03-01T00:00:00Z",
            Period::ThirtyDays,
            "2024-01-31T00:00:00Z",
        ),
    ] {
        let now = now.parse().unwrap();
        let (start, end) = window(period, now);
        assert_eq!(
            start,
            chrono::DateTime::parse_from_rfc3339(expected_start).unwrap(),
            "{period:?} at {now}"
        );
        assert_eq!(end, now);
    }
}

#[test]
fn client_fetches_jump2_usage_for_every_period() {
    let server = TestServer::start(|request: &TestRequest| {
        assert_eq!(request.path, "/v1/usage");
        assert_eq!(request.authorization, format!("Bearer {USAGE_KEY}"));
        let params: std::collections::HashMap<_, _> = request
            .query
            .split('&')
            .map(|part| part.split_once('=').expect("query parameter"))
            .collect();
        assert_eq!(params.len(), 3);
        let start = chrono::DateTime::parse_from_rfc3339(params["start"]).expect("start");
        let end = chrono::DateTime::parse_from_rfc3339(params["end"]).expect("end");
        let hours = match params["period"] {
            "1h" => Some(1),
            "12h" => Some(12),
            "24h" => Some(24),
            "7d" => Some(168),
            "30d" => Some(720),
            "today" | "this_week" | "this_month" => None,
            _ => panic!("unknown period"),
        };
        assert!(start <= end);
        if let Some(hours) = hours {
            assert_eq!(end - start, chrono::Duration::hours(hours));
        }
        TestResponse::json(
            200,
            serde_json::to_vec(&serde_json::json!({
                "scope": "recorded_usage",
                "apiKey": {"id": "key-id", "name": "key-name"},
                "period": {"start": params["start"], "end": params["end"]},
                "total": {"requests": 3, "promptTokens": 9, "completionTokens": 1, "cost": 0.25},
                "byModel": []
            }))
            .unwrap(),
        )
    });
    for period in [
        Period::OneHour,
        Period::TwelveHours,
        Period::TwentyFourHours,
        Period::SevenDays,
        Period::ThirtyDays,
        Period::Today,
        Period::ThisWeek,
        Period::ThisMonth,
    ] {
        let before = chrono::Utc::now() - chrono::Duration::seconds(1);
        let snapshot = Client::new()
            .fetch(Request {
                url: format!("{}/v1/usage", server.url()),
                period,
                api_key: USAGE_KEY.into(),
            })
            .expect("jump2 usage");
        assert_eq!(snapshot.period, period);
        assert_eq!(snapshot.summary.requests, 3);
        assert_eq!(snapshot.summary.cost, 0.25);
        let as_of = chrono::DateTime::parse_from_rfc3339(&snapshot.as_of).unwrap();
        assert!(as_of >= before && as_of <= chrono::Utc::now());
    }
}

#[test]
fn client_fetches_exact_usage_endpoint() {
    let server = TestServer::start(|request: &TestRequest| {
        assert_eq!(request.path, "/v1/usage");
        let parts: Vec<&str> = request.query.split('&').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "period=7d");
        let start = parts[1].strip_prefix("start=").expect("start");
        let end = parts[2].strip_prefix("end=").expect("end");
        let start = chrono::DateTime::parse_from_rfc3339(start).expect("start time");
        let end = chrono::DateTime::parse_from_rfc3339(end).expect("end time");
        assert_eq!(end - start, chrono::Duration::days(7));
        assert_eq!(request.authorization, format!("Bearer {USAGE_KEY}"));
        TestResponse::json(
            200,
            br#"{"period":"7d","summary":{"requests":3,"prompt_tokens":9,"completion_tokens":1,"cost":0.25},"by_model":[]}"#.to_vec(),
        )
    });
    let snapshot = Client::new()
        .fetch(Request {
            url: format!("{}/v1/usage", server.url()),
            period: Period::SevenDays,
            api_key: USAGE_KEY.into(),
        })
        .expect("fetch");
    assert_eq!(snapshot.summary.requests, 3);
    assert_eq!(snapshot.summary.cost, 0.25);
}

#[test]
fn client_maps_status_and_rejects_unsafe_urls() {
    let server = TestServer::start(|request: &TestRequest| {
        let status = if request.query.starts_with("period=today&") {
            404
        } else if request.query.starts_with("period=30d&") {
            401
        } else {
            502
        };
        TestResponse::json(status, Vec::new())
    });
    for (period, code) in [
        (Period::Today, ErrorCode::UsageUnavailable),
        (Period::ThirtyDays, ErrorCode::UsageAuthFailed),
        (Period::SevenDays, ErrorCode::UsageRequestFailed),
    ] {
        let error = Client::new()
            .fetch(Request {
                url: format!("{}/v1/usage", server.url()),
                period,
                api_key: USAGE_KEY.into(),
            })
            .expect_err("status");
        assert_eq!(code_of(&error), code);
        assert!(!error.to_string().contains(USAGE_KEY));
    }
    let error = Client::new()
        .fetch(Request {
            url: format!("{}/v1/usage?period=7d", server.url()),
            period: Period::SevenDays,
            api_key: USAGE_KEY.into(),
        })
        .expect_err("preset query");
    assert_eq!(code_of(&error), ErrorCode::UsageRequestFailed);
}

#[test]
fn client_rejects_oversized_body() {
    let mut body = String::from(
        r#"{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"pad":""#,
    );
    body.push_str(&"x".repeat(MAX_BODY_BYTES));
    body.push_str(r#""}"#);
    let server = TestServer::start(move |_| TestResponse::json(200, body.clone().into_bytes()));
    let error = Client::new()
        .fetch(Request {
            url: format!("{}/v1/usage", server.url()),
            period: Period::SevenDays,
            api_key: USAGE_KEY.into(),
        })
        .expect_err("oversized");
    assert_eq!(code_of(&error), ErrorCode::UsageResponseInvalid);
}
