# mtls-router

面向 AI 编程 Agent（Claude Code、Codex、opencode）的本地反向代理系统。本地客户端走明文 HTTP；上游流量走 HTTPS 并内嵌 mTLS 凭证。当前交付物是 **Tauri 桌面应用**（React + Rust）：Rust `router_core` 与 `manager_core` 在桌面进程内提供反向代理与 protocol v4 控制面。仓库仍保留三层历史实现作为冻结参考：

1. **mtls-router**（Go，冻结）— 单二进制反向代理，支持 SSE 流式、健康探测、后台模式
2. **mtls-router-manager**（Go，冻结）— 基于 stdin/stdout JSON 协议的控制面，负责路由生命周期与 Agent 配置
3. **Tauri 桌面应用** — 内嵌 Rust router 与 manager；`v0.4.1` 之后的 release 只发布桌面包

> 工作流偏好（本地开发 / 测试 / 提交 / 发布 / 文档规范）见 [AGENTS.md](AGENTS.md)。本 INDEX.md 仅描述项目理解与架构。

## 数据流

```
Tauri UI (React) ──invoke──▶ Rust commands ──protocol v4 JSON 行（进程内 ManagerClient）──▶ manager_core ──supervisor 线程──▶ router_core ──mTLS──▶ upstream
```

桌面应用绝不直接与 router 通信：命令层只通过 `ManagerClient` 发 protocol v4 请求，由同进程的 `manager_core` session 线程处理；router 由 `router_core` supervisor 在独立 runtime 线程上运行。正常路径不启动 Go 子进程。历史拓扑（Tauri 拉起 `mtls-router-manager serve` sidecar，再由其管理 `mtls-router` 子进程）对应 `v0.4.1` 及更早版本；桌面会识别该拓扑遗留的 router 并在完整身份校验后一次性迁移。

## CLI 停止维护（`v0.4.1` 之后）

新 release 不再构建或发布 `mtls-router`、`mtls-router-manager`、setup 脚本、CLI 归档与 systemd/Docker/NSSM 包装；`scripts/package-release.sh` 以 allowlist 拒绝它们。Go 源码、`scripts/build.sh`、`setup.*`、`systemd/`、`Dockerfile` 与全部 Go 测试保留在仓库中：它们是内嵌 Rust 实现的对照契约（HTTP 黑盒、protocol v4、状态文件、occupant、legacy 迁移 golden），CI 继续运行 `go test ./...` 与原生 occupant 测试。下文 Router / Manager / Setup 章节描述的是这份冻结实现及其不变量，Rust 实现按同一契约移植。

## Router（`main.go` + `internal/`，冻结）

`run()` 编排流程：meta flags → config.Load（flag > env > build-time > default）→ mTLS transport → upstream probe → reverse proxy + mux → graceful shutdown。

关键不变量：

- `/version` 与 `/health` 以精确 pattern 注册在与反向代理同一个 mux 上；ServeMux 按最具体 pattern 匹配，与注册顺序无关，因此二者永远不会被转发到 upstream
- `/health` 永远返回 HTTP 200；降级信息放在 JSON body 中
- 反向代理的 `FlushInterval: -1` 启用无缓冲 SSE 流式
- mTLS 凭证为链接期变量（`main.clientCertPEM`、`main.clientKeyPEM`、`main.upstreamCAPEM`、`main.upstreamURL`），通过 `-ldflags -X` 注入
- 启动探针失败即非零退出；router 绝不在 upstream 异常时接收流量
- 致命启动日志只输出封闭的安全原因码（配置、TLS 凭据、探针、监听等），不输出原始 transport/URL/证书细节

## Manager（`cmd/mtls-router-manager/` + `internal/manager/`）

Manager 是一个基本无状态、按请求处理的 management protocol v4 JSON 服务（`internal/manager/protocol/`）；一个例外是 `occupant.Service`，它只为允许强制终止的目标在 `Inspect` 与 `ForceTerminate` 之间持有一个内存中一次性确认 token（30 秒后过期；其他长生命周期状态位于 `lifecycle.Manager` 与 `agent.Service`）。它暴露 19 个方法，分组如下：

- `manager.info`、`diagnostics.collect` — 元数据
- `router.status/start/migrate_legacy/stop/health/version/logs` — 路由生命周期（spawn/监控 router 二进制）
- `router.inspect_occupant/force_terminate_occupant` — 端口冲突解决
- `agent.detect/models/render/preview/write`、`agent.cleanup.preview/write` — Agent 配置及按单 Agent 清理（Claude Code、opencode、Codex）
- `apikey.usage` — 经可信本地路由查询当前 API key 的有界用量快照

子包职责：

- `app` — 装配所有服务，映射协议错误，强制 API key 清零，并把无 key cleanup 请求直接分发到 Agent service
- `lifecycle` — 进程 spawn、状态文件、父进程监控、异常退出检测；稳定启动阶段、跨会话 reclaim 与经完整身份校验的 legacy 迁移
- `discovery` — 分类 router 状态（desktop_owned / external_compatible / degraded / stale / legacy_managed / absent）
- `agent` — 检测、配置渲染（按 agent 格式：JSON/TOML）、基于 sidecar 所有权的清理、支持 replace/delete 与备份/回滚的事务性写入
- `agent/modelconfig` — 无 key 的规范化 model config schema v1：`Decode`/`DecodeStructural`/`Canonical`/`DeepMerge`、目录/写入/cleanup token 签名
- `trustedrouter` — 经 router `/v1/models` 与 `/v1/usage` 的鉴权发现
- `apikeyusage` — 有界 per-key `GET /v1/usage` 客户端与 fail-closed 解析
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

