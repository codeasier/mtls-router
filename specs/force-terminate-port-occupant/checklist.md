# Force-Terminate Port Occupant Acceptance Checklist

Check an item only after automated evidence or recorded native verification
demonstrates the behavior.

## Scope and Compatibility

- [x] Unknown occupants are never terminated automatically.
- [x] Normal `router.stop`, Stop, Quit, parent-death cleanup, and tray behavior
  retain their existing ownership rules.
- [x] External-compatible and stale routers cannot be terminated through the
  occupant recovery flow.
- [x] Releasing the port does not automatically start the router.
- [x] The desktop does not silently choose another port.
- [x] No setup-script, router HTTP endpoint, or router CLI kill command is added.

## Listener Discovery

- [x] macOS maps the exact loopback TCP listener to one process using native
  process/socket facilities.
- [x] Linux maps `/proc/net/tcp*` socket inode data to one process FD owner.
- [x] Windows maps the exact endpoint to one owner PID through IP Helper APIs.
- [x] macOS, Linux, and Windows obtain a stable current-user identity for the
  candidate process.
- [x] All platforms combine PID, process start identity, normalized executable,
  user, address, protocol family, and socket ownership identity.
- [x] Ambiguous, wildcard-unprovable, inaccessible, malformed, multiple-owner,
  and unsupported listeners fail closed.
- [x] UDP, non-loopback, remote, and container-internal targets are not treated
  as eligible occupants.
- [x] No runtime dependency on `lsof`, `netstat`, `ps`, PowerShell, or `taskkill`
  is introduced.
- [x] All six manager release targets build with `CGO_ENABLED=0`.

## Ownership and Protection

- [x] A PID alone can never authorize occupant termination.
- [x] A process owned by another user receives no signal.
- [x] An occupant with incomplete identity receives no signal.
- [x] The desktop application receives no signal from occupant recovery.
- [x] The serving manager receives no signal from occupant recovery.
- [x] A router proven to be managed receives no signal from occupant recovery.
- [ ] PID, start identity, executable, user, address, protocol, or socket-owner
  changes cause `OCCUPANT_CHANGED` and no signal.
- [x] A replacement listener is never signaled using an earlier token.

## Confirmation Tokens

- [x] Tokens use a cryptographically secure random source with at least 32
  random bytes.
- [x] Tokens bind the complete occupant identity and configured address.
- [x] Tokens expire after 30 seconds.
- [x] Tokens are single-use and replay sends no signal.
- [x] A new inspection invalidates the previous token.
- [x] Manager restart invalidates all tokens.
- [x] Concurrent requests cannot consume one token twice or signal twice.
- [x] Tokens and complete internal occupant identities are never persisted.

## Manager Protocol

- [x] `router.inspect_occupant` accepts no parameters and has a two-second
  deadline.
- [x] Inspection is allowed only for `unknown_occupant` discovery.
- [x] Inspection returns only PID, process name, executable, listen address,
  token, and expiry.
- [x] `router.force_terminate_occupant` accepts only `confirmation_token` and
  has a three-second deadline.
- [x] Blank, malformed, missing, expired, superseded, consumed, and replayed
  tokens send no signal.
- [x] All eight specified stable occupant error codes are implemented.
- [x] Protocol errors and diagnostics are sanitized and never expose tokens,
  command lines, environment values, user IDs, unrelated socket data, or full
  occupant paths outside the inspection response.

## Termination Behavior

- [x] No signal is sent before explicit confirmation.
- [x] Complete identity, ownership, listener ownership, and protection are
  revalidated immediately before signaling.
- [x] An eligible confirmed occupant receives exactly one unconditional force
  termination signal without a preceding graceful signal.
- [x] The manager waits at most two seconds for the original identity to exit
  and the endpoint to reject TCP connections.
- [x] Target exit with an occupied endpoint reports `PORT_RELEASE_TIMEOUT` or a
  changed occupant without signaling the replacement.
- [x] Operating-system refusal or a surviving original target reports
  `OCCUPANT_TERMINATION_FAILED`.

## Desktop IPC and UI

- [x] Tauri exposes named inspect and force-terminate commands.
- [x] The frontend termination API submits only the opaque token, never PID or
  executable path.
- [x] Entering the occupied state triggers bounded inspection and supports
  explicit retry.
- [x] A destructive action appears only for a valid inspectable occupant.
- [x] The regular Stop action remains disabled for an unknown occupant.
- [x] The tray exposes no force-termination action.
- [x] Blocked states distinguish other-user, unverifiable, protected,
  ambiguous/changed, and temporary inspection failure.
- [x] The confirmation dialog shows process name, PID, complete wrapping path,
  and immediate data-loss warning.
- [x] Cancel has default focus.
- [x] Cancel and Escape before submission send no termination request.
- [x] Duplicate submission and dialog dismissal are disabled during termination.
- [x] Success closes the dialog, refreshes status, reports port release, and
  does not start the router.
- [x] Changed or expired confirmation clears stale target details and requires
  a new inspection.
- [x] English and Simplified Chinese copy remain equivalent.
- [ ] The recovery panel and dialog load correctly at desktop and mobile window
  widths.

## Tests and Release Evidence

- [x] Token expiry, replay, supersession, restart, and concurrency have unit
  tests.
- [ ] Every other-user, incomplete, protected, changed, and ambiguous path has
  a no-signal assertion.
- [ ] Platform parsing and owner-correlation tests execute on macOS, Linux, and
  Windows CI.
- [ ] A native helper-listener integration test proves exact inspection,
  termination, and port release.
- [ ] A replacement-listener integration test proves the replacement receives
  no signal.
- [x] Frontend tests cover blocked states, dialog details, default focus,
  cancel, duplicate prevention, success, expiration, and change races.
- [x] `go test -race ./...` passes.
- [x] `go vet ./...` passes.
- [x] `make test-shell` passes.
- [x] `make test-workflows` passes.
- [x] `test -z "$(gofmt -l .)"` passes.
- [x] `npm run verify` passes from `desktop/`.
- [ ] Native manual acceptance is recorded for macOS, Linux, and Windows.

## Documentation

- [x] The live Tauri desktop specification describes the explicit confirmed
  current-user exception to the former absolute prohibition.
- [x] The live desktop acceptance checklist contains the new unverified items.
- [x] English and Simplified Chinese desktop guides describe the action and its
  risk.
- [x] English and Simplified Chinese troubleshooting guides describe the
  in-app flow and manual fallback.
- [x] No live documentation incorrectly claims that unknown occupants can never
  be terminated under any circumstance.
