# desktop

Tauri 2 桌面应用（CodeasierRouter）：React 前端 + Rust 后端，通过 manager sidecar 管理 mtls-router。

## 架构

```
React UI ──Tauri invoke──▶ Rust commands.rs ──stdin/stdout JSON──▶ mtls-router-manager serve
                                    │                                      │
                                    │                               router.trusted_channel
                                    ▼
                              scheduler.rs ──poll──▶ manager ──emit──▶ React
                                    │
                              port_recovery.rs ──约 10 秒采样──▶ released / reoccupied
                                    │
                              updater.rs ──HTTPS + Tauri signature──▶ downloads.codeasier.top
                                    │
                              image_commands.rs ──单连接信任校验/认证──▶ loopback router
```

控制面以长驻子进程方式拉起 `mtls-router-manager serve`，带 `--desktop-session`、`--parent-pid/start/executable` flag。图片数据面是唯一的直连例外：Rust 从 manager 私有 `router.trusted_channel` 方法取得完整可信状态，在同一 loopback TCP 连接上校验 `/version`、进程身份和 `/health`，之后才读取凭据并调用图片目录/generation；webview 不直接联网。

- **协议与启动失败诊断**：桌面端严格校验 management protocol v4 结构；预启动失败按稳定阶段和可选数值 OS 错误码生成安全诊断，启动后失败会终止并等待自有子进程。lifecycle 保留有界原始输出，而 app 协议仅暴露脱敏的会话作用域诊断。
- **端口恢复**：RouterPage 只按结构化 action/reason 渲染按钮或 SCM/systemd 人工引导，不执行命令、不提权、不猜测 launchd label；Windows copy command 只生成适用于管理员 PowerShell 的安全引用文本，不适用于 `cmd.exe`。Rust `port_recovery.rs` 在 manager 报告分模式成功证据与首次释放后，由 scheduler 在约 10 秒内定期采样；只有持续的 `absent` 状态可产生 `released`，`unknown_occupant` 产生 `reoccupied`，其他状态、主动启动和 manager session 变化会取消观察。
- **桌面整包更新**：仅精确 stable `vX.Y.Z` release 配置 `https://downloads.codeasier.top/mtls-router/latest/latest.json` 与 updater 公钥；非 stable 构建保留可反序列化但无 endpoint 的禁用配置。启动时静默检查一次，Settings 可手动复查。用户确认后 `updater.rs` 重新检查精确 stable SemVer、下载并强制校验 Tauri 签名，只停止经验证的 desktop-owned router，安装包含 manager/router sidecar 的完整包并重启；不停止 external router，也不改变 CLI 更新路径。

## 前端（src/）

