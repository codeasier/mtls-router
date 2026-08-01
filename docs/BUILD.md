# Build and Release

[中文](zh-CN/BUILD.md)

This document is for maintainers building the router, Go manager, or Tauri desktop application. The checked-in CI and release workflows build all six native desktop package targets and inspect each package on a matching runner. Exact stable `vX.Y.Z` tag releases publish those desktop packages with signed updater artifacts and the CLI router/manager binaries and archives. Windows/macOS signing and macOS notarization/stapling are conditional on complete platform credentials; Tauri updater signing is separately mandatory for stable releases. Package inspection does not install, launch, or update the application; retain separate signing-status and successful target-runner install/launch/update evidence for every released package.

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

The repository build scripts accept manager-only build environment variable
`SIMPLIFY`. `scripts/build.sh` and `desktop/scripts/build-sidecars.sh` normalize
it before invoking any compiler: unset, empty, or any ASCII-case spelling of
`true` becomes `True`; any ASCII-case spelling of `false` becomes `False`.
Whitespace, numbers, non-ASCII lookalikes, and every other value fail with
`invalid SIMPLIFY value` before compilation. The normalized value is linked
only into `github.com/codeasier/mtls-router/internal/manager/modelcatalog.Simplify`;
the router binary never receives it. `True` is the default and excludes valid
model IDs containing ASCII `/` from the manager catalog; `False` retains every
valid ID. This is an immutable manager build policy, not a runtime setting,
configuration preference, router option, or participant in runtime
configuration precedence.

A direct manager build can explicitly disable the filter:

```bash
go build -trimpath \
  -ldflags "-X 'github.com/codeasier/mtls-router/internal/manager/modelcatalog.Simplify=False'" \
  -o mtls-router-manager ./cmd/mtls-router-manager
```

When building directly, omit this `-X` assignment to use the code default
`True`. Do not link an empty value: assigning an empty string to
`github.com/codeasier/mtls-router/internal/manager/modelcatalog.Simplify` with
`-X` overwrites the code default, and the manager exits at startup with `invalid
embedded simplify value` before protocol serving or Agent transaction recovery.

The manager accepts one optional build-time Agent model preset from
`AGENT_MODEL_PRESET_BASE64`. The value must be strict standard Base64 of a
key-free canonical version-1 model-config document containing at least one Agent
section. `scripts/build.sh` and `desktop/scripts/build-sidecars.sh` inject it
only into `github.com/codeasier/mtls-router/internal/manager/preset.Encoded` in
manager binaries; the router binary never receives the value. Unset or empty
means no preset. A malformed nonempty value makes the manager fail startup
before protocol serving or Agent transaction recovery, without printing the
encoded or decoded content.

`mtls-router-manager` has one command, `serve`. It reads one line-delimited JSON request at a time from stdin, processes requests sequentially, writes only protocol responses to stdout, and exits cleanly on stdin EOF. Diagnostics belong on stderr or in logs. Never add API keys to manager arguments or environment variables.

Agent configuration uses management protocol v4. Release tests validate generated Claude JSON, opencode JSON, and Codex TOML/auth output through repository parser and snapshot coverage. The exact current stable Agent/schema inputs used by those tests, including source URL, revision, digest, and retrieval date, are pinned in [`internal/manager/agent/testdata/compatibility.json`](../internal/manager/agent/testdata/compatibility.json). Updating a pin requires reviewing the upstream schema, updating renderer/schema tests where necessary, and keeping English/Chinese Agent documentation aligned.

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

`internal/version.ManagementProtocolVersion` is the code-owned protocol ID and is currently `4`; it is not an `-X` linker variable. `DeploymentID` is a non-sensitive identifier for the fixed service environment. A production build must use the same non-empty, non-`dev`, non-`unknown` deployment ID and protocol ID in router, manager, and desktop. External-router reuse is intentionally disabled for default development identities.

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

`AGENT_MODEL_PRESET_BASE64` is forwarded only to the packaged manager sidecar.
It is not injected into the router sidecar and is not a desktop runtime setting.
The normalized `SIMPLIFY` value is likewise forwarded only to the packaged
manager sidecar; release builds use the same value for standalone and desktop
managers.

### Layered local desktop development

