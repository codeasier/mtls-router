# Changelog

[中文](zh-CN/CHANGELOG.md)

## v0.3.3 - 2026-08-01

This release adds safe per-Agent cleanup of CodeasierRouter-managed settings, introduces layered desktop development workflows, unifies CodeasierRouter branding, and preserves the macOS fullscreen Space when closing the app to the tray.

### Added

- Added independent cleanup preview and write flows for removing one Agent's CodeasierRouter-managed provider, model, and file-backed authentication settings while preserving unrelated user configuration, the desktop global API key, and historical backups.
- Added browser-only mock development, reuse of existing Tauri sidecars, and disposable real-Agent configuration paths, with production builds rejecting bundled mock code.

### Changed

- Standardized the CodeasierRouter name across provider presentation, model-config exports and schema metadata, desktop diagnostics, and desktop release artifacts while preserving compatibility-sensitive internal identifiers.

### Fixed

- Hid the macOS application before hiding its window during close-to-tray so fullscreen windows keep their Space, without changing the Windows or Linux close sequence.

### Security and recovery

- Required trusted sidecar ownership for cleanup, bound key-free previews to signed revisions, required confirmation after managed-state drift, used private transactional backups with delete-capable journal recovery, and required a fresh preview after ambiguous delivery.

### Tests

- Expanded Go, frontend, Rust, and workflow coverage for cleanup ownership, revisions, file races, backup and rollback recovery, ambiguous delivery, responsive interaction, mock isolation, naming consistency, and native tray ordering.

---

## v0.3.2 - 2026-07-29

This release restores expected macOS focus behavior when closing the desktop app to the tray and improves Agent configuration discovery and form readability.

### Fixed

- Deactivated the macOS application when the main window closes to the tray so the previous app regains focus, then reactivated it before restoring the window from the tray or a second launch.
- Centered Agent discovery and execution status when no editor is visible, kept active editor operations in the sticky status rail, removed decorative request badges, and increased configuration typography for high-resolution displays.

### Tests

- Expanded native tray close and activation coverage plus frontend regression coverage for Agent status placement, responsive layout, badge removal, and configuration typography.

---

## v0.3.1 - 2026-07-28

This release improves desktop layout density and status clarity, removes unreliable Agent CLI installation probing, and makes authenticated model discovery visibly responsive.

### Changed

- Removed Agent CLI installation probing and its desktop status; protocol v4 keeps the compatibility fields and returns fixed `detected=true` and `command=""` values.
- Tightened desktop spacing across supported viewport sizes, simplified section headers, and replaced the router status code dial with a traffic-light indicator.

### Fixed

- Displayed an in-progress state while an Agent panel initializes authenticated model discovery instead of leaving the panel apparently idle.

### Tests

- Updated manager and desktop coverage for fixed Agent compatibility fields, model discovery progress, responsive layout density, router status indicators, and occupant focus restoration.

---

## v0.3.0 - 2026-07-28

This release introduces persistent desktop Agent credentials, an Agent status overview, and durable single-Agent configuration panels. It also advances the management contract to protocol v4 and hardens router startup and privilege-aware port-conflict recovery.

### Added

- Added an API Keys page backed by a private desktop credential store for one global Agent API key. The webview can read only a summary; Rust loads the plaintext on demand for authenticated discovery and reloads it immediately before writes.
- Added an Agent overview with separate Claude Code, OpenCode, and Codex cards that report CLI installation independently from configuration existence, writability, and validity, while still allowing writable preinstallation configuration.
- Added persistent single-Agent panels that stay open after successful writes, protect unsaved drafts, refresh external state on demand or throttled native focus, and require explicit conflict resolution without polling or background file rewrites.

### Changed

- Advanced the matched router, manager, setup, release metadata, and desktop management contract to protocol v4; mixed generations remain rejected.
- Reworked port-conflict handling around structured recovery actions and reasons, bounded Windows Service/Linux systemd supervisor identifiers, explicit manual guidance, and sampled post-termination observation for reoccupation.

### Fixed

- Preserved sanitized pre-launch router diagnostics, reconciled desktop status after rejected starts, and allowed startup when unrelated stale CLI state exists.
- Rejected known different-user Windows SIDs instead of degrading them to PID-only recovery, required terminate-access preflight when identity is unreadable, and kept supervised recovery non-elevating and fail-closed.
- Kept empty preview collections correctly typed and serialized router lifecycle operations so overlapping start, stop, and quit requests cannot corrupt desktop state.

### Tests

