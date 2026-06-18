# mtls-router

`mtls-router` is a single-binary, cross-platform local reverse proxy. It accepts plain HTTP from local clients such as Claude Code or Codex CLI, then forwards requests to a public upstream mTLS server using an embedded client certificate, private key, upstream CA, and upstream URL.

The proxy streams request bodies and Server-Sent Events responses transparently. It does not perform protocol conversion: local traffic is HTTP, and upstream traffic is HTTPS with mTLS.

## Download

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

On macOS or Linux, make the binary executable:

```bash
chmod +x ./mtls-router-*
```

Optionally rename it:

```bash
mv ./mtls-router-darwin-arm64 ./mtls-router
```

## Run

```bash
./mtls-router
```

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

Additional flags:

| Flag | Description |
|---|---|
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

## Deployment

Systemd and Docker artifacts are available for deployment:

- systemd: copy the binary to `/usr/local/bin/mtls-router`, install `systemd/mtls-router.service`, then enable and start it with `systemctl`;
- Docker: build the provided `Dockerfile`, which produces a static binary in a `scratch` image;
- bare metal: run `./mtls-router` directly.

## Design

See `docs/superpowers/specs/2026-06-17-mtls-router-design.md`.

## License

MIT
