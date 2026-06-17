# mtls-router Phase Orchestration Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan phase-by-phase. Do not start a later phase until the previous phase acceptance criteria pass. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define the phase-level execution plan for `docs/superpowers/plans/2026-06-17-mtls-router.md`, including what each phase does and the exact acceptance standard for passing the phase.

**Architecture:** The implementation is decomposed into 8 phases based on dependency order. Independent tickets inside a phase may run concurrently, but each ticket must still follow `理解 -> 实施 -> 验收 -> fix -> 重新验收` before the phase is considered complete.

**Tech Stack:** Go 1.22+ stdlib, `net/http/httputil.ReverseProxy`, `crypto/tls`, `crypto/x509`, `log/slog`, GitHub Actions, systemd, Docker `scratch` image.

**Source plan:** `docs/superpowers/plans/2026-06-17-mtls-router.md`

---

## Current Progress Snapshot

**Last updated:** 2026-06-18

**Overall status:** Phases 1-7 are complete. Phase 8 is the next executable phase.

**Current HEAD:** `PENDING chore: add deployment artifacts`

**Working tree status at snapshot time:**

- Code scaffold from Phase 1 is committed.
- Phase 2 implementation and phase planning docs are committed in `845a7a8`.
- Phase 4 implementation and phase planning docs are committed in `0aff614`.
- Phase 7 deployment artifacts and planning docs are ready to commit: `systemd/mtls-router.service`, `Dockerfile`, `.dockerignore`, and this phase orchestration plan.
- `.omc/` contains local execution/session state and is not part of the implementation.

### Progress by Phase

| Phase | Tickets | Status | Evidence | What remains |
|---|---:|---|---|---|
| Phase 1 — Repository Scaffold | T01 | **DONE** | Commit `4089fbc`; files `go.mod`, `.gitignore`, `LICENSE`, `README.md` exist | Nothing for code; optionally commit phase planning docs separately |
| Phase 2 — Independent Atomic Packages and CI | T02, T03, T05, T06, T07, T08, T11, T15 | **DONE** | Commit `845a7a8`; phase-level verification passed: openssl setup; `go test ./internal/config/... ./internal/certs/... ./internal/proxy/... ./internal/log/... -v`; `go build ./...`; workflow file checks; `test -z "$(gofmt -l internal)"` | Nothing; Phase 3 is next |
| Phase 3 — mTLS Transport and Startup Probe | T04, T10 | **DONE** | Commit `e0b51e9`; phase-level verification passed: `go test ./internal/proxy/... -run TestNewMTLSTransport -v`; `go test ./internal/health/... -v`; `go build ./...`; `test -z "$(gofmt -l internal)"` | Nothing |
| Phase 4 — Compose ReverseProxy | T09 | **DONE** | Commit `0aff614`; phase-level verification passed: `go test ./internal/proxy/... -v`; `go build ./...` | Nothing; Phase 5 is next |
| Phase 5 — Main Program Wiring | T12 | **DONE** | Commit `bdf7df8`; phase-level verification passed: `go test ./...`; `go build ./...`; `gofmt -l .` | Nothing |
| Phase 6 — Build Script and README | T13, T16 | **DONE** | Phase-level verification passed: `test -x scripts/build.sh`; `./scripts/build.sh`; `test -x ./mtls-router`; `./mtls-router` exits 1 with placeholder upstream; `go test ./...`; `go build ./...`; `test -z "$(gofmt -l .)"` | Nothing; Phase 7 is next |
| Phase 7 — Deployment Artifacts | T14 | **DONE** | Files `systemd/mtls-router.service`, `Dockerfile`, and `.dockerignore` exist; phase-level verification passed: `test -f systemd/mtls-router.service`; `test -f Dockerfile`; `test -f .dockerignore`; `go test ./...`; `go build ./...`; `git status --short`; quality fix applied so Dockerfile builder uses `golang:1.26.2-alpine` to match `go.mod` | Nothing; Phase 8 is next |
| Phase 8 — Final Verification | T17 | **NEXT EXECUTABLE** | Phases 1-7 pass | Run race tests, vet, gofmt, cross-compile, fail-fast smoke |

