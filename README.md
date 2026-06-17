# mtls-router

`mtls-router` is a single-binary, cross-platform local reverse proxy. It accepts plain HTTP from local clients such as Claude Code or Codex CLI, then forwards requests to a public upstream mTLS server using a build-time-injected client certificate, private key, upstream CA, and upstream URL.

The proxy streams request bodies and Server-Sent Events responses transparently. It does not perform protocol conversion: local traffic is HTTP, and upstream traffic is HTTPS with mTLS.

## Quick start

```bash
./scripts/build.sh
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

The developer build script injects a placeholder upstream URL, so the resulting binary fails fast during startup until you build with a real upstream URL and certificate material.

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
MTLS_UPSTREAM_URL=https://router.example.com \
MTLS_TLS_MIN=tls1.3 \
./mtls-router -timeout 10s
```

## Build with placeholder certs

For local development:

```bash
./scripts/build.sh
```

The script creates these files if they are missing:

- `secrets/client.pem`
- `secrets/client.key`
- `secrets/upstream-ca.pem`

It then runs `go build -trimpath` and writes `./mtls-router`.

## Build with real certs

```bash
go build -trimpath \
  -ldflags "-s -w \
    -X 'main.clientCertPEM=$(cat secrets/client.pem)' \
    -X 'main.clientKeyPEM=$(cat secrets/client.key)' \
    -X 'main.upstreamCAPEM=$(cat secrets/upstream-ca.pem)' \
    -X 'main.upstreamURL=https://router.example.com'" \
  -o mtls-router .
```

The binary never reads cert files at runtime. Certificate PEM, key PEM, upstream CA PEM, and the default upstream URL are embedded at build time through linker variables:

- `main.clientCertPEM`
- `main.clientKeyPEM`
- `main.upstreamCAPEM`
- `main.upstreamURL`
- `main.version`

## Runtime behavior

At startup, `mtls-router` validates configuration, constructs the mTLS upstream transport, and probes the upstream before binding the local listener. If the probe fails, the process exits non-zero instead of accepting local traffic with broken upstream credentials or routing.

The local listener is plain HTTP on `127.0.0.1:19099` by default. The upstream connection uses the embedded client certificate and upstream CA for mTLS.

## Streaming and SSE

Request body sniffing detects JSON requests containing `"stream": true` without consuming or corrupting the body. Downstream readers still receive the original bytes.

SSE responses preserve streaming behavior and use SSE-safe headers, including:

- `Content-Type: text/event-stream`
- `Cache-Control: no-cache`

## Deployment

Systemd and Docker artifacts are added in the deployment phase. Planned deployment options include:

- systemd: copy the binary to `/usr/local/bin/mtls-router`, install the service unit, then enable and start it with `systemctl`;
- Docker: build a small `FROM scratch` image, expected to remain under 20 MB;
- bare metal: run `./mtls-router` directly.

## Design

See `docs/superpowers/specs/2026-06-17-mtls-router-design.md`.

## License

MIT
