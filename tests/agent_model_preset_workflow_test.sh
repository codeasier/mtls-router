#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCAL_BUILD="$ROOT/scripts/build.sh"
DESKTOP_BUILD="$ROOT/desktop/src-tauri/build.rs"
DESKTOP_RUNTIME="$ROOT/desktop/src-tauri/src/runtime.rs"
DESKTOP_SESSION="$ROOT/desktop/src-tauri/src/manager_core/session.rs"
RELEASE="$ROOT/.github/workflows/release.yml"
PREFLIGHT="$ROOT/scripts/preflight-agent-model-preset.sh"
NORMALIZER="$ROOT/scripts/normalize-simplify.sh"
SYMBOL='github.com/codeasier/mtls-router/internal/manager/preset.Encoded'
SIMPLIFY_SYMBOL='github.com/codeasier/mtls-router/internal/manager/modelcatalog.Simplify'
SOURCE='AGENT_MODEL_PRESET_BASE64'

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

[[ -f "$NORMALIZER" ]] || fail 'SIMPLIFY normalizer is missing'
[[ "$(unset SIMPLIFY; bash "$NORMALIZER")" == True ]] || fail 'unset SIMPLIFY did not normalize to True'
for value in '' true True TRUE tRuE; do
  [[ "$(SIMPLIFY="$value" bash "$NORMALIZER")" == True ]] || fail "SIMPLIFY true case did not normalize to True"
done
for value in false False FALSE fAlSe; do
  [[ "$(SIMPLIFY="$value" bash "$NORMALIZER")" == False ]] || fail "SIMPLIFY false case did not normalize to False"
done

test_work="$(mktemp -d)"
trap 'rm -rf "$test_work"' EXIT
invalid_canary='invalid-simplify-canary-value'
for value in 1 0 yes no ' true' 'false ' 'ｔｒｕｅ' 'truе' 'fаlse' "$invalid_canary"; do
  error_file="$test_work/normalizer-error"
  output_file="$test_work/normalizer-output"
  if SIMPLIFY="$value" bash "$NORMALIZER" >"$output_file" 2>"$error_file"; then
    fail 'invalid SIMPLIFY passed normalization'
  fi
  [[ ! -s "$output_file" ]] || fail 'invalid SIMPLIFY produced standard output'
  [[ "$(<"$error_file")" == 'invalid SIMPLIFY value' ]] || fail 'invalid SIMPLIFY failure is not fixed and sanitized'
  if grep -Fq -- "$value" "$error_file"; then
    fail 'SIMPLIFY normalizer leaked configured input'
  fi
done

grep -Fq "${SOURCE}:-" "$LOCAL_BUILD" || fail "$(basename "$LOCAL_BUILD") does not read optional $SOURCE"
[[ "$(grep -Fc "$SYMBOL" "$LOCAL_BUILD")" -eq 1 ]] || fail "$(basename "$LOCAL_BUILD") must inject the preset symbol exactly once"
[[ "$(grep -Fc "$SIMPLIFY_SYMBOL" "$LOCAL_BUILD")" -eq 1 ]] || fail "$(basename "$LOCAL_BUILD") must inject the simplify symbol exactly once"

local_root_line="$(awk '/^cd "\$\(dirname "\$0"\)\/\.\."$/{print NR; exit}' "$LOCAL_BUILD")"
local_normalize_line="$(awk '/^simplify="\$\(bash \.\/scripts\/normalize-simplify\.sh\)"$/{print NR; exit}' "$LOCAL_BUILD")"
[[ -n "$local_root_line" && "$local_normalize_line" -eq $((local_root_line + 1)) ]] || fail 'local build must normalize SIMPLIFY immediately after root resolution'

