# internal/tlspolicy

TLS 最低版本字符串解析。

## 文件

| 文件 | 职责 |
|------|------|
| `minversion.go` | `MinVersion(version string)` → `(uint16, error)` |

## 映射

| 输入 | 输出 |
|-------|--------|
| `""` 或 `"tls1.2"` | `tls.VersionTLS12` |
| `"tls1.3"` | `tls.VersionTLS13` |
| 其他 | error |

## 消费者

- `internal/config` —— flag 校验
- `internal/proxy/transport.go` —— transport 构造
- `internal/health/probe.go` —— prober 构造
