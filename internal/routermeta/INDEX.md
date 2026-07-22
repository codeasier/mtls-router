# internal/routermeta

Management HTTP handlers for `/version` and `/health` endpoints on the router listener.

## Files

| File | Role |
|------|------|
| `handlers.go` | `VersionHandler(InfoProvider)`, `HealthHandler(health.ProbeFunc)` — both return `http.Handler` |

## Behavior

- **`/version`**: GET-only, `no-store` JSON with version/commit/build_date/deployment_id/management_protocol_version/pid/started_at. Build identity fields cannot be overridden by the InfoProvider.
- **`/health`**: GET-only, always HTTP 200. Body is `{"status":"ok","upstream":"reachable"}` or `{"status":"degraded","upstream":"unreachable","error":"..."}`.
- Non-GET methods receive 405 with `Allow: GET`.

## Key invariants

- These endpoints must be registered before `/` on the mux so they take precedence over the proxy route.
- `/health` must NEVER return a non-200 HTTP status — setup scripts use "connection refused = not started, 200 = started".
- `/version` exposes precise build metadata — never expose publicly.

## Dependencies

- `internal/health` — `ProbeFunc` type
- `internal/version` — `Info()` for build metadata