Choose the shortest loop for the change type. None of these commands bypass sidecar hash checks, manager handshake checks, credential isolation, preview/revision checks, or Agent transactional writes.

| Change type | Command | Notes |
| --- | --- | --- |
| React/UI only | `cd desktop && npm run dev:mock` | Vite + HMR only. Injects an in-memory `DesktopApi` through the existing `App` boundary. Never reads or writes real credentials or Agent config files. Optional scenarios: `?mockScenario=success\|protocol-error\|preview-stale\|write-fail` (or `window.__MTLS_MOCK_SCENARIO__`). Production builds cannot enable mock mode (`DEV && VITE_MOCK=true` only). |
| Rust/Tauri with unchanged sidecars | `cd desktop && npm run dev:tauri:reuse` | Runs `tauri dev` without `sidecars:build`. Fails closed if host-target sidecars are missing and prompts for a full prepare. Runtime still validates embedded hashes and the manager handshake. |
| Real Agent path against disposable dirs | `cd desktop && npm run dev:agent` | Explicitly overrides `MTLS_ROUTER_DESKTOP_DATA_DIR`, `CLAUDE_CONFIG_DIR`, `OPENCODE_CONFIG`, and `CODEX_HOME` to a disposable root (or `MTLS_ROUTER_DEV_AGENT_ROOT`), then wraps reuse. Does **not** isolate the fixed router port `127.0.0.1:19099`; avoid a concurrent daily router instance. |
| Manager/router, presets, or certs | `npm run sidecars:build` then reuse or `npm run tauri -- dev` | Go sidecar byte changes require rebuild so Rust can re-embed SHA-256 values. |
| Installer / release layout | `make desktop-package-current` | Full package path. |

First-time native prepare and full local launch still use:

```bash
cd desktop
DEPLOYMENT_ID=dev VERSION=dev MANAGEMENT_PROTOCOL_VERSION=4 npm run tauri -- dev
```

`npm run tauri` always runs `sidecars:build` first. Prefer `dev:tauri:reuse` once valid host-target sidecars already exist under `src-tauri/binaries/`.

For a native bundle build, set release metadata explicitly:

```bash
DEPLOYMENT_ID=production-service \
VERSION=v0.2.0 \
MANAGEMENT_PROTOCOL_VERSION=4 \
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

### Updater signing and channel

Tauri updater signing and operating-system platform signing serve different trust boundaries and neither replaces the other:

- The Tauri updater signature proves that an online-update artifact was signed by the private key corresponding to the public key embedded in the installed application. Tauri requires this signature before installation on Windows, macOS, and Linux.
- Windows Authenticode and macOS code signing/notarization establish publisher and platform trust for downloaded and installed software. They remain required wherever distribution policy requires them, even when the Tauri updater signature is valid. Linux currently has no configured platform package signature, but its online updater artifact still requires the Tauri signature.

Online updates are produced only for an exact stable `vX.Y.Z` tag. Validation dispatches, prerelease tags, and other refs keep `createUpdaterArtifacts` disabled and do not advance the channel. Stable builds embed the endpoint `https://downloads.codeasier.top/mtls-router/latest/latest.json`. Release assembly publishes a six-platform `latest.json`, each platform's updater artifact and `.sig`, and then atomically advances the mirrored `latest` symlink. Windows and Linux reuse the final NSIS/AppImage package as the updater artifact; macOS additionally publishes a signed `CodeasierRouter-darwin-<arch>.app.tar.gz`. Every updater asset, signature, and `latest.json` is covered by `SHA256SUMS`.

Generate the updater keypair once on a trusted, non-recorded operator workstation from `desktop/`. The command prompts for the password and contains no key or password value:

```bash
npm exec tauri -- signer generate -w /secure/offline/CodeasierRouter-updater.key
```

Do not pass `--password`, put the password or private key in an environment variable, enable verbose shell tracing, record the terminal, or paste generated key material into a command, log, issue, or repository file. Protect and back up the generated private-key file and its password separately; preserve the generated companion public key for configuration. Losing the private key prevents publishing updates trusted by installed applications. Rotating the public key also requires an explicit migration or manual-reinstall plan because existing applications trust the public key embedded when they were built.

In the repository's GitHub **Settings > Secrets and variables > Actions** UI, create these repository Secrets through the protected operator process, pasting values only into the secret-value fields:

