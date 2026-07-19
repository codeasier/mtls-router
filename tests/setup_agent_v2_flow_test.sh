#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
platform="$(case "$(uname -s)" in Linux) printf linux;; Darwin) printf darwin;; *) fail unsupported;; esac)-$(case "$(uname -m)" in x86_64|amd64) printf amd64;; arm64|aarch64) printf arm64;; *) fail unsupported;; esac)"
target="${platform/-//}"; tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
package="$tmp/package"; home="$tmp/home"; bin="$tmp/bin"; mkdir -p "$package" "$home" "$bin" "$home/codex"
cp "$ROOT/setup.sh" "$package/setup.sh"
router="mtls-router-$platform"; manager="mtls-router-manager-$platform"; printf '#!/usr/bin/env bash\nexit 0\n' >"$package/$router"
cat >"$package/$manager" <<'MANAGER'
#!/usr/bin/env bash
set -euo pipefail
request="$(dd bs=4194304 count=1 2>/dev/null)"; method="$(printf '%s' "$request" | jq -r .method)"
printf 'method %s\n' "$method" >>"$FAKE_LOG"
case "$method" in
  manager.info) jq -cn --arg target "$FAKE_TARGET" '{id:"setup-secret-info",result:{version:"v2-test",commit:"test",build_date:"test",target:$target,deployment_id:"fake-v2",management_protocol_version:"2"}}' ;;
  agent.detect) jq -cn '{id:"detect",result:{agents:[{agent:"claude",name:"Claude Code",detected:true,path:"/tmp/claude",format:"json"},{agent:"opencode",name:"opencode",detected:true,path:"/tmp/opencode",format:"json"},{agent:"codex",name:"Codex",detected:true,path:"/tmp/codex",auth_path:"/tmp/auth",format:"toml"}]}}' ;;
  agent.models) printf '%s' "$request" | jq -e '.params.owner=="cli" and .params.api_key=="v2-key-canary" and (.params|keys)==["agents","api_key","owner"]' >/dev/null; printf 'models key_ok\n' >>"$FAKE_LOG"; case "${FAKE_SCENARIO:-success}" in auth) jq -cn '{id:"models",error:{code:"MODEL_AUTH_FAILED",message:"authentication failed"}}';; empty) jq -cn '{id:"models",error:{code:"MODEL_CATALOG_EMPTY",message:"catalog empty"}}';; preset|override) jq -cn '{id:"models",result:{models:["preset-base","override-base"],catalog_token:"catalog-special",router_base_url:"http://127.0.0.1:19099",api_base_url:"http://127.0.0.1:19099/v1",existing:{model_config:{},unavailable_models:{},drifted_agents:[]},preset:{model_config:{version:1,claude:{primary:{model:"preset-base",name:"预设名称",context:"1m"},haiku:{inherit_primary:true},sonnet:{inherit_primary:true},opus:{inherit_primary:true}}},unavailable_agents:{}}}}';; existing) jq -cn '{id:"models",result:{models:["existing-base","preset-base"],catalog_token:"catalog-special",router_base_url:"http://127.0.0.1:19099",api_base_url:"http://127.0.0.1:19099/v1",existing:{model_config:{version:1,claude:{primary:{model:"existing-base",name:"现有名称"},haiku:{inherit_primary:true},sonnet:{inherit_primary:true},opus:{inherit_primary:true}}},unavailable_models:{},drifted_agents:[]},preset:{model_config:{version:1,claude:{primary:{model:"preset-base",context:"1m"},haiku:{inherit_primary:true},sonnet:{inherit_primary:true},opus:{inherit_primary:true}}},unavailable_agents:{}}}}';; mixed) jq -cn '{id:"models",result:{models:["existing-base","open-preset","manual-codex"],catalog_token:"catalog-special",router_base_url:"http://127.0.0.1:19099",api_base_url:"http://127.0.0.1:19099/v1",existing:{model_config:{version:1,claude:{primary:{model:"existing-base"},haiku:{inherit_primary:true},sonnet:{inherit_primary:true},opus:{inherit_primary:true}}},unavailable_models:{},drifted_agents:[]},preset:{model_config:{version:1,claude:{primary:{model:"must-not-merge",name:"preset-only-name"},haiku:{inherit_primary:true},sonnet:{inherit_primary:true},opus:{inherit_primary:true}},opencode:{default_model:"open-preset",models:{"open-preset":{name:"Open 推荐"}}}},unavailable_agents:{codex:{code:"MODEL_NOT_AVAILABLE",models:["missing-codex"]}}}}}';; unavailable) jq -cn '{id:"models",result:{models:["manual-base"],catalog_token:"catalog-special",router_base_url:"http://127.0.0.1:19099",api_base_url:"http://127.0.0.1:19099/v1",existing:{model_config:{},unavailable_models:{},drifted_agents:[]},preset:{model_config:{},unavailable_agents:{claude:{code:"MODEL_NOT_AVAILABLE",models:["missing-a","missing-z"]}}}}}';; *) printf '%s' "$request" | jq -e '.params.agents==["claude","opencode","codex"]' >/dev/null; jq -cn '{id:"models",result:{models:["模型/α [x]","model:two"],catalog_token:"catalog-special",router_base_url:"http://127.0.0.1:19099",api_base_url:"http://127.0.0.1:19099/v1",existing:{model_config:{},unavailable_models:{},drifted_agents:[]},preset:{model_config:{},unavailable_agents:{}}}}';; esac ;;
  agent.render) printf '%s' "$request" | jq -c '.params.model_config' >>"$FAKE_CONFIG_LOG"; case "${FAKE_SCENARIO:-success}" in success) printf '%s' "$request" | jq -e '.params.catalog_token=="catalog-special" and .params.model_config.claude.primary.model=="模型/α [x]"' >/dev/null;; preset) printf '%s' "$request" | jq -e '.params.model_config.claude.primary=={model:"preset-base",name:"预设名称",context:"1m"}' >/dev/null;; existing) printf '%s' "$request" | jq -e '.params.model_config.claude.primary=={model:"existing-base",name:"现有名称"} and (.params.model_config.claude.primary|has("context")|not)' >/dev/null;; unavailable) printf '%s' "$request" | jq -e '.params.model_config.claude.primary=={model:"manual-base",name:"手动名称",context:"1m"}' >/dev/null;; override) ;; esac; printf 'render shape_ok\n' >>"$FAKE_LOG"; jq -cn '{id:"render",result:{model_config:{version:1},fragments:[{agent:"claude",role:"config",path:"/tmp/special path.json",format:"json",content:"redacted dynamic fragment"}]}}' ;;
  agent.preview) printf '%s' "$request" | jq -e '.params.catalog_token=="catalog-special" and .params.model_config.codex.model=="model:two"' >/dev/null; printf 'preview shape_ok\n' >>"$FAKE_LOG"; if [[ "${FAKE_SCENARIO:-}" == stale ]]; then jq -cn '{id:"preview",error:{code:"PREVIEW_STALE",message:"preview stale"}}'; else jq -cn '{id:"preview",result:{revision_token:"revision-special",model_config:{version:1},fragments:[{agent:"codex",role:"auth",path:"/tmp/auth",format:"json",content:"redacted auth"}],files:[{path:"/tmp/codex",role:"config",format:"toml",operation:"replace",backup_path:"/tmp/codex.bak"}],managed_config_drift:true,drifted_agents:["codex"],managed_collisions:[{agent:"codex",path:"/model",type:"managed",action:"replace"}],requires_codex_auth_approval:true,state_change:{path:"/tmp/state",role:"state",format:"json",operation:"create"},state_backup:{path:"/tmp/state.bak",role:"state",format:"json",operation:"create"}}}'; fi ;;
  agent.write) printf '%s' "$request" | jq -e '.params.catalog_token=="catalog-special" and .params.revision_token=="revision-special" and .params.approve_managed_overwrite==true and .params.approve_codex_auth_change==true and .params.api_key=="v2-key-canary" and (.params|keys)==["agents","api_key","approve_codex_auth_change","approve_managed_overwrite","catalog_token","model_config","revision_token"] and .params.model_config.opencode.models["模型/α [x]"].reasoning==true and .params.model_config.opencode.models["模型/α [x]"].limit=={context:10,output:5} and .params.model_config.codex.context_window==10' >/dev/null; printf 'write key_ok approvals_ok typed_ok\n' >>"$FAKE_LOG"; if [[ "${FAKE_SCENARIO:-}" == removed ]]; then jq -cn '{id:"write",error:{code:"MODEL_NOT_AVAILABLE",message:"selected model unavailable"}}'; else jq -cn '{id:"write",result:{transaction_id:"tx",agents:[{agent:"codex",success:true,changed:["/tmp/codex"],backups:["/tmp/codex.bak"]}]}}'; fi ;;
  *) exit 9 ;;
