#!/usr/bin/env bash
set -euo pipefail

REPO="codeasier/mtls-router"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
INSTALL_DIR="${MTLS_ROUTER_INSTALL_DIR:-$HOME/.local/bin}"
STATE_DIR="${MTLS_ROUTER_STATE_DIR:-$HOME/.mtls-router}"
DEFAULT_DOWNLOAD_BASE_URL=""
DOWNLOAD_BASE_URL="${MTLS_ROUTER_DOWNLOAD_URL:-$DEFAULT_DOWNLOAD_BASE_URL}"
DOWNLOAD_USER="${MTLS_ROUTER_DOWNLOAD_USER:-}"
DOWNLOAD_PASSWORD="${MTLS_ROUTER_DOWNLOAD_PASSWORD:-}"
ALLOW_DOWNLOAD="${MTLS_ROUTER_ALLOW_DOWNLOAD:-0}"
ROUTER_PATH="$INSTALL_DIR/mtls-router"
MANAGER_PATH="$INSTALL_DIR/mtls-router-manager"
RECEIPT_PATH="$STATE_DIR/install-receipt.json"
PENDING_PATH="$STATE_DIR/install-pending.json"
BACKUP_DIR="$STATE_DIR/install-previous"
ROUTER_BASE_URL="http://127.0.0.1:19099"
MANAGEMENT_PROTOCOL_VERSION="2"
MAX_MODEL_CONFIG_SIZE=$((2 * 1024 * 1024))
TRANSIENT_API_KEY=""
TRANSIENT_REQUEST=""

info() { printf '\033[36m%s\033[0m\n' "$1"; }
success() { printf '\033[32m%s\033[0m\n' "$1"; }
warn() { printf '\033[33m%s\033[0m\n' "$1"; }
fail() { printf '\033[31m%s\033[0m\n' "$1" >&2; exit 1; }
cleanup_agent_secrets() { TRANSIENT_API_KEY=""; TRANSIENT_REQUEST=""; }
trap cleanup_agent_secrets EXIT

print_banner() {
  info "============================================================"
  info " mtls-router 代理配置向导"
  info "============================================================"
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    fail "未找到 SHA-256 校验工具（sha256sum 或 shasum）。"
  fi
}

expected_checksum() {
  local manifest="$1" asset="$2" candidates candidate
  candidates="$(awk -v asset="$asset" 'substr($0, length($0)-length(asset)+1)==asset && substr($0, length($0)-length(asset),1) ~ /[[:space:]\*]/ {print}' "$manifest")"
  [[ "$(printf '%s\n' "$candidates" | awk 'NF {n++} END {print n+0}')" == 1 ]] ||
    fail "SHA256SUMS 必须包含且仅包含一条 $asset 校验候选记录。"
  candidate="$candidates"
  [[ "$candidate" =~ ^[[:xdigit:]]{64}[[:space:]]([[:space:]]|\*)$asset$ ]] ||
    fail "SHA256SUMS 中的 $asset 校验记录格式无效。"
  printf '%s\n' "${candidate:0:64}" | tr '[:upper:]' '[:lower:]'
}

verify_checksum() {
  local actual expected
  expected="$(expected_checksum "$2" "$3")"
  actual="$(sha256_file "$1" | tr '[:upper:]' '[:lower:]')"
  [[ "$actual" == "$expected" ]] || fail "SHA-256 校验失败：$3"
}

detect_assets() {
  local os arch ext=""
  case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=darwin ;;
    *) fail "不支持的操作系统：$(uname -s)。" ;;
  esac
  case "$(uname -m)" in
    x86_64|amd64) arch=amd64 ;;
    arm64|aarch64) arch=arm64 ;;
    *) fail "不支持的 CPU 架构：$(uname -m)。" ;;
  esac
  ROUTER_ASSET="mtls-router-${os}-${arch}${ext}"
  MANAGER_ASSET="mtls-router-manager-${os}-${arch}${ext}"
}

sync_storage() {
  sync >/dev/null 2>&1 || true
}

