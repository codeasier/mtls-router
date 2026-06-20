#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

source_setup() {
  local sourced
  sourced="$(mktemp)"
  sed '/^main "\$@"$/d' "$ROOT/setup.sh" >"$sourced"
  # shellcheck source=/dev/null
  source "$sourced"
  rm -f "$sourced"
}

assert_eq() {
  local got="$1" want="$2" label="$3"
  [[ "$got" == "$want" ]] || {
    printf 'FAIL %s: got %q want %q\n' "$label" "$got" "$want" >&2
    return 1
  }
}

test_opencode_jsonc_migration_creates_json() (
  source_setup
  local home jsonc_path json_path result_path backup_path migration_output
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  mkdir -p "$home/.config/opencode"
  jsonc_path="$home/.config/opencode/opencode.jsonc"
  json_path="$home/.config/opencode/opencode.json"
  cat >"$jsonc_path" <<'JSONC'
{
  // existing default model
  "model": "anthropic/claude-sonnet-4-6",
  "provider": {
    "anthropic": {
      "options": {
        "apiKey": "{env:ANTHROPIC_API_KEY}",
        "baseURL": "https://example.com/v1//literal",
        "literal": "/* not a comment */",
      },
    },
  },
}
JSONC

  migration_output="$(migrate_opencode_jsonc_to_json "$jsonc_path" "$json_path")"
  result_path="$(printf '%s\n' "$migration_output" | sed -n '1p')"
  backup_path="$(printf '%s\n' "$migration_output" | sed -n '2p')"

  assert_eq "$result_path" "$json_path" "jsonc migration written path"
  [[ -f "$json_path" ]] || { printf 'FAIL: opencode.json not created\n' >&2; return 1; }
  [[ -f "$backup_path" ]] || { printf 'FAIL: backup not created at %q\n' "$backup_path" >&2; return 1; }
  local model api_key base_url literal
  model="$(jq -r '.model' "$json_path")"
  assert_eq "$model" "anthropic/claude-sonnet-4-6" "jsonc migration model preserved"
  api_key="$(jq -r '.provider.anthropic.options.apiKey' "$json_path")"
  assert_eq "$api_key" "{env:ANTHROPIC_API_KEY}" "jsonc migration anthropic apiKey preserved"
  base_url="$(jq -r '.provider.anthropic.options.baseURL' "$json_path")"
  assert_eq "$base_url" "https://example.com/v1//literal" "jsonc migration URL literal preserved"
  literal="$(jq -r '.provider.anthropic.options.literal' "$json_path")"
  assert_eq "$literal" "/* not a comment */" "jsonc migration comment-like string preserved"
)

test_opencode_jsonc_migration_rejects_invalid_jsonc_without_changes() (
  source_setup
  local home jsonc_path json_path before
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  mkdir -p "$home/.config/opencode"
  jsonc_path="$home/.config/opencode/opencode.jsonc"
  json_path="$home/.config/opencode/opencode.json"
  printf '{\n  // invalid JSONC\n  "provider": {\n}\n  "model": "anthropic/claude"\n}\n' >"$jsonc_path"
  before="$(<"$jsonc_path")"

  if ( migrate_opencode_jsonc_to_json "$jsonc_path" "$json_path" ) >/dev/null 2>&1; then
    printf 'FAIL: expected invalid JSONC migration to fail\n' >&2
    return 1
  fi

  [[ ! -e "$json_path" ]] || { printf 'FAIL: opencode.json should not be created\n' >&2; return 1; }
  local after
  after="$(<"$jsonc_path")"
  assert_eq "$after" "$before" "invalid jsonc unchanged"
  local baks
  baks=( "$jsonc_path".bak-* )
  [[ ! -e "${baks[0]}" ]] || { printf 'FAIL: expected no backup for invalid JSONC migration\n' >&2; return 1; }
)

