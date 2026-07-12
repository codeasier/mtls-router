#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

platform="$(case "$(uname -s)" in Linux) printf linux;; Darwin) printf darwin;; *) fail unsupported;; esac)-$(case "$(uname -m)" in x86_64|amd64) printf amd64;; arm64|aarch64) printf arm64;; *) fail unsupported;; esac)"
router_asset="mtls-router-$platform"
manager_asset="mtls-router-manager-$platform"

write_pair() {
  local dir="$1" generation="$2"
  mkdir -p "$dir"
  cp "$ROOT/setup.sh" "$dir/setup.sh"
  cat >"$dir/$router_asset" <<ROUTER
#!/usr/bin/env bash
if [[ "\${1:-}" == --version || "\${1:-}" == -version ]]; then printf 'mtls-router $generation\\n'; exit; fi
exit 0
ROUTER
  cat >"$dir/$manager_asset" <<MANAGER
#!/usr/bin/env bash
set -euo pipefail
[[ "\${1:-}" == serve ]] || exit 1
IFS= read -r request
id="\$(printf '%s' "\$request" | jq -r '.id // empty')"
method="\$(printf '%s' "\$request" | jq -r '.method // empty')"
if [[ "\$method" == manager.info ]]; then
  jq -cn --arg id "\$id" '{id:\$id,result:{version:"$generation",commit:"test",build_date:"test",target:"test/test",deployment_id:"test-deployment",management_protocol_version:"1"}}'
else
  jq -cn --arg id "\$id" '{id:\$id,result:{}}'
fi
MANAGER
  chmod +x "$dir/setup.sh" "$dir/$router_asset" "$dir/$manager_asset"
  {
    printf '%s  %s\n' "$(sha256 "$dir/$router_asset")" "$router_asset"
    printf '%s  %s\n' "$(sha256 "$dir/$manager_asset")" "$manager_asset"
  } >"$dir/SHA256SUMS"
}

run_install() {
  local package="$1" install="$2" home="$3"
  shift 3
  MTLS_ROUTER_INSTALL_DIR="$install" MTLS_ROUTER_STATE_DIR="$home/state" HOME="$home" \
    bash "$package/setup.sh" router install "$@"
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
package="$tmp/package"
install="$tmp/install"
home="$tmp/home"
write_pair "$package" v2
run_install "$package" "$install" "$home" >/dev/null
[[ "$($install/mtls-router --version)" == 'mtls-router v2' ]] || fail "router was not installed"
[[ -x "$install/mtls-router-manager" ]] || fail "manager was not installed"
receipt="$home/state/install-receipt.json"
jq -e --arg router "$install/mtls-router" --arg manager "$install/mtls-router-manager" '
  .schema_version == 1 and .deployment_id == "test-deployment" and
  .management_protocol_version == "1" and .router.path == $router and
  .manager.path == $manager and .router.version == "v2" and .manager.version == "v2"' "$receipt" >/dev/null || fail "receipt metadata is incomplete"
[[ "$(stat -f %Lp "$receipt" 2>/dev/null || stat -c %a "$receipt")" == 600 ]] || fail "receipt is not private"

for missing in "$router_asset" "$manager_asset" SHA256SUMS; do
  rm -rf "$package"
  write_pair "$package" v3
  rm "$package/$missing"
  if run_install "$package" "$install" "$home" --download >/dev/null 2>&1; then fail "partial sibling package missing $missing should fail"; fi
  [[ "$($install/mtls-router --version)" == 'mtls-router v2' ]] || fail "partial package replaced router"
  [[ "$($install/mtls-router-manager serve <<< '{"id":"x","method":"manager.info"}' | jq -r .result.version)" == v2 ]] || fail "partial package replaced manager"
done

for mode in duplicate malformed mismatch; do
  rm -rf "$package"
  write_pair "$package" v3
  case "$mode" in
    duplicate) printf '%s  %s\n' "$(sha256 "$package/$router_asset")" "$router_asset" >>"$package/SHA256SUMS" ;;
    malformed) printf 'not-a-checksum  %s\n' "$manager_asset" >>"$package/SHA256SUMS" ;;
    mismatch) printf '%064d  %s\n' 0 "$manager_asset" >"$package/SHA256SUMS" ;;
  esac
  if run_install "$package" "$install" "$home" >/dev/null 2>&1; then fail "$mode manifest should fail"; fi
  [[ "$($install/mtls-router --version)" == 'mtls-router v2' ]] || fail "$mode changed committed pair"
done

for point in after-router after-manager before-receipt; do
  rm -rf "$package"
  write_pair "$package" "crash-$point"
  if MTLS_ROUTER_INSTALL_CRASH_POINT="$point" run_install "$package" "$install" "$home" >/dev/null 2>&1; then fail "$point should simulate a crash"; fi
  [[ -f "$home/state/install-pending.json" ]] || fail "$point did not leave pending marker"
  MTLS_ROUTER_INSTALL_DIR="$install" MTLS_ROUTER_STATE_DIR="$home/state" HOME="$home" \
    bash "$package/setup.sh" router status >/dev/null
  [[ ! -e "$home/state/install-pending.json" ]] || fail "$point was not reconciled before execution"
  router_version="$($install/mtls-router --version)"
  manager_version="$($install/mtls-router-manager serve <<< '{"id":"x","method":"manager.info"}' | jq -r .result.version)"
  case "$point" in
    after-router) [[ "$router_version" == 'mtls-router v2' && "$manager_version" == v2 ]] || fail "$point did not restore previous generation" ;;
    *) [[ "$router_version" == "mtls-router crash-$point" && "$manager_version" == "crash-$point" ]] || fail "$point did not commit complete new generation" ;;
  esac
done

# Network mode must fetch and verify router, manager, and one manifest before replacement.
rm -rf "$package" "$tmp/remote" "$tmp/bin"
mkdir -p "$package" "$tmp/remote" "$tmp/bin"
cp "$ROOT/setup.sh" "$package/setup.sh"
write_pair "$tmp/remote" network-v4
cat >"$tmp/bin/curl" <<'CURL'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$FAKE_CURL_LOG"
out=""; url=""
while (($#)); do case "$1" in -o) out="$2"; shift 2;; https://*) url="$1"; shift;; *) shift;; esac; done
cp "$FAKE_REMOTE_DIR/${url##*/}" "$out"
CURL
chmod +x "$tmp/bin/curl"
common=(env PATH="$tmp/bin:$PATH" FAKE_CURL_LOG="$tmp/curl.log" FAKE_REMOTE_DIR="$tmp/remote" MTLS_ROUTER_INSTALL_DIR="$install" MTLS_ROUTER_STATE_DIR="$home/state" HOME="$home" MTLS_ROUTER_DOWNLOAD_URL="https://downloads.example.test" MTLS_ROUTER_VERSION=v4)
"${common[@]}" bash "$package/setup.sh" router install --download >/dev/null
[[ "$($install/mtls-router --version)" == 'mtls-router network-v4' ]] || fail "network router was not installed"
[[ "$(wc -l <"$tmp/curl.log" | tr -d ' ')" == 3 ]] || fail "network install did not download exactly both binaries and manifest"

rm -f "$tmp/curl.log"
if "${common[@]}" MTLS_ROUTER_DOWNLOAD_URL=http://downloads.example.test bash "$package/setup.sh" router install --download >/dev/null 2>&1; then fail "HTTP download should fail"; fi
[[ ! -e "$tmp/curl.log" ]] || fail "HTTP rejection invoked downloader"

printf 'PASS: secure pair installation and reconciliation\n'
