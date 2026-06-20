# mtls-router

[中文](docs/zh-CN/README.md)

`mtls-router` is a single-binary, cross-platform local reverse proxy. It accepts plain HTTP from local clients such as Claude Code or Codex CLI, then forwards requests to a public upstream mTLS server using an embedded client certificate, private key, upstream CA, and upstream URL.

The proxy streams request bodies and Server-Sent Events responses transparently. It does not perform protocol conversion: local traffic is HTTP, and upstream traffic is HTTPS with mTLS.

## Release notes

See [docs/CHANGELOG.md](docs/CHANGELOG.md) for the full changelog.

### v0.1.1

Adds one-click setup scripts, background mode, log file support, agent configuration wizard, Windows setup improvements, and setup test coverage.

### v0.1.0

Initial release of the single-binary local reverse proxy for forwarding local HTTP traffic to an upstream HTTPS mTLS endpoint.

## One-click setup

These scripts download the latest `mtls-router` binary for your operating system and CPU architecture and start `mtls-router` in backend mode. They do not install or launch any agent, and the default setup path does not modify agent configuration.

On macOS or Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/codeasier/mtls-router/main/setup.sh | bash
```

On Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/codeasier/mtls-router/main/setup.ps1 | iex
```

The scripts install `mtls-router` under `~/.local/bin` by default. To choose another install directory, set `MTLS_ROUTER_INSTALL_DIR` before running the script.

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

- macOS / Linux: `~/.mtls-router/mtls-router-<start-timestamp>.log`
- Windows: `%LOCALAPPDATA%\mtls-router\mtls-router-<start-timestamp>.log`

## Management endpoints

`mtls-router` exposes two management endpoints on the same listener as the reverse proxy. They are **not** forwarded to the upstream.

### `GET /version`

Returns JSON describing the running binary and process:

```json
{
  "version": "v0.1.1",
  "commit": "abc1234",
  "build_date": "2026-06-21T09:23:24Z",
  "pid": 12345,
  "started_at": "2026-06-21T09:23:24Z"
}
```

`version`, `commit`, and `build_date` are set at link time via `-ldflags -X` in `.github/workflows/release.yml`, `Dockerfile`, and `scripts/build.sh`. Local builds default to `dev` / `unknown`. `started_at` is the time the current process started.

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

`agent print-config` only prints configuration snippets. `agent write-config --agent=...` only writes agent configuration and requires an explicit `--agent=` value. The legacy top-level `--print-config` and `--write-config --agent=...` options remain compatibility aliases for the agent commands.

The `mtls-router` binary itself manages the router only; it does not provide agent configuration commands such as `print-config`.

- Claude Code writes the `env` block into `~/.claude/settings.json` (or `$CLAUDE_CONFIG_DIR/settings.json`).
- opencode writes the `mtls-router` provider into the chosen opencode.json (respecting `OPENCODE_CONFIG` and falling back to `~/.config/opencode/opencode.json`).
- Codex CLI writes the `[model_providers.mtls-router]` block and router profiles into `~/.codex/config.toml` (respecting `CODEX_HOME`).

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

Request body sniffing detects JSON requests containing `"stream": true` without consuming or corrupting the body. Downstream readers still receive the original bytes.

SSE responses preserve streaming behavior and use SSE-safe headers, including:

- `Content-Type: text/event-stream`
- `Cache-Control: no-cache`

## Build from source

Maintainer build and release instructions live in `docs/BUILD.md`.

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

## Design

See `docs/superpowers/specs/2026-06-17-mtls-router-design.md`.

## License

MIT