- Added desktop regression coverage for credential lifecycle and zeroization boundaries, Agent overview accessibility, persistent panel drafts, refresh conflicts, flow cleanup, write reloads, and guarded exits.
- Expanded native and cross-platform coverage for startup diagnostics, stale discovery state, supervisor classification, permission preflight, termination evidence, reoccupation sampling, stable errors, and protocol-v4 artifact consistency.

---

## v0.2.1 - 2026-07-24

This release refines desktop navigation and Agent presentation, improves readability and scrolling, and preserves bounded, sanitized diagnostics when a desktop-owned router fails during startup.

### Added

- Added a collapsible desktop sidebar with a persisted preference while keeping every section accessible from the compact icon rail.

### Changed

- Compacted Agent selection cards, added official Claude Code, OpenCode, and Codex logos, and standardized the user-visible `OpenCode` name without changing internal identifiers or configuration paths.

### Fixed

- Increased Agent configuration label, hint, model identifier, and control sizing for better readability.
- Kept the desktop tab header fixed while constraining scrolling to the content pane across desktop and mobile layouts.
- Preserved bounded, sanitized output from failed router startups, including immediate Windows exits and inherited-handle cases, and exposed the diagnostics through router status and runtime logs without mixing external CLI router output.

### Tests

- Added desktop regression coverage for sidebar collapse and persistence, configuration typography, control sizing, scroll ownership, and failure-log navigation.
- Expanded manager lifecycle and app coverage for startup output draining, cleanup, sanitization, log merging, and Windows immediate-exit behavior.

---

## v0.2.0 - 2026-07-23

This release introduces authenticated, catalog-driven Agent configuration across the manager, setup scripts, and desktop app, with immutable build presets, explicit recovery workflows, and fail-closed handling of model availability, authorization, and sensitive configuration state.

### Added

- Added authenticated `GET /v1/models` discovery and one common all-endpoint-compatible catalog for Claude Code, opencode, and Codex.
- Added protocol-v2 `agent.models`/`agent.render`, canonical key-free model config, Agent-native options, redacted render/preview, write-time catalog refresh, managed ownership state, and drift/Codex-auth approvals.
- Added optional Claude display names and canonical `context: "1m"` for every explicit selection. Canonical and catalog identity remains the base model ID; `[1m]` is appended only at the Claude rendering boundary, without capability inference or management of `CLAUDE_CODE_DISABLE_1M_CONTEXT`.
- Added immutable key-free build presets to manager binaries. Protocol v2 now returns stable `preset.model_config` and `preset.unavailable_agents` objects after independent authenticated validation of each requested Agent section.
- Added manager-only `SIMPLIFY` build policy. Unset/empty and ASCII-case booleans normalize before compilation, default `True` excludes valid IDs containing ASCII `/`, and `False` retains every valid ID.
- Added separately approved desktop backup-and-rebuild recovery for narrowly eligible syntax-invalid Claude Code, opencode, and Codex configurations. Valid syntax, unsafe targets, and unresolved transaction recovery remain ineligible.

### Changed

- Moved Shell, PowerShell, and desktop configuration to key-before-discovery with per-Agent `existing > preset > empty` editable initialization, omission of unset optional fields, and no static/cached model fallback. Explicit `--model-config` and desktop imports remain complete replacements.
- Refined automatic-selection behavior: only a visible build preset whose exact IDs pass the current authenticated catalog may initialize a form. First-model selection, model-name or capability heuristics, partial preset repair, substitution, and runtime fallback remain prohibited.
- Migrated Claude to managed `env` merge, opencode to the exact selected provider catalog, and Codex from the historical `custom` provider to dedicated `mtls-router` plus separately approved file-backed API-key auth.
- Detection now describes local structural completeness only; current authorization is established only by discovery and write-time refresh.
- Made the validated, deduplicated, bounded, sorted, and build-filtered catalog authoritative for protocol tokens/results, existing and preset availability, import, preview, and write-time refresh. Filtering happens after complete validation, so malformed hidden IDs remain `MODEL_RESPONSE_INVALID`, all-filtered catalogs return `MODEL_CATALOG_EMPTY`, and refresh disappearance remains fail-closed.
- Rebuild now renders managed-only files rather than preservation-merging malformed input: Claude keeps only managed `env`, opencode becomes strict JSON at the approved path, and Codex replaces both `config.toml` and `auth.json`. Setup-script Agent commands remain merge-only, with no force-overwrite fallback.

### Security and release

