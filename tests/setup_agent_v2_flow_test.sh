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
  agent.models) printf '%s' "$request" | jq -e '.params.owner=="cli" and .params.agents==["claude","opencode","codex"] and .params.api_key=="v2-key-canary" and (.params|keys)==["agents","api_key","owner"]' >/dev/null; printf 'models key_ok\n' >>"$FAKE_LOG"; case "${FAKE_SCENARIO:-success}" in auth) jq -cn '{id:"models",error:{code:"MODEL_AUTH_FAILED",message:"authentication failed"}}';; empty) jq -cn '{id:"models",error:{code:"MODEL_CATALOG_EMPTY",message:"catalog empty"}}';; *) jq -cn '{id:"models",result:{models:["模型/α [x]","model:two"],catalog_token:"catalog-special",router_base_url:"http://127.0.0.1:19099",api_base_url:"http://127.0.0.1:19099/v1",existing:{model_config:{},unavailable_models:{},drifted_agents:[]}}}';; esac ;;
  agent.render) printf '%s' "$request" | jq -e '.params.catalog_token=="catalog-special" and .params.model_config.claude.primary.model=="模型/α [x]"' >/dev/null; printf 'render shape_ok\n' >>"$FAKE_LOG"; jq -cn '{id:"render",result:{model_config:{version:1},fragments:[{agent:"claude",role:"config",path:"/tmp/special path.json",format:"json",content:"redacted dynamic fragment"}]}}' ;;
  agent.preview) printf '%s' "$request" | jq -e '.params.catalog_token=="catalog-special" and .params.model_config.codex.model=="model:two"' >/dev/null; printf 'preview shape_ok\n' >>"$FAKE_LOG"; if [[ "${FAKE_SCENARIO:-}" == stale ]]; then jq -cn '{id:"preview",error:{code:"PREVIEW_STALE",message:"preview stale"}}'; else jq -cn '{id:"preview",result:{revision_token:"revision-special",model_config:{version:1},fragments:[{agent:"codex",role:"auth",path:"/tmp/auth",format:"json",content:"redacted auth"}],files:[{path:"/tmp/codex",role:"config",format:"toml",operation:"replace",backup_path:"/tmp/codex.bak"}],managed_config_drift:true,drifted_agents:["codex"],managed_collisions:[{agent:"codex",path:"/model",type:"managed",action:"replace"}],requires_codex_auth_approval:true,state_change:{path:"/tmp/state",role:"state",format:"json",operation:"create"},state_backup:{path:"/tmp/state.bak",role:"state",format:"json",operation:"create"}}}'; fi ;;
  agent.write) printf '%s' "$request" | jq -e '.params.catalog_token=="catalog-special" and .params.revision_token=="revision-special" and .params.approve_managed_overwrite==true and .params.approve_codex_auth_change==true and .params.api_key=="v2-key-canary" and (.params|keys)==["agents","api_key","approve_codex_auth_change","approve_managed_overwrite","catalog_token","model_config","revision_token"] and .params.model_config.opencode.models["模型/α [x]"].reasoning==true and .params.model_config.opencode.models["模型/α [x]"].limit=={context:10,output:5} and .params.model_config.codex.context_window==10' >/dev/null; printf 'write key_ok approvals_ok typed_ok\n' >>"$FAKE_LOG"; if [[ "${FAKE_SCENARIO:-}" == removed ]]; then jq -cn '{id:"write",error:{code:"MODEL_NOT_AVAILABLE",message:"selected model unavailable"}}'; else jq -cn '{id:"write",result:{transaction_id:"tx",agents:[{agent:"codex",success:true,changed:["/tmp/codex"],backups:["/tmp/codex.bak"]}]}}'; fi ;;
  *) exit 9 ;;
