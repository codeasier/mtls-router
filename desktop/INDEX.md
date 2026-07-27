# desktop

Tauri 2 桌面应用（CodeasierRouter）：React 前端 + Rust 后端，通过 manager sidecar 管理 mtls-router。

## 架构

```
React UI ──Tauri invoke──▶ Rust commands.rs ──stdin/stdout JSON──▶ mtls-router-manager serve
                                    │
                              scheduler.rs ──poll──▶ manager ──emit──▶ "router-poll-snapshot" event ──▶ React
                                    │
                              port_recovery.rs ──约 10 秒采样──▶ released / reoccupied
```

桌面应用绝不直接与 router 通信。它以长驻子进程方式拉起 `mtls-router-manager serve`，带 `--desktop-session`、`--parent-pid/start/executable` flag。

- **协议与启动失败诊断**：桌面端严格校验 management protocol v4 结构；预启动失败按稳定阶段和可选数值 OS 错误码生成安全诊断，启动后失败会终止并等待自有子进程。lifecycle 保留有界原始输出，而 app 协议仅暴露脱敏的会话作用域诊断。
- **端口恢复**：RouterPage 只按结构化 action/reason 渲染按钮或 SCM/systemd 人工引导，不执行命令、不提权、不猜测 launchd label；Windows copy command 只生成适用于管理员 PowerShell 的安全引用文本，不适用于 `cmd.exe`。Rust `port_recovery.rs` 在 manager 报告分模式成功证据与首次释放后，由 scheduler 在约 10 秒内定期采样；只有持续的 `absent` 状态可产生 `released`，`unknown_occupant` 产生 `reoccupied`，其他状态、主动启动和 manager session 变化会取消观察。

## 前端（src/）

| 文件                                | 职责                                                                                                              |
| ----------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `ipc.ts`                            | `DesktopApi` 接口 + `createDesktopApi()` —— 所有 Tauri 命令的类型化包装；`sanitizeSensitiveText()` 用于客户端脱敏 |
| `App.tsx`                           | 根布局：侧边栏导航（Router/Agents/API 密钥/Logs/Settings）+ 区块渲染                                              |
| `RouterPage.tsx`                    | router 状态、start/stop、health、占用者检查/终止                                                                  |
| `AgentPage.tsx`                     | Agents 页面协调器：本地检测、单 Agent 模型加载、错误行动项、flow 生命周期与总览/工作流切换                        |
| `AgentOverview.tsx`                 | Claude Code / OpenCode / Codex 总览；分别展示 CLI 安装与配置状态，不依赖模型上游                                  |
| `AgentWorkflow.tsx`                 | 单 Agent 配置、导入导出、preview、审批与事务写入的过渡工作流；不接触明文 API key                                  |
| `agentPresentation.tsx`             | Agent 名称/logo、完整 detection 校验、安装/配置状态与 recovery 文案的共享展示模型                                 |
| `ApiKeysPage.tsx`                   | 全局 API key 保存、替换、删除及摘要展示；提交后清空输入，不提供明文回读                                           |
| `LogsPage.tsx`                      | 有界的、安全过滤的 router 日志，手动刷新                                                                          |
| `SettingsPage.tsx`                  | 自启动、诊断、卸载准备、语言                                                                                      |
| `model.ts`                          | 共享类型（`SectionId`、`navigationItems`）                                                                        |
| `i18n.tsx`                          | I18n context provider，含 `zh-CN` 与 `en` locale                                                                  |
| `locales/zh-CN.ts`、`locales/en.ts` | 翻译字典                                                                                                          |

## 后端（src-tauri/src/）

| 文件                  | 职责                                                                                                                                              |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `lib.rs`              | 应用入口：插件注册、setup（sidecar 校验、manager spawn、调度器启动、托盘）、invoke handler 注册                                                   |
| `commands.rs`         | 所有 `#[tauri::command]` handler；`AppState`（manager client、调度器、路径、无密钥 model flow、全局凭据存储）                                     |
| `credential.rs`       | `CredentialStore`：严格 schema、原子写入、Unix 0600、摘要/使用/删除及进程内并发控制；明文使用值以 `Zeroizing<String>` 返回                        |
| `manager.rs`          | `ManagerClient` + `TauriTransportFactory` —— spawn manager 子进程、发送 JSON 请求、读取响应；仅向协议定义凭据字段的 `agent.models/write` 注入 key |
| `scheduler.rs`        | `PollScheduler` —— 周期性 router 状态/健康轮询；推进端口释放观察；emit `router-poll-snapshot` 事件；可见性感知间隔                                |
| `port_recovery.rs`    | `PortRecovery` —— manager session epoch 绑定的约 10 秒定期采样状态；区分 `observing`、`released`、`reoccupied`                                    |
| `sidecar.rs`          | `SidecarPaths::resolve()` —— 在 app 二进制旁定位 `mtls-router[.exe]` 与 `mtls-router-manager[.exe]`（运行时纯名字）；校验 SHA-256 + 原生架构/格式 |
| `tray.rs`             | 系统托盘图标/菜单；状态感知标签；窗口显示/隐藏                                                                                                    |
| `orchestration.rs`    | `first_launch()` —— sidecar 有效且无 router 运行时自动启动 router                                                                                 |
| `model_config.rs`     | model config 导入/导出 JSON 校验                                                                                                                  |
| `paths.rs`            | 桌面数据目录解析（委托给 `MTLS_ROUTER_DESKTOP_DATA_DIR` 或 OS 默认），并派生 `credentials.json` 路径                                              |
| `process_identity.rs` | `current()` —— 捕获 PID + 启动时间 + 可执行文件用于父身份 flag                                                                                    |
| `autostart.rs`        | 登录启动插件包装；首次启动默认启用                                                                                                                |
| `types.rs`            | 镜像 manager 协议结果的 serde 类型                                                                                                                |
| `error.rs`            | `CommandError` —— 将 manager 协议错误映射为用户可见字符串                                                                                         |

## 安全约束

- webview 能力：仅 `core:default` —— 无 shell/fs/http/opener 权限（由 `lib.rs` 中的测试强制保证）。
- API key 持久化于数据目录的 `credentials.json`；读取兼容 UTF-8 BOM，Unix 强制 0600，Windows 当前为用户数据目录 ACL 的最佳努力实现。
- 凭据写入先同步临时文件再原子替换；Unix 写入和删除还会同步父目录，确保目录项变更持久化。
- Webview 只能保存/删除 key，不能回读明文；Rust 单次调用使用 `Zeroizing<String>`，manager 请求 JSON 与序列化缓冲在发送后清零。
- CSP：`default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' asset: http://asset.localhost; style-src 'self' 'unsafe-inline'`。
- manager 握手在启动时校验：version、management protocol v4、deployment ID；v4 occupant 响应枚举、字段组合、标识上限和 token 规则均 fail closed。

## 构建

```bash
npm run sidecars:build    # 为 host target 构建 Go router + manager 到 src-tauri/binaries/
npm exec tauri -- build   # 完整 Tauri 构建（需 sidecar 已就位）
```

sidecar 命名：`src-tauri/binaries/` 下的构建输入使用 target-triple 名（`mtls-router-<target-triple>`，如 `mtls-router-aarch64-apple-darwin`）；Tauri 打包后安装的二进制使用纯名字（`mtls-router`、`mtls-router-manager`，Windows 带 `.exe`）。

## 测试

```bash
npm test                  # vitest（前端单测，jsdom）
npm run rust:test         # cargo test（Rust 后端测试）
npm run verify            # 全套：eslint + prettier + tsc + vitest + vite build + cargo fmt + cargo test
```
