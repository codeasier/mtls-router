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
  assert_section_contains "$path" '[model_providers.mtls-router]' 'env_key = "MTLS_ROUTER_API_KEY"' 'provider env_key'
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
  assert_section_contains "$path" '[model_providers.mtls-router]' 'env_key = "MTLS_ROUTER_API_KEY"' 'replacement provider env_key'
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
