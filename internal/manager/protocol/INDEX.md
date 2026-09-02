# internal/manager/protocol

`mtls-router-manager` 客户端使用的私有 stdin/stdout JSON 契约（management protocol v4）。

## 文件

| 文件 | 职责 |
|------|------|
| `types.go` | `Method` 常量（19 个）、`ErrorCode` 常量、`Request`/`Response`/`Error`、cleanup detection/preview/write、`apikey.usage` 与其他方法的 params/result 类型、`Deadlines()`、`IsLegacyLineageVersion`（protocol 1/3 遗留祖先集合的唯一事实来源） |
| `server.go` | `Server`、`NewServer(map[Method]Handler)`、`Serve(ctx, input, output)`、`DecodeParams`；请求体积上限、逐行解析、所有 object depth 的重复/未知字段拒绝 |

## 协议方法（19 个）

```
manager.info              diagnostics.collect
router.status             router.start            router.migrate_legacy
router.stop
router.health             router.version          router.logs
router.inspect_occupant   router.force_terminate_occupant
agent.detect              agent.models            agent.render
agent.preview             agent.write
agent.cleanup.preview     agent.cleanup.write
apikey.usage
```

`Deadlines()` 为**每个**方法返回必需的内部超时；新增方法必须同时在此登记。`apikey.usage` 为 60 秒：启动 20 秒 + 可信 `/version` 15 秒 + 用量聚合 25 秒，三段独立，避免启动或校验吃掉慢聚合预算。

## 错误码

稳定、可用于客户端分支判断，分为几组：

- 协议层：`INVALID_REQUEST`、`UNKNOWN_METHOD`、`INVALID_PARAMS`、`OPERATION_TIMEOUT`
- sidecar：`SIDECAR_MISSING`、`SIDECAR_INVALID`
- router 生命周期：`ROUTER_NOT_FOUND`、`ROUTER_ALREADY_RUNNING`、`ROUTER_START_FAILED`、`ROUTER_NOT_READY`、`ROUTER_DEGRADED`、`ROUTER_NOT_OWNED`、`ROUTER_STATE_STALE`、`ROUTER_LEGACY_MANAGED`、`PORT_OCCUPIED`
- 端口占用恢复：`OCCUPANT_NOT_FOUND`、`OCCUPANT_NOT_OWNED`、`OCCUPANT_IDENTITY_UNAVAILABLE`、`OCCUPANT_CHANGED`、`OCCUPANT_PROTECTED`、`OCCUPANT_PERMISSION_DENIED`、`OCCUPANT_TERMINATION_FAILED`、`PORT_RELEASE_TIMEOUT`、`CONFIRMATION_EXPIRED`
- Agent 配置/清理：`AGENT_NOT_FOUND`、`AGENT_NOT_MANAGED`、`CONFIG_INVALID`、`CONFIG_NOT_WRITABLE`、`PREVIEW_STALE`、`BACKUP_FAILED`、`WRITE_FAILED`、`ROLLBACK_FAILED`，以及 model catalog/state/drift/busy/Codex auth 系列错误
- API key 用量：`USAGE_AUTH_FAILED`、`USAGE_UNAVAILABLE`、`USAGE_REQUEST_FAILED`、`USAGE_RESPONSE_INVALID`

## 关键不变量

- **错误码稳定、可分支；错误消息仅作诊断用途**，客户端不得据消息文本判断行为。
- 单个请求上限 4 MiB（`maxRequestSize`）；超限按协议错误处理而非撕裂连接。
- 请求/响应为换行分隔的 JSON，解析时容忍 `\r\n`（Windows 客户端）。
- `boundErrorDetails` 对错误 detail 做上限约束，避免把无界内容回传给客户端。
- 协议版本 `4` 由 `internal/version.ManagementProtocolVersion` 常量提供，不经 `-ldflags` 注入。
- 可显式迁移的 protocol 1/3 遗留祖先集合只由 `IsLegacyLineageVersion` 判定；`../discovery` 分类与 `../lifecycle` 迁移门禁都必须经它判断，禁止在调用方重新硬编码版本字面量。
- `router.migrate_legacy` 是私有协议中的向后兼容增量方法：不改变既有方法的请求/响应 schema，且桌面包通过内嵌 sidecar 哈希与 `manager.info` 握手拒绝混合组件，因此继续使用 v4；若修改既有方法的必需字段或语义，则必须升级协议版本。
- `router.migrate_legacy` deadline 为 27 秒，只供显式 legacy stop-and-restart；Rust 客户端将其与 `router.stop` 一并视为不确定投递后不可重放。
- Cleanup preview/write deadline 分别为 5 秒和 30 秒；请求严格只接受单个 `agent`，write 另接受 revision token 与显式漂移批准，不接受 API key、catalog/model config、flow 或批量 Agents。

## 依赖

- 仅标准库。本包刻意不依赖任何其他 manager 子包，以便所有子包都能引用它的错误码。
