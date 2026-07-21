# mtls-router

[English](../../README.md)

`mtls-router` 二进制是一个单二进制、跨平台的本地反向代理。它接收来自 Claude Code、Codex CLI 等本地客户端的 plain HTTP 请求，然后使用内嵌的客户端证书、私钥、上游 CA 和上游 URL，将请求转发到公开的上游 mTLS 服务。本项目还会分发 Go manager 和 Tauri 桌面应用。

代理会透明转发请求体和 Server-Sent Events 响应。它不做协议转换：本地流量是 HTTP，上游流量是带 mTLS 的 HTTPS。

## 发布说明

完整更新日志见 [CHANGELOG.md](CHANGELOG.md)。

### v0.1.1

新增一键安装脚本、后台运行模式、日志文件支持、agent 配置向导、Windows 安装体验改进，以及安装脚本测试覆盖。

### v0.1.0

首个发布版本，提供单二进制本地反向代理，用于将本地 HTTP 流量转发到上游 HTTPS mTLS 服务。

## 桌面应用

Tauri 桌面应用提供当前用户 router 控制、托盘操作、默认登录时启动、健康/日志视图，以及 Claude Code、opencode 和 Codex 的显式预览/写入流程。它把 manager 和 router 作为经过验证的 sidecar 打包，首次启动绝不会修改 Agent 文件。

当前仓库中的 CI 和 release workflow 会构建六个原生桌面包：Windows x86_64/arm64 NSIS 安装器、macOS Intel/Apple Silicon DMG，以及 Linux x86_64/arm64 AppImage。每个匹配的目标 runner 都会检查对应包的内容、架构、版本/deployment 身份、sidecar 哈希和可执行权限。Release job 仅在签名凭据完整时签名 Windows 和 macOS 包，仅在额外 Apple 凭据完整时 notarize 并 staple macOS 应用，并为每个目标发布一个明确的签名状态文件。包检查不会安装或启动应用，因此在把桌面 release 视为完整验证之前，仍需单独提供目标 runner 上成功安装/启动的证据。安装、首次启动、Agent、凭据和卸载行为见[桌面应用](DESKTOP.md)，恢复指导见[桌面应用故障排查](TROUBLESHOOTING.md)，精确证据边界见[构建与发布](BUILD.md)。

## 一键安装

