# Issue #35 — Remove dead stream-sniffing code

**Status:** approved
**Date:** 2026-06-23
**Branch:** `fix/issue-35-remove-dead-stream`
**Worktree:** `.worktrees/issue-35-remove-dead-stream`
**Issue:** https://github.com/codeasier/mtls-router/issues/35
**Review follow-up:** https://github.com/codeasier/mtls-router/pull/36

## Problem

`internal/proxy/proxy.go` and `internal/proxy/stream.go` carry a request-side stream-detection layer that:

1. Writes a `streamKey` boolean into the request context, but **nothing in production reads it**.
2. Pre-reads the first 4 KB of the request body to feed that detection, but **`httputil.ReverseProxy` reads `r.Body` itself** and streams the bytes to the upstream — the pre-read buffer is then re-injected onto a body the proxy never would have consumed otherwise.

The mechanisms that actually make streaming work (`FlushInterval: -1` on the reverse proxy and SSE header injection in `ModifyResponse`) are unconditional and independent of any pre-read or context value.

Evidence (collected by the issue reporter and confirmed during review):

- `streamKey` is written in exactly one production call site: `internal/proxy/proxy.go` (`contextWithStream(ctx)`). It is read in zero production call sites — only the deleted `stream_test.go` used the reader.
- `SniffStream` (the exported "full" sniffer in `stream.go`) has no production callers.
- The 4 KB pre-read buffer's only consumer was the now-deleted `containsStreamTrue` detector. After removal, the buffer has no consumer at all; `httputil.ReverseProxy` streams `r.Body` to the upstream regardless.
- `WrapHandler` becomes `http.HandlerFunc(rp.ServeHTTP)` — a pure pass-through with no remaining behavior, same dead-structure shape as the deleted `stream.go`.

## Decision

Delete the entire dead detection path **and** the now-orphaned 4 KB body pre-read, then collapse `WrapHandler` into the single remaining call site. This is the cleanup path from the issue's "Suggested Resolution" list, extended with the body-pre-read removal that surfaced during code review.

We do **not** adopt option 2 from the issue (wire `IsStreamRequest` into the access log) — the user explicitly chose cleanup over observability. We do **not** adopt option 3 (comment-only) — it leaves the test/coverage gap. We do **not** keep `WrapHandler` as a "future extension point" — that is the same "code that pretends to do something" pattern the cleanup is trying to eliminate, and any future request-side hook can intercept at the `mux.Handle("/", withAccessLog(...))` call site directly.

### Behavioral side effect (tracked separately)

Removing the 4 KB pre-read changes one error path:

- **Before:** a non-EOF read error inside the 4 KB window short-circuits with `http.StatusBadRequest` (plain text).
- **After:** the read error reaches `httputil.ReverseProxy.Transport.RoundTrip`, where `NewErrorHandler()` returns `502 {"error":"Bad Gateway"}` (504 on timeout).

Client-body read failures beyond the 4 KB window already went through `ReverseProxy` before, so this only narrows the surface where a client error gets reclassified. We accept the change in #36; the question of whether to add a dedicated `400-on-client-body-error` contract is tracked in #37.

## Changes

### 1. Delete `internal/proxy/stream.go`

Unreachable once its sole caller stops using `containsStreamTrue` / `contextWithStream` / `SniffStream` / `replayReadCloser`. None of these have production callers.

### 2. Delete `internal/proxy/stream_test.go`

All tests exercise `SniffStream` or `IsStreamRequest`, both of which go away.

### 3. Delete `internal/proxy/proxy_test.go`

After collapsing `WrapHandler`, the package has no test surface left in this file (the remaining `director_test.go`, `errorhandler_test.go`, `modifyresponse_test.go`, `transport_test.go` cover their own units).

### 4. Modify `internal/proxy/proxy.go`

- Remove `WrapHandler` entirely.
- Drop the `bytes` and `httputil.ReverseProxy`-via-`http.Handler` wrapper imports if no longer used.
- The remaining exported surface is just `New` / `NewMTLSTransport` / `NewDirector` / `NewModifyResponse` / `NewErrorHandler` / `Options` / `WithTLSMin` / the `*httputil.ReverseProxy` returned by `New`.

### 5. Modify `main.go`

The single caller of `WrapHandler` becomes:

```go
mux.Handle("/", withAccessLog(reverseProxy, logger))
```

### 6. Update `AGENTS.md`

- Delete the `proxy.WrapHandler` row from CODE MAP.
- Delete the `SniffStream` row (already gone from the prior pass; verify).
- Rewrite the ANTI-PATTERNS bullet about buffering request bodies to reflect that the proxy streams bodies straight through with no pre-read.

### 7. Update `README.md` / `docs/zh-CN/README.md`

The "Streaming and SSE" paragraph must describe pass-through streaming and `FlushInterval: -1`, not body buffering.

## Out of Scope

- Adding observability (e.g., a `stream: true|false` field in `AccessLog`) — explicitly rejected.
- Replacing `FlushInterval: -1` with per-request stream-aware flushing — streaming already works.
- A `400-on-client-body-error` contract — tracked in #37, not here.

## Files Touched

| File | Action |
|---|---|
| `internal/proxy/stream.go` | delete |
| `internal/proxy/stream_test.go` | delete |
| `internal/proxy/proxy_test.go` | delete |
| `internal/proxy/proxy.go` | edit (drop `WrapHandler`, drop unused imports) |
| `main.go` | edit (drop `WrapHandler` wrapper) |
| `AGENTS.md` | edit (CODE MAP + ANTI-PATTERNS) |
| `README.md` | edit (Streaming paragraph) |
| `docs/zh-CN/README.md` | edit (Streaming paragraph) |

## Verification

Run from the worktree root:

```sh
go test ./...
go vet ./...
test -z "$(gofmt -l .)"
```

Acceptance:

- `go test ./...` passes.
- `go vet ./...` is clean.
- `gofmt -l .` prints nothing.
- `git grep -n "streamKey\|IsStreamRequest\|contextWithStream\|SniffStream\|containsStreamTrue\|WrapHandler\|replayReadCloser\|bodyReplayLimit" -- '*.go'` returns no matches.
- `git grep -n "WrapHandler" -- '*.go' 'AGENTS.md'` returns no matches.

## Risk

Low. No production code path reads the deleted symbols. Streaming continues to work via `FlushInterval: -1` + SSE header injection, both unconditional. The only observable behavior change is the body-read error mapping described above, tracked in #37.