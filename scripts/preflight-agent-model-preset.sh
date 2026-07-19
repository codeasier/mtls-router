#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
encoded="${AGENT_MODEL_PRESET_BASE64:-}"
[[ -n "$encoded" ]] || exit 0

if [[ ! "$encoded" =~ ^[A-Za-z0-9+/]*={0,2}$ || $((${#encoded} % 4)) -ne 0 ]]; then
  printf 'invalid embedded Agent model preset\n' >&2
  exit 1
fi

state_dir="$(mktemp -d "${TMPDIR:-/tmp}/mtls-router-preset-preflight.XXXXXX")"
trap 'rm -rf "$state_dir"' EXIT

(
  cd "$repo_dir"
  MTLS_ROUTER_DESKTOP_DATA_DIR="$state_dir/desktop" \
    MTLS_ROUTER_STATE_DIR="$state_dir/cli" \
    MTLS_ROUTER_LOG_PATH="$state_dir/cli/router.log" \
    CLAUDE_CONFIG_DIR="$state_dir/claude" \
    OPENCODE_CONFIG="$state_dir/opencode.json" \
    CODEX_HOME="$state_dir/codex" \
    go run -trimpath \
      -ldflags "-X 'github.com/codeasier/mtls-router/internal/manager/preset.Encoded=$encoded'" \
      ./cmd/mtls-router-manager serve </dev/null >/dev/null
)