- Added private signed catalog/revision state, shared operation locking, transactional sidecar/backups, compatibility revision pins, and release preflight rejection for mixed protocol generations.
- Documented that Agent and approved backup files may contain keys while model config, tokens, sidecar, logs, diagnostics, and protocol results do not.
- Added optional `AGENT_MODEL_PRESET_BASE64` release input with preflight validation and identical standalone/desktop manager injection. Invalid nonempty input fails manager startup without content leakage; empty input is valid, and router binaries, including desktop router sidecars, never receive preset data.
- Release builds normalize `SIMPLIFY` before compilation and inject the same value into standalone and desktop managers only. It is not a router/runtime preference and does not change proxy route support.
- Rebuild backs up every existing file in the complete approved set byte-for-byte before replacement. Successful results report sensitive backup paths without contents; backup failure changes no target, later failure rolls back transactionally, and unproven recovery disables further writes.

---

## v0.1.8 - 2026-07-16

This release improves Windows desktop process containment so routers started by CodeasierRouter do not outlive their owning desktop session or leave visible console windows.

### Fixed

- Contained Windows desktop-started routers in kill-on-close Job Objects and stopped owned routers when their manager session ends.
- Marked production Windows desktop builds as GUI applications so launching CodeasierRouter no longer opens a console window.

### Tests

- Added Windows lifecycle coverage for suspended process launch, Job Object configuration, and router termination when containment closes.
- Added release package verification for the Windows GUI subsystem and manager-session cleanup coverage.

---

## v0.1.7 - 2026-07-15

This release supersedes the unpublished `v0.1.6` tag. The `v0.1.6` Release workflow did not publish a GitHub Release because fallback Intel macOS bundle sealing failed; its tag remains unchanged for auditability.

### Fixed

- Fixed fallback macOS packaging by ad-hoc signing the embedded router and manager sidecars before bundling, then signing the generated desktop executable before sealing the application bundle.
- Kept fallback signing explicit and non-recursive so package verification can continue comparing packaged sidecar hashes with their signed source files.

### Tests

- Expanded release workflow assertions to enforce dependency-ordered fallback macOS signing and continue rejecting recursive bundle signing.

---

## v0.1.6 - 2026-07-15

This release simplifies the CodeasierRouter desktop interface and hardens fallback macOS application packaging so unsigned builds remain structurally valid before DMG assembly.

### Changed

- Simplified the desktop router page, navigation, settings entry, and status presentation to reduce visual density and keep primary router controls prominent.

### Fixed

- Added ad-hoc code signing for fallback macOS application bundles before DMG creation, ensuring modified bundles are sealed even when release signing credentials are unavailable.

### Tests

- Updated desktop UI coverage for the simplified router experience.
- Expanded package verification and release workflow regression coverage for fallback macOS bundle sealing.

---

## v0.1.5 - 2026-07-15

This release refreshes the CodeasierRouter desktop interface and improves macOS installation and tray integration. Agent configuration is now available even when supported CLI executables are not visible on the manager process PATH, and release publication gains a controlled recovery path with stricter artifact validation.

### Added

- Added a controlled release-recovery workflow that reuses validated artifacts from a matching failed tag build without moving or rewriting the release tag.
- Added a native macOS tray template asset with Retina dimensions and transparent safe bounds.

### Changed

- Reworked the desktop interface with a warm beige and orange visual system across navigation, router controls, Agent configuration, logs, and settings.
- Made Claude Code, opencode, and Codex configuration targets available independently of CLI discovery while retaining executable paths as optional diagnostics.
- Extracted deterministic release assembly into a shared packaging script used by normal and recovery publication.

### Fixed

- Added an `Applications` shortcut to macOS DMGs and package checks that reject missing or incorrect shortcuts.
- Replaced the dense macOS tray monogram with a stable native template icon that follows light and dark menu-bar rendering.
- Hardened failed-release recovery with draft resumption, explicit repository targeting, the correct GitHub upload endpoint, exact asset-manifest verification, tag-SHA revalidation, and latest-version downgrade protection.

### Tests

- Expanded release workflow and packaging regression coverage for recovery safety, deterministic assembly, and exact artifact manifests.
- Added Agent detection/configuration tests for missing CLIs and macOS package/tray checks.

---

## v0.1.4 - 2026-07-13

This release introduces CodeasierRouter, a Tauri 2 desktop control panel and shared management service, while hardening the CLI router's TLS, streaming, background-process, installation, and process-identity behavior. Release tooling now builds and inspects native desktop packages and matched router/manager artifacts across all six supported OS/architecture targets.

### Added

