# internal/manager/preset

加载构建期注入的不可变 Agent model preset。

## 文件

| 文件 | 职责 |
|------|------|
| `preset.go` | `Encoded`（链接期变量）；`Load()` —— 严格解码并结构化校验内嵌 preset，返回 `*modelconfig.Config` |

## 行为

- `Encoded` 通过 `-ldflags -X github.com/codeasier/mtls-router/internal/manager/preset.Encoded=<base64>` 只注入到 manager 二进制；router 二进制不含该变量。
- `Load()` 走 `modelconfig.DecodeStructural`，即不要求校验 catalog，仅做结构合法性检查。

## 关键不变量

- `Load()` 返回的错误**刻意是常量文本**，不携带解码细节，以免 preset 内容经错误消息泄露。
- preset 是无 key 数据（由 `modelconfig` schema v1 硬约束保证）；任何 key-like 字段都会在解码阶段被拒绝。
- preset 值对用户始终可编辑，不代表任何批准 —— 见 [`../agent/INDEX.md`](../agent/INDEX.md)。

## 依赖

- `internal/manager/agent/modelconfig` —— `Config`、`DecodeStructural`
