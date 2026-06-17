# mtls-router Phase 3: mTLS Transport and Startup Probe Plan

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

**Prerequisite:** Phase 2 must pass.

**Next phase after pass:** Phase 4: Compose ReverseProxy

---
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
---

## Phase Completion Checklist

- [ ] Every ticket in this phase completed: 理解 -> 实施 -> 验收 -> fix if needed -> 重新验收.
- [ ] Every expected file listed in this phase exists.
- [ ] Every ticket-level verification command passed.
- [ ] Every phase-level verification command passed from `/Users/test1/liuyekang/dev/code/mtls-router`.
- [ ] Any failed verification produced a minimal fix and successful re-verification.
- [ ] Expected commit(s) exist, unless batching commits was explicitly chosen.
- [ ] `git status --short` contains no unexpected source changes.
