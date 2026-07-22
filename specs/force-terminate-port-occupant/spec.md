# Force-Terminate Port Occupant Specification

## Motivation

The desktop reports an unknown process listening on `127.0.0.1:19099`, but the
user must leave the application and use operating-system tools to release the
port. The desktop should offer an explicit recovery action when the listener can
use the default complete-identity path, plus a narrowly degraded Windows-only
path when one exact TCP4 listener has one unique owner PID.

The existing `router.stop` method is not changed. It remains limited to router
processes proven to be owned by the applicable desktop or CLI lifecycle state.

## Scope

This change adds:

- native listener-owner discovery on macOS, Linux, and Windows;
- complete occupant identity and current-user ownership validation by default;
- a Windows-only PID-based exception with an explicit unverified warning;
- a short-lived, single-use confirmation-token protocol;
- immediate force termination after explicit desktop confirmation;
- bounded verification that the target exited and the port was released;
- an occupied-state recovery panel and destructive confirmation dialog;
- stable protocol errors, tests, and paired English/Chinese documentation.

## Exclusions

This change does not:

- terminate an occupant automatically;
- request administrator elevation;
- accept a client-supplied PID, executable path, or user identity for signaling;
- add force termination to the tray, setup scripts, router HTTP endpoints, or
  the `mtls-router` binary;
- change normal `router.stop`, Stop, Quit, or external-router ownership rules;
- terminate UDP, remote-host, container-internal, IPv6, ambiguous wildcard, or
  socket-owner-ambiguous listeners;
- automatically start the router after releasing the port;
- depend on `lsof`, `netstat`, `ps`, PowerShell, or `taskkill`.

## Terminology

- **Occupant:** the process that owns the TCP listener matching the configured
  loopback address.
- **Complete occupant identity:** listen address, protocol family, local socket
  identity, PID, process start identity, normalized executable path, and
  operating-system user identity.
- **Inspectable occupant:** one unique occupant whose complete identity is
  readable and belongs to the current user.
- **Windows PID-only occupant:** one exact TCP4 listener with one unique owner
  PID, but without verified process identity, owner, start time, or executable.
- **Protected process:** the desktop application, the serving manager, or a
  router proven by lifecycle state to be managed.

## Requirements

### FR-1: Supported endpoint and platforms

The manager MUST support exact TCP listener discovery for
`127.0.0.1:19099` on macOS, Linux, and Windows while preserving
`CGO_ENABLED=0` builds.

The implementation MAY support another configured explicit loopback TCP
address for tests and manager configuration. It MUST fail closed for UDP,
non-loopback addresses, an ambiguous wildcard listener, multiple candidate
owners, or an unsupported platform. Inaccessible complete identity MUST fail
closed on macOS and Linux; on Windows it MAY use the PID-only exception in FR-2.

Platform implementations MUST use native facilities:

- macOS: Darwin process and socket inspection facilities;
- Linux: `/proc/net/tcp*`, `/proc/<pid>/fd`, and process metadata in procfs;
- Windows: IP Helper TCP owner tables and process-token user SID metadata.

#### Scenario: unique current-user listener

Given one current-user process owns the exact loopback TCP listener, when the
manager inspects the occupant, then it returns that process's complete internal
identity and safe presentation fields.

#### Scenario: ambiguous or inaccessible listener

Given listener ownership cannot be reduced to one complete identity on macOS
or Linux, or Windows cannot reduce the exact TCP4 owner table to one unique PID,
when the manager inspects the occupant, then it reports identity unavailable
and sends no signal.

### FR-2: Process and user identity

Complete identity MUST remain the default on every platform. In that mode,
process comparison MUST include PID, operating-system start identity, and
normalized executable path. Occupant comparison MUST additionally include user
identity, listen address, protocol family, and socket ownership identity.

On Windows only, when the exact TCP4 owner table proves one unique PID but the
complete path is unavailable, inspection MAY return `windows_pid_only`. This
mode MUST NOT claim verified identity or current-user ownership. A failed SID,
process inspection failure, or readable other-user SID MAY enter this mode.
macOS and Linux MUST NOT produce or execute a PID-only target. No mode may
request elevation.

The desktop process and serving manager process MUST be protected from both
modes. Complete identity MUST retain its existing managed-router protection.
PID-only mode MUST protect a router PID found in readable desktop or CLI
lifecycle state. An unreadable lifecycle state file is skipped and does not
alone block PID-only recovery; this approved availability tradeoff can leave a
managed router unrecognized.

#### Scenario: PID or listener reuse

Given any complete-identity field changes, or a Windows PID-only listener
disappears, changes PID, or becomes duplicate, wildcard, malformed, or otherwise
ambiguous after inspection, when termination is requested, then the manager
returns `OCCUPANT_CHANGED` and sends no signal to the old or replacement
process. The immediate PID owner recheck narrows but cannot eliminate PID reuse
between reading the TCP table and obtaining the termination handle.

