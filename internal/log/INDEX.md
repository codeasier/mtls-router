# internal/log (package mlog)

Access log response recorder and structured logging middleware.

## Files

| File | Role |
|------|------|
| `log.go` | `ResponseRecorder` (wraps `http.ResponseWriter`, captures status/bytes); `AccessLog(logger, req, rec, start)` |

## Behavior

- `ResponseRecorder` implements `http.ResponseWriter` + `Unwrap()` for `http.ResponseController` compatibility.
- `AccessLog` emits `slog.Info("access", ...)` with method/path/status/bytes/latency.
- Request bodies are intentionally never logged.

## Usage

Applied as middleware in `main.go:withAccessLog` wrapping the reverse proxy handler only (not `/version` or `/health`).