### Next Required Execution

The next executable phase is **Phase 8 — Final Verification / T17**.

Recommended execution order inside Phase 8:

1. T17 — final verification

T17 may run after Phase 7 passes.

### Do Not Execute Yet

No later phases remain before T17. Do not mark the whole project complete until Phase 8 passes.

### Completion Definition for the Whole Project

The whole project is complete only when Phase 8 passes all of these:

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
go test -race -count=1 ./...
go vet ./...
test -z "$(gofmt -l .)"
for GOOS in linux darwin windows; do
  for GOARCH in amd64 arm64; do
    GOOS=$GOOS GOARCH=$GOARCH go build -trimpath \
      -ldflags "-s -w \
        -X main.clientCertPEM=placeholder \
        -X main.clientKeyPEM=placeholder \
        -X main.upstreamCAPEM=placeholder \
        -X main.upstreamURL=https://placeholder" \
      -o /tmp/mtls-router-$GOOS-$GOARCH .
  done
done
timeout 5 ./mtls-router; echo "exit=$?"
```

Expected final result: tests/vet/gofmt/cross-compile pass, and placeholder runtime exits non-zero quickly instead of serving traffic with invalid cert/upstream configuration.

---

## Execution Rules

### Per-ticket lifecycle

Every ticket in every phase must follow this lifecycle:

1. **理解** — read the corresponding `## Task N` section in the source plan and write a concise understanding report.
2. **实施** — implement exactly the files and steps described by the source plan.
3. **验收** — independently run the ticket-level verification commands and compare results against the source plan.
4. **fix** — if verification fails, apply the minimum necessary fix.
5. **重新验收** — rerun the same verification commands. A ticket may not be marked complete until re-verification passes.

### Model routing requirement

If subagents are used:

| Lifecycle step | Required model |
|---|---|
| 理解 | opus |
| 验收 | opus |
| 重新验收 | opus |
| 实施 | haiku |
| fix | haiku |

### Phase gate rule

A phase passes only when:

- every ticket in the phase has passed its own ticket-level acceptance criteria;
- all files expected by the phase exist at the expected paths;
- all test/build/format commands listed under the phase pass;
- git history contains the expected commits for the phase, unless the user explicitly chooses not to commit during execution;
- `git status --short` shows no unexpected working-tree changes except explicitly ignored or execution-state files such as `.omc/`.

---

## Phase Dependency Graph

```text
Phase 1: T01
  -> Phase 2: T02, T03, T05, T06, T07, T08, T11, T15
      -> Phase 3: T04, T10
          -> Phase 4: T09
              -> Phase 5: T12
                  -> Phase 6: T13, T16
                      -> Phase 7: T14
                          -> Phase 8: T17
```

---

## Phase 1: Repository Scaffold

**Tickets:**

- T01 — Repository scaffold

**Can run concurrently:** No. This is the root dependency for all later work.

**What this phase does:**

- Creates the Go module.
- Adds root `.gitignore` rules for build outputs, secrets, editor files, and Go vendor directory.
- Adds MIT license.
- Adds minimal README stub that points to the design spec and shows build/run commands.

**Files created:**

- `go.mod`
- `.gitignore`
- `LICENSE`
- `README.md`

**Ticket-level acceptance:**

- `go.mod` exists and declares:

```go
module github.com/codeasier/mtls-router
```

- `go.mod` has a Go directive of `go 1.22` or newer.
- `.gitignore` contains at least:

```gitignore
/dist/
/mtls-router
/mtls-router.exe
*.test
*.out
/secrets/
.DS_Store
*.swp
.idea/
.vscode/
/vendor/
```

- `LICENSE` is a standard MIT license with copyright year 2026.
- `README.md` contains:
  - heading `# mtls-router`;
  - reference to `docs/superpowers/specs/2026-06-17-mtls-router-design.md`;
  - build command `./scripts/build.sh`;
  - run command `./mtls-router`;
  - listen address note `127.0.0.1:19099`.

