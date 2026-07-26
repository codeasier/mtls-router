# internal/routermeta

router 监听器上 `/version` 与 `/health` 端点的管理 HTTP handler。

## 文件

| 文件 | 职责 |
|------|------|
| `handlers.go` | `VersionHandler(InfoProvider)`、`HealthHandler(health.ProbeFunc)` —— 均返回 `http.Handler` |

## 行为

- **`/version`**：仅 GET，`no-store` JSON，含 version/commit/build_date/deployment_id/management_protocol_version/pid/started_at。构建身份字段不可被 InfoProvider 覆盖。
- **`/health`**：仅 GET，始终 HTTP 200。body 为 `{"status":"ok","upstream":"reachable"}` 或 `{"status":"degraded","upstream":"unreachable","error":"..."}`。
- 非 GET 方法返回 405 并带 `Allow: GET`。

## 关键不变量

- 这些端点必须以精确 pattern 注册在承载反向代理的同一个 mux 上。ServeMux 按最具体 pattern 匹配，与注册顺序无关，因此它们不会被 `/` 捕获并转发到 upstream。
- `/health` 绝不可返回非 200 的 HTTP 状态 —— setup 脚本依赖「connection refused = 未启动，200 = 已启动」。
- `/version` 暴露精确构建元数据 —— 绝不公开暴露。

## 依赖

- `internal/health` —— `ProbeFunc` 类型
- `internal/version` —— `Info()` 提供构建元数据
