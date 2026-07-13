# Tauri Desktop Application Acceptance Checklist

This checklist defines observable release acceptance. Check an item only after
the behavior has been demonstrated by an automated test or recorded manual
verification on the stated platform.

## Specification and Scope

- [ ] The implementation matches `spec.md` and all approved scope decisions.
- [x] Runtime certificate/private-key/CA import is not exposed.
- [x] Multiple upstream profiles are not exposed.
- [x] Automatic application/router updates are not enabled.
- [x] The bundled router is not installed into `PATH`.
- [x] Uninstall does not restore or delete Agent configuration, backups, logs,
  or diagnostic state.
- [ ] Windows uninstall removes the current-user autostart registration.
- [x] macOS and Linux provide in-app uninstall preparation that removes
  autostart and exits before application deletion.

## Shared Manager

- [x] The manager uses line-delimited JSON over stdin/stdout and opens no
  management network listener.
- [x] The manager processes requests sequentially and exits cleanly on stdin
  EOF.
- [x] Every response correlates to a request ID.
- [x] Malformed or missing request IDs produce `id:null` with
  `INVALID_REQUEST`.
- [x] Protocol stdout contains no human-readable log output.
- [x] Invalid requests and operation failures return stable documented error
  codes.
- [x] Manager version metadata is available to CLI and desktop consumers.
- [x] Manager method deadlines and Rust watchdog deadlines fail boundedly and
  do not leave a second owner or router.
- [x] Every protocol method uses its specified deadline and timeout error;
  router-start timeout cleans up its verified child.
- [x] API keys are absent from manager responses, state, logs, errors, and
  diagnostics.

## Router Lifecycle and Safety

- [x] Desktop startup runs the bundled router without `-backend`.
- [x] Desktop startup clears inherited `MTLS_*` variables and cannot be forced
  into backend mode, a public listener, or a different upstream by the parent
  environment.
- [x] CLI manager start uses detached launch without parsing human-readable
  router output.
- [x] Desktop state records PID, listen address, executable path, log path,
  process start identity, versions, ownership, desktop session ID, and manager
  PID/start/executable identity.
- [x] State writes are atomic and use user-restrictive permissions where
  supported.
- [x] Running and upstream-healthy states are distinct.
- [x] A degraded upstream leaves the router process running and provides retry.
- [x] A normal stop attempts graceful shutdown before force termination.
- [x] Force termination occurs only after complete process identity
  revalidation.
- [x] A stale state sends no signal.
- [x] PID reuse sends no signal to the reused process.
- [ ] A replaced Linux executable remains safely manageable when its recorded
  process identity is otherwise genuine.
- [x] Unexpected router exit is visible with sanitized recent logs.
- [x] Startup failure does not trigger an unlimited restart loop.
- [x] Manager parent-death handling stops only a verified desktop-owned router.
- [x] A router left by manager failure is reclaimed only when complete desktop
  state identity matches.
- [x] Parent PID reuse does not keep an orphan manager alive.
- [x] An exclusive ownership lock prevents two managers from controlling one
  router.
- [x] One bounded manager restart can reclaim safely; failed recovery disables
  lifecycle controls and starts/signals no second process.

## Existing Router and Port Conflict

- [x] A compatible setup-managed router is detected and shown as external.
- [x] The desktop can use health/version information from an external router.
- [x] Desktop quit does not stop an external router.
- [x] An unknown process on port 19099 is reported and never killed.
- [x] External reuse requires complete CLI state, full process identity,
  matching deployment ID, and matching management protocol version.
- [x] Endpoint-only or manually started routers without trusted setup state are
  treated as unknown.
- [x] Production router, manager, and desktop share one non-default deployment
  ID and code-owned management protocol version.
- [x] Production release preflight rejects empty/default/mismatched identities;
  development defaults cannot reuse external routers.
- [x] The desktop does not silently switch to a different port.
- [x] An unverifiable PID or endpoint is treated as unknown/stale, not owned.

## Setup CLI Compatibility

- [x] `setup.sh router install/start/setup/status/log/stop` still works.
- [ ] `setup.ps1 router install/start/setup/status/log/stop` still works.
- [x] Agent print/write commands and compatibility aliases still work.
- [x] `MTLS_ROUTER_OPENAI_API_KEY` no longer supplies a key and fails with the
  documented migration guidance when used as the only key source.
