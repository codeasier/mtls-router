# internal/certs

将 PEM 字符串解析为 TLS client 证书与 upstream CA pool。

## 文件

| 文件 | 职责 |
|------|------|
| `certs.go` | `LoadFromStrings(certPEM, keyPEM, caPEM)` → `(*tls.Certificate, *x509.CertPool, error)` |

## 行为

- 三个输入均必填；空字符串即硬错误。
- client cert/key 用 `tls.X509KeyPair`，CA 用 `x509.CertPool.AppendCertsFromPEM`。
- 启动时由 `proxy.NewMTLSTransport` 与 `health.NewProber` 各调用一次。

## 消费者

- `internal/proxy/transport.go`
- `internal/health/probe.go`
