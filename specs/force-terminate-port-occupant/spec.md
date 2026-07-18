# Force-Terminate Port Occupant Specification

## Motivation

The desktop reports an unknown process listening on `127.0.0.1:19099`, but the
user must leave the application and use operating-system tools to release the
port. The desktop should offer an explicit recovery action when the listener is
owned by the current user and can be identified without trusting a PID alone.

The existing `router.stop` method is not changed. It remains limited to router
processes proven to be owned by the applicable desktop or CLI lifecycle state.

## Scope

This change adds:

- native listener-owner discovery on macOS, Linux, and Windows;
- complete occupant identity and current-user ownership validation;
- a short-lived, single-use confirmation-token protocol;
- immediate force termination after explicit desktop confirmation;
- bounded verification that the target exited and the port was released;
- an occupied-state recovery panel and destructive confirmation dialog;
- stable protocol errors, tests, and paired English/Chinese documentation.

## Exclusions

This change does not:

- terminate an occupant automatically;
- terminate another user's process or request administrator elevation;
- accept a client-supplied PID, executable path, or user identity for signaling;
- add force termination to the tray, setup scripts, router HTTP endpoints, or
  the `mtls-router` binary;
- change normal `router.stop`, Stop, Quit, or external-router ownership rules;
- terminate UDP, remote-host, container-internal, ambiguous wildcard, or
  unverifiable listeners;
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
owners, inaccessible identity, or an unsupported platform.

Platform implementations MUST use native facilities:

- macOS: Darwin process and socket inspection facilities;
- Linux: `/proc/net/tcp*`, `/proc/<pid>/fd`, and process metadata in procfs;
- Windows: IP Helper TCP owner tables and process-token user SID metadata.

#### Scenario: unique current-user listener

Given one current-user process owns the exact loopback TCP listener, when the
manager inspects the occupant, then it returns that process's complete internal
identity and safe presentation fields.

#### Scenario: ambiguous or inaccessible listener

Given listener ownership cannot be reduced to one complete identity, when the
manager inspects the occupant, then it reports identity unavailable and sends
no signal.

### FR-2: Process and user identity

No termination path may trust a PID alone. Process comparison MUST include PID,
operating-system start identity, and normalized executable path. Occupant
comparison MUST additionally include user identity, listen address, protocol
family, and socket ownership identity.

Inspection and termination MUST require the occupant to belong to the current
manager user. The manager MUST NOT request elevation or signal an other-user
process.

The desktop process, serving manager process, and any router proven by current
lifecycle state to be managed MUST be protected from this operation.

#### Scenario: PID or listener reuse

Given any identity field changes after inspection, when termination is
requested, then the manager returns `OCCUPANT_CHANGED` and sends no signal to
the old or replacement process.

#### Scenario: other-user process

Given the listener belongs to another user, when inspection is requested, then
the manager returns `OCCUPANT_NOT_OWNED` and exposes no termination token.

### FR-3: Occupant inspection protocol

The manager MUST add `router.inspect_occupant`. The method accepts no parameters
and has a two-second internal deadline. It is valid only while router discovery
classifies the configured endpoint as `unknown_occupant`.

On success it returns:

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

`process_name` MUST be presentation metadata derived from the executable
basename. The response MUST NOT include command-line arguments, environment
variables, user identifiers, unrelated socket data, or open-file details.

The token MUST be generated with a cryptographically secure random source. It
MUST bind the complete occupant identity and configured address, expire after
30 seconds, be single-use, exist only in manager memory, and become invalid on
manager restart. A new successful inspection for the address MUST invalidate
the previous token.

#### Scenario: inspection supersedes an earlier token

Given two successful inspections occur, when the first token is submitted,
then the manager returns `CONFIRMATION_EXPIRED` and sends no signal.

### FR-4: Force-termination protocol

The manager MUST add `router.force_terminate_occupant`. The method accepts only
`confirmation_token` and has a three-second internal deadline, including a
two-second post-signal wait.

The manager MUST atomically consume the token before performing termination
checks. Consumed, expired, malformed, missing, superseded, or replayed tokens
MUST NOT authorize another attempt.

Before signaling, the manager MUST:

1. verify the unexpired token;
2. confirm discovery remains `unknown_occupant`;
3. rediscover one unique listener for the same address;
4. compare the complete occupant identity with the token-bound identity;
5. recheck current-user ownership and protected-process rules;
6. perform final process identity validation immediately before signaling.

If every check passes, the manager MUST immediately use the platform's
unconditional process termination operation. It MUST NOT first send a graceful
signal.

The manager MUST then wait up to two seconds for the original identity to
disappear and the address to reject new TCP connections. It MUST never signal a
replacement listener discovered during that wait.

Successful termination returns router state `absent`. It does not start the
router.

#### Scenario: confirmed unchanged occupant

Given a valid token and unchanged complete current-user identity, when the user
confirms termination, then the manager sends exactly one force-termination
signal and verifies that the original identity exits.

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

For an inspectable occupant, the page MUST show its process name and PID and
enable a destructive `Force terminate occupant` action. The regular Stop action
MUST remain disabled.

Selecting the destructive action MUST open an accessible confirmation dialog
that shows:

- process name;
- PID;
- complete wrapping executable path;
- a warning that termination is immediate and unsaved data may be lost;
- a default-focused Cancel action;
- a destructive Force terminate action.

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