| 文件                                                      | 职责                                                                                                                  |
| --------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `ipc.ts`                                                  | `DesktopApi` 类型化包装；严格定义 cleanup、updater、图片操作事件与精确 invoke payload                                 |
| `dev/fixtures.ts`                                         | 与 Vitest 解耦的浏览器/单测共用 fixture 数据、cleanup preview/result 与 mock 场景解析                                 |
| `dev/mockDesktopApi.ts`                                   | 仅开发 mock 的内存 `DesktopApi`；模拟 cleanup、updater 与图片状态，但不读写真实凭据、Agent 配置或网络                 |
| `dev/resolveDesktopApi.ts`                                | mock 入口门控：仅 `DEV && VITE_MOCK=true` 时启用；生产构建始终走真实 Tauri API                                        |
| `App.tsx`                                                 | 根布局、六区块导航、单次静默启动更新检查与注册式 Agent leave guard；面板存续期同步原生退出保护并共用可访问确认框      |
| `RouterPage.tsx`                                          | router 状态、start/stop、health、占用者检查/终止                                                                      |
| `AgentPage.tsx`                                           | Agents 页面协调器：本地检测总览、互斥的单 Agent 配置/cleanup 目标、cleanup busy leave guard、完成后刷新与返回焦点恢复 |
| `AgentOverview.tsx`                                       | Claude Code / OpenCode / Codex 总览；展示配置与 cleanup 状态；只为 managed/available Agent 提供单 Agent 清理入口      |
| `AgentPanel.tsx`                                          | 持久单 Agent 面板：编辑器与 sticky preview/status rail 共存，覆盖导入导出、刷新冲突、写入、结果 dismiss 与 guard      |
| `useAgentPanelController.ts`                              | 单 Agent detection/discovery flow、草稿基线、刷新冲突、preview/write/reload 的持久状态机                              |
| `AgentConfigFields.tsx`                                   | Claude Code / OpenCode / Codex 结构化配置字段；提供 imperative snapshot 以同步本地 JSON 草稿                          |
| `AgentPreviewPane.tsx`                                    | 脱敏 preview、文件影响、漂移/auth 审批、rebuild 确认与写入结果；导出配置/cleanup 共用的 `AgentFileEffectCard`         |
| `AgentCleanupPanel.tsx`                                   | 独立的无 key cleanup 审阅：removed paths、replace/delete、敏感备份、漂移批准、stale/retry 与结果                      |
| `agentCleanupState.ts`                                    | Cleanup-only reducer：`loading-preview/previewing/writing/result/stale/failed` 与写入门控                             |
| `useAgentCleanupController.ts`                            | 单 Agent cleanup preview/write/repreview/retry 编排；generation guard 丢弃迟到结果，不调用凭据/目录/model flow        |
| `AgentCleanupPanel.test.tsx`、`agentCleanupState.test.ts` | Cleanup UI、漂移门控、stale/retry、重复提交、保留数据警告与 reducer transition 测试                                   |
| `agentPresentation.tsx`                                   | Agent 名称/logo、完整 detection 校验、安装/配置状态与 recovery 文案的共享展示模型                                     |
| `ApiKeysPage.tsx`                                         | 全局 API key 保存、替换、删除及摘要展示；提交后清空输入，不提供明文回读                                               |
| `ConversationsPage.tsx`                                   | 图片对话、精确模型选择、独立草稿、单图引用、全局 generation/cancel 与迟到 operation event 防护                        |
| `LogsPage.tsx`                                            | 有界的、安全过滤的 router 日志，手动刷新                                                                              |
| `SettingsPage.tsx`                                        | 自启动、组件版本、手动更新检查、确认后安装/进度、诊断、卸载准备与语言                                                 |
| `model.ts`                                                | 共享类型（`SectionId`、`navigationItems`）                                                                            |
| `i18n.tsx`                                                | I18n context provider，含 `zh-CN` 与 `en` locale                                                                      |
| `locales/zh-CN.ts`、`locales/en.ts`                       | 翻译字典                                                                                                              |

## 后端（src-tauri/src/）

