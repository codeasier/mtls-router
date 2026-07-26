# internal/manager/modelcatalog

获取并规范化 router 暴露的有界模型目录。

## 文件

| 文件 | 职责 |
|------|------|
| `client.go` | `Client`、`New(transport, simplify)`、`Fetch(ctx, Request)`；`Request{URL, APIKey}`；`Error` 与 `CodeOf(err)` → `protocol.ErrorCode` |
| `parse.go` | `Parse(body, simplify)` —— 流式解码 `/v1/models` 响应，去重并校验模型 ID |
| `simplify.go` | `Simplify`（链接期变量）、`ParseSimplify()` |

## 行为

- `Parse` 用 `json.Decoder` 逐项解码而非整体反序列化，未知字段直接跳过，避免上游新增字段导致解析失败。
- `simplify` 为真时过滤掉含 `/` 的模型 ID（供应商前缀形式），只保留简单 ID。
- `Simplify` 由 `-ldflags -X github.com/codeasier/mtls-router/internal/manager/modelcatalog.Simplify=<bool>` 注入，仅存在于 manager 二进制；默认 `"True"`。

## 关键不变量

- API key 只以 HTTP 头形式随请求发出，**绝不进入日志、错误消息或返回值**。
- `Error` 只携带稳定错误码；`CodeOf` 把它映射为协议错误码供客户端分支判断。
- 目录是有界的：解析阶段就限制数量与 ID 合法性，不把上游任意内容原样透传给前端。

## 依赖

- `../protocol` —— 错误码

## 被依赖

- `../trustedrouter` —— 在校验过绑定的通道上调用 `Fetch`
