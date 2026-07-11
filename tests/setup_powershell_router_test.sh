#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/setup.ps1"

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

assert_contains() {
  local needle="$1" haystack="$2" label="$3"
  [[ "$haystack" == *"$needle"* ]] || fail "$label: missing '$needle'"
}

grep -q -- '--download' "$SCRIPT" || fail "PowerShell setup should parse --download"
grep -q "Scheme -ne 'https'" "$SCRIPT" || fail "PowerShell setup should reject non-HTTPS downloads"
grep -q 'Get-FileHash.*SHA256' "$SCRIPT" || fail "PowerShell setup should verify SHA-256"
grep -q '\$PSScriptRoot' "$SCRIPT" || fail "PowerShell setup should discover sibling payloads"
grep -q 'process_started_at' "$SCRIPT" || fail "PowerShell state should persist process start identity"
grep -q 'process_executable' "$SCRIPT" || fail "PowerShell state should persist executable identity"
grep -q 'Get-RouterIdentityStatus' "$SCRIPT" || fail "PowerShell signals should validate full process identity"
grep -q 'SecurityProtocol.*-bor.*Tls12' "$SCRIPT" || fail "PowerShell setup should preserve protocols and enable TLS 1.2"

if ! command -v pwsh >/dev/null 2>&1; then
  printf 'skip: pwsh not available\n'
  exit 0
fi

build_fake_router() {
  local dir="$1"
  cat >"$dir/mtls-router.exe" <<'ROUTER'
#!/usr/bin/env bash
set -euo pipefail
log=""
while (( $# > 0 )); do
  case "$1" in
    -log|--log)
      log="$2"
      shift 2
      ;;
    -log=*|--log=*)
      log="${1#*=}"
      shift
      ;;
    -backend|--backend)
      shift
      ;;
    *)
      shift
      ;;
  esac
done
[[ -n "$log" ]] || log="$(dirname "$0")/mtls-router.log"
mkdir -p "$(dirname "$log")"
printf 'fake powershell router log with pid=999999 distraction\n' >>"$log"
sleep 60 >/dev/null 2>&1 &
pid=$!
printf 'debug pid=111 should not be parsed\n'
printf 'mtls-router started in background, pid=%s, log=%s\n' "$pid" "$log"
ROUTER
  chmod +x "$dir/mtls-router.exe"
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

package="$tmp/package"
install="$tmp/install"
mkdir -p "$package" "$install"
cp "$SCRIPT" "$package/setup.ps1"
printf 'packaged-windows-binary\n' >"$package/mtls-router-windows-amd64.exe"
hash="$(shasum -a 256 "$package/mtls-router-windows-amd64.exe" | cut -d' ' -f1)"
printf '%s  mtls-router-windows-amd64.exe\n' "$hash" >"$package/SHA256SUMS"
PROCESSOR_ARCHITECTURE=AMD64 MTLS_ROUTER_INSTALL_DIR="$install" USERPROFILE="$tmp/home" pwsh -NoProfile -File "$package/setup.ps1" router install >/dev/null
[[ "$(cat "$install/mtls-router.exe")" == "packaged-windows-binary" ]] || fail "PowerShell sibling payload was not installed"

printf 'old-binary\n' >"$install/mtls-router.exe"
printf '%064d  mtls-router-windows-amd64.exe\n' 0 >"$package/SHA256SUMS"
if PROCESSOR_ARCHITECTURE=AMD64 MTLS_ROUTER_INSTALL_DIR="$install" USERPROFILE="$tmp/home" pwsh -NoProfile -File "$package/setup.ps1" router install >/dev/null 2>&1; then
  fail "PowerShell checksum mismatch should fail"
fi
[[ "$(cat "$install/mtls-router.exe")" == "old-binary" ]] || fail "PowerShell checksum mismatch replaced installed binary"

for extra in 'not-a-checksum  mtls-router-windows-amd64.exe' "$hash mtls-router-windows-amd64.exe"; do
  printf 'old-binary\n' >"$install/mtls-router.exe"
  printf '%s  mtls-router-windows-amd64.exe\n%s\n' "$hash" "$extra" >"$package/SHA256SUMS"
  if PROCESSOR_ARCHITECTURE=AMD64 MTLS_ROUTER_INSTALL_DIR="$install" USERPROFILE="$tmp/home" pwsh -NoProfile -File "$package/setup.ps1" router install >/dev/null 2>&1; then
    fail "PowerShell ambiguous checksum candidates should fail"
  fi
  [[ "$(cat "$install/mtls-router.exe")" == "old-binary" ]] || fail "PowerShell ambiguous checksum replaced installed binary"
done

tls_out="$(pwsh -NoProfile -Command "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls; & '$SCRIPT' --help > \$null; [Net.ServicePointManager]::SecurityProtocol.ToString()")"
assert_contains "Tls" "$tls_out" "PowerShell preserved existing TLS protocol"
assert_contains "Tls12" "$tls_out" "PowerShell enabled TLS 1.2"

mkdir -p "$tmp/stale-home/.mtls-router"
sleep 60 & stale_pid=$!
pwsh -NoProfile -Command "[ordered]@{ pid = $stale_pid; binary_path = '$tmp/not-the-process'; process_started_at = 'forged'; process_executable = '$tmp/not-the-process' } | ConvertTo-Json | Set-Content -LiteralPath '$tmp/stale-home/.mtls-router/setup-state.json'"
stale_out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" USERPROFILE="$tmp/stale-home" pwsh -NoProfile -File "$SCRIPT" router stop 2>&1)"
assert_contains "stale" "$stale_out" "PowerShell stale router stop"
kill -0 "$stale_pid" || fail "PowerShell stale stop signaled unrelated process"
[[ -f "$tmp/stale-home/.mtls-router/setup-state.json" ]] || fail "PowerShell stale stop removed retained state"
kill "$stale_pid"
wait "$stale_pid" 2>/dev/null || true

build_fake_router "$tmp"

help_out="$(pwsh -NoProfile -File "$SCRIPT" --help 2>&1)"
assert_contains "router status" "$help_out" "PowerShell help"

start_out="$(MTLS_ROUTER_SKIP_DOWNLOAD=1 MTLS_ROUTER_INSTALL_DIR="$tmp" USERPROFILE="$tmp/home" pwsh -NoProfile -File "$SCRIPT" router start 2>&1)"
assert_contains "mtls-router 已启动" "$start_out" "PowerShell router start"
state="$tmp/home/.mtls-router/setup-state.json"
[[ -f "$state" ]] || fail "PowerShell router start should write state file"
pwsh -NoProfile -Command "\$s = Get-Content '$state' -Raw | ConvertFrom-Json; if (-not \$s.pid -or -not \$s.log_path -or -not \$s.binary_path -or -not \$s.process_started_at -or -not \$s.process_executable) { exit 1 }" || fail "PowerShell state file should contain process identity"

status_out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" USERPROFILE="$tmp/home" pwsh -NoProfile -File "$SCRIPT" router status 2>&1)"
log_out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" USERPROFILE="$tmp/home" pwsh -NoProfile -File "$SCRIPT" router log --tail=1 2>&1)"
stop_out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" USERPROFILE="$tmp/home" pwsh -NoProfile -File "$SCRIPT" router stop 2>&1)"
assert_contains "running" "$status_out" "PowerShell router status"
assert_contains "fake powershell router log" "$log_out" "PowerShell router log"
assert_contains "stopped" "$stop_out" "PowerShell router stop"
