# internal/health

启动期与按需（经 `/health`）的 upstream mTLS 可达性探针。

## 文件

| 文件 | 职责 |
|------|------|
| `probe.go` | `Prober` 结构体，含 `NewProber(ProbeOptions)`、`Probe()`、`Close()`；一次性 `Probe(opts)`；`ProbeFunc` 类型 |

## 行为

- `NewProber` 构建专用 `http.Client`，使用 mTLS transport（与代理相同的 cert/key/CA）。
- `Probe()` 带超时向上游 URL 发 GET；status >= 500 视为失败。
- 启动期：探针失败 → 进程非零退出（绝不在 upstream 异常时接收流量）。
- 运行期 `/health`：探针失败 → HTTP 200 + `{"status":"degraded"}`（绝不在 HTTP 层失败）。
- `ProbeFunc` 即 `func() error` —— 供 `routermeta.HealthHandler` 与测试桩使用。

## 依赖

- `internal/certs` —— PEM → TLS 证书
- `internal/tlspolicy` —— TLS 最低版本
