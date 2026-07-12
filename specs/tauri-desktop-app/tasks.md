# Tauri Desktop Application Tasks

Tasks are ordered by dependency. A task is complete only when its code,
tests, and required documentation pass the listed verification. Execution
must not begin until this specification package is approved.

## Phase 1: Shared Go Control Plane

- [x] **1.1 Define the manager protocol contract**
  - Add request, response, result, and error-code types.
  - Implement the specified `manager.info`, `diagnostics.collect`,
    `router.*`, and `agent.*` method names.
  - Define line framing, malformed-request recovery, request correlation, and
    stdout/stderr rules for `mtls-router-manager serve`.
  - Process requests sequentially and exit cleanly on stdin EOF.
  - Use `id:null` for malformed requests without a recoverable non-empty string
    request ID.
  - Enforce the complete deadline table from `spec.md` for every method and
    return `OPERATION_TIMEOUT` with method-specific cleanup.
  - Add protocol tests for valid requests, malformed JSON, unknown methods,
    invalid parameters, stable errors, and stdout purity.
  - Verification: `go test ./...`.

- [x] **1.2 Extract platform user paths and atomic state storage**
  - Add shared path resolution for CLI and desktop state/log directories.
  - Add atomic JSON state read/write and restrictive permissions.
  - Keep the existing setup state schema readable or provide an explicit
    migration with tests if the schema must change.
  - Add corrupt, missing, legacy, permission, and concurrent-write tests.
  - Add a cross-platform exclusive desktop ownership lock and state fields for
    desktop session ID plus manager PID/start/executable identity.
  - Verification: `go test ./...`.

- [x] **1.3 Extract router process identity validation**
  - Move PID, process start identity, executable path normalization, and stale
    classification into shared Go packages.
  - Preserve Linux replaced-binary handling and Windows process-path behavior.
  - Ensure no stop path trusts PID alone.
  - Port existing fake-router and PID-reuse coverage to Go tests while keeping
    script-level regression coverage.
  - Verification: `go test ./... && make test-shell`.

- [x] **1.4 Implement router discovery and classification**
  - Probe port 19099, `/version`, and `/health` with bounded timeouts.
  - Correlate desktop state, CLI state, and process identity when available.
  - Add non-sensitive `deployment_id` and `management_protocol_version` to
    router `/version`, manager info, desktop expected values, release linking,
    CLI state, and install receipt.
  - Define the protocol version as a code-owned non-empty constant; fail
    production preflight for empty/default deployment IDs or artifact mismatch,
    and disable external reuse for development defaults.
  - Reuse an external router only when complete CLI state, full process
    identity, deployment ID, and management protocol version match.
  - Treat endpoint-only/manual routers as unknown even when response shape
    matches.
  - Return desktop-owned, external-compatible, degraded, stale, absent, and
    unknown-occupant states through typed results.
  - Add local test-server and unknown-occupant scenarios.
  - Verification: `go test ./...`.

- [x] **1.5 Implement manager-owned router lifecycle**
  - Support `desktop` foreground supervision and `cli` detached launch without
    parsing human-readable router output.
  - Start desktop routers without `-backend`.
  - Clear all inherited `MTLS_*` values for desktop launch and pass explicit
    localhost, foreground, and log values; prevent parent environment override
    of the embedded upstream and other router policy.
  - Capture output to the desktop log and retain bounded recent output.
  - Persist ownership state only for the verified child.
  - Wait for `/version`, classify `/health`, and report unexpected exit.
  - Implement graceful stop, five-second wait, identity revalidation, and
    force-stop fallback.
  - Reject stop for external, unknown, or stale processes.
  - Monitor the complete Tauri parent PID/start/executable identity and stop a
    verified desktop-owned router when that identity disappears.
  - Require the exclusive ownership lock for start/reclaim and support safe
    manager replacement only when the previous manager is absent and session,
    manager, and router identity rules match.
  - Add repeated-start, failed-start, not-ready, degraded, graceful-stop,
    forced-stop, stale, unexpected-exit, parent PID reuse, competing manager,
    manager crash, successful reclaim, and failed reclaim tests.
  - Verification: `go test ./...`.

- [x] **1.6 Extract Agent detection and path resolution**
  - Implement Claude Code, opencode, and Codex detection with current
    `CLAUDE_CONFIG_DIR`, `OPENCODE_CONFIG`, and `CODEX_HOME` semantics.
  - Preserve Codex desktop detection by home-directory existence.
  - Return path, existence, format, writability, and configured/invalid state.
  - Ensure results never include stored API-key values.
  - Verification: `go test ./...`.

