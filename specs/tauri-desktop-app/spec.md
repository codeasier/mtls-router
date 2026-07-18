# Tauri Desktop Application Specification

> **Agent model behavior superseded:** `specs/agent-models-config/spec.md`
> supersedes the Agent portions of FR-2, FR-8 through FR-11, and FR-15. In
> particular, the old method list/deadlines, key-after-preview sequence,
> key-free/static preview, whole Claude `env` replacement, static opencode
> models, Codex `custom` provider/auth shape, and old `configured` meaning are
> historical v1 behavior only. They are not protocol-v2 acceptance evidence.
> Unrelated desktop, lifecycle, transaction, packaging, and security clauses in
> this document remain current.

## Change ID

`tauri-desktop-app`

## Status

Proposed. Execution requires explicit approval of this specification package.

## Source Design

This specification formalizes the approved design in
`docs/superpowers/specs/2026-07-12-tauri-desktop-design.md`.

## Motivation

`mtls-router` is cross-platform, but its primary setup experience requires a
user to download an archive, open a terminal, run `setup.sh` or `setup.ps1`,
understand router lifecycle commands, and separately configure local Agents.
The scripts are also maintaining parallel Unix and Windows implementations of
process identity, state, JSON/JSONC/TOML mutation, and backup behavior.

Trusted internal users need a current-user desktop application that starts the
fixed mTLS router without a terminal, remains available in the system tray,
shows the difference between process and upstream health, and safely previews
and writes Claude Code, opencode, and Codex configuration. Existing CLI users
must retain their workflows, and desktop and CLI behavior must use one shared
Go control plane rather than diverging implementations.

## Goals

- Provide a Tauri 2 desktop application for Windows, macOS, and Linux.
- Use React and TypeScript for the desktop frontend.
- Package the fixed-credential `mtls-router` binary with the desktop app.
- Add a cross-platform Go manager used by both Tauri and setup scripts.
- Manage router lifecycle, status, health, version, and logs safely.
- Run in the system tray and enable current-user autostart by default.
- Reuse compatible externally started routers without taking ownership.
- Provide an explicit, confirmed recovery action for an inspectable current-user
  process occupying port 19099.
- Detect, preview, and configure Claude Code, opencode, and Codex.
- Keep API keys out of desktop persistence, process arguments, environment
  variables, logs, state, protocol responses, and diagnostics.
- Preserve existing setup commands and user-visible semantics.
- Produce current-user packages that do not require administrator rights.

## Non-Goals

- Importing certificates, private keys, or CA files at runtime.
- Supporting multiple upstream services or switchable profiles.
- Preventing extraction of credentials embedded in a distributed binary.
- Automatically updating the desktop application or router.
- Installing the bundled router into `PATH`.
- Installing or launching Claude Code, opencode, or Codex.
- Exposing router management endpoints publicly.
- Automatically choosing a new local port when 19099 is occupied.
- Automatically killing an unrecognized process that occupies port 19099, or
  killing one without current-user ownership and complete identity validation.
- Automatically restoring or deleting Agent files, backups, logs, or state
  during uninstall.
- Adding a general-purpose shell, arbitrary file API, or arbitrary process API
  to the frontend.

## Users and Assumptions

- Users are trusted internal users who accept that the shared client private
  key can be extracted from a distributed package.
- Each release is bound to one fixed upstream service/environment.
- The router listens on trusted localhost at `127.0.0.1:19099`.
- The local Agent API base is `http://127.0.0.1:19099/v1` where applicable.
- Credential revocation and rotation are handled by producing and distributing
  a replacement release.
- API keys are entered by users when Agent files are written. Agent files may
  persist those keys as required by the Agent; desktop/manager state and logs
  create no extra persistent copy. User-approved recovery backups are an
  explicit exception: they are sensitive Agent artifacts that may contain an
  old key and MUST be protected like the original Agent file.

## Architecture

### `mtls-router`

`mtls-router` remains the data plane and retains its current flags, environment
variables, embedded mTLS values, startup probe, streaming behavior,
`/version`, `/health`, `-backend`, and `-log` behavior.

The desktop path starts it in foreground mode as a managed child and does not
pass `-backend`. Product changes must not make `mtls-router` responsible for
Agent configuration or desktop lifecycle.

### `mtls-router-manager`

A new cross-platform Go manager is the shared control plane. It owns reusable
packages for:

