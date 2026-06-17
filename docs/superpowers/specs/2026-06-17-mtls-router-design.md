# mtls-router Design Spec

**Date:** 2026-06-17
**Status:** Draft (pending user review of written spec)
**Author:** Brainstorming session

## 1. Purpose

`mtls-router` is a small, single-binary, cross-platform local reverse proxy whose
sole job is to:

- Accept plain HTTP requests from local clients (e.g. Claude Code, Codex CLI) on
  `127.0.0.1:19099`.
- Re-emit those requests to a public-facing mTLS server (Nginx reverse proxy)
  using a built-in client certificate.
- Stream request and response bodies transparently without protocol conversion.
- Trust only the upstream server whose CA certificate is also baked into the
  binary.

The upstream Nginx is reached over a WireGuard tunnel that connects to a
deployed 9router instance. `mtls-router` itself is **not** involved in
WireGuard configuration; that is handled out-of-band at the OS / network layer.

## 2. Non-Goals

- No protocol transformation (Anthropic ↔ OpenAI, Chat ↔ Responses, etc.).
- No request body rewriting, no model mapping, no retry, no failover.
- No UI, no daemon mode beyond systemd, no config file format beyond env/flags.
- No WireGuard management, no CA issuance, no certificate rotation tooling.

## 3. End-to-End Link

```
Claude Code / Codex CLI
        │  plain HTTP
        ▼
   127.0.0.1:19099  (mtls-router)
        │  mTLS, client cert from build-time injection
        ▼
   router.<placeholder>:443  (public Nginx, mTLS server)
        │  verifies client cert against CA
        ▼
   WireGuard tunnel  (network layer, not in mtls-router's scope)
        ▼
   9router (OpenAI / Anthropic compatible aggregator)
```

## 4. Architecture

### 4.1 Process Model

`mtls-router` is a single Go process with no persistent state.

Startup sequence:

1. Parse configuration. Precedence: CLI flag > env var > build-time ldflags
   value > built-in default. For the upstream URL this means the build-time
   `upstreamURL` is the default, and a runtime `-upstream` flag or
   `MTLS_UPSTREAM_URL` env var overrides it.
2. Load build-time-injected client cert PEM, client key PEM, and upstream CA
   PEM from `ldflags -X` variables.
3. Build an `*http.Transport` configured with the mTLS `tls.Config`.
4. Run a startup mTLS probe: `GET <upstreamURL>/` with a 10s timeout. Any
   non-5xx response (including 401/403/404) is treated as "mTLS channel
   reachable". A 5xx or a transport error causes `log.Fatal` + `os.Exit(1)` so
   systemd / Docker can restart with backoff.
5. Start `http.Server` with the reverse proxy as the handler.
6. On `SIGINT` / `SIGTERM`: `server.Shutdown(ctx)` with a 5s timeout.

### 4.2 Repository Layout

Independent repository at `/Users/test1/liuyekang/dev/code/mtls-router/`.

```
mtls-router/
├── go.mod
├── go.sum
├── main.go                         // ldflags -X injection points
├── internal/
│   ├── config/
│   │   └── config.go               // env + flag parsing
│   ├── certs/
│   │   └── certs.go                // load build-time PEMs
│   ├── proxy/
│   │   ├── transport.go            // mTLS http.Transport
│   │   ├── director.go             // ReverseProxy.Director
│   │   ├── stream.go               // request body stream sniff
│   │   ├── modifyresponse.go       // SSE Content-Type enforcement
│   │   └── errorhandler.go         // ReverseProxy.ErrorHandler
│   ├── health/
│   │   └── probe.go                // startup mTLS probe
│   └── log/
│       └── log.go                  // access log + debug
├── test/
│   ├── transport_test.go
│   ├── stream_test.go
│   ├── director_test.go
│   ├── modifyresponse_test.go
│   └── errorhandler_test.go
├── .github/
│   └── workflows/
│       └── release.yml             // 6-platform matrix
├── README.md
├── LICENSE
└── scripts/
    └── build.sh                    // local dev build
```

