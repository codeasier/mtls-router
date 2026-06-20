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

# the old interactive "[y/N]" prompt is gone
assert_not_contains '是否备份并写入配置？[y/N]'
assert_not_contains '请输入编号，多个用空格分隔；直接回车则逐个询问'
assert_not_contains '0) 全部覆盖配置'

# shared building blocks
assert_contains 'function Detect-Agents'
assert_contains 'function Backup-File'
assert_contains 'function Configure-Claude'
assert_contains 'function Configure-Opencode'
assert_contains 'function Remove-CodexBlock'
assert_contains 'function Configure-Codex'
assert_contains 'function Print-NextSteps'
assert_contains 'function ConvertFrom-JsoncText'
assert_contains 'function Convert-OpencodeJsoncToJson'
assert_contains '是否尝试备份该 JSONC，并迁移为标准 JSON opencode.json 后写入 mtls-router provider？[y/N]'
assert_contains 'JSONC 注释和原格式不会保留'
assert_contains '已跳过 opencode 写入。可手动创建标准 JSON，或设置 OPENCODE_CONFIG 指向 JSON 文件。'

# new non-interactive flag API
assert_contains 'function Show-Usage'
assert_contains 'function ConvertTo-AgentKey'
assert_contains 'function ConvertFrom-AgentKey'
assert_contains '--print-config'
assert_contains '--write-config'
assert_contains '--agent='
assert_contains 'Main @args'
assert_contains 'function Download-MtlsRouter'
assert_contains 'function Start-MtlsRouter'
assert_contains '& $BinaryPath -backend'

# Codex root-key cleanup regexes must use PowerShell/.NET regex escaping.
assert_not_contains "^\\\\s*\\\\["
assert_not_contains "^\\\\s*(model_provider|model|disable_response_storage)\\\\s*="
assert_contains "^\\s*\\["
assert_contains "^\\s*(model_provider|model|disable_response_storage)\\s*="

# helpful messages
assert_contains '未对 agent 配置做任何改动'

printf 'ok: setup.ps1 exposes non-interactive flag API\n'