- Platform user paths and state.
- Router process launch, monitoring, identity validation, and stopping.
- Router management endpoint inspection and external-router classification.
- Router log access and sanitized diagnostics.
- Agent detection, structured preview, backup, atomic write, and rollback.
- A machine-readable, line-delimited JSON protocol.

The manager does not open a network listener. Protocol messages are the only
content written to stdout; diagnostics use stderr or a log file.

The manager runs as `mtls-router-manager serve`. It reads one request per line,
processes requests sequentially, writes one response per request, and exits
cleanly on stdin EOF. Tauri keeps one manager process alive for the desktop
session. Setup wrappers may start `serve`, send one request, close stdin, and
consume the single response.

Every manager method MUST have an internal deadline:

- `manager.info`, `router.status`, and `router.version`: one second.
- `router.logs`: two seconds.
- `diagnostics.collect`, `router.health`, `agent.detect`, and `agent.preview`:
  five seconds.
- `router.stop`: seven seconds, including the five-second graceful wait.
- `router.inspect_occupant`: two seconds.
- `router.force_terminate_occupant`: three seconds, including a two-second
  post-termination wait.
- `router.start`: twenty seconds, including startup probe/readiness.
- `agent.write`: thirty seconds, including rollback.

The Rust client applies a deadline one second longer than the corresponding
manager deadline. Deadline failures use `OPERATION_TIMEOUT`. Router start
timeout MUST stop the verified child before responding. Agent-write timeout
MUST follow the transaction recovery rules in FR-10. A manager that exceeds the
Rust deadline is treated as unresponsive and follows the manager-recovery
policy in FR-3.

### Setup scripts

`setup.sh` and `setup.ps1` remain public wrappers supporting:

```text
router install
router start
router setup
router status
router log
router stop
agent print-config
agent write-config
```

Compatibility aliases remain supported. The scripts retain human-readable
messages and download/install entry-point behavior, but router identity and
Agent mutation logic move to the shared Go manager. Release CLI archives add
the platform manager binary and its checksum entry.

### Tauri desktop

The desktop application uses Tauri 2 with a thin Rust layer and a React +
TypeScript frontend.

Rust owns:

- Sidecar invocation and manager process supervision.
- Typed Tauri commands and validation.
- Window close-to-tray and explicit quit behavior.
- Tray menu and state synchronization.
- Current-user autostart.
- Mapping manager errors into safe frontend responses.
- Enforcing one desktop application instance per user. A second launch MUST
  activate the existing window and MUST NOT start another manager or router.

React owns presentation, navigation, selection, confirmation, localization,
and transient API-key input. It has no arbitrary shell, file, or process
capability.

## Functional Requirements

### FR-1: Fixed sidecar packaging

Each desktop artifact MUST include an architecture-compatible
`mtls-router-manager` and `mtls-router`. Tauri permissions MUST whitelist only
the required packaged sidecar operations. The desktop MUST NOT copy the router
to a PATH directory. The desktop build MUST embed the expected SHA-256 and
target triple for both sidecars. Startup MUST verify the files against those
values before execution and MUST complete a manager version/target handshake.

#### Scenario: sidecars are available

Given a valid desktop installation, when the application starts, then it can
locate and execute the packaged manager and the manager can locate the packaged
router.

#### Scenario: sidecar is missing or invalid

Given a missing, non-executable, wrong-architecture, or integrity-invalid
sidecar, when startup validation runs, then the application reports that
reinstallation is required and does not download a replacement silently.

#### Scenario: second desktop launch

Given the desktop is already running for the user, when it is launched again,
then the existing window is activated and no second manager or router is
created.

### FR-2: Manager protocol (Agent subset superseded by protocol v2)

The manager MUST support line-delimited JSON requests and responses over stdin
and stdout. Every message MUST carry a request ID. Responses MUST use either a
typed result or an error with a stable code. Human-readable logs MUST NOT be
written to protocol stdout.

Required capabilities are:

- `manager.info`
- `diagnostics.collect`
- `router.status`
- `router.start`
- `router.stop`
- `router.health`
- `router.version`
- `router.logs`
- `router.inspect_occupant`
- `router.force_terminate_occupant`
- `agent.detect`
- `agent.preview`
- `agent.write`

The manager MUST process one request at a time. Tauri MAY issue calls from
multiple UI operations, but the Rust manager client MUST serialize them onto
the manager input stream. This avoids concurrent lifecycle or file mutation
operations while retaining request IDs for correlation and diagnostics.