- `TAURI_SIGNING_PRIVATE_KEY`: complete generated private-key contents.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: the password entered at generation.
- `TAURI_UPDATER_PUBKEY`: complete generated companion public-key contents. The public key is not confidential, but this workflow deliberately sources it from the protected Secrets interface and stable release preflight requires it.

Also create repository variable `TAURI_UPDATER_PUBKEY_SHA256` with the canonical public-key fingerprint printed by `node ./scripts/updater-public-key-fingerprint.mjs /secure/offline/CodeasierRouter-updater.key.pub`. This non-secret pin makes an accidental public-key replacement fail before building. Changing it is a deliberate key-rotation event and still requires the migration plan described above.

Do not use command-line secret setters for the three Secrets: command arguments, shell interpolation, redirected ad hoc files, debug output, and captured CI logs are not approved secret transport. A stable release fails before packaging if any updater key input, the pinned fingerprint, or the HTTPS endpoint is absent or inconsistent. Each native package check also verifies the generated updater signature against the embedded public key before uploading artifacts; updater configuration is written to a private runner-temporary config and key contents must never be printed.

## Package verification

Both workflows invoke `desktop/scripts/verify-package.sh` for every one of the six packages on a native matching runner. The script rejects a host/target mismatch; unpacks the NSIS, DMG, or AppImage; checks package/version identity; checks desktop, manager, and router formats and architectures; compares packaged sidecar hashes with the sidecars built for that job; checks executable permissions on macOS/Linux; and validates manager version, target, deployment ID, and protocol. The release workflow then verifies each generated `.sha256` before publication.

That automated inspection does not install or launch the packaged application. Before publication, retain the workflow inspection outputs and separate evidence from every matching target runner for the full release checklist:

1. Confirm package and executable architecture match the target.
2. Inspect package contents for exactly one architecture-compatible manager and router sidecar and no raw PEM/key files.
3. Confirm executable permissions on macOS/Linux and current-user installation/launch without elevation.
4. Recompute and compare packaged sidecar SHA-256 values with the values embedded in the desktop build.
5. Run `manager.info` and router `/version`; require matching version, non-default deployment ID, and management protocol `4` across desktop, manager, router, setup metadata, and release artifact metadata. Reject every mixed protocol combination before key-bearing Agent requests.
6. Verify Windows signature status or macOS code signature, notarization, and stapling with native platform tools; explicitly record absent status.
7. Install and launch, validate first launch, second-instance activation, sidecar failure behavior, tray/close/quit, default autostart, external reuse, unknown port conflict, Agent preview/write/rollback, logs, and uninstall preparation/cleanup.
8. Confirm Windows uninstall removes current-user autostart. Confirm macOS/Linux **Prepare for uninstall** removes autostart and exits before deletion.
9. Confirm uninstall does not remove or rewrite Agent files, sensitive backups, logs, or state.
10. Scan source, logs, diagnostics, package contents outside the router, and published checksums for accidental API keys or credential files.
11. On every real Windows x86_64/arm64, macOS Intel/Apple Silicon, and Linux x86_64/arm64 target, install the previous stable package in a supported writable location and update it to the candidate through a controlled real feed. Confirm the startup and manual checks, explicit confirmation, signature verification, download/install/restart, candidate desktop/manager/router versions, router ownership behavior, and recovery from a deliberately unsupported or non-writable installation location. Mock UI, package inspection, and a fresh candidate install do not satisfy this previous-to-next test.

Do not advance the stable update channel if any target lacks package inspection, signing status, successful install/launch evidence, or the previous-to-next real-platform update evidence. Workflow configuration, an uploaded artifact, a local Tauri build, mock updater behavior, and package inspection alone are not runtime evidence.

## Native port-recovery acceptance

The CI and release target runners execute `go test ./internal/manager/occupant -count=1` natively on Windows, macOS, and Linux. The Windows helper is a child of the test process under the same account and integrity level; it proves same-privilege native inspection, non-destructive terminate-access preflight, exact helper termination, and port release only. It is not cross-privilege, other-user, Service, PPL, root, restricted-procfs, systemd, or launchd proof. CI must not elevate itself or create platform services for these tests.

