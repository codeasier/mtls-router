# mtls-router

[English](../../README.md)

`mtls-router` 是一个单二进制、跨平台的本地反向代理。它接收来自 Claude Code、Codex CLI 等本地客户端的 plain HTTP 请求，然后使用内嵌的客户端证书、私钥、上游 CA 和上游 URL，将请求转发到公开的上游 mTLS 服务。

代理会透明转发请求体和 Server-Sent Events 响应。它不做协议转换：本地流量是 HTTP，上游流量是带 mTLS 的 HTTPS。

## 发布说明

完整更新日志见 [CHANGELOG.md](CHANGELOG.md)。

### v0.1.1

新增一键安装脚本、后台运行模式、日志文件支持、agent 配置向导、Windows 安装体验改进，以及安装脚本测试覆盖。

### v0.1.0

首个发布版本，提供单二进制本地反向代理，用于将本地 HTTP 流量转发到上游 HTTPS mTLS 服务。

## 一键安装

这些脚本会下载适合当前操作系统和 CPU 架构的最新 `mtls-router` 二进制文件，并以后台模式启动 `mtls-router`。脚本不会安装或启动任何 agent，默认安装路径也不会修改 agent 配置。

macOS 或 Linux：

```bash
curl -fsSL https://raw.githubusercontent.com/codeasier/mtls-router/main/setup.sh | bash
```

Windows PowerShell：

```powershell
irm https://raw.githubusercontent.com/codeasier/mtls-router/main/setup.ps1 | iex
```

默认情况下，脚本会把 `mtls-router` 安装到 `~/.local/bin`。在 Windows 上对应 `%USERPROFILE%\.local\bin`（例如 `C:\Users\<你>\.local\bin`）。如需指定其他安装目录，请在运行脚本前设置 `MTLS_ROUTER_INSTALL_DIR`。

## 手动下载

从 GitHub Releases 下载适合当前平台的二进制文件：

```text
https://github.com/codeasier/mtls-router/releases
```

选择对应的 asset：

| 平台 | Asset |
|---|---|
| Linux x86_64 | `mtls-router-linux-amd64` |
| Linux arm64 | `mtls-router-linux-arm64` |
| macOS Intel | `mtls-router-darwin-amd64` |
| macOS Apple Silicon | `mtls-router-darwin-arm64` |
| Windows x86_64 | `mtls-router-windows-amd64.exe` |
| Windows arm64 | `mtls-router-windows-arm64.exe` |

macOS 或 Linux 下，给二进制文件添加执行权限并按需重命名：

```bash
chmod +x ./mtls-router-*
mv ./mtls-router-darwin-arm64 ./mtls-router
```

Windows 下，下载 `.exe` asset，并可在 PowerShell 中重命名：

```powershell
Rename-Item .\mtls-router-windows-amd64.exe mtls-router.exe
```

如果你使用 Windows arm64，请选择 `arm64` asset。

## 在 macOS 或 Linux 上运行

```bash
./mtls-router
```

## 在 Windows 上运行

在包含下载后可执行文件的目录中打开 PowerShell，然后运行：

```powershell
.\mtls-router.exe
```

## 后台运行

`-backend` 会启动 detached 子进程并把控制权返回给 shell。同一份 release 二进制同时支持前台和后台模式，不需要单独的后台版本。

macOS 或 Linux：

```bash
./mtls-router -backend
```

Windows PowerShell：

```powershell
.\mtls-router.exe -backend
```

如果使用 `-backend` 但没有指定 `-log`，日志会写入二进制文件旁边的 `mtls-router.log`。如需显式指定日志文件，请传入 `-log`：

```bash
./mtls-router -backend -log /tmp/mtls-router.log
```

```powershell
.\mtls-router.exe -backend -log C:\mtls-router\mtls-router.log
```

该模式适合本地后台使用。生产环境建议使用 systemd、Docker、launchd 或 Windows service wrapper，以便由平台负责重启和进程管理。

通过一键安装脚本启动时，日志文件不会放在安装目录下：

- macOS / Linux：`~/.mtls-router/mtls-router.log`
- Windows：`%USERPROFILE%\.mtls-router\mtls-router.log`

启动后，路径会写入 `setup-state.json`；也可用 `MTLS_ROUTER_LOG_PATH` 覆盖默认路径。

## 配置 agents

安装脚本会把 router 生命周期命令和 agent 配置命令分开：

```bash
./setup.sh router install
./setup.sh router start
./setup.sh router setup
./setup.sh agent print-config
./setup.sh agent write-config --agent=claude
```

```powershell
.\setup.ps1 router install
.\setup.ps1 router start
.\setup.ps1 router setup
.\setup.ps1 agent print-config
.\setup.ps1 agent write-config --agent=claude
```