`router.start` MUST include an owner mode:

- `desktop`: the long-running manager starts and supervises a foreground
  router child.
- `cli`: the one-shot manager starts a detached router using the shared
  platform process package and writes CLI ownership state before responding.

Both modes use the same identity validation and state schema rules. The CLI
mode preserves the current user-visible background behavior without parsing
human-readable router output.

`agent.write` MUST carry `api_key` only inside the request line read from
manager stdin. The Rust client MUST construct that request in transient memory,
MUST NOT log it, and MUST clear its application-held key values immediately
after the response or cancellation. Setup wrappers MUST read the key without
echo and send the same stdin request; they MUST NOT export it or place it in a
command argument.

#### Scenario: valid request

Given a valid request, when the manager handles it, then it emits exactly one
correlated JSON response and writes any diagnostic logging outside stdout.

#### Scenario: invalid request

Given malformed JSON, an unknown method, invalid parameters, or an operation
failure, when the manager handles it, then it returns a stable error code and a
sanitized message without terminating unrelated operations.

Malformed JSON that has no recoverable request ID MUST return a response with
`id:null`. All syntactically valid requests MUST contain a non-empty string ID;
otherwise they receive `INVALID_REQUEST` with `id:null`.

### FR-3: Router lifecycle and ownership

The manager MUST start the bundled router in foreground mode, capture logs,
write desktop ownership state atomically, wait for the listener, validate
`/version`, query `/health`, and monitor process exit.

Desktop launch MUST construct a controlled child environment. It MUST remove
all inherited `MTLS_*` variables before starting the router and MUST pass
explicit safe values for localhost listen address, foreground mode, and the
desktop log path. Inherited environment values MUST NOT override the embedded
upstream, TLS policy, timeout, debug mode, backend mode, listen address, or log
path. Standalone CLI mode retains documented router flag/environment behavior
except for the separately approved removal of
`MTLS_ROUTER_OPENAI_API_KEY` from setup Agent writes.

Desktop state MUST record at least PID, listen address, actual executable path,
log path, process start identity, manager/router versions, and
`owner=desktop`. It MUST also record a random desktop session ID, manager PID,
manager process start identity, and manager executable identity.

Stop MUST signal only a desktop-owned process whose PID, process start identity,
and executable identity still match. It MUST attempt graceful shutdown, wait up
to five seconds, revalidate identity, and only then force termination if still
necessary.

For desktop mode, Tauri starts the manager with a random session ID and the
non-sensitive parent PID, parent process start identity, and parent executable
identity. The manager MUST acquire and hold an exclusive cross-platform
desktop ownership lock before starting or reclaiming a router. It MUST monitor
the complete parent identity, not PID alone. If the parent disappears or no
longer matches, the manager attempts to stop its verified desktop-owned router,
releases the lock, and exits.

If the manager exits while Tauri remains alive, Rust MUST disable lifecycle
commands and perform at most one automatic manager restart. The replacement
manager may reclaim the router only after acquiring the exclusive lock,
confirming the previous manager process identity is absent, and matching the
recorded session ID plus complete router identity. It then atomically records
its new manager identity under the same desktop session. If restart or reclaim
fails, the UI enters a manager-failed state and requires application restart;
it MUST NOT start another router or signal the existing process. The same
exclusive-lock and identity checks apply after a full desktop crash/relaunch.

#### Scenario: normal start and stop

Given port 19099 is free and the upstream is reachable, when the desktop starts
the router, then the UI reaches “running and healthy,” records desktop
ownership, and explicit tray quit stops the same verified process.

#### Scenario: stale state

Given a state file whose process identity does not match the current PID, when
status or stop runs, then status reports stale and stop sends no signal.

#### Scenario: router exits unexpectedly

Given a desktop-owned router that exits, when the manager observes the exit,
then the UI reports start failure or unexpected exit with sanitized recent
logs and does not enter an unlimited automatic restart loop.

### FR-4: Existing-router discovery

When port 19099 is occupied, the manager MUST query `/version` and `/health`
and correlate available process and CLI state. It MUST classify the result as
compatible external router, degraded compatible router, or unknown process.

Endpoint shape alone is not trusted. A router is reusable only when all of the
following match:

