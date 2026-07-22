# internal/config

Flag/env/default configuration loading and validation for the mtls-router binary.

## Files

| File | Role |
|------|------|
| `config.go` | `Config` struct, `Load(defaults, args)`, `Validate()`, `applyEnv()` |

## Precedence

```
flag > env (MTLS_*) > build-time default > hardcoded default
```

## Exported fields

| Field | Env | Flag | Default |
|-------|-----|------|---------|
| `ListenAddr` | `MTLS_LISTEN_ADDR` | `-listen` | `127.0.0.1:19099` |
| `UpstreamURL` | `MTLS_UPSTREAM_URL` | `-upstream` | build-time `upstreamURL` |
| `TLSMin` | `MTLS_TLS_MIN` | `-tls-min` | `tls1.2` |
| `Timeout` | `MTLS_TIMEOUT` | `-timeout` | `10s` |
| `Debug` | `MTLS_DEBUG` | `-debug` | `false` |
| `Backend` | `MTLS_BACKEND` | `-backend` | `false` |
| `LogPath` | `MTLS_LOG` | `-log` | `""` |

## Validation rules

- `UpstreamURL` must be non-empty, parseable, scheme `https`, non-empty host.
- `TLSMin` must be `tls1.2` or `tls1.3` (validated via `internal/tlspolicy`).

## Dependencies

- `internal/tlspolicy` — TLS version string validation