esac
MANAGER
chmod +x "$package/setup.sh" "$package/$router" "$package/$manager"
for command in claude opencode codex; do printf '#!/usr/bin/env bash\nexit 0\n' >"$bin/$command"; chmod +x "$bin/$command"; done
printf '%s  %s\n%s  %s\n' "$(sha256 "$package/$router")" "$router" "$(sha256 "$package/$manager")" "$manager" >"$package/SHA256SUMS"
config="$tmp/model config.json"; printf '%s\n' '{"version":1,"claude":{"primary":{"model":"模型/α [x]"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}},"opencode":{"default_model":"模型/α [x]","models":{"模型/α [x]":{"reasoning":true,"limit":{"context":10,"output":5},"modalities":{"input":["text","image"],"output":["text"]},"interleaved":{"field":"reasoning_details"},"options":{"reasoningEffort":"high"},"extra":{"status":"active"}}}},"codex":{"model":"model:two","reasoning_effort":"high","reasoning_summary":"auto","verbosity":"medium","context_window":10,"auto_compact_token_limit":5,"extra":{"model_auto_compact_token_limit_scope":"body_after_prefix"}}}' >"$config"
common=(env PATH="$bin:$PATH" HOME="$home" CODEX_HOME="$home/codex" CLAUDE_CONFIG_DIR="$home/claude" OPENCODE_CONFIG="$home/opencode.json" FAKE_TARGET="$target" FAKE_LOG="$tmp/manager.log" FAKE_CONFIG_LOG="$tmp/config.log")
library="$tmp/setup-library.sh"; awk '!/^main "\$@"$/' "$package/setup.sh" >"$library"
mixed_models='{"result":{"existing":{"model_config":{"version":1,"claude":{"primary":{"model":"existing-base"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}},"preset":{"model_config":{"version":1,"claude":{"primary":{"model":"must-not-merge","name":"preset-only-name"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}},"opencode":{"default_model":"open-preset","models":{"open-preset":{"name":"Open 推荐"}}}},"unavailable_agents":{"codex":{"code":"MODEL_NOT_AVAILABLE","models":["missing-codex"]}}}}}'
mixed_summary="$(bash -c 'source "$1"; TARGETS=(claude opencode codex); initialize_model_defaults "$2" '\''["claude","opencode","codex"]'\''; printf "\n%s" "$INITIAL_MODEL_CONFIG"' _ "$library" "$mixed_models")"
printf '%s' "${mixed_summary##*$'\n'}" | jq -e '.claude.primary=={model:"existing-base"} and (.claude.primary|has("name")|not) and .opencode.default_model=="open-preset" and (.codex|not)' >/dev/null || fail 'per-Agent existing/preset/empty precedence deep-merged or substituted a section'
[[ "$mixed_summary" == *'CONFIG SOURCE claude: existing'* && "$mixed_summary" == *'CONFIG SOURCE opencode: preset'* && "$mixed_summary" == *'CONFIG SOURCE codex: empty'* && "$mixed_summary" == *'PRESET UNAVAILABLE codex: missing-codex'* ]] || fail 'mixed source summary missing'
run_pty() {
  local mode="$1" scenario="${2:-success}" command_style="${3:-subcommand}" config_arg="${4:-config}" expected="${5:-success}"
  "${common[@]}" FAKE_SCENARIO="$scenario" python3 - "$package/setup.sh" "$config" "$mode" "$command_style" "$config_arg" "$expected" <<'PY'
import os, pty, select, sys, termios, time
script, config, mode, command_style, config_arg, expected = sys.argv[1:]
args = ["bash", script]
args += (["agent", mode + "-config"] if command_style == "subcommand" else ["--" + mode + "-config"])
args += ["--agent=claude,opencode,codex"]
if config_arg == "config": args += ["--model-config=" + config]
elif config_arg != "wizard": args += ["--model-config=" + config_arg]
pid, fd = pty.fork()
if pid == 0: os.execvp("bash", args)
out = bytearray(); sent = set(); replies = [("输入隐藏", b"v2-key-canary\n")]
if mode == "write": replies += [("OVERWRITE", b"OVERWRITE\n"), ("CODEX-AUTH", b"CODEX-AUTH\n"), ("输入 WRITE", b"WRITE\n")]
if config_arg == "wizard": replies += [("Claude primary model ID", b"\n")]
while True:
    ready, _, _ = select.select([fd], [], [], 15)
    if not ready: raise SystemExit("flow timed out")
    try: chunk = os.read(fd, 4096)
    except OSError: break
    if not chunk: break
    out.extend(chunk)
    for marker, reply in replies:
        if marker not in sent and marker.encode() in out:
            if "隐藏" in marker:
                deadline = time.monotonic() + 1
                while termios.tcgetattr(fd)[3] & termios.ECHO:
                    if time.monotonic() >= deadline: raise SystemExit("secret input echo was not disabled")
                    time.sleep(0.01)
            os.write(fd, reply); sent.add(marker)
_, status = os.waitpid(pid, 0)
failed = os.waitstatus_to_exitcode(status) != 0
if failed != (expected == "failure"):
    sys.stderr.buffer.write(out); raise SystemExit("flow failed")
if b"v2-key-canary" in out: raise SystemExit("key canary leaked")
if expected == "success" and len(sent) != len(replies): raise SystemExit("missing expected prompt")
sys.stdout.buffer.write(out)
PY
}
run_claude_wizard() {
  local scenario="$1" responses="$2" expected="${3:-success}"
  "${common[@]}" FAKE_SCENARIO="$scenario" python3 - "$package/setup.sh" "$responses" "$expected" <<'PY'
import os, pty, select, sys, termios, time
script, responses, expected = sys.argv[1:]
pid, fd = pty.fork()
if pid == 0: os.execvp("bash", ["bash", script, "agent", "print-config", "--agent=claude"])
out = bytearray(); key_sent = False; answers_sent = False
while True:
    ready, _, _ = select.select([fd], [], [], 15)
    if not ready: raise SystemExit("wizard timed out")
    try: chunk = os.read(fd, 4096)
    except OSError: break
    if not chunk: break
    out.extend(chunk)
    if not key_sent and "输入隐藏".encode() in out:
        deadline = time.monotonic() + 1
        while termios.tcgetattr(fd)[3] & termios.ECHO:
            if time.monotonic() >= deadline: raise SystemExit("secret input echo was not disabled")
            time.sleep(0.01)
        os.write(fd, b"v2-key-canary\n"); key_sent = True
    if key_sent and not answers_sent and b"Claude primary model ID" in out:
        os.write(fd, responses.encode()); answers_sent = True
_, status = os.waitpid(pid, 0)
failed = os.waitstatus_to_exitcode(status) != 0
if failed != (expected == "failure"):
    sys.stderr.buffer.write(out); raise SystemExit("wizard result mismatch")
if b"v2-key-canary" in out: raise SystemExit("key canary leaked")
sys.stdout.buffer.write(out)
PY
}
print_out="$(run_pty print)"; [[ "$print_out" == *'redacted dynamic fragment'* ]] || fail 'render output missing'; write_out="$(run_pty write)"; [[ "$write_out" == *'COLLISION codex /model'* && "$write_out" == *'BACKUP: /tmp/codex.bak'* ]] || fail 'exact preview effects missing'
alias_out="$(run_pty print success alias)"; [[ "$alias_out" == *'redacted dynamic fragment'* ]] || fail 'compatibility alias diverged'
auth_out="$(run_pty print auth subcommand config failure)"; [[ "$auth_out" == *MODEL_AUTH_FAILED* ]] || fail 'auth failure missing'
empty_out="$(run_pty print empty subcommand config failure)"; [[ "$empty_out" == *MODEL_CATALOG_EMPTY* ]] || fail 'empty catalog failure missing'
stale_out="$(run_pty write stale subcommand config failure)"; [[ "$stale_out" == *PREVIEW_STALE* ]] || fail 'stale preview failure missing'
removed_out="$(run_pty write removed subcommand config failure)"; [[ "$removed_out" == *MODEL_NOT_AVAILABLE* ]] || fail 'removed model failure missing'
wizard_out="$(run_pty print success subcommand wizard failure)"; [[ "$wizard_out" == *'model ID 不能为空'* ]] || fail 'wizard automatically selected a model'
preset_out="$(run_claude_wizard preset $'\n\n\n\n\n\n\n\n\n\n')"; [[ "$preset_out" == *'CONFIG SOURCE claude: preset'* ]] || fail 'preset source summary missing'
existing_out="$(run_claude_wizard existing $'\n\n\n\n\n\n\n\n\n\n')"; [[ "$existing_out" == *'CONFIG SOURCE claude: existing'* ]] || fail 'existing did not win over preset'
unavailable_out="$(run_claude_wizard unavailable $'manual-base\n手动名称\n1m\ninherit\ninherit\ninherit\n\n\n\n\n')"; [[ "$unavailable_out" == *'CONFIG SOURCE claude: empty'* && "$unavailable_out" == *'PRESET UNAVAILABLE claude: missing-a, missing-z'* ]] || fail 'unavailable preset summary missing'
override_out="$(run_claude_wizard override $'override-base\n覆盖名称\nstandard\ninherit\ninherit\ninherit\n\n\n\n\n')"; printf '%s' "$(tail -n 1 "$tmp/config.log")" | jq -e '.claude.primary=={model:"override-base",name:"覆盖名称"} and (.claude.primary|has("context")|not)' >/dev/null || fail 'editable preset override or Unicode name failed'
explicit_roles_out="$(run_claude_wizard override $'primary-base\nPrimary display\nstandard\nhaiku-base\nHaiku display\nstandard\nsonnet-base\nSonnet display\n1m\nopus-base\nOpus display\n1m\n\n\n\n\n')"
for prompt in 'Claude haiku name' 'Claude haiku context（standard/1m）' 'Claude sonnet name' 'Claude sonnet context（standard/1m）' 'Claude opus name' 'Claude opus context（standard/1m）'; do
  [[ "$explicit_roles_out" == *"$prompt"* ]] || fail "explicit role prompt missing: $prompt"
