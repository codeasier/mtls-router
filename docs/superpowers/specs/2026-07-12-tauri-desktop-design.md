# mtls-router Tauri Desktop Design

## 1. Summary

`mtls-router` currently provides a cross-platform local reverse proxy and
shell-based setup workflows. This design adds a Tauri desktop application for
trusted internal users. The desktop application packages a prebuilt,
credential-embedded `mtls-router` binary and provides a simple GUI for router
lifecycle management and Agent configuration.

The desktop application is Chinese-first, keeps an English switching path,
runs in the current user's context, starts with the operating system by
default, and remains in the system tray. Closing the window hides it without
stopping the router. Quitting from the tray stops only a router process owned
by that desktop instance.

The first release does not support runtime certificate import, automatic
updates, PATH installation, or automatic cleanup during uninstall.

## 2. Decisions and Constraints

### 2.1 Credential model

Each desktop release is bound to one fixed upstream service/environment. The
packaged `mtls-router` binary continues to receive the client certificate,
private key, upstream CA, and upstream URL through Go linker variables at
build time.

The desktop package is intended for trusted internal users and accepts the
risk that an embedded private key can be extracted. Credential rotation and
release replacement are the mitigation for a leaked shared credential.

The GUI does not display or edit certificate material or the fixed upstream
URL.

### 2.2 Distribution and updates

The desktop package includes both the manager and router sidecars. The router
is not copied to PATH and is not installed as a separate CLI by the desktop
application. The existing `setup.sh` and `setup.ps1` workflows remain
available for standalone CLI users.

The MVP has no automatic update mechanism. The desktop application, manager,
and router are released as one versioned package and upgraded by reinstalling
the complete package.

Target packaging is current-user installation without administrator rights:

- Windows: current-user installer.
- macOS: user-installable application/DMG package.
- Linux: an unprivileged AppImage as the initial target.

### 2.3 Scope

The first desktop release supports:

- Router start, stop, status, health, version, and logs.
- System tray residency.
- Default-on current-user autostart.
- Safe reuse of an already running compatible router.
- Claude Code, opencode, and Codex detection and configuration.
- Configuration preview, confirmation, backups, and transactional writes.
- Temporary API key input without desktop-side persistence.
- Chinese-first UI with an English switching path.

It does not support:

- Runtime certificate, private-key, or CA import.
- Multiple upstream profiles.
- Automatic application or router updates.
- PATH installation of the bundled router.
- Automatic restoration or deletion of Agent configuration during uninstall.

## 3. Architecture

### 3.1 Component responsibilities

#### `mtls-router`

The existing Go binary remains the data plane. Its responsibilities do not
expand:

- Accept local HTTP and proxy to the fixed HTTPS/mTLS upstream.
- Preserve request and SSE streaming behavior.
- Keep `/version` and `/health` management endpoints.
- Keep current runtime flags, environment variables, and foreground behavior.
- Keep `-backend` and `-log` for standalone CLI compatibility.

The desktop application starts this binary as a foreground child process and
does not use `-backend`.

#### `mtls-router-manager`

Add a cross-platform Go management binary as the shared control plane. It is
responsible for:

- Validating bundled binary identity and executable availability.
- Starting, monitoring, stopping, and inspecting router processes.
- Reading router logs and management endpoints.
- Checking port occupancy and identifying external routers.
- Persisting and validating process identity state.
- Detecting Claude Code, opencode, and Codex.
- Producing Agent configuration previews.
- Backing up and atomically writing Agent configuration.
- Rolling back files changed by a failed multi-file write.
- Returning machine-readable JSON and stable error codes.

The manager must not expose an additional network listener.

#### `setup.sh` and `setup.ps1`

The existing scripts remain public CLI entry points. They preserve the
current commands and user-visible behavior while becoming thin wrappers over
the shared Go management implementation. They continue to provide
human-readable output, argument compatibility, and platform-specific shell
entry points, but must not maintain a second implementation of process
identity or Agent file mutation logic.

Existing commands remain supported:

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

The scripts and manager must share contract tests for status, lifecycle, Agent
preview, and Agent write behavior.

#### Tauri application

The Tauri/Rust layer is a thin desktop shell. It is responsible for:

- Window lifecycle and close-to-tray behavior.
- System tray icon, menu, and status presentation.
- Current-user autostart integration.
- Sidecar process invocation and lifecycle supervision.
- Typed IPC commands exposed to the frontend.
- Mapping manager errors to UI-safe messages.

The frontend is responsible for presentation, form validation, selection,
confirmation, and localization. It cannot directly execute shell commands,
read arbitrary files, terminate processes, or invoke arbitrary sidecars.

### 3.2 Package contents

Each platform package contains:

```text
Tauri GUI
mtls-router-manager sidecar
mtls-router sidecar
```

Tauri sidecar configuration and permissions must whitelist only the packaged
manager and router paths. The frontend invokes Rust commands; it does not
invoke sidecars directly.

### 3.3 Process ownership and state

The CLI and desktop application use separate state locations:

