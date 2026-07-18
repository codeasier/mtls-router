# PROJECT KNOWLEDGE BASE

**Generated:** 2026-06-23
**Commit:** ba002da
**Branch:** main

## OVERVIEW

`mtls-router` is a Go 1.26 single-binary local reverse proxy: local clients send plain HTTP, upstream traffic is HTTPS with embedded mTLS credentials.
Runtime includes transparent request/SSE streaming, startup upstream probing, background mode, and localhost management endpoints.

## STRUCTURE

```text
./
├── main.go                 # CLI flags, startup flow, mux, graceful shutdown
├── setup.sh / setup.ps1    # installer + router/agent configuration workflows
├── scripts/build.sh        # local build with placeholder cert generation
├── internal/
│   ├── proxy/              # reverse proxy, mTLS transport, SSE headers
│   ├── background/         # detached child process + log file handling
│   ├── config/             # flag/env/default precedence and validation
│   ├── health/             # upstream mTLS reachability probe
│   ├── routermeta/         # /version and /health handlers
│   ├── certs/              # PEM parsing into client cert + CA pool
│   ├── version/            # link-time build metadata
│   └── log/                # access log response recorder
├── tests/                  # shell tests for setup scripts and agent config output
├── docs/                   # bilingual user, Agent model, troubleshooting, and release contracts
├── docs/superpowers/       # historical specs/plans; do not treat as live code
├── systemd/                # service unit
└── Dockerfile              # static scratch-image build
```

## WHERE TO LOOK

| Task | Location | Notes |
|---|---|---|
| CLI/runtime wiring | `main.go` | `run()` handles meta flags first, then config, probe, proxy, mux, shutdown. |
| Config precedence | `internal/config/config.go` | Precedence is `flag > env > build-time/default`; README must stay aligned. |
| mTLS setup | `internal/proxy/transport.go`, `internal/certs/` | Release injects PEM strings via `-ldflags -X main.*`. |
| Proxy routing | `internal/proxy/director.go`, `proxy.go` | Preserve path/query/body/header behavior when rewriting upstream. |
| Streaming/SSE | `internal/proxy/modifyresponse.go` | SSE response headers; `FlushInterval: -1` on the reverse proxy keeps the body unbuffered. |
| Upstream readiness | `internal/health/probe.go` | Startup exits non-zero on probe failure; `/health` still returns HTTP 200 with JSON status. |
| Management endpoints | `internal/routermeta/handlers.go` | `/version` and `/health` must take precedence over proxy route. |
| Background mode | `internal/background/`, `main.go:startBackend` | Child args remove `-backend`; default log sits beside binary. |
| Setup behavior | `setup.sh`, `setup.ps1`, `tests/setup_*_test.sh` | Router lifecycle and agent config commands are intentionally separate. |
| Agent model contract | `docs/AGENT_MODELS.md`, `internal/manager/agent/modelconfig/` | Protocol v2 uses authenticated discovery and canonical key-free model config. |
| Release packaging | `.github/workflows/release.yml`, `scripts/build.sh` | Matrix builds inject certs, upstream URL, version, commit, build date. |

## CODE MAP

| Symbol | Type | Location | Role |
|---|---|---|---|
| `run` | func | `main.go` | Orchestrates meta flags, config, mTLS transport, probe, mux, server lifecycle. |
| `handleMetaFlags` | func | `main.go` | Handles `-version`/help and rejects unexpected positional commands. |
| `config.Load` | func | `internal/config/config.go` | Merges defaults, env, flags; validates upstream URL and TLS min. |
| `proxy.New` | func | `internal/proxy/proxy.go` | Builds `httputil.ReverseProxy` with custom director/response/error hooks. |
| `proxy.NewMTLSTransport` | func | `internal/proxy/transport.go` | Parses embedded cert/key/CA and builds TLS client transport. |
| `health.Probe` | func | `internal/health/probe.go` | Performs upstream mTLS reachability check. |
| `routermeta.VersionHandler` | func | `internal/routermeta/handlers.go` | Emits no-store JSON with build/process metadata. |
| `background.ChildArgs` | func | `internal/background/args.go` | Rewrites parent CLI args for detached child process. |

## CONVENTIONS

- Keep Go files `gofmt`-clean; CI enforces `test -z "$(gofmt -l .)"`.
- Tests live beside Go packages as `*_test.go`; setup-script integration tests live under `tests/setup_*_test.sh` and run through `make test-shell`.
- Preserve README/docs parity in English and `docs/zh-CN/` when changing user-visible setup, flags, endpoints, or release behavior.
- The binary does router lifecycle only. Agent configuration belongs to setup scripts (`agent print-config`, `agent write-config --agent=...`), not `mtls-router` CLI commands.
- Build metadata and embedded mTLS values are link-time variables: `main.clientCertPEM`, `main.clientKeyPEM`, `main.upstreamCAPEM`, `main.upstreamURL`, and `internal/version.*`.

## ANTI-PATTERNS (THIS PROJECT)

- Never commit real local secrets; `.gitignore` marks local secrets as `NEVER commit`.
- Do not expose management endpoints publicly; `/version` includes precise commit/build metadata.
- Do not pass `-backend` under service managers such as NSSM/systemd/Docker; those supervisors own process backgrounding.
- Do not buffer entire request bodies in memory; let `httputil.ReverseProxy` stream them to the upstream.
- Do not add pass-through request-pipeline wrappers in `internal/proxy`; compose request-side hooks at the mux call site or in a dedicated middleware package.
- Do not leak upstream/certificate/private-key details in proxy error JSON; tests assert sanitization.
- Do not make `/health` fail at the HTTP layer for degraded upstream state; it returns HTTP 200 with `status` in the body.
- Do not partially overwrite `secrets/` in `scripts/build.sh`; all three files must exist or all three placeholders are generated.

## UNIQUE STYLES

- `slog` is the runtime logging API; access logs record method/path/status/bytes/latency and intentionally avoid request bodies.
- Runtime flags have env twins prefixed `MTLS_`; docs express precedence as `flag > env > build-time > default`.
- Release artifacts are cross-platform binaries named `mtls-router-${GOOS}-${GOARCH}` with `.exe` only for Windows.
- `setup.ps1` must keep a UTF-8 BOM for Windows PowerShell 5.1 compatibility; `main_test.go` checks this.

## COMMANDS

```bash
go test ./...
go vet ./...
make test-shell
test -z "$(gofmt -l .)"
./scripts/build.sh
```

## NOTES

- `gopls` was not installed during this generation; codemap used AST-grep and direct reads instead of LSP reference counts.
- Current working tree contains user/untracked OMC and worktree artifacts under hidden directories; ignore those when reasoning about product code.
- `scripts/build.sh` may create placeholder PEMs under `secrets/` for local builds; real release secrets come from GitHub secrets/vars.
- Historical planning docs under `docs/superpowers/` are useful context, not source-of-truth for current behavior.
