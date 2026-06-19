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

test_claude_config_creates_file_when_missing() {
  (
    source_setup
    local home tmp_path
    home="$(mktemp -d)"
    trap 'rm -rf "$home"' EXIT
    tmp_path="$home/.claude/settings.json"
    local result
    result="$(configure_claude "$tmp_path")"
    local first_line
    first_line="$(printf '%s\n' "$result" | sed -n '1p')"
    assert_eq "$first_line" "$tmp_path" "written path"
    [[ -f "$tmp_path" ]] || { printf 'FAIL: file not created\n' >&2; return 1; }
    local env_block
    env_block="$(jq -c '.env' "$tmp_path")"
    assert_eq "$env_block" "$(claude_env_block | jq -c '.')" "env contents"
  )
}

test_claude_config_preserves_non_env_keys_and_replaces_env() {
  (
    source_setup
    local home tmp_path
    home="$(mktemp -d)"
    trap 'rm -rf "$home"' EXIT
    mkdir -p "$home/.claude"
    tmp_path="$home/.claude/settings.json"
    cat >"$tmp_path" <<'JSON'
{
  "SENTINEL": "keep-me",
  "env": {
    "ANTHROPIC_BASE_URL": "http://old",
    "OTHER": "should-go"
  },
  "permissions": {"edit": "allow"}
}
JSON
    local result
    result="$(configure_claude "$tmp_path")"
    local got_url
    got_url="$(jq -r '.env.ANTHROPIC_BASE_URL' "$tmp_path")"
    assert_eq "$got_url" "http://127.0.0.1:19099" "url replaced"
    local other_present
    other_present="$(jq -r '.env.OTHER // "missing"' "$tmp_path")"
    assert_eq "$other_present" "missing" "old env removed"
    local perm
    perm="$(jq -r '.permissions.edit' "$tmp_path")"
    assert_eq "$perm" "allow" "other keys preserved"
    local -a baks
    baks=( "$home/.claude/settings.json.bak-"* )
    assert_eq "${#baks[@]}" "1" "exactly one backup"
    [[ "${baks[0]}" == *.bak-* ]] || { printf 'FAIL: backup path has unexpected suffix: %q\n' "${baks[0]}" >&2; return 1; }
    local backup_sentinel
    backup_sentinel="$(jq -r '.SENTINEL' "${baks[0]}")"
    assert_eq "$backup_sentinel" "keep-me" "backup preserves sentinel"
    local second_line
    second_line="$(printf '%s\n' "$result" | sed -n '2p')"
    assert_eq "$second_line" "${baks[0]}" "backup path on stdout"
  )
}

test_claude_config_rejects_invalid_json() {
  (
    source_setup
    local home tmp_path
    home="$(mktemp -d)"
    trap 'rm -rf "$home"' EXIT
    mkdir -p "$home/.claude"
    tmp_path="$home/.claude/settings.json"
    echo 'not json' >"$tmp_path"
    if ( configure_claude "$tmp_path" >/dev/null 2>&1 ); then
      printf 'FAIL: expected invalid JSON to fail\n' >&2
      return 1
    fi
  )
}

test_claude_config_uses_real_api_key_when_provided() {
  (
    source_setup
    local home tmp_path
    home="$(mktemp -d)"
    trap 'rm -rf "$home"' EXIT
    tmp_path="$home/.claude/settings.json"
    configure_claude "$tmp_path" "real-key-123" >/dev/null
    local token
    token="$(jq -r '.env.ANTHROPIC_AUTH_TOKEN' "$tmp_path")"
    assert_eq "$token" "real-key-123" "real auth token written"
    local placeholder
    placeholder="$(grep -c '{UserApiKey}' "$tmp_path" || true)"
    assert_eq "$placeholder" "0" "placeholder removed"
  )
}

test_claude_config_creates_file_when_missing
test_claude_config_preserves_non_env_keys_and_replaces_env
test_claude_config_rejects_invalid_json
test_claude_config_uses_real_api_key_when_provided
