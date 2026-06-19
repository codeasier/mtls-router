#!/usr/bin/env bash
set -euo pipefail

REPO="codeasier/mtls-router"
ROUTER_BASE_URL="http://127.0.0.1:19099"
ANTHROPIC_BASE_URL_VALUE="$ROUTER_BASE_URL"
INSTALL_DIR="${MTLS_ROUTER_INSTALL_DIR:-$HOME/.local/bin}"
BINARY_NAME="mtls-router"
BINARY_PATH="$INSTALL_DIR/$BINARY_NAME"

info() { printf '\033[36m%s\033[0m\n' "$1"; }
success() { printf '\033[32m%s\033[0m\n' "$1"; }
warn() { printf '\033[33m%s\033[0m\n' "$1"; }
fail() { printf '\033[31m%s\033[0m\n' "$1" >&2; exit 1; }

print_banner() {
  info "============================================================"
  info " mtls-router 代理配置向导"
  info "============================================================"
}

require_downloader() {
  if command -v curl >/dev/null 2>&1; then
    DOWNLOADER="curl"
  elif command -v wget >/dev/null 2>&1; then
    DOWNLOADER="wget"
  else
    fail "未找到 curl 或 wget，请先安装其中一个后重试。"
  fi
}

download_to() {
  local url="$1"
  local output="$2"

  if [[ "$DOWNLOADER" == "curl" ]]; then
    curl -fL --retry 3 --connect-timeout 15 -o "$output" "$url"
  else
    wget -O "$output" "$url"
  fi
}

latest_version() {
  local api_url="https://api.github.com/repos/$REPO/releases/latest"
  local web_url="https://github.com/$REPO/releases/latest"
  local json version headers

  if [[ "$DOWNLOADER" == "curl" ]]; then
    json="$(curl -fsSL --retry 3 --connect-timeout 15 "$api_url" 2>/dev/null || true)"
  else
    json="$(wget -qO- "$api_url" 2>/dev/null || true)"
  fi

  version="$(printf '%s\n' "$json" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)"
  if [[ -n "$version" ]]; then
    printf '%s\n' "$version"
    return
  fi

  if [[ "$DOWNLOADER" == "curl" ]]; then
    headers="$(curl -fsSLI --retry 3 --connect-timeout 15 "$web_url" 2>/dev/null || true)"
  else
    headers="$(wget --server-response --spider "$web_url" 2>&1 || true)"
  fi

  printf '%s\n' "$headers" | sed -n 's|^[Ll]ocation:[[:space:]]*.*/releases/tag/\([^[:space:]\r]*\).*|\1|p' | tail -n 1
}

detect_asset() {
  local os arch

  case "$(uname -s)" in
    Linux) os="linux" ;;
    Darwin) os="darwin" ;;
    *) fail "不支持的操作系统：$(uname -s)。请手动下载对应 release asset。" ;;
  esac

  case "$(uname -m)" in
    x86_64|amd64) arch="amd64" ;;
    arm64|aarch64) arch="arm64" ;;
    *) fail "不支持的 CPU 架构：$(uname -m)。请手动下载对应 release asset。" ;;
  esac

  ASSET="mtls-router-${os}-${arch}"
}

download_router() {
  if [[ "${MTLS_ROUTER_SKIP_DOWNLOAD:-}" == "1" ]]; then
    info "[下载] 跳过（MTLS_ROUTER_SKIP_DOWNLOAD=1）"
    return 0
  fi
  info "[下载] 检测并下载最新 mtls-router..."
  require_downloader
  detect_asset

  local version
  version="$(latest_version)"
  [[ -n "$version" ]] || fail "无法获取 GitHub 最新 release 版本。"

  mkdir -p "$INSTALL_DIR"

  local url="https://github.com/$REPO/releases/download/$version/$ASSET"
  local tmp
  tmp="$(mktemp)"
  trap "rm -f '$tmp'" EXIT

  info "  版本：$version"
  info "  平台：$ASSET"
  info "  安装：$BINARY_PATH"
  download_to "$url" "$tmp"
  chmod +x "$tmp"
  mv "$tmp" "$BINARY_PATH"
  success "  已安装 mtls-router：$BINARY_PATH"
}

