# Force-Terminate Port Occupant Design

## Problem

The desktop reports an unknown process listening on `127.0.0.1:19099`, but it
cannot help the user release the port. Users must leave the application, find
the listener with operating-system tools, verify that they own it, terminate
it, and return to retry. This is unnecessarily difficult when the listener is a
process owned by the current user.

The existing `router.stop` operation is not a suitable extension point. It
stops only a router proven to be owned by the current desktop or CLI lifecycle
state, and it deliberately refuses unknown or stale identities. That ownership
contract must remain unchanged.

## Decision

Add an explicit, two-stage desktop recovery flow for an unknown port occupant:

1. Inspect the process listening on the configured loopback address.
2. Show its process name, PID, and executable path in a confirmation dialog.
3. Require the user to confirm a destructive action.
4. Reinspect and validate the complete target identity and port ownership.
5. Immediately force-terminate the process.
6. Wait for the port to become free and refresh Router status.

This flow is available only when the listener belongs to the current user and
its complete identity can be read. It does not request elevation and never
signals another user's process. It does not automatically start the router
after releasing the port.

Unknown occupants are never terminated automatically. The former requirement
that an unknown occupant is never killed is replaced by the narrower rule that
it may be force-terminated only after explicit confirmation and successful
identity, ownership, and listener revalidation.

## Safety Boundary

The operation must not trust a PID alone. An occupant identity contains:

- loopback listen address;
- PID;
- operating-system process start identity;
- normalized executable path;
- current-user identity;
- protocol family and local socket identity needed to prove listener ownership.

Inspection fails closed if any required field is unavailable, multiple
processes cannot be reduced to one unambiguous listener, or the process does not
belong to the current user.

The following processes are protected even if they otherwise pass inspection:

- the desktop application;
- the manager process serving the request;
- the packaged router when current lifecycle state proves it is managed;
- any process whose complete identity or user ownership cannot be verified.

The operation supports only an explicit loopback TCP address. It does not kill
all processes associated with a numeric port, wildcard listeners that cannot be
unambiguously correlated, UDP listeners, container-internal processes, or
remote-host processes.

## Manager Protocol

Add two methods without changing `router.stop`:

### `router.inspect_occupant`

This method accepts no caller-supplied PID. It is valid only when discovery
classifies the configured address as `unknown_occupant`.

It returns:

```json
{
  "pid": 12345,
  "process_name": "example-server",
  "executable": "/path/to/example-server",
  "listen_addr": "127.0.0.1:19099",
  "confirmation_token": "opaque-value",
  "expires_at": "2026-07-17T12:00:30Z"
}
```

The response excludes command-line arguments, environment variables, open-file
details, and user identifiers. The process name is derived from the executable
basename and is presentation metadata only.

The manager creates the confirmation token with `crypto/rand` and stores the
corresponding full occupant identity only in memory. A token:

- is bound to one complete occupant identity and listen address;
- expires after 30 seconds;
- is single-use;
- becomes invalid when the manager exits or restarts;
- is removed after any termination attempt, including a failed revalidation.

At most one active token is retained for the configured address. A new
inspection invalidates the previous token.

### `router.force_terminate_occupant`

This method accepts only the opaque confirmation token. It never accepts a PID,
path, or user identity supplied by the client.

Before signaling, the manager must:

1. consume and validate the unexpired token;
2. confirm discovery still classifies the endpoint as `unknown_occupant`;
3. rediscover the unique listener for the same address;
4. compare PID, process start identity, normalized executable, current-user
   identity, protocol family, and listener ownership with the stored identity;
5. confirm the process is not protected;
6. perform one final complete process identity validation immediately before
   signaling.

If every check passes, the manager sends the platform's unconditional process
termination operation immediately. It does not first send a graceful signal.
It then waits for up to two seconds for the process to disappear and the listen
address to reject new TCP connections. Success means the original identity is
gone and the configured address is no longer occupied. A replacement listener
appearing during that interval is reported as a changed occupant and is never
signaled using the consumed token.

## Stable Errors

Add stable protocol codes for client branching:

- `OCCUPANT_NOT_FOUND`: no listener currently owns the configured address.
- `OCCUPANT_NOT_OWNED`: the listener does not belong to the current user.
- `OCCUPANT_IDENTITY_UNAVAILABLE`: complete identity cannot be established.
- `OCCUPANT_CHANGED`: identity or listener ownership changed after inspection.
- `OCCUPANT_PROTECTED`: the target is a desktop, manager, or managed router.
- `OCCUPANT_TERMINATION_FAILED`: the operating system rejected termination or
  the target remained alive.
- `PORT_RELEASE_TIMEOUT`: the original target exited but the address did not
  become free within the bounded wait.
- `CONFIRMATION_EXPIRED`: the token is absent, consumed, superseded, or expired.

Errors are sanitized. They may report a PID already shown by inspection but
must not expose command-line arguments, environment variables, socket table
contents, or unrelated process metadata.

Both methods use bounded protocol deadlines. Inspection must finish within two
seconds. Force termination must finish within three seconds, including the
two-second release wait.

## Platform Implementation

Create a focused manager package that discovers a TCP listener and returns a
platform-independent occupant identity. Keep platform socket-table code behind
build-tagged files and reuse the existing process identity package where
possible.

### macOS

Use Darwin system process/socket facilities to enumerate TCP listeners and
associate the exact local endpoint with a PID. Use existing Darwin process
inspection for process start and executable identity, and obtain process user
ownership from native process metadata. Do not shell out to `lsof`, `netstat`,
or `ps`.

### Linux

