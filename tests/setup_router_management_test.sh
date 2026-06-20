#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/setup.sh"

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

assert_contains() {
  local needle="$1" haystack="$2" label="$3"
  [[ "$haystack" == *"$needle"* ]] || fail "$label: missing '$needle'"
}

assert_file_contains() {
  local path="$1" needle="$2" label="$3"
  grep -F -q -- "$needle" "$path" || fail "$label: $path missing '$needle'"
}

build_fake_router() {
  local dir="$1"
  cat >"$dir/mtls-router" <<'ROUTER'
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
printf 'fake router log\n' >>"$log"
sleep 60 >/dev/null 2>&1 &
pid=$!
printf 'mtls-router started in background, pid=%s, log=%s\n' "$pid" "$log"
ROUTER
  chmod +x "$dir/mtls-router"
}

test_router_start_writes_state() {
  local tmp out state
  tmp="$(mktemp -d)"
  build_fake_router "$tmp"
  out="$(MTLS_ROUTER_SKIP_DOWNLOAD=1 MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router start 2>&1)"
  state="$tmp/home/.mtls-router/setup-state"
  assert_contains "mtls-router 已启动" "$out" "router start"
  [[ -f "$state" ]] || fail "router start should write state file"
  assert_file_contains "$state" "pid=" "state file"
  assert_file_contains "$state" "log_path=$tmp/home/.mtls-router/mtls-router.log" "state file"
  assert_file_contains "$state" "binary_path=$tmp/mtls-router" "state file"
  bash "$SCRIPT" router stop >/dev/null 2>&1 || true
  rm -rf "$tmp"
}

test_router_status_log_stop_use_state() {
  local tmp start_out status_out log_out stop_out
  tmp="$(mktemp -d)"
  build_fake_router "$tmp"
  start_out="$(MTLS_ROUTER_SKIP_DOWNLOAD=1 MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router start 2>&1)"
  status_out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router status 2>&1)"
  log_out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router log 2>&1)"
  stop_out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router stop 2>&1)"
  assert_contains "running" "$status_out" "router status"
  assert_contains "pid=" "$status_out" "router status"
  assert_contains "binary_path=$tmp/mtls-router" "$status_out" "router status"
  assert_contains "log_path=$tmp/home/.mtls-router/mtls-router.log" "$status_out" "router status"
  assert_contains "fake router log" "$log_out" "router log"
  assert_contains "stopped" "$stop_out" "router stop"
  rm -rf "$tmp"
}

test_router_status_without_state_is_not_running() {
  local tmp out
  tmp="$(mktemp -d)"
  out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router status 2>&1)"
  assert_contains "not running" "$out" "router status without state"
  rm -rf "$tmp"
}

test_router_start_writes_state
test_router_status_log_stop_use_state
test_router_status_without_state_is_not_running