- Added a Tauri 2 desktop control panel for current-user router lifecycle, separate process/upstream health, tray operation, default launch-at-login, bounded logs, diagnostics, settings, and Chinese/English UI.
- Added verified architecture-specific `mtls-router-manager` and credential-injected `mtls-router` desktop sidecars with build-time/runtime hash and architecture checks plus manager version/target/deployment/protocol handshake.
- Added safe external CLI-router reuse, strict unknown-port conflict behavior on `127.0.0.1:19099`, stale identity protection, and degraded/stale health presentation.
- Added Claude Code, opencode, and Codex detection, structured preview, sensitive backups, atomic transactional writes, stale-preview rejection, and rollback/recovery through the shared Go manager.
- Added bilingual desktop operation and troubleshooting guides, including install, first launch, Agent safety boundaries, uninstall, credential rotation, and package verification.

### Changed

- Extracted router lifecycle and Agent file management into `mtls-router-manager serve`, a sequential line-delimited JSON stdin/stdout protocol shared by the desktop and setup wrappers.
- Changed CLI release installation to stage, verify, install, and receipt-track the router and manager as one matched pair.
- Removed `MTLS_ROUTER_OPENAI_API_KEY`. Interactive setup reads the key without echo; automation must preview and send `agent.write` with the transient key only through manager stdin.

### Fixed

- Rejected non-HTTPS upstream URLs and applied the configured minimum TLS version consistently to startup probes, `/health`, and proxied upstream traffic.
- Preserved immediate response streaming through access logging and kept proxy request handling on the reverse proxy's direct streaming pipeline without pass-through body wrappers.
- Prevented detached child processes from inheriting backend mode and recursively spawning.
- Hardened router stop and install reconciliation against missing, stale, or mismatched process identity and release artifact state.
- Cleared recoverable desktop router error alerts after a newer healthy snapshot while keeping sidecar integrity failures fail-closed.

### Security

- Desktop/manager state, logs, diagnostics, protocol responses, process arguments, and environment variables do not intentionally retain Agent API keys. Agent-owned files and explicitly approved recovery backups remain the persistence boundary and must be protected as sensitive data.
- Documented that the router's shared embedded client private key is extractable from distributed binaries and must be rotated through a complete replacement release plus server-side revocation.
- Uninstall preserves Agent files, sensitive backups, logs, and state. Windows installer integration must remove current-user autostart; macOS/Linux users must run **Prepare for uninstall**, wait for exit, then delete the application.

### Tests

- Expanded Go, shell, React, Rust, and workflow coverage for TLS policy, streaming, background child state, process identity, manager protocol, Agent configuration transactions, desktop orchestration, package verification, and signing-status reporting.

### Release status

- CI and release workflows now build native desktop packages for all six targets: Windows x86_64/arm64 NSIS, macOS Intel/Apple Silicon DMG, and Linux x86_64/arm64 AppImage. Each matching target runner performs mandatory package inspection. Release jobs sign Windows/macOS packages only when signing credentials are complete, notarize/staple macOS applications only when the additional Apple credentials are complete, and emit one explicit status file per target. Package inspection does not install or launch the application, so separate successful target-runner install/launch evidence remains a release gate.

---

## v0.1.3 - 2026-07-10

This release focuses on release-install reliability and proxy correctness. Packaged setup scripts now point at the reachable download server by default, the proxy no longer carries unused stream-detection plumbing, client request-body failures are reported as bad requests, and `/health` now probes the configured upstream runtime target.

### Changed

- Simplified proxy request handling by removing the unused stream-detection pre-read path while preserving direct reverse-proxy streaming behavior.
- Updated release-packaged setup defaults so installers download binaries from the configured release server address.

### Fixed

- Fixed packaged setup scripts to use the direct download server IP, avoiding hostname reachability issues in affected environments.
- Fixed client request-body read failures so they return `400 Bad Request` instead of being classified as upstream proxy failures.
- Fixed `/health` so runtime probe options are passed through and the handler checks the configured upstream target.

### Tests

- Added regression coverage for client body read error classification and health probe option propagation.
- Kept setup script coverage aligned with release download defaults and router health behavior.

---

## v0.1.2 - 2026-06-21

### Added

- Added split setup entry points for router and agent configuration commands.
- Added interactive migration for existing opencode JSONC configuration.
- Added router lifecycle commands to the setup flow for start, stop, restart, and status-style management.
- Added build metadata embedding across all build targets:
  - version
  - commit
  - build date
- Added internal build metadata endpoint at `/internal/version`.
- Added router listener management endpoints:
  - `/version`
  - `/health`

### Changed

- Updated Codex CLI setup to use minimal custom provider configuration.
- Updated opencode setup to use the `/v1` base URL and JSONC-based configuration handling.
- Updated model IDs to drop the `cx/` prefix for Claude and opencode targets.
- Updated README to match current setup script defaults and document the management endpoints.