- A complete CLI setup state exists.
- PID, process start identity, and executable identity match that state.
- `/version` reports the expected management protocol version.
- `/version` reports the expected deployment ID.

`deployment_id` is a non-sensitive build-time identifier for the fixed service
environment. `management_protocol_version` is a build-time API compatibility
identifier. Both MUST be added to router version metadata, release builds, and
CLI state without exposing upstream URLs or credential material. A manually
started router without trusted setup state is `unknown_process` even if its
endpoints look compatible.

The management protocol version MUST be a non-empty code-owned constant shared
by router and manager builds. Production release input MUST provide a non-empty
deployment ID other than `dev` or `unknown`. The same expected deployment ID
and protocol version MUST be embedded in router, manager, and desktop. Router
`/version` and `manager.info` MUST report them. Release preflight MUST fail when
any production value is absent/defaulted or when the three artifacts disagree.
Local development MAY use `dev`, but external-router reuse MUST be disabled for
default development identities.

#### Scenario: compatible CLI router

Given a compatible router started by the setup scripts, when the desktop
starts, then it displays “external router running,” uses it for health and
Agent guidance, and does not stop it when the desktop quits.

#### Scenario: unknown occupant

Given an unrecognized service on port 19099, when the desktop starts, then it
reports a port conflict, does not terminate the service automatically, and does
not silently choose another port.

An unknown occupant MAY be force-terminated only when native inspection proves
one complete process identity owned by the current manager user. The Router
page MUST show the process name and PID and require a destructive confirmation
dialog that also shows the complete executable path and warns that termination
is immediate and may lose unsaved data. The manager MUST consume a short-lived,
single-use confirmation token, revalidate the full listener and process
identity immediately before signaling, and use unconditional process
termination without first sending a graceful signal.

This exception MUST NOT request elevation or apply to another user's,
ambiguous, changed, protected, or unverifiable process. It MUST NOT alter the
regular Stop, Quit, parent-death, external-router, or tray ownership rules. The
tray MUST NOT expose force termination, and releasing the port MUST NOT start
the router automatically. If inspection or termination is unavailable, the UI
MUST retain manual operating-system-tool recovery guidance.

#### Scenario: explicitly confirmed current-user occupant

Given native inspection identifies one complete current-user occupant, when the
user verifies its process name, PID, and complete executable path, accepts the
immediate data-loss warning, and explicitly confirms force termination, then
the manager revalidates and terminates only that unchanged identity, verifies
port release, and leaves the router stopped.

### FR-5: Router status and health UI

The Router page MUST show process state, upstream health, local address,
ownership, and desktop/manager/router versions. Process availability and
upstream health MUST be distinct.

The observable UI states are:

- Not started.
- Starting.
- Running and healthy.
- Running with unavailable upstream.
- External router running.
- Port occupied.
- Start failed.
- Stopping.

The Rust layer MUST schedule process-only `router.status` immediately after
lifecycle actions, every two seconds while the window is visible, and every ten
seconds while hidden in the tray. It MUST schedule a fresh `router.health`
after a router becomes available, every ten seconds while visible, every thirty
seconds while hidden, and on explicit retry. Health results older than thirty
seconds MUST be shown as stale rather than healthy.

These intervals govern scheduling, not guaranteed completion. A tick is
skipped while another manager request is in flight, and due status/health work
is requested immediately when that operation completes. Only the latest
request generation may update UI and tray state; stale responses are discarded.
This polling is the MVP mechanism for detecting unexpected exits and health
changes; the manager protocol emits no unsolicited events.

#### Scenario: health degrades after startup

Given a running router whose `/health` response becomes degraded, when the next
health refresh completes, then the process remains running and the UI shows an
upstream warning with a retry action.

### FR-6: First-launch behavior

On first launch the desktop MUST validate sidecars, inspect port 19099, reuse a
compatible router or start the bundled router, poll `/version` and `/health`,
and open the Router page. It MUST NOT modify any Agent file automatically.

#### Scenario: successful first launch

Given a clean supported user account, when the app starts for the first time,
then the router becomes usable without opening a terminal and Agent
configuration remains unchanged until explicit confirmation.

### FR-7: Tray and autostart

Closing the main window MUST hide it without stopping the router. The tray menu
MUST provide open window, start router, stop router, view logs, and quit. Tray
status MUST represent normal, warning, and error router states.