write_private_json() {
  local path="$1" value="$2" dir tmp
  dir="$(dirname "$path")"
  mkdir -p "$dir"
  chmod 700 "$dir"
  tmp="$(mktemp "$dir/.${path##*/}.XXXXXX")"
  printf '%s\n' "$value" >"$tmp"
  chmod 600 "$tmp"
  sync_storage
  mv -f "$tmp" "$path"
  sync_storage
}

hash_matches() {
  [[ -f "$1" && "$(sha256_file "$1" | tr '[:upper:]' '[:lower:]')" == "$2" ]]
}

receipt_is_valid() {
  [[ -f "$RECEIPT_PATH" ]] || return 1
  jq -e --arg router "$ROUTER_PATH" --arg manager "$MANAGER_PATH" --arg protocol "$MANAGEMENT_PROTOCOL_VERSION" '
    .schema_version == 1 and .deployment_id != "" and
    .management_protocol_version == $protocol and
    .router.path == $router and .manager.path == $manager and
    (.router.sha256 | test("^[0-9a-f]{64}$")) and
    (.manager.sha256 | test("^[0-9a-f]{64}$"))' "$RECEIPT_PATH" >/dev/null 2>&1 || return 1
  hash_matches "$ROUTER_PATH" "$(jq -r .router.sha256 "$RECEIPT_PATH")" || return 1
  hash_matches "$MANAGER_PATH" "$(jq -r .manager.sha256 "$RECEIPT_PATH")"
}

commit_pending_generation() {
  local receipt
  receipt="$(jq '{schema_version:1,deployment_id,management_protocol_version,installed_at,router,manager}' "$PENDING_PATH")"
  write_private_json "$RECEIPT_PATH" "$receipt"
  rm -f "$PENDING_PATH"
  rm -rf "$BACKUP_DIR"
  sync_storage
}

restore_previous_generation() {
  local had_previous router_hash manager_hash
  had_previous="$(jq -r '.previous.committed' "$PENDING_PATH")"
  if [[ "$had_previous" == true ]]; then
    router_hash="$(jq -r '.previous.router.sha256' "$PENDING_PATH")"
    manager_hash="$(jq -r '.previous.manager.sha256' "$PENDING_PATH")"
    hash_matches "$BACKUP_DIR/mtls-router" "$router_hash" || return 1
    hash_matches "$BACKUP_DIR/mtls-router-manager" "$manager_hash" || return 1
    cp "$BACKUP_DIR/mtls-router" "$INSTALL_DIR/.mtls-router.restore"
    cp "$BACKUP_DIR/mtls-router-manager" "$INSTALL_DIR/.mtls-router-manager.restore"
    chmod 0755 "$INSTALL_DIR/.mtls-router.restore" "$INSTALL_DIR/.mtls-router-manager.restore"
    mv -f "$INSTALL_DIR/.mtls-router.restore" "$ROUTER_PATH"
    mv -f "$INSTALL_DIR/.mtls-router-manager.restore" "$MANAGER_PATH"
    hash_matches "$ROUTER_PATH" "$router_hash" || return 1
    hash_matches "$MANAGER_PATH" "$manager_hash" || return 1
    write_private_json "$RECEIPT_PATH" "$(jq '.previous.receipt' "$PENDING_PATH")"
  else
    rm -f "$ROUTER_PATH" "$MANAGER_PATH" "$RECEIPT_PATH"
    [[ ! -e "$ROUTER_PATH" && ! -e "$MANAGER_PATH" ]] || return 1
  fi
  rm -f "$PENDING_PATH"
  rm -rf "$BACKUP_DIR"
  sync_storage
}

reconcile_install() {
  [[ -e "$PENDING_PATH" ]] || return 0
  jq -e --arg router "$ROUTER_PATH" --arg manager "$MANAGER_PATH" '
    .schema_version == 1 and .router.path == $router and .manager.path == $manager and
    (.previous.committed == true or .previous.committed == false)' "$PENDING_PATH" >/dev/null 2>&1 ||
    fail "安装事务标记损坏，拒绝执行 router 或 manager：$PENDING_PATH"
  if hash_matches "$ROUTER_PATH" "$(jq -r .router.sha256 "$PENDING_PATH")" &&
    hash_matches "$MANAGER_PATH" "$(jq -r .manager.sha256 "$PENDING_PATH")"; then
    commit_pending_generation
    success "  已完成中断的 mtls-router/manager 安装事务。" >&2
    return
  fi
  restore_previous_generation || fail "无法证明新旧任一完整安装代，拒绝执行 router 或 manager。"
  warn "  已回滚中断的 mtls-router/manager 安装事务。" >&2
}

binary_version() {
  local output
  output="$("$1" --version 2>/dev/null)" || fail "无法读取已验证二进制版本：$1"
  printf '%s\n' "${output##* }"
}

manager_info_from() {
  local request response
  request='{"id":"setup-info","method":"manager.info","params":{}}'
  response="$(printf '%s\n' "$request" | "$1" serve --router-sidecar "$2")" || fail "无法读取 manager 元数据。"
  [[ "$(printf '%s\n' "$response" | wc -l | tr -d ' ')" == 1 ]] || fail "manager 返回了无效的多行响应。"
  printf '%s\n' "$response" | jq -e --arg protocol "$MANAGEMENT_PROTOCOL_VERSION" '.id == "setup-info" and .result.version != "" and .result.deployment_id != "" and .result.management_protocol_version == $protocol' >/dev/null ||
    fail "manager 元数据响应无效。"
  printf '%s\n' "$response"
}

install_pair() {
  local source_router="$1" source_manager="$2" router_hash manager_hash info_json router_version pending previous=false
  router_hash="$(sha256_file "$source_router" | tr '[:upper:]' '[:lower:]')"
  manager_hash="$(sha256_file "$source_manager" | tr '[:upper:]' '[:lower:]')"
  info_json="$(manager_info_from "$source_manager" "$source_router")"
  router_version="$(binary_version "$source_router")"
  [[ "$router_version" == "$(printf '%s' "$info_json" | jq -r .result.version)" ]] ||
    fail "router 与 manager 版本不匹配，拒绝安装。"
  mkdir -p "$INSTALL_DIR" "$STATE_DIR"
  chmod 700 "$STATE_DIR"
  reconcile_install

  rm -rf "$BACKUP_DIR"
  mkdir -p "$BACKUP_DIR"
  chmod 700 "$BACKUP_DIR"
  if [[ -e "$ROUTER_PATH" || -e "$MANAGER_PATH" || -e "$RECEIPT_PATH" ]]; then
    receipt_is_valid || fail "现有安装不是 receipt 验证的完整二进制对；拒绝部分覆盖。"
    cp "$ROUTER_PATH" "$BACKUP_DIR/mtls-router"
    cp "$MANAGER_PATH" "$BACKUP_DIR/mtls-router-manager"
    chmod 0700 "$BACKUP_DIR/mtls-router" "$BACKUP_DIR/mtls-router-manager"
    previous=true
  fi
  pending="$(jq -n \
    --arg id "$(date +%s)-$$" --arg installed_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg deployment "$(printf '%s' "$info_json" | jq -r .result.deployment_id)" \
    --arg protocol "$(printf '%s' "$info_json" | jq -r .result.management_protocol_version)" \
    --arg router_path "$ROUTER_PATH" --arg router_hash "$router_hash" --arg router_version "$router_version" \
    --arg manager_path "$MANAGER_PATH" --arg manager_hash "$manager_hash" \
    --arg manager_version "$(printf '%s' "$info_json" | jq -r .result.version)" \
    --argjson committed "$previous" --slurpfile old <(if [[ "$previous" == true ]]; then cat "$RECEIPT_PATH"; else printf '{}'; fi) '
    {schema_version:1,transaction_id:$id,installed_at:$installed_at,deployment_id:$deployment,
     management_protocol_version:$protocol,
     router:{path:$router_path,sha256:$router_hash,version:$router_version},
     manager:{path:$manager_path,sha256:$manager_hash,version:$manager_version},
     previous:{committed:$committed,receipt:$old[0],router:($old[0].router // {}),manager:($old[0].manager // {})}}')"
  write_private_json "$PENDING_PATH" "$pending"

  cp "$source_router" "$INSTALL_DIR/.mtls-router.new"
  chmod 0755 "$INSTALL_DIR/.mtls-router.new"
  mv -f "$INSTALL_DIR/.mtls-router.new" "$ROUTER_PATH"
  sync_storage
  [[ "${MTLS_ROUTER_INSTALL_CRASH_POINT:-}" != after-router ]] || exit 97

  cp "$source_manager" "$INSTALL_DIR/.mtls-router-manager.new"
  chmod 0755 "$INSTALL_DIR/.mtls-router-manager.new"
  mv -f "$INSTALL_DIR/.mtls-router-manager.new" "$MANAGER_PATH"
  sync_storage
  [[ "${MTLS_ROUTER_INSTALL_CRASH_POINT:-}" != after-manager ]] || exit 97

  hash_matches "$ROUTER_PATH" "$router_hash" && hash_matches "$MANAGER_PATH" "$manager_hash" ||
    fail "安装后二进制哈希验证失败；下次执行将先回滚。"
  [[ "${MTLS_ROUTER_INSTALL_CRASH_POINT:-}" != before-receipt ]] || exit 97
  commit_pending_generation
}

require_downloader() {
  if command -v curl >/dev/null 2>&1; then DOWNLOADER=curl
  elif command -v wget >/dev/null 2>&1; then DOWNLOADER=wget
  else fail "未找到 curl 或 wget，请先安装其中一个后重试。"; fi
}

download_to() {
  local url="$1" output="$2"
  [[ "$url" == https://* ]] || fail "拒绝非 HTTPS 下载地址：$url"
  if [[ "$DOWNLOADER" == curl ]]; then
    if [[ -n "$DOWNLOAD_USER" || -n "$DOWNLOAD_PASSWORD" ]]; then
      curl -fL --retry 3 --connect-timeout 15 -u "${DOWNLOAD_USER}:${DOWNLOAD_PASSWORD}" -o "$output" "$url"
    else curl -fL --retry 3 --connect-timeout 15 -o "$output" "$url"; fi
  else
    if [[ -n "$DOWNLOAD_USER" || -n "$DOWNLOAD_PASSWORD" ]]; then
      wget --user="$DOWNLOAD_USER" --password="$DOWNLOAD_PASSWORD" -O "$output" "$url"
    else wget -O "$output" "$url"; fi
  fi
}

latest_version() {
  local json version headers
  if [[ "$DOWNLOADER" == curl ]]; then json="$(curl -fsSL --retry 3 --connect-timeout 15 "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null || true)"
  else json="$(wget -qO- "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null || true)"; fi
  version="$(printf '%s\n' "$json" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)"
  [[ -z "$version" ]] || { printf '%s\n' "$version"; return; }
  if [[ "$DOWNLOADER" == curl ]]; then headers="$(curl -fsSLI --retry 3 --connect-timeout 15 "https://github.com/$REPO/releases/latest" 2>/dev/null || true)"
  else headers="$(wget --server-response --spider "https://github.com/$REPO/releases/latest" 2>&1 || true)"; fi
  printf '%s\n' "$headers" | sed -n 's|^[Ll]ocation:[[:space:]]*.*/releases/tag/\([^[:space:]\r]*\).*|\1|p' | tail -n 1
}

asset_url() {
  local version="$1" asset="$2"
  if [[ -n "$DOWNLOAD_BASE_URL" ]]; then
    if [[ "$DOWNLOAD_BASE_URL" == */mtls-router-* ]]; then printf '%s/%s\n' "${DOWNLOAD_BASE_URL%/*}" "$asset"
    else printf '%s/%s\n' "${DOWNLOAD_BASE_URL%/}" "$asset"; fi
  else printf 'https://github.com/%s/releases/download/%s/%s\n' "$REPO" "$version" "$asset"; fi
}

install_router_pair() {
  [[ "${MTLS_ROUTER_SKIP_DOWNLOAD:-}" != 1 ]] || { info "[下载] 跳过（MTLS_ROUTER_SKIP_DOWNLOAD=1）"; reconcile_install; return; }
  detect_assets
  local packaged_router="$SCRIPT_DIR/$ROUTER_ASSET" packaged_manager="$SCRIPT_DIR/$MANAGER_ASSET" manifest="$SCRIPT_DIR/SHA256SUMS" any_payload=0
  compgen -G "$SCRIPT_DIR/mtls-router-*-*" >/dev/null && any_payload=1 || true
  if (( any_payload )); then
    [[ -f "$packaged_router" && -f "$packaged_manager" && -f "$manifest" ]] ||
      fail "检测到不完整的随包二进制对；拒绝联网回退。需要 $ROUTER_ASSET、$MANAGER_ASSET 和 SHA256SUMS。"
    verify_checksum "$packaged_router" "$manifest" "$ROUTER_ASSET"
    verify_checksum "$packaged_manager" "$manifest" "$MANAGER_ASSET"
    install_pair "$packaged_router" "$packaged_manager"
    success "  已验证并安装 mtls-router 与 manager：$INSTALL_DIR"
    return
  fi
  if [[ "$ALLOW_DOWNLOAD" != 1 ]]; then
    if [[ -t 0 && -t 2 ]]; then
      local answer; read -r -p "未找到随包二进制，是否通过 HTTPS 下载 router 与 manager？[y/N] " answer
      [[ "$answer" =~ ^[Yy]$ ]] || fail "已取消联网下载。"
    else fail "未找到随包二进制；非交互安装需显式传入 --download。"; fi
  fi
  require_downloader
  local version="${MTLS_ROUTER_VERSION:-}" tmp_dir router_url manager_url manifest_url
  if [[ -z "$version" ]]; then
    if [[ -n "$DOWNLOAD_BASE_URL" ]]; then version=latest; else version="$(latest_version)"; fi
  fi
  [[ -n "$version" ]] || fail "无法获取 GitHub 最新 release 版本。"
  router_url="$(asset_url "$version" "$ROUTER_ASSET")"
  manager_url="$(asset_url "$version" "$MANAGER_ASSET")"
  manifest_url="$(asset_url "$version" SHA256SUMS)"
  [[ "$router_url" == https://* && "$manager_url" == https://* && "$manifest_url" == https://* ]] || fail "拒绝非 HTTPS 下载地址。"
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "${tmp_dir:-}"' RETURN
  download_to "$router_url" "$tmp_dir/$ROUTER_ASSET"
  download_to "$manager_url" "$tmp_dir/$MANAGER_ASSET"
  download_to "$manifest_url" "$tmp_dir/SHA256SUMS"
  verify_checksum "$tmp_dir/$ROUTER_ASSET" "$tmp_dir/SHA256SUMS" "$ROUTER_ASSET"
  verify_checksum "$tmp_dir/$MANAGER_ASSET" "$tmp_dir/SHA256SUMS" "$MANAGER_ASSET"
  chmod 0700 "$tmp_dir/$ROUTER_ASSET" "$tmp_dir/$MANAGER_ASSET"
  install_pair "$tmp_dir/$ROUTER_ASSET" "$tmp_dir/$MANAGER_ASSET"
  rm -rf "$tmp_dir"; trap - RETURN
  success "  已安装 mtls-router 与 manager：$INSTALL_DIR"
}

resolve_manager() {
  detect_assets
  reconcile_install
  local sibling_router="$SCRIPT_DIR/$ROUTER_ASSET" sibling_manager="$SCRIPT_DIR/$MANAGER_ASSET" manifest="$SCRIPT_DIR/SHA256SUMS" any_payload=0
  compgen -G "$SCRIPT_DIR/mtls-router-*-*" >/dev/null && any_payload=1 || true
  if (( any_payload )); then
    [[ -f "$sibling_router" && -f "$sibling_manager" && -f "$manifest" ]] ||
      fail "随包二进制对不完整；Agent 命令不会联网下载。请重新解压完整包或运行 router install。"
    verify_checksum "$sibling_router" "$manifest" "$ROUTER_ASSET"
    verify_checksum "$sibling_manager" "$manifest" "$MANAGER_ASSET"
    RESOLVED_MANAGER="$sibling_manager"
    RESOLVED_ROUTER="$sibling_router"
    return
  fi
  receipt_is_valid || fail "未找到 receipt 验证的 manager/router；Agent 命令不会隐式下载。请先运行 router install。"
  RESOLVED_MANAGER="$MANAGER_PATH"
  RESOLVED_ROUTER="$ROUTER_PATH"
}

manager_call() {
  local request="$1" response
  resolve_manager
  response="$(printf '%s\n' "$request" | "$RESOLVED_MANAGER" serve --router-sidecar "$RESOLVED_ROUTER")" || fail "mtls-router-manager 执行失败。"
  [[ "$(printf '%s\n' "$response" | wc -l | tr -d ' ')" == 1 ]] || fail "manager 返回了无效的多行响应。"
  if [[ "$(printf '%s' "$response" | jq -r '.error.code // empty')" != "" ]]; then
    fail "manager $(printf '%s' "$response" | jq -r '.error.code'): $(printf '%s' "$response" | jq -r '.error.message')"
  fi
  printf '%s\n' "$response"
}

manager_secret_call() {
  local request="$1" info_response operation_response expected_target
  resolve_manager
  info_response="$(printf '%s\n' '{"id":"setup-secret-info","method":"manager.info","params":{}}' | "$RESOLVED_MANAGER" serve --router-sidecar "$RESOLVED_ROUTER")" || fail "无法读取 manager 元数据。"
  [[ "$(printf '%s\n' "$info_response" | wc -l | tr -d ' ')" == 1 ]] || fail "manager 返回了无效的握手响应。"
  expected_target="${ROUTER_ASSET#mtls-router-}"; expected_target="${expected_target/-//}"
  printf '%s' "$info_response" | jq -e --arg protocol "$MANAGEMENT_PROTOCOL_VERSION" --arg target "$expected_target" '.id == "setup-secret-info" and .error == null and .result.deployment_id != "" and .result.target == $target and .result.management_protocol_version == $protocol' >/dev/null ||
    fail "manager protocol v2 握手失败；未接受含密钥请求。"
  if [[ "$RESOLVED_MANAGER" == "$MANAGER_PATH" ]]; then
    [[ "$(printf '%s' "$info_response" | jq -r .result.deployment_id)" == "$(jq -r .deployment_id "$RECEIPT_PATH")" ]] || fail "manager deployment 握手与安装 receipt 不匹配；未接受含密钥请求。"
  fi
  operation_response="$(printf '%s\n' "$request" | "$RESOLVED_MANAGER" serve --router-sidecar "$RESOLVED_ROUTER")" || fail "mtls-router-manager 执行失败。"
  [[ "$(printf '%s\n' "$operation_response" | wc -l | tr -d ' ')" == 1 ]] || fail "manager 返回了无效的响应。"
  if [[ "$(printf '%s' "$operation_response" | jq -r '.error.code // empty')" != "" ]]; then
    fail "manager $(printf '%s' "$operation_response" | jq -r '.error.code'): $(printf '%s' "$operation_response" | jq -r '.error.message')"
  fi
  printf '%s\n' "$operation_response"
}

router_start() {
  [[ "${MTLS_ROUTER_SKIP_START:-}" != 1 ]] || { info "[启动] 跳过（MTLS_ROUTER_SKIP_START=1）"; return; }
  local response
  response="$(manager_call '{"id":"setup","method":"router.start","params":{"owner":"cli"}}')"
  success "  mtls-router 已启动，监听地址通常为 $ROUTER_BASE_URL"
  info "pid=$(printf '%s' "$response" | jq -r '.result.pid // ""')"
}

router_status() {
  local response state
  response="$(manager_call '{"id":"status","method":"router.status","params":{}}')"
  state="$(printf '%s' "$response" | jq -r .result.state)"
  case "$state" in
    external_compatible|desktop_owned|degraded) success "router running" ;;
    stale|unknown_occupant) warn "router state stale" ;;
    absent) info "router 未运行" ;;
    *) info "router $state" ;;
  esac
  info "pid=$(printf '%s' "$response" | jq -r '.result.pid // ""')"
  info "listen_addr=$(printf '%s' "$response" | jq -r '.result.listen_addr // "127.0.0.1:19099"')"
  [[ ! -f "$STATE_DIR/setup-state.json" ]] || {
    info "binary_path=$(jq -r '.binary_path // ""' "$STATE_DIR/setup-state.json")"
    info "log_path=$(jq -r '.log_path // ""' "$STATE_DIR/setup-state.json")"
  }
}

router_log() {
  local lines="$1" response
  [[ "$lines" =~ ^[0-9]+$ && "$lines" -gt 0 && "$lines" -le 1000 ]] || fail "router log --tail 需要 1-1000 的行数。"
  response="$(manager_call "$(jq -cn --argjson limit "$lines" '{id:"logs",method:"router.logs",params:{limit:$limit}}')")"
  printf '%s' "$response" | jq -r '.result.lines[]?'
}

router_stop() {
  local response
  resolve_manager
  response="$(printf '%s\n' '{"id":"stop","method":"router.stop","params":{}}' | "$RESOLVED_MANAGER" serve --router-sidecar "$RESOLVED_ROUTER")" || fail "mtls-router-manager 执行失败。"
  if [[ "$(printf '%s' "$response" | jq -r '.error.code // empty')" == ROUTER_NOT_FOUND ]]; then
    info "router 未运行"
    return
  fi
  [[ "$(printf '%s' "$response" | jq -r '.error.code // empty')" == "" ]] ||
    fail "manager $(printf '%s' "$response" | jq -r .error.code): $(printf '%s' "$response" | jq -r .error.message)"
  success "router stopped"
}

parse_targets() {
  local filter="$1" detect="$2" token seen=" "
  TARGETS=()
  if [[ -z "$filter" ]]; then
    while IFS= read -r token; do [[ -z "$token" ]] || TARGETS+=("$token"); done < <(printf '%s' "$detect" | jq -r '.result.agents[] | select(.detected) | .agent')
    return
  fi
  IFS=',' read -ra values <<<"$filter"
  for token in "${values[@]}"; do
    token="${token// /}"
    [[ -n "$token" ]] || continue
    [[ "$token" == claude || "$token" == opencode || "$token" == codex ]] || fail "未知 agent: $token"
    [[ "$seen" != *" $token "* ]] || fail "--agent 列表中有重复项：$token"
    printf '%s' "$detect" | jq -e --arg agent "$token" '.result.agents[] | select(.agent==$agent and .detected)' >/dev/null || fail "未检测到 agent: $token"
    TARGETS+=("$token"); seen+="$token "
  done
}

prompt_value() { printf '%s' "$1" >&2; IFS= read -r PROMPT_VALUE || fail "输入已结束。"; }
optional_string() { [[ -z "$1" ]] && printf null || jq -Rn --arg v "$1" '$v'; }
prompt_json_object() {
  prompt_value "$1 [JSON object，留空表示未设置]: "; [[ -z "$PROMPT_VALUE" ]] && { PROMPT_JSON=null; return; }
  PROMPT_JSON="$(printf '%s' "$PROMPT_VALUE" | jq -ce 'if type=="object" then . else error("object") end')" || fail "$1 必须是 JSON object。"
}
prompt_bool() { while true; do prompt_value "$1 [true/false/留空]: "; case "$PROMPT_VALUE" in "") PROMPT_JSON=null; return;; true|false) PROMPT_JSON="$PROMPT_VALUE"; return;; esac; done; }
prompt_int() { prompt_value "$1 [正整数/留空]: "; [[ -z "$PROMPT_VALUE" || "$PROMPT_VALUE" =~ ^[1-9][0-9]*$ ]] || fail "$1 必须是正整数。"; PROMPT_JSON="${PROMPT_VALUE:-null}"; }

