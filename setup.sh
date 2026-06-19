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
    local out=""
    for ((i = 1; i <= total; i++)); do
      out+="${i} "
    done
    printf '%s' "${out% }"
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

print_next_steps() {
  success "============================================================"
  success "配置完成。"
  success "============================================================"
  info "mtls-router 已在后台运行："
  info "  $ROUTER_BASE_URL"
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
  info "可手动启动 agent。"
}

detect_agents() {
  DETECTED_NAMES=()
  DETECTED_COMMANDS=()
  DETECTED_CONFIG_PATHS=()
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

  local codex_bin
  codex_bin="$(type -P codex || true)"
  if [[ -n "$codex_bin" ]]; then
    DETECTED_NAMES+=("Codex")
    DETECTED_COMMANDS+=("$codex_bin")
    if [[ -n "${CODEX_HOME:-}" ]]; then
      DETECTED_CONFIG_PATHS+=("$CODEX_HOME/config.toml")
    else
      DETECTED_CONFIG_PATHS+=("$HOME/.codex/config.toml")
    fi
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
  cat <<'JSON'
{
  "ANTHROPIC_BASE_URL": "http://127.0.0.1:19099",
  "ANTHROPIC_AUTH_TOKEN": "{UserApiKey}",
  "ANTHROPIC_DEFAULT_HAIKU_MODEL": "cx/gpt-5.5",
  "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME": "gpt-5.5",
  "ANTHROPIC_DEFAULT_OPUS_MODEL": "cx/gpt-5.5",
  "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME": "gpt-5.5",
  "ANTHROPIC_DEFAULT_SONNET_MODEL": "cx/gpt-5.4[1M]",
  "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": "gpt-5.4",
  "ANTHROPIC_MODEL": "cx/gpt-5.5",
  "ENABLE_TOOL_SEARCH": "true",
  "DISABLE_AUTOUPDATER": "1"
}
JSON
}

configure_claude() {
  local path="$1"
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
    jq --argjson env "$(claude_env_block)" 'del(.env) + {env: $env}' "$path" >"$tmp"
  else
    jq -n --argjson env "$(claude_env_block)" '{env: $env}' >"$tmp"
  fi
  mv "$tmp" "$path"
  printf '%s\n%s\n' "$path" "${backup:-}"
}

opencode_provider_block() {
  cat <<'JSON'
{
  "mtls-router": {
    "npm": "@ai-sdk/openai-compatible",
    "name": "mtls-router",
    "options": {
      "baseURL": "http://127.0.0.1:19099",
      "apiKey": "{UserApiKey}"
    },
    "models": {
      "cx/gpt-5.5": {
        "name": "GPT-5.5",
        "reasoning": true,
        "attachment": true,
        "tool_call": true,
        "limit": { "context": 272000, "input": 244800, "output": 27200 },
        "modalities": { "input": ["text", "image"], "output": ["text"] },
        "options": { "reasoningEffort": "medium" }
      },
      "cx/gpt-5.4": {
        "name": "GPT-5.4",
        "reasoning": true,
        "attachment": true,
        "tool_call": true,
        "limit": { "context": 1000000, "input": 900000, "output": 100000 },
        "modalities": { "input": ["text", "image"], "output": ["text"] },
        "options": { "reasoningEffort": "medium" }
      }
    }
  }
}
JSON
}

