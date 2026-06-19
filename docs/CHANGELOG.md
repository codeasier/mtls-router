# Changelog

[中文](zh-CN/CHANGELOG.md)

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
