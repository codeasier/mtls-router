#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/setup.sh"

fail() { printf 'FAIL: %s
' "$1" >&2; exit 1; }

if grep -q 'npm install -g @anthropic-ai/claude-code' "$SCRIPT"; then
  fail 'setup.sh still contains Claude Code npm install'
fi

if grep -q 'exec claude' "$SCRIPT"; then
  fail 'setup.sh still contains exec claude'
fi

if grep -q '^ensure_claude' "$SCRIPT"; then
  fail 'setup.sh still defines ensure_claude'
fi

if ! grep -F -q 'mtls-router 代理配置向导' "$SCRIPT"; then
  fail 'setup.sh missing agent wizard banner'
fi

if grep -F -q 'Claude Code 一键配置工具' "$SCRIPT"; then
  fail 'setup.sh still contains old Claude Code banner'
fi

printf 'ok: setup.sh no longer installs or launches Claude Code
'
