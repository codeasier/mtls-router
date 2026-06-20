#!/usr/bin/env bash
# Behavioral tests for the new non-interactive flag API in setup.sh.
#
# Run setup.sh with a stub for download_router / start_router and verify:
#  - default: no agent config changes
#  - --print-config: prints what would be written, never touches files
#  - --write-config without --agent=: fails
#  - --write-config --agent=claude: writes only that agent's config
#  - --write-config --agent=claude,nonexistent: fails with a clear message
#  - --write-config --agent=claude,claude: fails on duplicates
#  - --foo: unknown flag fails

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/setup.sh"

# Build a tiny shim that sources setup.sh with a no-op download/start and
# then runs main with whatever args we pass. We use this to exercise the
# real flag-parsing and target-resolution code paths without hitting the
# network or actually launching mtls-router.
build_shim() {
  # Make setup.sh source-able without invoking main, then call main with
  # the args we want. This mirrors the trick used by other tests in this
  # directory (see setup_select_targets_test.sh).
  local stub_script
  stub_script="$(mktemp)"
  chmod +x "$stub_script"
  cat >"$stub_script" <<'SHIM'
#!/usr/bin/env bash
set -euo pipefail
SCRIPT="__SETUP_SH_PATH__"
shift 0 2>/dev/null || true
# Strip the trailing `main "$@"` so we can call main ourselves after
# defining stubs. The header pattern is the same as in
# setup_select_targets_test.sh.
SOURCED="$(mktemp)"
sed '/^main "\$@"$/d' "$SCRIPT" >"$SOURCED"
# shellcheck source=/dev/null
source "$SOURCED"
rm -f "$SOURCED"
# Now override network-touching functions so the test is hermetic.
download_router() { echo "DOWNLOAD_CALLED"; }
start_router() {
  if [[ "${MTLS_ROUTER_SKIP_START:-}" == "1" ]]; then
    echo "[启动] 跳过（MTLS_ROUTER_SKIP_START=1）"
    return 0
  fi
  echo "START_CALLED"
}
if [[ "${STUB_DETECTED_AGENTS:-}" == "1" ]]; then
  detect_agents() {
    local base="${STUB_CONFIG_DIR:-/tmp}"
    DETECTED_NAMES=("Claude Code" "Codex")
    DETECTED_COMMANDS=("/tmp/claude" "<desktop>")
    DETECTED_CONFIG_PATHS=("$base/settings.json" "$base/.codex/config.toml")
    DETECTED_AUTH_PATHS=("" "$base/.codex/auth.json")
  }
fi
main "$@"
SHIM
  python3 - <<PY
from pathlib import Path
p = Path('$stub_script')
p.write_text(p.read_text().replace('__SETUP_SH_PATH__', '$SCRIPT'))
PY
  printf '%s' "$stub_script"
}

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

assert_contains() {
  local needle="$1" haystack="$2" label="$3"
  [[ "$haystack" == *"$needle"* ]] || fail "$label: missing '$needle'"
}

assert_not_contains() {
  local needle="$1" haystack="$2" label="$3"
  [[ "$haystack" != *"$needle"* ]] || fail "$label: should not contain '$needle'"
}

# --- 1. default mode does not invoke main with any agent key ---------
test_default_invokes_no_agents() {
  local shim out
  shim="$(build_shim)"
  out="$(bash "$shim" 2>&1)"
  rm -f "$shim"
  assert_contains "未对 agent 配置做任何改动" "$out" "default mode"
  # None of the agent names should appear in the "已写入" section.
  assert_not_contains "已写入 Claude Code 配置" "$out" "default should not write claude"
  assert_not_contains "已写入 opencode 配置" "$out" "default should not write opencode"
}