start_router() {
  if [[ "${MTLS_ROUTER_SKIP_START:-}" == "1" ]]; then
    info "[启动] 跳过（MTLS_ROUTER_SKIP_START=1）"
    return 0
  fi
  info "[启动] 启动 mtls-router 后台模式..."
  "$BINARY_PATH" -backend
  success "  mtls-router 已启动，监听地址通常为 $ROUTER_BASE_URL"
}

select_targets() {
  local input="${1:-}"
  shift || true
  local total=0
  for _ in "$@"; do
    total=$((total + 1))
  done
  if [[ "$total" -eq 0 ]]; then
    return 0
  fi
  if [[ -z "$input" ]]; then
    return 0
  fi
  local result=""
  local saw_zero=0
  set -f
  for token in $input; do
    set +f
    if [[ "$token" == "0" ]]; then
      saw_zero=1
      set -f
      continue
    fi
    if ! [[ "$token" =~ ^[0-9]+$ ]]; then
      printf '无效编号：%s\n' "$token" >&2
      return 1
    fi
    if [[ "$token" =~ ^0[0-9]+$ ]]; then
      printf '无效编号：%s\n' "$token" >&2
      return 1
    fi
    if (( 10#$token < 1 || 10#$token > total )); then
      printf '编号越界：%s（有效范围 1-%d）\n' "$token" "$total" >&2
      return 1
    fi
    case " $result " in
      *" $token "*) ;;
      *) result+="$token " ;;
    esac
    set -f
  done
  set +f
  if (( saw_zero )); then
    local all=""
    for ((i = 1; i <= total; i++)); do
      all+="${i} "
    done
    printf '%s' "${all% }"
    return 0
  fi
  printf '%s' "${result% }"
}

# Convert an agent name to its canonical key used by --agent=...
agent_key() {
  case "$1" in
    "Claude Code") printf 'claude' ;;
    "opencode") printf 'opencode' ;;
    "Codex") printf 'codex' ;;
    *) return 1 ;;
  esac
}

# Reverse: canonical key -> displayed name.
agent_name_from_key() {
  case "$1" in
    claude) printf 'Claude Code' ;;
    opencode) printf 'opencode' ;;
    codex) printf 'Codex' ;;
    *) return 1 ;;
  esac
}

print_usage() {
  cat <<'USAGE'
用法: setup.sh [选项]

默认行为：下载并启动 mtls-router，**不会修改任何 agent 配置文件**。

选项:
  --print-config [--agent=claude,opencode,codex]
      把要写入的配置片段打印到 stdout（只输出，不动文件）。默认所有检测到的 agent。
  --write-config [--agent=claude,opencode,codex]
      把 mtls-router 配置写入检测到的 agent 配置文件。会先备份原文件。
      需要先 --agent= 指定至少一个，否则报错。
  --agent=LIST
      逗号分隔的 agent key：claude / opencode / codex。
      与 --print-config 或 --write-config 搭配使用。
  -h, --help
      显示本帮助。

示例:
  # 只下载并启动 router，不动 agent 配置
  ./setup.sh

  # 看看会写入哪些内容（不动文件）
  ./setup.sh --print-config

  # 只为 Claude Code 写入配置
  ./setup.sh --write-config --agent=claude

  # 为 opencode 和 Codex 写入配置
  ./setup.sh --write-config --agent=opencode,codex
USAGE
}

print_next_steps() {
  success "============================================================"
  success "配置完成。"
  success "============================================================"
  if [[ "${ROUTER_STARTED:-}" == "1" ]]; then
    info "mtls-router 已在后台运行："
    info "  $ROUTER_BASE_URL"
  else
    info "未启动 mtls-router（本次仅处理配置）。如需启动，请运行："
    info "  $0"
  fi
  if [[ -n "${CONFIGURED_AGENT_PATHS:-}" ]]; then
    info "已写入配置："
    while IFS= read -r line; do
      [[ -n "$line" ]] && info "  $line"
    done <<<"$CONFIGURED_AGENT_PATHS"
  else
    info "未写入任何 agent 配置。"
  fi
  if [[ -n "${CONFIGURED_BACKUPS:-}" ]]; then
    info "已备份："
    while IFS= read -r line; do
      [[ -n "$line" ]] && info "  $line"
    done <<<"$CONFIGURED_BACKUPS"
  fi
  info "现在可以手动启动 agent。"
}

