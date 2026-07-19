#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/setup.ps1"
fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
contains() { grep -Fq -- "$1" "$SCRIPT" || fail "setup.ps1 missing: $1"; }
not_contains() { ! grep -Fq -- "$1" "$SCRIPT" || fail "setup.ps1 should not contain: $1"; }

contains 'function Invoke-ManagerAt'
contains 'function Resolve-ManagerPair'
contains 'function Install-Pair'
contains 'function Repair-PendingInstall'
contains 'install-receipt.json'
contains 'install-pending.json'
contains 'mtls-router-manager.exe'
contains 'agent.preview'
contains 'agent.models'
contains 'agent.render'
contains 'agent.write'
contains 'api_key=$script:TransientApiKey'
contains 'catalog_token=$models.result.catalog_token'
contains 'function Get-AgentInitialConfig'
contains 'CONFIG SOURCE'
contains 'PRESET UNAVAILABLE'
contains 'function Read-StringDefault'
contains 'function Read-ClaudeContext'
contains "@('standard', '1m')"
contains "if (\$context -ceq '1m')"
contains "foreach (\$role in @('haiku','sonnet','opus'))"
contains 'Read-OptionalString "Claude $role name"'
contains 'Read-ClaudeContext "Claude $role"'
contains '$section[$role] = $entry'
contains 'New-AgentModelConfig $targets $initialConfig'
contains 'approve_managed_overwrite=$approveDrift'
contains 'approve_codex_auth_change=$approveCodex'
contains '--model-config'
contains 'MTLS_ROUTER_OPENAI_API_KEY 已移除'
contains 'Main @args'
contains '--print-config'
contains '--write-config'
contains '--agent='
contains '提示：未对 agent 配置做任何改动。'
not_contains 'function Configure-Claude'
not_contains 'function Configure-Opencode'
not_contains 'function Configure-Codex'
not_contains 'function Get-RouterIdentityStatus'
not_contains '{UserApiKey}'
not_contains 'gpt-5.5'
not_contains 'Invoke-Expression'
not_contains 'iex '

[[ "$(xxd -p -l 3 "$SCRIPT")" == efbbbf ]] || fail 'setup.ps1 UTF-8 BOM missing'

printf 'PASS: setup.ps1 is a manager wrapper\n'
