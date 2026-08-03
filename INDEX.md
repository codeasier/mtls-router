# mtls-router

面向 AI 编程 Agent（Claude Code、Codex、opencode）的 Go 1.26 本地反向代理系统。本地客户端走明文 HTTP；上游流量走 HTTPS 并内嵌 mTLS 凭证。系统分三层：

1. **mtls-router** — 单二进制反向代理，支持 SSE 流式、健康探测、后台模式
2. **mtls-router-manager** — 基于 stdin/stdout JSON 协议的控制面，负责路由生命周期与 Agent 配置
3. **Tauri 桌面应用** — React + Rust GUI，以 sidecar 方式拉起 manager 并通过 JSON 协议通信

> 工作流偏好（本地开发 / 测试 / 提交 / 发布 / 文档规范）见 [AGENTS.md](AGENTS.md)。本 INDEX.md 仅描述项目理解与架构。

## 三层数据流

```
Tauri UI (React) ──invoke──▶ Rust commands ──stdin/stdout JSON──▶ mtls-router-manager ──spawn/HTTP──▶ mtls-router ──mTLS──▶ upstream
```

桌面应用绝不直接与 router 通信。它以长驻子进程方式拉起 `mtls-router-manager serve`，通过 stdin/stdout 交换换行分隔的 JSON 请求/响应。

## Router（`main.go` + `internal/`）

`run()` 编排流程：meta flags → config.Load（flag > env > build-time > default）→ mTLS transport → upstream probe → reverse proxy + mux → graceful shutdown。

关键不变量：

- `/version` 与 `/health` 以精确 pattern 注册在与反向代理同一个 mux 上；ServeMux 按最具体 pattern 匹配，与注册顺序无关，因此二者永远不会被转发到 upstream
- `/health` 永远返回 HTTP 200；降级信息放在 JSON body 中
- 反向代理的 `FlushInterval: -1` 启用无缓冲 SSE 流式
- mTLS 凭证为链接期变量（`main.clientCertPEM`、`main.clientKeyPEM`、`main.upstreamCAPEM`、`main.upstreamURL`），通过 `-ldflags -X` 注入
- 启动探针失败即非零退出；router 绝不在 upstream 异常时接收流量

## Manager（`cmd/mtls-router-manager/` + `internal/manager/`）

Manager 是一个基本无状态、按请求处理的 management protocol v4 JSON 服务（`internal/manager/protocol/`）；一个例外是 `occupant.Service`，它只为允许强制终止的目标在 `Inspect` 与 `ForceTerminate` 之间持有一个内存中一次性确认 token（30 秒后过期；其他长生命周期状态位于 `lifecycle.Manager` 与 `agent.Service`）。它暴露 17 个方法，分组如下：

- `manager.info`、`diagnostics.collect` — 元数据
- `router.status/start/stop/health/version/logs` — 路由生命周期（spawn/监控 router 二进制）
- `router.inspect_occupant/force_terminate_occupant` — 端口冲突解决
- `agent.detect/models/render/preview/write`、`agent.cleanup.preview/write` — Agent 配置及按单 Agent 清理（Claude Code、opencode、Codex）

子包职责：

- `app` — 装配所有服务，映射协议错误，强制 API key 清零，并把无 key cleanup 请求直接分发到 Agent service
- `lifecycle` — 进程 spawn、状态文件、父进程监控、异常退出检测
- `discovery` — 分类 router 状态（desktop_owned / external_compatible / degraded / stale / absent）
- `agent` — 检测、配置渲染（按 agent 格式：JSON/TOML）、基于 sidecar 所有权的清理、支持 replace/delete 与备份/回滚的事务性写入
- `agent/modelconfig` — 无 key 的规范化 model config schema v1：`Decode`/`DecodeStructural`/`Canonical`/`DeepMerge`、目录/写入/cleanup token 签名
- `trustedrouter` — 经 router `/v1/models` 的鉴权模型目录发现
- `occupant` — 结构化端口占用诊断、Windows 权限预检、SCM/systemd supervisor 分类与受保护的精确强制终止
- `protocol` — 请求/响应类型、方法超时、错误码
- `state` — router 进程身份的 JSON 状态文件读写
- `process` — PID + 启动时间 + 可执行文件三元身份校验
- `preset` — 加载经 `-ldflags -X` 注入的不可变 Agent model preset（base64）
- `modelcatalog` — 模型目录 HTTP 客户端与 simplify 过滤策略（链接期 `Simplify` 变量）
- `metadata` — manager 握手信息与生产身份校验
- `paths` — 跨平台按用户路径解析（CLI 状态目录 + 桌面数据目录）

### API key 处理（实现细节）

内存与无 key 数据：

- Go manager 在成功 decode 后将 `request.APIKey = ""`（尽力而为；底层 JSON/Scanner 缓冲区由 GC 管理，不保证清零）。
- Rust 桌面端把全局 key 持久化到私有 `credentials.json`，按需以 `Zeroizing<String>` 加载；`ModelFlow` 不含 key，cleanup command 也不读取凭据。
- `modelconfig`（schema v1）是无 key 的设计硬约束，主动拒绝 key-like 字段名；sidecar 状态文件、事务 journal 与 cleanup revision claim 只存 HMAC 摘要和无密钥声明，不含 key。

