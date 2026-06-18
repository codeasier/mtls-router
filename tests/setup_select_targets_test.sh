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
    printf 'FAIL %s: got %q want %q
' "$label" "$got" "$want" >&2
    return 1
  }
}

test_select_targets_empty_returns_all() {
  source_setup
  local got
  got="$(select_targets "" "claude" "opencode" "codex")"
  assert_eq "$got" "1 2 3" "empty -> all"
}

test_select_targets_zero_returns_all() {
  source_setup
  local got
  got="$(select_targets "0" "claude" "opencode" "codex")"
  assert_eq "$got" "1 2 3" "0 -> all"
}

test_select_targets_zero_with_extras_returns_all() {
  source_setup
  local got
  got="$(select_targets "0 2" "claude" "opencode" "codex")"
  assert_eq "$got" "1 2 3" "0 2 -> all"
}

test_select_targets_specific_numbers() {
  source_setup
  local got
  got="$(select_targets "1 3" "claude" "opencode" "codex")"
  assert_eq "$got" "1 3" "1 3"
}

test_select_targets_invalid_returns_error() {
  source_setup
  if select_targets "9" "claude" "opencode" "codex" >/dev/null 2>&1; then
    printf 'FAIL: expected invalid selection to fail
' >&2
    return 1
  fi
}

test_select_targets_dedupes() {
  source_setup
  local got
  got="$(select_targets "1 1 2" "claude" "opencode" "codex")"
  assert_eq "$got" "1 2" "dedupe"
}

test_select_targets_empty_when_no_agents() {
  source_setup
  local got
  got="$(select_targets "0")"
  assert_eq "$got" "" "no agents"
}

test_select_targets_rejects_glob_token() {
  source_setup
  if select_targets "*" "claude" >/dev/null 2>&1; then
    printf 'FAIL: expected glob token to fail
' >&2
    return 1
  fi
}

test_select_targets_rejects_leading_zero() {
  source_setup
  if select_targets "08" "claude" >/dev/null 2>&1; then
    printf 'FAIL: expected leading-zero selection to fail
' >&2
    return 1
  fi
}

test_select_targets_empty_returns_all
test_select_targets_zero_returns_all
test_select_targets_zero_with_extras_returns_all
test_select_targets_specific_numbers
test_select_targets_invalid_returns_error
test_select_targets_dedupes
test_select_targets_empty_when_no_agents
test_select_targets_rejects_glob_token
test_select_targets_rejects_leading_zero
