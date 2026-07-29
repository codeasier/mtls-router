# internal/manager/agent

检测受支持的 Agent、渲染其配置、并以带备份/回滚的事务写入目标文件。manager 中最大的子包。

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
| `service.go` | `Service`、`NewService(Options)`；`ErrorCode`、`OperationError`、`CodeOf`；`CatalogBinding`、`ValidatePreview`、`ValidateRefreshedModels`；写入 journal 文件名 |
| `detect.go` | `Detector`、`Detect()`；`State`、`Format`、`RecoveryReason`、`RecoveryState`；配置体积上限 |
| `paths.go` | `Kind` 常量；`ClaudePaths`/`OpenCodePaths`/`CodexPaths` —— 与 setup 脚本一致的路径语义 |
| `render.go` | `Render(...)`、`Fragment`、`RenderResult`；片段渲染的公共流程与 `apiURL` 派生 |
| `render_claude.go` | Claude `settings.json` 片段渲染与合并；受管 env key 集合 |
| `render_opencode.go` | opencode provider 片段渲染与合并 |
| `render_codex.go` | Codex `config.toml` + `auth.json` 渲染；`CodexMergeAssessment` 判定与既有认证方式冲突 |
| `preview.go` | 写入计划构建（`writePlan`/`plannedFile`）、按 agent 的计划分支、重建计划与校验；JSONC 迁移/规范化警告 |
| `models.go` | `DiscoverModels(...)`；`ModelsExisting`/`ModelsPreset`/`ModelsResult` —— existing 与 preset 的合并与不可用模型标记 |
| `sidecar.go` | last-applied sidecar 状态读写与校验；只存 HMAC 摘要 |
| `files.go` | 事务写入原语：`writeAtomic`、`createPrivateBackup`、`verifyPrivateBackup`、`postReplaceError` |
| `files_unix.go`、`files_windows.go` | 平台权限与原子替换（Unix `0o600` / Windows DACL 仅当前用户） |
| `lock.go`、`lock_unix.go`、`lock_windows.go` | 写入期间的事务锁 |
| `recovery.go`、`recovery_unix.go`、`recovery_windows.go` | 非法配置的分类与恢复状态判定；符号链接检测 |
| `trust.go` | 写入前的 router 信任校验对接 |
| `toml.go` | Codex TOML 编解码辅助 |

## 服务方法

`Detect`、`DiscoverModels`、`Render`、`Preview` / `PreviewRequest`、`ValidatePreview`、`Write`、`CatalogBinding`、`Recover` / `RecoveryError` / `WritesDisabled`。

## 关键不变量

- **写入是事务性的**：受限权限的同目录临时文件（`os.CreateTemp` + `restrictPrivate`）→ `replaceAtomic` 原子重命名；替换前的原始字节进入事务备份。defer 中的 `os.Remove(tmpPath)` 只在 replace 失败的路径上生效。
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

- `detect_test.go`、`render_test.go`、`service_test.go`、`sidecar_test.go`、`trust_test.go`、`models_test.go`
- `files_test.go`、`files_windows_test.go`、`lock_test.go` —— 事务写入、权限与锁
- `internal/manager/agent/testdata/` —— 各 Agent 的配置样本