detect_agents() {
  DETECTED_NAMES=()
  DETECTED_COMMANDS=()
  DETECTED_CONFIG_PATHS=()
  DETECTED_AUTH_PATHS=()
  local opencode_path=""

  local claude_bin
  claude_bin="$(type -P claude || true)"
  if [[ -n "$claude_bin" ]]; then
    DETECTED_NAMES+=("Claude Code")
    DETECTED_COMMANDS+=("$claude_bin")
    if [[ -n "${CLAUDE_CONFIG_DIR:-}" ]]; then
      DETECTED_CONFIG_PATHS+=("$CLAUDE_CONFIG_DIR/settings.json")
    else
      DETECTED_CONFIG_PATHS+=("$HOME/.claude/settings.json")
    fi
  fi

  local opencode_bin
  opencode_bin="$(type -P opencode || true)"
  if [[ -n "$opencode_bin" ]]; then
    DETECTED_NAMES+=("opencode")
    DETECTED_COMMANDS+=("$opencode_bin")
    if [[ -n "${OPENCODE_CONFIG:-}" ]]; then
      opencode_path="$OPENCODE_CONFIG"
    elif [[ -f "$HOME/.config/opencode/opencode.json" ]]; then
      opencode_path="$HOME/.config/opencode/opencode.json"
    elif [[ -f "$HOME/.config/opencode/opencode.jsonc" ]]; then
      opencode_path="$HOME/.config/opencode/opencode.jsonc"
    else
      opencode_path="$HOME/.config/opencode/opencode.json"
    fi
    DETECTED_CONFIG_PATHS+=("$opencode_path")
  fi

  local codex_home
  if [[ -n "${CODEX_HOME:-}" ]]; then
    codex_home="$CODEX_HOME"
  else
    codex_home="$HOME/.codex"
  fi
  # Detect Codex when either the CLI is on PATH, or the Codex Desktop
  # app has been used (which writes ~/.codex/config.toml and auth.json
  # without ever installing the CLI).
  local codex_bin
  codex_bin="$(type -P codex || true)"
  if [[ -n "$codex_bin" || -d "$codex_home" ]]; then
    DETECTED_NAMES+=("Codex")
    DETECTED_COMMANDS+=("${codex_bin:-<desktop>}")
    DETECTED_CONFIG_PATHS+=("$codex_home/config.toml")
    DETECTED_AUTH_PATHS+=("$codex_home/auth.json")
  fi
}

backup_file() {
  local path="$1"
  if [[ -f "$path" ]]; then
    local stamp
    stamp="$(date +%Y%m%d-%H%M%S%N)-$$"
    cp "$path" "${path}.bak-${stamp}"
    printf '%s' "${path}.bak-${stamp}"
  fi
}

claude_env_block() {
  local api_key="${1-}"
  [[ -n "$api_key" ]] || api_key='{UserApiKey}'
  jq -n \
    --arg token "$api_key" \
    '{
      ANTHROPIC_BASE_URL: "http://127.0.0.1:19099",
      ANTHROPIC_AUTH_TOKEN: $token,
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "cx/gpt-5.5",
      ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME: "gpt-5.5",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "cx/gpt-5.5",
      ANTHROPIC_DEFAULT_OPUS_MODEL_NAME: "gpt-5.5",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "cx/gpt-5.4[1M]",
      ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: "gpt-5.4",
      ANTHROPIC_MODEL: "cx/gpt-5.5",
      ENABLE_TOOL_SEARCH: "true",
      DISABLE_AUTOUPDATER: "1"
    }'
}

configure_claude() {
  local path="$1"
  local api_key="${2-}"
  [[ -n "$api_key" ]] || api_key='{UserApiKey}'
  local backup=""
  if [[ -f "$path" ]]; then
    if ! jq empty "$path" >/dev/null 2>&1; then
      fail "Claude Code 配置文件不是合法 JSON：$path"
    fi
    backup="$(backup_file "$path")"
  else
    mkdir -p "$(dirname "$path")"
  fi
  local tmp
  tmp="$(mktemp)"
  if [[ -f "$path" ]]; then
    jq --argjson env "$(claude_env_block "$api_key")" 'del(.env) + {env: $env}' "$path" >"$tmp"
  else
    jq -n --argjson env "$(claude_env_block "$api_key")" '{env: $env}' >"$tmp"
  fi
  mv "$tmp" "$path"
  printf '%s\n%s\n' "$path" "${backup:-}"
}

