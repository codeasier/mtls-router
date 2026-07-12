# Changelog

[中文](zh-CN/CHANGELOG.md)

## Unreleased

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

### Security

- Desktop/manager state, logs, diagnostics, protocol responses, process arguments, and environment variables do not intentionally retain Agent API keys. Agent-owned files and explicitly approved recovery backups remain the persistence boundary and must be protected as sensitive data.
- Documented that the router's shared embedded client private key is extractable from distributed binaries and must be rotated through a complete replacement release plus server-side revocation.
- Uninstall preserves Agent files, sensitive backups, logs, and state. Windows installer integration must remove current-user autostart; macOS/Linux users must run **Prepare for uninstall**, wait for exit, then delete the application.

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
