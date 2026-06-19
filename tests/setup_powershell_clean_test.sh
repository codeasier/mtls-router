#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/setup.ps1"

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

if grep -q 'npm install -g .*claude-code' "$SCRIPT"; then
  fail 'setup.ps1 still contains Claude Code npm install'
fi

if grep -q '^function Ensure-ClaudeCode' "$SCRIPT"; then
  fail 'setup.ps1 still defines Ensure-ClaudeCode'
fi

if grep -q '^\s*claude\s*$' "$SCRIPT"; then
  fail 'setup.ps1 still launches claude'
fi

if ! grep -F -q 'mtls-router 代理配置向导' "$SCRIPT"; then
  fail 'setup.ps1 missing agent wizard banner'
fi

if grep -F -q 'Claude Code 一键配置工具' "$SCRIPT"; then
  fail 'setup.ps1 still contains old Claude Code banner'
fi

printf 'ok: setup.ps1 no longer installs or launches Claude Code\n'