done
printf '%s' "$(tail -n 1 "$tmp/config.log")" | jq -e '.claude=={primary:{model:"primary-base",name:"Primary display"},haiku:{model:"haiku-base",name:"Haiku display"},sonnet:{model:"sonnet-base",name:"Sonnet display",context:"1m"},opus:{model:"opus-base",name:"Opus display",context:"1m"}}' >/dev/null || fail 'explicit Claude role names or contexts changed in canonical Shell output'
renders_before="$(grep -c '^method agent.render$' "$tmp/manager.log")"; invalid_context_out="$(run_claude_wizard preset $'\n\nunsupported\n' failure)"; [[ "$invalid_context_out" == *'context 只接受 standard 或 1m'* ]] || fail 'unsupported context error missing'; [[ "$(grep -c '^method agent.render$' "$tmp/manager.log")" == "$renders_before" ]] || fail 'unsupported context reached manager render'
grep -Fq 'render shape_ok' "$tmp/manager.log" && grep -Fq 'write key_ok approvals_ok' "$tmp/manager.log" || fail 'v2 request checks missing'; ! grep -Fq 'v2-key-canary' "$tmp/manager.log" || fail 'fake manager log leaked key'
[[ "$(grep -c '^method agent.preview$' "$tmp/manager.log")" == 3 && "$(grep -c '^method agent.write$' "$tmp/manager.log")" == 2 ]] || fail 'print/no-state or terminal flow method matrix changed'
link="$tmp/config-link"; ln -s "$config" "$link"; if "${common[@]}" bash "$package/setup.sh" agent print-config --agent=claude --model-config="$link" </dev/null >/dev/null 2>&1; then fail 'noninteractive/symlink config accepted'; fi
invalid="$tmp/invalid.json"; printf '[]\n' >"$invalid"; models_before="$(grep -c '^method agent.models$' "$tmp/manager.log")"; if invalid_out="$(run_pty print success subcommand "$invalid" failure)"; then :; fi; [[ "$invalid_out" != *'输入隐藏'* ]] || fail 'invalid model config prompted for key'; [[ "$(grep -c '^method agent.models$' "$tmp/manager.log")" == "$models_before" ]] || fail 'invalid model config reached model discovery'
printf 'PASS: executable Shell v2 Agent model flow\n'
