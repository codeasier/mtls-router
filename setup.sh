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
  info " mtls-router Claude Code 一键配置工具"
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
  local json

  if [[ "$DOWNLOADER" == "curl" ]]; then
    json="$(curl -fsSL --retry 3 --connect-timeout 15 "$api_url")"
  else
    json="$(wget -qO- "$api_url")"
  fi

  printf '%s\n' "$json" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1
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

ensure_claude() {
  info "[检测] Claude Code 安装状态..."
  if command -v claude >/dev/null 2>&1; then
    success "  已找到 Claude Code：$(command -v claude)"
    return
  fi

  if [[ -x "$HOME/.local/bin/claude" ]]; then
    success "  已找到 Claude Code：$HOME/.local/bin/claude"
    export PATH="$HOME/.local/bin:$PATH"
    return
  fi

  warn "  未找到 Claude Code，开始尝试通过 npm 安装..."
  if ! command -v npm >/dev/null 2>&1; then
    fail "未找到 npm。请先安装 Node.js/npm 后重试。"
  fi

  npm install -g @anthropic-ai/claude-code --registry https://registry.npmmirror.com
  command -v claude >/dev/null 2>&1 || fail "Claude Code 安装后仍不可用，请检查 npm 全局 bin 是否在 PATH 中。"
  success "  Claude Code 安装完成：$(command -v claude)"
}

update_settings_json() {
  info "[修复] 写入 Claude Code settings.json..."
  local claude_dir="$HOME/.claude"
  local settings="$claude_dir/settings.json"
  mkdir -p "$claude_dir"

  if command -v node >/dev/null 2>&1; then
    SETTINGS_PATH="$settings" ANTHROPIC_BASE_URL_VALUE="$ANTHROPIC_BASE_URL_VALUE" node <<'NODE'
const fs = require('fs');
const path = process.env.SETTINGS_PATH;
let data = {};
try {
  if (fs.existsSync(path)) data = JSON.parse(fs.readFileSync(path, 'utf8'));
} catch {
  data = {};
}
data.env = data.env && typeof data.env === 'object' && !Array.isArray(data.env) ? data.env : {};
data.env.ANTHROPIC_BASE_URL = process.env.ANTHROPIC_BASE_URL_VALUE;
fs.writeFileSync(path, JSON.stringify(data, null, 2) + '\n');
NODE
  else
    if [[ -f "$settings" ]]; then
      fail "检测到已有 settings.json，但未找到 node，无法安全合并配置。请先安装 Node.js 后重试。"
    fi
    cat >"$settings" <<JSON
{
  "env": {
    "ANTHROPIC_BASE_URL": "$ANTHROPIC_BASE_URL_VALUE"
  }
}
JSON
  fi

  success "  已配置 ANTHROPIC_BASE_URL=$ANTHROPIC_BASE_URL_VALUE"
}

update_shell_profile() {
  info "[修复] 写入 shell 环境变量..."
  local profile
  if [[ -f "$HOME/.zshrc" ]]; then
    profile="$HOME/.zshrc"
  else
    profile="$HOME/.bashrc"
  fi

  touch "$profile"
  if grep -q '^export ANTHROPIC_BASE_URL=' "$profile"; then
    if [[ "$(uname -s)" == "Darwin" ]]; then
      sed -i '' "s|^export ANTHROPIC_BASE_URL=.*|export ANTHROPIC_BASE_URL=\"$ANTHROPIC_BASE_URL_VALUE\"|" "$profile"
    else
      sed -i "s|^export ANTHROPIC_BASE_URL=.*|export ANTHROPIC_BASE_URL=\"$ANTHROPIC_BASE_URL_VALUE\"|" "$profile"
    fi
  else
    printf '\nexport ANTHROPIC_BASE_URL="%s"\n' "$ANTHROPIC_BASE_URL_VALUE" >>"$profile"
  fi

  export ANTHROPIC_BASE_URL="$ANTHROPIC_BASE_URL_VALUE"
  if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    export PATH="$INSTALL_DIR:$PATH"
  fi
  success "  已更新：$profile"
}

start_router() {
  info "[启动] 启动 mtls-router 后台模式..."
  "$BINARY_PATH" -backend
  success "  mtls-router 已启动，监听地址通常为 $ROUTER_BASE_URL"
}

main() {
  print_banner
  download_router
  ensure_claude
  update_settings_json
  update_shell_profile
  start_router

  success "============================================================"
  success "配置完成！即将启动 Claude Code。"
  success "============================================================"
  exec claude
}

main "$@"