build_model_config() {
  local config='{"version":1}' agent model name role answer entry section ids default field context input output value extra='{}'
  for agent in "${TARGETS[@]}"; do case "$agent" in
    claude)
      prompt_value "Claude primary model ID（不会自动选择）: "; model="$PROMPT_VALUE"; [[ -n "$model" ]] || fail "model ID 不能为空。"; prompt_value "Claude primary name [留空]: "; name="$PROMPT_VALUE"
      section="$(jq -cn --arg m "$model" --argjson n "$(optional_string "$name")" '{primary:({model:$m}+if $n==null then {} else {name:$n} end)}')"
      for role in haiku sonnet opus; do prompt_value "Claude $role：输入 inherit 或 model ID: "; answer="$PROMPT_VALUE"; [[ -n "$answer" ]] || fail "$role 不能为空。"; if [[ "$answer" == inherit ]]; then entry='{"inherit_primary":true}'; else prompt_value "Claude $role name [留空]: "; entry="$(jq -cn --arg m "$answer" --argjson n "$(optional_string "$PROMPT_VALUE")" '{model:$m}+if $n==null then {} else {name:$n} end')"; fi; section="$(jq -cn --argjson s "$section" --arg r "$role" --argjson e "$entry" '$s+{($r):$e}')"; done
      extra='{}'; for field in ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION; do prompt_value "Claude $field [留空]: "; [[ -z "$PROMPT_VALUE" ]] || extra="$(jq -cn --argjson e "$extra" --arg f "$field" --arg v "$PROMPT_VALUE" '$e+{($f):$v}')"; done; [[ "$extra" == '{}' ]] || section="$(jq -cn --argjson s "$section" --argjson e "$extra" '$s+{extra:$e}')"; config="$(jq -cn --argjson c "$config" --argjson s "$section" '$c+{claude:$s}')" ;;
    opencode)
      prompt_value "opencode model IDs（逗号分隔，不会自动选择）: "; ids="$PROMPT_VALUE"; [[ -n "$ids" ]] || fail "至少需要一个 model。"; prompt_value "opencode default model ID: "; default="$PROMPT_VALUE"; local models='{}'; IFS=',' read -ra values <<<"$ids"
      for model in "${values[@]}"; do model="${model// /}"; [[ -n "$model" ]] || fail "model ID 不能为空。"; entry='{}'; prompt_value "opencode $model name [留空]: "; [[ -z "$PROMPT_VALUE" ]] || entry="$(jq -cn --argjson e "$entry" --arg v "$PROMPT_VALUE" '$e+{name:$v}')"; for field in reasoning attachment tool_call temperature; do prompt_bool "opencode $model $field"; [[ "$PROMPT_JSON" == null ]] || entry="$(jq -cn --argjson e "$entry" --arg f "$field" --argjson v "$PROMPT_JSON" '$e+{($f):$v}')"; done; prompt_int "opencode $model limit.context"; context="$PROMPT_JSON"; prompt_int "opencode $model limit.input"; input="$PROMPT_JSON"; prompt_int "opencode $model limit.output"; output="$PROMPT_JSON"; if [[ "$context$input$output" != nullnullnull ]]; then [[ "$context" != null && "$output" != null ]] || fail "limit 需要 context 和 output。"; entry="$(jq -cn --argjson e "$entry" --argjson c "$context" --argjson i "$input" --argjson o "$output" '$e+{limit:({context:$c,output:$o}+if $i==null then {} else {input:$i} end)}')"; fi; prompt_json_object "opencode $model modalities（input/output arrays）"; [[ "$PROMPT_JSON" == null ]] || entry="$(jq -cn --argjson e "$entry" --argjson v "$PROMPT_JSON" '$e+{modalities:$v}')"; prompt_value "opencode $model interleaved [true/reasoning/reasoning_content/reasoning_details/留空]: "; value="$PROMPT_VALUE"; [[ -z "$value" ]] || { [[ "$value" == true ]] && PROMPT_JSON=true || PROMPT_JSON="$(jq -cn --arg f "$value" '{field:$f}')"; entry="$(jq -cn --argjson e "$entry" --argjson v "$PROMPT_JSON" '$e+{interleaved:$v}')"; }; prompt_json_object "opencode $model options"; [[ "$PROMPT_JSON" == null ]] || entry="$(jq -cn --argjson e "$entry" --argjson v "$PROMPT_JSON" '$e+{options:$v}')"; prompt_json_object "opencode $model extra"; [[ "$PROMPT_JSON" == null ]] || entry="$(jq -cn --argjson e "$entry" --argjson v "$PROMPT_JSON" '$e+{extra:$v}')"; models="$(jq -cn --argjson m "$models" --arg id "$model" --argjson e "$entry" '$m+{($id):$e}')"; done; config="$(jq -cn --argjson c "$config" --arg d "$default" --argjson m "$models" '$c+{opencode:{default_model:$d,models:$m}}')" ;;
    codex)
      prompt_value "Codex model ID（不会自动选择）: "; model="$PROMPT_VALUE"; [[ -n "$model" ]] || fail "model ID 不能为空。"; section="$(jq -cn --arg m "$model" '{model:$m}')"; for field in reasoning_effort reasoning_summary verbosity; do prompt_value "Codex $field [留空]: "; [[ -z "$PROMPT_VALUE" ]] || section="$(jq -cn --argjson s "$section" --arg f "$field" --arg v "$PROMPT_VALUE" '$s+{($f):$v}')"; done; for field in context_window auto_compact_token_limit; do prompt_int "Codex $field"; [[ "$PROMPT_JSON" == null ]] || section="$(jq -cn --argjson s "$section" --arg f "$field" --argjson v "$PROMPT_JSON" '$s+{($f):$v}')"; done; prompt_value "Codex model_auto_compact_token_limit_scope [total/body_after_prefix/留空]: "; [[ -z "$PROMPT_VALUE" ]] || section="$(jq -cn --argjson s "$section" --arg v "$PROMPT_VALUE" '$s+{extra:{model_auto_compact_token_limit_scope:$v}}')"; config="$(jq -cn --argjson c "$config" --argjson s "$section" '$c+{codex:$s}')" ;;
  esac; done; MODEL_CONFIG="$config"
}