| 文件                  | 职责                                                                                                                                                |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `lib.rs`              | 应用入口：插件注册、setup（sidecar 校验、manager spawn、调度器启动、托盘）、invoke handler 注册                                                     |
| `commands.rs`         | 所有 `#[tauri::command]` handler；`AppState`；cleanup preview/write 使用严格 request 且不接收凭据或 model flow，write 纳入 lifecycle busy/quit 保护 |
| `credential.rs`       | `CredentialStore`：严格 schema、原子写入、Unix 0600、摘要/使用/删除及进程内并发控制；明文使用值以 `Zeroizing<String>` 返回                          |
| `manager.rs`          | `ManagerClient` + `TauriTransportFactory` —— spawn/通信；cleanup watchdog 对齐 Go 5/30 秒 deadline 再加 1 秒，cleanup write 不在不确定投递后 replay |
| `scheduler.rs`        | `PollScheduler` —— 周期性 router 状态/健康轮询；推进端口释放观察；emit `router-poll-snapshot` 事件；可见性感知间隔                                  |
| `port_recovery.rs`    | `PortRecovery` —— manager session epoch 绑定的约 10 秒定期采样状态；区分 `observing`、`released`、`reoccupied`                                      |
| `updater.rs`          | Tauri updater command：stable-only 比较、有限网络超时与二次版本绑定、下载进度、desktop-owned router 停止/失败恢复、整包安装和重启                   |
| `sidecar.rs`          | `SidecarPaths::resolve()` —— 在 app 二进制旁定位 `mtls-router[.exe]` 与 `mtls-router-manager[.exe]`（运行时纯名字）；校验 SHA-256 + 原生架构/格式   |
| `tray.rs`             | 系统托盘图标/菜单；状态感知标签；关闭主窗口隐藏到托盘（macOS 先 `AppHandle::hide` 让出原生全屏 Space，再隐藏窗口）；托盘/二次实例共用激活路径       |
| `orchestration.rs`    | `first_launch()` —— sidecar 有效且无 router 运行时自动启动 router                                                                                   |
| `model_config.rs`     | model config 导入/导出 JSON 校验                                                                                                                    |
| `paths.rs`            | 桌面数据目录解析（委托给 `MTLS_ROUTER_DESKTOP_DATA_DIR` 或 OS 默认），并派生 `credentials.json` 路径                                                |
| `process_identity.rs` | `current()` —— 捕获 PID + 启动时间 + 可执行文件用于父身份 flag                                                                                      |
| `autostart.rs`        | 登录启动插件包装；首次启动默认启用                                                                                                                  |
| `types.rs`            | 镜像 manager 协议结果及桌面更新状态的严格 serde 类型，含 cleanup preview、update info/check/progress 与文件影响                                     |
| `error.rs`            | `CommandError` —— 将 manager 协议错误映射为用户可见字符串                                                                                           |
| `image_limits.rs`     | 20 KiB prompt、20 MiB 图片、32 MiB response、16,384 单边和 64 MP 的单一常量来源                                                                     |
| `router_process.rs`   | 独立读取并验证 router PID、OS 启动身份和可执行文件；不改变桌面父进程 `process_identity.rs` 语义                                                     |
| `trusted_channel.rs`  | 单个 loopback `TcpStream` 上的 HTTP/1.1 version/process/health 信任链；认证前禁止调用、禁止重拨、限制 framing/响应/超时                             |
| `image_models.rs`     | 严格解析 `/v1/models/image`，与两个不可变预置做精确 ID 交集                                                                                         |
| `image_client.rs`     | 固定单图 generation JSON、显式 data URI 编辑和有界 `b64_json` 结果解析                                                                              |
| `image_validation.rs` | PNG/JPEG/WebP magic-byte、静态格式、base64、字节和像素边界验证                                                                                      |
| `image_store.rs`      | 版本化快照、SHA-256 资产、原子替换、启动中断恢复、提交后删除和孤儿清理                                                                              |
| `image_commands.rs`   | 窄图片 IPC、readiness 刷新、exactly-one operation guard、取消、`rfd` 文件选择和 `image-asset` 路径校验                                              |

## 安全约束

- webview 能力：仅 `core:default` —— 无 shell/fs/http/opener 权限（由 `lib.rs` 中的测试强制保证）。
- API key 持久化于数据目录的 `credentials.json`；读取兼容 UTF-8 BOM，Unix 强制 0600，Windows 当前为用户数据目录 ACL 的最佳努力实现。
- 凭据写入先同步临时文件再原子替换；Unix 写入和删除还会同步父目录，确保目录项变更持久化。
- Webview 只能保存/删除 key，不能回读明文；Rust 单次调用使用 `Zeroizing<String>`，manager 请求 JSON 与序列化缓冲在发送后清零。
- Cleanup IPC 精确只发送 Agent、revision token 和漂移批准，不访问 `CredentialStore`、model flow、router 或模型目录；cleanup write 属于 non-replayable lifecycle 操作。
- CSP：`default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' image-asset: http://image-asset.localhost; style-src 'self' 'unsafe-inline'`；自定义图片协议只接受规范 SHA-256 asset ID。
- manager 握手在启动时校验：version、management protocol v4、deployment ID；v4 occupant 响应枚举、字段组合、标识上限和 token 规则均 fail closed。
- Updater 公钥与 endpoint 只在 stable tag 构建的 runner 临时配置中注入；公钥指纹由 repository variable 固定，每个原生产物上传前均验证签名与公钥匹配。Tauri updater 签名独立于 Windows/macOS 平台签名/notarization；非 stable 构建不生成 updater 产物，私钥和密码只来自 GitHub Secrets，不写入仓库配置或日志。

