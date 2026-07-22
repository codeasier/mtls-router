# internal/certs

PEM string parsing into TLS client certificate and upstream CA pool.

## Files

| File | Role |
|------|------|
| `certs.go` | `LoadFromStrings(certPEM, keyPEM, caPEM)` → `(*tls.Certificate, *x509.CertPool, error)` |

## Behavior

- All three inputs are required; empty string is a hard error.
- Uses `tls.X509KeyPair` for client cert/key and `x509.CertPool.AppendCertsFromPEM` for CA.
- Called once at startup by both `proxy.NewMTLSTransport` and `health.NewProber`.

## Consumers

- `internal/proxy/transport.go`
- `internal/health/probe.go`