- [x] Hidden interactive input and stdin-based automation both write Agent
  configuration successfully.
- [x] No-argument setup still installs/starts the router without changing
  Agent files.
- [x] Package-first checksum verification remains fail-closed.
- [x] HTTPS-only custom download validation remains enforced.
- [x] Setup state retains process-start and executable identity protections.
- [x] CLI packages include manager, router, setup script, and matching checksum
  entries.
- [x] `router install/setup` installs or updates router and manager as one
  staged transaction and restores the prior committed pair on failure.
- [x] Partial sibling packages fail closed without network fallback.
- [x] Download installation verifies both binaries before replacing either.
- [x] Agent commands execute only a checksum-verified sibling manager or a
  receipt-verified installed manager and never download one implicitly.
- [x] A pending install transaction is reconciled before execution; crash
  points between binary replacements and receipt commit never execute a mixed
  generation.
- [x] `setup.ps1` retains its UTF-8 BOM.

## First Launch and Router UI

- [ ] A new user can start the router without opening a terminal.
- [x] First launch validates both sidecars.
- [x] Sidecar SHA-256, target triple, and manager version/target handshake are
  validated before use.
- [x] First launch reuses a compatible external router or starts the bundled
  router.
- [x] First launch does not write any Agent file.
- [x] The Router page shows local address, owner, health, and all component
  versions.
- [x] The UI represents not started, starting, healthy, degraded, external,
  occupied, failed, and stopping states.
- [x] Missing/invalid sidecars report reinstall required and do not download.
- [ ] The Router page works at normal desktop and narrow window widths.
- [ ] A second desktop launch activates the existing window and creates no
  second manager/router.
- [x] Router/tray state refreshes within two seconds while visible and ten
  seconds while hidden when idle; active operations skip ticks, trigger an
  immediate post-operation refresh, and stale responses cannot overwrite newer
  state.
- [x] Fresh health is scheduled every ten seconds while visible and thirty
  seconds while hidden; health older than thirty seconds is shown stale.

## Tray and Autostart

- [ ] Closing the main window hides it without changing the router PID.
- [ ] The tray opens the window and provides start, stop, logs, and quit.
- [ ] Tray status distinguishes normal, warning, and error.
- [ ] Tray quit stops a verified desktop-owned router.
- [ ] Tray quit leaves an external router running.
- [ ] Autostart is enabled by default for the current user.
- [ ] Autostart can be disabled and re-enabled without administrator rights.
- [ ] Autostart uses normal router discovery and ownership rules.

## Agent Detection and Preview

- [x] Claude Code detection respects `CLAUDE_CONFIG_DIR`.
- [x] opencode detection respects `OPENCODE_CONFIG` and canonical JSON/JSONC
  fallback.
- [x] Codex detection respects `CODEX_HOME` and desktop home-directory
  detection.
- [x] Detection reports path, existence, format, writability, and state.
- [x] Detection does not return stored API-key values.
- [x] Absent or invalid Agents are not preselected.
- [x] Preview writes no files.
- [x] Preview lists every affected path and backup plan.
- [x] Preview distinguishes create, replace, and preserve operations.
- [x] opencode JSONC migration explicitly warns about comment/format loss.
- [x] Explicit `.jsonc` `OPENCODE_CONFIG` previews as exact-path strict JSON
  normalization with an in-place warning and a required backup for existing
  content.
- [x] Codex preview separately shows `config.toml` and `auth.json` effects.
- [x] Preview never renders an API-key value.

## Agent Write and Rollback

- [x] Write requires explicit Agent selection, preview approval, and API-key
  entry.
- [x] The API key is not passed through process arguments or environment
  variables.
- [x] The desktop clears transient API-key input on success, cancel,
  navigation, and error.
- [x] File revision changes after preview produce `PREVIEW_STALE` before
  mutation.
- [x] Existing files are backed up before mutation.
- [x] Backups use collision-resistant names and preserve original content.
- [x] Backups containing previous keys use inherited or stricter user-private
  permissions and are identified as sensitive recovery artifacts.
- [ ] Sensitive backup and rollback-backup permissions are verified on Windows,
  macOS, and Linux with over-permissive source fixtures.
- [x] Writes use same-directory atomic replacement where supported.
- [x] A failed multi-Agent transaction restores files already changed by that
  transaction.
