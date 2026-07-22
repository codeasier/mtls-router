# internal/proxy

Reverse proxy core: builds `httputil.ReverseProxy` with mTLS transport, SSE streaming, and sanitized error handling.

## Files

| File | Role |
|------|------|
| `proxy.go` | `New(Options)` — assembles ReverseProxy with director, ModifyResponse, ErrorHandler, `FlushInterval: -1` |
| `transport.go` | `NewMTLSTransport(certPEM, keyPEM, caPEM, ...TransportOption)` — builds `*http.Transport` with embedded mTLS certs |
| `director.go` | `NewDirector(upstream)` — rewrites request Scheme/Host only; preserves path/query/body |
| `modifyresponse.go` | `NewModifyResponse()` — normalizes SSE headers (`text/event-stream`, `no-cache`, `X-Accel-Buffering: no`) |
| `errorhandler.go` | `NewErrorHandler(logger)` — returns sanitized JSON errors; classifies 502/504/400 without leaking upstream details |
| `bodyerror.go` | `wrapBodyReadErrors(r)` — wraps request body to tag client-side read failures as `clientBodyReadError` |

## Key invariants

- `FlushInterval: -1` is mandatory for SSE/chunked streaming — never buffer responses.
- Error JSON must never contain upstream URLs, cert details, or raw error strings; tests assert sanitization.
- `sanitizedProxyLogWriter` replaces the default `log.Logger` output to suppress `httputil` internal messages.
- Do not add request-pipeline middleware here; compose hooks at the mux call site in `main.go`.

## Dependencies

- `internal/certs` — PEM string → `tls.Certificate` + `x509.CertPool`
- `internal/tlspolicy` — version string → `uint16` TLS constant

## Tests

- `service_contract_test.go` — end-to-end proxy contract assertions
- `bodyerror_policy_test.go` — client body error → 400 classification
- `transport_test.go` — mTLS transport construction and TLS min enforcement
