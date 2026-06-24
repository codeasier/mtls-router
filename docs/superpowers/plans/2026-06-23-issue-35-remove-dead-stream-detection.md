# Issue #35 — Remove Dead Stream-Detection Code Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete the dead `streamKey` request-context detection layer in the `internal/proxy` package, the orphaned 4 KB body pre-read it fed, and the now-pure-pass-through `WrapHandler` wrapper left behind by the cleanup.

**Architecture:** Pure deletion. `stream.go` (which writes a context value nothing reads) is removed entirely. `WrapHandler`'s 4 KB pre-read is removed because `httputil.ReverseProxy` reads `r.Body` itself. `WrapHandler` itself is then removed because after both deletions it is `http.HandlerFunc(rp.ServeHTTP)` — the same dead-structure pattern we are deleting. The single caller in `main.go` switches to `withAccessLog(reverseProxy, logger)` directly.

**Tech Stack:** Go 1.26, `httputil.ReverseProxy`, `log/slog`.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `internal/proxy/stream.go` | delete | Holds `streamKey`, `IsStreamRequest`, `contextWithStream`, `SniffStream`, `restoreBody`, `replayReadCloser`, `containsStreamTrue`, `sniffLimit`. All dead in production. |
| `internal/proxy/stream_test.go` | delete | Tests `SniffStream` / `IsStreamRequest` — both APIs go away. |
| `internal/proxy/proxy_test.go` | delete | After collapsing `WrapHandler`, the package has no test surface left in this file (the remaining `*_test.go` files cover their own units). |
| `internal/proxy/proxy.go` | edit | Drop `WrapHandler`. The package keeps `New` / `Options` only. |
| `main.go` | edit | Drop `WrapHandler` wrapper at the single call site. |
| `AGENTS.md` | edit | Drop the `proxy.WrapHandler` row; rewrite the request-body ANTI-PATTERNS bullet to reflect pass-through streaming. |
| `README.md` / `docs/zh-CN/README.md` | edit | Rewrite the "Streaming and SSE" paragraph to describe pass-through + `FlushInterval: -1`, not body buffering. |

---

## Task 1: Delete the dead detection files and the pre-read test scaffold

**Files:**
- Delete: `internal/proxy/stream.go`
- Delete: `internal/proxy/stream_test.go`
- Delete: `internal/proxy/proxy_test.go`

- [ ] **Step 1: Delete the three files**

Run from the worktree root:

```bash
rm internal/proxy/stream.go internal/proxy/stream_test.go internal/proxy/proxy_test.go
```

- [ ] **Step 2: Run `go build ./...` to surface missing references**

Run: `go build ./...`
Expected: FAIL with compile errors in `internal/proxy/proxy.go` and `main.go` referencing the deleted `WrapHandler` and its test helpers.

This is expected — Task 2 fixes them.

---

## Task 2: Edit `internal/proxy/proxy.go` to drop `WrapHandler`

**Files:**
- Modify: `internal/proxy/proxy.go`

- [ ] **Step 1: Replace the file with the cleaned-up version**

Final `internal/proxy/proxy.go`:

```go
package proxy

import (
	"log/slog"
	"net/http"
	"net/http/httputil"
	"net/url"
)

type Options struct {
	Upstream  *url.URL
	Transport *http.Transport
	ErrorLog  *slog.Logger
}

func New(opts Options) *httputil.ReverseProxy {
	rp := &httputil.ReverseProxy{
		Director:       NewDirector(opts.Upstream),
		ModifyResponse: NewModifyResponse(),
		ErrorHandler:   NewErrorHandler(),
		FlushInterval:  -1,
		Transport:      opts.Transport,
	}
	if opts.ErrorLog != nil {
		rp.ErrorLog = slog.NewLogLogger(opts.ErrorLog.Handler(), slog.LevelError)
	}
	return rp
}
```

Notes on the diff vs. the original:

- `WrapHandler` is removed entirely. The downstream proxy handles its own request body via `httputil.ReverseProxy`.
- The `bytes` import is no longer needed (gone with the pre-read).
- The closure-capture comment on the body assignment is gone with the assignment.
- `http.Handler` is no longer imported (no wrapper function left in this package).

- [ ] **Step 2: Run `go build ./...`**

Run: `go build ./...`
Expected: still FAIL because `main.go` references the deleted `WrapHandler`. Task 3 fixes it.

---

## Task 3: Edit `main.go` to drop the `WrapHandler` wrapper

**Files:**
- Modify: `main.go`

- [ ] **Step 1: Replace the call site**

Find:

```go
mux.Handle("/", withAccessLog(proxy.WrapHandler(reverseProxy), logger))
```

Replace with:

```go
mux.Handle("/", withAccessLog(reverseProxy, logger))
```

- [ ] **Step 2: Run `go build ./...`**

Run: `go build ./...`
Expected: PASS, no output.

---

## Task 4: Update `AGENTS.md`

**Files:**
- Modify: `AGENTS.md`

- [ ] **Step 1: Drop the `proxy.WrapHandler` row from the CODE MAP table**

Delete the entire row.

- [ ] **Step 2: Rewrite the request-body ANTI-PATTERNS bullet**

Find the bullet that mentions the 4 KB body replay / sniff window. Replace it with a bullet that captures the new invariant: do not buffer arbitrary-size request bodies; the proxy streams them straight through.

- [ ] **Step 3: Re-read the file**

Confirm the CODE MAP table has no `WrapHandler` row and the ANTI-PATTERNS section reflects pass-through streaming.

---

## Task 5: Update `README.md` and `docs/zh-CN/README.md`

**Files:**
- Modify: `README.md`
- Modify: `docs/zh-CN/README.md`

- [ ] **Step 1: Rewrite the "Streaming and SSE" paragraph**

The new wording must describe pass-through streaming + `FlushInterval: -1`, not request-body buffering or sniffing.

- [ ] **Step 2: Re-read both files to confirm parity**

---

## Task 6: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Run the full verification suite**

From the worktree root:

```bash
go test ./...
go vet ./...
test -z "$(gofmt -l .)"
git grep -n "streamKey\|IsStreamRequest\|contextWithStream\|SniffStream\|containsStreamTrue\|WrapHandler\|replayReadCloser\|bodyReplayLimit" -- '*.go' 'AGENTS.md'
```

Expected: all four commands pass. The last `git grep` prints nothing.

- [ ] **Step 2: Confirm working tree is clean**

Run: `git status`
Expected: clean working tree (apart from any untracked editor/IDE artifacts).

- [ ] **Step 3: Report results back**

Summarize:

- Files deleted: 3 (`stream.go`, `stream_test.go`, `proxy_test.go`)
- Files modified: 4 (`proxy.go`, `main.go`, `AGENTS.md`, `README.md`, `docs/zh-CN/README.md`)
- Test, vet, gofmt results
- `git grep` clean
- Branch name and final commit hash