### 4.3 Module Responsibilities

#### 4.3.1 `main.go` — Injection Points

```go
var (
    clientCertPEM string // -X main.clientCertPEM=...
    clientKeyPEM  string // -X main.clientKeyPEM=...
    upstreamCAPEM string // -X main.upstreamCAPEM=...
    upstreamURL   string // -X main.upstreamURL=https://...
    version       = "dev"
)
```

`certs` package reads these string constants (defined in `main.go`) and parses
them once at startup with `tls.X509KeyPair(certPEM, keyPEM)` and
`x509.NewCertPool().AppendCertsFromPEM`. **The runtime never reads certificate
files from disk**; everything is compiled into the binary via `-ldflags -X`.
This is a deliberate security choice — see §11.

#### 4.3.2 `internal/proxy/transport.go`

- `tls.Config.Certificates = []tls.Certificate{clientCert}`
- `tls.Config.RootCAs = upstreamCAPool`
- `tls.Config.MinVersion = tls.VersionTLS12` (default; configurable via
  `MTLS_TLS_MIN=tls1.3`)
- `http.Transport.TLSClientConfig` set to the above `tls.Config`
- `IdleConnTimeout = 90s`, `MaxIdleConnsPerHost = 100`
- One shared `*http.Transport` per process for connection reuse.

#### 4.3.3 `internal/proxy/director.go`

Resolves the upstream URL once at startup. For every request:

