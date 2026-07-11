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
  cat >"$dir/fake-router.go" <<'ROUTER'
package main
import ("fmt"; "os"; "os/exec"; "path/filepath"; "time")
func main() { if len(os.Args)>1 && os.Args[1]=="--child" { for { time.Sleep(time.Second) } }; log:=filepath.Join(filepath.Dir(os.Args[0]),"mtls-router.log"); for i,a:=range os.Args { if (a=="-log"||a=="--log") && i+1<len(os.Args) { log=os.Args[i+1] } }; os.MkdirAll(filepath.Dir(log),0755); f,_:=os.OpenFile(log,os.O_CREATE|os.O_APPEND|os.O_WRONLY,0644); fmt.Fprintln(f,"fake router log with pid=999999 distraction"); f.Close(); c:=exec.Command(os.Args[0],"--child"); c.Start(); fmt.Println("debug pid=111 should not be parsed"); fmt.Printf("mtls-router started in background, pid=%d, log=%s\n",c.Process.Pid,log) }
ROUTER
  go build -o "$dir/mtls-router" "$dir/fake-router.go"
}

test_router_start_writes_state() {
  local tmp out state
  tmp="$(mktemp -d)"
  build_fake_router "$tmp"
  out="$(MTLS_ROUTER_SKIP_DOWNLOAD=1 MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router start 2>&1)"
  state="$tmp/home/.mtls-router/setup-state.json"
  assert_contains "mtls-router 已启动" "$out" "router start"
  [[ -f "$state" ]] || fail "router start should write state file"
  jq -e '.pid and .log_path and .binary_path and .listen_addr and .started_at and .process_started_at and .process_executable' "$state" >/dev/null || fail "state file should be JSON with router identity fields"
  [[ "$(stat -f %Lp "$state" 2>/dev/null || stat -c %a "$state")" == "600" ]] || fail "state file should be mode 600"
  assert_file_contains "$state" '"log_path":' "state file"
  assert_file_contains "$state" "$tmp/home/.mtls-router/mtls-router.log" "state file"
  assert_file_contains "$state" "$tmp/mtls-router" "state file"
  bash "$SCRIPT" router stop >/dev/null 2>&1 || true
  rm -rf "$tmp"
}

assert_stale_state_does_not_stop_router() {
  local tmp="$1" label="$2" pid status_out stop_out
  pid="$(jq -r '.pid' "$tmp/home/.mtls-router/setup-state.json")"
  status_out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router status 2>&1)"
  stop_out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router stop 2>&1)"
  assert_contains "stale" "$status_out" "$label status"
  [[ "$status_out" != *"router running"* ]] || fail "$label reported running"
  assert_contains "stale" "$stop_out" "$label stop"
  kill -0 "$pid" || fail "$label stop signaled the live process"
  [[ -f "$tmp/home/.mtls-router/setup-state.json" ]] || fail "$label state should be retained"
  kill "$pid"; wait "$pid" 2>/dev/null || true
  rm -rf "$tmp"
}

test_start_identity_mismatch_never_signals_live_process() {
  local tmp state
  tmp="$(mktemp -d)"
  build_fake_router "$tmp"
  MTLS_ROUTER_SKIP_DOWNLOAD=1 MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router start >/dev/null 2>&1
  state="$tmp/home/.mtls-router/setup-state.json"
  jq '.process_started_at = "reused-pid-start-identity"' "$state" >"$state.tmp"
  mv "$state.tmp" "$state"
  assert_stale_state_does_not_stop_router "$tmp" "start identity mismatch"
}

test_executable_mismatch_never_signals_live_process() {
  local tmp state
  tmp="$(mktemp -d)"
  build_fake_router "$tmp"
  MTLS_ROUTER_SKIP_DOWNLOAD=1 MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router start >/dev/null 2>&1
  state="$tmp/home/.mtls-router/setup-state.json"
  jq --arg executable "$tmp/unrelated-router" '.process_executable = $executable' "$state" >"$state.tmp"
  mv "$state.tmp" "$state"
  assert_stale_state_does_not_stop_router "$tmp" "executable mismatch"
}

test_missing_process_is_not_running() {
  local tmp status_out stop_out
  tmp="$(mktemp -d)"
  mkdir -p "$tmp/home/.mtls-router"
  jq -n --arg binary "$tmp/mtls-router" '{pid:999999,binary_path:$binary,process_started_at:"missing",process_executable:$binary}' >"$tmp/home/.mtls-router/setup-state.json"
  status_out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router status 2>&1)"
  stop_out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router stop 2>&1)"
  assert_contains "router 未运行" "$status_out" "missing process status"
  [[ "$status_out" != *"router running"* ]] || fail "missing process reported running"
  assert_contains "router 未运行" "$stop_out" "missing process stop"
  [[ -f "$tmp/home/.mtls-router/setup-state.json" ]] || fail "unconfirmed state should be retained"
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

test_replaced_running_binary_remains_managed_on_linux() {
  [[ "$(uname -s)" == "Linux" ]] || return 0
  local tmp pid stop_out
  tmp="$(mktemp -d)"
  build_fake_router "$tmp"
  MTLS_ROUTER_SKIP_DOWNLOAD=1 MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router start >/dev/null 2>&1
  pid="$(jq -r '.pid' "$tmp/home/.mtls-router/setup-state.json")"
  cp "$tmp/mtls-router" "$tmp/mtls-router.new"
  mv -f "$tmp/mtls-router.new" "$tmp/mtls-router"
  [[ "$(readlink "/proc/$pid/exe")" == *' (deleted)' ]] || fail "replaced running binary did not expose Linux deleted suffix"
  stop_out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router stop 2>&1)"
  assert_contains "stopped" "$stop_out" "stop after binary replacement"
  for _ in $(seq 1 25); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; done
  kill -0 "$pid" 2>/dev/null && fail "genuine replaced router process was not stopped"
  [[ ! -e "$tmp/home/.mtls-router/setup-state.json" ]] || fail "state remained after genuine stop"
  rm -rf "$tmp"
}

test_router_status_without_state_is_not_running() {
  local tmp out
  tmp="$(mktemp -d)"
  out="$(MTLS_ROUTER_INSTALL_DIR="$tmp" HOME="$tmp/home" bash "$SCRIPT" router status 2>&1)"
  assert_contains "router 未运行" "$out" "router status without state"
  rm -rf "$tmp"
}

test_router_start_writes_state
test_router_status_log_stop_use_state
test_router_status_without_state_is_not_running
test_start_identity_mismatch_never_signals_live_process
test_executable_mismatch_never_signals_live_process
test_missing_process_is_not_running
test_replaced_running_binary_remains_managed_on_linux