#### Scenario: other-user process

Given complete inspection proves the listener belongs to another user on macOS
or Linux, when inspection is requested, then the manager returns
`OCCUPANT_NOT_OWNED` and exposes no termination token. On Windows, a readable
other-user SID MAY instead produce a warned `windows_pid_only` target.

### FR-3: Occupant inspection protocol

The manager MUST add `router.inspect_occupant`. The method accepts no parameters
and has a two-second internal deadline. It is valid only while router discovery
classifies the configured endpoint as `unknown_occupant`.

On success it returns:

```json
{
  "pid": 12345,
  "verification_mode": "verified_identity",
  "process_name": "example-server",
  "executable": "/path/to/example-server",
  "listen_addr": "127.0.0.1:19099",
  "confirmation_token": "opaque-value",
  "expires_at": "2026-07-17T12:00:30Z"
}
```

`verification_mode` MUST be `verified_identity` or `windows_pid_only`.
`process_name` MUST be presentation metadata derived from the executable
basename and, with `executable`, MUST be omitted for `windows_pid_only`. A
PID-only response shows only PID and listen address as target details. The
response MUST NOT include command-line arguments, environment variables, user
identifiers, unrelated socket data, or open-file details.

The token MUST be generated with a cryptographically secure random source. It
MUST bind the target mode and configured address, plus either the complete
occupant identity or Windows unique owner PID. It MUST expire after 30 seconds,
be single-use, exist only in manager memory, and become invalid on manager
restart. A new successful inspection for the address MUST invalidate the
previous token.

#### Scenario: inspection supersedes an earlier token

Given two successful inspections occur, when the first token is submitted,
then the manager returns `CONFIRMATION_EXPIRED` and sends no signal.

### FR-4: Force-termination protocol

The manager MUST add `router.force_terminate_occupant`. The method accepts only
`confirmation_token` and uses a three-second protocol window. Release
verification uses the mode-specific timing defined below; this window is not a
strict three-second wall-clock completion guarantee for the Windows PID-only
final classification lookup.

The manager MUST atomically consume the token before performing termination
checks. Consumed, expired, malformed, missing, superseded, or replayed tokens
MUST NOT authorize another attempt.

Before signaling a complete-identity target, the manager MUST:

1. verify the unexpired token;
2. confirm discovery remains `unknown_occupant`;
3. rediscover one unique listener for the same address;
4. compare the complete occupant identity with the token-bound identity;
5. recheck current-user ownership and protected-process rules;
6. perform final process identity validation immediately before signaling.

Before signaling a Windows PID-only target, the manager MUST consume the same
single-use token, confirm discovery remains `unknown_occupant`, immediately
query the exact TCP4 owner again, require the same unique PID on the same exact
address, and recheck desktop, manager, and readable managed-router PID
protection. Disappearance, PID change, duplicate rows, wildcard ownership,
malformed rows, or any other ambiguity MUST return `OCCUPANT_CHANGED` without a
signal.

If every check passes, the manager MUST immediately use the platform's
unconditional process termination operation. It MUST NOT first send a graceful
signal.

The manager MUST then verify release. Complete mode retains the existing
at-most-two-second wait for the original identity to disappear and the address
to reject new TCP connections. PID-only mode polls exact listener ownership
under a two-second release deadline, then performs one final
`PollInterval`-bounded owner lookup after polling cancellation or deadline,
solely to classify release. Disappearance is success, the same PID after the
final lookup is `PORT_RELEASE_TIMEOUT`, and a different or ambiguous owner is
`OCCUPANT_CHANGED`. It MUST never signal a replacement listener discovered
during polling or the final lookup. This final classification lookup may finish
after the three-second protocol window; it does not authorize another signal.

Successful termination returns router state `absent`. It does not start the
router.

#### Scenario: confirmed unchanged occupant

Given a valid token and unchanged complete current-user identity, when the user
confirms termination, then the manager sends exactly one force-termination
signal and verifies that the original identity exits.

#### Scenario: confirmed Windows PID-only occupant

Given an explicitly warned PID-only target and valid token, when the same exact
TCP4 listener still has the same unique unprotected PID immediately before
termination, then the manager attempts one Windows PID termination and verifies
release without starting the router. Windows MAY deny termination, which MUST
return `OCCUPANT_TERMINATION_FAILED` without exposing system details.

#### Scenario: replacement listener during release wait

Given the original process exits and a replacement immediately occupies the
address, when release verification runs, then the replacement receives no
signal and the manager reports the changed or unreleased port state.

### FR-5: Stable errors and sanitization

The manager MUST expose these stable errors:

- `OCCUPANT_NOT_FOUND`
- `OCCUPANT_NOT_OWNED`
- `OCCUPANT_IDENTITY_UNAVAILABLE`
- `OCCUPANT_CHANGED`
- `OCCUPANT_PROTECTED`
- `OCCUPANT_TERMINATION_FAILED`
- `PORT_RELEASE_TIMEOUT`
- `CONFIRMATION_EXPIRED`