local_router_block="$(awk '/^go build -trimpath/{build++} build == 1{print} build == 2{exit}' "$LOCAL_BUILD")"
local_manager_block="$(awk '/^go build -trimpath/{build++} build == 2{print}' "$LOCAL_BUILD")"
[[ "$local_router_block" != *"$SYMBOL"* ]] || fail 'local router build receives the Agent preset'
[[ "$local_manager_block" == *"$SYMBOL"* ]] || fail 'local manager build does not receive the Agent preset'
[[ "$local_router_block" != *"$SIMPLIFY_SYMBOL"* ]] || fail 'local router build receives the simplify setting'
[[ "$local_manager_block" == *"$SIMPLIFY_SYMBOL"'=${simplify}'* ]] || fail 'local manager build does not receive normalized simplify metadata'

# The desktop embeds the manager: build.rs carries the preset and normalized
# simplify setting into compile-time environment, and only the embedded
# manager assembly reads them. No Go linker symbol is involved.
[[ ! -e "$ROOT/desktop/scripts/build-sidecars.sh" ]] || fail 'desktop must not build Go sidecars'
grep -Fq 'env::var("AGENT_MODEL_PRESET_BASE64").unwrap_or_default()' "$DESKTOP_BUILD" || fail 'desktop build does not read the optional preset'
grep -Fq 'cargo:rustc-env=MTLS_AGENT_MODEL_PRESET_BASE64=' "$DESKTOP_BUILD" || fail 'desktop build does not embed the preset'
grep -Fq 'fn normalize_simplify' "$DESKTOP_BUILD" || fail 'desktop build does not normalize SIMPLIFY'
grep -Fq 'panic!("invalid SIMPLIFY value: {value}")' "$DESKTOP_BUILD" || fail 'desktop build does not reject invalid SIMPLIFY'
grep -Fq 'cargo:rustc-env=MTLS_SIMPLIFY=' "$DESKTOP_BUILD" || fail 'desktop build does not embed normalized simplify'
grep -Fq 'load_embedded_preset(env!("MTLS_AGENT_MODEL_PRESET_BASE64"))' "$DESKTOP_RUNTIME" || fail 'embedded manager does not load the compiled preset'
grep -Fq 'simplify: env!("MTLS_SIMPLIFY") == "true"' "$DESKTOP_RUNTIME" || fail 'embedded manager does not read normalized simplify'
grep -Fq 'pub fn load_embedded_preset' "$DESKTOP_SESSION" || fail 'embedded preset loader is missing'
grep -Fq 'base64::engine::general_purpose::STANDARD' "$DESKTOP_SESSION" || fail 'embedded preset loader must use strict standard base64 like Go preset.Load'
if grep -Fq "$SYMBOL" "$DESKTOP_BUILD" "$DESKTOP_RUNTIME"; then
  fail 'desktop build must not reference the Go manager linker symbol'
fi

[[ "$(grep -Fc "$SYMBOL" "$RELEASE")" -eq 0 ]] || fail 'release workflow must not inject the preset into a Go manager'
[[ "$(grep -Fc "$SIMPLIFY_SYMBOL" "$RELEASE")" -eq 0 ]] || fail 'release workflow must not inject simplify into a Go manager'
[[ "$(grep -Fc "AGENT_MODEL_PRESET_BASE64: \${{ vars.AGENT_MODEL_PRESET_BASE64 }}" "$RELEASE")" -eq 2 ]] || fail 'preflight and the desktop producer must source the same repository variable'
release_desktop_block="$(awk '$0 == "  desktop:" { capture=1; start=NR } capture && NR > start && $0 ~ /^  [A-Za-z0-9_-]+:/ { exit } capture { print }' "$RELEASE")"
[[ "$release_desktop_block" == *'AGENT_MODEL_PRESET_BASE64: ${{ vars.AGENT_MODEL_PRESET_BASE64 }}'* ]] || fail 'release desktop build does not receive the Agent preset'
[[ "$release_desktop_block" == *'SIMPLIFY: ${{ needs.prepare.outputs.simplify }}'* ]] || fail 'release desktop build does not receive prepared simplify'
[[ "$release_desktop_block" == *"RELEASE_BUILD: '1'"* ]] || fail 'release desktop build does not enable release guards'
grep -Fq 'run: ./scripts/preflight-agent-model-preset.sh' "$RELEASE" || fail 'release preset preflight is not configured'
grep -Fq "$SYMBOL" "$PREFLIGHT" || fail 'preflight does not validate through the exact manager linker symbol'

