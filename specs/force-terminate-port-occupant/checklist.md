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
- [x] Complete mode on macOS, Linux, and Windows obtains a stable current-user
  identity for the candidate process.
- [x] Complete mode combines PID, process start identity, normalized executable,
  user, address, protocol family, and socket ownership identity.
- [ ] Windows alone may fall back to PID-only mode only for one unique owner PID
  on the exact TCP4 endpoint; macOS and Linux remain unchanged.
- [x] Socket-owner ambiguity, wildcard-unprovable, malformed, multiple-owner,
  and unsupported listeners fail closed. Inaccessible complete metadata fails
  closed except for the defined Windows PID-only fallback.
- [x] UDP, non-loopback, remote, and container-internal targets are not treated
  as eligible occupants.
- [x] No runtime dependency on `lsof`, `netstat`, `ps`, PowerShell, or `taskkill`
  is introduced.
- [x] All six manager release targets build with `CGO_ENABLED=0`.

## Ownership and Protection

- [x] Complete identity remains the default authorization on every platform.
- [ ] Only Windows can authorize the explicit warned PID-only exception, and a
  readable other-user SID may enter that degraded flow.
- [x] Incomplete identity receives no signal on macOS/Linux and cannot enter
  complete mode on Windows.
- [x] The desktop application receives no signal from occupant recovery.
- [x] The serving manager receives no signal from occupant recovery.
- [x] A router proven to be managed receives no signal from occupant recovery.
- [ ] PID-only mode protects router PIDs from readable desktop/CLI lifecycle
  state and skips unreadable state as the approved availability tradeoff.
- [ ] PID, start identity, executable, user, address, protocol, or socket-owner
  changes cause `OCCUPANT_CHANGED` and no signal.
- [x] A replacement listener is never signaled using an earlier token.

## Confirmation Tokens

- [x] Tokens use a cryptographically secure random source with at least 32
  random bytes.
- [x] Tokens bind mode/address and either complete identity or the Windows PID.
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
- [x] Inspection returns verification mode, PID, listen address, token, expiry,
  and process name/executable only for complete identity.
- [x] `router.force_terminate_occupant` accepts only `confirmation_token` and
  uses a three-second protocol window; the PID-only final classification lookup
  may extend beyond it as specified below.
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
- [ ] Windows PID-only revalidates the same exact port/PID and protection
  immediately before signaling.
- [ ] PID-only disappearance, PID change, duplicate, wildcard, malformed, or
  ambiguous ownership is refused without a signal.
- [x] An eligible confirmed occupant receives exactly one unconditional force
  termination signal without a preceding graceful signal.
- [x] Complete mode retains its existing at-most-two-second wait for the
  original identity to exit and the endpoint to reject TCP connections.
- [ ] PID-only mode polls exact owner PID under a two-second release deadline,
  then performs one final `PollInterval`-bounded owner lookup after cancellation
  or deadline solely to classify release.
- [ ] PID-only disappearance succeeds; the same PID after the final lookup is
  `PORT_RELEASE_TIMEOUT`; replacement or ambiguity is `OCCUPANT_CHANGED`; no
  replacement receives a signal.
- [x] In complete mode, operating-system signal refusal or a surviving original
  identity reports `OCCUPANT_TERMINATION_FAILED`.
- [ ] In Windows PID-only mode, signal refusal returns internal
  `ErrTerminationFailed` (`OCCUPANT_TERMINATION_FAILED`), while the same PID
  after the final release lookup returns `ErrPortReleaseTimeout`
  (`PORT_RELEASE_TIMEOUT`). PID reuse remains a documented residual risk.

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
- [x] Complete-mode confirmation shows process name, PID, complete wrapping
  path, and immediate data-loss warning.
- [ ] Windows PID-only panel/dialog show only PID/address and explicitly warn
  that identity, owner, start time, and executable are unverified, owner is
  rechecked, and PID reuse/unreadable managed state remain risks.
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
- [ ] Every blocked complete-mode other-user/incomplete path and every
  protected, changed, or ambiguous path has a no-signal assertion; Windows
  readable other-user SID fallback has explicit PID-only coverage.
- [ ] Platform parsing and owner-correlation tests execute on macOS, Linux, and
  Windows CI.
- [x] Windows CI executes native parser/owner-correlation tests and exact
  helper-listener inspection on a Windows runner.
- [x] Native integration tests collectively prove exact listener inspection,
  helper termination, and exact port release.
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

- [x] The live Tauri desktop specification describes complete identity by
  default and the explicit confirmed Windows PID-only exception.
- [x] The live desktop acceptance checklist contains the new unverified items.
- [x] English and Simplified Chinese desktop guides describe both modes, the
  narrow Windows exception, and its approved residual risks.
- [x] English and Simplified Chinese troubleshooting guides describe the
  in-app flow and manual fallback.
- [x] No live documentation incorrectly claims that unknown occupants can never
  be terminated under any circumstance.
