#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
platform="$(case "$(uname -s)" in Linux) printf linux;; Darwin) printf darwin;; *) fail unsupported;; esac)-$(case "$(uname -m)" in x86_64|amd64) printf amd64;; arm64|aarch64) printf arm64;; *) fail unsupported;; esac)"
router_asset="mtls-router-$platform"; manager_asset="mtls-router-manager-$platform"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/package" "$tmp/home"
cp "$ROOT/setup.sh" "$tmp/package/setup.sh"
cat >"$tmp/package/$router_asset" <<'ROUTER'
#!/usr/bin/env bash
[[ "${1:-}" == --version || "${1:-}" == -version ]] && printf 'mtls-router fixture\n'
ROUTER
cat >"$tmp/package/$manager_asset" <<'MANAGER'
#!/usr/bin/env bash
set -euo pipefail
IFS= read -r request
id="$(printf '%s' "$request" | jq -r .id)"; method="$(printf '%s' "$request" | jq -r .method)"
case "$method" in
  manager.info) result='{"version":"fixture","commit":"x","build_date":"x","target":"x/x","deployment_id":"fixture-deployment","management_protocol_version":"1"}' ;;
  router.start) result='{"state":"external_compatible","owner":"cli","listen_addr":"127.0.0.1:19099","pid":4242}' ;;
  router.status) result='{"state":"external_compatible","owner":"cli","listen_addr":"127.0.0.1:19099","pid":4242}' ;;
  router.logs) result='{"lines":["fixture router log"]}' ;;
  router.stop) result='{"state":"absent"}' ;;
  *) result='{}' ;;
esac
jq -cn --arg id "$id" --argjson result "$result" '{id:$id,result:$result}'
MANAGER
chmod +x "$tmp/package/setup.sh" "$tmp/package/$router_asset" "$tmp/package/$manager_asset"
{
  printf '%s  %s\n' "$(sha256 "$tmp/package/$router_asset")" "$router_asset"
  printf '%s  %s\n' "$(sha256 "$tmp/package/$manager_asset")" "$manager_asset"
} >"$tmp/package/SHA256SUMS"
common=(env HOME="$tmp/home" MTLS_ROUTER_INSTALL_DIR="$tmp/install" MTLS_ROUTER_STATE_DIR="$tmp/home/state")

"${common[@]}" bash "$tmp/package/setup.sh" router install >/dev/null
start="$("${common[@]}" bash "$tmp/package/setup.sh" router start 2>&1)"
status="$("${common[@]}" bash "$tmp/package/setup.sh" router status 2>&1)"
logs="$("${common[@]}" bash "$tmp/package/setup.sh" router log --tail=1 2>&1)"
stop="$("${common[@]}" bash "$tmp/package/setup.sh" router stop 2>&1)"
[[ "$start" == *'mtls-router 已启动'* && "$start" == *'pid=4242'* ]] || fail "router start output changed"
[[ "$status" == *'router running'* && "$status" == *'pid=4242'* ]] || fail "router status output changed"
[[ "$logs" == *'fixture router log'* ]] || fail "router log output changed"
[[ "$stop" == *'router stopped'* ]] || fail "router stop output changed"

printf 'PASS: setup router lifecycle manager calls\n'
