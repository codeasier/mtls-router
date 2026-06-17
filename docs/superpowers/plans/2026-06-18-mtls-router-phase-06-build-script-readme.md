# mtls-router Phase 6: Build Script and README Plan

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

**Prerequisite:** Phase 5 must pass.

**Next phase after pass:** Phase 7: Deployment Artifacts

---
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
---

## Phase Completion Checklist

- [ ] Every ticket in this phase completed: 理解 -> 实施 -> 验收 -> fix if needed -> 重新验收.
- [ ] Every expected file listed in this phase exists.
- [ ] Every ticket-level verification command passed.
- [ ] Every phase-level verification command passed from `/Users/test1/liuyekang/dev/code/mtls-router`.
- [ ] Any failed verification produced a minimal fix and successful re-verification.
- [ ] Expected commit(s) exist, unless batching commits was explicitly chosen.
- [ ] `git status --short` contains no unexpected source changes.