AGENT_MODEL_PRESET_BASE64='' "$PREFLIGHT" || fail 'empty preset preflight failed'
decoded_canary='malformed-decoded-preset-canary'
encoded_canary="$(printf '%s' "$decoded_canary" | base64 | tr -d '\r\n')"
preflight_error="$test_work/preflight-error"
if AGENT_MODEL_PRESET_BASE64="$encoded_canary" "$PREFLIGHT" >/dev/null 2>"$preflight_error"; then
  fail 'invalid configured preset passed preflight'
fi
grep -Fq '"code":"MANAGER_INIT_FAILED"' "$preflight_error" || fail 'invalid preset failure is not sanitized'
if grep -Fq "$decoded_canary" "$preflight_error" || grep -Fq "$encoded_canary" "$preflight_error"; then
  fail 'preset preflight leaked configured input'
fi

invalid_bin="$test_work/invalid-bin"
mkdir -p "$invalid_bin"
for tool in go rustc; do
  printf '%s\n' '#!/usr/bin/env bash' 'printf '\''%s\n'\'' "$(basename "$0")" >>"$FAKE_TOOL_LOG"' 'exit 99' >"$invalid_bin/$tool"
  chmod +x "$invalid_bin/$tool"
done
tool_log="$test_work/$(basename "$LOCAL_BUILD").tools"
error_file="$test_work/$(basename "$LOCAL_BUILD").error"
if PATH="$invalid_bin:$PATH" FAKE_TOOL_LOG="$tool_log" SIMPLIFY="$invalid_canary" bash "$LOCAL_BUILD" >/dev/null 2>"$error_file"; then
  fail "$(basename "$LOCAL_BUILD") accepted invalid SIMPLIFY"
fi
[[ "$(<"$error_file")" == 'invalid SIMPLIFY value' ]] || fail "$(basename "$LOCAL_BUILD") did not preserve the sanitized normalization error"
[[ ! -e "$tool_log" ]] || fail "$(basename "$LOCAL_BUILD") reached a compiler before rejecting SIMPLIFY"
if grep -Fq "$invalid_canary" "$error_file"; then
  fail "$(basename "$LOCAL_BUILD") leaked invalid SIMPLIFY"
fi

invocation_block() {
  local log=$1
  local wanted=$2

  awk -v wanted="$wanted" '
    $0 == "BEGIN" { invocation++; capture=(invocation == wanted); next }
    $0 == "END" { if (capture) exit; next }
    capture { print }
  ' "$log"
}

argument_after() {
  local block=$1
  local option=$2

  printf '%s\n' "$block" | awk -v option="$option" '$0 == option { getline; print; exit }'
}

