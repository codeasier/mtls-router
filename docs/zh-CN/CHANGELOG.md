# 更新日志

[English](../CHANGELOG.md)

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
