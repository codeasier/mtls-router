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
  has_5_5="$(jq -r '.provider."mtls-router".models | has("cx/gpt-5.5")' "$path")"
  assert_eq "$has_5_5" "true" "gpt-5.5 model present"
  has_5_4="$(jq -r '.provider."mtls-router".models | has("cx/gpt-5.4")' "$path")"
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

test_opencode_config_rejects_jsonc() (
  source_setup
  local home path
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  mkdir -p "$home/.config/opencode"
  path="$home/.config/opencode/opencode.jsonc"
  echo '{}' >"$path"
  local before
  before="$(cat "$path")" 2>/dev/null || before=""
  if ( configure_opencode "$path" ) >/dev/null 2>&1; then
    printf 'FAIL: expected JSONC to be rejected\n' >&2
    return 1
  fi
  local after
  after="$(cat "$path")" 2>/dev/null || after=""
  assert_eq "$after" "$before" "jsonc unchanged"
  local baks
  baks=( "$path".bak-* )
  [[ ! -e "${baks[0]}" ]] || { printf 'FAIL: expected no backup for JSONC rejection\n' >&2; return 1; }
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

test_opencode_config_creates_when_missing
test_opencode_config_preserves_other_providers
test_opencode_config_rejects_jsonc
test_opencode_config_rejects_invalid_json
test_opencode_config_rejects_non_object_provider
test_opencode_config_uses_real_api_key_when_provided
