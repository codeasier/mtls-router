# mtls-router

[中文](docs/zh-CN/README.md)

The `mtls-router` binary is a single-binary, cross-platform local reverse proxy. It accepts plain HTTP from local clients such as Claude Code or Codex CLI, then forwards requests to a public upstream mTLS server using an embedded client certificate, private key, upstream CA, and upstream URL. The project also ships a Go manager and a Tauri desktop application.

The proxy streams request bodies and Server-Sent Events responses transparently. It does not perform protocol conversion: local traffic is HTTP, and upstream traffic is HTTPS with mTLS.

## Release notes

See [docs/CHANGELOG.md](docs/CHANGELOG.md) for the full changelog.

### v0.1.1

Adds one-click setup scripts, background mode, log file support, agent configuration wizard, Windows setup improvements, and setup test coverage.

### v0.1.0

Initial release of the single-binary local reverse proxy for forwarding local HTTP traffic to an upstream HTTPS mTLS endpoint.

## Desktop application

The Tauri desktop application provides current-user router control, tray operation, default launch-at-login, health/log views, supervised port-conflict recovery, whole-package online updates, image conversations, and explicit preview/write flows for Claude Code, opencode, and Codex. The image workspace supports the exact `cx/gpt-5.5-image` and `ag/gemini-3.1-flash-image` presets when the authenticated `/v1/models/image` catalog reports them, with prompt-only generation and explicit single-image editing. Port recovery reports structured reasons and supervisor guidance, never elevates or executes stop commands, and periodically samples the port for about 10 seconds to report whether reoccupation was detected. It packages the manager and router as verified sidecars and never changes Agent files on first launch.

The checked-in CI and release workflows build six native desktop packages: Windows x86_64/arm64 NSIS installers, macOS Intel/Apple Silicon DMGs, and Linux x86_64/arm64 AppImages. Stable desktop builds silently check `https://downloads.codeasier.top/mtls-router/latest/latest.json` at startup and also provide a manual check in Settings. An available update is downloaded, signature-verified, installed, and restarted only after user confirmation; it replaces the complete desktop package, including its matched manager and router sidecars. This does not add or change a CLI updater. Each matching target runner inspects its package contents, architecture, version/deployment identity, sidecar hashes, and executable permissions, then constructs the packaged Tauri application to catch initialization failures without entering the event loop. Release jobs sign Windows and macOS packages only when signing credentials are complete, notarize and staple macOS applications only when the additional Apple credentials are complete, and publish one explicit signing-status file per target. Package inspection does not install, perform a normal launch, or exercise a previous-to-next update, so separate successful target-runner evidence is still required before a desktop release is considered fully validated. See [Desktop Application](docs/DESKTOP.md) for installation, updates, first-launch, Agent, credential, and uninstall behavior, [Desktop Troubleshooting](docs/TROUBLESHOOTING.md) for recovery guidance, and [Build and Release](docs/BUILD.md) for the exact evidence boundary.

## One-click setup