# --- 2. --help exits 0 and shows usage -------------------------------
test_help() {
  local shim
  shim="$(build_shim)"
  local out
  out="$(bash "$shim" --help 2>&1)"
  rm -f "$shim"
  assert_contains "用法: setup.sh" "$out" "--help shows usage"
  assert_contains "--print-config" "$out" "--help mentions --print-config"
  assert_contains "--write-config" "$out" "--help mentions --write-config"
  assert_contains "--agent=" "$out" "--help mentions --agent="
}

# --- 3. --print-config is read-only: no download/start ----------------
test_print_config_does_not_download_or_start() {
  local shim out
  shim="$(build_shim)"
  out="$(STUB_DETECTED_AGENTS=1 bash "$shim" --print-config 2>&1)"
  rm -f "$shim"
  assert_contains "### Claude Code -> /tmp/settings.json" "$out" "--print-config should print config snippets"
  assert_not_contains "DOWNLOAD_CALLED" "$out" "--print-config should not download"
  assert_not_contains "START_CALLED" "$out" "--print-config should not start"
}

# --- 4. --write-config is config-only: no download/start -------------
test_write_config_does_not_download_or_start() {
  local shim tmp out
  shim="$(build_shim)"
  tmp="$(mktemp -d)"
  out="$(STUB_DETECTED_AGENTS=1 STUB_CONFIG_DIR="$tmp" MTLS_ROUTER_OPENAI_API_KEY=sk-test bash "$shim" --write-config --agent=codex 2>&1)"
  rm -f "$shim"
  rm -rf "$tmp"
  assert_contains "已写入 Codex 配置" "$out" "--write-config should write config"
  assert_contains "未启动 mtls-router" "$out" "--write-config summary should not claim router is running"
  assert_not_contains "mtls-router 已在后台运行" "$out" "--write-config should not claim router started"
  assert_not_contains "DOWNLOAD_CALLED" "$out" "--write-config should not download"
  assert_not_contains "START_CALLED" "$out" "--write-config should not start"
}

# --- 5. --write-config without --agent= fails ------------------------
test_write_config_without_agent_fails() {
  local shim
  shim="$(build_shim)"
  local rc=0
  bash "$shim" --write-config >/dev/null 2>&1 || rc=$?
  rm -f "$shim"
  [[ "$rc" -ne 0 ]] || fail "--write-config without --agent= should fail (got rc=0)"
}

# --- 4. --write-config with unknown agent fails ----------------------
test_write_config_with_unknown_agent_fails() {
  local shim
  shim="$(build_shim)"
  local rc=0 out
  out="$(bash "$shim" --write-config --agent=claude,nonexistent 2>&1)" || rc=$?
  rm -f "$shim"
  [[ "$rc" -ne 0 ]] || fail "unknown agent should fail (got rc=0)"
  assert_contains "未检测到 agent" "$out" "unknown agent error message"
}

# --- 5. --write-config with duplicate agent fails -------------------
test_write_config_with_duplicate_agent_fails() {
  local shim
  shim="$(build_shim)"
  local rc=0 out
  out="$(bash "$shim" --write-config --agent=claude,claude 2>&1)" || rc=$?
  rm -f "$shim"
  [[ "$rc" -ne 0 ]] || fail "duplicate agent should fail (got rc=0)"
  assert_contains "重复项" "$out" "duplicate agent error message"
}

# --- 6. unknown flag fails -------------------------------------------
test_unknown_flag_fails() {
  local shim
  shim="$(build_shim)"
  local rc=0
  bash "$shim" --foo >/dev/null 2>&1 || rc=$?
  rm -f "$shim"
  [[ "$rc" -ne 0 ]] || fail "unknown flag should fail (got rc=0)"
}

# --- 8. namespaced router setup matches default behavior ---------------
test_router_setup_downloads_and_starts() {
  local shim out
  shim="$(build_shim)"
  out="$(bash "$shim" router setup 2>&1)"
  rm -f "$shim"
  assert_contains "DOWNLOAD_CALLED" "$out" "router setup should download"
  assert_contains "START_CALLED" "$out" "router setup should start"
  assert_contains "未对 agent 配置做任何改动" "$out" "router setup should not configure agents"
}

