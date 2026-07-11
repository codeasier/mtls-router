#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

new_package() {
  local dir="$1" asset="$2"
  mkdir -p "$dir"
  cp "$ROOT/setup.sh" "$dir/setup.sh"
  printf 'new-binary\n' >"$dir/$asset"
  printf '%s  %s\n' "$(sha256 "$dir/$asset")" "$asset" >"$dir/SHA256SUMS"
}

run_install() {
  local package="$1" install="$2"
  shift 2
  (cd / && MTLS_ROUTER_INSTALL_DIR="$install" MTLS_ROUTER_SKIP_START=1 bash "$package/setup.sh" router install "$@")
}

asset="mtls-router-$(case "$(uname -s)" in Linux) printf linux;; Darwin) printf darwin;; *) fail unsupported;; esac)-$(case "$(uname -m)" in x86_64|amd64) printf amd64;; arm64|aarch64) printf arm64;; *) fail unsupported;; esac)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

package="$tmp/package"
install="$tmp/install"
new_package "$package" "$asset"
run_install "$package" "$install"
[[ "$(cat "$install/mtls-router")" == "new-binary" ]] || fail "sibling payload was not installed"
[[ -x "$install/mtls-router" ]] || fail "installed payload is not executable"

for mode in missing duplicate malformed mismatch valid-plus-malformed valid-plus-single-space; do
  rm -rf "$package" "$install"
  new_package "$package" "$asset"
  mkdir -p "$install"
  printf 'old-binary\n' >"$install/mtls-router"
  case "$mode" in
    missing) rm "$package/SHA256SUMS" ;;
    duplicate) cat "$package/SHA256SUMS" >>"$package/SHA256SUMS.copy"; cat "$package/SHA256SUMS.copy" >>"$package/SHA256SUMS" ;;
    malformed) printf '%s %s\n' "$(sha256 "$package/$asset")" "$asset" >"$package/SHA256SUMS" ;;
    mismatch) printf '%064d  %s\n' 0 "$asset" >"$package/SHA256SUMS" ;;
    valid-plus-malformed) printf 'not-a-checksum  %s\n' "$asset" >>"$package/SHA256SUMS" ;;
    valid-plus-single-space) printf '%s %s\n' "$(sha256 "$package/$asset")" "$asset" >>"$package/SHA256SUMS" ;;
  esac
  if run_install "$package" "$install" >/dev/null 2>&1; then fail "$mode manifest should fail"; fi
  [[ "$(cat "$install/mtls-router")" == "old-binary" ]] || fail "$mode failure replaced installed binary"
done

rm -rf "$package" "$install"
mkdir -p "$package" "$install" "$tmp/bin" "$tmp/remote"
cp "$ROOT/setup.sh" "$package/setup.sh"
printf 'old-binary\n' >"$install/mtls-router"
printf 'remote-binary\n' >"$tmp/remote/$asset"
printf '%s  %s\n' "$(sha256 "$tmp/remote/$asset")" "$asset" >"$tmp/remote/SHA256SUMS"
cat >"$tmp/bin/curl" <<'CURL'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$FAKE_CURL_LOG"
out=""; url=""
while (( $# )); do
  case "$1" in -o) out="$2"; shift 2;; http*) url="$1"; shift;; *) shift;; esac
done
cp "$FAKE_REMOTE_DIR/${url##*/}" "$out"
CURL
chmod +x "$tmp/bin/curl"

common=(env PATH="$tmp/bin:$PATH" FAKE_CURL_LOG="$tmp/curl.log" FAKE_REMOTE_DIR="$tmp/remote" MTLS_ROUTER_INSTALL_DIR="$install" MTLS_ROUTER_DOWNLOAD_URL="https://downloads.example.test" MTLS_ROUTER_VERSION=v1 MTLS_ROUTER_SKIP_START=1)
if "${common[@]}" bash "$package/setup.sh" router install >/dev/null 2>&1; then fail "non-interactive fallback without authorization should fail"; fi
[[ ! -e "$tmp/curl.log" ]] || fail "unauthorized fallback invoked downloader"

"${common[@]}" bash "$package/setup.sh" router install --download >/dev/null
[[ "$(cat "$install/mtls-router")" == "remote-binary" ]] || fail "--download did not install verified remote payload"

printf 'old-binary\n' >"$install/mtls-router"
rm -f "$tmp/curl.log"
MTLS_ROUTER_ALLOW_DOWNLOAD=1 "${common[@]}" bash "$package/setup.sh" router install >/dev/null
[[ "$(cat "$install/mtls-router")" == "remote-binary" ]] || fail "environment authorization did not install payload"

printf 'old-binary\n' >"$install/mtls-router"
printf '%064d  %s\n' 0 "$asset" >"$tmp/remote/SHA256SUMS"
if "${common[@]}" bash "$package/setup.sh" router install --download >/dev/null 2>&1; then fail "remote mismatch should fail"; fi
[[ "$(cat "$install/mtls-router")" == "old-binary" ]] || fail "remote mismatch replaced installed binary"

rm -f "$tmp/curl.log"
if env PATH="$tmp/bin:$PATH" FAKE_CURL_LOG="$tmp/curl.log" FAKE_REMOTE_DIR="$tmp/remote" MTLS_ROUTER_INSTALL_DIR="$install" MTLS_ROUTER_DOWNLOAD_URL="http://downloads.example.test" MTLS_ROUTER_VERSION=v1 MTLS_ROUTER_SKIP_START=1 bash "$package/setup.sh" router install --download >/dev/null 2>&1; then fail "HTTP fallback should fail"; fi
[[ ! -e "$tmp/curl.log" ]] || fail "HTTP fallback invoked downloader"

printf 'PASS: secure Unix installation\n'