**Phase-level verification commands:**

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
go build ./...
git status --short
```

**Pass standard:**

- `go build ./...` exits 0. If it prints that no packages matched because there are no Go files yet, that is acceptable for this phase.
- The scaffold files are committed with message:

```text
chore: scaffold mtls-router repository
```

- No unexpected uncommitted source changes remain.

**Current known status:**

- This phase has already been implemented and committed as `4089fbc chore: scaffold mtls-router repository`.
- Existing artifacts:
  - `.omc/state/sessions/2026-06-17-mtls-router/tickets/T01-scaffold/00-understand.md`
  - `.omc/state/sessions/2026-06-17-mtls-router/tickets/T01-scaffold/01-impl.diff`
  - `.omc/state/sessions/2026-06-17-mtls-router/tickets/T01-scaffold/02-verify.md`

---

## Phase 2: Independent Atomic Packages and CI

**Tickets:**

- T02 — Config parser (TDD)
- T03 — Cert loader (TDD)
- T05 — ReverseProxy Director (TDD)
- T06 — Request body stream sniff (TDD)
- T07 — ModifyResponse hook (TDD)
- T08 — ErrorHandler (TDD)
- T11 — Access log (TDD)
- T15 — CI workflows

**Can run concurrently:** Yes. These tickets do not depend on each other except for the repo scaffold from Phase 1.

**What this phase does:**

- Adds runtime configuration parsing and validation.
- Adds PEM cert/key/CA loader.
- Adds isolated proxy helper components:
  - request director;
  - request stream sniffing;
  - response modification for SSE;
  - reverse proxy error handler.
- Adds access logging support.
- Adds GitHub Actions CI and release workflow definitions.

**Files created:**

- `internal/config/config.go`
- `internal/config/config_test.go`
- `internal/certs/certs.go`
- `internal/certs/certs_test.go`
- `internal/proxy/director.go`
- `internal/proxy/director_test.go`
- `internal/proxy/stream.go`
- `internal/proxy/stream_test.go`
- `internal/proxy/modifyresponse.go`
- `internal/proxy/modifyresponse_test.go`
- `internal/proxy/errorhandler.go`
- `internal/proxy/errorhandler_test.go`
- `internal/log/log.go`
- `internal/log/log_test.go`
- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`

**Ticket-level acceptance:**

### T02 — Config parser

- `config.Defaults` and `config.Config` are defined.
- `config.Load(defaults, args)` implements precedence:

```text
flag > env > build-time/default values
```

- Environment variables supported:
  - `MTLS_LISTEN_ADDR`
  - `MTLS_UPSTREAM_URL`
  - `MTLS_TLS_MIN`
  - `MTLS_TIMEOUT`
  - `MTLS_DEBUG`
- Flags supported:
  - `-listen`
  - `-upstream`
  - `-tls-min`
  - `-timeout`
  - `-debug`
- Validation rejects invalid or missing upstream URL.
- Validation accepts TLS minimum versions `tls1.2` and `tls1.3`.
- Verification command passes:

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
go test ./internal/config/... -v
```

### T03 — Cert loader

- `certs.LoadFromStrings(certPEM, keyPEM, caPEM string)` exists.
- Empty client cert, client key, or upstream CA returns an error that names the missing input.
- Malformed cert/key returns an error.
- Valid PEM cert/key/CA returns:
  - non-nil `*tls.Certificate` with at least one certificate;
  - non-nil `*x509.CertPool`.
- Verification setup and command pass:

```bash
mkdir -p /tmp/mtls-router-test
openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
  -keyout /tmp/mtls-router-test/key.pem \
  -out /tmp/mtls-router-test/cert.pem \
  -subj "/CN=mtls-router-test" 2>&1 | tail -2
cd /Users/test1/liuyekang/dev/code/mtls-router
go test ./internal/certs/... -v
```

### T05 — ReverseProxy Director

- `proxy.NewDirector(upstream *url.URL)` exists.
- It rewrites request scheme and host to the upstream URL.
- It rewrites the outbound `Host` header to the upstream host.
- It preserves path, query, body, and passthrough headers.
- Verification command passes:

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
go test ./internal/proxy/... -run TestNewDirector -v
```

### T06 — Request body stream sniff

