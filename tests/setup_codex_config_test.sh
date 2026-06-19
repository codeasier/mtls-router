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

assert_contains() {
  local file="$1" needle="$2" label="$3"
  grep -F -q -- "$needle" "$file" || {
    printf 'FAIL %s: missing %q in %s\n' "$label" "$needle" "$file" >&2
    return 1
  }
}

assert_not_contains() {
  local file="$1" needle="$2" label="$3"
  if grep -F -q -- "$needle" "$file"; then
    printf 'FAIL %s: should not contain %q in %s\n' "$label" "$needle" "$file" >&2
    return 1
  fi
}

assert_file_exists() {
  local file="$1" label="$2"
  [[ -f "$file" ]] || {
    printf 'FAIL %s: missing file %s\n' "$label" "$file" >&2
    return 1
  }
}

assert_section_contains() {
  local file="$1" section="$2" needle="$3" label="$4"
  awk -v section="$section" -v needle="$needle" '
    $0 == section { in_section = 1; next }
    /^\[/ { in_section = 0 }
    in_section && $0 == needle { found = 1 }
    END { exit(found ? 0 : 1) }
  ' "$file" || {
    printf 'FAIL %s: missing %q in section %s of %s\n' "$label" "$needle" "$section" "$file" >&2
    return 1
  }
}

test_codex_config_creates_when_missing() (
  source_setup
  local home path result
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  path="$home/.codex/config.toml"
  result="$(configure_codex "$path")"
  local first_line
  first_line="$(printf '%s\n' "$result" | sed -n '1p')"
  [[ "$first_line" == "$path" ]] || { printf 'FAIL: wrong written path\n' >&2; return 1; }
  assert_file_exists "$path" 'created config'
  assert_contains "$path" '[model_providers.mtls-router]' 'provider section'
  assert_contains "$path" '[profiles.gpt-5-5-router]' '5.5 profile'
  assert_contains "$path" '[profiles.gpt-5-4-1m-router]' '5.4 profile'
  assert_section_contains "$path" '[model_providers.mtls-router]' 'base_url = "http://127.0.0.1:19099/v1"' 'provider base_url'
  assert_section_contains "$path" '[model_providers.mtls-router]' 'env_key = "OPENAI_API_KEY"' 'provider env_key'
  assert_not_contains "$path" 'env_key = "{UserApiKey}"' 'placeholder removed'
  assert_section_contains "$path" '[model_providers.mtls-router]' 'wire_api = "responses"' 'provider wire_api'
  assert_section_contains "$path" '[profiles.gpt-5-5-router]' 'model = "gpt-5.5"' '5.5 model'
  assert_section_contains "$path" '[profiles.gpt-5-5-router]' 'model_provider = "mtls-router"' '5.5 provider'
  assert_section_contains "$path" '[profiles.gpt-5-4-1m-router]' 'model = "gpt-5.4"' '5.4 model'
  assert_section_contains "$path" '[profiles.gpt-5-4-1m-router]' 'model_provider = "mtls-router"' '5.4 provider'
)