- [x] **1.7 Implement structured Agent previews**
  - Produce typed previews for Claude JSON, opencode JSON/JSONC, Codex TOML,
    and Codex auth JSON.
  - Include affected paths, create/replace/preserve operations, backup plans,
    migration warnings, and revision tokens.
  - Preserve existing configuration semantics and reject unsafe JSONC target
    conflicts.
  - Add fixtures for missing, valid, invalid, conflicting, and already
    configured files.
  - Verification: `go test ./...`.

- [ ] **1.8 Implement transactional Agent writes**
  - Accept `api_key` only in the `agent.write` JSON request read from stdin,
    never command arguments or environment variables.
  - Revalidate preview revisions before mutation.
  - Create collision-resistant backups, use same-directory atomic replacement,
    and preserve supported file permissions.
  - Treat backups as sensitive Agent artifacts: inherit or tighten user-only
    permissions, identify the risk in preview/results, and exclude backup
    contents from logs/diagnostics.
  - Restore all files changed by a failed multi-Agent transaction while
    retaining diagnostic backups.
  - Persist and sync a sensitive-data-free transaction journal before the first
    replacement, update progress after each replacement, and recover incomplete
    journals before accepting manager requests.
  - Return per-Agent paths/results without returning the key.
  - Add success, stale-preview, backup failure, write failure, rollback,
    rollback-failure, deadline-after-first-replacement, and manager-crash-after-
    replacement tests.
  - Add assertions that keys are absent from logs, state, responses, and
    diagnostics.
  - Verify backup and rollback-backup permissions on Windows, macOS, and Linux,
    including intentionally over-permissive source files.
  - Verification: `go test ./...`.

- [x] **1.9 Add the `mtls-router-manager` command**
  - Wire the shared packages to the line-delimited JSON protocol.
  - Keep protocol stdout clean and direct diagnostics to stderr/logs.
  - Add subprocess contract tests covering sequential requests and sensitive
    input.
  - Add manager version metadata compatible with release linking.
  - Verification: `go test ./... && go vet ./...`.

- [x] **1.10 Convert setup scripts to manager wrappers**
  - Preserve all current router and Agent commands and compatibility aliases.
  - Preserve secure package-first install/download/checksum behavior.
  - Replace duplicated identity and Agent mutation paths with manager calls.
  - Keep human-readable shell output and no-argument router setup semantics.
  - Remove `MTLS_ROUTER_OPENAI_API_KEY`; keep hidden interactive input and add
    documented stdin manager requests for noninteractive automation.
  - Package the manager with CLI archives and verify both binaries in
    `SHA256SUMS`.
  - Install router and manager together into `MTLS_ROUTER_INSTALL_DIR` using a
    staged pair transaction, pending marker, committed receipt, and rollback.
  - Add a user-private install receipt with hashes, versions, deployment ID,
    protocol version, and paths.
  - Resolve only a checksum-verified sibling manager or receipt-verified
    installed manager; Agent commands do not implicitly download it.
  - Fail closed when either packaged/downloaded/installed member is missing,
    invalid, or mismatched, without partial overwrite or network fallback from
    a partial sibling package.
  - Reconcile pending transactions before executing either installed binary;
    add crash-point tests between replacements and before receipt commit.
  - Update shell fixtures/tests to execute a fake or built manager contract.
  - Verification: `make test-shell && go test ./...`.

- [x] **1.11 Complete Phase 1 regression verification**
  - Run Go formatting, all Go tests, vet, and shell tests.
  - Verify `setup.ps1` retains its UTF-8 BOM.
  - Verify existing README-documented CLI commands still behave as specified.
  - Verification: `test -z "$(gofmt -l .)" && go test ./... && go vet ./... && make test-shell`.

- [x] **1.12 Audit raw Go logging before desktop integration**
  - Ensure router startup, access, proxy error, and manager protocol logs never
    write query strings, authentication headers, request/response bodies,
    process environments, Agent payloads, or key-shaped canaries.
  - Preserve useful method/path/status/bytes/latency logging without request
    query data or sensitive upstream details.
  - Add focused Go tests for raw log files, malformed protocol input, startup
    failure, and proxy errors.
  - Verification: `go test ./... && go vet ./...`.

## Phase 2: Tauri Shell and Router UI

- [x] **2.1 Scaffold the Tauri 2 desktop workspace**
  - Add a Tauri 2 Rust application and React + TypeScript frontend in a focused
    desktop directory.
  - Pin Node and Rust dependencies with lockfiles.
  - Define frontend type-check/test/build and Rust format/test commands.
  - Do not add a third-party state library unless implementation demonstrates
    a concrete need.
  - Enforce a single desktop instance per user; a second launch activates the
    existing window without creating another manager/router.
  - Verification: frontend type check/tests/build and Rust format/tests.