# --- 9. router install downloads only ----------------------------------
test_router_install_downloads_only() {
  local shim out
  shim="$(build_shim)"
  out="$(bash "$shim" router install 2>&1)"
  rm -f "$shim"
  assert_contains "DOWNLOAD_CALLED" "$out" "router install should download"
  assert_not_contains "START_CALLED" "$out" "router install should not start"
  assert_contains "未启动 mtls-router" "$out" "router install should say router is not running"
}

# --- 10. router start starts only --------------------------------------
test_router_start_starts_only() {
  local shim out
  shim="$(build_shim)"
  out="$(bash "$shim" router start 2>&1)"
  rm -f "$shim"
  assert_not_contains "DOWNLOAD_CALLED" "$out" "router start should not download"
  assert_contains "START_CALLED" "$out" "router start should invoke start path"
}

# --- 11. router start respects skip-start -------------------------------
test_router_start_skip_does_not_claim_started() {
  local shim out
  shim="$(build_shim)"
  out="$(MTLS_ROUTER_SKIP_START=1 bash "$shim" router start 2>&1)"
  rm -f "$shim"
  assert_contains "[启动] 跳过" "$out" "router start should honor skip-start"
  assert_contains "未启动 mtls-router" "$out" "router start skip should say router is not running"
  assert_not_contains "mtls-router 已在后台运行" "$out" "router start skip should not claim started"
}

# --- 12. agent print-config is read-only --------------------------------
test_agent_print_config_does_not_download_or_start() {
  local shim out
  shim="$(build_shim)"
  out="$(STUB_DETECTED_AGENTS=1 bash "$shim" agent print-config 2>&1)"
  rm -f "$shim"
  assert_contains "### Claude Code -> /tmp/settings.json" "$out" "agent print-config should print config snippets"
  assert_not_contains "DOWNLOAD_CALLED" "$out" "agent print-config should not download"
  assert_not_contains "START_CALLED" "$out" "agent print-config should not start"
}

# --- 12. agent write-config is config-only ------------------------------
test_agent_write_config_does_not_download_or_start() {
  local shim tmp out
  shim="$(build_shim)"
  tmp="$(mktemp -d)"
  out="$(STUB_DETECTED_AGENTS=1 STUB_CONFIG_DIR="$tmp" MTLS_ROUTER_OPENAI_API_KEY=sk-test bash "$shim" agent write-config --agent=codex 2>&1)"
  rm -f "$shim"
  rm -rf "$tmp"
  assert_contains "已写入 Codex 配置" "$out" "agent write-config should write config"
  assert_not_contains "DOWNLOAD_CALLED" "$out" "agent write-config should not download"
  assert_not_contains "START_CALLED" "$out" "agent write-config should not start"
}

# --- 13. invalid namespace boundaries fail ------------------------------
test_invalid_namespaced_commands_fail() {
  local shim cmd rc
  for cmd in "print-config" "agent" "router" "router print-config" "agent start"; do
    shim="$(build_shim)"
    rc=0
    # shellcheck disable=SC2086
    bash "$shim" $cmd >/dev/null 2>&1 || rc=$?
    rm -f "$shim"
    [[ "$rc" -ne 0 ]] || fail "$cmd should fail (got rc=0)"
  done
}

test_default_invokes_no_agents
test_help
test_print_config_does_not_download_or_start
test_write_config_does_not_download_or_start
test_write_config_without_agent_fails
test_write_config_with_unknown_agent_fails
test_write_config_with_duplicate_agent_fails
test_unknown_flag_fails
test_router_setup_downloads_and_starts
test_router_install_downloads_only
test_router_start_starts_only
test_router_start_skip_does_not_claim_started
test_agent_print_config_does_not_download_or_start
test_agent_write_config_does_not_download_or_start
test_invalid_namespaced_commands_fail
