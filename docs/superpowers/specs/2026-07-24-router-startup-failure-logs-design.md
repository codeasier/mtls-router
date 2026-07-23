# Router Startup Failure Logs Design

## Problem

The desktop manager captures router stdout and stderr in both the router log file and a bounded in-memory buffer. A router that exits immediately during startup can still lose its diagnostic association:

- On Windows, the suspended child is resumed before lifecycle readiness checks begin. It can exit before process identity inspection.
- An identity inspection error currently returns from startup without first waiting for `child.Wait()`. Because stdout and stderr are connected through an `io.MultiWriter`, `Wait()` is the synchronization point that drains the `os/exec` copy pipes.
- A process that fails before readiness is never installed as the active desktop run, so its completed in-memory snapshot is not reported through the existing unexpected-exit path.
- The app maps lifecycle errors to safe generic protocol errors and does not latch startup output as `start_failed` diagnostics.
- `router.logs` can therefore read an empty or temporarily inaccessible file before the failed child's output has been drained, with no reliable in-memory fallback.

The result is a generic router startup failure screen and an empty log viewer at the point where diagnostics are most needed.

## Goals

- Complete stdout and stderr draining before any launched-process startup failure is returned.
- Preserve a bounded snapshot of immediate startup failure output for the current manager session.
- Expose sanitized recent output through the existing `start_failed.recent_logs` status shape.
- Make the same output available through `router.logs`, even when the disk log is empty or temporarily unreadable.
- Show a diagnostic summary on the router failure page and retain access to the fuller bounded log in the Logs page.
- Preserve stable protocol error codes and safe user-facing error messages.
- Cover the Windows immediate-exit race and the common cross-platform lifecycle contract with tests.

## Non-Goals

- Persisting the `start_failed` status or its summary across desktop application restarts.
- Adding a new manager protocol method or changing protocol version 3.
- Exposing raw lifecycle errors or unsanitized router output to the desktop webview.
- Adding automatic polling to the Logs page.
- Redesigning the router or Logs pages beyond the existing diagnostic presentation patterns.

## Considered Approaches

### 1. Synchronize lifecycle failures and reuse `start_failed` diagnostics

All startup failure paths wait for process cleanup and output draining, then return a structured lifecycle failure containing bounded output. The app sanitizes and latches that output using the existing startup-failure status shape. `router.logs` uses the completed snapshot as a fallback or missing-tail source.

This is the selected approach because it fixes the race at its owner, preserves security boundaries, and reuses existing protocol and UI concepts.

### 2. Retry log loading from the desktop after `router.start` fails

The desktop could delay and retry `router.logs`. This may hide the most common timing window, but it does not guarantee output draining, associate output with the failed attempt, handle file-access failures, or populate the router failure summary.

### 3. Persist startup failure events

The manager could write a separate failure event file. This would support restoring the failure card across application restarts, but that behavior is not required and would add state migration, cleanup, permissions, and sensitive-data handling responsibilities.

## Architecture

Responsibilities remain separated by layer:

- `internal/manager/lifecycle` owns child process launch, termination, waiting, output capture, and the raw bounded startup failure snapshot.
- `internal/manager/app` owns protocol-safe error mapping, sensitive-text sanitization, session-scoped failure latching, and log result assembly.
- The Rust desktop backend continues to transport existing manager protocol results without interpreting raw process output.
- The React frontend renders the existing `start_failed.recent_logs` diagnostics and provides navigation to the Logs page.

No raw startup output is placed in the generic protocol error message. The synchronous `router.start` response keeps its stable error code and generic safe text. Diagnostics become available through the subsequent `router.status` refresh and `router.logs` request.

## Lifecycle Data Flow

1. Lifecycle creates the bounded in-memory output capture and opens the router log destination.
2. Lifecycle launches the router with stdout and stderr connected to the combined destination.
3. Readiness polling observes one of four outcomes: ready, exited, process identity cannot be inspected, or timeout/cancellation.
4. For any non-ready outcome after launch, lifecycle enters one cleanup path:
   - terminate the child if it can still be running;
   - wait for `child.Wait()` completion;
   - close the manager-owned log handle;
   - read the final bounded output snapshot;
   - remove or avoid writing runtime state for the failed attempt.
5. Lifecycle returns a typed startup failure that retains the existing public error category and carries the raw bounded snapshot for in-process consumers.
6. The app maps the public category to the existing stable protocol error while separately sanitizing and latching the snapshot as `start_failed.recent_logs`.
7. The desktop scheduler refreshes `router.status`, and the Router page displays the sanitized summary.

Cleanup must be idempotent within a start attempt. Waiting and log closure must have a single owner so timeout, inspection failure, and observed exit cannot race to close or consume output twice.

