# Build and Release

This document is for maintainers who need to build `mtls-router` from source or publish GitHub Release binaries.

## Local placeholder build

For local development, run:

```bash
./scripts/build.sh
```

The script creates these files if they are missing:

- `secrets/client.pem`
- `secrets/client.key`
- `secrets/upstream-ca.pem`

It then runs `go build -trimpath` and writes `./mtls-router`.

The generated placeholder binary is expected to fail fast at startup until it is built with real upstream configuration and certificate material.

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

## GitHub Release configuration

The release workflow reads these repository secrets:

- `CLIENT_CERT_PEM`
- `CLIENT_KEY_PEM`
- `UPSTREAM_CA_PEM`

It also reads this repository variable:

- `UPSTREAM_URL`

Set them with `gh`:

```bash
gh secret set CLIENT_CERT_PEM --repo codeasier/mtls-router < secrets/client.pem
gh secret set CLIENT_KEY_PEM --repo codeasier/mtls-router < secrets/client.key
gh secret set UPSTREAM_CA_PEM --repo codeasier/mtls-router < secrets/upstream-ca.pem
gh variable set UPSTREAM_URL --repo codeasier/mtls-router --body "https://router.example.com"
```

## Publish a release

Push a version tag:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The GitHub Actions release workflow cross-compiles binaries for:

- `linux/amd64`
- `linux/arm64`
- `darwin/amd64`
- `darwin/arm64`
- `windows/amd64`
- `windows/arm64`

The workflow uploads each binary as a workflow artifact and, for tag builds, attaches the binaries to the GitHub Release.
