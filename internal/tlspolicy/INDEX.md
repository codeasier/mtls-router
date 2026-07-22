# internal/tlspolicy

TLS minimum version string parsing.

## Files

| File | Role |
|------|------|
| `minversion.go` | `MinVersion(version string)` → `(uint16, error)` |

## Mapping

| Input | Output |
|-------|--------|
| `""` or `"tls1.2"` | `tls.VersionTLS12` |
| `"tls1.3"` | `tls.VersionTLS13` |
| anything else | error |

## Consumers

- `internal/config` — flag validation
- `internal/proxy/transport.go` — transport construction
- `internal/health/probe.go` — prober construction