含 key 的持久化（Agent 凭据文件）：

- `agent.write` 会把 key **明文**写入目标 Agent 的凭据文件：Claude `~/.claude/settings.json`（`env.ANTHROPIC_AUTH_TOKEN`）、opencode `~/.config/opencode/opencode.json`（`provider.mtls-router.options.apiKey`）、Codex `~/.codex/auth.json`（`OPENAI_API_KEY`）；Codex `config.toml` 不含 key。
- 路径解析遵循 setup 脚本语义（`internal/manager/agent/paths.go`）：`CLAUDE_CONFIG_DIR`、`OPENCODE_CONFIG`、`CODEX_HOME` 可覆盖默认位置；opencode 按 JSON-before-JSONC 回退，`opencode.json` 不存在而 `opencode.jsonc` 存在时选中后者。两种 JSONC 情形写入时都会丢失注释与格式（预览阶段给出警告）：默认路径下 `opencode.jsonc` 被**迁移为 `opencode.json`**；`OPENCODE_CONFIG` 指向 JSONC 时则**就地规范化为严格 JSON**。
- 写入使用权限受限的原子化临时文件（`os.CreateTemp` + `restrictPrivate`，Unix `0o600` / Windows DACL 仅当前用户），随后 `replaceAtomic` 将临时文件原子重命名为目标文件（替换前的原始字节进入事务备份）；defer 中的 `os.Remove(tmpPath)` 仅在 replace 失败的路径上清理临时文件，成功后该路径已不存在。
- 事务备份（`*.bak-*`、`*.rollback-*`）与源文件同目录、权限受限（`0o600`/DACL），内容为目标文件替换前的原始字节，可能含旧 key。

key 绝不出现于环境变量、CLI 参数、model config、日志或 journal 中。

## 桌面应用（`desktop/`）

完整文件映射见 [desktop/INDEX.md](desktop/INDEX.md)；下面只列主干。

**前端**（React 19 + TypeScript + Vite）：

- `src/ipc.ts` — 类型化的 `DesktopApi` 接口，包装 Tauri invoke 命令与 updater 下载进度事件；所有敏感文本在客户端脱敏
- `src/App.tsx` — 根布局与侧边栏导航，分发到 4 个页面组件，并在启动时执行一次静默更新检查
- `src/RouterPage.tsx`、`src/AgentPage.tsx`、`src/LogsPage.tsx`、`src/SettingsPage.tsx` — 各区块页面；Agent 页面协调独立配置与 cleanup 目标
- `src/AgentCleanupPanel.tsx`、`src/agentCleanupState.ts`、`src/useAgentCleanupController.ts` — 单 Agent cleanup 审阅、状态机与无 key preview/write 编排
- `src/model.ts` — 共享类型与导航模型
- i18n：`src/i18n.tsx`（context provider）+ `src/locales/zh-CN.ts`、`src/locales/en.ts`

**后端**（Rust，Tauri 2）：

- `src/lib.rs` — 应用入口：插件注册、setup、invoke handler 注册
- `src/commands.rs` — Tauri 命令处理器，代理到 manager client；`AppState`、`ModelFlow`，以及不接收凭据/model flow 的 cleanup preview/write command
- `src/manager.rs` — spawn 并通过 stdin/stdout 与 `mtls-router-manager serve` 通信；握手校验；cleanup write 禁止不确定投递后的自动 replay
- `src/scheduler.rs` — 轮询调度器，向前端 emit `router-poll-snapshot` 事件
- `src/port_recovery.rs` — manager 报告首次释放后约 10 秒定期采样，区分未检测到重新占用与已采样到重新占用
- `src/updater.rs` — stable-only 桌面整包检查/安装：有限网络超时、Tauri 签名下载、desktop-owned router 停止与失败恢复、安装后重启
- `src/sidecar.rs` — 解析并校验 sidecar 二进制路径（运行时纯名字 `mtls-router[-manager][.exe]`；target-triple 名仅为构建输入）
- `src/tray.rs` — 系统托盘，状态感知菜单
- `src/orchestration.rs` — 首次启动流程（sidecar 有效则自动启动 router）
- `src/model_config.rs` — model config 导入/导出校验
- `src/autostart.rs` — 登录启动插件包装（首次启动默认启用）
- `src/paths.rs` — 桌面数据目录解析
- `src/process_identity.rs` — 捕获 PID + 启动时间 + 可执行文件，用于父身份 flag
- `src/types.rs` — 镜像 manager 协议结果的严格 serde 类型，包含 cleanup detection、preview 与 delete/backup 文件影响
- `src/error.rs` — 将 manager 协议错误映射为用户可见字符串

