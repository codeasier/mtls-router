# Issues #48 and #49: Secure Installation and Process Identity

## Scope

This design fixes two independent security problems:

- Issue #48: release installers download an executable over HTTP and install it without integrity verification.
- Issue #49: setup-managed router commands trust a stored PID and may signal an unrelated process after PID reuse.

The changes preserve the existing `setup.sh` and `setup.ps1` command structure. Agent configuration remains separate from router lifecycle management.

## Release Artifacts

The release workflow builds these six router binaries:

- `mtls-router-linux-amd64`
- `mtls-router-linux-arm64`
- `mtls-router-darwin-amd64`
- `mtls-router-darwin-arm64`
- `mtls-router-windows-amd64.exe`
- `mtls-router-windows-arm64.exe`

After all binaries are available, one release job generates a deterministic `SHA256SUMS` file containing exactly one entry for each binary. The same job creates one platform package per binary:

- Linux and macOS packages use `.tar.gz`.
- Windows packages use `.zip`.

Each package contains only:

- The setup script appropriate for the platform.
- The platform's router binary.
- The complete release `SHA256SUMS` file.

The workflow publishes the six standalone binaries, `SHA256SUMS`, and the six platform packages to the GitHub Release. The package is the recommended installation entry point; standalone binaries remain available for service managers, containers, and manual deployments.

Release installers no longer receive an HTTP default download URL. The release workflow validates that installer placeholders have their expected values and that `setup.ps1` retains its UTF-8 BOM.

## Local Payload Installation

`router install` first resolves the setup script's physical directory rather than using the caller's current working directory. It derives the expected asset name from the current OS and architecture.

If the expected binary exists beside the setup script, installation follows this sequence:

1. Require a sibling `SHA256SUMS` file.
2. Find exactly one well-formed checksum entry for the expected asset.
3. Compute the local binary's SHA-256 digest using a supported platform facility.
4. Compare digests without case sensitivity.
5. Copy the verified binary to a temporary file in the installation directory.
6. Set executable permissions on Unix.
7. Atomically replace the installed binary.

Missing checksums, duplicate entries, malformed digests, unsupported hash tools, or digest mismatch are hard failures. If a local payload exists but fails verification, setup does not offer a network fallback. This prevents a corrupted or tampered package from being silently bypassed.

The currently installed binary is not modified until verification and temporary-file preparation succeed.

## HTTPS Fallback

Network fallback is considered only when the expected sibling binary does not exist.

In an interactive terminal, setup asks whether it may download the binary and checksum over HTTPS. Declining exits without modifying the installed binary.

In a non-interactive environment, setup fails by default. Network access must be explicitly authorized with either:

- `router install --download`
- `MTLS_ROUTER_ALLOW_DOWNLOAD=1`

`--download` authorizes network access; it does not change the source URL. `--download-url` continues to configure a custom source but does not by itself authorize a non-interactive download.

The default source is the matching GitHub Release over HTTPS. A custom download base URL must use `https://`. Setup rejects `http://` and all other URL schemes before making any request. Download credentials are therefore never sent over plaintext HTTP.

Setup downloads both the selected binary and `SHA256SUMS` into a temporary directory. It applies the same strict checksum parsing and verification used for local payloads, then atomically installs the verified binary. A missing or invalid remote checksum is a hard failure.

Temporary files are removed on success and failure. Download, checksum, permission, or replacement failures leave the previously installed binary unchanged.

## Command Compatibility

`router setup` continues to compose router installation, startup, and agent configuration. Its installation phase follows the same local-first rules as `router install`.

`MTLS_ROUTER_SKIP_DOWNLOAD=1` remains available to tests and workflows that already provide an installed binary. It bypasses both local payload installation and network fallback, preserving its current intent.

Existing custom download credentials remain supported only for HTTPS URLs. Help output documents the local package behavior, `--download`, and the non-interactive default.

## Process Identity

