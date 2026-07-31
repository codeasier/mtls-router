# internal/manager/agent

检测受支持的 Agent、渲染其配置，并以带备份/回滚的事务写入或按可信 sidecar 所有权清理目标文件。manager 中最大的子包。

无 key 的 model config schema 见 [`modelconfig/INDEX.md`](modelconfig/INDEX.md)。

## 支持的 Agent 与目标文件

| `Kind` | 配置文件 | 凭据字段 |
|---|---|---|
| `claude` | `settings.json`（`CLAUDE_CONFIG_DIR`，默认 `~/.claude`） | `env.ANTHROPIC_AUTH_TOKEN` |
| `opencode` | `opencode.json`（`OPENCODE_CONFIG`，默认 `~/.config/opencode`；JSON-before-JSONC 回退） | `provider.mtls-router.options.apiKey` |
| `codex` | `config.toml` + `auth.json`（`CODEX_HOME`，默认 `~/.codex`） | 仅 `auth.json` 的 `OPENAI_API_KEY`；`config.toml` 不含 key |

## 文件

| 文件 | 职责 |
|------|------|
| `service.go` | `Service`、`NewService(Options)`；`ErrorCode`、`OperationError`、`CodeOf`；普通/cleanup 共用的备份、journal v3、apply、回滚与启动恢复；`OperationDelete` |
| `cleanup.go` | `CleanupPreview`/`CleanupWrite`；基于 last-applied sidecar 构建单 Agent 清理计划、有效所有权收窄、漂移/revision 校验、cleanup token 生成与 sidecar replace/delete |
| `detect.go` | `Detector`、`Detect()`；`State`、`Format`、`RecoveryReason`、`RecoveryState`；`CleanupState` 与 `not_managed/model_state_invalid/writes_disabled` 标注；配置体积上限 |
| `paths.go` | `Kind` 常量；`ClaudePaths`/`OpenCodePaths`/`CodexPaths` —— 与 setup 脚本一致的路径语义 |
| `render.go` | `Render(...)`、`Fragment`、`RenderResult`；片段渲染的公共流程与 `apiURL` 派生 |
| `render_claude.go` | Claude `settings.json` 片段渲染/合并；受管 env key 集合；按已记录 `env.*` 路径执行纯 cleanup transform |
| `render_opencode.go` | opencode provider 片段渲染/合并；删除受管 provider，并仅在有效所有权下删除 `mtls-router/` 根 model 的纯 cleanup transform |
| `render_codex.go` | Codex `config.toml` + `auth.json` 渲染；认证冲突判定；按已保存 model config 收窄可选 root 并移除文件认证字段的纯 cleanup transform |
| `preview.go` | 写入计划构建（`writePlan`/`plannedFile`）、按 agent 的计划分支、重建计划与校验；JSONC 迁移/规范化警告 |
| `models.go` | `DiscoverModels(...)`；`ModelsExisting`/`ModelsPreset`/`ModelsResult` —— existing 与 preset 的合并与不可用模型标记 |
| `sidecar.go` | last-applied sidecar 状态读写与校验；新写入只记录 merge 实际取得的 OpenCode 根 model/Codex 可选字段所有权，同时兼容旧宽记录 |
| `files.go` | 事务文件原语：`writeAtomic`、`removeTargetAndSync`、`createPrivateBackup`、`verifyPrivateBackup`、`postReplaceError` |
| `files_unix.go`、`files_windows.go` | 平台权限与原子替换（Unix `0o600` / Windows DACL 仅当前用户） |
| `lock.go`、`lock_unix.go`、`lock_windows.go` | OS 事务锁；写入/恢复可创建私有协调状态，cleanup preview 只打开并校验已存在的私有 lock |
| `recovery.go`、`recovery_unix.go`、`recovery_windows.go` | 非法配置的分类与恢复状态判定；符号链接检测 |
| `trust.go` | 写入前的 router 信任校验对接 |
| `toml.go` | Codex TOML 编解码辅助 |

## 服务方法

`Detect`、`DiscoverModels`、`Render`、`Preview` / `PreviewRequest`、`ValidatePreview`、`Write`、`CleanupPreview`、`CleanupWrite`、`CatalogBinding`、`Recover` / `RecoveryError` / `WritesDisabled`。

## 关键不变量

- **写入是事务性的**：受限权限的同目录临时文件（`os.CreateTemp` + `restrictPrivate`）→ `replaceAtomic` 原子重命名；替换前的原始字节进入事务备份。defer 中的 `os.Remove(tmpPath)` 只在 replace 失败的路径上生效。
- **删除与替换共用事务证据**：journal v3 为每项显式记录 `replace/delete`；delete 的 post revision 为不存在。Agent 文件先执行、manager sidecar 最后执行，回滚顺序相反；legacy v1/v2 journal 继续按隐式 replace 恢复。
- **cleanup 必须有可信所有权**：只读取 sidecar 记录的绝对路径，不按当前环境重新推导；无条目返回 `AGENT_NOT_MANAGED`，sidecar/signing 状态无效返回 `MODEL_STATE_INVALID`，不按配置外观猜测。
- **cleanup 不使用 key/router/catalog**：preview/write 不调用目录校验、router trust、普通 Render，也不创建 model flow；preview 不创建文件、目录、lock、备份或 journal，只打开并校验已存在的私有事务 lock。
- **cleanup 删除范围按格式收窄**：Claude 只删记录的 `env.*`；OpenCode 只删 `provider.mtls-router` 及仍为精确 `mtls-router/` 前缀的已拥有根 model；Codex 可选 root 由已保存 model config 决定，auth 只删 `auth_mode` / `OPENAI_API_KEY`。语义根为空才删除文件。
- **备份可能含旧 key**：`*.bak-*` / `*.rollback-*` 与源文件同目录、权限受限（`0o600` / DACL），内容是替换前的原始字节。
- **key 明文落盘仅限 Agent 凭据文件**（见上表）。key 绝不进入环境变量、CLI 参数、model config、日志或 journal。
- **检测不探测 CLI 安装**：`agent.detect` 只检查配置文件；protocol v4 兼容字段固定返回 `detected=true`、`command=""`，不读取进程 `PATH`。
- **JSONC 会丢失注释与格式**：默认路径下 `opencode.jsonc` 被迁移为 `opencode.json`；`OPENCODE_CONFIG` 指向 JSONC 时就地规范化为严格 JSON。两种情形都在预览阶段给出警告。
- OpenCode/Codex 的 provider key 与模型前缀固定为兼容标识 `mtls-router`；新写入的 provider 展示名是 `CodeasierRouter`，检测与合并同时接受旧展示名 `mtls-router`。
- 只改写受管键：Claude 的受管 env key、Codex 的受管 root key 集合是显式白名单，用户其余配置原样保留。
- 预览与写入之间必须一致 —— `ValidatePreview` 检出漂移时返回 `PREVIEW_STALE`，绝不带着过期计划写入。

## 依赖

- `modelconfig` —— 配置 schema、规范化、token 签名
- `../protocol`（错误码）、`../trustedrouter`（写入前重验绑定）
- `github.com/BurntSushi/toml`（Codex `config.toml`）

## 测试

- `cleanup_test.go`、`detect_test.go`、`render_test.go`、`service_test.go`、`sidecar_test.go`、`trust_test.go`、`models_test.go`
- `files_test.go`、`files_windows_test.go`、`lock_test.go` —— 事务写入、权限与锁
- `internal/manager/agent/testdata/` —— 各 Agent 的配置样本
