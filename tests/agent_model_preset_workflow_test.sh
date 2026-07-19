#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCAL_BUILD="$ROOT/scripts/build.sh"
SIDECAR_BUILD="$ROOT/desktop/scripts/build-sidecars.sh"
RELEASE="$ROOT/.github/workflows/release.yml"
PREFLIGHT="$ROOT/scripts/preflight-agent-model-preset.sh"
SYMBOL='github.com/codeasier/mtls-router/internal/manager/preset.Encoded'
SOURCE='AGENT_MODEL_PRESET_BASE64'

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

for script in "$LOCAL_BUILD" "$SIDECAR_BUILD"; do
  grep -Fq "${SOURCE}:-" "$script" || fail "$(basename "$script") does not read optional $SOURCE"
  [[ "$(grep -Fc "$SYMBOL" "$script")" -eq 1 ]] || fail "$(basename "$script") must inject the preset symbol exactly once"
done

local_router_block="$(awk '/^go build -trimpath/{build++} build == 1{print} build == 2{exit}' "$LOCAL_BUILD")"
local_manager_block="$(awk '/^go build -trimpath/{build++} build == 2{print}' "$LOCAL_BUILD")"
[[ "$local_router_block" != *"$SYMBOL"* ]] || fail 'local router build receives the Agent preset'
[[ "$local_manager_block" == *"$SYMBOL"* ]] || fail 'local manager build does not receive the Agent preset'

sidecar_router_line="$(grep 'go build.*-o "\$router"' "$SIDECAR_BUILD")"
sidecar_manager_line="$(grep 'go build.*-o "\$manager"' "$SIDECAR_BUILD")"
[[ "$sidecar_router_line" != *manager_metadata* && "$sidecar_router_line" != *"$SYMBOL"* ]] || fail 'desktop router sidecar receives the Agent preset'
[[ "$sidecar_manager_line" == *manager_metadata* ]] || fail 'desktop manager sidecar does not receive manager-only metadata'

[[ "$(grep -Fc "$SYMBOL" "$RELEASE")" -eq 1 ]] || fail 'release workflow must inject the preset directly only into the standalone manager'
release_router_block="$(awk '/go build -trimpath/{build++} build == 1{print} build == 2{exit}' "$RELEASE")"
release_manager_block="$(awk '/go build -trimpath/{build++} build == 2{print} build == 3{exit}' "$RELEASE")"
[[ "$release_router_block" != *"$SYMBOL"* ]] || fail 'release router receives the Agent preset'
[[ "$release_manager_block" == *"$SYMBOL"* ]] || fail 'standalone release manager does not receive the Agent preset'
[[ "$(grep -Fc "AGENT_MODEL_PRESET_BASE64: \${{ vars.AGENT_MODEL_PRESET_BASE64 }}" "$RELEASE")" -eq 3 ]] || fail 'preflight and both release manager producers must source the same repository variable'
grep -Fq 'run: ./scripts/preflight-agent-model-preset.sh' "$RELEASE" || fail 'release preset preflight is not configured'
grep -Fq "$SYMBOL" "$PREFLIGHT" || fail 'preflight does not validate through the exact manager linker symbol'

AGENT_MODEL_PRESET_BASE64='' "$PREFLIGHT" || fail 'empty preset preflight failed'
decoded_canary='malformed-decoded-preset-canary'
encoded_canary="$(printf '%s' "$decoded_canary" | base64 | tr -d '\r\n')"
preflight_error="$(mktemp)"
trap 'rm -f "$preflight_error"' EXIT
if AGENT_MODEL_PRESET_BASE64="$encoded_canary" "$PREFLIGHT" >/dev/null 2>"$preflight_error"; then
  fail 'invalid configured preset passed preflight'
fi
grep -Fq 'invalid embedded Agent model preset' "$preflight_error" || fail 'invalid preset failure is not sanitized'
if grep -Fq "$decoded_canary" "$preflight_error" || grep -Fq "$encoded_canary" "$preflight_error"; then
  fail 'preset preflight leaked configured input'
fi

printf 'PASS: Agent model preset build and release integration\n'