### Fixed

- Fixed setup so config-writing steps do not unexpectedly start the router.
- Fixed setup logging so logs stay out of the install directory by default.
- Fixed Windows router startup behavior.
- Fixed Windows PowerShell script encoding and JSON parsing behavior.
- Fixed Windows Codex CLI config matching and auth file generation, including BOM-free `auth.json` output.
- Hardened router lifecycle command behavior in setup scripts.

### Tests

- Updated shell tests for the non-interactive setup flag flow.
- Extended setup coverage for PowerShell JSON handling and lifecycle-related behavior.

---

## v0.1.1 - 2026-06-19

### Added

- Added one-click setup scripts for macOS/Linux and Windows:
  - `setup.sh`
  - `setup.ps1`
- Added automatic latest release binary download in setup scripts.
- Added interactive setup wizard for configuring supported local agents:
  - Claude Code
  - opencode
  - Codex CLI
- Added automatic backup of existing agent configuration files before modification.
- Added background runtime mode with `-backend`.
- Added log file support with `-log`.
- Added environment variable support for background mode and log path:
  - `MTLS_BACKEND`
  - `MTLS_LOG`
- Added cross-platform detached process support for background mode:
  - Unix/macOS/Linux process session handling
  - Windows detached process creation
- Added setup script test suite.
- Added PowerShell setup flow tests.
- Added `make test-shell` for running shell-based setup tests.
- Added maintainer build and release documentation in `docs/BUILD.md`.

### Changed

- Updated README with one-click setup, manual binary download, Windows usage, background mode, and agent configuration instructions.
- Moved detailed build and release instructions from README into `docs/BUILD.md`.
- Updated CI to run shell setup tests.
- Improved meta flag handling so runtime flags can pass through correctly.
- Clarified Windows release usage and production service guidance.

### Fixed

- Fixed parsing behavior for runtime flags such as `-backend` and `-log`.
- Fixed Windows setup wizard behavior to better match the Unix setup flow.
- Fixed Windows setup agent configuration behavior.
- Tightened background startup integration and log file handling.

### Tests

- Added Go tests for background argument handling and log behavior.
- Added Go tests for new configuration fields.
- Added tests for `-version`, `-help`, and runtime flag handling.
- Added setup tests covering clean setup, latest version detection, target selection, Claude Code, opencode, Codex CLI, and PowerShell flows.

---

## v0.1.0 - 2026-06-18

### Added

- Initial release of `mtls-router`.
- Added single-binary local reverse proxy for routing plain local HTTP traffic to an upstream HTTPS mTLS endpoint.
- Added embedded certificate support through build-time linker variables:
  - `main.clientCertPEM`
  - `main.clientKeyPEM`
  - `main.upstreamCAPEM`
  - `main.upstreamURL`
  - `main.version`
- Added local HTTP listener with default address `127.0.0.1:19099`.
- Added upstream mTLS transport using embedded client certificate, private key, and upstream CA.
- Added startup health probe against the upstream before accepting local traffic.
- Added request forwarding to the configured upstream URL.
- Added transparent request body streaming.
- Added Server-Sent Events response handling with streaming-safe headers.
- Added stream request detection for JSON requests containing `"stream": true`.
- Added structured access logging.
- Added graceful shutdown on `SIGINT` and `SIGTERM`.
- Added runtime configuration via flags, environment variables, build-time values, and defaults.
- Added local development build script: `scripts/build.sh`.
- Added placeholder certificate generation for local development builds.
- Added GitHub Actions CI workflow.
- Added GitHub Actions release workflow.
- Added release cross-compilation for Linux, macOS, and Windows on amd64 and arm64.
- Added Docker support with `Dockerfile`.
- Added systemd service unit: `systemd/mtls-router.service`.
- Added README, design specification, implementation plans, and MIT license.

### Configuration

- Added configuration precedence: flag > environment variable > build-time value > default.
- Added environment variables:
  - `MTLS_LISTEN_ADDR`
  - `MTLS_UPSTREAM_URL`
  - `MTLS_TLS_MIN`
  - `MTLS_TIMEOUT`
  - `MTLS_DEBUG`
- Added runtime flags:
  - `-listen`
  - `-upstream`
  - `-tls-min`
  - `-timeout`
  - `-debug`
  - `-version`
  - `-help`
  - `-h`

### Tests

- Added unit tests for certificate loading and validation.
- Added unit tests for configuration loading and validation.
- Added unit tests for upstream health probing.
- Added unit tests for logging helpers.
- Added unit tests for reverse proxy director behavior, error handling, response modification, streaming detection, and mTLS transport setup.
