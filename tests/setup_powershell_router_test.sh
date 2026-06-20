#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/setup.ps1"

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

assert_contains() {
  local needle="$1" haystack="$2" label="$3"
  [[ "$haystack" == *"$needle"* ]] || fail "$label: missing '$needle'"
}

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
build_fake_router "$tmp"

help_out="$(pwsh -NoProfile -File "$SCRIPT" --help 2>&1)"
assert_contains "router status" "$help_out" "PowerShell help"

start_out="$(MTLS_ROUTER_SKIP_DOWNLOAD=1 MTLS_ROUTER_INSTALL_DIR="$tmp" USERPROFILE="$tmp/home" pwsh -NoProfile -File "$SCRIPT" router start 2>&1)"
assert_contains "mtls-router 已启动" "$start_out" "PowerShell router start"
state="$tmp/home/.mtls-router/setup-state.json"
[[ -f "$state" ]] || fail "PowerShell router start should write state file"
pwsh -NoProfile -Command "\$s = Get-Content '$state' -Raw | ConvertFrom-Json; if (-not \$s.pid -or -not \$s.log_path -or -not \$s.binary_path) { exit 1 }" || fail "PowerShell state file should be valid JSON"

status_out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" USERPROFILE="$tmp/home" pwsh -NoProfile -File "$SCRIPT" router status 2>&1)"
log_out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" USERPROFILE="$tmp/home" pwsh -NoProfile -File "$SCRIPT" router log --tail=1 2>&1)"
stop_out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" USERPROFILE="$tmp/home" pwsh -NoProfile -File "$SCRIPT" router stop 2>&1)"
assert_contains "running" "$status_out" "PowerShell router status"
assert_contains "fake powershell router log" "$log_out" "PowerShell router log"
assert_contains "stopped" "$stop_out" "PowerShell router stop"
