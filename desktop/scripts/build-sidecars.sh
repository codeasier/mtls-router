#!/usr/bin/env bash
set -euo pipefail

desktop_dir="$(cd "$(dirname "$0")/.." && pwd)"
repo_dir="$(cd "$desktop_dir/.." && pwd)"
target="${TAURI_ENV_TARGET_TRIPLE:-${TARGET:-$(rustc --print host-tuple)}}"

case "$target" in
  aarch64-apple-darwin) goos=darwin; goarch=arm64; extension= ;;
  x86_64-apple-darwin) goos=darwin; goarch=amd64; extension= ;;
  aarch64-unknown-linux-gnu) goos=linux; goarch=arm64; extension= ;;
  x86_64-unknown-linux-gnu) goos=linux; goarch=amd64; extension= ;;
  aarch64-pc-windows-msvc) goos=windows; goarch=arm64; extension=.exe ;;
  x86_64-pc-windows-msvc) goos=windows; goarch=amd64; extension=.exe ;;
  *) printf 'unsupported desktop target: %s\n' "$target" >&2; exit 1 ;;
esac

out_dir="$desktop_dir/src-tauri/binaries"
mkdir -p "$out_dir"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/mtls-router-sidecars.XXXXXX")"
trap 'rm -rf "$tmp_dir"' EXIT

cert="$repo_dir/secrets/client.pem"
key="$repo_dir/secrets/client.key"
ca="$repo_dir/secrets/upstream-ca.pem"
upstream_url="${UPSTREAM_URL:-https://upstream.placeholder.invalid}"
present=0
for path in "$cert" "$key" "$ca"; do
  [[ -f "$path" ]] && present=$((present + 1))
done
if [[ "$present" -ne 0 && "$present" -ne 3 ]]; then
  printf 'partial secrets set; provide all three files or none\n' >&2
  exit 1
fi
embedded=0
for value in "${CLIENT_CERT_PEM:-}" "${CLIENT_KEY_PEM:-}" "${UPSTREAM_CA_PEM:-}"; do
  [[ -n "$value" ]] && embedded=$((embedded + 1))
done
if [[ "$embedded" -ne 0 && "$embedded" -ne 3 ]]; then
  printf 'partial embedded secrets set; provide all three values or none\n' >&2
  exit 1
fi
if [[ "$present" -gt 0 && "$embedded" -gt 0 ]]; then
  printf 'provide credential files or environment values, not both\n' >&2
  exit 1
fi
if [[ "$embedded" -eq 3 ]]; then
  cert="$tmp_dir/client.pem"
  key="$tmp_dir/client.key"
  ca="$tmp_dir/upstream-ca.pem"
  printf '%s' "$CLIENT_CERT_PEM" >"$cert"
  printf '%s' "$CLIENT_KEY_PEM" >"$key"
  printf '%s' "$UPSTREAM_CA_PEM" >"$ca"
  present=3
fi
if [[ "$present" -eq 0 ]]; then
  cert="$tmp_dir/client.pem"
  key="$tmp_dir/client.key"
  ca="$tmp_dir/upstream-ca.pem"
  MSYS2_ARG_CONV_EXCL='/CN=' openssl req -x509 -newkey rsa:2048 -nodes -days 1 -keyout "$key" -out "$cert" -subj /CN=mtls-router-placeholder 2>/dev/null
  cp "$cert" "$ca"
fi

version="${VERSION:-$(cd "$desktop_dir" && node -p "require('./package.json').version")}"
deployment_id="${DEPLOYMENT_ID:-dev}"
management_protocol_version="${MANAGEMENT_PROTOCOL_VERSION:-1}"
[[ "$management_protocol_version" == 1 ]] || {
  printf 'unsupported MANAGEMENT_PROTOCOL_VERSION: %s\n' "$management_protocol_version" >&2
  exit 1
}
if [[ "${RELEASE_BUILD:-0}" == 1 ]]; then
  case "$version" in dev|unknown|'') printf 'release VERSION must be non-default\n' >&2; exit 1 ;; esac
  case "$deployment_id" in dev|unknown|'') printf 'release DEPLOYMENT_ID must be non-default\n' >&2; exit 1 ;; esac
  [[ "$embedded" -eq 3 || "$present" -eq 3 ]] || { printf 'release credentials are required\n' >&2; exit 1; }
  [[ "$upstream_url" == https://* ]] || { printf 'release UPSTREAM_URL must use HTTPS\n' >&2; exit 1; }
fi
commit="$(git -C "$repo_dir" rev-parse --short HEAD 2>/dev/null || printf unknown)"
build_date="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
metadata="-s -w -X 'github.com/codeasier/mtls-router/internal/version.Version=$version' -X 'github.com/codeasier/mtls-router/internal/version.Commit=$commit' -X 'github.com/codeasier/mtls-router/internal/version.BuildDate=$build_date' -X 'github.com/codeasier/mtls-router/internal/version.DeploymentID=$deployment_id'"

router="$out_dir/mtls-router-$target$extension"
manager="$out_dir/mtls-router-manager-$target$extension"
(
  cd "$repo_dir"
  GOOS="$goos" GOARCH="$goarch" CGO_ENABLED=0 go build -trimpath -ldflags "$metadata -X 'main.clientCertPEM=$(cat "$cert")' -X 'main.clientKeyPEM=$(cat "$key")' -X 'main.upstreamCAPEM=$(cat "$ca")' -X 'main.upstreamURL=$upstream_url'" -o "$router" .
  GOOS="$goos" GOARCH="$goarch" CGO_ENABLED=0 go build -trimpath -ldflags "$metadata" -o "$manager" ./cmd/mtls-router-manager
)

printf 'built %s\nbuilt %s\n' "$manager" "$router"
