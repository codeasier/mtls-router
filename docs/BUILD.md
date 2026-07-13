# Build and Release

[中文](zh-CN/BUILD.md)

This document is for maintainers building the router, Go manager, or Tauri desktop application. The checked-in CI and release workflows build all six native desktop package targets and inspect each package on a matching runner. Tag-triggered releases publish those desktop packages with the CLI router/manager binaries and archives. Windows/macOS signing and macOS notarization/stapling are conditional on complete credentials, and package inspection does not install or launch the application; retain separate signing-status and successful target-runner install/launch evidence for every released package.

## Toolchains and lockfiles

- Go: `go.mod` requires Go `1.26.2`.
- Node.js: `desktop/package.json` requires Node.js `>=22.12.0` and declares `npm@11.6.2`; use `npm ci` with `desktop/package-lock.json`.
- Rust: the desktop crate declares `rust-version = 1.77.2`; use a Rust toolchain that satisfies it and build with `desktop/src-tauri/Cargo.lock` via `--locked`.
- Tauri: JavaScript and Rust Tauri dependencies are exact-version pinned in `desktop/package.json` and `desktop/src-tauri/Cargo.toml` and resolved by their lockfiles. Do not pass `--ignore-version-mismatches` during release builds.

Platform builds also need the Tauri 2 operating-system prerequisites: a supported WebView and native package tools, Rust target support, Go cross-compilation support, `openssl`, and standard archive/checksum tools. Produce and launch-test desktop packages on the target operating system rather than assuming a cross-compiled bundle is installable.

## Go checks and builds

Run the repository checks from the repository root:

```bash
test -z "$(gofmt -l .)"
go test ./...
go vet ./...
make test-shell
```

Build both Go programs for local development:

```bash
go build -trimpath -o mtls-router .
go build -trimpath -o mtls-router-manager ./cmd/mtls-router-manager
```

`mtls-router-manager` has one command, `serve`. It reads one line-delimited JSON request at a time from stdin, processes requests sequentially, writes only protocol responses to stdout, and exits cleanly on stdin EOF. Diagnostics belong on stderr or in logs. Never add API keys to manager arguments or environment variables.

## Local placeholder router

For local router development, run:

```bash
./scripts/build.sh
```

If none of these files exists, the script generates all three placeholders:

- `secrets/client.pem`
- `secrets/client.key`
- `secrets/upstream-ca.pem`

A partial set is rejected. The generated placeholder binary is expected to fail fast until built with real upstream configuration and certificate material. Never publish placeholder binaries.

## Production metadata and credentials

The router does not read certificates at runtime. Link these router-only values into `main`:

- `main.clientCertPEM`
- `main.clientKeyPEM`
- `main.upstreamCAPEM`
- `main.upstreamURL`

Link these shared metadata variables into both the router and manager:

- `github.com/codeasier/mtls-router/internal/version.Version`
- `github.com/codeasier/mtls-router/internal/version.Commit`
- `github.com/codeasier/mtls-router/internal/version.BuildDate`
- `github.com/codeasier/mtls-router/internal/version.DeploymentID`

`internal/version.ManagementProtocolVersion` is the code-owned protocol ID and is currently `1`; it is not an `-X` linker variable. `DeploymentID` is a non-sensitive identifier for the fixed service environment. A production build must use the same non-empty, non-`dev`, non-`unknown` deployment ID and protocol ID in router, manager, and desktop. External-router reuse is intentionally disabled for default development identities.

Example router build:

```bash
go build -trimpath \
  -ldflags "-s -w \
    -X 'main.clientCertPEM=$(cat secrets/client.pem)' \
    -X 'main.clientKeyPEM=$(cat secrets/client.key)' \
    -X 'main.upstreamCAPEM=$(cat secrets/upstream-ca.pem)' \
    -X 'main.upstreamURL=https://router.example.com' \
    -X 'github.com/codeasier/mtls-router/internal/version.Version=v0.2.0' \
    -X 'github.com/codeasier/mtls-router/internal/version.Commit=$(git rev-parse --short=12 HEAD)' \
    -X 'github.com/codeasier/mtls-router/internal/version.BuildDate=$(date -u +%Y-%m-%dT%H:%M:%SZ)' \
    -X 'github.com/codeasier/mtls-router/internal/version.DeploymentID=production-service'" \
  -o mtls-router .
```