Current-user autostart MUST be enabled by default and configurable from
Settings without administrator privileges. Autostart launches the desktop,
which then applies normal router discovery/ownership rules.

#### Scenario: close and reopen

Given a desktop-owned running router, when the user closes and reopens the
window from the tray, then the router PID remains unchanged.

#### Scenario: quit with external router

Given an external compatible router, when the user chooses tray quit, then the
desktop exits and the external router remains running.

### FR-8: Agent detection (configured semantics superseded)

The manager MUST detect Claude Code, opencode, and Codex using current setup
semantics, including environment-specific configuration locations and Codex
desktop detection through its home directory. Detection MUST return path,
existence, format, writability, and configured/invalid state without returning
stored API-key values.

#### Scenario: Agent is absent

Given an Agent is not detected, when detection completes, then the UI marks it
absent and does not select it for writing by default.

#### Scenario: invalid configuration

Given a detected Agent with invalid JSON, JSONC, or TOML for the required
operation, when detection or preview runs, then the UI reports the invalid
target and no file is modified.

### FR-9: Agent preview (request/config content superseded)

Preview MUST read the current target files and return a structured change set
containing affected paths, create/replace/preserve operations, backup plan,
format migration warnings, and a revision token derived from the current file
state. Preview MUST NOT write files or expose API-key values.

#### Scenario: canonical opencode JSONC

Given `OPENCODE_CONFIG` is empty and opencode uses the canonical
`~/.config/opencode/opencode.jsonc`, when preview runs, then it explicitly
shows migration to sibling `opencode.json`, warns that comments/formatting are
not preserved, and identifies the JSONC backup that will be created. An
existing sibling `opencode.json` is a migration collision and is not replaced.

#### Scenario: explicit opencode JSONC

Given `OPENCODE_CONFIG` explicitly names a `.jsonc` file, when preview and
write run, then the exact configured path is normalized to strict JSON in
place, an existing file is backed up, and comments/formatting are not
preserved. A missing path is created there without a backup, and any sibling
`opencode.json` remains unrelated and byte-identical.

#### Scenario: Codex preview

Given Codex is selected, when preview runs, then it separately identifies
changes to `config.toml` and `auth.json` and states that `auth.json` will contain
the supplied API key without displaying it.

### FR-10: Agent write transaction (key timing/request shape superseded)

The user MUST explicitly select Agents, approve a preview, and enter an API key
before write. The key MUST travel through a controlled stdin/IPC payload and
MUST NOT be passed as a command argument or environment variable.

Frontend cancellation/completion MUST clear application references to the
transient key immediately. Rust MUST use best-effort zeroization for buffers it
controls after request serialization and completion. This is logical-state and
best-effort buffer clearing, not a guarantee of forensic erasure from the
JavaScript runtime or operating system memory.

Before writing, the manager MUST re-read each target and compare it with the
preview revision. A mismatch MUST reject the write and require a new preview.
Existing files MUST be backed up before mutation. Writes MUST use same-directory
temporary files and atomic replacement where supported.

Backups are sensitive recovery artifacts and may contain a previous API key.
They MUST remain beside the original file, inherit or tighten its user-only
permissions, be identified as sensitive in preview/results/documentation, and
never have their content copied into logs or diagnostics.

One user-confirmed multi-Agent write is one transaction: if any required file
write fails, files changed by that transaction MUST be restored from their
pre-write state. Diagnostic backups MUST be retained. Results MUST identify
success/failure and changed/backup paths per Agent without returning the key.

Before the first target replacement, the manager MUST atomically persist and
sync a user-private transaction journal in its state directory. The journal
contains transaction ID, target paths, pre-write revisions, backup paths, and
per-file replacement progress, but no API key or file contents. Progress is
atomically updated after each replacement. On `agent.write` deadline, the
manager MUST roll back every recorded replacement before responding. On manager
startup, journal recovery MUST complete before any request is accepted: an
incomplete transaction is rolled back from its backups, the resulting files
are validated, and only then is the journal removed. If recovery cannot prove
restoration, all Agent writes remain disabled and `ROLLBACK_FAILED` is returned.
Rust watchdog recovery MUST NOT permit a replacement manager to process new
Agent operations until journal recovery finishes.

#### Scenario: successful write

Given unchanged valid files and a supplied key, when write runs, then selected
Agents receive current mtls-router configuration, unrelated settings are
preserved, and backups are reported.

#### Scenario: preview is stale

