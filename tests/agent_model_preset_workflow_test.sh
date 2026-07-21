#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCAL_BUILD="$ROOT/scripts/build.sh"
SIDECAR_BUILD="$ROOT/desktop/scripts/build-sidecars.sh"
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

for script in "$LOCAL_BUILD" "$SIDECAR_BUILD"; do
  grep -Fq "${SOURCE}:-" "$script" || fail "$(basename "$script") does not read optional $SOURCE"
  [[ "$(grep -Fc "$SYMBOL" "$script")" -eq 1 ]] || fail "$(basename "$script") must inject the preset symbol exactly once"
  [[ "$(grep -Fc "$SIMPLIFY_SYMBOL" "$script")" -eq 1 ]] || fail "$(basename "$script") must inject the simplify symbol exactly once"
done

local_root_line="$(awk '/^cd "\$\(dirname "\$0"\)\/\.\."$/{print NR; exit}' "$LOCAL_BUILD")"
local_normalize_line="$(awk '/^simplify="\$\(bash \.\/scripts\/normalize-simplify\.sh\)"$/{print NR; exit}' "$LOCAL_BUILD")"
[[ -n "$local_root_line" && "$local_normalize_line" -eq $((local_root_line + 1)) ]] || fail 'local build must normalize SIMPLIFY immediately after root resolution'
desktop_root_line="$(awk '/^repo_dir=/{print NR; exit}' "$SIDECAR_BUILD")"
desktop_normalize_line="$(awk '/^simplify="\$\(bash "\$repo_dir\/scripts\/normalize-simplify\.sh"\)"$/{print NR; exit}' "$SIDECAR_BUILD")"
[[ -n "$desktop_root_line" && "$desktop_normalize_line" -eq $((desktop_root_line + 1)) ]] || fail 'desktop build must normalize SIMPLIFY immediately after root resolution'

local_router_block="$(awk '/^go build -trimpath/{build++} build == 1{print} build == 2{exit}' "$LOCAL_BUILD")"
local_manager_block="$(awk '/^go build -trimpath/{build++} build == 2{print}' "$LOCAL_BUILD")"
[[ "$local_router_block" != *"$SYMBOL"* ]] || fail 'local router build receives the Agent preset'
[[ "$local_manager_block" == *"$SYMBOL"* ]] || fail 'local manager build does not receive the Agent preset'
[[ "$local_router_block" != *"$SIMPLIFY_SYMBOL"* ]] || fail 'local router build receives the simplify setting'
[[ "$local_manager_block" == *"$SIMPLIFY_SYMBOL"'=${simplify}'* ]] || fail 'local manager build does not receive normalized simplify metadata'

sidecar_router_line="$(grep 'go build.*-o "\$router"' "$SIDECAR_BUILD")"
sidecar_manager_line="$(grep 'go build.*-o "\$manager"' "$SIDECAR_BUILD")"
[[ "$sidecar_router_line" != *manager_metadata* && "$sidecar_router_line" != *"$SYMBOL"* ]] || fail 'desktop router sidecar receives the Agent preset'
[[ "$sidecar_manager_line" == *manager_metadata* ]] || fail 'desktop manager sidecar does not receive manager-only metadata'
[[ "$sidecar_router_line" != *"$SIMPLIFY_SYMBOL"* ]] || fail 'desktop router sidecar receives the simplify setting'
grep -Fq "$SIMPLIFY_SYMBOL"'=$simplify' <<<"$(grep '^manager_metadata=' "$SIDECAR_BUILD")" || fail 'desktop manager metadata does not receive normalized simplify metadata'

[[ "$(grep -Fc "$SYMBOL" "$RELEASE")" -eq 1 ]] || fail 'release workflow must inject the preset directly only into the standalone manager'
[[ "$(grep -Fc "$SIMPLIFY_SYMBOL" "$RELEASE")" -eq 1 ]] || fail 'release workflow must inject simplify directly only into the standalone manager'
release_router_block="$(awk '/go build -trimpath/{build++} build == 1{print} build == 2{exit}' "$RELEASE")"
release_manager_block="$(awk '/go build -trimpath/{build++} build == 2{print} build == 3{exit}' "$RELEASE")"
[[ "$release_router_block" != *"$SYMBOL"* ]] || fail 'release router receives the Agent preset'
[[ "$release_manager_block" == *"$SYMBOL"* ]] || fail 'standalone release manager does not receive the Agent preset'
[[ "$release_router_block" != *"$SIMPLIFY_SYMBOL"* ]] || fail 'release router receives the simplify setting'
[[ "$release_manager_block" == *"$SIMPLIFY_SYMBOL"'=${SIMPLIFY}'* ]] || fail 'standalone release manager does not receive prepared simplify metadata'
[[ "$(grep -Fc "AGENT_MODEL_PRESET_BASE64: \${{ vars.AGENT_MODEL_PRESET_BASE64 }}" "$RELEASE")" -eq 3 ]] || fail 'preflight and both release manager producers must source the same repository variable'
grep -Fq 'run: ./scripts/preflight-agent-model-preset.sh' "$RELEASE" || fail 'release preset preflight is not configured'
grep -Fq "$SYMBOL" "$PREFLIGHT" || fail 'preflight does not validate through the exact manager linker symbol'