- [ ] **2.2 Configure packaged manager and router sidecars**
  - Add architecture-qualified manager/router binaries to Tauri `externalBin`
    packaging.
  - Add least-privilege Tauri capabilities that permit only required sidecar
    execution.
  - Embed expected SHA-256 values and target triples in the desktop build.
  - Verify both sidecars before execution and complete a manager version/target
    handshake.
  - Implement safe `SIDECAR_MISSING`/`SIDECAR_INVALID` mapping.
  - Add build-time checks that both sidecars match the target architecture.
  - Verification: Rust tests and a local Tauri development launch.

- [x] **2.3 Implement the Rust manager client**
  - Supervise the manager process and line-delimited protocol.
  - Serialize frontend requests onto the manager stream, correlate request IDs,
    handle manager exit, and separate stderr logging from stdout protocol data.
  - Pass the Tauri parent PID to the manager for parent-death handling.
  - Pass parent start/executable identity and a random desktop session ID.
  - Apply per-method deadlines one second beyond manager deadlines.
  - On manager exit/timeout, disable lifecycle commands, attempt one bounded
    restart/reclaim, and enter manager-failed state if recovery fails.
  - Expose typed Rust results and sanitized errors.
  - Add tests with a fake manager process for valid, malformed, delayed, and
    terminated responses.
  - Verification: Rust format/tests.

- [x] **2.4 Add least-privilege Tauri commands**
  - Expose named commands for router status/start/stop/logs, Agent
    detect/preview/write, autostart, versions, paths, and diagnostics.
  - Validate every argument in Rust.
  - Ensure the frontend has no arbitrary shell, file, URL-fetch, or process
    command.
  - Add command-level permission and validation tests.
  - Verification: Rust tests and capability review.

- [x] **2.5 Implement first-launch router orchestration**
  - Validate sidecars, discover port 19099, reuse compatible external routers,
    start the bundled router when appropriate, and poll version/health.
  - Never modify Agent files during first launch.
  - Surface all required router states without unbounded restart loops.
  - Poll status immediately after actions, every two seconds while visible,
    and every ten seconds while hidden; skip ticks while another manager call
    is active, refresh immediately after it completes, and discard stale
    response generations.
  - Poll fresh health after availability, every ten seconds while visible,
    every thirty seconds while hidden, and on retry; mark health older than
    thirty seconds stale.
  - Verification: Rust integration tests with fake manager/router scenarios.

- [x] **2.6 Implement the Router page**
  - Show process state, upstream health, local address, owner, and versions.
  - Provide start, stop, health retry, and navigate-to-Agent actions.
  - Keep degraded state visually and semantically distinct from healthy.
  - Support desktop and mobile-width window layouts.
  - Add React component/state tests for every required router state.
  - Verification: frontend tests and production build.

- [x] **2.7 Implement logs and diagnostics UI**
  - Show a bounded recent log stream without loading the entire log file.
  - Add open-log-location and copy-sanitized-diagnostics actions.
  - Verify frontend rendering and copied output never reveal test secrets.
  - Consume the Phase 1 sanitized raw logs and verify frontend rendering and
    copied diagnostics remain sanitized.
  - Verification: frontend and Rust tests.

- [ ] **2.8 Implement tray lifecycle**
  - Add status-aware tray icon/menu with open, start, stop, logs, and quit.
  - Make window close hide the window without stopping the router.
  - Make quit stop only a verified desktop-owned router.
  - Keep an external router running on quit.
  - Add Rust lifecycle tests and manual platform checks.
  - Verification: Rust tests plus Windows/macOS/Linux tray smoke checks.

- [ ] **2.9 Implement current-user autostart**
  - Initialize Tauri autostart for Windows, macOS LaunchAgent, and Linux
    current-user startup.
  - Enable it on first launch by default and allow immediate disable/enable.
  - Ensure autostart applies normal discovery/ownership rules and does not
    require elevation.
  - Verification: plugin-level tests where possible and platform smoke checks.

## Phase 3: Agent UI

- [x] **3.1 Implement Agent detection cards**
  - Display Claude Code, opencode, and Codex detection, path, format,
    writability, and configuration state.
  - Do not preselect absent or invalid Agents.
  - Add refresh detection and actionable invalid-config messages.
  - Verification: frontend tests with all Agent states.

