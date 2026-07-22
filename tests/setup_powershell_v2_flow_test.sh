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
  manager.info) jq -cn '{id:"setup-secret-info",result:{version:"v2",commit:"test",build_date:"test",target:"windows/amd64",deployment_id:"fake",management_protocol_version:"3"}}' ;;
  agent.detect) jq -cn '{id:"detect",result:{agents:[{agent:"claude",name:"Claude Code",detected:true,path:"C:\\special path\\settings.json",format:"json"}]}}' ;;
  agent.models)
    printf '%s' "$request" | jq -e '.params.owner=="cli" and .params.api_key=="ps-v2-key-canary"' >/dev/null
    printf 'models-ok\n' >>"$FAKE_LOG"
    case "${FAKE_SCENARIO:-preset}" in
      existing) jq -cn '{id:"models",result:{models:["ps-existing","ps-preset"],catalog_token:"ps-catalog",router_base_url:"http://127.0.0.1:19099",api_base_url:"http://127.0.0.1:19099/v1",existing:{model_config:{version:1,claude:{primary:{model:"ps-existing",name:"现有名称"},haiku:{inherit_primary:true},sonnet:{inherit_primary:true},opus:{inherit_primary:true}}},unavailable_models:{},drifted_agents:[]},preset:{model_config:{version:1,claude:{primary:{model:"ps-preset",context:"1m"},haiku:{inherit_primary:true},sonnet:{inherit_primary:true},opus:{inherit_primary:true}}},unavailable_agents:{}}}}' ;;
      unavailable) jq -cn '{id:"models",result:{models:["ps-manual"],catalog_token:"ps-catalog",router_base_url:"http://127.0.0.1:19099",api_base_url:"http://127.0.0.1:19099/v1",existing:{model_config:{},unavailable_models:{},drifted_agents:[]},preset:{model_config:{},unavailable_agents:{claude:{code:"MODEL_NOT_AVAILABLE",models:["missing-ps"]}}}}}' ;;
      simplify-disabled) jq -cn '{id:"models",result:{models:["provider/ps-slash-model"],catalog_token:"ps-catalog",router_base_url:"http://127.0.0.1:19099",api_base_url:"http://127.0.0.1:19099/v1",existing:{model_config:{},unavailable_models:{},drifted_agents:[]},preset:{model_config:{},unavailable_agents:{}}}}' ;;
      *) jq -cn '{id:"models",result:{models:["模型／PS [1]","ps-preset","ps-override"],catalog_token:"ps-catalog",router_base_url:"http://127.0.0.1:19099",api_base_url:"http://127.0.0.1:19099/v1",existing:{model_config:{},unavailable_models:{},drifted_agents:[]},preset:{model_config:{version:1,claude:{primary:{model:"ps-preset",name:"预设名称",context:"1m"},haiku:{inherit_primary:true},sonnet:{inherit_primary:true},opus:{inherit_primary:true}}},unavailable_agents:{}}}}' ;;
    esac
    ;;
  agent.render)
    printf '%s' "$request" | jq -e '.params.catalog_token=="ps-catalog"' >/dev/null
    printf '%s' "$request" | jq -c '.params.model_config' >>"$FAKE_CONFIG_LOG"
    if [[ "${FAKE_SCENARIO:-}" == simplify-disabled ]]; then
      printf '%s' "$request" | jq -e '.params.model_config.claude.primary.model=="provider/ps-slash-model"' >/dev/null
    fi
    printf 'render-ok\n' >>"$FAKE_LOG"
    jq -cn '{id:"render",result:{model_config:{version:1},fragments:[{agent:"claude",role:"config",path:"C:\\special path\\settings.json",format:"json",content:"PowerShell dynamic redacted fragment"}]}}'
    ;;
  *) exit 8 ;;