test_opencode_jsonc_migration_rejects_existing_json() (
  source_setup
  local home jsonc_path json_path jsonc_before json_before
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  mkdir -p "$home/.config/opencode"
  jsonc_path="$home/.config/opencode/opencode.jsonc"
  json_path="$home/.config/opencode/opencode.json"
  printf '{\n  "provider": {\n    "anthropic": { "options": { "apiKey": "{env:ANTHROPIC_API_KEY}" } }\n  }\n}\n' >"$jsonc_path"
  printf '{"provider":{"existing":{"options":{"apiKey":"keep-json"}}}}\n' >"$json_path"
  jsonc_before="$(<"$jsonc_path")"
  json_before="$(<"$json_path")"

  if ( migrate_opencode_jsonc_to_json "$jsonc_path" "$json_path" ) >/dev/null 2>&1; then
    printf 'FAIL: expected migration to reject existing opencode.json\n' >&2
    return 1
  fi

  local jsonc_after json_after
  jsonc_after="$(<"$jsonc_path")"
  json_after="$(<"$json_path")"
  assert_eq "$jsonc_after" "$jsonc_before" "existing json rejection leaves jsonc unchanged"
  assert_eq "$json_after" "$json_before" "existing json rejection leaves json unchanged"
)

test_opencode_config_creates_when_missing() (
  source_setup
  local home path result
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  path="$home/.config/opencode/opencode.json"
  result="$(configure_opencode "$path")"
  local first_line
  first_line="$(printf '%s\n' "$result" | sed -n '1p')"
  assert_eq "$first_line" "$path" "written path"
  [[ -f "$path" ]] || { printf 'FAIL: file not created\n' >&2; return 1; }
  local npm name base_url api_key has_5_5 has_5_4
  npm="$(jq -r '.provider."mtls-router".npm' "$path")"
  assert_eq "$npm" "@ai-sdk/openai-compatible" "mtls-router npm"
  name="$(jq -r '.provider."mtls-router".name' "$path")"
  assert_eq "$name" "mtls-router" "mtls-router name"
  base_url="$(jq -r '.provider."mtls-router".options.baseURL' "$path")"
  assert_eq "$base_url" "http://127.0.0.1:19099/v1" "mtls-router baseURL"
  api_key="$(jq -r '.provider."mtls-router".options.apiKey' "$path")"
  assert_eq "$api_key" "{UserApiKey}" "mtls-router apiKey"
  has_5_5="$(jq -r '.provider."mtls-router".models | has("gpt-5.5")' "$path")"
  assert_eq "$has_5_5" "true" "gpt-5.5 model present"
  has_5_4="$(jq -r '.provider."mtls-router".models | has("gpt-5.4")' "$path")"
  assert_eq "$has_5_4" "true" "gpt-5.4 model present"
)

test_opencode_config_preserves_other_providers() (
  source_setup
  local home path
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  mkdir -p "$home/.config/opencode"
  path="$home/.config/opencode/opencode.json"
  cat >"$path" <<'JSON'
{
  "model": "anthropic/claude-sonnet-4-6",
  "provider": {
    "anthropic": { "options": { "apiKey": "{env:ANTHROPIC_API_KEY}" } }
  }
}
JSON
  local result
  result="$(configure_opencode "$path")"
  local model
  model="$(jq -r '.model' "$path")"
  assert_eq "$model" "anthropic/claude-sonnet-4-6" "model preserved"
  local has_mtls
  has_mtls="$(jq -r '.provider."mtls-router" | type' "$path")"
  assert_eq "$has_mtls" "object" "mtls-router present"
  local has_anthropic
  has_anthropic="$(jq -r '.provider.anthropic.options.apiKey' "$path")"
  assert_eq "$has_anthropic" "{env:ANTHROPIC_API_KEY}" "anthropic preserved"
  local second_line
  second_line="$(printf '%s\n' "$result" | sed -n '2p')"
  [[ "$second_line" == *.bak-* ]] || { printf 'FAIL: expected backup path, got %q\n' "$second_line" >&2; return 1; }
)