opencode_provider_block() {
  local api_key="${1-}"
  [[ -n "$api_key" ]] || api_key='{UserApiKey}'
  jq -n \
    --arg key "$api_key" \
    '{
      "mtls-router": {
        npm: "@ai-sdk/openai-compatible",
        name: "mtls-router",
        options: {
          baseURL: "http://127.0.0.1:19099/v1",
          apiKey: $key
        },
        models: {
          "cx/gpt-5.5": {
            name: "GPT-5.5",
            reasoning: true,
            attachment: true,
            tool_call: true,
            limit: { context: 272000, input: 244800, output: 27200 },
            modalities: { input: ["text", "image"], output: ["text"] },
            options: { reasoningEffort: "medium" }
          },
          "cx/gpt-5.4": {
            name: "GPT-5.4",
            reasoning: true,
            attachment: true,
            tool_call: true,
            limit: { context: 1000000, input: 900000, output: 100000 },
            modalities: { input: ["text", "image"], output: ["text"] },
            options: { reasoningEffort: "medium" }
          }
        }
      }
    }'
}

configure_opencode() {
  local path="$1"
  local api_key="${2-}"
  [[ -n "$api_key" ]] || api_key='{UserApiKey}'
  if [[ "$path" == *.jsonc ]]; then
    fail "opencode 当前选中的配置文件是 JSONC：$path（暂不支持就地合并）。请设置 OPENCODE_CONFIG 指向 JSON 文件。"
  fi
  local backup=""
  if [[ -f "$path" ]]; then
    if ! jq empty "$path" >/dev/null 2>&1; then
      fail "opencode 配置文件不是合法 JSON：$path"
    fi
    if ! jq -e '(.provider == null) or (.provider | type == "object")' "$path" >/dev/null 2>&1; then
      local ptype
      ptype="$(jq -r '.provider | type' "$path")"
      fail "opencode 现有 .provider 字段不是对象（实际为 ${ptype}），无法合并：$path"
    fi
    backup="$(backup_file "$path")"
  else
    mkdir -p "$(dirname "$path")"
  fi
  local tmp
  tmp="$(mktemp)"
  if [[ -f "$path" ]]; then
    jq --argjson prov "$(opencode_provider_block "$api_key")" '.provider = ((.provider // {}) + $prov)' "$path" >"$tmp"
  else
    jq -n --argjson prov "$(opencode_provider_block "$api_key")" '{provider: $prov}' >"$tmp"
  fi
  mv "$tmp" "$path"
  printf '%s\n%s\n' "$path" "${backup:-}"
}

remove_codex_block() {
  local file="$1" header="$2" tmp
  tmp="$(mktemp)"
  awk -v target="$header" '
    BEGIN { skip = 0 }
    {
      line = $0
      if (line ~ "^\\[" target "\\]") { skip = 1; next }
      if (skip && line ~ "^\\[") { skip = 0 }
      if (!skip) print line
    }
  ' "$file" >"$tmp"
  mv "$tmp" "$file"
}

