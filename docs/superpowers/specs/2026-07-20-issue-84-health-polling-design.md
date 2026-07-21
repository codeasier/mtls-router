# Issue 84: Reliable Health Polling and Connection Reuse

## Problem

CodeasierRouter Desktop can temporarily report `Upstream unavailable` or `Router failed to start` while the router process and proxy traffic remain healthy. The manager currently performs `/health` as part of `router.status`, but the status operation and management HTTP client both have one-second timeouts while the router's upstream probe may take up to ten seconds.

Health polling also creates excessive TCP connections in `TIME_WAIT`. Every upstream probe parses the credentials and creates a new `http.Transport` and `http.Client`. A successful response body is closed without being drained, so the connection cannot reliably return to an idle pool for reuse.

The fix must preserve prompt detection of genuine router failures and explicit degraded upstream responses while preventing a single slow or failed management poll from replacing a recent successful state.

## Goals

- Keep `router.status` lightweight and independent of upstream probe latency.
- Give explicit health checks enough time to contain the router's configured ten-second upstream probe.
- Reuse upstream health-check connections across startup and runtime probes.
- Keep health-check connections isolated from reverse-proxy traffic.
- Preserve a fresh successful desktop status or health result across a transient polling error.
- Continue to report genuine `start_failed` and explicit upstream `degraded` states immediately.

## Non-Goals

- Changing the proxy request path or its transport.
- Caching upstream health results in the router or Go manager.
- Changing the desktop polling intervals.
- Changing management response schemas.
- Retrying status requests or health probes automatically.
- Suppressing sustained failures after the existing health freshness window expires.

## Manager Discovery

Discovery will expose separate process-only and health-aware paths.

The process-only path performs the existing state-file reads, port check, `/version` request, process validation, and ownership correlation. It does not request `/health`. `router.status`, `router.version`, and other operations that only need trusted router identity use this path. A correlated running router remains `desktop_owned` or `external_compatible` regardless of upstream health.

The health-aware path validates router identity and then requests `/health`. It retains the current distinction between an absent local router, an unknown port occupant, and an explicit upstream `degraded` result. `router.health` uses this path.

Shared internal discovery logic will avoid duplicating identity and ownership classification. The public paths differ only in whether upstream health is requested and incorporated into the result.

## Timeout Budget

The timeout chain must give each outer layer more time than the operation it contains:

| Layer | Budget |
| --- | ---: |
| `router.status` manager deadline | 1 second |
| Router upstream probe | 10 seconds |
| Discovery `/health` HTTP request | 11 seconds |
| `router.health` manager deadline | 12 seconds |
| Desktop Rust watchdog | 13 seconds |

The one-second identity request timeout remains appropriate for `/version`. Health gets a separate request timeout rather than increasing every discovery request timeout.

These strict inequalities remove the existing boundary races. A healthy probe may consume its full router budget and still return through discovery and the protocol before either enclosing deadline expires.

## Reusable Health Client

The router will build one dedicated upstream health client during startup:

1. Validate the upstream URL and TLS minimum.
2. Parse the client certificate, private key, and upstream CA once.
3. Create one dedicated `http.Transport` with the mTLS configuration.
4. Create one `http.Client` backed by that transport.
5. Use the client for the startup probe and every runtime `/health` probe.
6. Close idle health connections during router shutdown.

The health transport will not be shared with the reverse proxy. This prevents probes from consuming proxy idle capacity or otherwise coupling management traffic to user traffic.

Each probe creates a new request context using the configured probe timeout. The shared client and transport do not impose a shorter global timeout.

On a successful HTTP response, the probe drains the response body to EOF and then closes it. Draining is required for HTTP/1.1 connection reuse. Responses with status 500 or higher remain probe failures, but their bodies are also drained and closed before returning.

The probe does not cache results. Every call still reaches the upstream; only the established transport connection and parsed TLS configuration are reused.

## Desktop State Presentation

Polling errors and observed router states remain separate signals.

### Status Errors