Given a target changes after preview, when write runs, then the manager rejects
the transaction before mutation and requests a new preview.

#### Scenario: partial operation fails

Given an earlier file was replaced and a later required file fails, when the
transaction ends, then the earlier file is restored and all Agents are reported
accurately.

### FR-11: Existing Agent semantics (superseded by Agent-native v2 rendering)

The manager MUST preserve these current behaviors:

- Claude Code preserves non-`env` top-level fields and replaces `env` with the
  approved mtls-router environment block.
- opencode preserves other root fields and providers and writes the
  `mtls-router` provider and current model definitions.
- With no explicit `OPENCODE_CONFIG`, canonical JSONC migration refuses to
  overwrite an existing sibling `opencode.json` target.
- With an explicit `.jsonc` `OPENCODE_CONFIG`, the exact path is normalized in
  place as strict JSON; existing content is backed up and a missing path is
  created without reading or changing a sibling `opencode.json`.
- Codex preserves unrelated root keys and sections while replacing managed root
  model keys and `[model_providers.custom]`.
- Codex writes `auth.json` with only `OPENAI_API_KEY` when a key is supplied.
- Existing target and auth files are backed up before change.

### FR-12: Logs and diagnostics

The Logs page MUST display a bounded recent log view and provide open-log-
location and copy-diagnostic-summary actions. Diagnostic output and GUI logs
MUST redact API keys, client certificates, private keys, CA contents, and
sensitive parameters.

Raw router logs remain in the current user's application data directory for
local diagnosis. No management endpoint may be exposed beyond the configured
localhost listener.

Before desktop release, router and manager logging MUST be audited and tested
at write time. Request query strings, authorization/authentication headers,
request/response bodies, process environments, Agent write payloads, and
key-shaped test values MUST NOT be written to raw logs. Upstream errors MUST
remain sanitized according to existing proxy policy.

### FR-13: Settings and localization

Settings MUST include:

- Current-user autostart, default enabled.
- Interface language, default Chinese, with an English switching path.
- Desktop, manager, and router versions.
- Application data and log locations.

Settings MUST NOT expose upstream URL, certificate import, router replacement,
automatic update, or PATH installation controls.

### FR-14: Uninstall behavior

Windows uninstall integration MUST remove the desktop application's
current-user autostart registration. Because deleting a macOS application from
a DMG or deleting an AppImage cannot reliably execute an uninstall hook, the
macOS and Linux Settings pages MUST provide a **Prepare for uninstall** action
that removes the current-user autostart entry and exits before the user deletes
the application. macOS and Linux documentation MUST state that sequence
explicitly.

No platform uninstall path may restore, delete, or rewrite Agent
configurations, backups, logs, or diagnostic state.

### FR-15: CLI compatibility (Agent automation subset superseded)

The existing setup commands, compatibility aliases, secure download behavior,
state safety, and documented outcomes MUST remain available after manager
extraction. Setup-managed status and stop MUST retain the current PID-reuse and
executable-identity protections.

`MTLS_ROUTER_OPENAI_API_KEY` is intentionally removed as a security-breaking
change. Interactive setup MUST continue to read the key without echo and send
it only in the manager stdin request. Automation MUST invoke
`mtls-router-manager serve` and provide `agent.write` through stdin. English
and Chinese migration documentation MUST describe this replacement.

English and Chinese README/build documentation MUST remain aligned for all
user-visible changes.

### FR-16: CLI manager installation and integrity

Platform manager assets MUST be named `mtls-router-manager-${GOOS}-${GOARCH}`
with `.exe` on Windows. CLI installation places the router and manager together
in `MTLS_ROUTER_INSTALL_DIR` as `mtls-router`/`mtls-router.exe` and
`mtls-router-manager`/`mtls-router-manager.exe`.

`router install` and `router setup` MUST install or update both binaries as one
staged transaction. If any sibling packaged payload exists, both exact platform
payloads and their unique valid `SHA256SUMS` entries are required; a missing or
invalid member fails closed without network fallback. Network installation
downloads both binaries and one manifest into a temporary directory and
verifies both before installation begins.