Retain the following manual evidence from controlled, disposable hosts before release. Record the inspection JSON with confirmation tokens redacted, the rendered action, and the final status or observation. A blocked inspection must omit both `confirmation_token` and `expires_at`; a forceable inspection must include both.

### Windows manual matrix

| Case | Controlled setup | Required evidence |
| --- | --- | --- |
| Same-account high integrity | Bind the port from a process launched elevated under the desktop user's account; run the desktop normally. | `manual_stop_required` / `insufficient_privilege`, no token or force action; stopping it from the matching high-integrity context releases the port. |
| Other user | Bind from a second local account in an interactive session. | `manual_stop_required` / `different_user`, no token, no PID-only downgrade, and no disclosed process metadata. |
| One Service | Run one disposable Windows Service whose service process owns the listener. | `manual_stop_required` / `service_managed`, `windows_service` / `system`, the exact service name, no token, and a safely quoted `sc.exe` command for manual use in Administrator PowerShell only, not `cmd.exe`. |
| Shared service host | Host two disposable Services in the exact listener process. | All service names are sorted and shown; no token or process-force action is offered, and guidance stops Services rather than the shared host PID. |
| Unreadable SID or process identity | Use an approved disposable same-session fixture that withholds the SID or complete process identity while retaining an exact unique listener PID and terminate access. | `windows_pid_only`, token and expiry present only after terminate-access preflight; a known different SID instead remains `manual_stop_required` / `different_user`. Success proves the termination request succeeded and the original listener PID disappeared from the port, not independent full-process exit. |
| SCM auto recovery | Configure delayed SCM recovery for the disposable listener Service, then terminate it from an authorized external test console. | The Service is blocked before termination; after SCM restarts it, the conflict appears again with the same Service diagnostic. Do not count this externally induced restart as app release-observation proof because the app correctly issued no force token. |
| PPL or System | Use an approved disposable protected-process fixture, or a System-owned listener, without weakening host protections. | No token or force action. Accept `insufficient_privilege` when terminate-access preflight is denied; accept `identity_unavailable` or `protected_process` when another identity/protection boundary blocks recovery. Record which boundary produced the result. |

### Linux manual matrix

| Case | Controlled setup | Required evidence |
| --- | --- | --- |
| User service | Bind from a disposable `systemd --user` `.service`. | `manual_stop_required` / `service_managed`, `systemd_user` / `user`, exact unit name, no token, and user-service stop guidance. |
| System service | Bind from a disposable system `.service`. | `manual_stop_required` / `service_managed`, `systemd_system` / `system`, exact unit name, no token, and system-service stop guidance. |
| `Restart=on-failure` | Add a delayed restart policy to the disposable unit and kill it from an authorized external console. | The unit is blocked before the kill and the conflict returns after restart with the same unit diagnostic. This external kill is not app release-observation proof because a classified unit is never forceable. |
| Root owner | Bind from an ordinary root-owned process that is not the managed router. | `manual_stop_required` / `different_user`, no token or force action. |
| Restricted procfs | Mount or configure a disposable environment so listener identity files are unreadable to the desktop user. | `unavailable` / `identity_unavailable`, no token, and no partial identity is treated as forceable. |
| Custom slice | Place a system unit below valid custom `.slice` ancestors, such as `codeasier.slice/router.slice`. | The exact `.service` is still classified as `systemd_system`; valid custom slice ancestry does not hide supervision. |
| Delegated service cgroup | Move a child below the owning `.service` cgroup while retaining valid delegated descendants. | The owning unit is still classified with the correct user/system scope; delegated children do not become ordinary forceable processes. |

### macOS manual matrix

| Case | Controlled setup | Required evidence |
| --- | --- | --- |
| Current user | Bind from an ordinary current-user process with no supervisor. | `force_terminate`, token and expiry present, verified original process identity absent and initial release, followed by `released` after sampled checks over about 10 seconds detect no reoccupation. |
| Other user | Bind from a second local account. | `manual_stop_required` / `different_user`, no token or force action, and no disclosed process metadata. |
| launchd `KeepAlive` | Bind from a disposable current-user launch agent configured for delayed restart. | The ordinary same-user process is forceable without a guessed launchd label; after verified process absence and initial release, a replacement PID seen by a sampled check produces `reoccupied` during the observation window. |

## Release workflow

