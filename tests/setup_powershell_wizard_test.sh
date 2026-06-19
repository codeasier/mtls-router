#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/setup.ps1"

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

assert_contains() {
  local needle="$1"
  grep -F -q -- "$needle" "$SCRIPT" || fail "setup.ps1 missing: $needle"
}

assert_not_contains() {
  local needle="$1"
  if grep -F -q -- "$needle" "$SCRIPT"; then
    fail "setup.ps1 should not contain: $needle"
  fi
}

# still no auto-install / auto-launch
assert_not_contains "function Ensure-ClaudeCode"
assert_not_contains "@anthropic-ai/claude-code"
if grep -q '^\s*claude\s*$' "$SCRIPT"; then
  fail 'setup.ps1 still launches claude'
fi

# wizard entry points expected after full alignment with setup.sh
assert_contains 'function Detect-Agents'
assert_contains 'function Select-Targets'
assert_contains 'function Backup-File'
assert_contains 'function Configure-Claude'
assert_contains 'function Configure-Opencode'
assert_contains 'function Remove-CodexBlock'
assert_contains 'function Configure-Codex'
assert_contains '0) 全部覆盖配置'
assert_contains '请输入编号，多个用空格分隔；直接回车则逐个询问：'
assert_contains '未检测到 Claude Code、opencode 或 Codex。mtls-router 已启动，但未写入 agent 配置。'
assert_contains '未选择任何 agent，跳过 agent 配置。'
assert_contains '未知 agent：'
assert_contains '已写入配置：'
assert_contains '已备份：'
assert_contains '可手动启动 agent。'

printf 'ok: setup.ps1 exposes full agent wizard flow\n'