- `req.URL.Scheme = "https"`
- `req.URL.Host = <upstreamHost>`
- `req.Host = <upstreamHost>` (rewrites the Host header; this is the Go standard
  library's behavior when assigning `req.Host`)
- `req.URL.Path` and `req.URL.RawQuery` are left untouched.
- All other request headers are passed through unchanged.

SNI is the original client `Host` header, preserved by virtue of `req.Host`
being set to the upstream host name. The TLS handshake negotiates with the
upstream Nginx using the configured CA to verify the server certificate.

#### 4.3.4 `internal/proxy/stream.go` — Request Body Sniff

- Use `bufio.NewReader` to peek up to 4096 bytes of the request body.
- Substring search the peeked bytes for `"stream":true` (case-insensitive,
  whitespace-tolerant).
- If found: **mark the request as streaming** by stashing a flag on the
  `http.Request` context (e.g. `context.WithValue(req.Context(), streamKey{}, true)`).
  This flag is read by the response-writing side to force flushes.
- If not found: passthrough. The request body is still streamed through
  `req.Body`; no buffering.

**Design note:** `httputil.ReverseProxy.FlushInterval` is an instance-level
field and cannot vary per request. Therefore `mtls-router` uses
`FlushInterval = -1` (immediate flush on every chunk) **unconditionally** for
all responses, and uses the request-side stream flag only to:

1. Skip unnecessary peeking on responses that will obviously be small JSON
   bodies (an optional optimization, off by default).
2. Allow `ModifyResponse` to know whether to enforce the SSE headers (see
   §4.3.5) without re-parsing the body.

The unconditional `FlushInterval = -1` has negligible cost for non-stream
responses (one extra flush at end-of-body) and guarantees that streaming
responses are never buffered into invisibility. This is the simplest correct
choice.

The peeked reader remains usable; no body bytes are lost. The peek is a single
4 KB allocation, with < 1 ms latency overhead even on large messages.

#### 4.3.5 `internal/proxy/modifyresponse.go`

A `ModifyResponse` hook attached to the ReverseProxy. The hook inspects the
upstream response and conditionally rewrites two headers:

- **Content-Type enforcement**: If the request context flag set by §4.3.4
  indicates a streaming request (`stream:true` was sniffed), **or** the
  upstream `Content-Type` already contains `text/event-stream`, force:
  - `Content-Type: text/event-stream`
  - `Cache-Control: no-cache`
- **Otherwise**: passthrough all response headers unchanged.

The ReverseProxy is configured with `FlushInterval = -1` (immediate flush on
every chunk) **unconditionally** — see §4.3.4 for rationale. This guarantees
that SSE events reach the local client with first-byte latency dominated only
by the upstream network RTT.

#### 4.3.6 `internal/proxy/errorhandler.go`

Called by the ReverseProxy when the upstream transport round-trip itself
fails (mTLS handshake failure, TCP reset, timeout, DNS error, etc.):

The error handler distinguishes three failure modes by inspecting the error
type returned by the transport:

| Underlying error | HTTP status | Body |
|------------------|-------------|------|
| `crypto/tls` handshake error (cert verify, bad cert, etc.) | 502 | `{"error":{"message":"upstream mTLS handshake failed","type":"proxy_error"}}` |
| `context.DeadlineExceeded` or net.Error.Timeout() | 504 | `{"error":{"message":"upstream timeout","type":"proxy_error"}}` |
| Any other transport error (refused, reset, DNS, etc.) | 502 | `{"error":{"message":"upstream unreachable","type":"proxy_error"}}` |

HTTP responses received from the upstream (including 4xx and 5xx) are passed
through unchanged. `mtls-router` does not interpret upstream application
errors.

#### 4.3.7 `internal/health/probe.go`

Startup probe:

- `client := &http.Client{Transport: mTLS transport, Timeout: 10s}`
- `GET <upstreamURL>/`
- Treat any non-5xx response (including 401/403/404) as success — these prove
  the mTLS channel is reachable, the client cert is accepted, and the server
  cert verifies against the bundled CA.
- A 5xx response or transport error logs the failure and calls `os.Exit(1)`.

#### 4.3.8 `internal/log/log.go`

Single access-log line per request:

```
[2026-06-17T10:23:45Z] INFO  POST /v1/messages 127.0.0.1:54321 -> 200 12345B 234ms
[2026-06-17T10:23:46Z] ERROR POST /v1/messages 127.0.0.1:54321 -> 502 0B 0ms upstream mTLS handshake failed
```

Fields: timestamp, level, method, path, client address, upstream status, body
bytes returned, latency, optional short error reason.

- Default: never log request or response body.
- `MTLS_DEBUG=1`: log full request/response bodies. This is a debug-only mode
  and will be very slow on streaming responses.

## 5. Data Flow

### 5.1 Request Path

```
Client
  POST /v1/messages HTTP/1.1
  Host: 127.0.0.1:19099
  Content-Type: application/json
  Authorization: Bearer xxx
  {"model":"claude-opus-4","messages":[...],"stream":true}
        │
        ▼
mtls-router: http.Server
  1. Accept connection.
  2. Handler = ReverseProxy (Director + ModifyResponse + ErrorHandler).
  3. Pre-handler: sniff request body for "stream":true → mark stream mode.
  4. Record start time, body bytes counter.
        │
        ▼
Director(req)
  req.URL.Scheme = "https"
  req.URL.Host = "router.example.com"
  req.Host       = "router.example.com"
  // path / query / body / other headers untouched
        │
        ▼
transport.RoundTrip(req)
  dial TLS, present client cert, verify server cert against bundled CA
        │
        ▼
ModifyResponse(resp)
  if Content-Type contains "text/event-stream":
    force Content-Type: text/event-stream
    force Cache-Control: no-cache
        │
        ▼
writeResponse
  io.Copy with periodic Flush (FlushInterval = -1)
  log: status, bytes, latency
        │
        ▼
Client receives response (SSE events stream through)
```

### 5.2 Stream vs Non-Stream

| Stage | Stream (`stream:true`) | Non-Stream (`stream:false`) |
|-------|------------------------|-----------------------------|
| Body sniff | Peek finds `stream:true` | Peek does not match |
| Response flush | `FlushInterval = -1` (immediate) | Default (buffered until chunk fills) |
| `ModifyResponse` | Force SSE headers | Passthrough |
| Upstream connection | Reuse transport keep-alive | Reuse transport keep-alive |
| Client view | SSE events as they arrive | One complete JSON body |

`FlushInterval = -1` is mandatory for SSE. With the default flush interval the
client will not see SSE events until the upstream response body is closed or
the internal buffer fills, which is the most common cause of "the LLM is
hanging" symptoms in proxies.

## 6. Error Handling

| Error source | HTTP status | Body |
|--------------|-------------|------|
| Upstream mTLS handshake failure (cert expired, untrusted CA, mismatch) | 502 | `{"error":{"message":"upstream mTLS handshake failed","type":"proxy_error"}}` |
| Upstream connection refused / timeout | 504 | `{"error":{"message":"upstream timeout","type":"proxy_error"}}` |
| Upstream 4xx / 5xx (HTTP-level, not transport) | **passthrough** | **passthrough** |
| Local client disconnects mid-response | n/a | ReverseProxy cancels upstream context |
| Startup mTLS probe failure | n/a | `log.Fatal` + `os.Exit(1)` |
| Mid-stream upstream error | n/a | Upstream closes connection; ReverseProxy propagates IO error to client |

### 6.1 Retry Policy

**No retries.** Reasoning:

- mTLS errors are deterministic; retrying cannot succeed without a cert change.
- Retrying a partial streaming response breaks the client contract.
- Application-level errors should be surfaced to the client as-is.

The client (e.g. Claude Code) is expected to implement its own retry policy on
top of an idempotency token if needed.

### 6.2 Local Client Disconnect

`req.Context().Done()` is the only signal we need. `httputil.ReverseProxy`
already cancels the upstream request when the client disconnects, freeing
upstream resources promptly.

## 7. Configuration

### 7.1 Build-Time (ldflags)

| Variable | Description | Source |
|----------|-------------|--------|
| `main.clientCertPEM` | PEM-encoded client certificate | `secrets/client.pem` |
| `main.clientKeyPEM` | PEM-encoded client private key | `secrets/client.key` |
| `main.upstreamCAPEM` | PEM-encoded CA that signs the public Nginx cert | `secrets/upstream-ca.pem` |
| `main.upstreamURL` | Default upstream URL, e.g. `https://router.example.com` | env / CI var |
| `main.version` | Git tag for `-version` output | `${GITHUB_REF_NAME}` |

The binary **never reads certificate files at runtime**. All secrets are
compiled in. The local filesystem has no plaintext key, even at rest.

### 7.1.1 Build-time injection mechanics

The injection is performed by the Go linker via `-ldflags -X`. The
mechanics, end-to-end, are:

1. **One-time, on the build machine or CI runner**, the operator generates
   the client cert (or receives it from the upstream Nginx operator) and
   places the three PEM files outside version control:

   ```
   secrets/
   ├── client.pem         # client cert (public)
   ├── client.key         # client private key
   └── upstream-ca.pem    # CA that signs the public Nginx cert
   ```

   `.gitignore` excludes the `secrets/` directory. CI uses GitHub Secrets
   (`MTLS_CLIENT_CERT`, `MTLS_CLIENT_KEY`, `MTLS_UPSTREAM_CA`).

2. **At build time**, the linker replaces four string variables in `main.go`:

   ```bash
   go build -trimpath \
     -ldflags "-s -w \
       -X main.clientCertPEM=$(cat secrets/client.pem) \
       -X main.clientKeyPEM=$(cat secrets/client.key) \
       -X main.upstreamCAPEM=$(cat secrets/upstream-ca.pem) \
       -X main.upstreamURL=${MTLS_UPSTREAM_URL} \
       -X main.version=${GITHUB_REF_NAME}" \
     -o dist/mtls-router-${GOOS}-${GOARCH}
   ```

   The `-X` flag takes a `package.Var=value` form. The value is a string
   literal that the linker writes into the data segment of the final binary
   at the address of the named package-level variable. This is a Go
   compiler feature documented in `go tool link -X`.

3. **At runtime**, the process loads the certs **once**, in `certs.Load()`
   at startup, by calling `tls.X509KeyPair([]byte(clientCertPEM),
   []byte(clientKeyPEM))` and `x509.NewCertPool().AppendCertsFromPEM(
   []byte(upstreamCAPEM))`. After that, no path on the runtime filesystem
   is ever opened. There is no fallback to disk; if the build-time
   injection is empty, `certs.Load()` returns an error and the process
   exits non-zero.

4. **Distribution**: end users receive the compiled binary only. There is
   no `secrets/` directory on their machines, no PEM file alongside the
   binary, no documentation telling them to download one. The only way to
   obtain the client cert is to extract it from the binary itself (see
   §11 for the threat-model discussion of this).

**Why not read PEM files at runtime?** Considered alternatives and why they
were rejected:

- OS Keychain / DPAPI / secret-service: each platform needs a different
  API, requires user interaction on first launch, and contradicts the
  "executable starts and works" goal.
- Read PEM from a fixed path next to the binary: the user would need a
  PEM file, contradicting the "built-in cert" requirement.
- PKCS#11 / TPM: not available on commodity hardware; adds 500+ lines.
- Encrypt the in-binary key with another hard-coded key: the attacker
  who reverses the binary recovers the encryption key, so this adds
  complexity with zero security gain.

The chosen `ldflags -X` approach is the simplest path that meets the
stated requirements, with the threat model documented in §11.

### 7.2 Runtime (env / flags)

| Setting | Env | Flag | Default |
|---------|-----|------|---------|
| Listen address | `MTLS_LISTEN_ADDR` | `-listen` | `127.0.0.1:19099` |
| Upstream URL | `MTLS_UPSTREAM_URL` | `-upstream` | build-time `upstreamURL` |
| TLS minimum version | `MTLS_TLS_MIN` | `-tls-min` | `tls1.2` |
| Non-stream timeout | `MTLS_TIMEOUT` | `-timeout` | `0` (no timeout) |
| Debug body logging | `MTLS_DEBUG=1` | `-debug` | off |

Flags override env which override build-time values which override defaults.
`MTLS_DEBUG` is intentionally not on by default; SSE bodies in production
logs would be both a privacy and disk-space hazard.

**Note on build-time vs runtime `upstreamURL`:** the `main.upstreamURL`
ldflags value is treated as the *default* upstream. The runtime
`MTLS_UPSTREAM_URL` env var and `-upstream` flag take precedence and can
override it. This allows the same binary to be re-pointed at a different
upstream without rebuilding — useful for staging vs production.

## 8. Build and Release

### 8.1 Build Command

```bash
go build -trimpath \
  -ldflags "-s -w \
    -X main.clientCertPEM=$(cat secrets/client.pem) \
    -X main.clientKeyPEM=$(cat secrets/client.key) \
    -X main.upstreamCAPEM=$(cat secrets/upstream-ca.pem) \
    -X main.upstreamURL=${MTLS_UPSTREAM_URL} \
    -X main.version=${GITHUB_REF_NAME}" \
  -o dist/mtls-router-${GOOS}-${GOARCH}
```

### 8.2 CI Matrix

`.github/workflows/release.yml` builds six artifacts per release tag:

| GOOS | GOARCH | Artifact |
|------|--------|----------|
| linux | amd64 | `mtls-router-linux-amd64` |
| linux | arm64 | `mtls-router-linux-arm64` |
| darwin | amd64 | `mtls-router-darwin-amd64` |
| darwin | arm64 | `mtls-router-darwin-arm64` |
| windows | amd64 | `mtls-router-windows-amd64.exe` |
| windows | arm64 | `mtls-router-windows-arm64.exe` |

Triggered by pushing a `v*` tag. Uploads each artifact plus a `SHA256SUMS`
file to the GitHub release.

### 8.3 CI Secrets

| GitHub secret | Purpose |
|---------------|---------|
| `MTLS_CLIENT_CERT` | Real client cert PEM |
| `MTLS_CLIENT_KEY` | Real client key PEM |
| `MTLS_UPSTREAM_CA` | Public Nginx CA PEM |
| `MTLS_UPSTREAM_URL` | Production upstream URL |

Dev tags (`v0.0.0-*`) use self-signed placeholders and skip the production
secrets.

### 8.4 Local Development

`scripts/build.sh` injects placeholder certs and a non-routable upstream so
that the binary builds and the startup probe fails fast (proving the wiring
is correct) without contacting any real infrastructure.

## 9. Deployment

### 9.1 systemd

`/etc/systemd/system/mtls-router.service`:

```
[Unit]
Description=mtls-router
After=network-online.target

[Service]
ExecStart=/usr/local/bin/mtls-router
Restart=on-failure
RestartSec=5
Environment=MTLS_LISTEN_ADDR=127.0.0.1:19099

[Install]
WantedBy=multi-user.target
```

### 9.2 Docker (optional)

`FROM scratch` with the static binary, exposing 19099. Image size < 20 MB.

### 9.3 Bare metal

`./mtls-router` listens on `127.0.0.1:19099` by default. The local client
points its API base URL at `http://127.0.0.1:19099/v1`.

## 10. Testing

### 10.1 Unit Tests

| File | Coverage |
|------|----------|
| `internal/proxy/transport_test.go` | Client cert sent correctly; CA verification succeeds / fails |
| `internal/proxy/stream_test.go` | `stream:true` detection; 4 KB boundary; chunked encoding |
| `internal/proxy/director_test.go` | Host rewritten; path/query passthrough; headers passthrough |
| `internal/proxy/modifyresponse_test.go` | SSE Content-Type forced; non-SSE passthrough |
| `internal/proxy/errorhandler_test.go` | mTLS failure → 502; upstream 4xx/5xx passthrough |

All tests use `httptest` servers. No external network dependency.

### 10.2 Manual Smoke Test (pre-release)

1. Build with staging cert + staging upstream URL.
2. Start `mtls-router`.
3. `curl -X POST http://127.0.0.1:19099/v1/messages -H 'Content-Type: application/json' -d @request.json`
4. Verify: 200, SSE events arrive incrementally, latency in access log.
5. Kill staging upstream; verify: 502, access log shows the reason.

### 10.3 CI Quality Gates

- `go test ./...` must pass on every PR.
- `go vet ./...` and `gofmt -l` must be clean.
- Cross-compile smoke: the release workflow builds all six targets and uploads
  them; a failed build is a failed release.

## 11. Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Compiled-in private key leaks via memory dump | Key is in a const string; Go does not move it, but it is still in process memory. Acceptable for a single-tenant local proxy. Document the threat model. |
| Compiled-in private key leaks via `strings(1)` on the binary | `-ldflags "-s -w"` strips the symbol table; key is still recoverable from raw binary strings, but only by an attacker who can read the binary. Document this. |
| WireGuard tunnel outage | `mtls-router` cannot detect WireGuard state directly; it surfaces the resulting transport errors as 502/504. Operators must monitor. |
| mTLS probe false positive (404 means channel is "up") | Acceptable: a working mTLS handshake against the bundled CA is the goal; the application-level 404 is a different concern. |
| Client sends a very large request body | 4 KB peek is O(1) and does not grow with body size. ReverseProxy streams the body without buffering. |
| Long-lived SSE connection killed by upstream idle timeout | `IdleConnTimeout = 90s` covers typical LLM streams; the upstream's own keep-alive is out of scope. |

## 12. Open Questions

None at design time. Concrete values (real upstream URL, real CA fingerprint)
will be filled into `scripts/build.sh` and CI secrets at the implementation
stage.

## 13. Out of Scope (for v1)

- Hot-reload of certificates (requires new build).
- Multiple upstream endpoints or failover.
- HTTP/2 to the local client (Go's `http.Server` already supports HTTP/2 with
  TLS; we use plain HTTP locally for simplicity).
- Prometheus metrics endpoint.
