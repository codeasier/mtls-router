#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

missing=0
for file in secrets/client.pem secrets/client.key secrets/upstream-ca.pem; do
  [[ -f "$file" ]] || missing=$((missing + 1))
done

if [[ "$missing" -eq 3 ]]; then
  echo ">> generating placeholder certs in secrets/"
  mkdir -p secrets
  openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
    -keyout secrets/client.key \
    -out secrets/client.pem \
    -subj "/CN=mtls-router-placeholder" 2>/dev/null
  cp secrets/client.pem secrets/upstream-ca.pem
elif [[ "$missing" -eq 0 ]]; then
  echo ">> reusing existing secrets/ certs"
else
  echo ">> partial secrets/ set found; refusing to overwrite existing files" >&2
  echo ">> provide all of secrets/client.pem, secrets/client.key, secrets/upstream-ca.pem or remove all three to generate placeholders" >&2
  exit 1
fi

echo ">> building mtls-router"
VERSION="${VERSION:-dev}"
if command -v git >/dev/null 2>&1 && git rev-parse --short HEAD >/dev/null 2>&1; then
  COMMIT="$(git rev-parse --short HEAD)"
else
  COMMIT="unknown"
fi
BUILD_DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

go build -trimpath \
  -ldflags "-s -w \
    -X 'main.clientCertPEM=$(cat secrets/client.pem)' \
    -X 'main.clientKeyPEM=$(cat secrets/client.key)' \
    -X 'main.upstreamCAPEM=$(cat secrets/upstream-ca.pem)' \
    -X 'main.upstreamURL=https://upstream.placeholder.invalid' \
    -X 'github.com/codeasier/mtls-router/internal/version.Version=${VERSION}' \
    -X 'github.com/codeasier/mtls-router/internal/version.Commit=${COMMIT}' \
    -X 'github.com/codeasier/mtls-router/internal/version.BuildDate=${BUILD_DATE}'" \
  -o mtls-router .

echo ">> built: ./mtls-router"
echo ">> run hint: ./mtls-router will fail fast because the placeholder upstream URL is not real"