## Failure State

The app keeps one session-scoped startup failure record. It contains only the information already allowed in `start_failed`, including sanitized, bounded recent log lines.

The record is set when a launched router fails before readiness and is cleared when:

- a later router start succeeds; or
- the manager process exits.

An intentional stop does not create a startup failure. A failure before any router process is launched, such as sidecar validation failure, continues to return its existing protocol error and has no synthetic router logs.

Startup failures remain distinct from unexpected exits after readiness. They may share sanitization and bounded-log helpers, but lifecycle startup failures are delivered synchronously rather than through the post-start unexpected-exit channel.

## Log Assembly

`router.logs` continues to treat the disk log as the primary historical source. The app also obtains the latest fully drained startup failure snapshot from lifecycle or its latched failure record.

The result is assembled as follows:

1. Read and sanitize the requested bounded tail from disk.
2. Sanitize and bound the in-memory startup failure lines with the same rules.
3. If disk is empty, return the available in-memory lines.
4. If disk has content but lacks lines present at the end of the completed snapshot, append only the missing tail while preserving order and the response limit.
5. If disk reading fails but in-memory lines are available, return those lines rather than discarding useful diagnostics.
6. If disk reading fails and no in-memory lines exist, return the existing log-read protocol error.

Deduplication is limited to overlap between the disk tail and the in-memory snapshot. It must not globally remove repeated log lines, because repeated router messages can be meaningful.

## Security And Bounds

- Raw child output remains inside the Go manager.
- Existing sensitive-text sanitization applies before output enters protocol results.
- Existing line-count, line-length, and total response bounds remain authoritative for `recent_logs` and `router.logs`.
- Generic startup error messages remain free of paths, upstream addresses, certificate material, keys, and raw process output.
- Logging behavior must not add request bodies, API keys, environment values, or command-line secrets.
- The in-memory startup failure record is not persisted and is discarded with the manager process.

## Desktop Behavior

After a failed start request, the Router page initially keeps its generic safe error. The scheduler refresh then returns `start_failed` with sanitized recent logs. The existing failure card displays the recent lines and offers an action that navigates to the Logs page.

The Logs page loads the bounded log through the existing `router.logs` command. Because the manager guarantees completed output and memory fallback, no frontend delay or retry loop is required. Manual refresh remains available.

Chinese and English text must remain equivalent. No raw Rust transport error, manager stderr, or lifecycle error is rendered.

## Testing

### Go lifecycle

- Launch a helper child that writes to stderr and exits immediately; assert `Start` does not return until the complete output is captured.
- Force process identity inspection to lose the race with child exit; assert output is still drained before return.
- Assert failed startup leaves no runtime state file and no active desktop run.
- Assert timeout and cancellation use the same cleanup contract and do not deadlock or double-close resources.
- Preserve the existing rule that startup failure is not reported as a post-readiness unexpected exit.

### Windows lifecycle

- Use a short-lived helper process to cover suspended launch, resume, immediate stderr output, and exit.
- Assert the platform launch path still applies required creation flags and job-object ownership.
- Run Windows-specific behavior in CI; locally verify cross-compilation or compilation where supported.

### Go app

- Assert a launched-process startup failure latches `start_failed` with sanitized bounded recent logs.
- Assert raw sensitive values are absent from both protocol errors and status diagnostics.
- Assert a successful later start clears the latched startup failure.
- Assert pre-launch validation failures do not synthesize startup output.
- Assert `router.logs` handles an empty file, unavailable file with memory fallback, overlapping disk and memory tails, and complete lack of both sources.

### Desktop frontend

- Assert scheduler status refresh displays startup failure recent logs on the Router page.
- Assert the failure action navigates to the Logs page.
- Assert the Logs page renders startup failure output returned by the existing API.
- Preserve existing redaction and bounded-display tests.

### Verification

Run:

```bash
go test ./internal/manager/lifecycle ./internal/manager/app
go test ./...
cd desktop && npm test
cd desktop && npm run static:check
cd desktop && npm run typecheck
```

Windows-specific tests are expected to execute in the repository's Windows CI job.

## Acceptance Criteria

- A router that writes a diagnostic line and exits immediately cannot return from lifecycle startup before that line has been drained.
- The Windows immediate-exit race produces a sanitized `start_failed.recent_logs` summary in the current desktop session.
- The same failure output is available in the Logs page without requiring an application restart.
- A temporary disk log read failure does not hide a completed in-memory startup failure snapshot.
- Successful startup clears the prior failure card; restarting the desktop app does not restore it.
- Protocol error codes, protocol version, and safe generic startup messages remain unchanged.
- All Go, frontend, formatting, type-check, and applicable Windows CI tests pass.
