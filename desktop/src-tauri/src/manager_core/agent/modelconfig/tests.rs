use super::*;
use std::collections::HashMap;

use base64::Engine;

fn jcs_vectors_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../internal/manager/agent/modelconfig/testdata/jcs-vectors.json")
}

#[test]
fn jcs_vectors_match_go() {
    let data = std::fs::read(jcs_vectors_path()).expect("jcs vectors");
    let vectors: serde_json::Value = serde_json::from_slice(&data).expect("vectors json");
    for vector in vectors.as_array().expect("array") {
        let name = vector["name"].as_str().unwrap();
        let agents: Vec<Agent> = vector["agents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| Agent::parse(value.as_str().unwrap()).unwrap())
            .collect();
        let catalog: Vec<String> = vector["catalog"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect();
        let input = vector["input"].as_str().unwrap();
        let want = vector["canonical"].as_str().unwrap();
        let config = decode(input.as_bytes(), &agents, &catalog)
            .unwrap_or_else(|error| panic!("{name}: decode failed: {error}"));
        let got = String::from_utf8(canonical(&config).expect(name)).expect("utf8");
        assert_eq!(got, want, "{name}");
    }
}

#[test]
fn decode_rejects_invalid_documents() {
    let base = r#"{"version":1,"opencode":{"default_model":"m","models":{"m":{}}}}"#;
    let trailing = format!("{base} {{}}");
    let extra = format!(
        "{},\"codex\":{{\"model\":\"m\"}}}}",
        &base[..base.len() - 1]
    );
    let cases: &[(&[u8], &[Agent], &[String], &str)] = &[
        (b"{\"\xff\"}", &[Agent::OpenCode], &["m".into()], "utf8"),
        (
            br#"{"version":1,"version":1,"opencode":{"default_model":"m","models":{"m":{}}}}"#,
            &[Agent::OpenCode],
            &["m".into()],
            "duplicate_key",
        ),
        (
            trailing.as_bytes(),
            &[Agent::OpenCode],
            &["m".into()],
            "trailing_json",
        ),
        (
            br#"{"version":1,"opencode":{"default_model":"m","models":{"m":{"name":"\ud800"}}}}"#,
            &[Agent::OpenCode],
            &["m".into()],
            "unicode",
        ),
        (
            br#"{"version":1,"opencode":{"default_model":"m","models":{"m":{"bogus":true}}}}"#,
            &[Agent::OpenCode],
            &["m".into()],
            "unknown_field",
        ),
        (
            br#"{"version":1}"#,
            &[Agent::OpenCode],
            &["m".into()],
            "section_scope",
        ),
        (
            extra.as_bytes(),
            &[Agent::OpenCode],
            &["m".into()],
            "section_scope",
        ),
        (
            base.as_bytes(),
            &[Agent::OpenCode, Agent::OpenCode],
            &["m".into()],
            "unique_agents",
        ),
        (
            base.as_bytes(),
            &[Agent::OpenCode],
            &["other".into()],
            "catalog",
        ),
        (
            br#"{"version":1,"opencode":{"default_model":"x","models":{"m":{}}}}"#,
            &[Agent::OpenCode],
            &["m".into(), "x".into()],
            "selected_model",
        ),
        (
            br#"{"version":1,"codex":{"model":"m","context_window":9007199254740992}}"#,
            &[Agent::Codex],
            &["m".into()],
            "positive_integer",
        ),
    ];
    for (input, agents, catalog, rule) in cases {
        let error = decode(input, agents, catalog).expect_err("expected reject");
        assert_eq!(
            error.rule,
            *rule,
            "input {}",
            String::from_utf8_lossy(input)
        );
    }
}

#[test]
fn claude_budgets_and_opencode_variants_round_trip() {
    let input = br#"{"version":1,"claude":{"primary":{"model":"m"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true},"context_window":353400,"max_output_tokens":100000},"opencode":{"default_model":"m","models":{"m":{"variants":{"medium":{"reasoningEffort":"medium"},"custom":{"reasoningSummary":"auto"}}}}}}"#;
    let config = decode(input, &[Agent::Claude, Agent::OpenCode], &["m".into()]).unwrap();
    assert_eq!(config.claude.as_ref().unwrap().context_window, Some(353400));
    assert_eq!(
        config.claude.as_ref().unwrap().max_output_tokens,
        Some(100000)
    );
    let got = String::from_utf8(canonical(&config).unwrap()).unwrap();
    assert!(got.contains(r#""context_window":353400"#));
    assert!(got.contains(r#""max_output_tokens":100000"#));
    assert!(got.contains(
        r#""variants":{"custom":{"reasoningSummary":"auto"},"medium":{"reasoningEffort":"medium"}}"#
    ));
}

#[test]
fn decode_structural_skips_catalog_and_still_rejects_keys() {
    let input = br#"{"version":1,"claude":{"primary":{"model":"not-live","context":"1m"},"haiku":{"inherit_primary":true},"sonnet":{"model":"also-not-live"},"opus":{"inherit_primary":true}},"codex":{"model":"codex-not-live"}}"#;
    let config = decode_structural(input).unwrap();
    assert!(config.claude.is_some() && config.codex.is_some() && config.opencode.is_none());
    assert!(decode(input, &[Agent::Claude, Agent::Codex], &[]).is_err());
    let error = decode_structural(br#"{"version":1}"#).unwrap_err();
    assert_eq!(error.rule, "agents");
}

#[test]
fn protected_extension_fields_are_rejected() {
    let error = decode(
        br#"{"version":1,"opencode":{"default_model":"m","models":{"m":{"options":{"api-Key":"x"}}}}}"#,
        &[Agent::OpenCode],
        &["m".into()],
    )
    .unwrap_err();
    assert_eq!(error.rule, "protected_path");
}

#[test]
fn deep_merge_is_recursive_and_non_mutating() {
    let mut nested = HashMap::new();
    nested.insert("x".into(), JsonValue::Number(JsonNumber("1".into())));
    nested.insert(
        "y".into(),
        JsonValue::Array(vec![JsonValue::Number(JsonNumber("1".into()))]),
    );
    let mut base = HashMap::new();
    base.insert("a".into(), JsonValue::Object(nested));
    base.insert("b".into(), JsonValue::Bool(true));
    let mut overlay_nested = HashMap::new();
    overlay_nested.insert("z".into(), JsonValue::Number(JsonNumber("2".into())));
    overlay_nested.insert(
        "y".into(),
        JsonValue::Array(vec![JsonValue::Number(JsonNumber("3".into()))]),
    );
    let mut overlay = HashMap::new();
    overlay.insert("a".into(), JsonValue::Object(overlay_nested));
    overlay.insert("b".into(), JsonValue::String("new".into()));
    let got = deep_merge(&base, &overlay);
    let a = got["a"].as_object().unwrap();
    assert_eq!(a["x"].as_number().unwrap().as_str(), "1");
    assert_eq!(a["z"].as_number().unwrap().as_str(), "2");
    if let JsonValue::Object(inner) = &base["a"] {
        assert_eq!(inner["x"].as_number().unwrap().as_str(), "1");
    }
}

#[test]
fn catalog_tokens_bind_context_and_reject_tampering() {
    let signer = TokenSigner::new(&[7u8; 32], "generation-1234567890").unwrap();
    let token = signer
        .sign_catalog(CatalogClaims {
            version: 0,
            models: vec!["z".into(), "a".into(), "a".into()],
            agents: vec![Agent::Codex, Agent::Claude],
            owner: "cli".into(),
            router_base_url: "http://127.0.0.1:19099".into(),
            deployment_id: "deployment-a".into(),
            protocol_version: "4".into(),
            simplify: true,
            canonicalization: String::new(),
            key_generation: String::new(),
        })
        .unwrap();
    let verified = signer.verify_catalog(&token).unwrap();
    assert_eq!(verified.models, vec!["a", "z"]);
    assert_eq!(verified.agents, vec![Agent::Claude, Agent::Codex]);
    assert!(verified.simplify);
    let parts: Vec<&str> = token.split('.').collect();
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[0])
        .unwrap();
    let tampered = String::from_utf8(payload)
        .unwrap()
        .replace("deployment-a", "deployment-b");
    let forged = format!(
        "{}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(tampered.as_bytes()),
        parts[1]
    );
    assert_eq!(
        signer.verify_catalog(&forged).unwrap_err(),
        TokenError::Invalid
    );
    let other = TokenSigner::new(&[0u8; 32], "generation-1234567890").unwrap();
    assert_eq!(
        other.verify_catalog(&token).unwrap_err(),
        TokenError::Invalid
    );
}

#[test]
fn catalog_token_omits_false_simplify() {
    let signer = TokenSigner::new(&[0x31u8; 32], "generation-legacy").unwrap();
    let token = signer
        .sign_catalog(CatalogClaims {
            version: 0,
            models: vec!["provider/model".into()],
            agents: vec![Agent::Claude],
            owner: "cli".into(),
            router_base_url: "http://127.0.0.1:19099".into(),
            deployment_id: "deployment".into(),
            protocol_version: "4".into(),
            simplify: false,
            canonicalization: String::new(),
            key_generation: String::new(),
        })
        .unwrap();
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(token.split('.').next().unwrap())
        .unwrap();
    assert!(!String::from_utf8(payload).unwrap().contains("simplify"));
}

#[test]
fn revision_tokens_bind_config_and_separate_mac_contexts() {
    let signer = TokenSigner::new(&[9u8; 32], "generation-r").unwrap();
    let token = signer
        .sign_revision(RevisionClaims {
            version: 0,
            agent_plans: vec![RevisionAgent {
                agent: Agent::OpenCode,
                mode: "rebuild".into(),
            }],
            canonical_config: br#"{"version":1}"#.to_vec(),
            catalog_identity: "catalog-mac".into(),
            sidecar_revision: RevisionState {
                exists: true,
                size: 1,
                mode: 0o600,
                digest: "sidecar-mac".into(),
            },
            router_base_url: "http://127.0.0.1:19099".into(),
            deployment_id: "deployment".into(),
            protocol_version: "4".into(),
            canonicalization: String::new(),
            key_generation: String::new(),
            managed_drift: true,
            requires_codex_auth_approval: false,
            drifted_agents: vec![Agent::OpenCode],
            files: vec![RevisionFile {
                agent: Agent::OpenCode,
                role: "config".into(),
                format: "json".into(),
                source_path: "/config".into(),
                target_path: "/config".into(),
                operation: "replace".into(),
                backup_required: true,
                backup_source: "/config".into(),
                source_revision: RevisionState {
                    exists: true,
                    size: 2,
                    mode: 0o600,
                    digest: "revision-mac".into(),
                },
                target_revision: RevisionState {
                    exists: true,
                    size: 2,
                    mode: 0o600,
                    digest: "revision-mac".into(),
                },
                companion_exists: false,
            }],
        })
        .unwrap();
    let verified = signer.verify_revision(&token).unwrap();
    assert!(verified.managed_drift);
    assert_eq!(verified.canonical_config, br#"{"version":1}"#);
    let mac1 = signer
        .revision_mac("agent-file", "/config", b"secret")
        .unwrap();
    let mac2 = signer
        .revision_mac("backup-verification", "/config", b"secret")
        .unwrap();
    assert_ne!(mac1, mac2);
}
