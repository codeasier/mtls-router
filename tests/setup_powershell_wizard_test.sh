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
contains 'agent.write'
contains 'api_key = $apiKey'
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

printf 'PASS: setup.ps1 is a manager wrapper\n'
