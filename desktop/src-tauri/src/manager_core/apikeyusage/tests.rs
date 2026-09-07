use crate::manager_core::httpget::{TestRequest, TestResponse, TestServer};
use crate::protocol::ErrorCode;

use super::client::{Client, Request, MAX_BODY_BYTES};
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
fn client_fetches_exact_usage_endpoint() {
    let server = TestServer::start(|request: &TestRequest| {
        assert_eq!(request.path, "/v1/usage");
        assert_eq!(request.query, "period=7d");
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
        let status = match request.query.as_str() {
            "period=today" => 404,
            "period=30d" => 401,
            _ => 502,
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
