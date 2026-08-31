# internal/manager/app

把各 manager 服务装配到私有 JSON 协议上：19 个方法的 handler、错误码映射、API key 清零，以及无 key Agent cleanup 的直接分发。

## 文件

| 文件 | 职责 |
|------|------|
| `app.go` | `App`、`New(Config, simplify)`、`Serve(ctx, input, output)`；19 个方法 handler；cleanup detection/preview/write 与 `apikey.usage` 显式映射；结果与错误码映射；启动失败锁存与诊断 |

## 结构

- `Config` 提供装配所需的一切：`RouterPath`、`ListenAddr`、`DesktopSession`、`ParentIdentity`、`ManagerIdentity`、`Paths`、`AgentDetector`、`Stderr`。
- `New` 构造 `lifecycle.Manager`、`discovery.Discoverer`、`agent.Service`、`occupant.Service`、`trustedrouter.Coordinator`，再交给 `protocol.NewServer`。
- 每个协议方法对应一个 `func(ctx, json.RawMessage) (any, *protocol.Error)`，方法与 handler 的映射表就是协议契约的唯一实现点。

## 关键不变量

- **API key 清零**：请求成功 decode 后立刻把 `request.APIKey` 置空（尽力而为 —— 底层 JSON/Scanner 缓冲区由 GC 管理，不保证清零）。key 绝不进入日志、状态文件或 journal。
- **cleanup 直接分发**：`agent.cleanup.preview/write` 严格解码单 Agent 参数后直接调用 `agent.Service`，不调用 catalog binder、trusted router、普通 preview/write handler，也不接收或清理 API key；`AGENT_NOT_MANAGED` 映射为稳定协议错误。
- **诊断脱敏**：`lifecycle` 保留有界的**原始**子进程输出，`app` 只向客户端暴露脱敏的、会话作用域的稳定启动阶段、数值 OS 错误码和同次启动输出（`startupDiagnostic`）。`diagnostics.collect` 另给出结构化 `router_state`；manager 启动前失败阶段由桌面 Rust 侧环缓冲合并，不回传原始 stderr。
- **启动失败锁存**：`latchStartupFailure` / `failedStatus` / `clearFailureAfterStart` 让一次失败的启动在后续 `router.status` 中仍可见，而不是被下一次轮询抹掉。
- **受保护 PID**：`protectedStatePID` 阻止强制终止落到状态文件记录的自有进程上。
- 服务层的错误（`agent.OperationError`、`lifecycle.Error`、`modelcatalog.Error`）在此统一映射为稳定的 `protocol.ErrorCode`；错误消息只作诊断用途。

## 依赖

装配层，依赖全部业务子包：`../protocol`、`../lifecycle`、`../discovery`、`../agent`、`../agent/modelconfig`、`../occupant`、`../trustedrouter`、`../metadata`、`../paths`、`../preset`、`../process`、`../state`、`../modelcatalog`

## 测试

- `app_test.go` —— 逐方法的协议契约断言，包括 cleanup key-free 直连、错误脱敏/映射与 key 清零