`router install` 只下载并安装二进制。`router start` 只启动已安装的二进制；如果不存在，会明确提示先执行 `router install` 或 `router setup`。`router setup` 会安装并启动 router，等价于无参数默认行为。

`agent print-config` 只打印配置片段。`agent write-config --agent=...` 只写入 agent 配置，并要求显式提供 `--agent=`。旧的顶层 `--print-config` 和 `--write-config --agent=...` 仍作为兼容别名保留。

`mtls-router` 二进制本身只管理 router，不提供 `print-config` 这类 agent 配置命令。

- Claude Code 会把 `env` block 写入 `~/.claude/settings.json`，或 `$CLAUDE_CONFIG_DIR/settings.json`。
- opencode 会把 `mtls-router` provider 写入选中的 opencode.json，遵循 `OPENCODE_CONFIG`，否则回退到 `~/.config/opencode/opencode.json`。
- Codex CLI 会把 `[model_providers.custom]` block（带 `model_provider = "custom"` 和 `name = "9router"`）写入 `~/.codex/config.toml`，遵循 `CODEX_HOME`。

安装脚本不会安装任何 agent，也不会启动任何 agent。

默认本地监听地址为：

```text
127.0.0.1:19099
```

本地客户端应指向：

```text
http://127.0.0.1:19099/v1
```

## 配置

配置优先级为：

```text
flag > env > build-time > default
```

| 配置项 | 环境变量 | 参数 | 默认值 |
|---|---|---|---|
| 监听地址 | `MTLS_LISTEN_ADDR` | `-listen` | `127.0.0.1:19099` |
| 上游 URL | `MTLS_UPSTREAM_URL` | `-upstream` | 构建期 `upstreamURL` |
| 最低 TLS 版本 | `MTLS_TLS_MIN` | `-tls-min` | `tls1.2` |
| 非流式超时 | `MTLS_TIMEOUT` | `-timeout` | `0` 表示不超时 |
| Debug body 日志 | `MTLS_DEBUG=1` | `-debug` | 关闭 |
| 后台模式 | `MTLS_BACKEND` | `-backend` | 关闭 |
| 日志文件 | `MTLS_LOG` | `-log` | 前台为 stderr；后台为 `<binary-dir>/mtls-router.log` |

额外参数：

| 参数 | 说明 |
|---|---|
| `-backend` | 启动 detached 后台进程并返回 |
| `-log` | 将日志写入指定文件 |
| `-version` | 打印版本并退出 |
| `-help`, `-h` | 打印帮助并退出 |

示例：

```bash
MTLS_LISTEN_ADDR=127.0.0.1:19099 \
MTLS_TLS_MIN=tls1.3 \
./mtls-router -timeout 10s
```

## 运行时行为

启动时，`mtls-router` 会校验配置，构造 mTLS 上游 transport，并在绑定本地监听前探测上游。如果探测失败，进程会以非零状态退出，避免在上游凭据或路由不可用时继续接受本地流量。

默认本地监听为 `127.0.0.1:19099` 上的 plain HTTP。上游连接使用嵌入的客户端证书和上游 CA 进行 mTLS。

## Streaming 和 SSE

请求体 sniffing 会检测 JSON 请求中是否包含 `"stream": true`，且不会消费或破坏请求体。下游 reader 仍会收到原始字节。

SSE 响应会保留流式行为，并使用适合 SSE 的响应头，包括：

- `Content-Type: text/event-stream`
- `Cache-Control: no-cache`

## 从源码构建

维护者构建和发布说明见 [`BUILD.md`](BUILD.md)。

`-backend` 和 `-log` 是同一份二进制中的运行时参数，因此 release workflow 不需要为后台模式创建额外 asset 或构建步骤。

## 部署

可用部署方式包括 systemd 和 Docker：

- systemd：将二进制复制到 `/usr/local/bin/mtls-router`，安装 `systemd/mtls-router.service`，然后通过 `systemctl` enable 并 start；
- Docker：构建提供的 `Dockerfile`，它会在 `scratch` image 中生成静态二进制；
- bare metal：直接运行 `./mtls-router`。

生产风格的 Windows 服务托管建议使用 NSSM，而不是 `-backend`：

```powershell
nssm install mtls-router
```

在 NSSM service editor 中配置：

- Path：`mtls-router.exe` 的完整路径；
- Startup directory：`mtls-router.exe` 所在目录；
- Arguments：除 `-backend` 之外的 router 参数，例如 `-listen`、`-upstream` 或 `-log`。

不要在 NSSM 下传入 `-backend`，因为 NSSM 会负责后台进程管理。

## 设计

见 `docs/superpowers/specs/2026-06-17-mtls-router-design.md`。

## License

MIT