AGENT_MODEL_PRESET_BASE64='' "$PREFLIGHT" || fail 'empty preset preflight failed'
decoded_canary='malformed-decoded-preset-canary'
encoded_canary="$(printf '%s' "$decoded_canary" | base64 | tr -d '\r\n')"
preflight_error="$test_work/preflight-error"
if AGENT_MODEL_PRESET_BASE64="$encoded_canary" "$PREFLIGHT" >/dev/null 2>"$preflight_error"; then
  fail 'invalid configured preset passed preflight'
fi
grep -Fq 'invalid embedded Agent model preset' "$preflight_error" || fail 'invalid preset failure is not sanitized'
if grep -Fq "$decoded_canary" "$preflight_error" || grep -Fq "$encoded_canary" "$preflight_error"; then
  fail 'preset preflight leaked configured input'
fi

invalid_bin="$test_work/invalid-bin"
mkdir -p "$invalid_bin"
for tool in go rustc; do
  printf '%s\n' '#!/usr/bin/env bash' 'printf '\''%s\n'\'' "$(basename "$0")" >>"$FAKE_TOOL_LOG"' 'exit 99' >"$invalid_bin/$tool"
  chmod +x "$invalid_bin/$tool"
done
for script in "$LOCAL_BUILD" "$SIDECAR_BUILD"; do
  tool_log="$test_work/$(basename "$script").tools"
  error_file="$test_work/$(basename "$script").error"
  if PATH="$invalid_bin:$PATH" FAKE_TOOL_LOG="$tool_log" SIMPLIFY="$invalid_canary" bash "$script" >/dev/null 2>"$error_file"; then
    fail "$(basename "$script") accepted invalid SIMPLIFY"
  fi
  [[ "$(<"$error_file")" == 'invalid SIMPLIFY value' ]] || fail "$(basename "$script") did not preserve the sanitized normalization error"
  [[ ! -e "$tool_log" ]] || fail "$(basename "$script") reached a compiler before rejecting SIMPLIFY"
  if grep -Fq "$invalid_canary" "$error_file"; then
    fail "$(basename "$script") leaked invalid SIMPLIFY"
  fi
done

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
mkdir -p "$fixture/scripts" "$fixture/desktop/scripts" "$fixture/desktop/src-tauri" "$fixture/secrets" "$success_bin"
cp "$LOCAL_BUILD" "$fixture/scripts/build.sh"
cp "$SIDECAR_BUILD" "$fixture/desktop/scripts/build-sidecars.sh"
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
  "$ROOT/desktop/src-tauri/binaries/mtls-router-x86_64-unknown-linux-gnu"
  "$ROOT/desktop/src-tauri/binaries/mtls-router-manager-x86_64-unknown-linux-gnu"
)
real_state_before="$test_work/real-state-before"
real_state_after="$test_work/real-state-after"
snapshot_real_build_paths "$real_state_before" "${real_paths[@]}"

local_go_log="$test_work/local-success-go.log"
PATH="$success_bin:$PATH" FAKE_GO_LOG="$local_go_log" SIMPLIFY=fAlSe VERSION=fixture-version \
  DEPLOYMENT_ID=fixture-deployment AGENT_MODEL_PRESET_BASE64='' bash "$fixture/scripts/build.sh" >/dev/null || \
  fail 'local build entry point rejected valid mixed-case SIMPLIFY=False'
assert_simplify_build_log local "$local_go_log" mtls-router mtls-router-manager

desktop_go_log="$test_work/desktop-success-go.log"
desktop_target=x86_64-unknown-linux-gnu
PATH="$success_bin:$PATH" FAKE_GO_LOG="$desktop_go_log" SIMPLIFY=fAlSe TARGET="$desktop_target" \
  VERSION=1.2.3 DEPLOYMENT_ID=fixture-deployment AGENT_MODEL_PRESET_BASE64='' \
  bash "$fixture/desktop/scripts/build-sidecars.sh" >/dev/null || \
  fail 'desktop build entry point rejected valid mixed-case SIMPLIFY=False'
assert_simplify_build_log desktop "$desktop_go_log" \
  "$fixture/desktop/src-tauri/binaries/mtls-router-$desktop_target" \
  "$fixture/desktop/src-tauri/binaries/mtls-router-manager-$desktop_target"

[[ "$(<"$fixture/secrets/client.pem")" == fixture-client-cert ]] || fail 'local fixture client certificate was modified'
[[ "$(<"$fixture/secrets/client.key")" == fixture-client-key ]] || fail 'local fixture client key was modified'
[[ "$(<"$fixture/secrets/upstream-ca.pem")" == fixture-upstream-ca ]] || fail 'local fixture upstream CA was modified'
[[ ! -e "$fixture/mtls-router" && ! -e "$fixture/mtls-router-manager" ]] || fail 'fake local build created binary output'
[[ ! -e "$fixture/desktop/src-tauri/binaries/mtls-router-$desktop_target" ]] || fail 'fake desktop build created router output'
[[ ! -e "$fixture/desktop/src-tauri/binaries/mtls-router-manager-$desktop_target" ]] || fail 'fake desktop build created manager output'

snapshot_real_build_paths "$real_state_after" "${real_paths[@]}"
cmp -s "$real_state_before" "$real_state_after" || fail 'isolated entry-point tests touched real repository outputs or secrets'

printf 'PASS: Agent model preset build and release integration\n'