The installer MUST write and sync a user-private pending transaction marker,
preserve the previous committed pair, replace the two fixed paths one at a time,
verify the installed hashes, then atomically commit the new receipt and remove
the pending marker. No code may claim that two path replacements are jointly
atomic. Before any installed manager or router is executed, setup MUST reconcile
a pending transaction: complete commit only when both installed hashes match
the pending generation, otherwise restore and verify the previous committed
pair. If neither generation can be proven complete, setup fails closed and
executes neither binary. Crash-point tests MUST cover failure between each path
replacement and before receipt commit.

Installation MUST write a user-private receipt containing installed paths,
hashes, versions, deployment ID, and management protocol version. Script
manager discovery uses a checksum-verified sibling manager first, then an
installed manager whose hash matches the receipt. Agent commands do not
implicitly download a missing manager; they fail with guidance to run
`router install`. A missing, invalid, or mismatched member never results in
executing the other member alone.

## Error Codes

The Phase 1 protocol contract MUST define and test stable codes covering at
least:

- `INVALID_REQUEST`
- `UNKNOWN_METHOD`
- `INVALID_PARAMS`
- `SIDECAR_MISSING`
- `SIDECAR_INVALID`
- `ROUTER_NOT_FOUND`
- `ROUTER_ALREADY_RUNNING`
- `ROUTER_START_FAILED`
- `ROUTER_NOT_READY`
- `ROUTER_DEGRADED`
- `ROUTER_NOT_OWNED`
- `ROUTER_STATE_STALE`
- `PORT_OCCUPIED`
- `OCCUPANT_NOT_FOUND`
- `OCCUPANT_NOT_OWNED`
- `OCCUPANT_IDENTITY_UNAVAILABLE`
- `OCCUPANT_CHANGED`
- `OCCUPANT_PROTECTED`
- `OCCUPANT_TERMINATION_FAILED`
- `PORT_RELEASE_TIMEOUT`
- `CONFIRMATION_EXPIRED`
- `AGENT_NOT_FOUND`
- `CONFIG_INVALID`
- `CONFIG_NOT_WRITABLE`
- `PREVIEW_STALE`
- `BACKUP_FAILED`
- `WRITE_FAILED`
- `ROLLBACK_FAILED`
- `OPERATION_TIMEOUT`

Messages may be localized in the UI. Branching logic MUST use codes, not
human-readable strings.

## Data and Permissions

- CLI state remains in the existing `~/.mtls-router` location or Windows
  equivalent.
- Desktop state and logs use the Tauri application user-data directory.
- State and log files containing process metadata use restrictive user-only
  permissions where the platform supports them.
- Agent backups remain beside their original files using collision-resistant
  names and user-private permissions. They are sensitive recovery artifacts
  and may contain an old Agent API key.
- No API key, certificate, private key, or CA content is stored in manager or
  desktop state.
- Frontend Tauri capabilities expose only named application commands.

## Release Impact

### Repository

- Add Go manager commands and internal management packages.
- Refactor setup scripts to consume the manager contract.
- Add a Tauri 2 Rust application and React + TypeScript frontend.
- Add Node/Rust tooling and lockfiles for reproducible desktop builds.
- Expand CI for Go, shell, frontend, Rust, and package checks.
- Expand release packaging to include manager binaries and desktop artifacts.
- Update English and Chinese documentation and changelogs.

### Release matrix

- Windows amd64 and arm64.
- macOS amd64 and arm64.
- Linux amd64 and arm64.

Signing and macOS notarization MUST be integrated when release credentials are
available. Their absence may block production distribution but MUST NOT cause
the workflow to claim a signed/notarized artifact.

## Dependencies

- Go 1.26 as required by the repository.
- Tauri 2 and its shell/sidecar, tray, and autostart capabilities.
- Rust toolchain supported by the chosen Tauri 2 version.
- Node.js toolchain for React + TypeScript and frontend tests.
- Platform WebView and packaging prerequisites.
- Existing release secrets for embedded mTLS material.
- Platform signing/notarization credentials for production packages.

Library versions and APIs MUST be selected from current upstream documentation
during implementation and pinned in lockfiles.

## Verification Commands

The final implementation MUST provide or preserve commands equivalent to:

```bash
test -z "$(gofmt -l .)"
go test ./...
go vet ./...
make test-shell
```

The desktop workspace MUST define reproducible commands for:

```text
frontend format/lint or equivalent static check
frontend unit tests
frontend production build
Rust format check
Rust tests
Tauri bundle build per supported platform
```

Exact desktop command names are fixed when the workspace manifest is added and
then documented in `tasks.md`, CI, and maintainer documentation.