Read `/proc/net/tcp` and `/proc/net/tcp6` to identify matching listening socket
inodes, associate those inodes with `/proc/<pid>/fd` entries, and obtain UID
from trusted procfs process metadata. Reuse `/proc/<pid>/stat` and
`/proc/<pid>/exe` for process start and executable identity. Treat procfs races,
permission failures, or multiple candidate owners as unverifiable.

### Windows

Use `GetExtendedTcpTable` from the IP Helper API to identify the owning PID for
the exact local TCP endpoint. Use the process token user SID for ownership and
reuse process creation time and executable path for complete identity. Use
native process termination rather than PowerShell or `taskkill`.

All platforms must handle `127.0.0.1:19099`. An IPv6 or wildcard listener may
be handled only when the platform implementation proves one unique owner for
the endpoint; otherwise inspection fails closed.

## Desktop Integration

The Router page retains `occupied` as a distinct state. While in that state it
requests occupant inspection independently of normal status polling.

When inspection succeeds, the page displays the process name and PID and
enables a destructive `Force terminate occupant` action. Selecting the action
opens a modal dialog containing:

- process name;
- PID;
- executable path;
- a warning that termination is immediate and unsaved data may be lost;
- a default-focused Cancel button;
- a destructive `Force terminate` confirmation button.

The executable path is allowed to wrap and is not truncated in a way that hides
the target identity. It is not copied to logs or telemetry.

While termination is in progress, both dialog actions and the underlying
recovery button are disabled. On success, the dialog closes, status refreshes,
and the UI reports that the port was released. The router remains stopped until
the user selects Start router.

When inspection is unavailable, the UI keeps the occupied state and explains
why the action is unavailable:

- another user owns the process;
- identity could not be verified;
- the target is protected;
- listener ownership is ambiguous;
- inspection failed and may be retried.

The regular Stop router action remains disabled for unknown occupants. The tray
does not expose force termination because it cannot provide adequate target
identity and confirmation context.

All new strings are localized in English and Simplified Chinese.

## State and Concurrency

Status polling may continue while the confirmation dialog is open. If polling
shows that the endpoint is no longer `unknown_occupant`, the dialog closes and
its token is discarded client-side; the manager still performs authoritative
token and identity validation if a raced request arrives.

Only one inspection or force-termination operation for the configured address
may execute at a time. The manager serializes these operations with occupant
token state so simultaneous desktop requests cannot reuse one confirmation.

No occupant identity or token is written to router state files. Diagnostics may
report the high-level occupied/unavailable reason but not the token or full
executable path.

## Documentation Changes

Update the live desktop specification and checklist to replace the absolute
prohibition on terminating unknown occupants. The revised requirement states:

- unknown occupants are never terminated automatically;
- the desktop may terminate a current-user occupant only through this explicit
  inspected and confirmed flow;
- other-user, ambiguous, changed, protected, and unverifiable occupants are
  never signaled.

Update English and Simplified Chinese desktop and troubleshooting documents.
The troubleshooting flow should prefer the inspected desktop recovery action
when available and retain operating-system guidance for unsupported targets.

## Testing

### Unit Tests

- Token generation uses unpredictable values and binds the full identity.
- Tokens expire after 30 seconds, are single-use, are invalidated by a newer
  inspection, and do not survive a manager restart.
- Missing, malformed, superseded, expired, and replayed tokens send no signal.
- A changed PID, start identity, executable, user identity, address, protocol
  family, or listener owner sends no signal.
- Other-user, protected, ambiguous, and incomplete identities send no signal.
- Sanitized errors do not expose command lines, environment data, or unrelated
  socket metadata.

### Manager and Platform Tests

- Start a current-user test listener on an ephemeral loopback port, inspect it,
  confirm termination, and verify that the exact identity exits and the port is
  released.
- Replace the listener between inspection and confirmation and verify that the
  replacement receives no signal.
- Exercise PID/start/executable mismatch paths through injected platform
  functions without relying on PID reuse occurring naturally.
- Verify forced termination is immediate and does not send a graceful signal.
- Verify a replacement listener appearing during the release wait is reported
  without being terminated.
- Run platform-specific discovery tests on macOS, Linux, and Windows CI.

Tests must use ephemeral ports where practical. Production behavior remains
bound to the configured `127.0.0.1:19099` address.

### Desktop Tests

- The destructive action appears only for an inspectable unknown occupant.
- Disabled states explain other-user, protected, ambiguous, and unverifiable
  targets.
- The modal displays process name, PID, path, and the data-loss warning.
- Cancel is initially focused and sends no termination request.
- Confirmation sends only the opaque token and prevents duplicate submission.
- Success closes the modal, refreshes status, and does not start the router.
- `OCCUPANT_CHANGED` and `CONFIRMATION_EXPIRED` close or refresh stale target
  details and require a new inspection.
- Existing ownership tests continue to prove that normal `router.stop` cannot
  stop external, stale, or unknown processes.

## Acceptance Criteria

1. No process receives a signal before explicit confirmation.
2. No process receives a signal based on PID alone.
3. Other-user, incomplete, ambiguous, protected, expired, or changed targets
   receive no signal.
4. A confirmed current-user target with unchanged complete identity and
   listener ownership is force-terminated immediately.
5. The operation verifies that the original target exited and reports whether
   the port was released.
6. Releasing the port never starts the router automatically.
7. macOS, Linux, and Windows expose the same manager protocol and desktop
   behavior without external process-discovery commands.
8. Existing safe lifecycle semantics for desktop-owned and CLI-owned routers
   remain unchanged.