- CLI: existing `~/.mtls-router/setup-state.json` or the Windows equivalent.
- Desktop: application user-data directory with a desktop-specific state
  file.

Both implementations use `/version`, `/health`, executable path, process
start identity, and PID checks when available. A desktop state record includes
at least:

- PID.
- Listen address.
- Actual executable path.
- Router log path.
- Process start identity.
- Manager/router version.
- Owner marker set to `desktop`.

The desktop may reuse a compatible CLI-owned router, but must mark it as
external and must not stop it on desktop exit. The desktop stops only a
process it started and whose complete identity still matches the recorded
state.

## 4. User Experience

### 4.1 Main navigation

The initial interface has four simple sections:

- **Router**: process state, upstream health, local URL, version, and start or
  stop actions.
- **Agent configuration**: Agent detection, selection, preview, API key input,
  and write results.
- **Logs**: recent router logs, diagnostic summary, and open-log-location
  action.
- **Settings**: autostart, language, versions, and data locations.

The UI presents “router process is running” separately from “upstream is
healthy.” A degraded upstream must not appear as a fully working state.

### 4.2 First launch

On first launch the application:

1. Verifies that manager and router sidecars are present and executable.
2. Checks whether `127.0.0.1:19099` is already occupied.
3. Reuses a compatible external router when possible.
4. Starts the bundled router when no compatible router is running.
5. Polls `/version` and `/health` and reports progress.
6. Opens the Router page.
7. Offers an explicit next step to configure Agents.

First launch never writes Agent configuration automatically.

Router UI states are:

- Not started.
- Starting.
- Running and healthy.
- Running with unavailable upstream.
- External router running.
- Port occupied.
- Start failed.
- Stopping.

### 4.3 Agent configuration flow

The Agent page displays Claude Code, opencode, and Codex cards with:

- Detection result.
- Configuration file path.
- Existence, format, and writability status.
- Configured/not configured/invalid status.
- Selection checkbox.

The user selects one or more Agents and chooses **Preview changes**. The
preview shows affected paths, fields to add or replace, preserved content,
backup behavior, and format migration warnings. It never displays API keys in
plain text.

After preview confirmation, the user enters the API key into a password-style
field. The key is sent through a controlled manager input channel, is not
persisted by the desktop application, and is not placed in command arguments,
environment variables, logs, state files, or diagnostic output.

The write result shows each Agent independently, including changed paths and
backup paths. A failure in one Agent must not silently report success for that
Agent or mutate its file.

Existing semantics remain:

- Claude Code keeps other top-level fields and replaces `.env`.
- opencode keeps other providers and handles JSONC migration only with an
  explicit warning and backup.
- Codex updates `config.toml` and, when required by the existing flow,
  `auth.json`.
- Existing files are backed up before mutation.
- Unrelated configuration is preserved.
- Historical backups are never automatically restored.

### 4.4 Tray and autostart

The application starts in the tray with autostart enabled by default. Closing
the main window hides it. The tray menu provides:

- Open window.
- Start router.
- Stop router.
- View logs.
- Quit.

Quit stops only a desktop-owned router. It does not stop an external CLI-owned
router. Autostart changes apply to the current user and do not require
administrator privileges.

### 4.5 Uninstall

Uninstall removes the desktop application and its autostart registration. It
does not restore, delete, or rewrite Agent configuration, backups, router
logs, or diagnostic state.

## 5. Manager Protocol and Data Flow

### 5.1 Transport

The manager uses a line-delimited JSON request/response protocol over standard
input and standard output. It is not an HTTP server. Every request and
response carries an ID:

```json
{"id":"42","method":"router.status","params":{}}
```

```json
{"id":"42","ok":true,"result":{"state":"running","owner":"desktop"}}
```

Manager protocol output must be the only content on stdout. Human-readable
logs go to stderr or the application log file.

Errors use stable machine-readable codes:

```json
{
  "id": "42",
  "ok": false,
  "error": {
    "code": "PORT_OCCUPIED",
    "message": "本地端口 19099 已被其他程序占用",
    "details": {}
  }
}
```

The exact method names may be finalized during implementation, but the
capabilities must cover:

- Router status, start, stop, health, version, and recent logs.
- Agent detection, preview, and write.
- Version and diagnostics.

### 5.2 Router startup

The manager:

1. Validates the bundled executable path.
2. Starts `mtls-router` in foreground mode as a child process.
3. Captures its output without exposing credentials.
4. Writes desktop ownership state atomically.
5. Waits for the listener.
6. Requests `/version` to confirm router identity.
7. Requests `/health` to determine upstream health.
8. Reports state changes to Tauri.
9. Monitors process exit and log streams.

The manager distinguishes process-not-started, listener-not-ready,
version-check-failed, healthy, degraded, and unexpected-exit states.

### 5.3 External router discovery

When port 19099 is occupied, the manager:

1. Connects to the local address.
2. Requests `GET /version`.
3. Validates the response shape and version fields.
4. Requests `GET /health`.
5. Correlates process identity and available CLI state when possible.

