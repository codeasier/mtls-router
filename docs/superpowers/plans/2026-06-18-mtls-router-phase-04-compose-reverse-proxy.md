# mtls-router Phase 4: Compose ReverseProxy Plan

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

**Prerequisite:** Phase 3 must pass.

**Next phase after pass:** Phase 5: Main Program Wiring

---
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
---

## Phase Completion Checklist

- [x] Every ticket in this phase completed: 理解 -> 实施 -> 验收 -> fix if needed -> 重新验收.
- [x] Every expected file listed in this phase exists.
- [x] Every ticket-level verification command passed.
- [x] Every phase-level verification command passed from `/Users/test1/liuyekang/dev/code/mtls-router`.
- [x] Any failed verification produced a minimal fix and successful re-verification.
- [x] Expected commit(s) exist, unless batching commits was explicitly chosen.
- [x] `git status --short` contains no unexpected source changes.

Latest Phase 4 evidence:

```bash
go test ./internal/proxy/... -v
go build ./...
git status --short
```
