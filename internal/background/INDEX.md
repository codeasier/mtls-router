# internal/background

Detached child process management and bounded log file handling for `-backend` mode.

## Files

| File | Role |
|------|------|
| `args.go` | `ChildArgs(args, logPath)` — strips `-backend`, ensures `-log` is present; `DefaultLogPath(exePath)` |
| `background.go` | `OpenLogFile`, `OpenBoundedLogWriter(path, maxBytes)` — append-only log with 4MB auto-truncate |
| `background_unix.go` | `Start(exePath, args, logPath)` — unix fork/exec detached child (setsid) |
| `background_windows.go` | `Start(...)` — windows CREATE_NEW_PROCESS_GROUP detached child |
| `env.go` | `ChildEnv(env)` — removes `MTLS_BACKEND`; `DesktopChildEnv(env)` — removes all `MTLS_*` vars |

## Key invariants

- `ChildArgs` must remove `-backend`/`--backend` to prevent infinite re-fork.
- `DesktopChildEnv` strips ALL `MTLS_*` environment variables — desktop launches use explicit flags only.
- `boundedLogWriter` truncates the file (no rename) when size exceeds `DefaultMaxLogBytes` (4MB).
- Log file permissions are `0600`.

## Consumers

- `main.go:startBackend` — CLI background mode
- `internal/manager/lifecycle` — desktop router child process launch
