# 更新日志

[English](../CHANGELOG.md)

## v0.1.3 - 2026-07-10

本次发布重点提升 release 安装可靠性和代理行为正确性。打包后的安装脚本默认指向可访问的下载服务器；代理移除了不再使用的流式检测预读路径；客户端请求体读取失败会正确返回 bad request；`/health` 现在会按运行时配置探测实际上游目标。

### 变更

- 简化代理请求处理，移除未使用的流式检测预读路径，同时保留反向代理直接流式转发行为。
- 调整 release 打包安装脚本默认值，使安装器默认从配置的发布服务器地址下载二进制文件。

### 修复

- 修复打包安装脚本使用下载服务器直连 IP，避免部分环境下域名不可达导致安装失败。
- 修复客户端请求体读取失败的错误分类，使其返回 `400 Bad Request`，不再误归类为上游代理失败。
- 修复 `/health` 未传入运行时探测配置的问题，使健康检查指向当前配置的上游目标。

### 测试

- 新增客户端请求体读取错误分类和 health probe 配置传递的回归覆盖。
- 保持安装脚本测试覆盖与 release 下载默认值、router health 行为同步。

---

## v0.1.2 - 2026-06-21

### 新增

- 新增拆分后的安装入口命令，分别用于 router 安装和 agent 配置。
- 新增已有 opencode JSONC 配置的交互式迁移流程。
- 新增安装流程中的 router 生命周期管理命令，支持启动、停止、重启和状态类操作。
- 新增全构建产物统一注入的构建元信息：
  - version
  - commit
  - build date
- 新增内部构建信息端点：`/internal/version`。
- 新增 router listener 管理端点：
  - `/version`
  - `/health`

### 变更

- 调整 Codex CLI 安装配置，改为使用最小化 custom provider 配置。
- 更新 opencode 安装配置，使用 `/v1` base URL，并适配 JSONC 配置格式。
- 调整 Claude 和 opencode 目标的模型 ID，去掉 `cx/` 前缀。
- 更新 README，使其与当前安装脚本默认行为一致，并补充管理端点说明。

### 修复

- 修复安装流程在仅写入配置时意外启动 router 的问题。
- 修复默认日志位置，避免将 setup 日志写入安装目录。
- 修复 Windows 下 router 启动行为。
- 修复 Windows PowerShell 脚本编码和 JSON 解析行为。
- 修复 Windows 下 Codex CLI 配置匹配和认证文件生成问题，包括无 BOM 的 `auth.json` 输出。
- 加强安装脚本中 router 生命周期命令的稳定性。

### 测试

- 更新非交互式安装参数流程的 shell 测试。
- 扩展 PowerShell JSON 处理和生命周期相关行为的安装脚本测试覆盖。

---

## v0.1.1 - 2026-06-19

### 新增

- 新增 macOS/Linux 和 Windows 一键安装脚本：
  - `setup.sh`
  - `setup.ps1`
- 安装脚本支持自动下载当前平台和 CPU 架构对应的最新 release 二进制文件。
- 新增交互式配置向导，支持配置以下本地 agent：
  - Claude Code
  - opencode
  - Codex CLI
- 修改 agent 配置前会自动备份已有配置文件。
- 新增后台运行模式：`-backend`。
- 新增日志文件参数：`-log`。
- 新增后台模式和日志路径环境变量：
  - `MTLS_BACKEND`
  - `MTLS_LOG`
- 新增跨平台后台进程启动支持：
  - Unix/macOS/Linux 使用独立进程会话
  - Windows 使用 detached process 创建方式
- 新增安装脚本测试套件。
- 新增 PowerShell 安装流程测试。
- 新增 `make test-shell`，用于运行 shell 安装脚本测试。
- 新增维护者构建和发布文档：`docs/BUILD.md`。

### 变更

- 更新 README，补充一键安装、手动下载、Windows 使用、后台模式和 agent 配置说明。
- 将详细构建和发布说明从 README 移动到 `docs/BUILD.md`。
- CI 新增 shell 安装脚本测试。
- 改进 meta flag 处理，使运行时参数可以正确透传。
- 补充 Windows release 使用方式和生产环境服务托管建议。

### 修复

- 修复 `-backend`、`-log` 等运行时参数的解析问题。
- 修复 Windows 安装向导行为，使其与 Unix 安装流程更一致。
- 修复 Windows 下 agent 配置行为。
- 收紧后台启动集成和日志文件处理。

### 测试

- 新增后台参数处理和日志行为的 Go 测试。
- 新增配置字段的 Go 测试。
- 新增 `-version`、`-help` 和运行时参数处理测试。
- 新增安装脚本测试，覆盖 clean setup、latest version detection、target selection、Claude Code、opencode、Codex CLI 和 PowerShell 流程。

---

## v0.1.0 - 2026-06-18

### 新增

- `mtls-router` 首个发布版本。
- 新增单二进制本地反向代理，用于将本地 plain HTTP 流量转发到上游 HTTPS mTLS 服务。
- 新增通过构建期 linker variables 嵌入证书和配置的能力：
  - `main.clientCertPEM`
  - `main.clientKeyPEM`
  - `main.upstreamCAPEM`
  - `main.upstreamURL`
  - `main.version`
- 新增本地 HTTP 监听，默认地址为 `127.0.0.1:19099`。
- 新增基于嵌入式客户端证书、私钥和上游 CA 的 mTLS transport。
- 启动时会先探测上游健康状态，成功后才开始接受本地流量。
- 新增请求转发到配置的上游 URL。
- 新增透明请求体流式转发。
- 新增 Server-Sent Events 响应处理，并保留适合流式传输的响应头。
- 新增对包含 `"stream": true` 的 JSON 请求的流式请求检测。
- 新增结构化访问日志。
- 新增 `SIGINT` 和 `SIGTERM` 优雅退出。
- 支持通过参数、环境变量、构建期值和默认值进行运行时配置。
- 新增本地开发构建脚本：`scripts/build.sh`。
- 本地开发构建支持自动生成占位证书文件。
- 新增 GitHub Actions CI workflow。
- 新增 GitHub Actions release workflow。
- 新增 Linux、macOS、Windows 的 amd64 和 arm64 release 交叉编译。
- 新增 Docker 支持：`Dockerfile`。
- 新增 systemd 服务单元：`systemd/mtls-router.service`。
- 新增 README、设计文档、实现计划文档和 MIT license。

### 配置

- 新增配置优先级：参数 > 环境变量 > 构建期值 > 默认值。
- 新增环境变量：
  - `MTLS_LISTEN_ADDR`
  - `MTLS_UPSTREAM_URL`
  - `MTLS_TLS_MIN`
  - `MTLS_TIMEOUT`
  - `MTLS_DEBUG`
- 新增运行时参数：
  - `-listen`
  - `-upstream`
  - `-tls-min`
  - `-timeout`
  - `-debug`
  - `-version`
  - `-help`
  - `-h`

### 测试

- 新增证书加载与校验单元测试。
- 新增配置加载与校验单元测试。
- 新增上游健康探测单元测试。
- 新增日志辅助函数单元测试。
- 新增反向代理 director、错误处理、响应修改、流式检测和 mTLS transport setup 相关单元测试。
