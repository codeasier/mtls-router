# Agent 模型配置

[English](../AGENT_MODELS.md)

本文是 management protocol v2 的用户与自动化契约。如果本文与客户端本地校验不一致，以 Go manager 为准。

## 服务契约

经过认证的上游 `GET /v1/models` 响应是候选模型的唯一来源。它针对所提供的 Bearer key，通过一个完整的 OpenAI 兼容 `data[].id` 目录返回全部模型。无需按名称筛选或执行 inference probe，每个返回 ID 都支持全部必需路由：

| 客户端 | 路由 | 契约 |
|---|---|---|
| 模型目录 | `GET /v1/models` | 一个有界的标准 JSON 响应 |
| Claude Code | `POST /v1/messages`，包括 `?beta=true` | Anthropic 请求字段和开放列表 `anthropic-*` header 原样透传；SSE 不缓冲 |
| Claude Code | `POST /v1/messages/count_tokens` | 精确 token 计数 |
| opencode | `POST /v1/chat/completions` | OpenAI Chat Completions 与流式响应 |
| 兼容客户端 | `POST /v1/completions` | OpenAI Completions 与可选流式响应 |
| Codex | `POST /v1/responses` | OpenAI Responses 与 SSE 流式响应 |

Claude deferred tool 字段以及未来开放列表 Anthropic 请求字段会原样透传。配置过程绝不调用 inference endpoint。Router 会保留 authorization，但不选择模型，也不存储 key。

## 交互流程

Shell、PowerShell 和桌面端都使用以下顺序：

1. 检测并选择 Claude Code、opencode 和/或 Codex。
2. 隐藏读取 API key，并建立可信的 protocol-v2 loopback router。
3. 调用 `agent.models`，然后才显示共同的已排序目录。
4. 要求明确选择各 Agent 的原生模型。绝不自动选择第一个模型，也不按名称推断偏好。
5. Print 时渲染脱敏片段；write 时预览精确文件、备份、迁移、所有权和漂移影响。
6. 写入前用临时 key 重新获取目录，再执行一次原子多文件事务。

`agent print-config` 同样需要 key，因为它必须根据当前目录验证选择。它不会修改 Agent 文件、事务 journal、备份或 last-applied sidecar。模型发现可能启动 router，首次使用可能创建私有 token signing key。

模型目录只是配置时快照。系统不会后台刷新或重写 Agent 文件。需要刷新时重新进入配置并提供 key。

## 规范模型配置

所有客户端使用同一份无 key JSON 文档。`version` 为 `1`；请求中的 `agents` 数组必须与实际存在的顶层 section 精确一致。除受限 `extra` 和 `options` 外，未知字段都会被拒绝。输入必须是严格 JSON：重复 key、无效 UTF-8、非有限数字、不安全整数范围和受保护的凭据/连接路径都会被拒绝。规范字节使用 RFC 8785 JCS，不执行 Unicode normalization。

包含三个 Agent 的最小结构：

```json
{
  "version": 1,
  "claude": {
    "primary": {"model": "model-a"},
    "haiku": {"inherit_primary": true},
    "sonnet": {"inherit_primary": true},
    "opus": {"inherit_primary": true}
  },
  "opencode": {
    "default_model": "model-a",
    "models": {"model-a": {}}
  },
  "codex": {"model": "model-a"}
}
```

可在 `agent print-config` 或 `agent write-config` 中使用 `--model-config=<path>`，以该文档替代模型设置问答。文件必须是不超过 2 MiB 的普通非链接 JSON 文件，不得包含 API key、URL、provider identity、header、已获取目录或任意 Agent 配置。

### Claude Code

`primary` 必填。`haiku`、`sonnet` 和 `opus` 各自继承主模型，或明确选择另一个目录模型。每个显式模型都可包含可选显示 `name`。`extra` 是字符串 map，仅允许以下 description key：

- `ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION`
- `ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION`
- `ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION`
- `ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION`

Manager 只拥有文档声明的 `env` key，将它们合并到现有 `env`，并保留无关顶层和环境值。

### opencode

`models` 包含一个或多个明确选择的目录 ID，`default_model` 必须是其中之一。每个模型的 typed option 包括显示名称、reasoning、attachment、tool call、temperature、context/input/output limit、输入/输出 modality、interleaved reasoning 和受限 provider `options` object。`extra` 只接受 pinned opencode schema 允许的字段，且不能与 typed 或 manager-owned 路径冲突。

