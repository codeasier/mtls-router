# AGENTS.md

This file provides guidance to Lingma (lingma.aliyun.com) when working with code in this repository.

## OVERVIEW

`mtls-router` is a Go 1.26 local reverse proxy system for AI coding agents (Claude Code, Codex, opencode). Local clients send plain HTTP; upstream traffic is HTTPS with embedded mTLS credentials. The system has three tiers:

1. **mtls-router** — single-binary reverse proxy with SSE streaming, health probing, background mode
2. **mtls-router-manager** — stdin/stdout JSON-protocol control plane for router lifecycle and Agent configuration
3. **Tauri desktop app** — React + Rust GUI that spawns the manager as a sidecar and communicates via the JSON protocol

## COMMANDS

### Go (router + manager)

```bash
go test ./...                          # all Go tests
go test ./internal/proxy/...           # single package
go test -run TestName ./internal/proxy # single test by name
go vet ./...
test -z "$(gofmt -l .)"               # formatting check (CI enforced)
./scripts/build.sh                     # local build (generates placeholder certs + both binaries)
```

### Shell integration tests

```bash
make test-shell                        # runs all tests/setup_*_test.sh in temp dirs
bash tests/setup_clean_test.sh         # run one shell test directly
make test-workflows                    # desktop + agent preset + release packaging workflow tests
```

### Desktop frontend (desktop/ directory)

```bash
cd desktop && npm ci
npm run static:check                   # eslint + prettier
npm run typecheck                      # tsc --noEmit
npm test                               # vitest run
npm run build                          # tsc + vite build
npm run verify                         # all of the above + rust format + rust test
```

### Desktop Rust (desktop/src-tauri/)

```bash
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked
```

### Desktop full package build

```bash
make desktop-package-current           # sidecars:build → tauri build → package:verify
```

## ARCHITECTURE

### Three-tier data flow

```
Tauri UI (React) ──invoke──▶ Rust commands ──stdin/stdout JSON──▶ mtls-router-manager ──spawn/HTTP──▶ mtls-router ──mTLS──▶ upstream
```

The desktop app never talks to the router directly. It spawns `mtls-router-manager serve` as a long-lived child process and exchanges newline-delimited JSON requests/responses over stdin/stdout.

### Router (`main.go` + `internal/`)

`run()` orchestrates: meta flags → config.Load (flag > env > build-time > default) → mTLS transport → upstream probe → reverse proxy + mux → graceful shutdown.

Key invariants:
- `/version` and `/health` are registered before `/` so they take precedence over the proxy route
- `/health` always returns HTTP 200; degradation is in the JSON body
- `FlushInterval: -1` on the reverse proxy enables unbuffered SSE streaming
- mTLS credentials are link-time variables (`main.clientCertPEM`, `main.clientKeyPEM`, `main.upstreamCAPEM`, `main.upstreamURL`) injected via `-ldflags -X`
- Startup probe failure exits non-zero; the router never accepts traffic with broken upstream

### Manager (`cmd/mtls-router-manager/` + `internal/manager/`)

The manager is a stateless-per-request JSON protocol server (`internal/manager/protocol/`). It exposes 15 methods grouped as:

- `manager.info`, `diagnostics.collect` — metadata
- `router.status/start/stop/health/version/logs` — router lifecycle (spawns/monitors the router binary)
- `router.inspect_occupant/force_terminate_occupant` — port conflict resolution
- `agent.detect/models/render/preview/write` — Agent configuration (Claude Code, opencode, Codex)

Sub-package responsibilities:
- `app` — wires all services, maps protocol errors, enforces API key zeroing
- `lifecycle` — process spawn, state file, parent monitoring, unexpected exit detection
- `discovery` — classifies router state (desktop_owned / external_compatible / degraded / stale / absent)
- `agent` — detection, config rendering (per-agent format: JSON/TOML), transactional write with backup/rollback
- `agent/modelconfig` — canonical key-free model config schema v3, validation, merge
- `trustedrouter` — authenticated model catalog discovery via router `/v1/models`
- `occupant` — port occupant identity inspection and guarded force-termination
- `protocol` — request/response types, method deadlines, error codes
- `state` — JSON state file read/write for router process identity
- `process` — PID + start-time + executable triple identity verification

### Desktop app (`desktop/`)

**Frontend** (React 19 + TypeScript + Vite):
- `src/ipc.ts` — typed `DesktopApi` interface wrapping Tauri invoke commands; all sensitive text is sanitized client-side
- `src/App.tsx` — 4 sections: Router, Agents, Logs, Settings
- `src/model.ts` — shared types and navigation model
- i18n: `src/locales/zh-CN.ts` and `src/locales/en.ts`

**Backend** (Rust, Tauri 2):
- `src/commands.rs` — Tauri command handlers that proxy to the manager client
- `src/manager.rs` — spawns and communicates with `mtls-router-manager serve` over stdin/stdout
- `src/scheduler.rs` — poll scheduler that emits `router-poll-snapshot` events to the frontend
- `src/sidecar.rs` — resolves and validates sidecar binary paths (target-triple naming)
- `src/tray.rs` — system tray with status-aware menu
- `src/orchestration.rs` — first-launch flow (auto-start router if sidecar is valid)
- `src/model_config.rs` — model config import/export validation

