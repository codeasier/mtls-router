#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

platform="$(case "$(uname -s)" in Linux) printf linux;; Darwin) printf darwin;; *) fail unsupported;; esac)-$(case "$(uname -m)" in x86_64|amd64) printf amd64;; arm64|aarch64) printf arm64;; *) fail unsupported;; esac)"
router_asset="mtls-router-$platform"
manager_asset="mtls-router-manager-$platform"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
package="$tmp/package"; home="$tmp/home"; install="$tmp/install"; bin="$tmp/bin"
clean="$tmp/clean"
mkdir -p "$package" "$clean" "$home" "$install" "$bin" "$home/codex"
export CODEX_HOME="$tmp/hostile-codex-home"
cp "$ROOT/setup.sh" "$package/setup.sh"
cat >"$package/$router_asset" <<'ROUTER'
#!/usr/bin/env bash
[[ "${1:-}" == --version || "${1:-}" == -version ]] && { printf 'mtls-router wrapper-test\n'; exit; }
exit 0
ROUTER
chmod +x "$package/setup.sh" "$package/$router_asset"
go build -trimpath -ldflags "-X github.com/codeasier/mtls-router/internal/version.Version=wrapper-test -X github.com/codeasier/mtls-router/internal/version.DeploymentID=wrapper-test-deployment" -o "$package/$manager_asset" "$ROOT/cmd/mtls-router-manager"
for command in claude opencode codex; do printf '#!/usr/bin/env bash\nexit 0\n' >"$bin/$command"; chmod +x "$bin/$command"; done
{
  printf '%s  %s\n' "$(sha256 "$package/$router_asset")" "$router_asset"
  printf '%s  %s\n' "$(sha256 "$package/$manager_asset")" "$manager_asset"
} >"$package/SHA256SUMS"

common=(env PATH="$bin:$PATH" HOME="$home" CODEX_HOME="$home/codex" CLAUDE_CONFIG_DIR="$home/claude" OPENCODE_CONFIG="$home/opencode.json" MTLS_ROUTER_INSTALL_DIR="$install" MTLS_ROUTER_STATE_DIR="$home/state" MTLS_ROUTER_DESKTOP_DATA_DIR="$home/desktop")

# No-argument setup installs the verified pair, can skip startup, and never writes Agent files.
MTLS_ROUTER_SKIP_START=1 "${common[@]}" bash "$package/setup.sh" >/dev/null
[[ -x "$install/mtls-router" && -x "$install/mtls-router-manager" ]] || fail "no-arg setup did not install pair"
[[ ! -e "$home/claude/settings.json" && ! -e "$home/opencode.json" && ! -e "$home/codex/config.toml" ]] || fail "no-arg setup changed Agent files"
cp "$ROOT/setup.sh" "$clean/setup.sh"

preview="$("${common[@]}" bash "$clean/setup.sh" agent print-config --agent=claude,opencode,codex 2>&1)"
[[ "$preview" == *"Claude Code"* && "$preview" == *"opencode"* && "$preview" == *"Codex"* ]] || fail "wrapper preview omitted selected Agent"
[[ "$preview" == *"未修改文件"* ]] || fail "wrapper preview did not preserve human output"

# The removed environment variable must not supply a secret or reach a file.
if MTLS_ROUTER_OPENAI_API_KEY=environment-canary "${common[@]}" bash "$clean/setup.sh" agent write-config --agent=claude >/dev/null 2>"$tmp/env-error"; then
  fail "removed MTLS_ROUTER_OPENAI_API_KEY unexpectedly supplied a key"
fi
grep -Fq 'MTLS_ROUTER_OPENAI_API_KEY 已移除' "$tmp/env-error" || fail "removed key migration guidance missing"
[[ ! -e "$home/claude/settings.json" ]] || fail "removed environment key changed Agent config"

# The public write command keeps hidden interactive input and sends it through manager stdin.
"${common[@]}" python3 - "$clean/setup.sh" <<'PY'
import os
import pty
import select
import sys

script = sys.argv[1]
secret = b"interactive-wrapper-key"
pid, fd = pty.fork()
if pid == 0:
    os.execvp("bash", ["bash", script, "agent", "write-config", "--agent=claude"])