esac
MANAGER
chmod +x "$package/setup.sh" "$package/$router" "$package/$manager"
for command in claude opencode codex; do printf '#!/usr/bin/env bash\nexit 0\n' >"$bin/$command"; chmod +x "$bin/$command"; done
printf '%s  %s\n%s  %s\n' "$(sha256 "$package/$router")" "$router" "$(sha256 "$package/$manager")" "$manager" >"$package/SHA256SUMS"
config="$tmp/model config.json"; printf '%s\n' '{"version":1,"claude":{"primary":{"model":"模型/α [x]"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}},"opencode":{"default_model":"模型/α [x]","models":{"模型/α [x]":{"reasoning":true,"limit":{"context":10,"output":5},"modalities":{"input":["text","image"],"output":["text"]},"interleaved":{"field":"reasoning_details"},"options":{"reasoningEffort":"high"},"extra":{"status":"active"}}}},"codex":{"model":"model:two","reasoning_effort":"high","reasoning_summary":"auto","verbosity":"medium","context_window":10,"auto_compact_token_limit":5,"extra":{"model_auto_compact_token_limit_scope":"body_after_prefix"}}}' >"$config"
common=(env PATH="$bin:$PATH" HOME="$home" CODEX_HOME="$home/codex" CLAUDE_CONFIG_DIR="$home/claude" OPENCODE_CONFIG="$home/opencode.json" FAKE_TARGET="$target" FAKE_LOG="$tmp/manager.log")
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
print_out="$(run_pty print)"; [[ "$print_out" == *'redacted dynamic fragment'* ]] || fail 'render output missing'; write_out="$(run_pty write)"; [[ "$write_out" == *'COLLISION codex /model'* && "$write_out" == *'BACKUP: /tmp/codex.bak'* ]] || fail 'exact preview effects missing'
alias_out="$(run_pty print success alias)"; [[ "$alias_out" == *'redacted dynamic fragment'* ]] || fail 'compatibility alias diverged'
auth_out="$(run_pty print auth subcommand config failure)"; [[ "$auth_out" == *MODEL_AUTH_FAILED* ]] || fail 'auth failure missing'
empty_out="$(run_pty print empty subcommand config failure)"; [[ "$empty_out" == *MODEL_CATALOG_EMPTY* ]] || fail 'empty catalog failure missing'
stale_out="$(run_pty write stale subcommand config failure)"; [[ "$stale_out" == *PREVIEW_STALE* ]] || fail 'stale preview failure missing'
removed_out="$(run_pty write removed subcommand config failure)"; [[ "$removed_out" == *MODEL_NOT_AVAILABLE* ]] || fail 'removed model failure missing'
wizard_out="$(run_pty print success subcommand wizard failure)"; [[ "$wizard_out" == *'model ID 不能为空'* ]] || fail 'wizard automatically selected a model'
grep -Fq 'render shape_ok' "$tmp/manager.log" && grep -Fq 'write key_ok approvals_ok' "$tmp/manager.log" || fail 'v2 request checks missing'; ! grep -Fq 'v2-key-canary' "$tmp/manager.log" || fail 'fake manager log leaked key'
[[ "$(grep -c '^method agent.preview$' "$tmp/manager.log")" == 3 && "$(grep -c '^method agent.write$' "$tmp/manager.log")" == 2 ]] || fail 'print/no-state or terminal flow method matrix changed'
link="$tmp/config-link"; ln -s "$config" "$link"; if "${common[@]}" bash "$package/setup.sh" agent print-config --agent=claude --model-config="$link" </dev/null >/dev/null 2>&1; then fail 'noninteractive/symlink config accepted'; fi
invalid="$tmp/invalid.json"; printf '[]\n' >"$invalid"; models_before="$(grep -c '^method agent.models$' "$tmp/manager.log")"; if invalid_out="$(run_pty print success subcommand "$invalid" failure)"; then :; fi; [[ "$invalid_out" != *'输入隐藏'* ]] || fail 'invalid model config prompted for key'; [[ "$(grep -c '^method agent.models$' "$tmp/manager.log")" == "$models_before" ]] || fail 'invalid model config reached model discovery'
printf 'PASS: executable Shell v2 Agent model flow\n'