If a status poll fails and a previous successful status exists, the desktop keeps presenting that status and displays a status-read warning. It does not change the main state to `Router failed to start`.

If no successful status has ever been received, the desktop displays a distinct `Router status unavailable` state. This state communicates uncertainty and does not imply that startup failed.

The `Router failed to start` presentation remains reserved for an explicit `start_failed` status or an actual failed start action that has not been superseded by a later successful status.

### Health Errors

If a health poll fails and a previous health result is still within the existing 30-second freshness window, the desktop keeps presenting that result and displays a health-check warning. A recent `ok` result therefore remains healthy after one transient timeout.

When the retained result crosses the 30-second freshness threshold, it becomes stale through the existing freshness calculation. If no previous health result exists, health remains unknown while the warning is shown.

An explicit successful `/health` response with `status: degraded` takes effect immediately and is not masked by a prior healthy result.

The tray follows the same principle: a polling error does not replace a known cached process state with a startup-failure presentation. An error with no cached status may still use an unavailable/error presentation.

## Data Flow

### Status Poll

1. Desktop sends `router.status`.
2. Manager runs process-only discovery.
3. Discovery checks local identity through `/version` and durable state.
4. Manager returns process status within the one-second deadline.
5. Desktop updates the cached status, or retains the previous status and records a warning if the poll fails.

No upstream health request occurs in this flow.

### Health Poll

1. Desktop sends `router.health`.
2. Manager runs health-aware discovery.
3. Discovery validates the local router and requests `/health` with an 11-second HTTP timeout.
4. Router performs the upstream request through its dedicated reusable mTLS client, bounded by ten seconds.
5. Manager returns the observed health within its 12-second deadline.
6. Desktop updates health, or retains the previous fresh result and records a warning if the poll fails.

## Error Handling

- Identity request failures remain local router discovery failures; they are not treated as upstream degradation.
- An explicit `/health` `degraded` response remains a successful management response containing degraded health.
- A health HTTP timeout returns the existing sanitized manager error without exposing upstream or credential details.
- A manager deadline expiry remains `OPERATION_TIMEOUT`.
- Probe response bodies are drained best-effort. A body-drain failure does not replace a more meaningful probe status or transport error.
- Health transport construction failures abort router startup, as invalid embedded mTLS configuration does today.
- Genuine unexpected desktop-router exits continue to produce the latched and sanitized `start_failed` result.

## Testing

### Go Health Tests

- Verify repeated successful probes use one TCP connection when the upstream supports keep-alive.
- Verify response bodies are drained and closed for reusable connections.
- Preserve coverage for successful mTLS probes, TLS failures, 5xx responses, invalid URLs, and timeouts.
- Verify idle health connections are closed during router shutdown where practical at the construction boundary.

### Go Discovery and Protocol Tests

- Verify process-only discovery never requests `/health`, including when `/health` blocks.
- Verify a health response slower than the identity timeout but within the health timeout remains healthy.
- Verify `router.status` uses process-only discovery and `router.health` uses health-aware discovery.
- Keep the one-second `router.status` deadline and update `router.health` to 12 seconds.
- Verify timeout errors remain stable and sanitized.

### Desktop Rust Tests

- Update the health watchdog expectation to 13 seconds while status remains two seconds.
- Verify scheduler snapshots retain the previous successful status and health values when polling records an error.
- Verify tray presentation uses cached status when available.

### React Tests

- Verify a status polling timeout with cached healthy state displays a warning without displaying `Router failed to start`.
- Verify a status polling timeout without cached state displays `Router status unavailable`.
- Verify a health polling timeout retains fresh healthy presentation and displays a warning.
- Verify retained health becomes stale after 30 seconds.
- Verify explicit `degraded` and genuine `start_failed` states still take effect immediately.
- Update English and Chinese locale coverage for the new status-unavailable presentation.

## Verification

Run focused package and component tests while implementing, followed by:

```bash
go test ./...
go vet ./...
make test-shell
test -z "$(gofmt -l .)"
```

Run the desktop project's configured React and Rust test suites using the commands declared in its package and Cargo configuration.