test_opencode_config_migrates_simple_jsonc() (
  source_setup
  local home jsonc_path json_path
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  mkdir -p "$home/.config/opencode"
  jsonc_path="$home/.config/opencode/opencode.jsonc"
  json_path="$home/.config/opencode/opencode.json"
  cat >"$jsonc_path" <<'JSON'
{
  "$schema": "https://opencode.ai/config.json"
}
JSON
  local result
  result="$(configure_opencode "$jsonc_path")"
  local first_line
  first_line="$(printf '%s\n' "$result" | sed -n '1p')"
  assert_eq "$first_line" "$json_path" "migrated written path"
  [[ -f "$json_path" ]] || { printf 'FAIL: json file not created\n' >&2; return 1; }
  local schema has_mtls second_line
  schema="$(jq -r '."$schema"' "$json_path")"
  assert_eq "$schema" "https://opencode.ai/config.json" "schema preserved"
  has_mtls="$(jq -r '.provider."mtls-router" | type' "$json_path")"
  assert_eq "$has_mtls" "object" "mtls-router present after jsonc migration"
  second_line="$(printf '%s\n' "$result" | sed -n '2p')"
  [[ "$second_line" == "$jsonc_path".bak-* ]] || { printf 'FAIL: expected jsonc backup path, got %q\n' "$second_line" >&2; return 1; }
)

test_opencode_config_rejects_invalid_json() (
  source_setup
  local home path
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  mkdir -p "$home/.config/opencode"
  path="$home/.config/opencode/opencode.json"
  echo 'not json' >"$path"
  local before
  before="$(cat "$path")" 2>/dev/null || before=""
  if ( configure_opencode "$path" ) >/dev/null 2>&1; then
    printf 'FAIL: expected invalid JSON to be rejected\n' >&2
    return 1
  fi
  local after
  after="$(cat "$path")" 2>/dev/null || after=""
  assert_eq "$after" "$before" "invalid json unchanged"
  local baks
  baks=( "$path".bak-* )
  [[ ! -e "${baks[0]}" ]] || { printf 'FAIL: expected no backup for invalid JSON rejection\n' >&2; return 1; }
)

test_opencode_config_rejects_non_object_provider() (
  source_setup
  local home path
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  mkdir -p "$home/.config/opencode"
  path="$home/.config/opencode/opencode.json"
  echo '{"provider": "openai"}' >"$path"
  local before
  before="$(cat "$path")" 2>/dev/null || before=""
  if ( configure_opencode "$path" ) >/dev/null 2>&1; then
    printf 'FAIL: expected non-object provider to be rejected\n' >&2
    return 1
  fi
  local after
  after="$(cat "$path")" 2>/dev/null || after=""
  assert_eq "$after" "$before" "non-object provider unchanged"
  local baks
  baks=( "$path".bak-* )
  [[ ! -e "${baks[0]}" ]] || { printf 'FAIL: expected no backup for non-object provider rejection\n' >&2; return 1; }
)

test_opencode_config_uses_real_api_key_when_provided() (
  source_setup
  local home path
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  path="$home/.config/opencode/opencode.json"
  configure_opencode "$path" "real-key-456" >/dev/null
  local api_key
  api_key="$(jq -r '.provider."mtls-router".options.apiKey' "$path")"
  assert_eq "$api_key" "real-key-456" "real provider apiKey written"
  local placeholder
  placeholder="$(grep -c '{UserApiKey}' "$path" || true)"
  assert_eq "$placeholder" "0" "placeholder removed"
)

test_opencode_jsonc_migration_creates_json
test_opencode_jsonc_migration_rejects_invalid_jsonc_without_changes
test_opencode_jsonc_migration_rejects_existing_json
test_opencode_config_creates_when_missing
test_opencode_config_preserves_other_providers
test_opencode_config_migrates_simple_jsonc
test_opencode_config_rejects_invalid_json
test_opencode_config_rejects_non_object_provider
test_opencode_config_uses_real_api_key_when_provided
