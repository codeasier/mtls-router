# internal/manager/trustedrouter

模型目录发现所需的「通道绑定的 router 信任」：确认对面确实是本代 router，才把 API key 送出去。

## 文件

| 文件 | 职责 |
|------|------|
| `coordinator.go` | `Coordinator`、`Fetch(ctx, owner, apiKey)`、`Revalidate(ctx, owner, apiKey, binding)`；`Binding`、`Result`、`Lifecycle` 接口 |
| `channel.go` | `Channel.Fetch(...)` —— 在已确认可信的 router 上取目录；`versionMatches`、`trustedStateMatches` 等信任判据 |
| `listener.go` | `Listener`、`NormalizeListener(value)` —— 规范化监听地址并派生 `RouterBaseURL` / `APIBaseURL` |

## 行为

- `Coordinator.Fetch` 先通过注入的 `Discover` 判断 router 状态；必要且被允许时经 `Lifecycle` 启动 router，再由 `Channel` 取目录。`legacy_managed` 与 stale/unknown 一样不会自动启动，避免把 API key 送给旧协议世代。
- `Binding` 记录本次取目录时的 `RouterBaseURL`、`APIBaseURL`、`DeploymentID`、`ProtocolVersion`。
- `Revalidate` 在**写入 Agent 配置前**用同一个 binding 重新确认 router 身份未变，避免「取目录时是 A、写配置时已换成 B」。

## 关键不变量

- API key 只发往通过身份校验的 router：deployment ID、管理协议版本与 listener 都必须与 binding 一致，否则直接失败而不是降级重试。
- 失败一律 fail closed —— 发现失败、目录过期都返回协议错误，不返回空目录冒充成功。
- 本包返回 `*protocol.Error` 而非裸 error，使调用方拿到的错误码可直接回给客户端。

## 依赖

- `../discovery` —— router 状态与版本
- `../lifecycle` —— 需要时启动 router；`lifecycleProtocolError` 做错误码映射
- `../modelcatalog` —— 实际的目录 HTTP 请求
- `../process`、`../state`、`../protocol`
