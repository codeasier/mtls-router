# internal/log（包名 mlog）

访问日志响应记录器与结构化日志中间件。

## 文件

| 文件 | 职责 |
|------|------|
| `log.go` | `ResponseRecorder`（包装 `http.ResponseWriter`，捕获 status/bytes）；`AccessLog(logger, req, rec, start)` |

## 行为

- `ResponseRecorder` 实现 `http.ResponseWriter` + `Unwrap()`，兼容 `http.ResponseController`。
- `AccessLog` 发出 `slog.Info("access", ...)`，含 method/path/status/bytes/latency。
- 请求体刻意绝不记录。

## 用法

作为中间件在 `main.go:withAccessLog` 中应用，仅包装反向代理 handler（不包装 `/version` 或 `/health`）。