assert_simplify_build_log() {
  local label=$1
  local log=$2
  local expected_router_output=$3
  local expected_manager_output=$4
  local router manager manager_ldflags
  local -a linker_words
  local simplify_assignments=0
  local i

  [[ "$(grep -Fxc BEGIN "$log")" -eq 2 && "$(grep -Fxc END "$log")" -eq 2 ]] || \
    fail "$label must invoke go exactly twice"
  router="$(invocation_block "$log" 1)"
  manager="$(invocation_block "$log" 2)"
  [[ "$(printf '%s\n' "$router" | awk 'NF { last=$0 } END { print last }')" == . ]] || \
    fail "$label first go invocation is not the router build"
  [[ "$(printf '%s\n' "$manager" | awk 'NF { last=$0 } END { print last }')" == ./cmd/mtls-router-manager ]] || \
    fail "$label second go invocation is not the manager build"
  [[ "$(argument_after "$router" -o)" == "$expected_router_output" ]] || \
    fail "$label router output argument changed unexpectedly"
  [[ "$(argument_after "$manager" -o)" == "$expected_manager_output" ]] || \
    fail "$label manager output argument changed unexpectedly"
  [[ "$router" != *"$SIMPLIFY_SYMBOL"* ]] || fail "$label router go arguments contain the simplify symbol"

  manager_ldflags="$(argument_after "$manager" -ldflags)"
  read -r -a linker_words <<<"$manager_ldflags"
  for ((i = 0; i + 1 < ${#linker_words[@]}; i++)); do
    if [[ "${linker_words[i]}" == -X && "${linker_words[i + 1]}" == "'$SIMPLIFY_SYMBOL=False'" ]]; then
      simplify_assignments=$((simplify_assignments + 1))
    fi
  done
  [[ "$simplify_assignments" -eq 1 ]] || \
    fail "$label manager ldflags do not contain exactly one exact Simplify=False assignment"
}

snapshot_real_build_paths() {
  local destination=$1
  local path metadata checksum
  shift
  : >"$destination"
  for path in "$@"; do
    if [[ ! -e "$path" ]]; then
      printf 'missing\t%s\n' "$path" >>"$destination"
      continue
    fi
    [[ -f "$path" ]] || fail "unexpected non-file build path: $path"
    if metadata="$(stat -f '%m:%z' "$path" 2>/dev/null)"; then
      :
    elif metadata="$(stat -c '%Y:%s' "$path" 2>/dev/null)"; then
      :
    else
      fail "cannot inspect build path metadata: $path"
    fi
    checksum="$(cksum <"$path")"
    printf 'file\t%s\t%s\t%s\n' "$path" "$metadata" "$checksum" >>"$destination"
  done
}

fixture="$test_work/success-fixture"
success_bin="$test_work/success-bin"
mkdir -p "$fixture/scripts" "$fixture/secrets" "$success_bin"
cp "$LOCAL_BUILD" "$fixture/scripts/build.sh"
cp "$NORMALIZER" "$fixture/scripts/normalize-simplify.sh"
printf '%s\n' fixture-client-cert >"$fixture/secrets/client.pem"
printf '%s\n' fixture-client-key >"$fixture/secrets/client.key"
printf '%s\n' fixture-upstream-ca >"$fixture/secrets/upstream-ca.pem"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  '{' \
  '  printf '\''%s\n'\'' BEGIN' \
  '  printf '\''%s\n'\'' "$@"' \
  '  printf '\''%s\n'\'' END' \
  '} >>"$FAKE_GO_LOG"' >"$success_bin/go"
chmod +x "$success_bin/go"

real_paths=(
  "$ROOT/mtls-router"
  "$ROOT/mtls-router-manager"
  "$ROOT/secrets/client.pem"
  "$ROOT/secrets/client.key"
  "$ROOT/secrets/upstream-ca.pem"
)
real_state_before="$test_work/real-state-before"
real_state_after="$test_work/real-state-after"
snapshot_real_build_paths "$real_state_before" "${real_paths[@]}"

local_go_log="$test_work/local-success-go.log"
PATH="$success_bin:$PATH" FAKE_GO_LOG="$local_go_log" SIMPLIFY=fAlSe VERSION=fixture-version \
  DEPLOYMENT_ID=fixture-deployment AGENT_MODEL_PRESET_BASE64='' bash "$fixture/scripts/build.sh" >/dev/null || \
  fail 'local build entry point rejected valid mixed-case SIMPLIFY=False'
assert_simplify_build_log local "$local_go_log" mtls-router mtls-router-manager

[[ "$(<"$fixture/secrets/client.pem")" == fixture-client-cert ]] || fail 'local fixture client certificate was modified'
[[ "$(<"$fixture/secrets/client.key")" == fixture-client-key ]] || fail 'local fixture client key was modified'
[[ "$(<"$fixture/secrets/upstream-ca.pem")" == fixture-upstream-ca ]] || fail 'local fixture upstream CA was modified'
[[ ! -e "$fixture/mtls-router" && ! -e "$fixture/mtls-router-manager" ]] || fail 'fake local build created binary output'

snapshot_real_build_paths "$real_state_after" "${real_paths[@]}"
cmp -s "$real_state_before" "$real_state_after" || fail 'isolated entry-point tests touched real repository outputs or secrets'

printf 'PASS: Agent model preset build and release integration\n'