test_codex_config_replaces_existing_blocks_and_preserves_others() (
  source_setup
  local home path result
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  mkdir -p "$home/.codex"
  path="$home/.codex/config.toml"
  cat >"$path" <<'TOML'
model = "gpt-5-codex"
model_provider = "openai"
approval_policy = "on-request"
sandbox_mode = "workspace-write"

[model_providers.mtls-router]
name = "old"
base_url = "http://old"
env_key = "OLD"
wire_api = "responses"

[profiles.gpt-5-5-router]
model = "gpt-5.5"
model_provider = "openai"
TOML
  local original_content
  original_content="$(<"$path")"
  result="$(configure_codex "$path")"
  local count_provider count_5_5 count_5_4
  count_provider="$(grep -c '^\[model_providers\.mtls-router\]$' "$path" || true)"
  [[ "$count_provider" == "1" ]] || { printf 'FAIL: provider block duplicated\n' >&2; return 1; }
  count_5_5="$(grep -c '^\[profiles\.gpt-5-5-router\]$' "$path" || true)"
  [[ "$count_5_5" == "1" ]] || { printf 'FAIL: 5.5 profile duplicated\n' >&2; return 1; }
  count_5_4="$(grep -c '^\[profiles\.gpt-5-4-1m-router\]$' "$path" || true)"
  [[ "$count_5_4" == "1" ]] || { printf 'FAIL: 5.4 profile duplicated\n' >&2; return 1; }
  assert_contains "$path" 'model = "gpt-5-codex"' 'model preserved'
  assert_contains "$path" 'approval_policy = "on-request"' 'approval preserved'
  assert_not_contains "$path" 'name = "old"' 'old provider removed'
  assert_section_contains "$path" '[model_providers.mtls-router]' 'base_url = "http://127.0.0.1:19099/v1"' 'replacement provider base_url'
  assert_section_contains "$path" '[model_providers.mtls-router]' 'env_key = "OPENAI_API_KEY"' 'replacement provider env_key'
  assert_not_contains "$path" 'env_key = "{UserApiKey}"' 'replacement placeholder removed'
  assert_section_contains "$path" '[model_providers.mtls-router]' 'wire_api = "responses"' 'replacement provider wire_api'
  assert_section_contains "$path" '[profiles.gpt-5-5-router]' 'model = "gpt-5.5"' 'replacement 5.5 model'
  assert_section_contains "$path" '[profiles.gpt-5-5-router]' 'model_provider = "mtls-router"' 'replacement 5.5 provider'
  assert_section_contains "$path" '[profiles.gpt-5-4-1m-router]' 'model = "gpt-5.4"' 'replacement 5.4 model'
  assert_section_contains "$path" '[profiles.gpt-5-4-1m-router]' 'model_provider = "mtls-router"' 'replacement 5.4 provider'
  local second_line backup_path backup_content
  second_line="$(printf '%s\n' "$result" | sed -n '2p')"
  backup_path="$(find "$(dirname "$path")" -maxdepth 1 -type f -name "$(basename "$path").bak-*" -print)"
  [[ -n "$backup_path" ]] || { printf 'FAIL: expected backup path\n' >&2; return 1; }
  [[ "$second_line" == "$backup_path" ]] || { printf 'FAIL: backup stdout mismatch\n' >&2; return 1; }
  assert_file_exists "$backup_path" 'backup file'
  assert_contains "$backup_path" 'name = "old"' 'backup old provider content'
  backup_content="$(<"$backup_path")"
  [[ "$backup_content" == "$original_content" ]] || { printf 'FAIL: backup content differs from original\n' >&2; return 1; }
)

test_codex_config_creates_when_missing
test_codex_config_replaces_existing_blocks_and_preserves_others

# --- codex auth.json is written when an api key is provided -----------
test_codex_config_writes_auth_json_when_key_given() (
  source_setup
  local home path result auth_path
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  mkdir -p "$home/.codex"
  path="$home/.codex/config.toml"
  result="$(configure_codex "$path" "sk-fake-test-key-1234")"
  auth_path="$home/.codex/auth.json"
  assert_file_exists "$auth_path" 'auth.json written'
  # third line of result is "AUTH:<path>"; fourth is its backup (empty here).
  local auth_line
  auth_line="$(printf '%s\n' "$result" | sed -n '3p')"
  [[ "$auth_line" == "AUTH:$auth_path" ]] || { printf 'FAIL: missing AUTH line, got %q\n' "$auth_line" >&2; return 1; }
  # auth.json must contain the actual key, not a placeholder.
  local body
  body="$(<"$auth_path")"
  [[ "$body" == *"sk-fake-test-key-1234"* ]] || { printf 'FAIL: auth.json missing key, got %q\n' "$body" >&2; return 1; }
  [[ "$body" != *"{UserApiKey}"* ]] || { printf 'FAIL: auth.json still has placeholder\n' >&2; return 1; }
  # auth.json must be valid JSON containing OPENAI_API_KEY.
  jq -e '.OPENAI_API_KEY == "sk-fake-test-key-1234"' "$auth_path" >/dev/null
)