Release 安装优先使用平台包。请从 [GitHub Releases](https://github.com/codeasier/mtls-router/releases) 选择适合当前操作系统和 CPU 架构的压缩包（macOS/Linux 使用 `.tar.gz`，Windows 使用 `.zip`），解压后运行其中的安装脚本。每个压缩包都包含安装脚本、当前平台的 `mtls-router` 和 `mtls-router-manager` 二进制文件，以及两者在 `SHA256SUMS` 中的校验记录。

macOS 或 Linux：

```bash
tar -xzf mtls-router-darwin-arm64.tar.gz
./setup.sh router setup
```

Windows PowerShell：

```powershell
Expand-Archive .\mtls-router-windows-amd64.zip -DestinationPath .\mtls-router
.\mtls-router\setup.ps1 router setup
```

安装脚本会选择适合当前平台的两个同目录二进制文件，并要求它们各自在同目录 `SHA256SUMS` 中存在唯一、完全匹配的校验记录。只要目录里出现任一平台 payload，另一个二进制或 manifest 缺失、校验记录格式错误、重复或哈希不匹配都会导致硬失败：已有的已安装二进制对会被保留，脚本绝不会回退到联网下载。

如果同目录没有任何平台 payload，交互式安装会询问是否下载两个二进制文件和 `SHA256SUMS`。非交互式安装默认安全失败，除非通过 `router install --download`、`router setup --download` 或 `MTLS_ROUTER_ALLOW_DOWNLOAD=1` 显式授权下载。三个文件先下载到同一个临时目录，并在替换任一安装路径前完成两个二进制的 SHA-256 校验。

通过 `--download-url` 或 `MTLS_ROUTER_DOWNLOAD_URL` 指定的自定义来源必须使用 HTTPS；在调用下载器或使用凭据之前，普通 HTTP URL 就会被拒绝。Release 包中的脚本可以预配置私有主机的 HTTPS URL。认证仍需通过 `--download-user` / `--download-password` 或 `MTLS_ROUTER_DOWNLOAD_USER` / `MTLS_ROUTER_DOWNLOAD_PASSWORD` 显式提供；脚本不会内嵌凭据。

默认情况下，脚本会把 `mtls-router` 和 `mtls-router-manager` 一起安装到 `~/.local/bin`。在 Windows 上对应 `%USERPROFILE%\.local\bin`（例如 `C:\Users\<你>\.local\bin`）。如需指定其他安装目录，请在运行脚本前设置 `MTLS_ROUTER_INSTALL_DIR`。安装过程使用仅当前用户可读写的 pending marker、上一代备份、两个固定路径替换、安装后哈希验证和原子提交的 receipt。每个 setup 命令在执行已安装二进制前都会协调未完成事务，因此不会执行混合代。脚本不会安装或启动任何 agent，默认安装路径也不会修改 agent 配置。

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

安装脚本管理的 `router status` 和 `router stop` 不会只信任 PID。它们会同时校验 PID、记录的操作系统进程启动标识和可执行文件路径，并确认可执行文件与受管理的二进制一致。标识缺失或不匹配会报告为 stale state（陈旧状态）；该状态会保留用于诊断，`router stop` 不会向对应进程发送信号。停止期间以及强制终止前还会再次校验完整标识，避免向 PID 复用后的其他进程发送信号。

旧版安装脚本创建的状态文件不包含这些进程标识。因此升级安装脚本后，旧版已运行的 router 会被报告为 stale，且不会被自动停止。请先人工确认并停止该进程，删除旧的 `setup-state.json`，再运行 `router start` 创建包含进程标识的新状态。

## 管理端点

`mtls-router` 在反向代理的同一个 listener 上提供两个管理端点，它们**不会**被转发到上游。

这些端点假设 router 只监听可信 localhost。不要在生产环境中把它们暴露到公网，因为 `/version` 包含 commit SHA 等精确构建信息。

### `GET /version`

返回描述当前二进制和进程的 JSON：

```json
{
  "version": "v0.1.1",
  "commit": "abc1234",
  "build_date": "2026-06-21T09:23:24Z",
  "deployment_id": "production-service",
  "management_protocol_version": "2",
  "pid": 12345,
  "started_at": "2026-06-21T09:23:24Z"
}
```

`version`、`commit`、`build_date` 和 `deployment_id` 会通过 `.github/workflows/release.yml`、`Dockerfile` 和 `scripts/build.sh` 中的 `-ldflags -X` 在 link time 设置。`management_protocol_version` 是代码内兼容性 ID。本地构建默认使用 `dev` / `unknown`；生产 release preflight 要求非默认 deployment ID。`started_at` 是当前进程的启动时间。

### `GET /health`

返回 HTTP 200 和描述上游 mTLS+TCP 可达性的 JSON。HTTP status 始终为 200，由响应体区分 `ok` 和 `degraded`：

```json
{"status": "ok", "upstream": "reachable"}
```

```json
{"status": "degraded", "upstream": "unreachable", "error": "..."}
```

安装脚本使用 `/version` 和 `/health` 检测端口 19099 上是否已有以前安装的 router，并决定升级、重启还是保持不变。

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

`agent print-config` 和 `agent write-config --agent=...` 都会先隐藏读取 key，再发现 manager 经过认证和构建过滤的 `GET /v1/models` 目录。Manager 默认排除包含 ASCII `/` 的有效 ID；使用 `SIMPLIFY=False` 构建的 release 会保留它们。这个不可变的 manager 构建策略控制配置选择和刷新校验；它不是运行时偏好，也不限制下文列出的 proxy 路由。命令绝不会选择目录中的第一个模型、按模型名称或能力推断选择，也不会替换为另一个模型。Release 可以提供可见、可编辑的 preset，但只有在 manager 根据该过滤目录验证某个 Agent section 的全部精确模型 ID 后，才会提供该 section；否则整个 section 不可用，且不会选择替代项。每个 Agent 的初始化优先级为 `existing > preset > empty`。Print 返回 manager 动态渲染且 API-key 脱敏的托管片段；write 显示精确预览，并在一次事务写入前立即重新验证目录。可添加 `--model-config=<path>`，使用无 key 的规范 JSON 选择替代模型问答；该显式导入会完整替换全部生成的默认值。旧的顶层 `--print-config` 和 `--write-config --agent=...` 仍作为同一 v2 流程的兼容别名。Agent 命令只会执行经 checksum 验证的同目录 manager，或经安装 receipt 验证的 manager；绝不会隐式下载 manager。

由于环境变量不是安全的 secret 传输方式，`MTLS_ROUTER_OPENAI_API_KEY` 已移除，不再提供 key。非交互自动化必须验证 `manager.info` protocol `2`，调用 `agent.models`，构造规范 model config，再调用 `agent.render` 或 `agent.preview` 与 `agent.write`。Key 只出现在 `agent.models` 和 `agent.write` stdin 请求体中。不要把 key 放入命令行参数、环境变量、model config、日志、shell history 或临时请求文件。完整契约见 [Agent 模型配置](AGENT_MODELS.md#protocol-v2-自动化)。

`mtls-router` 二进制本身只管理 router，不提供 `print-config` 这类 agent 配置命令。

- Claude Code 只把受管理的 `env` key 合并到 `~/.claude/settings.json` 或 `$CLAUDE_CONFIG_DIR/settings.json`，并支持主模型及可继承的 Haiku、Sonnet、Opus 选择。Fable 是可选的：启用后可继承 primary，或显式选择模型、显示名称及 Standard/1M context；省略时不会隐式添加或管理。启用 Fable 会渲染 `ANTHROPIC_DEFAULT_FABLE_MODEL`，设置名称时还会渲染 `ANTHROPIC_DEFAULT_FABLE_MODEL_NAME`。Claude preset 与 existing 初始化始终以完整 section 为原子单位，因此绝不会把 preset Fable 合并进已有 Claude section。Fable 禁用时，manager 会保留从未取得所有权的手工 Fable key，只删除 sidecar 能证明之前由 manager 所有的 stale Fable 路径；认领已有未托管值前必须经过 collision/drift 批准。Fable alias 要求 Claude Code 2.1.170 或更高版本。与此独立，数值 custom-model context override 从 Claude Code 2.1.193 起可直接作用于未知模型名称；更早版本可能忽略这些数值 override。每个显式选择都可设置显示名称和可选规范字段 `context: "1m"`；manager 始终以认证目录中的 base model ID 作为规范身份，只在渲染 Claude 模型环境变量时追加 `[1m]`。它不会推断 1M 能力；如果 Claude 或上游在运行时拒绝，也不会 fallback。
- opencode 会把精确选择的目录子集写入 `provider.mtls-router`，并写入由 manager 拥有的根默认模型。不设置显式 `OPENCODE_CONFIG` 时，已有的标准 `~/.config/opencode/opencode.jsonc` 会迁移到同目录的 `opencode.json`；显式指定 `.jsonc` 覆盖路径时，则会在该精确路径原地规范化。两种操作都会丢失注释和格式。
- Codex 会把专用 `[model_providers.mtls-router]` Responses provider 和选中的 typed model setting 写入 `~/.codex/config.toml`，遵循 `CODEX_HOME`。把 CLI/IDE 共享认证切换为官方 file-backed API-key 模式需要单独预览批准。

Manager 目录中保留的每个模型都视为支持 Claude Messages 与 token counting、opencode Chat Completions、兼容 Completions 和 Codex Responses，包括流式响应。`configured` 检测只表示本地托管结构完整；当前授权由模型发现和写入时刷新证明。未设置的可选模型字段保持省略。发现失败、目录 stale、模型消失、漂移或所有权状态无效都会安全失败，不使用静态/cache fallback，也不产生部分变更。无关设置会保留；托管漂移需要批准，备份可能含 key，必须妥善保护。规范 schema、选项、刷新、失败、迁移、所有权和备份契约见 [Agent 模型配置](AGENT_MODELS.md)。

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

上游 URL 必须使用 HTTPS。普通 HTTP 上游无法提供 mTLS，并会在没有传输加密的情况下发送请求，因此会被拒绝。

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

请求体会直接流式转发到上游 —— 路由器不会对请求体做任何缓冲。识别为 Server-Sent Events 的响应会带上适合 SSE 的响应头，包括：

- `Content-Type: text/event-stream`
- `Cache-Control: no-cache`

反向代理设置了 `FlushInterval: -1`，上游字节会立即 flush 到本地客户端。

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

见[设计规范](../superpowers/specs/2026-06-17-mtls-router-design.md)。

## License

MIT