- [x] **3.2 Implement structured preview UI**
  - Show files, create/replace/preserve operations, backups, and risk warnings.
  - Show explicit opencode JSONC migration and Codex two-file effects.
  - Never render API-key values from current files or preview data.
  - Require explicit confirmation before moving to API-key input.
  - Verification: frontend tests and sanitized snapshot/DOM assertions.

- [x] **3.3 Implement transient API-key input and write flow**
  - Use a password field and keep the key only in transient component/Rust
    request memory.
  - Clear the frontend field after completion, cancellation, navigation, or
    error.
  - Clear logical references immediately and best-effort zeroize Rust buffers
    under application control without claiming forensic JavaScript-memory
    erasure.
  - Put the key only in the transient `agent.write` request sent through the
    controlled manager stdin/IPC path.
  - Display per-Agent success/failure and changed/backup paths.
  - Handle `PREVIEW_STALE` by returning the user to a refreshed preview.
  - Add tests ensuring the key is not persisted, logged, emitted in errors, or
    included in diagnostics.
  - Verification: frontend and Rust integration tests.

- [ ] **3.4 Add end-to-end Agent configuration fixtures**
  - Exercise create, merge, JSONC migration, Codex auth write, stale preview,
    invalid config, and transaction rollback from the Tauri command boundary.
  - Verify unrelated content and original backups are preserved.
  - Verification: desktop integration test suite.

## Phase 4: Localization, Packaging, and Documentation

- [ ] **4.1 Add Chinese-first localization**
  - Externalize all user-facing frontend and tray strings.
  - Default to Chinese and provide an English switch in Settings.
  - Persist only the language preference, never sensitive data.
  - Test missing-key behavior and both language paths.
  - Verification: frontend tests and manual language review.

- [x] **4.2 Complete Settings UI**
  - Add autostart, language, versions, app-data path, and log path.
  - On macOS and Linux, add **Prepare for uninstall** to remove current-user
    autostart and exit before application deletion.
  - Do not add upstream, certificate, sidecar replacement, update, or PATH
    controls.
  - Verification: frontend/Rust settings tests.

- [ ] **4.3 Add cross-platform desktop CI**
  - Add frontend static checks/tests/build and Rust format/tests to pull-request
    CI.
  - Add supported-target sidecar builds and package validation.
  - Keep existing Go and shell checks unchanged or stricter.
  - Avoid publishing release artifacts from pull requests.
  - Verification: CI workflows pass on a representative change.

- [ ] **4.4 Add desktop release packaging**
  - Build desktop + manager + credential-injected router for Windows amd64/
    arm64, macOS amd64/arm64, and Linux amd64/arm64.
  - Validate package contents, versions, architecture, executable permissions,
    and checksums before publication.
  - Integrate Windows/macOS signing and macOS notarization when credentials are
    present; never label unsigned artifacts as signed.
  - Keep MVP update endpoints/plugins disabled.
  - Configure Windows uninstall to remove current-user autostart while
    retaining Agent files, backups, logs, and diagnostic state.
  - Verify macOS DMG and Linux AppImage workflows use in-app uninstall
    preparation rather than claiming unavailable delete hooks.
  - Verification: package inspection and launch smoke tests per platform.

- [x] **4.5 Update documentation and changelogs**
  - Document desktop installation, first launch, tray behavior, default
    autostart, external-router reuse, Agent preview/write, API-key boundary,
    uninstall behavior, and troubleshooting.
  - Document macOS/Linux **Prepare for uninstall**, exit, then delete the app as
    an explicit sequence.
  - Document removal of `MTLS_ROUTER_OPENAI_API_KEY` and the stdin automation
    replacement in English and Chinese.
  - Update English and `docs/zh-CN` documentation together.
  - Update maintainer build/release documentation for Go manager, Rust,
    Node.js, sidecars, signing, and packaging.
  - Keep CLI instructions and precedence documentation accurate.
  - Verification: documentation link/parity review.

- [ ] **4.6 Execute final acceptance matrix**
  - Complete every item in `checklist.md` on supported target runners or record
    an explicit release blocker.
  - Run all repository format, test, vet, shell, frontend, Rust, build, package,
    and smoke checks.
  - Verify no real credentials or API keys appear in source, fixtures, logs,
    artifacts outside the credential-injected router, or diagnostics.
  - Exercise process identity, PID reuse, graceful/forced stop, external-router
    trust, and ownership on Windows, macOS, and Linux; test both architectures
    where process APIs or packaging behavior differ.
  - Verify sensitive backup permissions and rollback-retained backups on
    Windows, macOS, and Linux.
  - Verification: all acceptance evidence recorded before release approval.
