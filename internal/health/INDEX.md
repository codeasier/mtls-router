# internal/health

Upstream mTLS reachability probing at startup and on-demand via `/health`.

## Files

| File | Role |
|------|------|
| `probe.go` | `Prober` struct with `NewProber(ProbeOptions)`, `Probe()`, `Close()`; one-shot `Probe(opts)`; `ProbeFunc` type |

## Behavior

- `NewProber` builds a dedicated `http.Client` with mTLS transport (same cert/key/CA as the proxy).
- `Probe()` sends GET to the upstream URL with a timeout; status >= 500 is a failure.
- Startup: probe failure → process exits non-zero (never accepts traffic with broken upstream).
- Runtime `/health`: probe failure → HTTP 200 with `{"status":"degraded"}` (never fails at HTTP layer).
- `ProbeFunc` is `func() error` — used by `routermeta.HealthHandler` and test stubs.

## Dependencies

- `internal/certs` — PEM → TLS certificate
- `internal/tlspolicy` — TLS min version