Rust 侧绝不向 webview 暴露 shell/fs/http 权限（由 `lib.rs` 中的测试强制保证）。

桌面在线更新仅由精确 stable `vX.Y.Z` release 启用，固定检查 `https://github.com/codeasier/mtls-router/releases/latest/download/latest.json`，`latest.json` 中各平台产物 URL 指向 `https://github.com/codeasier/mtls-router/releases/download/<tag>/`。更新包包含桌面应用及匹配的 manager/router sidecar，必须通过独立 Tauri updater 签名校验并经用户确认后安装；该能力不改变 CLI router、manager 或 setup 脚本的更新行为。

## Setup 脚本（`setup.sh` / `setup.ps1`）

路由生命周期（`router install/start/stop/status/setup`）与 Agent 配置（`agent print-config/write-config`）是有意分离的两个命令组。脚本同时安装 `mtls-router` 与 `mtls-router-manager`，按 `SHA256SUMS` 校验 SHA-256，并使用带 pending 标记的事务性安装。

## 构建元数据

通过 `-ldflags -X` 注入的链接期变量：

- `main.clientCertPEM`、`main.clientKeyPEM`、`main.upstreamCAPEM`、`main.upstreamURL`（仅 router）
- `github.com/codeasier/mtls-router/internal/version.Version/Commit/BuildDate/DeploymentID`（两个二进制都含）
- `github.com/codeasier/mtls-router/internal/manager/preset.Encoded`（仅 manager，base64 agent model preset）
- `github.com/codeasier/mtls-router/internal/manager/modelcatalog.Simplify`（仅 manager，模型过滤策略）

## 包索引

下表列出的包/模块均有专属 `INDEX.md`，含详细的文件映射、导出、不变量与依赖。在该区域工作时请先读对应的 INDEX.md。

| 包                    | 索引                                     | 范围                                                                                                                                 |
| --------------------- | ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `internal/proxy`      | [INDEX.md](internal/proxy/INDEX.md)      | 反向代理、mTLS transport、SSE 流式、错误脱敏                                                                                         |
| `internal/background` | [INDEX.md](internal/background/INDEX.md) | 分离子进程、日志文件、参数改写                                                                                                       |
| `internal/config`     | [INDEX.md](internal/config/INDEX.md)     | flag/env/build-time 优先级与校验                                                                                                     |
| `internal/health`     | [INDEX.md](internal/health/INDEX.md)     | upstream mTLS 可达性探针                                                                                                             |
| `internal/routermeta` | [INDEX.md](internal/routermeta/INDEX.md) | `/version` 与 `/health` handler                                                                                                      |
| `internal/certs`      | [INDEX.md](internal/certs/INDEX.md)      | PEM 解析为 client cert + CA pool                                                                                                     |
| `internal/version`    | [INDEX.md](internal/version/INDEX.md)    | 链接期构建元数据变量                                                                                                                 |
| `internal/log`        | [INDEX.md](internal/log/INDEX.md)        | 访问日志响应记录器                                                                                                                   |
| `internal/tlspolicy`  | [INDEX.md](internal/tlspolicy/INDEX.md)  | TLS 最低版本解析                                                                                                                     |
| `internal/manager`    | [INDEX.md](internal/manager/INDEX.md)    | 控制面：17 个协议方法、生命周期、发现、Agent 配置与清理。其 14 个子包各有专属 INDEX，导航见 [子包表](internal/manager/INDEX.md#子包) |
| `desktop`             | [INDEX.md](desktop/INDEX.md)             | Tauri 2 应用：React 前端 + Rust 后端、sidecar 管理                                                                                   |

## 辅助参考

- `scripts/build.sh` 在 `secrets/` 下生成占位 PEM 供本地构建；真实发布密钥来自 GitHub secrets/vars。
- `desktop/scripts/prepare-updater-config.sh` 为 stable tag 生成私有 Tauri updater overlay 并校验固定公钥指纹；`desktop/scripts/updater-public-key-fingerprint.mjs` 生成该指纹；`desktop/scripts/create-macos-updater.sh` 从最终 macOS app 生成签名 `.app.tar.gz`；`desktop/scripts/verify-package.sh` 收集六平台 updater 产物及 `.sig` 并验证签名与公钥匹配。
- `scripts/package-release.sh` 对精确 stable tag 汇总六平台 updater artifact/signature、生成 `latest.json`（平台 URL 指向 GitHub releases）并纳入 `SHA256SUMS`；release/recovery workflow 另将 tag 目录镜像至 `downloads.codeasier.top` 并单调、原子推进其 `latest` symlink 作为二级分发点，但 updater feed 与产物下载均以 GitHub releases 为准。Updater 签名密钥与 Windows/macOS 平台签名凭据属于独立信任链。
- `.worktrees/` 目录含 git worktree 产物，已在 `.gitignore` 中忽略；分析产品代码时忽略。
- 管理协议当前版本为 `4`；router、manager、setup receipt、release metadata 与桌面端必须同版本，桌面端在启动握手时校验并拒绝混合代。