load_model_config() { local path="$1" size; [[ -f "$path" && ! -L "$path" ]] || fail "--model-config 必须是普通文件且不能是符号链接。"; size="$(wc -c <"$path" | tr -d ' ')"; (( size > 0 && size <= MAX_MODEL_CONFIG_SIZE )) || fail "--model-config 必须大于 0 且不超过 2 MiB。"; MODEL_CONFIG="$(jq -ce 'if type=="object" then . else error("object") end' "$path")" || fail "--model-config 必须是有效 JSON object。"; }
show_fragments() { printf '%s' "$1" | jq -r '.result.fragments[] | "### \(.agent) [\(.role)] -> \(.path) (\(.format))\n\(.content)\n"'; }
show_preview() { show_fragments "$1"; printf '%s' "$1" | jq -r '(.result.files[] | "FILE \(.operation): \(.path)"+(if .backup_path then "\n  BACKUP: \(.backup_path)" else "" end)), (.result.managed_collisions[] | "COLLISION \(.agent) \(.path): \(.type) -> \(.action)"), (if .result.state_change then "STATE \(.result.state_change.operation): \(.result.state_change.path)" else empty end), (if .result.state_backup then "STATE BACKUP: \(.result.state_backup.path)" else empty end)'; }

agent_flow() {
  local mode="$1" filter="$2" config_path="$3" detect agents_json models catalog response preview approve_drift=false approve_codex=false
  [[ "$mode" != write || -n "$filter" ]] || fail "--write-config 需要 --agent=claude,opencode,codex 显式指定要操作的 agent。"; detect="$(manager_call '{"id":"detect","method":"agent.detect","params":{}}')"
  if [[ -z "$filter" ]]; then [[ -t 0 ]] || fail "Agent 配置需要交互选择。非交互自动化请使用 manager protocol v2 stdin。"; printf '%s' "$detect" | jq -r '.result.agents[] | select(.detected) | "DETECTED \(.agent): \(.path)"'; prompt_value "选择 Agent（逗号分隔 claude,opencode,codex）: "; filter="$PROMPT_VALUE"; [[ -n "$filter" ]] || fail "必须显式选择至少一个 Agent。"; fi
  parse_targets "$filter" "$detect"; ((${#TARGETS[@]})) || { warn "  未检测到 Agent。"; return; }; agents_json="$(printf '%s\n' "${TARGETS[@]}" | jq -R . | jq -s .)"
  [[ -t 0 ]] || fail "Agent 配置需要隐藏交互输入。非交互自动化请使用 manager protocol v2 stdin：manager.info 握手后调用 agent.models、agent.render/agent.preview、agent.write；key 不得进入参数、环境或文件。MTLS_ROUTER_OPENAI_API_KEY 已移除。"
  printf '请输入 mtls-router OPENAI_API_KEY（输入隐藏）：' >&2; IFS= read -rs TRANSIENT_API_KEY || true; printf '\n' >&2; [[ -n "$TRANSIENT_API_KEY" ]] || fail "API key 不能为空。"
  TRANSIENT_REQUEST="$(jq -cn --argjson a "$agents_json" --arg k "$TRANSIENT_API_KEY" '{id:"models",method:"agent.models",params:{owner:"cli",agents:$a,api_key:$k}}')"; models="$(manager_secret_call "$TRANSIENT_REQUEST")"; TRANSIENT_REQUEST=""; printf '%s' "$models" | jq -r '.result.models[] | "MODEL \(.)"'; catalog="$(printf '%s' "$models" | jq -r .result.catalog_token)"
  if [[ -n "$config_path" ]]; then load_model_config "$config_path"; else build_model_config; fi
  if [[ "$mode" == print ]]; then response="$(manager_call "$(jq -cn --argjson a "$agents_json" --arg t "$catalog" --argjson c "$MODEL_CONFIG" '{id:"render",method:"agent.render",params:{agents:$a,catalog_token:$t,model_config:$c}}')")"; show_fragments "$response"; cleanup_agent_secrets; info "manager 动态渲染完成；未修改 Agent、事务或 sidecar 文件。发现模型可能启动 router 生命周期状态。"; return; fi
  preview="$(manager_call "$(jq -cn --argjson a "$agents_json" --arg t "$catalog" --argjson c "$MODEL_CONFIG" '{id:"preview",method:"agent.preview",params:{agents:$a,catalog_token:$t,model_config:$c}}')")"; show_preview "$preview"
  if [[ "$(printf '%s' "$preview" | jq -r .result.managed_config_drift)" == true ]]; then prompt_value "managed drift：输入 OVERWRITE 批准: "; [[ "$PROMPT_VALUE" == OVERWRITE ]] || fail "已取消。"; approve_drift=true; fi; if [[ "$(printf '%s' "$preview" | jq -r .result.requires_codex_auth_approval)" == true ]]; then prompt_value "Codex auth 将变更并清理冲突认证；输入 CODEX-AUTH 批准: "; [[ "$PROMPT_VALUE" == CODEX-AUTH ]] || fail "已取消。"; approve_codex=true; fi; prompt_value "输入 WRITE 确认写入: "; [[ "$PROMPT_VALUE" == WRITE ]] || fail "已取消。"
  TRANSIENT_REQUEST="$(jq -cn --argjson a "$agents_json" --arg t "$catalog" --argjson c "$MODEL_CONFIG" --arg r "$(printf '%s' "$preview" | jq -r .result.revision_token)" --argjson d "$approve_drift" --argjson x "$approve_codex" --arg k "$TRANSIENT_API_KEY" '{id:"write",method:"agent.write",params:{agents:$a,catalog_token:$t,model_config:$c,revision_token:$r,approve_managed_overwrite:$d,approve_codex_auth_change:$x,api_key:$k}}')"; response="$(manager_secret_call "$TRANSIENT_REQUEST")"; cleanup_agent_secrets
  printf '%s' "$response" | jq -r '(.result.agents[] | .agent as $a | .changed[]? | "WROTE \($a): \(.)"), (.result.agents[].backups[]? | "BACKUP: \(.)")'
}

print_next_steps() {
  success "============================================================"
  success "配置完成。"
  success "============================================================"
  if [[ "${ROUTER_STARTED:-0}" == 1 ]]; then info "mtls-router 已在后台运行："; info "  $ROUTER_BASE_URL"
  else info "未启动 mtls-router（本次仅处理配置）。如需启动，请运行："; info "  $0 router start"; fi
}

print_usage() {
  cat <<'USAGE'
用法: setup.sh [router|agent] <command> [选项]

默认行为等价于 router setup，不修改 Agent 配置。

  router install|start|setup|status|log [--tail=N]|stop
  agent print-config [--agent=claude,opencode,codex] [--model-config=PATH]
  agent write-config --agent=claude,opencode,codex [--model-config=PATH]
  --print-config / --write-config   兼容旧别名

安装选项: --download --download-url=URL --download-user=USER
          --download-password=PASSWORD --version=VERSION

Agent 命令先隐藏读取 key，再发现模型；不会自动选择模型。向导覆盖 Agent-native
typed fields，或读取不超过 2 MiB 的普通 JSON --model-config 文件。
非交互自动化请使用 manager protocol v2 stdin：manager.info 握手后依次调用
agent.models、agent.render/agent.preview、agent.write；key 不得进入参数、环境或文件。
USAGE
}

main() {
  local action=setup agent_filter="" model_config_path="" lines=200
  if (($#)); then
    case "$1" in
      router) (($# >= 2)) || fail "router 需要子命令：install / start / setup / status / log / stop"; action="router-$2"; shift 2 ;;
      agent) (($# >= 2)) || fail "agent 需要子命令：print-config / write-config"; case "$2" in print-config) action=print;; write-config) action=write;; *) fail "未知 agent 子命令：$2";; esac; shift 2 ;;
      --print-config) action=print; shift ;;
      --write-config) action=write; shift ;;
      -h|--help) print_usage; return ;;
      --*) ;;
      *) fail "未知参数：$1（试试 --help）" ;;
    esac
  fi
  case "$action" in router-install|router-start|router-setup|router-status|router-log|router-stop|setup|print|write) ;; *) fail "未知 router 子命令：${action#router-}";; esac
  while (($#)); do
    case "$1" in
      --agent=*) agent_filter="${1#*=}"; shift ;;
      --agent) (($# > 1)) || fail "--agent 需要参数"; agent_filter="$2"; shift 2 ;;
      --model-config=*) model_config_path="${1#*=}"; [[ -n "$model_config_path" ]] || fail "--model-config 需要路径"; shift ;;
      --model-config) (($# > 1)) || fail "--model-config 需要路径"; model_config_path="$2"; shift 2 ;;
      --tail=*) lines="${1#*=}"; shift ;;
      --tail) (($# > 1)) || fail "--tail 需要行数"; lines="$2"; shift 2 ;;
      --download) [[ "$action" == setup || "$action" == router-install || "$action" == router-setup ]] || fail "--download 仅适用于 router install/setup"; ALLOW_DOWNLOAD=1; shift ;;
      --download-url=*) DOWNLOAD_BASE_URL="${1#*=}"; shift ;;
      --download-url) DOWNLOAD_BASE_URL="$2"; shift 2 ;;
      --download-user=*) DOWNLOAD_USER="${1#*=}"; shift ;;
      --download-user) DOWNLOAD_USER="$2"; shift 2 ;;
      --download-password=*) DOWNLOAD_PASSWORD="${1#*=}"; shift ;;
      --download-password) DOWNLOAD_PASSWORD="$2"; shift 2 ;;
      --version=*) MTLS_ROUTER_VERSION="${1#*=}"; shift ;;
      --version) MTLS_ROUTER_VERSION="$2"; shift 2 ;;
      -h|--help) print_usage; return ;;
      *) fail "未知参数：$1（试试 --help）" ;;
    esac
  done
  print_banner
  case "$action" in
    setup|router-setup) install_router_pair; router_start; [[ "${MTLS_ROUTER_SKIP_START:-}" == 1 ]] || ROUTER_STARTED=1; info "提示：未对 agent 配置做任何改动。" ;;
    router-install) install_router_pair; print_next_steps ;;
    router-start) router_start; [[ "${MTLS_ROUTER_SKIP_START:-}" == 1 ]] || ROUTER_STARTED=1; print_next_steps ;;
    router-status) router_status ;;
    router-log) router_log "$lines" ;;
    router-stop) router_stop ;;
    print) agent_flow print "$agent_filter" "$model_config_path" ;;
    write) agent_flow write "$agent_filter" "$model_config_path"; print_next_steps ;;
  esac
}

main "$@"