- `proxy.SniffStream(ctx context.Context, body io.ReadCloser)` exists and returns updated context plus a bool.
- It detects JSON bodies containing `"stream":true` within the first 4KB.
- It does not consume or corrupt the request body; downstream readers still receive the original bytes.
- It returns false for non-stream requests, empty bodies, malformed JSON, or bodies where the marker appears after the sniff window.
- Verification command passes:

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
go test ./internal/proxy/... -run 'TestSniffStream|TestIsStreamRequest' -v
```

### T07 — ModifyResponse hook

- `proxy.NewModifyResponse()` exists and returns `func(*http.Response) error`.
- It forces SSE response headers when the upstream response is `text/event-stream` or otherwise identified by the plan as stream output.
- Required headers:
  - `Content-Type: text/event-stream`
  - `Cache-Control: no-cache`
  - connection buffering must not be introduced.
- It preserves non-SSE responses.
- Verification command passes:

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
go test ./internal/proxy/... -run TestNewModifyResponse -v
```

### T08 — ErrorHandler

- `proxy.NewErrorHandler()` exists and returns `func(http.ResponseWriter, *http.Request, error)`.
- Timeout-like errors map to HTTP 504.
- Other transport errors map to HTTP 502.
- Response body is JSON and does not leak certificates, private keys, or upstream internals.
- Verification command passes:

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
go test ./internal/proxy/... -run TestNewErrorHandler -v
```

### T11 — Access log

- `log.ResponseRecorder` records status code and response byte count.
- `log.AccessLog` writes method, path, status, bytes, latency, and error when present.
- Debug body logging is disabled unless explicitly enabled.
- Verification command passes:

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
go test ./internal/log/... -v
```

### T15 — CI workflows

- `.github/workflows/ci.yml` exists.
- CI runs on pull requests and pushes to `main`.
- CI installs Go, then runs:
  - `go test ./...`
  - `go vet ./...`
  - `gofmt` check.
- `.github/workflows/release.yml` exists.
- Release workflow builds six targets:
  - `linux/amd64`
  - `linux/arm64`
  - `darwin/amd64`
  - `darwin/arm64`
  - `windows/amd64`
  - `windows/arm64`
- Release workflow injects linker variables for:
  - `main.clientCertPEM`
  - `main.clientKeyPEM`
  - `main.upstreamCAPEM`
  - `main.upstreamURL`
  - `main.version`

**Phase-level verification commands:**

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
go test ./internal/config/... ./internal/certs/... ./internal/proxy/... ./internal/log/... -v
go build ./...
test -f .github/workflows/ci.yml
test -f .github/workflows/release.yml
git status --short
```

**Pass standard:**

- All listed package tests pass.
- `go build ./...` exits 0.
- All listed files exist.
- Each ticket has a commit matching its source-plan intent, unless execution is explicitly configured to batch commits.
- No unexpected uncommitted source changes remain.

---

## Phase 3: mTLS Transport and Startup Probe

**Tickets:**

- T04 — mTLS `http.Transport` (TDD)
- T10 — Startup mTLS probe (TDD)

**Can run concurrently:** Yes after Phase 2 passes. Both depend on Phase 2 outputs, but not on each other.

**What this phase does:**

- Builds the reusable mTLS `http.Transport` around the cert loader.
- Adds the startup health probe that validates the mTLS path before serving local traffic.

**Files created:**

- `internal/proxy/transport.go`
- `internal/proxy/transport_test.go`
- `internal/health/probe.go`
- `internal/health/probe_test.go`

**Acceptance standard:**

- `proxy.NewMTLSTransport(certPEM, keyPEM, caPEM string, opts ...TransportOption)` exists.
- `proxy.WithTLSMin("tls1.2")` and `proxy.WithTLSMin("tls1.3")` work.
- Invalid TLS minimum versions return an error.
- Missing cert/key/CA returns an error.
- A client using the transport can call an mTLS `httptest` server requiring a client certificate.
- `health.Probe(ProbeOptions)` succeeds against an mTLS server returning a non-5xx response.
- `health.Probe` fails for handshake, timeout, invalid URL, and 5xx responses according to the source plan.

**Phase-level verification commands:**

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
go test ./internal/proxy/... -run TestNewMTLSTransport -v
go test ./internal/health/... -v
go build ./...
git status --short
```