output = bytearray()
sent = False
while True:
    ready, _, _ = select.select([fd], [], [], 10)
    if not ready:
        raise SystemExit("interactive wrapper timed out")
    try:
        chunk = os.read(fd, 4096)
    except OSError:
        break
    if not chunk:
        break
    output.extend(chunk)
    if not sent and "输入隐藏".encode() in output:
        os.write(fd, secret + b"\n")
        sent = True
_, status = os.waitpid(pid, 0)
if not sent or os.waitstatus_to_exitcode(status) != 0:
    raise SystemExit("interactive wrapper failed")
if secret in output:
    raise SystemExit("interactive key was echoed")
PY
jq -e '.env.ANTHROPIC_AUTH_TOKEN == "interactive-wrapper-key"' "$home/claude/settings.json" >/dev/null || fail "hidden interactive key was not written"

# Noninteractive automation talks directly to manager stdin: preview then write.
manager="$install/mtls-router-manager"; router="$install/mtls-router"
preview_json="$(printf '%s\n' '{"id":"p","method":"agent.preview","params":{"agents":["claude","opencode","codex"]}}' | "${common[@]}" "$manager" serve --router-sidecar "$router")"
token="$(printf '%s' "$preview_json" | jq -r .result.revision_token)"
request="$(jq -cn --arg token "$token" --arg key 'stdin-automation-key' '{id:"w",method:"agent.write",params:{agents:["claude","opencode","codex"],revision_token:$token,api_key:$key}}')"
response="$(printf '%s\n' "$request" | "${common[@]}" "$manager" serve --router-sidecar "$router")"
printf '%s' "$response" | jq -e '.result.agents | length == 3 and all(.success)' >/dev/null || fail "manager stdin automation failed"
[[ "$response" != *'stdin-automation-key'* ]] || fail "manager response leaked stdin key"
jq -e '.env.ANTHROPIC_AUTH_TOKEN == "stdin-automation-key"' "$home/claude/settings.json" >/dev/null || fail "Claude write missing stdin key"
jq -e '.provider."mtls-router".options.apiKey == "stdin-automation-key"' "$home/opencode.json" >/dev/null || fail "opencode write missing stdin key"
jq -e '.OPENAI_API_KEY == "stdin-automation-key" and (keys | length) == 1' "$home/codex/auth.json" >/dev/null || fail "Codex auth write is not minimal"
grep -Fq '[model_providers.custom]' "$home/codex/config.toml" || fail "Codex config missing provider"

# Compatibility aliases continue to execute manager previews and never download.
alias_out="$("${common[@]}" bash "$clean/setup.sh" --print-config --agent=claude 2>&1)"
[[ "$alias_out" == *"Claude Code"* ]] || fail "legacy print alias failed"
if "${common[@]}" bash "$clean/setup.sh" --write-config --agent=claude </dev/null >/dev/null 2>&1; then fail "legacy write alias accepted noninteractive keyless use"; fi

# Agent commands with no sibling or verified receipt fail without touching a downloader.
mkdir -p "$tmp/uninstalled-home"
if env PATH="$bin:$PATH" HOME="$tmp/uninstalled-home" CODEX_HOME="$tmp/uninstalled-home/codex" \
  MTLS_ROUTER_INSTALL_DIR="$tmp/uninstalled" MTLS_ROUTER_STATE_DIR="$tmp/uninstalled-state" \
  MTLS_ROUTER_ALLOW_DOWNLOAD=1 bash "$clean/setup.sh" agent print-config --agent=claude >/dev/null 2>"$tmp/uninstalled-error"; then
  fail "Agent command without verified manager should fail"
fi
grep -Fq '不会隐式下载' "$tmp/uninstalled-error" || fail "Agent missing-manager guidance is absent"

# A hostile inherited CODEX_HOME is not touched when the test fixes its isolated value.
[[ ! -e "$CODEX_HOME" ]] || fail "hostile outer CODEX_HOME was touched"

printf 'PASS: setup manager wrapper and stdin Agent automation\n'
