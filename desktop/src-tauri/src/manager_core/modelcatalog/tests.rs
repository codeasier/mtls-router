use std::time::{Duration, Instant};

use crate::manager_core::httpget::{TestRequest, TestResponse, TestServer};
use crate::protocol::ErrorCode;

use super::client::{Client, Request, MAX_BODY_BYTES};
use super::error::code_of;
use super::parse::{parse, MAX_MODELS};
use super::simplify::{
    parse_simplify, parse_simplify_value, set_simplify_for_test, SIMPLIFY_DEFAULT,
};

fn assert_code(error: super::CatalogError, want: ErrorCode) {
    assert_eq!(code_of(&error), want, "{}", error);
}

#[test]
fn parse_normalizes_catalog() {
    let body = r#"{
  "ignored":{"nested":[1,true,null,{"value":"unused"}]},
  "data":[
    {"id":"z-model","owned_by":"service"},
    {"created":1,"id":"é-model"},
    {"id":"a-model"},
    {"id":"z-model"}
  ]
}"#;
    let models = parse(body.as_bytes(), false).expect("parse");
    assert_eq!(models, ["a-model", "z-model", "é-model"]);
}

#[test]
fn parse_rejects_invalid_responses() {
    let cases: &[(&str, &[u8])] = &[
        ("empty", b""),
        (
            "invalid UTF-8",
            &[
                b'{', b'"', b'd', b'a', b't', b'a', b'"', b':', b'[', b'{', b'"', b'i', b'd', b'"',
                b':', b'"', 0xff, b'"', b'}', b']', b'}',
            ],
        ),
        ("root null", b"null"),
        ("root array", b"[]"),
        ("missing data", b"{}"),
        ("duplicate data", br#"{"data":[],"data":[]}"#),
        ("data null", br#"{"data":null}"#),
        ("data object", br#"{"data":{}}"#),
        ("item scalar", br#"{"data":["model"]}"#),
        ("item array", br#"{"data":[[]]}"#),
        ("missing id", br#"{"data":[{}]}"#),
        ("duplicate id", br#"{"data":[{"id":"a","id":"b"}]}"#),
        ("id null", br#"{"data":[{"id":null}]}"#),
        ("id number", br#"{"data":[{"id":1}]}"#),
        ("empty id", br#"{"data":[{"id":""}]}"#),
        ("leading ASCII whitespace", br#"{"data":[{"id":" model"}]}"#),
        (
            "trailing Unicode whitespace",
            "{\"data\":[{\"id\":\"model\u{3000}\"}]}".as_bytes(),
        ),
        (
            "leading non-breaking space",
            "{\"data\":[{\"id\":\"\u{00a0}model\"}]}".as_bytes(),
        ),
        ("ASCII control", br#"{"data":[{"id":"model\u0000x"}]}"#),
        ("Unicode control", br#"{"data":[{"id":"model\u0085x"}]}"#),
        ("malformed", br#"{"data":["#),
        ("trailing object", br#"{"data":[{"id":"a"}]} {}"#),
        ("trailing scalar", br#"{"data":[{"id":"a"}]} true"#),
        (
            "malformed ignored field",
            br#"{"ignored":[},"data":[{"id":"a"}]}"#,
        ),
    ];
    for (name, body) in cases {
        let error = parse(*body, false).expect_err(name);
        assert_eq!(code_of(&error), ErrorCode::ModelResponseInvalid, "{name}");
    }
    let oversized = format!(r#"{{"data":[{{"id":"{}"}}]}}"#, "a".repeat(257));
    assert_eq!(
        code_of(&parse(oversized.as_bytes(), false).expect_err("257")),
        ErrorCode::ModelResponseInvalid
    );
}

#[test]
fn parse_id_boundaries() {
    for id in [
        "a".repeat(256),
        "model with interior space".to_owned(),
        "模型-é".to_owned(),
        "model\u{200b}zero-width-space".to_owned(),
    ] {
        let body = serde_json::json!({"data":[{"id": id}]}).to_string();
        let models = parse(body.as_bytes(), false).unwrap_or_else(|_| panic!("Parse({id})"));
        assert_eq!(models, [id.clone()]);
    }
}

#[test]
fn parse_catalog_count_boundaries() {
    let mut body = String::from(r#"{"data":["#);
    for index in 0..MAX_MODELS {
        if index > 0 {
            body.push(',');
        }
        body.push_str(&format!(r#"{{"id":"model-{index:04}"}}"#));
    }
    body.push_str("]}");
    let models = parse(body.as_bytes(), false).expect("1000 models");
    assert_eq!(models.len(), MAX_MODELS);

    let mut overflow = String::from(r#"{"data":["#);
    for index in 0..=MAX_MODELS {
        if index > 0 {
            overflow.push(',');
        }
        overflow.push_str(&format!(r#"{{"id":"model-{index:04}"}}"#));
    }
    overflow.push_str("]}");
    assert_code(
        parse(overflow.as_bytes(), false).expect_err("overflow"),
        ErrorCode::ModelResponseInvalid,
    );

    let duplicates = format!(
        r#"{{"data":[{}{{"id":"same"}}]}}"#,
        r#"{"id":"same"},"#.repeat(MAX_MODELS + 10)
    );
    let models = parse(duplicates.as_bytes(), false).expect("duplicates");
    assert_eq!(models, ["same"]);
}

#[test]
fn parse_empty_catalog() {
    assert_code(
        parse(br#"{"data":[]}"#, false).expect_err("empty"),
        ErrorCode::ModelCatalogEmpty,
    );
}

#[test]
fn parse_simplifies_catalog() {
    let body = r#"{"data":[
  {"id":"provider/model"},
  {"id":"plain"},
  {"id":"wide／slash"},
  {"id":"back\\slash"},
  {"id":"provider/nested/model"},
  {"id":"plain"}
]}"#
    .as_bytes();
    assert_eq!(
        parse(body, false).expect("disabled"),
        [
            "back\\slash",
            "plain",
            "provider/model",
            "provider/nested/model",
            "wide／slash"
        ]
    );
    let simplified = parse(body, true).expect("enabled");
    assert_eq!(simplified, ["back\\slash", "plain", "wide／slash"]);
    assert_eq!(simplified.capacity(), simplified.len());
}

#[test]
fn parse_simplification_preserves_validation_and_limits() {
    assert_code(
        parse(br#"{"data":[{"id":"provider/model"}]}"#, true).expect_err("filtered"),
        ErrorCode::ModelCatalogEmpty,
    );
    assert_code(
        parse(
            br#"{"data":[{"id":"valid"},{"id":"provider/model\u0000"}]}"#,
            true,
        )
        .expect_err("malformed"),
        ErrorCode::ModelResponseInvalid,
    );
    let mut body = String::from(r#"{"data":["#);
    for index in 0..=MAX_MODELS {
        if index > 0 {
            body.push(',');
        }
        body.push_str(&format!(r#"{{"id":"provider/model-{index:04}"}}"#));
    }
    body.push_str("]}");
    assert_code(
        parse(body.as_bytes(), true).expect_err("limit before filter"),
        ErrorCode::ModelResponseInvalid,
    );
}

#[test]
fn parse_simplify_default() {
    assert_eq!(SIMPLIFY_DEFAULT, "True");
    assert_eq!(parse_simplify().expect("default"), true);
}

#[test]
fn parse_simplify_accepts_case_insensitive_booleans() {
    for (value, want) in [
        ("true", true),
        ("TRUE", true),
        ("TrUe", true),
        ("false", false),
        ("FALSE", false),
        ("FaLsE", false),
    ] {
        let _guard = set_simplify_for_test(value);
        assert_eq!(parse_simplify().expect(value), want);
    }
}

#[test]
fn parse_simplify_rejects_invalid_values() {
    for value in [
        "",
        " ",
        "\t",
        " true",
        "false ",
        "1",
        "0",
        "yes",
        "no",
        "falſe",
        "truK",
        "enabled-secret",
    ] {
        let _guard = set_simplify_for_test(value);
        let error = parse_simplify().expect_err(value);
        assert_eq!(error, "invalid embedded simplify value");
    }
    assert_eq!(
        parse_simplify_value("falſe"),
        Err("invalid embedded simplify value")
    );
}

#[test]
fn fetch_sends_exact_request() {
    const KEY: &str = "catalog-key-canary";
    let server = TestServer::start(|request: &TestRequest| {
        assert_eq!(request.path, "/v1/models");
        assert!(request.query.is_empty());
        assert_eq!(request.authorization, format!("Bearer {KEY}"));
        TestResponse::json(200, br#"{"data":[{"id":"b"},{"id":"a"}]}"#.to_vec())
    });
    let models = Client::new(false)
        .fetch(Request {
            url: format!("{}/v1/models", server.url()),
            api_key: KEY.into(),
        })
        .expect("fetch");
    assert_eq!(models, ["a", "b"]);
}

#[test]
fn fetch_status_mapping_and_sanitization() {
    const KEY: &str = "secret-key-canary";
    const BODY: &str = "response-body-canary";
    for (status, code) in [
        (401, ErrorCode::ModelAuthFailed),
        (403, ErrorCode::ModelAuthFailed),
        (400, ErrorCode::ModelDiscoveryFailed),
        (429, ErrorCode::ModelDiscoveryFailed),
        (500, ErrorCode::ModelDiscoveryFailed),
    ] {
        let server =
            TestServer::start(move |_| TestResponse::json(status, BODY.as_bytes().to_vec()));
        let error = Client::new(false)
            .fetch(Request {
                url: format!("{}/v1/models", server.url()),
                api_key: KEY.into(),
            })
            .expect_err("status");
        let message = error.to_string();
        assert_code(error, code);
        assert!(!message.contains(KEY));
        assert!(!message.contains(BODY));
        assert!(!message.contains(&server.url()));
    }
}

#[test]
fn fetch_does_not_follow_redirect() {
    const KEY: &str = "redirect-key-canary";
    let target_hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hits = target_hits.clone();
    let server = TestServer::start(move |request| {
        if request.path == "/key-target" {
            hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        TestResponse {
            status: 302,
            body: Vec::new(),
            connection_close: false,
        }
    });
    let error = Client::new(false)
        .fetch(Request {
            url: format!("{}/v1/models", server.url()),
            api_key: KEY.into(),
        })
        .expect_err("redirect");
    assert_code(error, ErrorCode::ModelDiscoveryFailed);
    assert_eq!(target_hits.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn fetch_body_limit() {
    let prefix = r#"{"data":[{"id":"at-limit"}],"padding":""#;
    let suffix = r#""}"#;
    let at_limit = format!(
        "{prefix}{}{suffix}",
        "x".repeat(MAX_BODY_BYTES - prefix.len() - suffix.len())
    );
    let server = TestServer::start(move |_| TestResponse::json(200, at_limit.clone().into_bytes()));
    let models = Client::new(false)
        .fetch(Request {
            url: format!("{}/v1/models", server.url()),
            api_key: "limit-key".into(),
        })
        .expect("at limit");
    assert_eq!(models, ["at-limit"]);

    let over = "x".repeat(MAX_BODY_BYTES + 1);
    let server = TestServer::start(move |_| TestResponse::json(200, over.clone().into_bytes()));
    let error = Client::new(false)
        .fetch(Request {
            url: format!("{}/v1/models", server.url()),
            api_key: "limit-key".into(),
        })
        .expect_err("over");
    assert_code(error, ErrorCode::ModelResponseInvalid);
}

#[test]
fn fetch_validates_response_catalog() {
    let unique_overflow = {
        let mut body = String::from(r#"{"data":["#);
        for index in 0..=MAX_MODELS {
            if index > 0 {
                body.push(',');
            }
            body.push_str(&format!(r#"{{"id":"model-{index:04}"}}"#));
        }
        body.push_str("]}");
        body
    };
    for (name, body, code) in [
        (
            "malformed JSON",
            r#"{"data":["#.into(),
            ErrorCode::ModelResponseInvalid,
        ),
        (
            "trailing JSON",
            r#"{"data":[{"id":"a"}]} {}"#.into(),
            ErrorCode::ModelResponseInvalid,
        ),
        (
            "invalid item",
            r#"{"data":[{"id":1}]}"#.into(),
            ErrorCode::ModelResponseInvalid,
        ),
        (
            "boundary whitespace",
            r#"{"data":[{"id":" model"}]}"#.into(),
            ErrorCode::ModelResponseInvalid,
        ),
        (
            "too many unique models",
            unique_overflow,
            ErrorCode::ModelResponseInvalid,
        ),
        (
            "empty catalog",
            r#"{"data":[]}"#.into(),
            ErrorCode::ModelCatalogEmpty,
        ),
    ] {
        let server = TestServer::start({
            let body = body.clone();
            move |_| TestResponse::json(200, body.clone().into_bytes())
        });
        let error = Client::new(false)
            .fetch(Request {
                url: format!("{}/v1/models", server.url()),
                api_key: "response-key".into(),
            })
            .expect_err(name);
        assert_eq!(code_of(&error), code, "{name}");
        assert!(!error.to_string().contains("response-key"));
    }
}

#[test]
fn fetch_timeouts_during_headers() {
    let server = TestServer::start(|_| {
        std::thread::sleep(Duration::from_millis(200));
        TestResponse::json(200, br#"{"data":[{"id":"late"}]}"#.to_vec())
    });
    let started = Instant::now();
    let error = Client::new(false)
        .with_timeout(Duration::from_millis(50))
        .fetch(Request {
            url: format!("{}/v1/models", server.url()),
            api_key: "timeout-key".into(),
        })
        .expect_err("timeout");
    assert_code(error, ErrorCode::ModelDiscoveryFailed);
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
}

#[test]
fn fetch_honors_timeout_and_rejects_invalid_endpoints() {
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = hits.clone();
    let server = TestServer::start(move |_| {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        TestResponse::json(200, br#"{"data":[{"id":"x"}]}"#.to_vec())
    });
    for endpoint in [
        format!("{}/v1/models?query=1", server.url()),
        format!("{}/v1/models?", server.url()),
        format!("{}/v1/models#fragment", server.url()),
        format!("{}/other", server.url()),
        "not a URL".into(),
    ] {
        let error = Client::new(false)
            .fetch(Request {
                url: endpoint,
                api_key: "unsent-key".into(),
            })
            .expect_err("invalid");
        assert_code(error, ErrorCode::ModelDiscoveryFailed);
    }
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn fetch_propagates_simplify_setting() {
    let server = TestServer::start(|_| {
        TestResponse::json(
            200,
            br#"{"data":[{"id":"provider/model"},{"id":"plain"}]}"#.to_vec(),
        )
    });
    let full = Client::new(false)
        .fetch(Request {
            url: format!("{}/v1/models", server.url()),
            api_key: "simplify-key".into(),
        })
        .expect("full");
    assert_eq!(full, ["plain", "provider/model"]);
    let simple = Client::new(true)
        .fetch(Request {
            url: format!("{}/v1/models", server.url()),
            api_key: "simplify-key".into(),
        })
        .expect("simple");
    assert_eq!(simple, ["plain"]);
}