# --- codex auth.json merges with existing keys ------------------------
test_codex_config_merges_auth_json_existing_keys() (
  source_setup
  local home path result auth_path
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  mkdir -p "$home/.codex"
  path="$home/.codex/config.toml"
  auth_path="$home/.codex/auth.json"
  echo '{"OPENAI_API_KEY": "old-key", "extra": "keep-me"}' >"$auth_path"
  configure_codex "$path" "new-key" >/dev/null
  jq -e '.OPENAI_API_KEY == "new-key" and .extra == "keep-me"' "$auth_path" >/dev/null
)

# --- codex auth.json backup is reported in result ---------------------
test_codex_config_auth_json_backup_reported() (
  source_setup
  local home path auth_path
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  mkdir -p "$home/.codex"
  path="$home/.codex/config.toml"
  auth_path="$home/.codex/auth.json"
  echo '{"OPENAI_API_KEY": "old-key"}' >"$auth_path"
  local result
  result="$(configure_codex "$path" "new-key")"
  local auth_backup_line
  auth_backup_line="$(printf '%s\n' "$result" | sed -n '4p')"
  [[ -n "$auth_backup_line" ]] || { printf 'FAIL: expected 4th line with auth backup, got nothing\n' >&2; return 1; }
  [[ "$auth_backup_line" == "$auth_path"*.bak-* ]] || { printf 'FAIL: backup path %q does not look like auth.json.bak-* \n' "$auth_backup_line" >&2; return 1; }
  [[ -f "$auth_backup_line" ]] || { printf 'FAIL: backup file does not exist\n' >&2; return 1; }
  grep -F -q 'old-key' "$auth_backup_line" || { printf 'FAIL: backup missing old key\n' >&2; return 1; }
)

# --- codex is detected from ~/.codex directory even without CLI ------
test_detect_agents_codex_via_dotdir() (
  source_setup
  local home
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  mkdir -p "$home/.codex"
  HOME="$home" detect_agents
  local found=0
  for n in "${DETECTED_NAMES[@]}"; do
    [[ "$n" == "Codex" ]] && found=1
  done
  [[ "$found" -eq 1 ]] || { printf 'FAIL: Codex not detected via %s\n' "$home/.codex" >&2; return 1; }
  # Path should be $HOME/.codex/config.toml, not just "$HOME/codex".
  local got=""
  for i in "${!DETECTED_NAMES[@]}"; do
    if [[ "${DETECTED_NAMES[$i]}" == "Codex" ]]; then
      got="${DETECTED_CONFIG_PATHS[$i]}"
    fi
  done
  [[ "$got" == "$home/.codex/config.toml" ]] || { printf 'FAIL: codex path = %q, want %q\n' "$got" "$home/.codex/config.toml" >&2; return 1; }
)

# --- print-config for codex includes auth.json snippet with placeholder
test_print_config_codex_emits_auth_json_placeholder() (
  source_setup
  local home
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  mkdir -p "$home/.codex"
  HOME="$home" CODEX_HOME="$home/.codex" detect_agents
  local out
  out="$(HOME="$home" CODEX_HOME="$home/.codex" main --print-config --agent=codex 2>&1)"
  [[ "$out" == *"$home/.codex/auth.json"* ]] || { printf 'FAIL: print-config does not mention auth.json\n' >&2; return 1; }
  [[ "$out" == *'OPENAI_API_KEY'* ]] || { printf 'FAIL: print-config missing OPENAI_API_KEY key\n' >&2; return 1; }
  [[ "$out" == *'{UserApiKey}'* ]] || { printf 'FAIL: print-config should keep {UserApiKey} placeholder\n' >&2; return 1; }
  [[ "$out" != *'sk-'* ]] || { printf 'FAIL: print-config must not embed any real key shape\n' >&2; return 1; }
)

test_codex_config_writes_auth_json_when_key_given
test_codex_config_merges_auth_json_existing_keys
test_codex_config_auth_json_backup_reported
test_detect_agents_codex_via_dotdir
test_print_config_codex_emits_auth_json_placeholder
