# internal/config

mtls-router 二进制的 flag/env/default 配置加载与校验。

## 文件

| 文件 | 职责 |
|------|------|
| `config.go` | `Config` 结构体、`Load(defaults, args)`、`Validate()`、`applyEnv()` |

## 优先级

```
flag > env (MTLS_*) > build-time default > hardcoded default
```

## 导出字段

| 字段 | Env | Flag | 默认值 |
|-------|-----|------|---------|
| `ListenAddr` | `MTLS_LISTEN_ADDR` | `-listen` | `127.0.0.1:19099` |
| `UpstreamURL` | `MTLS_UPSTREAM_URL` | `-upstream` | build-time `upstreamURL` |
| `TLSMin` | `MTLS_TLS_MIN` | `-tls-min` | `tls1.2` |
| `Timeout` | `MTLS_TIMEOUT` | `-timeout` | `10s` |
| `Debug` | `MTLS_DEBUG` | `-debug` | `false` |
| `Backend` | `MTLS_BACKEND` | `-backend` | `false` |
| `LogPath` | `MTLS_LOG` | `-log` | `""` |

## 校验规则

- `UpstreamURL` 必须非空、可解析、scheme 为 `https`、host 非空。
- `TLSMin` 必须为 `tls1.2` 或 `tls1.3`（经 `internal/tlspolicy` 校验）。

## 依赖

- `internal/tlspolicy` —— TLS 版本字符串校验
