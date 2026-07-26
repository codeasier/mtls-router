# internal/manager/protocol

`mtls-router-manager` 客户端使用的私有 stdin/stdout JSON 契约（management protocol v4）。

## 文件

| 文件 | 职责 |
|------|------|
| `types.go` | `Method` 常量（15 个）、`ErrorCode` 常量、`Request`/`Response`/`Error`、各方法的 params/result 类型、`Deadlines()` |
| `server.go` | `Server`、`NewServer(map[Method]Handler)`、`Serve(ctx, input, output)`、`DecodeParams`；请求体积上限与逐行解析 |

## 协议方法（15 个）

```
manager.info              diagnostics.collect
router.status             router.start            router.stop
router.health             router.version          router.logs
router.inspect_occupant   router.force_terminate_occupant
agent.detect              agent.models            agent.render
agent.preview             agent.write
```

`Deadlines()` 为**每个**方法返回必需的内部超时；新增方法必须同时在此登记。

## 错误码

稳定、可用于客户端分支判断，分为几组：

- 协议层：`INVALID_REQUEST`、`UNKNOWN_METHOD`、`INVALID_PARAMS`、`OPERATION_TIMEOUT`
- sidecar：`SIDECAR_MISSING`、`SIDECAR_INVALID`
- router 生命周期：`ROUTER_NOT_FOUND`、`ROUTER_ALREADY_RUNNING`、`ROUTER_START_FAILED`、`ROUTER_NOT_READY`、`ROUTER_DEGRADED`、`ROUTER_NOT_OWNED`、`ROUTER_STATE_STALE`、`PORT_OCCUPIED`
- 端口占用恢复：`OCCUPANT_NOT_FOUND`、`OCCUPANT_NOT_OWNED`、`OCCUPANT_IDENTITY_UNAVAILABLE`、`OCCUPANT_CHANGED`、`OCCUPANT_PROTECTED`、`OCCUPANT_PERMISSION_DENIED`、`OCCUPANT_TERMINATION_FAILED`、`PORT_RELEASE_TIMEOUT`、`CONFIRMATION_EXPIRED`
- Agent 配置：`AGENT_NOT_FOUND`、`CONFIG_INVALID`、`CONFIG_NOT_WRITABLE`、`PREVIEW_STALE`、`BACKUP_FAILED`、`WRITE_FAILED`、`ROLLBACK_FAILED`

## 关键不变量

- **错误码稳定、可分支；错误消息仅作诊断用途**，客户端不得据消息文本判断行为。
- 单个请求上限 4 MiB（`maxRequestSize`）；超限按协议错误处理而非撕裂连接。
- 请求/响应为换行分隔的 JSON，解析时容忍 `\r\n`（Windows 客户端）。
- `boundErrorDetails` 对错误 detail 做上限约束，避免把无界内容回传给客户端。
- 协议版本 `4` 由 `internal/version.ManagementProtocolVersion` 常量提供，不经 `-ldflags` 注入。

## 依赖

- 仅标准库。本包刻意不依赖任何其他 manager 子包，以便所有子包都能引用它的错误码。
