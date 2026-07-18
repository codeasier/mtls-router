#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
if ! command -v pwsh >/dev/null 2>&1; then printf 'skip: pwsh not available; executable PowerShell v2 flow not run\n'; exit 0; fi
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT; package="$tmp/package"; home="$tmp/home"; bin="$tmp/bin"; mkdir -p "$package" "$home" "$bin"
cp "$ROOT/setup.ps1" "$package/setup.ps1"; router='mtls-router-windows-amd64.exe'; manager='mtls-router-manager-windows-amd64.exe'; printf '#!/usr/bin/env bash\nexit 0\n' >"$package/$router"
cat >"$package/$manager" <<'MANAGER'
#!/usr/bin/env bash
set -euo pipefail
request="$(dd bs=4194304 count=1 2>/dev/null)"; method="$(printf '%s' "$request" | jq -r .method)"
case "$method" in
  manager.info) jq -cn '{id:"setup-secret-info",result:{version:"v2",commit:"test",build_date:"test",target:"windows/amd64",deployment_id:"fake",management_protocol_version:"2"}}' ;;
  agent.detect) jq -cn '{id:"detect",result:{agents:[{agent:"claude",name:"Claude Code",detected:true,path:"C:\\special path\\settings.json",format:"json"}]}}' ;;
  agent.models) printf '%s' "$request" | jq -e '.params.owner=="cli" and .params.api_key=="ps-v2-key-canary"' >/dev/null; printf 'models-ok\n' >>"$FAKE_LOG"; jq -cn '{id:"models",result:{models:["模型/PS [1]"],catalog_token:"ps-catalog",router_base_url:"http://127.0.0.1:19099",api_base_url:"http://127.0.0.1:19099/v1",existing:{model_config:{},unavailable_models:{},drifted_agents:[]}}}' ;;
  agent.render) printf '%s' "$request" | jq -e '.params.catalog_token=="ps-catalog" and .params.model_config.claude.primary.model=="模型/PS [1]"' >/dev/null; printf 'render-ok\n' >>"$FAKE_LOG"; jq -cn '{id:"render",result:{model_config:{version:1},fragments:[{agent:"claude",role:"config",path:"C:\\special path\\settings.json",format:"json",content:"PowerShell dynamic redacted fragment"}]}}' ;;
  *) exit 8 ;;
esac
MANAGER
chmod +x "$package/$router" "$package/$manager"; for command in claude; do printf '#!/usr/bin/env bash\nexit 0\n' >"$bin/$command"; chmod +x "$bin/$command"; done
if command -v sha256sum >/dev/null 2>&1; then rh="$(sha256sum "$package/$router" | cut -d' ' -f1)"; mh="$(sha256sum "$package/$manager" | cut -d' ' -f1)"; else rh="$(shasum -a 256 "$package/$router" | cut -d' ' -f1)"; mh="$(shasum -a 256 "$package/$manager" | cut -d' ' -f1)"; fi; printf '%s  %s\n%s  %s\n' "$rh" "$router" "$mh" "$manager" >"$package/SHA256SUMS"
config="$tmp/model config.json"; printf '%s\n' '{"version":1,"claude":{"primary":{"model":"模型/PS [1]"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}' >"$config"
output="$(env PATH="$bin:$PATH" HOME="$home" USERPROFILE="$home" PROCESSOR_ARCHITECTURE=AMD64 FAKE_LOG="$tmp/manager.log" python3 - "$package/setup.ps1" "$config" <<'PY'
import os, pty, select, sys, termios, time
script, config = sys.argv[1:]
pid, fd = pty.fork()
if pid == 0: os.execvp("pwsh", ["pwsh", "-NoProfile", "-File", script, "agent", "print-config", "--agent=claude", "--model-config=" + config])
out = bytearray(); sent = False
while True:
    ready, _, _ = select.select([fd], [], [], 15)
    if not ready: raise SystemExit("PowerShell flow timed out")
    try: chunk = os.read(fd, 4096)
    except OSError: break
    if not chunk: break
    out.extend(chunk)
    if not sent and "输入隐藏".encode() in out:
        deadline = time.monotonic() + 1
        while termios.tcgetattr(fd)[3] & termios.ECHO:
            if time.monotonic() >= deadline: raise SystemExit("PowerShell secret input echo was not disabled")
            time.sleep(0.01)
        os.write(fd, b"ps-v2-key-canary\n"); sent = True
_, status = os.waitpid(pid, 0)
if os.waitstatus_to_exitcode(status): sys.stderr.buffer.write(out); raise SystemExit("PowerShell flow failed")
if b"ps-v2-key-canary" in out: raise SystemExit("PowerShell key leaked")
sys.stdout.buffer.write(out)
PY
)"
[[ "$output" == *'PowerShell dynamic redacted fragment'* ]] || fail 'PowerShell manager render output missing'; grep -Fq 'models-ok' "$tmp/manager.log" && grep -Fq 'render-ok' "$tmp/manager.log" || fail 'PowerShell v2 requests were not validated'; ! grep -Fq 'ps-v2-key-canary' "$tmp/manager.log" || fail 'PowerShell manager log leaked key'
[[ "$(xxd -p -l 3 "$package/setup.ps1")" == efbbbf ]] || fail 'PowerShell BOM was not preserved'
printf 'PASS: executable PowerShell v2 Agent model flow\n'