Invalid method shapes, including non-empty inspection params or blank
termination tokens, MUST use existing request/parameter errors.

Errors MAY identify a PID already returned by successful inspection. Errors,
logs, diagnostics, state files, and unrelated protocol responses MUST NOT
contain a confirmation token, full occupant executable path, command-line
arguments, environment values, user identifiers, or unrelated socket metadata.

### FR-6: Desktop IPC

Tauri MUST expose named commands for occupant inspection and force termination.
The frontend termination API MUST accept only the opaque confirmation token.
PID and executable path MUST never be accepted as termination arguments.

After successful termination, the Rust orchestration layer MUST request a
router-status refresh and update scheduler state. It MUST NOT invoke router
startup.

### FR-7: Router occupied UI

The Router page MUST retain `occupied` as a distinct state. It MUST inspect the
occupant when entering that state and allow an explicit retry if inspection
fails.

For a complete-identity occupant, the page MUST show its process name and PID.
For a Windows PID-only occupant, it MUST show only PID and listen address plus
an explicit unverified warning. Either valid mode MAY enable a destructive
`Force terminate occupant` action. The regular Stop action MUST remain disabled.

For complete identity, the accessible confirmation dialog MUST show:

- process name;
- PID;
- complete wrapping executable path;
- a warning that termination is immediate and unsaved data may be lost;
- a default-focused Cancel action;
- a destructive Force terminate action.

For Windows PID-only mode, the dialog MUST instead show PID and listen address,
must not infer a process name or path, and MUST state that identity, owner, start
time, and executable were not verified. It MUST state that the manager will
immediately recheck the same port/PID, while PID reuse and unreadable
managed-router state remain residual risks. Cancel MUST remain default-focused,
and the destructive action MUST require a separate explicit selection.

Cancel and Escape before submission MUST close the dialog without a manager
termination request. During submission, duplicate actions and dialog dismissal
MUST be disabled.

On success, the dialog MUST close, status MUST refresh, and the page MUST report
that the port was released. The router MUST remain stopped until the user
selects Start router.

When inspection is blocked, the page MUST explain whether the occupant is
other-user, unverifiable, protected, ambiguous/changed, or temporarily failed.
It MUST not expose a usable destructive action without a valid inspection.

The tray MUST NOT expose force termination. All new copy MUST be available in
English and Simplified Chinese and work at desktop and mobile window widths.

#### Scenario: cancel confirmation

Given a valid inspection is displayed, when the user opens the dialog and
cancels, then no termination request is sent and the occupant remains running.

#### Scenario: stale confirmation

Given polling leaves `unknown_occupant` or the manager reports
`OCCUPANT_CHANGED` or `CONFIRMATION_EXPIRED`, when the UI reconciles the result,
then it discards stale target details and requires a new inspection.

### FR-8: Concurrency and persistence

The manager MUST serialize inspection and termination token operations for the
configured address. Concurrent requests MUST not reuse a token or signal a
process twice.

Status polling MAY continue while the dialog is open. Manager-side validation
remains authoritative if polling and user confirmation race.

Occupant identities and tokens MUST NOT be persisted. High-level unavailable
reasons MAY appear in diagnostics, but tokens and executable paths MUST NOT.

### FR-9: Existing behavior compatibility

`router.stop`, Stop, Quit, parent-death cleanup, external-router reuse, stale
state handling, and CLI-managed router ownership MUST retain their existing
identity and ownership semantics.

Unknown occupants MUST never be terminated automatically. They may be
force-terminated only through the inspection, explicit confirmation, and
revalidation flow defined here.

The desktop MUST continue using exactly its configured listener and MUST NOT
silently choose another port.

## Impact

### Code

- New manager occupant discovery and token service.
- Small additions to process identity APIs without a PID-only signal API.
- Two manager methods, result types, error mappings, and deadlines.
- Tauri and TypeScript IPC additions.
- Router page recovery state, dialog, styles, tests, and localization.

### Build and release

- Manager release targets remain macOS amd64/arm64, Linux amd64/arm64, and
  Windows amd64/arm64 with `CGO_ENABLED=0`.
- No external runtime command dependency is added.

### Documentation

- The live desktop specification and acceptance language must no longer claim
  that unknown occupants are never killed under any circumstance.
- English and Simplified Chinese desktop and troubleshooting documentation must
  describe the explicit recovery action and manual fallback.

## Verification

Required automated verification:

```bash
go test -race ./...
go vet ./...
make test-shell
make test-workflows
test -z "$(gofmt -l .)"
```

From `desktop/`:

```bash
npm run verify
```

All six manager release targets MUST cross-build with `CGO_ENABLED=0`.
Platform-native CI MUST execute listener discovery tests on macOS, Linux, and
Windows. Manual native acceptance MUST prove that cancel sends no request and
confirmed termination releases 19099 without automatically starting the router.