- [x] Agent-write deadline after an earlier replacement rolls back before
  response.
- [x] Manager restart recovers an interrupted write journal before accepting
  new Agent operations.
- [x] Diagnostic backups remain available after rollback.
- [x] Results accurately identify success/failure, changed files, and backups
  per Agent.
- [x] Results and errors never return the API key.

## Agent Configuration Semantics

- [x] Claude preserves non-`env` top-level fields and replaces the managed
  `env` block.
- [x] opencode preserves unrelated root fields and providers.
- [x] opencode writes current `mtls-router` provider/model definitions.
- [x] Canonical JSONC migration refuses to overwrite an existing sibling
  `opencode.json`.
- [x] Explicit `.jsonc` `OPENCODE_CONFIG` writes only its exact path and does
  not read, overwrite, or fall back to a sibling `opencode.json`.
- [x] Codex preserves unrelated root keys and sections.
- [x] Codex replaces managed root model keys and
  `[model_providers.custom]` exactly once.
- [x] Codex `auth.json` contains only the supplied `OPENAI_API_KEY` when
  written.
- [x] Invalid JSON/JSONC/TOML remains unchanged.

## Logs, Diagnostics, and Security

- [x] The Logs page reads a bounded recent range rather than the entire file.
- [x] Users can open the log location.
- [x] Users can copy a sanitized diagnostic summary.
- [x] GUI logs and diagnostics redact API keys and key-shaped test secrets.
- [x] GUI logs and diagnostics redact certificates, private keys, CA contents,
  and sensitive parameters.
- [x] Raw router and manager logs never write query strings, auth headers,
  bodies, process environments, Agent write payloads, or key-shaped test
  values.
- [x] The frontend has no arbitrary shell, arbitrary file, or arbitrary process
  capability.
- [x] Tauri sidecar permissions are restricted to packaged manager/router
  operations.
- [x] Management endpoints remain on trusted localhost only.
- [ ] No real secret is committed in source or test fixtures.

## Settings and Localization

- [x] Chinese is the default UI language.
- [x] Users can switch to English.
- [ ] All frontend and tray strings use localization resources.
- [x] Settings include autostart.
- [x] macOS and Linux Settings include **Prepare for uninstall**.
- [x] Settings show desktop, manager, and router versions.
- [x] Settings show application data and log locations.
- [x] Settings expose no upstream, certificate import, sidecar replacement,
  automatic update, or PATH controls.

## Build, Test, and Release

- [x] `test -z "$(gofmt -l .)"` passes.
- [x] `go test ./...` passes.
- [x] `go vet ./...` passes.
- [x] `make test-shell` passes.
- [x] Frontend static checks/type checking pass.
- [x] Frontend unit tests pass.
- [x] Frontend production build passes.
- [x] Rust formatting check passes.
- [x] Rust tests pass.
- [ ] Windows amd64 package builds and launches.
- [ ] Windows arm64 package builds and launches.
- [ ] macOS Intel package builds and launches.
- [ ] macOS Apple Silicon package builds and launches.
- [ ] Linux amd64 package builds and launches.
- [ ] Linux arm64 package builds and launches.
- [ ] Each package contains architecture-matching manager and router sidecars.
- [ ] Each package installs/runs for the current user without elevation.
- [ ] Package content, versions, permissions, and checksums are validated before
  publication.
- [ ] Signed/notarized status is reported accurately and never inferred when
  credentials are absent.
- [ ] Process identity, PID reuse, graceful/forced stop, external trust, and
  ownership checks pass on Windows, macOS, and Linux.

## Documentation

- [x] English README documents desktop installation and behavior.
- [x] Chinese README contains equivalent desktop guidance.
- [x] Build documentation covers Go manager, Node.js, Rust, sidecars,
  packaging, signing, and notarization.
- [x] Troubleshooting covers sidecar failure, unknown port occupation, stale
  state, degraded upstream, invalid Agent config, and stale preview.
- [x] Documentation explains that embedded shared credentials are extractable
  and rotated through replacement releases.
- [x] Documentation explains that the desktop does not retain API keys beyond
  Agent files and explicitly approved sensitive recovery backups.
- [x] English and Chinese migration docs explain removal of
  `MTLS_ROUTER_OPENAI_API_KEY` and stdin-based automation.
- [x] Changelogs are updated in English and Chinese.