Release installation is package-first. From [GitHub Releases](https://github.com/codeasier/mtls-router/releases), select the archive for your operating system and CPU architecture (`.tar.gz` for macOS/Linux or `.zip` for Windows), extract it, and run the setup script from the extracted directory. Each archive contains the setup script, the exact-platform `mtls-router` and `mtls-router-manager` binaries, and `SHA256SUMS` entries for both binaries.

On macOS or Linux:

```bash
tar -xzf mtls-router-darwin-arm64.tar.gz
./setup.sh router setup
```

On Windows PowerShell:

```powershell
Expand-Archive .\mtls-router-windows-amd64.zip -DestinationPath .\mtls-router
.\mtls-router\setup.ps1 router setup
```

The setup script selects both sibling binaries for the current platform and requires one exact valid entry for each in the sibling `SHA256SUMS` manifest. If any packaged platform payload is present, a missing sibling, manifest, malformed/duplicate entry, or hash mismatch is a hard failure: the existing installed pair is preserved and the script never falls back to a network download.

If no sibling payload is present, interactive setup asks whether to download both binaries and `SHA256SUMS`. Non-interactive setup fails closed unless downloading is explicitly authorized with `router install --download`, `router setup --download`, or `MTLS_ROUTER_ALLOW_DOWNLOAD=1`. All three files are downloaded into one temporary directory, and both binaries are SHA-256 verified before either installed path is replaced.

Custom sources set with `--download-url` or `MTLS_ROUTER_DOWNLOAD_URL` must use HTTPS; plain HTTP URLs are rejected before any credentials or downloader are used. Release-packaged scripts may include a preconfigured HTTPS URL for a private host. Authentication remains explicit through `--download-user` / `--download-password` or `MTLS_ROUTER_DOWNLOAD_USER` / `MTLS_ROUTER_DOWNLOAD_PASSWORD`; the scripts do not embed credentials.

The scripts install `mtls-router` and `mtls-router-manager` together under `~/.local/bin` by default. On Windows this resolves to `%USERPROFILE%\.local\bin` (for example `C:\Users\<you>\.local\bin`). To choose another install directory, set `MTLS_ROUTER_INSTALL_DIR` before running the script. Installation uses a private pending marker, previous-generation backup, fixed-path replacements, installed-hash verification, and an atomic committed receipt. Every setup command reconciles an interrupted transaction before executing an installed binary, so a mixed generation is never used. The scripts do not install or launch any agent, and the default setup path does not modify agent configuration.

## Manual download

Download the binary for your platform from GitHub Releases:

```text
https://github.com/codeasier/mtls-router/releases
```

Choose the matching asset:

| Platform | Asset |
|---|---|
| Linux x86_64 | `mtls-router-linux-amd64` |
| Linux arm64 | `mtls-router-linux-arm64` |
| macOS Intel | `mtls-router-darwin-amd64` |
| macOS Apple Silicon | `mtls-router-darwin-arm64` |
| Windows x86_64 | `mtls-router-windows-amd64.exe` |
| Windows arm64 | `mtls-router-windows-arm64.exe` |

On macOS or Linux, make the binary executable and optionally rename it:

```bash
chmod +x ./mtls-router-*
mv ./mtls-router-darwin-arm64 ./mtls-router
```

On Windows, download the `.exe` asset and optionally rename it in PowerShell:

```powershell
Rename-Item .\mtls-router-windows-amd64.exe mtls-router.exe
```

Use the `arm64` asset instead if you are on Windows arm64.

## Run on macOS or Linux

```bash
./mtls-router
```

## Run on Windows

Open PowerShell in the folder containing the downloaded executable, then run:

```powershell
.\mtls-router.exe
```

## Run in the background

`-backend` starts a detached child process and returns control to the shell. The same release binaries support foreground and background mode; no separate background build is required.

On macOS or Linux:

```bash
./mtls-router -backend
```

On Windows PowerShell:

```powershell
.\mtls-router.exe -backend
```

When `-backend` is used without `-log`, logs are written to `mtls-router.log` next to the binary. To choose an explicit log file, pass `-log`:

```bash
./mtls-router -backend -log /tmp/mtls-router.log
```

```powershell
.\mtls-router.exe -backend -log C:\mtls-router\mtls-router.log
```

This mode is convenient for local background use. For production supervision, prefer systemd, Docker, launchd, or a Windows service wrapper so the process can be restarted and managed by the platform.

When started via the one-click setup script, the log file lives outside the install directory:

- macOS / Linux: `~/.mtls-router/mtls-router.log`
- Windows: `%USERPROFILE%\.mtls-router\mtls-router.log`

The exact path is recorded in `setup-state.json` after each start and can be overridden with `MTLS_ROUTER_LOG_PATH`.

Setup-managed `router status` and `router stop` do not trust a PID alone. They validate the PID together with the recorded OS process start identity and executable path, including that the executable matches the managed binary. Missing or mismatched identity is reported as stale state; the state is retained for diagnosis, and `router stop` sends no signal to that process. Identity is checked again while stopping and immediately before any forced termination, preventing a reused PID from being signaled.

State files created by older setup scripts do not contain this process identity. After upgrading the setup script, an older running router is therefore reported as stale and is not stopped automatically. Confirm and stop that process manually, remove the old `setup-state.json`, then run `router start` again to create identity-aware state.

## Management endpoints

`mtls-router` exposes two management endpoints on the same listener as the reverse proxy. They are **not** forwarded to the upstream.

These endpoints assume the router listens on trusted localhost. Do not expose them to the public internet in production, because `/version` includes precise build metadata such as the commit SHA.

### `GET /version`

Returns JSON describing the running binary and process:

```json
{
  "version": "v0.1.1",
  "commit": "abc1234",
  "build_date": "2026-06-21T09:23:24Z",
  "deployment_id": "production-service",
  "management_protocol_version": "4",
  "pid": 12345,
  "started_at": "2026-06-21T09:23:24Z"
}
```

`version`, `commit`, `build_date`, and `deployment_id` are set at link time via `-ldflags -X` in `.github/workflows/release.yml`, `Dockerfile`, and `scripts/build.sh`. `management_protocol_version` is a code-owned compatibility ID. Local builds default to `dev` / `unknown`; production release preflight requires a non-default deployment ID. `started_at` is the time the current process started.

### `GET /health`

Returns 200 with JSON describing upstream mTLS+TCP reachability. The HTTP status is always 200; the body distinguishes `ok` from `degraded`:

```json
{"status": "ok", "upstream": "reachable"}
```

```json
{"status": "degraded", "upstream": "unreachable", "error": "..."}
```

The setup script uses `/version` and `/health` to detect when a previously-installed router is already running on port 19099 and decide whether to upgrade, restart, or leave it alone.

## Configure agents

The setup scripts separate router lifecycle commands from agent configuration commands:

```bash
./setup.sh router install
./setup.sh router start
./setup.sh router setup
./setup.sh agent print-config
./setup.sh agent write-config --agent=claude
```

```powershell
.\setup.ps1 router install
.\setup.ps1 router start
.\setup.ps1 router setup
.\setup.ps1 agent print-config
.\setup.ps1 agent write-config --agent=claude
```

`router install` only downloads and installs the binary. `router start` only starts an already installed binary and fails with a clear message if it is missing. `router setup` installs and starts the router, matching the no-argument default behavior.

`agent print-config` and `agent write-config --agent=...` both read the key without echo before discovering the manager's authenticated, build-filtered `GET /v1/models` catalog. By default the manager excludes valid IDs containing ASCII `/`; releases built with `SIMPLIFY=False` retain them. This immutable manager build policy controls configuration choices and refresh validation; it is not a runtime preference and does not limit the proxy routes listed below. The commands never choose the first model, infer a choice from model names or capabilities, or substitute another model. A release may provide a visible, editable preset, but each Agent section is offered only after its exact model IDs are validated against that filtered catalog; otherwise the complete section is unavailable and no replacement is selected. Per Agent, initialization is `existing > preset > empty`. Print returns manager-rendered, API-key-redacted managed fragments; write shows an exact preview and revalidates the catalog immediately before one transactional write. Add `--model-config=<path>` to supply the key-free canonical JSON choices instead of answering model prompts; this explicit import replaces all generated defaults. The legacy top-level `--print-config` and `--write-config --agent=...` options remain compatibility aliases for the Agent configuration flow introduced in protocol v2. Agent commands only execute a checksum-verified sibling manager or a receipt-verified installed manager; they never download a manager implicitly.

The current receipt-verified manager handshake is:

```json
{"id":"info","method":"manager.info","params":{}}
{"id":"info","result":{"version":"v0.1.1","commit":"abc1234","build_date":"2026-06-21T09:23:24Z","target":"linux/amd64","deployment_id":"production-service","management_protocol_version":"4"}}
```

`MTLS_ROUTER_OPENAI_API_KEY` has been removed because environment variables are an unsafe secret transport. It no longer supplies a key. Noninteractive automation must verify `manager.info` protocol `4`, call `agent.models`, construct canonical model config, then call `agent.render` or `agent.preview` and `agent.write`. The key appears only in the `agent.models` and `agent.write` stdin request bodies. Do not put it in command-line arguments, environment variables, model config, logs, shell history, or temporary request files. See the complete [Agent Model Configuration](docs/AGENT_MODELS.md#protocol-v4-automation) contract.

The `mtls-router` binary itself manages the router only; it does not provide agent configuration commands such as `print-config`.

Normal Agent writes use a preservation merge: the manager changes only its documented paths and retains unrelated supported settings. The setup-script `agent print-config` and `agent write-config` commands are merge-only. They never request destructive recovery, bypass parsing, or retry `CONFIG_INVALID` with a force overwrite.

The desktop additionally offers **Back up and rebuild** for a narrowly eligible syntax-invalid configuration. This is destructive: it replaces the complete approved Agent file set with clean managed-only output and discards unrelated settings, comments, formatting, and valid Codex companion-file metadata. Every existing file in that set is first backed up byte-for-byte beside its source; those backups may contain API keys and must be protected like the original configuration. Rebuild has a separate preview and confirmation, and there is no automatic or global force-overwrite fallback. See [Desktop Application](docs/DESKTOP.md#configure-agents), the exact [Agent recovery contract](docs/AGENT_MODELS.md#destructive-rebuild-recovery), and [recovery troubleshooting](docs/TROUBLESHOOTING.md#invalid-agent-configuration).

- Claude Code merges only its managed `env` keys into `~/.claude/settings.json` (or `$CLAUDE_CONFIG_DIR/settings.json`) and supports primary plus inheritable Haiku, Sonnet, and Opus selections. Fable is optional: when enabled it can inherit primary or explicitly select a model, display name, and Standard/1M context; when absent it is not implicitly added or managed. Enabled Fable renders `ANTHROPIC_DEFAULT_FABLE_MODEL` and, when named, `ANTHROPIC_DEFAULT_FABLE_MODEL_NAME`. Claude preset and existing initialization remain whole-section atomic, so preset Fable is never merged into an existing Claude section. The manager preserves never-owned manual Fable keys while Fable is disabled, removes only stale Fable paths proven previously owned, and requires collision/drift approval before claiming an unowned value. Fable aliases require Claude Code 2.1.170 or newer. Separately, numeric custom-model context overrides work directly for unknown model names on Claude Code 2.1.193 or newer; older versions may ignore those numeric overrides. Every explicit selection may have a display name and optional canonical `context: "1m"`; the manager keeps the authenticated base model ID canonical and appends `[1m]` only when rendering Claude's model environment values. It does not infer 1M capability or fall back if Claude or the upstream rejects it at runtime.
- opencode writes the exact selected catalog subset under `provider.mtls-router` and its owned root default model. With no explicit `OPENCODE_CONFIG`, an existing canonical `~/.config/opencode/opencode.jsonc` is migrated to sibling `opencode.json`; an explicit `.jsonc` override is normalized in place at that exact path. Both operations lose comments and formatting.
- Codex writes the dedicated `[model_providers.mtls-router]` Responses provider and selected typed model settings into `~/.codex/config.toml` (respecting `CODEX_HOME`). Switching shared CLI/IDE authentication to official file-backed API-key mode requires separate preview approval.

Every model retained in the manager catalog is treated as supported for Claude Messages and token counting, opencode Chat Completions, compatibility Completions, and Codex Responses, including streaming. `configured` detection means only that the local managed structure is complete; current authorization is established by discovery and write-time refresh. Optional model settings remain omitted when unset. Discovery failures, stale catalogs, removed models, drift, or invalid ownership state fail closed without static/cached fallback or partial changes. Normal merge preserves unrelated settings; managed drift requires approval, and backups may contain keys and must be protected. See [Agent Model Configuration](docs/AGENT_MODELS.md) for the canonical schema, options, refresh, failures, migration, ownership, recovery, and backup contract.

The setup scripts do not install any agent and do not launch any agent.

The default local listen address is:

```text
127.0.0.1:19099
```

Point local clients at:

```text
http://127.0.0.1:19099/v1
```

## Configuration

Configuration precedence is:

```text
flag > env > build-time > default
```

| Setting | Environment variable | Flag | Default |
|---|---|---|---|
| Listen address | `MTLS_LISTEN_ADDR` | `-listen` | `127.0.0.1:19099` |
| Upstream URL | `MTLS_UPSTREAM_URL` | `-upstream` | build-time `upstreamURL` |
| Minimum TLS version | `MTLS_TLS_MIN` | `-tls-min` | `tls1.2` |
| Non-stream timeout | `MTLS_TIMEOUT` | `-timeout` | `0` means no timeout |
| Debug body logging | `MTLS_DEBUG=1` | `-debug` | off |
| Backend mode | `MTLS_BACKEND` | `-backend` | off |
| Log file | `MTLS_LOG` | `-log` | stderr in foreground; `<binary-dir>/mtls-router.log` in backend mode |

The upstream URL must use HTTPS. Plain HTTP upstreams are rejected because they cannot provide mTLS and would transmit requests without transport encryption.

Additional flags:

| Flag | Description |
|---|---|
| `-backend` | Start a detached background process and return |
| `-log` | Write logs to the specified file |
| `-version` | Print version and exit |
| `-help`, `-h` | Print usage and exit |

Example:

```bash
MTLS_LISTEN_ADDR=127.0.0.1:19099 \
MTLS_TLS_MIN=tls1.3 \
./mtls-router -timeout 10s
```

## Runtime behavior

At startup, `mtls-router` validates configuration, constructs the mTLS upstream transport, and probes the upstream before binding the local listener. If the probe fails, the process exits non-zero instead of accepting local traffic with broken upstream credentials or routing.

The local listener is plain HTTP on `127.0.0.1:19099` by default. The upstream connection uses the embedded client certificate and upstream CA for mTLS.

## Streaming and SSE

Request bodies stream straight through to the upstream — the router does not buffer them. Responses that look like Server-Sent Events get SSE-safe headers, including:

- `Content-Type: text/event-stream`
- `Cache-Control: no-cache`

The reverse proxy is configured with `FlushInterval: -1`, so upstream bytes are flushed to the local client immediately.

## Build from source

Maintainer build and release instructions live in [`docs/BUILD.md`](docs/BUILD.md).

`-backend` and `-log` are runtime flags in the same binary, so the release workflow does not need separate assets or build steps for background mode.

## Deployment

Systemd and Docker artifacts are available for deployment:

- systemd: copy the binary to `/usr/local/bin/mtls-router`, install `systemd/mtls-router.service`, then enable and start it with `systemctl`;
- Docker: build the provided `Dockerfile`, which produces a static binary in a `scratch` image;
- bare metal: run `./mtls-router` directly.

For production-style Windows service hosting, use NSSM instead of `-backend`:

```powershell
nssm install mtls-router
```

In the NSSM service editor, configure:

- Path: the full path to `mtls-router.exe`;
- Startup directory: the directory containing `mtls-router.exe`;
- Arguments: any router flags except `-backend`, such as `-listen`, `-upstream`, or `-log`.

Do not pass `-backend` under NSSM because NSSM manages the background process.

## License

MIT
