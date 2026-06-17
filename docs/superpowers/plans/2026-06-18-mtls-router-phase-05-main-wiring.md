# mtls-router Phase 5: Main Program Wiring Plan

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

**Prerequisite:** Phase 4 must pass.

**Next phase after pass:** Phase 6: Build Script and README

---
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
---

## Phase Completion Checklist

- [ ] Every ticket in this phase completed: 理解 -> 实施 -> 验收 -> fix if needed -> 重新验收.
- [ ] Every expected file listed in this phase exists.
- [ ] Every ticket-level verification command passed.
- [ ] Every phase-level verification command passed from `/Users/test1/liuyekang/dev/code/mtls-router`.
- [ ] Any failed verification produced a minimal fix and successful re-verification.
- [ ] Expected commit(s) exist, unless batching commits was explicitly chosen.
- [ ] `git status --short` contains no unexpected source changes.
