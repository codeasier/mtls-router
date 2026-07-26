# internal/manager/agent/modelconfig

权威的、**无 key** 的 Agent model config schema v1：解码、校验、合并、规范化序列化与 token 签名。

## 文件

| 文件 | 职责 |
|------|------|
| `types.go` | `Config`、`Agent`、`Model`、`ClaudeConfig`/`ClaudeRole`/`ClaudeContext`、`OpenCodeConfig`/`OpenCodeModelConfig`、`CodexConfig`、`Limit`、`Modalities`、`InterleavedField` |
| `decode.go` | `Decode(data, selected, catalog)`、`DecodeStructural(data)`；严格 JSON 解码（校验 Unicode 转义、拒绝重复键） |
| `validate.go` | 各 agent section 的解析与校验（`parseClaude`/`parseOpenCode`/`parseModel`/`parseRole`/`parseLimit` 等） |
| `canonical.go` | `Canonical(config)`、`CanonicalValue(value)` —— 规范化 JSON 序列化：键按 UTF-16 码元序排序、数字固定格式 |
| `merge.go` | `DeepMerge(base, overlay)` 及深拷贝辅助 |
| `token.go` | `TokenSigner`、`NewTokenSigner(key, generation)`；`SignCatalog`/`VerifyCatalog`、`SignRevision`/`VerifyRevision`、`RevisionMAC`；`CatalogClaims`、`RevisionClaims` 等 |
| `schema.go` | `GenerateSchema()` —— 导出 JSON Schema |

## 两种解码模式

- `Decode(data, selected, catalog)` —— 完整校验：要求每个模型 ID 都在给定 catalog 内，用于用户提交的配置。
- `DecodeStructural(data)` —— 只做结构校验，不要求 catalog，用于加载构建期注入的 preset（见 [`../../preset/INDEX.md`](../../preset/INDEX.md)）。

## 关键不变量

- **无 key 是设计硬约束**：schema 主动拒绝 key-like 字段名。model config 在任何环节都不得携带 API key。
- **规范化必须稳定**：`Canonical` 按 UTF-16 码元序排键、数字用固定格式。签名与摘要都建立在它之上，序列化一旦不稳定，token 校验就会假阴性。
- token 只承载摘要与声明，**不含 key**；sidecar 状态文件与事务 journal 同样只存 HMAC 摘要。
- `Decode` 的错误经 `ValidationError` 携带字段路径，便于前端定位，但不回显被拒绝的值本身。

## 依赖

- 仅标准库（`crypto/hmac`、`encoding/json`、`bytes` 等）。刻意不依赖其他 manager 子包，使其可被 `preset`、`agent`、`app` 共同引用。

## 测试

- `modelconfig_test.go` —— 解码/校验/合并/规范化/签名的向量测试；桌面前端另有 `desktop/src/modelConfigVectors.test.ts` 对齐同一批向量