The embedded client private key is a shared credential and is extractable by anyone who receives a router binary or desktop package. Release only to trusted internal users. Rotation requires a replacement release with new credential material and server-side revocation of the old credential; the desktop has no runtime credential import or sidecar updater.

## Desktop checks

Install locked Node dependencies and run the complete desktop verification from `desktop/`:

```bash
npm ci
npm run sidecars:build
npm run verify
```

The Rust build script requires native sidecars even for tests, so build them before `verify` in a fresh checkout. The sidecar command uses the current Rust host target unless `TAURI_ENV_TARGET_TRIPLE` or `TARGET` selects one of the supported triples below.

The individual pinned commands are:

```bash
npm run static:check
npm run typecheck
npm test
npm run build
npm run rust:format
npm run rust:test
```

`npm run rust:test` expands to `cargo test --manifest-path src-tauri/Cargo.toml --locked`. Use `cargo build --manifest-path src-tauri/Cargo.toml --locked` for an unbundled Rust build.

## Desktop sidecars

Tauri `bundle.externalBin` contains `binaries/mtls-router-manager` and `binaries/mtls-router`. Before Tauri runs, `npm run tauri` invokes `desktop/scripts/build-sidecars.sh`. It maps the Tauri/Rust target triple to Go OS/architecture, builds both sidecars, and writes the names Tauri requires:

```text
src-tauri/binaries/mtls-router-manager-<target-triple>[.exe]
src-tauri/binaries/mtls-router-<target-triple>[.exe]
```

Supported mappings are:

| Release target | Rust/Tauri target triple | Go target |
|---|---|---|
| Windows x86_64 | `x86_64-pc-windows-msvc` | `windows/amd64` |
| Windows arm64 | `aarch64-pc-windows-msvc` | `windows/arm64` |
| macOS Intel | `x86_64-apple-darwin` | `darwin/amd64` |
| macOS Apple Silicon | `aarch64-apple-darwin` | `darwin/arm64` |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `linux/amd64` |
| Linux arm64 | `aarch64-unknown-linux-gnu` | `linux/arm64` |

The sidecar script uses all three real files in `secrets/` when present; if none is present, it generates temporary placeholders. A production package must therefore fail preflight unless all real credential inputs are deliberately supplied. The Rust build script rejects missing, non-native, wrong-format, wrong-architecture, or non-executable sidecars and embeds each sidecar SHA-256. Runtime startup rechecks files against those hashes and performs a manager target/version/deployment/protocol handshake.

For a local native development launch:

```bash
DEPLOYMENT_ID=dev VERSION=dev MANAGEMENT_PROTOCOL_VERSION=1 npm run tauri -- dev
```

For a native bundle build, set release metadata explicitly:

```bash
DEPLOYMENT_ID=production-service \
VERSION=v0.2.0 \
MANAGEMENT_PROTOCOL_VERSION=1 \
npm run tauri -- build --target aarch64-apple-darwin
```

Use the target triple for the runner from the table. `npm run tauri -- build` runs the frontend production build and Tauri bundling; `bundle.targets` is currently `all`. The intended initial deliverables are current-user Windows installers, macOS applications/DMGs, and Linux AppImages, but a package is not releasable merely because Tauri emitted it.

## Signing and notarization

The release workflow implements conditional platform signing and status verification:

- Windows packages remain unsigned when either `WINDOWS_CERTIFICATE` or `WINDOWS_CERTIFICATE_PASSWORD` is unavailable. When both are present, the workflow signs the two sidecars, desktop executable, and NSIS installer, then validates the installer and all three packaged executables with Authenticode.
- macOS packages remain unsigned when either `APPLE_CERTIFICATE` or `APPLE_CERTIFICATE_PASSWORD` is unavailable. When both are present, the workflow signs the sidecars, application executable, application bundle, and DMG, then verifies the signatures.
- A signed macOS application is notarized and stapled only when `APPLE_ID`, `APPLE_PASSWORD`, and `APPLE_TEAM_ID` are also all present. The workflow validates Gatekeeper assessment and the stapled ticket when these steps run.
- Linux package signing is not configured.
- Every desktop target emits `signing-status-<os>-<arch>.txt`, explicitly reporting unsigned, signed, or signed-and-notarized status and why a stronger status was unavailable.
- Never infer status from a successful Tauri build, filename, CI job name, or presence of a certificate variable.