## 分层本地开发

按改动类型选择最短反馈循环（均不绕过 sidecar 哈希、manager 握手、preview/revision 或事务写入保护）：

| 改动类型                    | 命令                                                                            | 说明                                                                                                                                          |
| --------------------------- | ------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| 仅 React/UI                 | `npm run dev:mock`                                                              | 只跑 Vite + HMR；注入内存 `DesktopApi`；可选 `?mockScenario=success\|protocol-error\|preview-stale\|write-fail`                               |
| Rust/Tauri（sidecar 未变）  | `npm run dev:tauri:reuse`                                                       | 启动 `tauri dev` 但跳过 `sidecars:build`；sidecar 缺失则 fail closed                                                                          |
| 真实 Agent 链路（隔离路径） | `npm run dev:agent`                                                             | 显式覆盖 `MTLS_ROUTER_DESKTOP_DATA_DIR` / `CLAUDE_CONFIG_DIR` / `OPENCODE_CONFIG` / `CODEX_HOME` 后走 reuse；不隔离固定端口 `127.0.0.1:19099` |
| 自定义 upstream 图片链路    | `npm run dev:image -- --upstream <base-url>`                                    | 短期 mTLS bridge + 重建 sidecar + 隔离桌面/Agent/router 数据；API key 仍只经 API Keys 页面写入                                                |
| Manager/router 或嵌入元数据 | `npm run sidecars:build` 后 `npm run dev:tauri:reuse` 或 `npm run tauri -- dev` | Go sidecar 变化后必须重建；Rust 会重新嵌入哈希                                                                                                |
| 安装器/发布验证             | `make desktop-package-current`                                                  | 完整打包                                                                                                                                      |

`npm run tauri` 仍始终先执行 `sidecars:build`，适合首次准备与完整本地启动。

`scripts/dev-image.mjs` 负责生成并清理短期 TLS、隔离本地数据、临时重建并在退出时恢复 host sidecar，以及启动/终止 Tauri 子进程；`scripts/dev-image-bridge.mjs` 仅在 loopback 暴露 readiness、`/v1/models/image` 与 `/v1/images/generations`，透明转发认证 header 但不读取或记录 API key。明文 upstream 只允许 private/loopback IP，`build-sidecars.sh` 的开发证书目录输入在 release 构建中 fail closed。

## 构建

```bash
npm run sidecars:build    # 为 host target 构建 Go router + manager 到 src-tauri/binaries/
npm exec tauri -- build   # 完整 Tauri 构建（需 sidecar 已就位）
```

sidecar 命名：`src-tauri/binaries/` 下的构建输入使用 target-triple 名（`mtls-router-<target-triple>`，如 `mtls-router-aarch64-apple-darwin`）；Tauri 打包后安装的二进制使用纯名字（`mtls-router`、`mtls-router-manager`，Windows 带 `.exe`）。

Release updater 辅助脚本：`scripts/prepare-updater-config.sh` 只为 stable tag 生成权限受限的 Tauri overlay config，并校验 updater 公钥固定指纹及完整签名输入；`scripts/updater-public-key-fingerprint.mjs` 从公钥文件生成该指纹；`scripts/create-macos-updater.sh` 在最终签名 app bundle 后生成并签名 `.app.tar.gz`；`scripts/verify-package.sh` 从包内 desktop executable 构造一次 Tauri app 以覆盖插件初始化，再验证 manager 握手、规范化六平台 updater 产物及 `.sig`，并通过 `src-tauri/examples/verify_updater_signature.rs` 验证签名。仓库根 `scripts/package-release.sh` 只为 stable tag 汇总六平台 `latest.json`、产物与签名并纳入 `SHA256SUMS`。

## 测试

```bash
npm test                  # vitest（前端单测，jsdom）+ Node 开发脚本测试
npm run rust:test         # cargo test（Rust 后端测试）
npm run verify            # 全套：eslint + prettier + tsc + 前端/脚本测试 + vite build + cargo fmt + cargo test
```