- `src/lib.rs` — 应用入口：插件注册（无 shell 插件）、setup（`installation.json` → `runtime` → 进程内 `InProcessFactory`）、invoke handler 注册；`--verify-manager-handshake` 在临时目录构造内嵌 manager 校验身份，并由 CLI 打印 version、deployment ID、protocol、manager target 与 rustc target triple
- `src/runtime.rs` — 生产运行时装配：内嵌凭据（`build.rs` 写入 `OUT_DIR`）、`SupervisorConfig`、`CurrentLineage`/`SessionConfig`、编译期 preset/simplify
- `src/commands.rs` — Tauri 命令处理器，代理到 manager client；`AppState`、`ModelFlow`，以及不接收凭据/model flow 的 cleanup preview/write command
- `src/manager.rs` — `ManagerClient` 与 `TransportFactory` trait：单请求在飞、watchdog、一次性恢复；生产 transport 为 `manager_core::InProcessFactory`；cleanup write 禁止不确定投递后的自动 replay
- `src/manager_core/` — 内嵌 protocol v4 控制面：lifecycle、occupant、Agent、trusted-router、`discovery`（Go discovery 移植）、`embedded`（生产 `Backend`：状态调和 + 端口分类 + 会话日志 + `desktop-state.json` 自有记录）、`legacy`（一次性完整身份校验迁移与每代际终止闩）、`session`（生产装配）
- `src/router_core/` — 内嵌 router：HTTP/1 + rustls/`ring`，probe、精确 `/version`/`/health`、流式代理、白名单访问日志、独立 runtime supervisor。监督器 `request_timeout`（默认 120s，覆盖 connect+发送+等响应头）与并发上限（默认 32）是进程内隔离，只约束代理请求，不是冻结 Go HTTP 契约；二者与探针 `DEFAULT_TIMEOUT`（10s，对应 Go `-timeout`/`MTLS_TIMEOUT`）分离；精确 `/version`/`/health` 豁免，`/health` 在探针失败或上游慢时仍返回 HTTP 200。上游连接目前按请求重拨，不进入 parity 契约
- `src/scheduler.rs` — 轮询调度器，向前端 emit `router-poll-snapshot` 事件
- `src/port_recovery.rs` — manager 报告首次释放后约 10 秒定期采样，区分未检测到重新占用与已采样到重新占用
- `src/updater.rs` — stable-only 桌面整包检查/安装：有限网络超时、Tauri 签名下载、desktop-owned router 停止与失败恢复、安装后重启
- `src/tray.rs` — 系统托盘，状态感知菜单
- `src/orchestration.rs` — 首次启动流程（无 router 时自动启动内嵌 router；`legacy_managed` 走 `router.migrate_legacy`）
- `src/model_config.rs` — model config 导入/导出校验
- `src/autostart.rs` — 登录启动插件包装（首次启动默认启用）
- `src/paths.rs` — 桌面数据目录解析
- `src/types.rs` — 镜像 manager 协议结果的严格 serde 类型，包含 cleanup detection、preview 与 delete/backup 文件影响
- `src/error.rs` — 将 manager 协议错误映射为用户可见字符串

Rust 侧绝不向 webview 暴露 shell/fs/http 权限（由 `lib.rs` 中的测试强制保证）；`bundle.externalBin` 为空，Rust 亦不依赖 shell 插件。

桌面在线更新仅由精确 stable `vX.Y.Z` release 启用，固定检查 `https://release.codeasier.top/latest.json`，`latest.json` 中各平台产物 URL 指向 `https://release.codeasier.top/mtls-router/<tag>/`。更新包即完整桌面应用（内嵌 manager/router），必须通过独立 Tauri updater 签名校验并经用户确认后安装。

## Setup 脚本（`setup.sh` / `setup.ps1`，冻结）

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
| `internal/manager`    | [INDEX.md](internal/manager/INDEX.md)    | 控制面：19 个协议方法、生命周期、发现、Agent 配置与清理、API key 用量。其 15 个子包各有专属 INDEX，导航见 [子包表](internal/manager/INDEX.md#子包) |
| `desktop`             | [INDEX.md](desktop/INDEX.md)             | Tauri 2 应用：React 前端 + Rust 后端，内嵌 router 与 manager                                                                          |

## 辅助参考

- `scripts/build.sh` 在 `secrets/` 下生成占位 PEM 供本地构建；真实发布密钥来自 GitHub secrets/vars。
- `desktop/scripts/prepare-updater-config.sh` 为 stable tag 生成私有 Tauri updater overlay 并校验固定公钥指纹；`desktop/scripts/updater-public-key-fingerprint.mjs` 生成该指纹；`desktop/scripts/create-macos-updater.sh` 从最终 macOS app 生成签名 `.app.tar.gz`；`desktop/scripts/verify-package.sh` 收集六平台 updater 产物及 `.sig` 并验证签名与公钥匹配。
- `scripts/package-release.sh` 对精确 stable tag 汇总六平台 updater artifact/signature、生成 `latest.json`（平台 URL 指向 `release.codeasier.top`）并纳入 `SHA256SUMS`；release/recovery workflow 另将 tag 目录镜像至 `downloads.codeasier.top` 并单调、原子推进其 `latest` symlink 作为二级分发点，而 updater feed 与产物下载均以 `release.codeasier.top` 为准。Updater 签名密钥与 Windows/macOS 平台签名凭据属于独立信任链。
- `.worktrees/` 目录含 git worktree 产物，已在 `.gitignore` 中忽略；分析产品代码时忽略。
- 管理协议当前版本为 `4`；router、manager、setup receipt、release metadata 与桌面端必须同版本，桌面端在启动握手时校验并拒绝混合代。