remove_codex_root_keys() {
  local file="$1" tmp
  tmp="$(mktemp)"
  awk '
    BEGIN { in_root = 1 }
    /^[[:space:]]*\[/ { in_root = 0 }
    in_root && /^[[:space:]]*(model_provider|model|disable_response_storage)[[:space:]]*=/ { next }
    { print }
  ' "$file" >"$tmp"
  mv "$tmp" "$file"
}

configure_codex() {
  local path="$1"
  local api_key="${2:-}"
  local backup=""
  if [[ -f "$path" ]]; then
    backup="$(backup_file "$path")"
  else
    mkdir -p "$(dirname "$path")"
    : >"$path"
  fi
  remove_codex_root_keys "$path"
  remove_codex_block "$path" 'model_providers\.custom'

  local body_tmp final_tmp
  body_tmp="$(mktemp)"
  final_tmp="$(mktemp)"
  cp "$path" "$body_tmp"
  cat >"$final_tmp" <<'TOML'
model_provider = "custom"
model = "gpt-5.5"
disable_response_storage = true

[model_providers.custom]
name = "9router"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:19099/v1"
TOML
  if [[ -s "$body_tmp" ]]; then
    printf '\n' >>"$final_tmp"
    cat "$body_tmp" >>"$final_tmp"
  fi
  mv "$final_tmp" "$path"
  rm -f "$body_tmp"

  if [[ -n "$api_key" ]]; then
    local auth_path auth_perm
    auth_path="$(dirname "$path")/auth.json"
    local auth_backup=""
    if [[ -f "$auth_path" ]]; then
      auth_perm="$(stat -f %Lp "$auth_path" 2>/dev/null || stat -c %a "$auth_path" 2>/dev/null || echo '')"
      auth_backup="$(backup_file "$auth_path")"
    else
      mkdir -p "$(dirname "$auth_path")"
      auth_perm=""
    fi
    local auth_tmp
    auth_tmp="$(mktemp)"
    jq -n --arg key "$api_key" '{OPENAI_API_KEY: $key}' >"$auth_tmp"
    mv "$auth_tmp" "$auth_path"
    if [[ -n "$auth_perm" ]]; then
      chmod "$auth_perm" "$auth_path" 2>/dev/null || true
    fi
    printf '%s\n%s\n' "$path" "${backup:-}"
    printf 'AUTH:%s\n%s\n' "$auth_path" "${auth_backup:-}"
  else
    printf '%s\n%s\n' "$path" "${backup:-}"
  fi
}

main() {
  local action="start"
  local agent_filter=""

  while (( $# > 0 )); do
    case "$1" in
      --print-config)
        action="print"
        shift
        ;;
      --write-config)
        action="write"
        shift
        ;;
      --agent=*)
        agent_filter="${1#--agent=}"
        shift
        ;;
      --agent)
        if (( $# < 2 )); then
          fail "--agent 需要一个参数（逗号分隔的列表）"
        fi
        agent_filter="$2"
        shift 2
        ;;
      -h|--help)
        print_usage
        exit 0
        ;;
      *)
        fail "未知参数：$1（试试 --help）"
        ;;
    esac
  done

  print_banner

  # Default action: download + start only. Never touch agent config.
  if [[ "$action" == "start" ]]; then
    download_router
    start_router
    ROUTER_STARTED=1
    info "提示：未对 agent 配置做任何改动。如需写入 mtls-router 配置："
    info "  $0 --write-config --agent=claude,opencode,codex"
    info "先看会写什么：$0 --print-config"
    if [[ "${MTLS_ROUTER_SKIP_START:-}" == "1" ]]; then
      info "（已跳过实际启动 mtls-router）"
    fi
    return 0
  fi

  detect_agents

  # --write-config 必须显式指定 --agent=。
  # --print-config 不指定时打印所有检测到的 agent。
  if [[ -z "$agent_filter" && "$action" == "write" ]]; then
    fail "--write-config 需要 --agent=claude,opencode,codex 显式指定要操作的 agent。"
  fi

  # Resolve the target list.
  local targets=()
  if [[ -z "$agent_filter" ]]; then
    if [[ "${#DETECTED_NAMES[@]}" -eq 0 ]]; then
      warn "  未检测到 Claude Code、opencode 或 Codex。"
      info "提示：用 --agent=claude,opencode,codex 显式指定目标。"
      return 0
    fi
    for name in "${DETECTED_NAMES[@]}"; do
      targets+=("$(agent_key "$name")")
    done
  else
    local token
    local seen=" "
    local idx
    local -a tokens
    IFS=',' read -ra tokens <<<"$agent_filter"
    for token in "${tokens[@]}"; do
      token="${token// /}"
      [[ -z "$token" ]] && continue
      case " $seen " in
        *" $token "*) fail "--agent 列表中有重复项：$token" ;;
      esac
      seen+="$token "

      local idx
      local detected=""
      idx=-1
      for ((i = 0; i < ${#DETECTED_NAMES[@]}; i++)); do
        if [[ "$(agent_key "${DETECTED_NAMES[$i]}")" == "$token" ]]; then
          idx=$i
          break
        fi
      done
      if (( idx < 0 )); then
        if [[ "${#DETECTED_NAMES[@]}" -gt 0 ]]; then
          detected="${DETECTED_NAMES[*]}"
        else
          detected="(无)"
        fi
        fail "未检测到 agent: $token (已检测: $detected)"
      fi
      targets+=("$token")
    done
  fi

  CONFIGURED_AGENT_PATHS=""
  CONFIGURED_BACKUPS=""

  local shared_api_key=""
  if [[ "$action" == "write" ]]; then
    local needs_api_key=0
    for key in "${targets[@]}"; do
      case "$key" in
        claude|opencode|codex)
          needs_api_key=1
          break
          ;;
      esac
    done
    if (( needs_api_key )); then
      shared_api_key="${MTLS_ROUTER_OPENAI_API_KEY:-}"
      if [[ -z "$shared_api_key" && -t 0 ]]; then
        printf '请输入 mtls-router OPENAI_API_KEY（输入隐藏）：' >&2
        IFS= read -rs shared_api_key || true
        printf '\n' >&2
      fi
      if [[ -z "$shared_api_key" ]]; then
        fail "写入 claude/opencode/codex 配置需要 apikey。可在 TTY 下重试，或通过 MTLS_ROUTER_OPENAI_API_KEY 环境变量传入。"
      fi
    fi
  fi

  local key
  for key in "${targets[@]}"; do
    name="$(agent_name_from_key "$key")"
    local idx=-1
    for ((i = 0; i < ${#DETECTED_NAMES[@]}; i++)); do
      if [[ "$(agent_key "${DETECTED_NAMES[$i]}")" == "$key" ]]; then
        idx=$i
        break
      fi
    done
    path="${DETECTED_CONFIG_PATHS[$idx]}"

    if [[ "$action" == "print" ]]; then
      case "$key" in
        claude)
          printf '### %s -> %s\n' "$name" "$path"
          printf '### 把以下片段合并到 %s 的现有 settings.json 中（保留其他字段）：\n\n' "$path"
          jq '{env: (.env + $env)}' --argjson env "$(claude_env_block)" \
            <(printf '{"env":{}}') 2>/dev/null || claude_env_block
          printf '\n'
          ;;
        opencode)
          printf '### %s -> %s\n' "$name" "$path"
          printf '### 把以下片段合并到 %s（写入 .provider 字段）：\n\n' "$path"
          opencode_provider_block
          printf '\n'
          ;;
        codex)
          local auth_path
          auth_path="$(dirname "$path")/auth.json"
          printf '### %s -> %s\n' "$name" "$path"
          printf '### 使用以下最小 TOML 配置 %s：\n\n' "$path"
          cat <<'TOML'
model_provider = "custom"
model = "gpt-5.5"
disable_response_storage = true

[model_providers.custom]
name = "9router"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:19099/v1"
TOML
          printf '\n### %s -> %s\n' "$name" "$auth_path"
          printf '### 将 %s 覆盖为以下最小 JSON：\n\n' "$auth_path"
          cat <<'JSON'
{
  "OPENAI_API_KEY": "{UserApiKey}"
}
JSON
          printf '\n'
          ;;
      esac
      continue
    fi

    # action=write
    local result
    case "$key" in
      claude) result="$(configure_claude "$path" "$shared_api_key")" ;;
      opencode) result="$(configure_opencode "$path" "$shared_api_key")" ;;
      codex)
        result="$(configure_codex "$path" "$shared_api_key")"
        ;;
      *) fail "未知 agent key：$key" ;;
    esac
    local wrote backup
    wrote="$(printf '%s\n' "$result" | sed -n '1p')"
    backup="$(printf '%s\n' "$result" | sed -n '2p')"
    CONFIGURED_AGENT_PATHS+="${name}: ${wrote}"$'\n'
    if [[ -n "$backup" ]]; then
      CONFIGURED_BACKUPS+="${backup}"$'\n'
    fi
    # codex also writes auth.json; the configure_codex output may include
    # an "AUTH:<auth_path>" line followed by the auth backup. Surface it.
    if [[ "$key" == "codex" ]]; then
      local auth_line auth_path auth_backup
      auth_line="$(printf '%s\n' "$result" | sed -n '3p' || true)"
      if [[ "$auth_line" == AUTH:* ]]; then
        auth_path="${auth_line#AUTH:}"
        auth_backup="$(printf '%s\n' "$result" | sed -n '4p' || true)"
        CONFIGURED_AGENT_PATHS+="${name} auth: ${auth_path}"$'\n'
        if [[ -n "$auth_backup" ]]; then
          CONFIGURED_BACKUPS+="${auth_backup}"$'\n'
        fi
        success "  已写入 ${name} auth.json：${auth_path}"
        if [[ -n "$auth_backup" ]]; then
          success "  备份：${auth_backup}"
        fi
      fi
    fi
    success "  已写入 ${name} 配置：${wrote}"
    if [[ -n "$backup" ]]; then
      success "  备份：${backup}"
    fi
  done

  if [[ "$action" == "print" ]]; then
    return 0
  fi

  print_next_steps
}

main "$@"
