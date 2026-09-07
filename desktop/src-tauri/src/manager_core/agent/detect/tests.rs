use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::manager_core::agent::paths::{claude_paths, codex_paths, opencode_paths};
use crate::manager_core::agent::types::RecoveryReason;

fn test_detector(home: &Path) -> Detector {
    Detector {
        home_dir: Some(home.to_path_buf()),
        getenv: |_| String::new(),
    }
}

fn must_detect(home: &Path) -> Vec<State> {
    test_detector(home).detect().expect("detect")
}

fn write_file(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn has_reason(reasons: &[RecoveryReason], want: RecoveryReason) -> bool {
    reasons.contains(&want)
}

#[test]
fn detect_returns_all_agents_and_respects_environment_paths() {
    let home = tempfile_home();
    let claude_dir = home.join("claude-config");
    let opencode_path = home.join("open-code").join("custom.jsonc");
    let codex_home = home.join("codex-home");
    let states = must_detect(&home);
    assert_eq!(states.len(), 3);
    assert!(states
        .iter()
        .all(|state| state.detected && state.command.is_empty()));
    assert!(states[0].path.ends_with("settings.json"));
    assert_eq!(
        claude_paths(&home, &claude_dir.to_string_lossy()).config_path,
        claude_dir.join("settings.json")
    );
    assert_eq!(
        opencode_paths(&home, &opencode_path.to_string_lossy()).config_path,
        opencode_path
    );
    assert_eq!(
        codex_paths(&home, &codex_home.to_string_lossy()).config_path,
        codex_home.join("config.toml")
    );
}

fn tempfile_home() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mtls-agent-detect-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn detect_treats_supported_agents_as_configurable_without_config() {
    let home = tempfile_home();
    for state in must_detect(&home) {
        assert!(state.detected);
        assert_eq!(state.command, "");
        assert!(!state.exists);
        assert!(state.writable);
    }
}

#[test]
fn detect_claude_configured_invalid_and_missing() {
    let home = tempfile_home();
    let path = home.join(".claude").join("settings.json");
    write_file(
        &path,
        r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:19443","ANTHROPIC_AUTH_TOKEN":"claude-secret-canary","ANTHROPIC_MODEL":"dynamic-main","ANTHROPIC_DEFAULT_HAIKU_MODEL":"dynamic-main","ANTHROPIC_DEFAULT_SONNET_MODEL":"dynamic-sonnet","ANTHROPIC_DEFAULT_OPUS_MODEL":"dynamic-main"}}"#,
    );
    let states = must_detect(&home);
    assert!(states[0].exists && states[0].writable && states[0].configured && !states[0].invalid);

    write_file(&path, r#"{"env":"#);
    let states = must_detect(&home);
    assert!(states[0].invalid && !states[0].configured);

    fs::remove_file(&path).unwrap();
    let states = must_detect(&home);
    assert!(!states[0].exists && !states[0].configured && !states[0].invalid && states[0].writable);
}

#[test]
fn detect_claude_accepts_utf8_bom() {
    let home = tempfile_home();
    let path = home.join(".claude").join("settings.json");
    write_file(
        &path,
        "\u{feff}{\"env\":{\"ANTHROPIC_BASE_URL\":\"http://127.0.0.1:19443\",\"ANTHROPIC_AUTH_TOKEN\":\"claude-secret-canary\",\"ANTHROPIC_MODEL\":\"dynamic-main\",\"ANTHROPIC_DEFAULT_HAIKU_MODEL\":\"dynamic-main\",\"ANTHROPIC_DEFAULT_SONNET_MODEL\":\"dynamic-sonnet\",\"ANTHROPIC_DEFAULT_OPUS_MODEL\":\"dynamic-main\"}}",
    );
    let state = &must_detect(&home)[0];
    assert!(state.exists && state.writable && state.configured && !state.invalid);
    assert!(!state.recovery.eligible && state.recovery.reasons.is_empty());
}

#[test]
fn detect_classifies_recovery_eligibility() {
    let home = tempfile_home();
    let path = home.join(".claude").join("settings.json");
    write_file(&path, r#"{"env":"#);
    let state = &must_detect(&home)[0];
    assert!(state.invalid && state.recovery.eligible);
    assert!(has_reason(
        &state.recovery.files[0].reasons,
        RecoveryReason::SyntaxInvalid
    ));

    write_file(&path, "[]");
    let state = &must_detect(&home)[0];
    assert!(state.invalid && !state.recovery.eligible);
    assert!(has_reason(
        &state.recovery.reasons,
        RecoveryReason::UnsupportedStructure
    ));

    write_file(&path, r#"{"env":[]}"#);
    let state = &must_detect(&home)[0];
    assert!(!state.invalid && !state.recovery.eligible);
    assert!(has_reason(
        &state.recovery.reasons,
        RecoveryReason::UnsupportedStructure
    ));
}

#[test]
fn detect_recovery_rejects_oversized_non_regular_and_linked_targets() {
    let home = tempfile_home();
    let path = home.join(".claude").join("settings.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, vec![0u8; MAX_CONFIG_SIZE + 1]).unwrap();
    let state = &must_detect(&home)[0];
    assert!(!state.recovery.eligible);
    assert!(has_reason(
        &state.recovery.reasons,
        RecoveryReason::Oversized
    ));

    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let state = &must_detect(&home)[0];
    assert!(!state.recovery.eligible);
    assert!(has_reason(
        &state.recovery.reasons,
        RecoveryReason::NonRegular
    ));

    fs::remove_dir(&path).unwrap();
    let target = home.join("target.json");
    write_file(&target, r#"{"env":"#);
    #[cfg(unix)]
    {
        if std::os::unix::fs::symlink(&target, &path).is_err() {
            return;
        }
        let state = &must_detect(&home)[0];
        assert!(!state.invalid && !state.recovery.eligible);
        assert!(has_reason(&state.recovery.reasons, RecoveryReason::Linked));
    }
}

#[test]
fn detect_opencode_jsonc_configured_and_provider_invalid() {
    let home = tempfile_home();
    let path = home.join(".config").join("opencode").join("opencode.jsonc");
    write_file(
        &path,
        r#"{
  // comments and trailing commas are accepted
  "provider": {
    "mtls-router": {
      "options": {
        "baseURL": "http://127.0.0.1:19099/v1//literal",
        "apiKey": "open-code-secret-canary",
      },
    },
  },
}"#,
    );
    let states = must_detect(&home);
    assert_eq!(states[1].format, Format::Jsonc);
    assert!(!states[1].configured && !states[1].invalid);

    write_file(
        &path,
        r#"{"model":"mtls-router/dynamic-main","provider":{"mtls-router":{"npm":"@ai-sdk/openai-compatible","name":"mtls-router","options":{"baseURL":"http://127.0.0.1:19443/v1","apiKey":"open-code-secret-canary"},"models":{"dynamic-main":{"name":"dynamic-main"}}}}}"#,
    );
    let states = must_detect(&home);
    assert!(states[1].configured && !states[1].invalid);

    write_file(
        &path,
        r#"{"model":"mtls-router/dynamic-main","provider":{"mtls-router":{"npm":"@ai-sdk/openai-compatible","name":"CodeasierRouter","options":{"baseURL":"http://127.0.0.1:19443/v1","apiKey":"open-code-secret-canary"},"models":{"dynamic-main":{"name":"dynamic-main"}}}}}"#,
    );
    let states = must_detect(&home);
    assert!(states[1].configured && !states[1].invalid);

    write_file(&path, r#"{"provider":"invalid"}"#);
    let states = must_detect(&home);
    assert!(states[1].invalid && !states[1].configured);
}

#[test]
fn detect_codex_configured_and_invalid() {
    let home = tempfile_home();
    let config_path = home.join(".codex").join("config.toml");
    let auth_path = home.join(".codex").join("auth.json");
    write_file(
        &config_path,
        r#"model_provider = "custom"
model = "gpt-5.5"
disable_response_storage = true
approval_policy = "on-request"
notify = [
  "keep",
]

[model_providers.custom]
name = "9router"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:19099/v1"

[features]
js_repl = false
"#,
    );
    write_file(&auth_path, r#"{"OPENAI_API_KEY":"codex-secret-canary"}"#);
    let states = must_detect(&home);
    assert!(
        states[2].detected
            && !states[2].configured
            && states[2].migratable
            && !states[2].invalid
            && states[2].writable
    );

    write_file(
        &config_path,
        r#"model_provider = "mtls-router"
model = "dynamic-main"
cli_auth_credentials_store = "file"
model_reasoning_effort = "medium"
[model_providers.mtls-router]
name = "mtls-router"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:19443/v1"
"#,
    );
    write_file(
        &auth_path,
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"codex-secret-canary"}"#,
    );
    let states = must_detect(&home);
    assert!(states[2].configured && !states[2].migratable && !states[2].invalid);

    write_file(
        &config_path,
        r#"model_provider = "mtls-router"
model = "dynamic-main"
cli_auth_credentials_store = "file"
[model_providers.mtls-router]
name = "CodeasierRouter"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:19443/v1"
"#,
    );
    let states = must_detect(&home);
    assert!(states[2].configured && !states[2].migratable && !states[2].invalid);

    write_file(&config_path, "model =\n");
    let states = must_detect(&home);
    assert!(states[2].invalid && !states[2].configured);

    fs::remove_file(&config_path).unwrap();
    write_file(&auth_path, r#"{"OPENAI_API_KEY":"#);
    let states = must_detect(&home);
    assert!(!states[2].exists && states[2].invalid && !states[2].configured);
}

#[test]
fn detect_codex_accepts_dots_inside_quoted_keys() {
    let home = tempfile_home();
    write_file(
        &home.join(".codex").join("config.toml"),
        r#"model_provider = "mtls-router"
model = "dynamic-main"
cli_auth_credentials_store = "file"

[model_providers.mtls-router]
name = "mtls-router"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:19443/v1"

[projects.'e:\minecraft\example\.minecraft\versions\demo']
trust_level = "trusted"

[projects."e:\\minecraft\\example\\version.2\\demo"]
trust_level = "trusted"
"#,
    );
    write_file(
        &home.join(".codex").join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"codex-secret-canary"}"#,
    );
    let state = &must_detect(&home)[2];
    assert!(state.exists && state.writable && state.configured && !state.invalid);
    assert!(!state.recovery.eligible);
    assert_eq!(state.recovery.files.len(), 2);
    assert_eq!(state.recovery.files[0].role, "config");
    assert_eq!(state.recovery.files[1].role, "auth");
}

#[test]
fn detect_codex_rejects_duplicate_toml_keys() {
    let home = tempfile_home();
    write_file(
        &home.join(".codex").join("config.toml"),
        "model = \"first\"\nmodel = \"second\"\n",
    );
    let state = &must_detect(&home)[2];
    assert!(state.invalid && !state.configured);
}

#[test]
fn detection_result_cannot_return_stored_api_keys() {
    let home = tempfile_home();
    write_file(
        &home.join(".claude").join("settings.json"),
        r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:19099","ANTHROPIC_AUTH_TOKEN":"claude-secret-canary"}}"#,
    );
    write_file(
        &home.join(".config").join("opencode").join("opencode.json"),
        r#"{"provider":{"mtls-router":{"options":{"baseURL":"http://127.0.0.1:19099/v1","apiKey":"open-code-secret-canary"}}}}"#,
    );
    write_file(
        &home.join(".codex").join("config.toml"),
        r#"model_provider = "custom"
model = "gpt-5.5"
disable_response_storage = true
[model_providers.custom]
name = "9router"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:19099/v1""#,
    );
    write_file(
        &home.join(".codex").join("auth.json"),
        r#"{"OPENAI_API_KEY":"codex-secret-canary"}"#,
    );
    let encoded = serde_json::to_string(&must_detect(&home)).unwrap();
    for canary in [
        "claude-secret-canary",
        "open-code-secret-canary",
        "codex-secret-canary",
    ] {
        assert!(!encoded.contains(canary), "{encoded}");
    }
}
