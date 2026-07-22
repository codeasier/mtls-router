# internal/version

Link-time build metadata variables and accessors.

## Files

| File | Role |
|------|------|
| `version.go` | `Version`, `Commit`, `BuildDate`, `DeploymentID` vars; `ManagementProtocolVersion` const; `Info()`, `InfoJSON()` |

## Link-time variables

Set via `-ldflags -X github.com/codeasier/mtls-router/internal/version.*`:

| Variable | Default | Description |
|----------|---------|-------------|
| `Version` | `"dev"` | Semantic version / git tag |
| `Commit` | `"unknown"` | Short git SHA |
| `BuildDate` | `"unknown"` | UTC ISO-8601 build timestamp |
| `DeploymentID` | `"dev"` | Service environment identifier |

## Constants

- `ManagementProtocolVersion = "3"` — code-owned, cannot be overridden at link time.

## Injection sites

- `scripts/build.sh` (local)
- `Dockerfile` (container)
- `.github/workflows/release.yml` (release)
- `desktop/scripts/build-sidecars.sh` (desktop sidecars)
