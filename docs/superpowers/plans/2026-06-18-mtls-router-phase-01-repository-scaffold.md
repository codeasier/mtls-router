# mtls-router Phase 1: Repository Scaffold Plan

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

**Prerequisite:** None. This is the root phase.

**Next phase after pass:** Phase 2: Independent Atomic Packages and CI

---
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
---

## Phase Completion Checklist

- [ ] Every ticket in this phase completed: 理解 -> 实施 -> 验收 -> fix if needed -> 重新验收.
- [ ] Every expected file listed in this phase exists.
- [ ] Every ticket-level verification command passed.
- [ ] Every phase-level verification command passed from `/Users/test1/liuyekang/dev/code/mtls-router`.
- [ ] Any failed verification produced a minimal fix and successful re-verification.
- [ ] Expected commit(s) exist, unless batching commits was explicitly chosen.
- [ ] `git status --short` contains no unexpected source changes.