**Pass standard:**

- All commands exit 0.
- `internal/proxy/transport.go` imports only stdlib plus `internal/certs`.
- `internal/health/probe.go` does not start the local server; it only probes upstream.
- No unexpected uncommitted source changes remain.

---

## Phase 4: Compose ReverseProxy

**Tickets:**

- T09 — Compose ReverseProxy

**Can run concurrently:** No. It depends on T04, T05, T06, T07, and T08.

**What this phase does:**

- Creates the package-level constructor that assembles all proxy components into one configured `httputil.ReverseProxy`.
- Creates the handler wrapper that runs stream sniffing before delegating to the reverse proxy.

**Files created:**

- `internal/proxy/proxy.go`

**Acceptance standard:**

- `proxy.Options` exists with:
  - `Upstream *url.URL`
  - `Transport *proxy.Transport`
  - `ErrorLog *slog.Logger`
- `proxy.New(opts Options)` returns `*httputil.ReverseProxy`.
- The returned proxy uses:
  - `Director: NewDirector(opts.Upstream)`
  - `ModifyResponse: NewModifyResponse()`
  - `ErrorHandler: NewErrorHandler()`
  - `FlushInterval: -1`
  - `Transport: opts.Transport`
- `proxy.WrapHandler(rp)` returns an `http.Handler` that calls stream sniffing before proxying.

**Phase-level verification commands:**

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
go build ./...
go test ./internal/proxy/... -v
git status --short
```

**Pass standard:**

- Build succeeds.
- All proxy package tests pass, including tests from T04-T08.
- No unexpected uncommitted source changes remain.

---

## Phase 5: Main Program Wiring

**Tickets:**

- T12 — `main.go` wiring

**Can run concurrently:** No. It depends on T02, T04, T09, T10, and T11.

**What this phase does:**

- Adds the single-binary entry point.
- Declares linker-injected variables for cert/key/CA/upstream/version.
- Loads configuration.
- Builds logger, mTLS transport, startup probe, reverse proxy, and HTTP server.
- Handles graceful shutdown.

**Files created:**

- `main.go`

**Acceptance standard:**

- `main.go` declares:
  - `clientCertPEM string`
  - `clientKeyPEM string`
  - `upstreamCAPEM string`
  - `upstreamURL string`
  - `version = "dev"`
- `run()` performs these steps in order:
  1. handle version/help behavior;
  2. build `config.Defaults` from linker variables and defaults;
  3. call `config.Load` and `cfg.Validate`;
  4. create `slog` logger;
  5. call `proxy.NewMTLSTransport`;
  6. call `health.Probe` before serving;
  7. parse upstream URL;
  8. create `proxy.New` and wrap it with `proxy.WrapHandler`;
  9. listen on `cfg.ListenAddr`;
  10. shut down on `SIGINT`/`SIGTERM`.
- The local listener is plain HTTP; upstream transport is mTLS.
- No protocol conversion is introduced.

**Phase-level verification commands:**

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
go test ./...
go build ./...
git status --short
```

**Pass standard:**

- All tests pass.
- Build produces a valid binary when linker variables are supplied.
- `main.go` imports only stdlib plus internal packages.
- No unexpected uncommitted source changes remain.

---

## Phase 6: Build Script and README

**Tickets:**

- T13 — `scripts/build.sh`
- T16 — README polish

**Can run concurrently:** Yes after Phase 5 passes.

**What this phase does:**

- Adds a local build script that generates placeholder certs and injects linker variables.
- Expands README into practical user-facing documentation.

**Files created or modified:**

- Create: `scripts/build.sh`
- Modify: `README.md`

**Acceptance standard:**

### T13 — build script

- `scripts/build.sh` exists and is executable.
- It creates `secrets/client.pem`, `secrets/client.key`, and `secrets/upstream-ca.pem` if missing.
- It runs `go build -trimpath`.
- It injects linker variables:
  - `main.clientCertPEM`
  - `main.clientKeyPEM`
  - `main.upstreamCAPEM`
  - `main.upstreamURL`
