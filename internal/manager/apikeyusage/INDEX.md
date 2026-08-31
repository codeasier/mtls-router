# internal/manager/apikeyusage

经可信 router 拉取**当前 API key** 的有界用量快照。只接受 `GET /v1/usage?period=`，响应必须只描述调用密钥自身。

## 文件

| 文件 | 职责 |
|------|------|
| `client.go` | `Client`、`New(transport)`、`Fetch(ctx, Request)`；`Request{URL, Period, APIKey}`；`Error` 与 `CodeOf(err)` → `protocol.ErrorCode` |
| `parse.go` | `Period`、`Snapshot`、`NormalizePeriod()`、`Parse(body, period)` —— 流式解码并校验有界 per-key 用量 |

## 上游契约

`GET /v1/usage?period=today|7d|30d`，`Authorization: Bearer <api_key>`。本地 router 原样转发；jump 只放行 `/v1/*`，因此用量必须挂在 `/v1/usage`，不能走 `/api/usage`。

成功体最少包含：

```json
{
  "period": "7d",
  "summary": {
    "requests": 0,
    "prompt_tokens": 0,
    "completion_tokens": 0,
    "cost": 0
  },
  "by_model": []
}
```

可选：`as_of`（RFC3339）、`quota:{used,limit,unit,resets_at}`。`limit` 可为 `null`（无上限）。`unit` 只能是 `usd`、`tokens`、`requests`。`period` 必须回显请求窗口。

状态映射：`401/403` → `USAGE_AUTH_FAILED`；`404/405/501` → `USAGE_UNAVAILABLE`；其他非 200 → `USAGE_REQUEST_FAILED`；体不合法 → `USAGE_RESPONSE_INVALID`。

## 关键不变量

- API key 只以 HTTP 头发出，**绝不进入日志、错误消息或返回值**。
- 解析拒绝 `api_key` / `token` / `secret` 等字段名，未知安全字段跳过；`by_model` 最多 64 行。
- 计数必须是非负整数；费用与配额必须是有限非负数。
- 失败一律 fail closed，不把上游原文或空快照冒充成功。

## 依赖

- `../protocol` —— 错误码

## 被依赖

- `../trustedrouter` —— 在校验过绑定的通道上调用 `Fetch`