The current `.github/workflows/release.yml` builds router and manager binaries for all six Go targets and creates six platform archives containing the exact router/manager pair and setup script. In parallel, six native runners build and inspect Windows x86_64/arm64 NSIS installers, macOS Intel/Apple Silicon DMGs, and Linux x86_64/arm64 AppImages. A manual dispatch is validation-only and may select one paired CLI/desktop target plus an optional HTTPS upstream override; it does not produce updater artifacts. An exact stable version tag always ignores validation overrides, waits for all 12 build jobs, verifies the six desktop package checksums and signed updater pairs, assembles one `SHA256SUMS` and `latest.json`, publishes and mirrors the CLI and desktop assets plus six signing-status files, and atomically advances the `latest` updater channel. No release workflow change adds self-update behavior to the standalone CLI router, manager, archives, or setup scripts.

Production CLI and desktop sidecars require repository secrets `CLIENT_CERT_PEM`, `CLIENT_KEY_PEM`, and `UPSTREAM_CA_PEM`, plus variables `UPSTREAM_URL` and a non-default `DEPLOYMENT_ID`. Stable desktop updater publication additionally requires `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, and `TAURI_UPDATER_PUBKEY`, plus the pinned repository variable `TAURI_UPDATER_PUBKEY_SHA256`. Optional repository variable `AGENT_MODEL_PRESET_BASE64` supplies the same preset to every standalone manager and desktop manager sidecar; empty is valid and means no preset. Release preflight validates a configured value through the manager loader before matrix builds without printing its contents. Optional repository variable `SIMPLIFY` follows the normalization rules above and defaults to `True` when unset or empty. It is normalized before matrix fan-out and propagated to every standalone and desktop manager as the same canonical value; the desktop build script may idempotently validate and normalize it again. Router builds never receive either manager-only value. Optional platform credentials select the signed/notarized release branches described above; unlike those optional credentials, all updater-key inputs are mandatory for an exact stable tag.

Each CLI and desktop matrix producer emits code-owned protocol metadata. `scripts/package-release.sh` requires exactly one metadata file per producer and requires every file to declare schema `1` and management protocol `4` before it assembles archives. This preflight is shared by normal and recovery publication, so a valid but mixed-protocol artifact set is not publishable.

Set release inputs with `gh`:

```bash
gh secret set CLIENT_CERT_PEM --repo codeasier/mtls-router < secrets/client.pem
gh secret set CLIENT_KEY_PEM --repo codeasier/mtls-router < secrets/client.key
gh secret set UPSTREAM_CA_PEM --repo codeasier/mtls-router < secrets/upstream-ca.pem
gh variable set UPSTREAM_URL --repo codeasier/mtls-router --body "https://router.example.com"
gh variable set DEPLOYMENT_ID --repo codeasier/mtls-router --body "production-service"
gh variable set SIMPLIFY --repo codeasier/mtls-router --body "False"
```

Omit or clear the `SIMPLIFY` repository variable for the default `True` policy.
It is not a user preference and cannot be changed after the managers are built.

Set `AGENT_MODEL_PRESET_BASE64` only through the protected repository-variable
process after producing strict standard Base64 from the reviewed key-free
canonical document. Do not place API keys, URLs, provider identities, headers,
catalog responses, or arbitrary Agent settings in a preset.

Configure the optional Windows signing and Apple signing/notarization secrets only through the repository's protected secret-management process. Do not put credential values in documentation, commands that persist in shell history, or repository files.

Run a validation-only Windows amd64 build with `gh`:

```bash
gh workflow run release.yml \
  --repo codeasier/mtls-router \
  --ref main \
  -f version=0.2.0-windows-test.1 \
  -f target=windows-amd64 \
  -f upstream_url=https://router.example.com
```

The selected target produces both `mtls-router-cli-windows-amd64` and `CodeasierRouter-desktop-windows-amd64`. Omit `upstream_url` to use the repository `UPSTREAM_URL`; omit `target` to use `all`. Workflow inputs are visible in GitHub Actions metadata, so the override must not contain credentials, tokens, or sensitive query parameters. It must also be compatible with the client certificate and upstream CA stored in repository Secrets.

Publish the CLI and desktop release by pushing a version tag only after the required target-platform launch evidence has been reviewed:

```bash
git tag v0.2.0
git push origin v0.2.0
```
