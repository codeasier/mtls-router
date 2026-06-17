# mtls-router Phase 8: Final Verification Plan

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

**Prerequisite:** Phases 1-7 must pass.

**Next phase after pass:** None. This is the final phase.

---
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
---

## Phase Completion Checklist

- [ ] Every ticket in this phase completed: 理解 -> 实施 -> 验收 -> fix if needed -> 重新验收.
- [ ] Every expected file listed in this phase exists.
- [ ] Every ticket-level verification command passed.
- [ ] Every phase-level verification command passed from `/Users/test1/liuyekang/dev/code/mtls-router`.
- [ ] Any failed verification produced a minimal fix and successful re-verification.
- [ ] Expected commit(s) exist, unless batching commits was explicitly chosen.
- [ ] `git status --short` contains no unexpected source changes.
