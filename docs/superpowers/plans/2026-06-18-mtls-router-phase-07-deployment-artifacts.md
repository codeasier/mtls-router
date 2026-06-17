# mtls-router Phase 7: Deployment Artifacts Plan

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

**Prerequisite:** Phase 6 should pass; Phase 5 is strictly required.

**Next phase after pass:** Phase 8: Final Verification

---
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
---

## Phase Completion Checklist

- [ ] Every ticket in this phase completed: 理解 -> 实施 -> 验收 -> fix if needed -> 重新验收.
- [ ] Every expected file listed in this phase exists.
- [ ] Every ticket-level verification command passed.
- [ ] Every phase-level verification command passed from `/Users/test1/liuyekang/dev/code/mtls-router`.
- [ ] Any failed verification produced a minimal fix and successful re-verification.
- [ ] Expected commit(s) exist, unless batching commits was explicitly chosen.
- [ ] `git status --short` contains no unexpected source changes.