The Rust side never exposes shell/fs/http permissions to the webview (enforced by test in `lib.rs`).

### Setup scripts (`setup.sh` / `setup.ps1`)

Router lifecycle (`router install/start/stop/status/setup`) and Agent configuration (`agent print-config/write-config`) are intentionally separate command groups. The scripts install both `mtls-router` and `mtls-router-manager` together, verify SHA-256 from `SHA256SUMS`, and use transactional install with pending markers.

## CONVENTIONS

- Go files must be `gofmt`-clean; CI enforces `test -z "$(gofmt -l .)"`.
- Go tests live beside packages as `*_test.go`; shell integration tests live under `tests/setup_*_test.sh`.
- Preserve README/docs parity in English and `docs/zh-CN/` when changing user-visible behavior.
- The `mtls-router` binary does router lifecycle only. Agent configuration belongs to the manager and setup scripts.
- `slog` is the logging API; access logs record method/path/status/bytes/latency, never request bodies.
- Runtime flags have env twins prefixed `MTLS_`; precedence is `flag > env > build-time > default`.
- Release artifacts: `mtls-router-${GOOS}-${GOARCH}` with `.exe` only for Windows.
- `setup.ps1` must keep a UTF-8 BOM for Windows PowerShell 5.1; `main_test.go` asserts this.
- Desktop sidecar build inputs use target-triple naming in `src-tauri/binaries/` (`mtls-router-<target>`); after Tauri packaging, installed binaries use plain names (`mtls-router`, `mtls-router-manager`).
- Manager protocol error codes are stable for branching; messages are diagnostic only.
- API keys are confined to short-lived, zeroizable memory: manager zeroes `request.APIKey = ""` after successful decode; desktop holds the key in `ModelFlow.api_key: Zeroizing<String>` from model discovery until config write, then memory is zeroed on drop. Keys never appear in env vars, CLI args, model config, logs, or temp files.

## ANTI-PATTERNS

- Do not commit real secrets; `.gitignore` marks `secrets/` as never-commit.
- Do not expose `/version` or `/health` publicly; `/version` includes commit/build metadata.
- Do not pass `-backend` under service managers (NSSM/systemd/Docker); supervisors own backgrounding.
- Do not buffer request bodies; let `httputil.ReverseProxy` stream them.
- Do not add pass-through request-pipeline wrappers in `internal/proxy`; compose hooks at the mux call site.
- Do not leak upstream/certificate/key details in error JSON; tests assert sanitization.
- Do not make `/health` fail at the HTTP layer for degraded upstream.
- Do not partially overwrite `secrets/`; all three files must exist or all three placeholders are generated.
- Do not put API keys in environment variables, CLI arguments, model config, logs, or temp files.
- Do not grant the desktop webview shell/fs/http/opener capabilities beyond `core:default`.

## BUILD METADATA

Link-time variables injected via `-ldflags -X`:
- `main.clientCertPEM`, `main.clientKeyPEM`, `main.upstreamCAPEM`, `main.upstreamURL` (router only)
- `github.com/codeasier/mtls-router/internal/version.Version/Commit/BuildDate/DeploymentID` (both binaries)
- `github.com/codeasier/mtls-router/internal/manager/preset.Encoded` (manager only, base64 agent model preset)
- `github.com/codeasier/mtls-router/internal/manager/modelcatalog.Simplify` (manager only, model filtering policy)

## PACKAGE INDEX

Each sub-package has a dedicated `INDEX.md` with detailed file maps, exports, invariants, and dependencies. Read the relevant INDEX.md when working in that area.

| Package | Index | Scope |
|---------|-------|-------|
| `internal/proxy` | [INDEX.md](internal/proxy/INDEX.md) | Reverse proxy, mTLS transport, SSE streaming, error sanitization |
| `internal/background` | [INDEX.md](internal/background/INDEX.md) | Detached child process, log file, arg rewriting |
| `internal/config` | [INDEX.md](internal/config/INDEX.md) | Flag/env/build-time precedence, validation |
| `internal/health` | [INDEX.md](internal/health/INDEX.md) | Upstream mTLS reachability probe |
| `internal/routermeta` | [INDEX.md](internal/routermeta/INDEX.md) | `/version` and `/health` handlers |
| `internal/certs` | [INDEX.md](internal/certs/INDEX.md) | PEM parsing into client cert + CA pool |
| `internal/version` | [INDEX.md](internal/version/INDEX.md) | Link-time build metadata variables |
| `internal/log` | [INDEX.md](internal/log/INDEX.md) | Access log response recorder |
| `internal/tlspolicy` | [INDEX.md](internal/tlspolicy/INDEX.md) | TLS minimum version parsing |
| `internal/manager` | [INDEX.md](internal/manager/INDEX.md) | Control plane: 14 sub-packages, 15 protocol methods, lifecycle, discovery, agent config |
| `desktop` | [INDEX.md](desktop/INDEX.md) | Tauri 2 app: React frontend + Rust backend, sidecar management |

## NOTES

- `docs/superpowers/` contains historical specs/plans; not source-of-truth for current behavior.
- `scripts/build.sh` creates placeholder PEMs under `secrets/` for local builds; real release secrets come from GitHub secrets/vars.
- The `.worktrees/` directory contains git worktree artifacts; ignore when reasoning about product code.
- Management protocol version is currently `3`; the desktop handshake validates this on startup.