Local packages are unsigned unless a separate verified signing process signs them. CI intentionally uses `--no-sign` for package validation, while the release workflow selects signed or unsigned branches according to credential availability and records the result. Production distribution must remain blocked where organizational policy requires signing/notarization and the corresponding status file does not prove it.

## Package verification

Both workflows invoke `desktop/scripts/verify-package.sh` for every one of the six packages on a native matching runner. The script rejects a host/target mismatch; unpacks the NSIS, DMG, or AppImage; checks package/version identity; checks desktop, manager, and router formats and architectures; compares packaged sidecar hashes with the sidecars built for that job; checks executable permissions on macOS/Linux; and validates manager version, target, deployment ID, and protocol. The release workflow then verifies each generated `.sha256` before publication.

That automated inspection does not install or launch the packaged application. Before publication, retain the workflow inspection outputs and separate evidence from every matching target runner for the full release checklist:

1. Confirm package and executable architecture match the target.
2. Inspect package contents for exactly one architecture-compatible manager and router sidecar and no raw PEM/key files.
3. Confirm executable permissions on macOS/Linux and current-user installation/launch without elevation.
4. Recompute and compare packaged sidecar SHA-256 values with the values embedded in the desktop build.
5. Run `manager.info` and router `/version`; require matching version, non-default deployment ID, and management protocol `1` across desktop, manager, and router.
6. Verify Windows signature status or macOS code signature, notarization, and stapling with native platform tools; explicitly record absent status.
7. Install and launch, validate first launch, second-instance activation, sidecar failure behavior, tray/close/quit, default autostart, external reuse, unknown port conflict, Agent preview/write/rollback, logs, and uninstall preparation/cleanup.
8. Confirm Windows uninstall removes current-user autostart. Confirm macOS/Linux **Prepare for uninstall** removes autostart and exits before deletion.
9. Confirm uninstall does not remove or rewrite Agent files, sensitive backups, logs, or state.
10. Scan source, logs, diagnostics, package contents outside the router, and published checksums for accidental API keys or credential files.

Do not publish if any target lacks package-inspection, signing-status, and successful install/launch evidence. Workflow configuration, an uploaded artifact, a local Tauri build, and package inspection alone are not launch evidence.

## Release workflow

The current `.github/workflows/release.yml` builds router and manager binaries for all six Go targets and creates six platform archives containing the exact router/manager pair and setup script. In parallel, six native runners build and inspect Windows x86_64/arm64 NSIS installers, macOS Intel/Apple Silicon DMGs, and Linux x86_64/arm64 AppImages. A manual dispatch is validation-only; a version tag waits for all 12 build jobs, verifies the six desktop package checksums, assembles one `SHA256SUMS`, and publishes and mirrors the CLI and desktop assets plus six signing-status files.

Production CLI and desktop sidecars require repository secrets `CLIENT_CERT_PEM`, `CLIENT_KEY_PEM`, and `UPSTREAM_CA_PEM`, plus variables `UPSTREAM_URL` and a non-default `DEPLOYMENT_ID`. Optional platform credentials select the signed/notarized release branches described above.

Set release inputs with `gh`:

```bash
gh secret set CLIENT_CERT_PEM --repo codeasier/mtls-router < secrets/client.pem
gh secret set CLIENT_KEY_PEM --repo codeasier/mtls-router < secrets/client.key
gh secret set UPSTREAM_CA_PEM --repo codeasier/mtls-router < secrets/upstream-ca.pem
gh variable set UPSTREAM_URL --repo codeasier/mtls-router --body "https://router.example.com"
gh variable set DEPLOYMENT_ID --repo codeasier/mtls-router --body "production-service"
```

Configure the optional Windows signing and Apple signing/notarization secrets only through the repository's protected secret-management process. Do not put credential values in documentation, commands that persist in shell history, or repository files.

Publish the CLI and desktop release by pushing a version tag only after the required target-platform launch evidence has been reviewed:

```bash
git tag v0.2.0
git push origin v0.2.0
```
