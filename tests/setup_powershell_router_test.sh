#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/setup.ps1"
fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

grep -q -- '--download' "$SCRIPT" || fail "PowerShell setup should parse --download"
grep -q "Scheme -ne 'https'" "$SCRIPT" || fail "PowerShell setup should reject non-HTTPS downloads"
grep -q 'Get-FileHash.*SHA256' "$SCRIPT" || fail "PowerShell setup should verify SHA-256"
grep -q '\$PSScriptRoot' "$SCRIPT" || fail "PowerShell setup should discover sibling payloads"
grep -q 'router.start' "$SCRIPT" || fail "PowerShell start should call manager"
grep -q 'router.status' "$SCRIPT" || fail "PowerShell status should call manager"
grep -q 'router.logs' "$SCRIPT" || fail "PowerShell log should call manager"
grep -q 'router.stop' "$SCRIPT" || fail "PowerShell stop should call manager"
grep -q 'after-router' "$SCRIPT" || fail "PowerShell installer should expose first crash point"
grep -q 'after-manager' "$SCRIPT" || fail "PowerShell installer should expose second crash point"
grep -q 'before-receipt' "$SCRIPT" || fail "PowerShell installer should expose receipt crash point"
grep -q 'SecurityProtocol.*-bor.*Tls12' "$SCRIPT" || fail "PowerShell setup should preserve protocols and enable TLS 1.2"

if ! command -v pwsh >/dev/null 2>&1; then
  printf 'skip: pwsh not available; static PowerShell wrapper checks passed\n'
  exit 0
fi

pwsh_home="$(mktemp -d)"
trap 'rm -rf "$pwsh_home"' EXIT
if ! help_out="$(env -u USERPROFILE HOME="$pwsh_home" pwsh -NoProfile -File "$SCRIPT" --help 2>&1)"; then
  fail "PowerShell help failed without USERPROFILE: $help_out"
fi
[[ "$help_out" == *'router install'* && "$help_out" == *'agent write-config'* ]] || fail "PowerShell help omitted public commands"
tls_out="$(pwsh -NoProfile -Command "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls; & '$SCRIPT' --help > \$null; [Net.ServicePointManager]::SecurityProtocol.ToString()")"
[[ "$tls_out" == *Tls* && "$tls_out" == *Tls12* ]] || fail "PowerShell TLS policy was not preserved"

printf 'PASS: PowerShell manager lifecycle wrapper\n'