The setup-managed state file records these process identity fields in addition to existing metadata:

- `pid`
- `process_started_at`
- `process_executable`

These values describe the spawned child process, not the setup command's observation time or configured binary path.

After startup returns the child PID, setup obtains the process's OS-reported start time and executable path. It resolves the executable path to a normalized physical path where the platform permits it. Startup fails safely and does not write a usable state file if identity capture fails.

Unix obtains identity from platform process metadata:

- Linux uses `/proc/<pid>/stat` start ticks and `/proc/<pid>/exe`.
- macOS uses `ps` for an unambiguous process start value and command/executable information, normalized against the expected installed binary.

PowerShell uses `Get-Process` and records `StartTime.ToUniversalTime()` plus the process executable path.

The representation only needs to be stable for comparisons on the same host and platform. Unix and Windows need equivalent safety guarantees, not identical serialized values.

## Status and Stop Semantics

`router status` and `router stop` load the state and validate all of the following:

1. PID is present and currently exists.
2. Stored process start identity is present and matches the live process.
3. Stored executable identity is present and matches the live process.
4. The executable identity matches the setup-managed binary recorded in state after path normalization.

If all checks pass, the process is the setup-managed router.

If the PID does not exist, status reports not running/stale state. If the PID exists but any identity field is missing or mismatched, status reports stale state and does not report `router running`.

`router stop` sends TERM/normal stop and eventual KILL/forced stop only after complete identity validation. On stale state it sends no signal and leaves the state file available for diagnosis. The user may remove stale state manually or replace it by successfully starting a new setup-managed router.

Old state files without the new identity fields are treated as stale. There is intentionally no PID-only compatibility fallback because that would preserve the vulnerability.

During the graceful-stop wait loop, setup continues checking process identity rather than PID existence alone. Before force-killing, it validates identity again so a rapidly reused PID cannot receive the forced signal.

## Failure Handling

Security-sensitive failures are fail-closed:

- Local payload present but unverifiable: abort without network fallback.
- HTTP or unsupported download URL: abort before credentials or requests are sent.
- Remote checksum missing or invalid: abort without replacing the installed binary.
- Process identity unavailable at startup: do not create trusted state.
- Stored state missing identity: report stale and never signal.
- Live identity changes while stopping: stop waiting and never force-kill the new process.

Messages distinguish `not running`, `stale state`, checksum failure, and network authorization failure so users can recover without weakening validation.

## Testing

Shell tests cover:

- Installing a valid sibling payload from a different working directory.
- Local checksum success, mismatch, missing entry, duplicate entry, and missing file.
- Preserving an existing installed binary on every failure path.
- Missing payload in interactive-decline and non-interactive modes.
- Explicit `--download` HTTPS fallback with mocked downloads.
- Rejecting HTTP before invoking the downloader.
- Requiring and validating remote `SHA256SUMS`.
- Writing process start and executable identity to state.
- Reporting a missing PID as not running/stale.
- Reporting a live unrelated PID and forged state as stale.
- Sending no signal for stale or mismatched identity.
- Successfully reporting and stopping a genuine setup-managed router.
- Revalidating identity before forced termination.

PowerShell tests provide equivalent coverage where `pwsh` is available. Static tests verify the Windows implementation when PowerShell execution is unavailable.

Release tests verify artifact names and contents, deterministic checksum generation, HTTPS-only defaults, exact installer placeholder handling, and preservation of the PowerShell UTF-8 BOM.

The full verification remains:

```bash
go test ./...
go vet ./...
make test-shell
test -z "$(gofmt -l .)"
```

## Non-Goals

- Code signing or notarization is not introduced in this change.
- Setup does not verify a cryptographic signature independent of GitHub HTTPS. `SHA256SUMS` provides package consistency and corruption detection; GitHub HTTPS supplies source authenticity for fallback downloads.
- No self-extracting installer is added.
- No PID-only backward compatibility is retained.
- Agent configuration behavior is not redesigned.