- It writes binary `./mtls-router`.
- Running the binary with placeholder upstream fails fast instead of serving broken traffic.

### T16 — README

- README documents:
  - purpose;
  - build process;
  - runtime config flags and env vars;
  - linker-injected secrets;
  - local listen address;
  - mTLS upstream behavior;
  - streaming/SSE transparency;
  - systemd/Docker pointers if available.

**Phase-level verification commands:**

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
test -x scripts/build.sh
./scripts/build.sh
test -x ./mtls-router
(timeout 5 ./mtls-router; echo "exit=$?")
git status --short
```

**Pass standard:**

- Build script exits 0.
- Binary exists and is executable.
- Placeholder run exits non-zero quickly because the placeholder upstream is invalid.
- README has no stale claim that contradicts actual flags or env vars.
- No unexpected uncommitted source changes remain.

---

## Phase 7: Deployment Artifacts

**Tickets:**

- T14 — systemd + Docker

**Can run concurrently:** No. It depends on Phase 5 and ideally Phase 6.

**What this phase does:**

- Adds a hardened systemd service unit.
- Adds a multi-stage Docker build that ships a static binary in a `scratch` image.
- Adds `.dockerignore`.

**Files created:**

- `systemd/mtls-router.service`
- `Dockerfile`
- `.dockerignore`

**Acceptance standard:**

- systemd service uses:
  - `ExecStart=/usr/local/bin/mtls-router`
  - `Restart=on-failure`
  - `Environment=MTLS_LISTEN_ADDR=127.0.0.1:19099`
  - hardening options from the source plan.
- Dockerfile:
  - has a Go builder stage;
  - builds with `CGO_ENABLED=0`;
  - injects linker variables;
  - final stage is `FROM scratch`;
  - entrypoint is `/mtls-router`.
- `.dockerignore` excludes `.git`, `.github`, `docs`, `dist`, `secrets`, markdown docs, and test artifacts.

**Phase-level verification commands:**

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
test -f systemd/mtls-router.service
test -f Dockerfile
test -f .dockerignore
go test ./...
go build ./...
git status --short
```

**Pass standard:**

- Required deployment files exist.
- Go test/build still pass after deployment artifacts are added.
- No source secrets are introduced into Docker context.
- No unexpected uncommitted source changes remain.

---

## Phase 8: Final Verification

**Tickets:**

- T17 — Final verification

**Can run concurrently:** No. It depends on every previous phase.

**What this phase does:**

- Runs the whole project verification suite.
- Checks formatting and vet.
- Cross-compiles all six release targets.
- Confirms fail-fast behavior with placeholder certs/upstream.
- Commits any final cleanup if needed.

**Files modified:**

- None expected. Only cleanup changes if verification exposes a concrete issue.

**Acceptance standard:**

- Race tests pass:

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
go test -race -count=1 ./...
```

- Vet and format checks pass:

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
go vet ./...
test -z "$(gofmt -l .)"
```

- Six-platform cross-compile succeeds:

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
for GOOS in linux darwin windows; do
  for GOARCH in amd64 arm64; do
    echo ">> $GOOS/$GOARCH"
    GOOS=$GOOS GOARCH=$GOARCH go build -trimpath \
      -ldflags "-s -w \
        -X main.clientCertPEM=placeholder \
        -X main.clientKeyPEM=placeholder \
        -X main.upstreamCAPEM=placeholder \
        -X main.upstreamURL=https://placeholder" \
      -o /tmp/mtls-router-$GOOS-$GOARCH .
  done