只有显示 `name` 默认等于模型 ID。其他未设置可选字段会省略，而不是猜测。Manager 拥有 `provider.mtls-router` 和已取得所有权的根 `model`，同时保留其他 provider、`small_model` 和无关根设置。JSONC normalization 可能移除注释和格式，预览会明确显示。

### Codex

`model` 选择一个目录 ID。可选 typed field 为 `reasoning_effort`、`reasoning_summary`、`verbosity`、`context_window` 和 `auto_compact_token_limit`。v1 唯一允许的 `extra` key 是 `model_auto_compact_token_limit_scope`，值为 `total` 或 `body_after_prefix`。未设置可选 key 会省略；如果之前由 manager 所有，则会移除。

专用 provider 为 `model_providers.mtls-router`，使用 `wire_api = "responses"` 和可信 loopback `/v1` URL。Codex CLI 与 IDE 共享认证。切换到 `cli_auth_credentials_store = "file"`，并采用官方 `auth_mode = "apikey"` 加 `OPENAI_API_KEY` 文件认证前，预览会单独要求批准。不会删除 OS keyring 凭据。

## 检测、失败与刷新

`agent.detect` 不需要 key。`configured=true` 仅表示本地托管结构完整且内部一致，不表示模型当前可见或已获授权。只有成功的 `agent.models` 发现与写入时刷新才能证明当前授权；系统不持久化 verified timestamp。

发现和写入始终 fail closed。`MODEL_AUTH_FAILED`、`MODEL_DISCOVERY_FAILED`、`MODEL_RESPONSE_INVALID`、`MODEL_CATALOG_EMPTY`、`MODEL_CATALOG_STALE`、`MODEL_CONFIG_INVALID`、`MODEL_NOT_AVAILABLE`、`MANAGED_CONFIG_DRIFT`、`MODEL_STATE_INVALID`、`AGENT_OPERATION_BUSY` 和 `CODEX_AUTH_UNSUPPORTED` 都不会触发静态模型、旧目录 cache、隐式复用现有模型、替换模型或部分文件变更。处理方式见[故障排查](TROUBLESHOOTING.md#模型配置错误)。

## 所有权、迁移与备份

Manager 只在私有 `agent-transactions/last-applied-model-config.json` sidecar 中记录规范已选模型 section、owned path、target path 和 keyed revision MAC。它不存储 key、目录、渲染文件、原始响应或无关 Agent 设置。OS-backed 当前用户 lock 会串行化桌面端与 CLI 操作。

已知 manager-owned 路径可以更新或删除，无关设置会保留。未知 extension 冲突会被拒绝；托管 namespace 漂移必须通过绑定预览的 overwrite 批准。创建任何写入产物前，write 会重新检查 Agent 文件、sidecar revision、router identity 和当前模型可用性。

精确历史 v1 signature 可以迁移：Claude 从整个 `env` 替换改为 managed-key merge；opencode 从固定模型改为选定目录子集；Codex 从 `custom` 改为 `mtls-router`，并单独批准 auth 迁移。部分匹配或已修改的历史 signature 不会被认领。迁移绝不删除或重写已有备份。

已有文件与 sidecar 在同一个 journaled transaction 中备份和修改。适用时备份保留在源文件旁，使用私有权限，并可能包含当前或旧 API key，必须按敏感数据处理。Rollback 会一起恢复文件与 sidecar；恢复未解决时不要删除事务状态或备份。

## Protocol v2 自动化

自动化必须使用 receipt-verified `mtls-router-manager serve`。先调用 `manager.info` 并要求 management protocol `2`，然后调用：

1. `agent.models`，提供 `owner`、`agents` 和临时 `api_key`。
2. `agent.render` 获取 key 脱敏托管片段；或使用 `agents`、`catalog_token`、`model_config` 调用 `agent.preview`。
3. `agent.write`，提供上述字段、预览返回的 `revision_token`、两个显式 approval boolean 和临时 `api_key`。

Key 只能出现在两次 secret-bearing stdin/IPC 请求体中。不得放入参数、环境变量、model config、日志、shell history 或临时请求文件。Protocol v1 请求以及混合 v1/v2 router、manager、setup receipt 或 desktop artifact 都会被拒绝；必须整体更新同一 release。

精确请求/结果 schema 和稳定错误由仓库内 canonical JSON Schema 与 manager protocol type 定义。测试使用的 Agent revision 与 source digest 记录在 [`internal/manager/agent/testdata/compatibility.json`](../../internal/manager/agent/testdata/compatibility.json)。