The result is classified as `external_router`, `router_degraded`, or
`unknown_process`. An unknown process is never terminated and the desktop
does not silently switch ports.

### 5.4 Agent operations

Agent management has four logical stages:

1. `detect`: returns known Agent paths, existence, format, and writability;
   never returns API keys.
2. `preview`: reads files and returns a structured change set without
   writing.
3. `write`: receives a temporary key, revalidates the target files, creates
   backups, and performs atomic writes.
4. `result`: returns per-Agent success/failure, changed paths, and backup paths
   without returning the key.

If a file changes between preview and write, the manager rejects the write and
requires a new preview. A multi-file write failure rolls back files changed
by that operation while retaining backups for diagnosis.

## 6. Security and Failure Handling

### 6.1 Sensitive data

The desktop application and manager must not persist or expose:

- API keys outside the Agent files that require them.
- Client certificates, private keys, or upstream CA contents.
- Sensitive command arguments.

The API key must not appear in process arguments, environment variables,
manager state, logs, Tauri persistent state, error details, or copied
diagnostics. The Agent itself may persist the key in its normal configuration
files; the desktop application does not create an additional copy.

### 6.2 Fail-closed behavior

- Missing, non-executable, or invalid sidecars: report reinstall required;
  never download silently.
- Router start failure: show exit code and sanitized recent logs; do not loop
  indefinitely.
- Unknown port occupant: do not signal or kill it.
- External router: reuse without ownership.
- Stale identity: retain state and report stale; do not signal.
- Degraded `/health`: keep the router running and offer retry.
- Invalid/unwritable Agent file: leave that file unchanged.
- Preview/write mismatch: reject rather than overwrite silently.
- Tauri or manager crash: only clean up a process after complete identity
  revalidation; an uncertain process may be left running for manual diagnosis.

Logs shown in the GUI and copied diagnostic summaries must redact API keys,
certificate material, private keys, and sensitive parameters. Raw router logs
remain in the user data directory for local diagnosis.

## 7. Testing and Verification

### 7.1 Go manager

Unit and contract tests cover:

- Line protocol framing, request IDs, stable errors, and stdout purity.
- Platform paths, permissions, atomic state writes, and log handling.
- PID, process-start identity, executable identity, and stale state.
- External router, unknown port occupants, repeated start, stop timeout, and
  unexpected exit.
- Claude JSON, opencode JSON/JSONC, Codex TOML/auth.json preview, backup,
  write, and rollback behavior.
- API key absence from logs, state, responses, and diagnostics.

Existing fake-router and shell tests remain. They are extended to verify that
the scripts call the shared manager contract rather than maintaining separate
mutation behavior.

### 7.2 Tauri application

Tests cover:

- IPC parameter validation and error mapping.
- First launch and sidecar failure states.
- Tray hide, tray actions, quit ownership behavior, and external Router reuse.
- Current-user autostart enable/disable.
- Router status-to-UI mapping.
- Agent selection, preview confirmation, API key handoff, and result display.

### 7.3 Release verification

Build and launch checks cover:

- Windows x64 and ARM64.
- macOS Intel and Apple Silicon.
- Linux x64 and ARM64.
- Current-user installation without administrator rights.
- Matching sidecar architecture and executable permissions.
- Package contents and checksum verification.
- Platform signing, macOS notarization, and launch smoke tests where release
  credentials are available.

## 8. Implementation Phases

### Phase 1: Shared manager

Extract lifecycle, identity, state, Agent detection, preview, backup, and
write logic into Go packages and expose the machine-readable manager
protocol. Convert `setup.sh` and `setup.ps1` into wrappers while preserving
their commands, compatibility aliases, output expectations, and existing
documentation.

### Phase 2: Tauri shell

Add the Tauri application, sidecar packaging, typed IPC, Router lifecycle,
status, health, logs, system tray, and default current-user autostart.

### Phase 3: Agent UI

Add Agent detection, selection, structured preview, temporary API key input,
configuration writes, backups, migration warnings, and rollback result
handling.

### Phase 4: Productization

Complete Chinese-first localization with an English switching path, build
Windows/macOS/Linux current-user packages, add signing/notarization release
steps, expand CI to the desktop matrix, and update English and Chinese
documentation.

Each phase must have independently executable tests and must not require the
full GUI before validating the shared control plane.

## 9. MVP Acceptance Criteria

- A new user can install and start the router without opening a terminal.
- First launch never changes Agent files automatically.
- Claude Code, opencode, and Codex can be previewed and configured explicitly.
- Configuration writes create backups and do not destroy the original on
  failure.
- Closing the window does not stop the router.
- Tray quit stops only a router owned by the desktop instance.
- An existing compatible CLI router can be safely reused.
- An unknown process occupying port 19099 is never killed.
- Autostart is enabled by default and works without administrator privileges.
- The desktop application does not persist API keys or expose embedded mTLS
  material.
- Uninstall retains Agent configuration, backups, logs, and state.
- The Chinese interface is usable and has an English switching path.