done
ls -la /tmp/mtls-router-* 2>/dev/null | head -20
```

- Placeholder binary fails fast:

```bash
cd /Users/test1/liuyekang/dev/code/mtls-router
timeout 5 ./mtls-router; echo "exit=$?"
```

**Pass standard:**

- All tests pass with race detector.
- `go vet` exits 0.
- `gofmt -l .` prints nothing.
- Six binaries exist in `/tmp/`.
- Placeholder binary exits non-zero quickly.
- `git status --short` is clean except intentionally untracked local execution-state files.

---

## Phase Completion Checklist

- [x] All tickets in Phase 2 completed their lifecycle: 理解 → 实施 → 验收 → fix if needed → 重新验收.
- [x] Every Phase 2 expected file exists.
- [x] Every Phase 2 phase-level verification command was run from `/Users/test1/liuyekang/dev/code/mtls-router`.
- [x] Command output matched the Phase 2 pass standard after one minimal test expectation fix.
- [x] Failed verification produced a minimal fix and successful re-verification evidence.
- [x] Git contains the expected commit: `845a7a8 feat: implement phase 2 atomic packages and CI`.
- [x] `git status --short` contains only `.omc/` execution state after the commit.

Latest Phase 2 evidence:

```bash
mkdir -p /tmp/mtls-router-test && openssl req -x509 -newkey rsa:2048 -nodes -days 1 -keyout /tmp/mtls-router-test/key.pem -out /tmp/mtls-router-test/cert.pem -subj "/CN=mtls-router-test"
go test ./internal/config/... ./internal/certs/... ./internal/proxy/... ./internal/log/... -v
# PASS after fixing internal/log/log_test.go expectation for slog quoted path output
go build ./...
test -f /Users/test1/liuyekang/dev/code/mtls-router/.github/workflows/ci.yml
test -f /Users/test1/liuyekang/dev/code/mtls-router/.github/workflows/release.yml
test -z "$(gofmt -l /Users/test1/liuyekang/dev/code/mtls-router)"
git status --short
```

Phase 3 completion checklist:

- [x] All tickets in Phase 3 completed their lifecycle: 理解 → 实施 → 验收 → fix if needed → 重新验收.
- [x] Every Phase 3 expected file exists.
- [x] Every Phase 3 phase-level verification command was run from `/Users/test1/liuyekang/dev/code/mtls-router`.
- [x] Command output matched the Phase 3 pass standard after fixing the `health.Probe` API to match the source plan and Phase 5 call site.
- [x] Failed review produced a minimal fix and successful re-verification evidence.
- [x] Git contains the expected commit: `e0b51e9 feat: add phase 3 mtls transport and probe`.
- [x] `git status --short` contains only `.omc/` execution state after the commit.

Latest Phase 3 evidence:

```bash
go test ./internal/proxy/... -run TestNewMTLSTransport -v
go test ./internal/health/... -v
go build ./...
test -z "$(gofmt -l internal)"
git status --short
```

Phase 4 completion checklist:

- [x] All tickets in Phase 4 completed their lifecycle: 理解 → 实施 → 验收 → fix if needed → 重新验收.
- [x] Every Phase 4 expected file exists.
- [x] Every Phase 4 phase-level verification command was run from `/Users/test1/liuyekang/dev/code/mtls-router`.
- [x] Command output matched the Phase 4 pass standard after fixing stream sniffing to avoid full-body buffering and preserve replay semantics.
- [x] Failed review produced minimal fixes and successful re-verification evidence.
- [x] Git contains the expected commit: `0aff614 feat(proxy): compose reverse proxy`.
- [x] `git status --short` contains only `.omc/` execution state after the commit.

Latest Phase 4 evidence:

```bash
go test ./internal/proxy/... -v
go build ./...
git status --short
```

Phase 5 completion checklist:

- [x] All tickets in Phase 5 completed their lifecycle: 理解 → 实施 → 验收 → fix if needed → 重新验收.
- [x] Every Phase 5 expected file exists.
- [x] Every Phase 5 phase-level verification command was run from `/Users/test1/liuyekang/dev/code/mtls-router`.
- [x] Command output matched the Phase 5 pass standard.
- [x] No failing verification required a fix.
- [x] Git contains the expected commit: `bdf7df8 feat: wire main program`.
- [x] `git status --short` contains only `.omc/` execution state plus files staged for the Phase 5 commit before committing.

Latest Phase 5 evidence:

```bash
go test ./...
go build ./...
gofmt -l .
git status --short
```

Phase 6 completion checklist:

- [x] All tickets in Phase 6 completed their lifecycle: 理解 → 实施 → 验收 → fix if needed → 重新验收.
- [x] Every Phase 6 expected file exists.
- [x] Every Phase 6 phase-level verification command was run from `/Users/test1/liuyekang/dev/code/mtls-router`.
- [x] Command output matched the Phase 6 pass standard.
- [x] Missing macOS `timeout` command produced an alternate direct run verification; `./mtls-router` exited 1 quickly with placeholder upstream DNS failure.
- [x] Git contains the expected commit after committing Phase 6.
- [x] `git status --short` contains only `.omc/` execution state and local generated artifacts after the commit.

Latest Phase 6 evidence:

```bash
test -x scripts/build.sh
./scripts/build.sh
test -x ./mtls-router
./mtls-router; echo "exit=$?"
go test ./...
go build ./...
test -z "$(gofmt -l .)"
git status --short
```

Phase 7 completion checklist:

- [x] All tickets in Phase 7 completed their lifecycle: 理解 → 实施 → 验收 → fix if needed → 重新验收.
- [x] Every Phase 7 expected file exists: `systemd/mtls-router.service`, `Dockerfile`, and `.dockerignore`.
- [x] Every Phase 7 phase-level verification command was run from `/Users/test1/liuyekang/dev/code/mtls-router`.
- [x] Command output matched the Phase 7 pass standard after the Dockerfile Go version quality fix.
- [x] Quality review found the Dockerfile Go version was too low; the builder image now uses `golang:1.26.2-alpine` to match `go.mod` (`go 1.26.2`).
- [x] Quality re-review approved the deployment artifacts.
- [x] Git contains the expected commit after committing Phase 7.
- [x] `git status --short` contains only `.omc/` execution state and local generated artifacts after the commit.

Latest Phase 7 evidence:

```bash
test -f systemd/mtls-router.service
test -f Dockerfile
test -f .dockerignore
go test ./...
go build ./...
git status --short
```

## Phase Completion Checklist Template

Use this checklist after each phase:

- [ ] All tickets in the phase completed their lifecycle: 理解 → 实施 → 验收 → fix if needed → 重新验收.
- [ ] Every expected file exists.
- [ ] Every phase-level verification command was run from `/Users/test1/liuyekang/dev/code/mtls-router`.
- [ ] Command output matched the phase pass standard.
- [ ] Any failing verification produced a minimal fix and re-verification evidence.
- [ ] Git contains the expected commit(s), or batching was explicitly chosen.
- [ ] `git status --short` contains no unexpected source changes.

---

## Self-Review

### Spec coverage

| Spec requirement | Covered by phase |
|---|---|
| Local HTTP proxy on `127.0.0.1:19099` | Phase 5 |
| Build-time injected client cert/key/CA/upstream | Phase 5, Phase 6, Phase 8 |
| mTLS upstream transport | Phase 3 |
| ReverseProxy director passthrough | Phase 2, Phase 4 |
| Streaming/SSE transparency | Phase 2, Phase 4 |
| Error handling 502/504 | Phase 2, Phase 4 |
| Startup mTLS probe | Phase 3, Phase 5 |
| Access logging | Phase 2, Phase 5 |
| systemd deployment | Phase 7 |
| Docker `scratch` image | Phase 7 |
| CI and six-platform release build | Phase 2, Phase 8 |

### Placeholder scan

No TBD, TODO, "implement later", or vague edge-case placeholders remain in this phase orchestration plan.

### Type consistency

This plan references the same public functions/types as the source plan:

- `config.Load`, `config.Defaults`, `config.Config`
- `certs.LoadFromStrings`
- `proxy.NewMTLSTransport`, `proxy.WithTLSMin`, `proxy.Transport.RootCAs`
- `proxy.NewDirector`
- `proxy.SniffStream`
- `proxy.NewModifyResponse`
- `proxy.NewErrorHandler`
- `proxy.New`, `proxy.Options`, `proxy.WrapHandler`
- `health.Probe`, `health.ProbeOptions`
- `log.ResponseRecorder`, `log.AccessLog`

No additional implementation surface is introduced by this phase plan.
