# mtls-router Phase 2: Independent Atomic Packages and CI Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this phase task-by-task. Do not start this phase until all prerequisite phases have passed. Steps use checkbox (`- [ ]`) syntax for tracking.

**Source plan:** `docs/superpowers/plans/2026-06-17-mtls-router.md`

**Per-ticket lifecycle:** Every ticket in this phase must follow `理解 -> 实施 -> 验收 -> fix -> 重新验收` before it can be marked complete.

**Model routing if subagents are used:**

| Lifecycle step | Required model |
|---|---|
| 理解 | opus |
| 验收 | opus |
| 重新验收 | opus |
| 实施 | haiku |
| fix | haiku |

**Phase gate rule:** This phase passes only when every ticket in this document passes its ticket-level acceptance criteria, all phase-level verification commands pass, and `git status --short` has no unexpected source changes.

---

**Prerequisite:** Phase 1 must pass.

**Next phase after pass:** Phase 3: mTLS Transport and Startup Probe

---
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
---

## Phase Completion Checklist

- [ ] Every ticket in this phase completed: 理解 -> 实施 -> 验收 -> fix if needed -> 重新验收.
- [ ] Every expected file listed in this phase exists.
- [ ] Every ticket-level verification command passed.
- [ ] Every phase-level verification command passed from `/Users/test1/liuyekang/dev/code/mtls-router`.
- [ ] Any failed verification produced a minimal fix and successful re-verification.
- [ ] Expected commit(s) exist, unless batching commits was explicitly chosen.
- [ ] `git status --short` contains no unexpected source changes.