esac
MANAGER
chmod +x "$package/$router" "$package/$manager"; for command in claude; do printf '#!/usr/bin/env bash\nexit 0\n' >"$bin/$command"; chmod +x "$bin/$command"; done
if command -v sha256sum >/dev/null 2>&1; then rh="$(sha256sum "$package/$router" | cut -d' ' -f1)"; mh="$(sha256sum "$package/$manager" | cut -d' ' -f1)"; else rh="$(shasum -a 256 "$package/$router" | cut -d' ' -f1)"; mh="$(shasum -a 256 "$package/$manager" | cut -d' ' -f1)"; fi; printf '%s  %s\n%s  %s\n' "$rh" "$router" "$mh" "$manager" >"$package/SHA256SUMS"
config="$tmp/model config.json"; printf '%s\n' '{"version":1,"claude":{"primary":{"model":"模型／PS [1]"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}' >"$config"
slash_config="$tmp/slash model config.json"; printf '%s\n' '{"version":1,"claude":{"primary":{"model":"provider/ps-slash-model"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}' >"$slash_config"
run_ps_import() {
  local scenario="$1" config_path="$2"
  env PATH="$bin:$PATH" HOME="$home" USERPROFILE="$home" PROCESSOR_ARCHITECTURE=AMD64 FAKE_SCENARIO="$scenario" FAKE_LOG="$tmp/manager.log" FAKE_CONFIG_LOG="$tmp/config.log" python3 - "$package/setup.ps1" "$config_path" <<'PY'
import os, pty, select, sys, termios, time
script, config = sys.argv[1:]
pid, fd = pty.fork()
if pid == 0: os.execvp("pwsh", ["pwsh", "-NoProfile", "-File", script, "agent", "print-config", "--agent=claude", "--model-config=" + config])
out = bytearray(); sent = False; cursor_queries = 0
while True:
    ready, _, _ = select.select([fd], [], [], 15)
    if not ready: raise SystemExit("PowerShell flow timed out")
    try: chunk = os.read(fd, 4096)
    except OSError: break
    if not chunk: break
    out.extend(chunk)
    pending_queries = out.count(b"\x1b[6n") - cursor_queries
    if pending_queries:
        os.write(fd, b"\x1b[1;1R" * pending_queries); cursor_queries += pending_queries
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
}
output="$(run_ps_import preset "$config")"
[[ "$output" == *'PowerShell dynamic redacted fragment'* ]] || fail 'PowerShell manager render output missing'; grep -Fq 'models-ok' "$tmp/manager.log" && grep -Fq 'render-ok' "$tmp/manager.log" || fail 'PowerShell v2 requests were not validated'; ! grep -Fq 'ps-v2-key-canary' "$tmp/manager.log" || fail 'PowerShell manager log leaked key'
simplify_disabled_output="$(run_ps_import simplify-disabled "$slash_config")"
simplify_disabled_lines=$'\n'"${simplify_disabled_output//$'\r'/}"$'\n'
[[ "$simplify_disabled_lines" == *$'\nMODEL provider/ps-slash-model\n'* ]] || fail 'PowerShell simplify-disabled catalog output was filtered'
[[ "$simplify_disabled_output" == *'PowerShell dynamic redacted fragment'* ]] || fail 'PowerShell simplify-disabled slash import did not render'
run_ps_wizard() {
  local scenario="$1" responses="$2" expected="${3:-success}"
  env PATH="$bin:$PATH" HOME="$home" USERPROFILE="$home" PROCESSOR_ARCHITECTURE=AMD64 FAKE_SCENARIO="$scenario" FAKE_LOG="$tmp/manager.log" FAKE_CONFIG_LOG="$tmp/config.log" python3 - "$package/setup.ps1" "$responses" "$expected" <<'PY'
import os, pty, select, sys, termios, time
script, responses, expected = sys.argv[1:]
pid, fd = pty.fork()
if pid == 0: os.execvp("pwsh", ["pwsh", "-NoProfile", "-File", script, "agent", "print-config", "--agent=claude"])
out = bytearray(); key_sent = answers_sent = False; cursor_queries = 0
while True:
    ready, _, _ = select.select([fd], [], [], 15)
    if not ready: raise SystemExit("PowerShell preset wizard timed out")
    try: chunk = os.read(fd, 4096)
    except OSError: break
    if not chunk: break
    out.extend(chunk)
    pending_queries = out.count(b"\x1b[6n") - cursor_queries
    if pending_queries:
        os.write(fd, b"\x1b[1;1R" * pending_queries); cursor_queries += pending_queries
    if not key_sent and "输入隐藏".encode() in out:
        deadline = time.monotonic() + 1
        while termios.tcgetattr(fd)[3] & termios.ECHO:
            if time.monotonic() >= deadline: raise SystemExit("PowerShell secret input echo was not disabled")
            time.sleep(0.01)
        os.write(fd, b"ps-v2-key-canary\n"); key_sent = True
    if key_sent and not answers_sent and b"Claude primary model ID" in out:
        os.write(fd, responses.encode()); answers_sent = True
_, status = os.waitpid(pid, 0)
failed = os.waitstatus_to_exitcode(status) != 0
if failed != (expected == "failure"):
    sys.stderr.buffer.write(out); raise SystemExit("PowerShell wizard result mismatch")
if b"ps-v2-key-canary" in out: raise SystemExit("PowerShell key leaked")
sys.stdout.buffer.write(out)
PY
}
wizard_output="$(run_ps_wizard preset $'\n\n\n\n\n\n\n\n\n\n')"
[[ "$wizard_output" == *'CONFIG SOURCE claude: preset'* ]] || fail 'PowerShell preset source summary missing'
printf '%s' "$(tail -n 1 "$tmp/config.log")" | jq -e '.claude.primary=={model:"ps-preset",name:"预设名称",context:"1m"}' >/dev/null || fail 'PowerShell preset defaults or Unicode/context failed'
existing_output="$(run_ps_wizard existing $'\n\n\n\n\n\n\n\n\n\n')"; [[ "$existing_output" == *'CONFIG SOURCE claude: existing'* ]] || fail 'PowerShell existing section did not win over preset'; printf '%s' "$(tail -n 1 "$tmp/config.log")" | jq -e '.claude.primary=={model:"ps-existing",name:"现有名称"} and (.claude.primary|has("context")|not)' >/dev/null || fail 'PowerShell existing section was deep-merged with preset'
override_output="$(run_ps_wizard override $'ps-override\n覆盖名称\nstandard\ninherit\ninherit\ninherit\n\n\n\n\n')"; printf '%s' "$(tail -n 1 "$tmp/config.log")" | jq -e '.claude.primary=={model:"ps-override",name:"覆盖名称"} and (.claude.primary|has("context")|not)' >/dev/null || fail 'PowerShell editable override or Unicode name failed'
unavailable_output="$(run_ps_wizard unavailable $'ps-manual\n\n1m\ninherit\ninherit\ninherit\n\n\n\n\n')"; [[ "$unavailable_output" == *'CONFIG SOURCE claude: empty'* && "$unavailable_output" == *'PRESET UNAVAILABLE claude: missing-ps'* ]] || fail 'PowerShell unavailable preset summary missing'
renders_before="$(grep -c '^render-ok$' "$tmp/manager.log")"; invalid_output="$(run_ps_wizard preset $'\n\nunsupported\n' failure)"; [[ "$invalid_output" == *'context 只接受 standard 或 1m'* ]] || fail 'PowerShell unsupported context error missing'; [[ "$(grep -c '^render-ok$' "$tmp/manager.log")" == "$renders_before" ]] || fail 'PowerShell unsupported context reached manager render'
[[ "$(xxd -p -l 3 "$package/setup.ps1")" == efbbbf ]] || fail 'PowerShell BOM was not preserved'
printf 'PASS: executable PowerShell v2 Agent model flow\n'