configure_opencode() {
  local path="$1"
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
    jq --argjson prov "$(opencode_provider_block)" '.provider = ((.provider // {}) + $prov)' "$path" >"$tmp"
  else
    jq -n --argjson prov "$(opencode_provider_block)" '{provider: $prov}' >"$tmp"
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

configure_codex() {
  local path="$1"
  local backup=""
  if [[ -f "$path" ]]; then
    backup="$(backup_file "$path")"
  else
    mkdir -p "$(dirname "$path")"
    : >"$path"
  fi
  remove_codex_block "$path" 'model_providers\.mtls-router'
  remove_codex_block "$path" 'profiles\.gpt-5-5-router'
  remove_codex_block "$path" 'profiles\.gpt-5-4-1m-router'

  cat >>"$path" <<'TOML'

# mtls-router provider
[model_providers.mtls-router]
name = "mtls-router"
base_url = "http://127.0.0.1:19099/v1"
env_key = "{UserApiKey}"
wire_api = "responses"
request_max_retries = 2
stream_max_retries = 2
supports_websockets = false

# GPT-5.5 via mtls-router
[profiles.gpt-5-5-router]
model = "gpt-5.5"
model_provider = "mtls-router"
model_reasoning_effort = "medium"

# GPT-5.4 1M via mtls-router
[profiles.gpt-5-4-1m-router]
model = "gpt-5.4"
model_provider = "mtls-router"
model_reasoning_effort = "medium"
TOML

  printf '%s\n%s\n' "$path" "${backup:-}"
}

main() {
  print_banner
  download_router
  start_router
  detect_agents

  CONFIGURED_AGENT_PATHS=""
  CONFIGURED_BACKUPS=""

  if [[ "${#DETECTED_NAMES[@]}" -eq 0 ]]; then
    warn "  未检测到 Claude Code、opencode 或 Codex。mtls-router 已启动，但未写入 agent 配置。"
  else
    local total="${#DETECTED_NAMES[@]}"
    local selection
    if [[ "$total" -eq 1 ]]; then
      printf '\n检测到 %s：%s\n' "${DETECTED_NAMES[0]}" "${DETECTED_COMMANDS[0]}"
      printf '配置文件：%s\n' "${DETECTED_CONFIG_PATHS[0]}"
      printf '是否备份并写入配置？[y/N] '
      local reply
      read -r reply || reply=""
      if [[ "$reply" =~ ^[Yy]$ ]]; then
        selection="1"
      else
        selection=""
      fi
    else
      printf '\n检测到多个 Agent：\n'
      printf '0) 全部覆盖配置\n'
      for ((i = 0; i < total; i++)); do
        printf '%d) %s: %s -> %s\n' "$((i + 1))" "${DETECTED_NAMES[i]}" "${DETECTED_COMMANDS[i]}" "${DETECTED_CONFIG_PATHS[i]}"
      done
      printf '请输入编号，多个用空格分隔；直接回车则逐个询问： '
      read -r selection || true
      if [[ -z "$selection" ]]; then
        local prompted_selection=""
        local answer
        for ((i = 0; i < total; i++)); do
          printf '\n检测到 %s：%s\n' "${DETECTED_NAMES[i]}" "${DETECTED_COMMANDS[i]}"
          printf '配置文件：%s\n' "${DETECTED_CONFIG_PATHS[i]}"
          printf '是否备份并写入配置？[y/N] '
          read -r answer || answer=""
          if [[ "$answer" =~ ^[Yy]$ ]]; then
            prompted_selection+="$((i + 1)) "
          fi
        done
        selection="${prompted_selection% }"
      fi
    fi

    if [[ -z "$selection" ]]; then
      warn "  未选择任何 agent，跳过 agent 配置。"
      print_next_steps
      return 0
    fi

    local chosen
    if ! chosen="$(select_targets "$selection" "${DETECTED_NAMES[@]}")"; then
      fail "无效的 agent 选择：$selection"
    fi

    if [[ -z "$chosen" ]]; then
      warn "  未选择任何 agent，跳过 agent 配置。"
    else
      for token in $chosen; do
        local idx=$((token - 1))
        local name="${DETECTED_NAMES[$idx]}"
        local path="${DETECTED_CONFIG_PATHS[$idx]}"
        local result
        if [[ "$name" == "Claude Code" ]]; then
          result="$(configure_claude "$path")"
        elif [[ "$name" == "opencode" ]]; then
          result="$(configure_opencode "$path")"
        elif [[ "$name" == "Codex" ]]; then
          result="$(configure_codex "$path")"
        else
          fail "未知 agent：$name"
        fi
        local wrote backup
        wrote="$(printf '%s\n' "$result" | sed -n '1p')"
        backup="$(printf '%s\n' "$result" | sed -n '2p')"
        CONFIGURED_AGENT_PATHS+="${name}: ${wrote}"$'\n'
        if [[ -n "$backup" ]]; then
          CONFIGURED_BACKUPS+="${backup}"$'\n'
        fi
        success "  已写入 ${name} 配置：${wrote}"
      done
    fi
  fi

  print_next_steps
}

main "$@"
