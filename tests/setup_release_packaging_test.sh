#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKFLOW="$ROOT/.github/workflows/release.yml"

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
contains() { grep -Fq -- "$1" "$WORKFLOW" || fail "workflow missing: $1"; }

[[ "$(grep -c '^  release:$' "$WORKFLOW")" -eq 1 ]] || fail "expected one aggregation/release job"
[[ "$(grep -c 'softprops/action-gh-release@' "$WORKFLOW")" -eq 1 ]] || fail "release must be published once"
[[ "$(grep -c 'easingthemes/ssh-deploy@' "$WORKFLOW")" -eq 1 ]] || fail "staged outputs must be mirrored once"
contains 'needs: [build, desktop]'
contains 'actions/download-artifact@v4'
contains 'merge-multiple: true'
contains 'LC_ALL=C sha256sum release/mtls-router-*'
contains 'LC_ALL=C sort -k2 >release/SHA256SUMS'
contains 'packages/$package.tar.gz'
contains 'packages/$package.zip'
contains 'mtls-router-manager-${GOOS}-${GOARCH}${ext}'
contains './cmd/mtls-router-manager'
contains '"release/$manager"'
contains 'test -n "$CLIENT_CERT_PEM" && test -n "$CLIENT_KEY_PEM" && test -n "$UPSTREAM_CA_PEM"'
contains 'case "${UPSTREAM_URL:-}" in https://*)'
contains 'files: release/*'
contains 'SOURCE: release/'
contains 'TARGET: /home/codeasier/downloads/${{ github.event.repository.name }}/${{ github.ref_name }}/'
contains 'ARGS: -avz --delete'
contains "grep -Fxc 'DEFAULT_DOWNLOAD_BASE_URL=\"\"' setup.sh"
contains "grep -Fxc '\$DefaultDownloadBaseUrl = '\\'''\\''' setup.ps1"
contains "test \"\$(od -An -tx1 -N3 setup.ps1 | tr -d ' \\n')\" = efbbbf"
contains 'test "$(find release -maxdepth 1 -type f -name '\''mtls-router-*'\'' | wc -l)" -eq 12'
contains 'test "$(find release -maxdepth 1 -type f | wc -l)" -eq 19'
contains 'pattern: mtls-router-cli-*'
contains 'pattern: mtls-router-desktop-*'
contains 'test "$(find release -maxdepth 1 -type f -name '\''mtls-router-desktop-*'\'' | wc -l)" -eq 12'
contains 'test "$(find release -maxdepth 1 -type f -name '\''signing-status-*'\'' | wc -l)" -eq 6'

if grep -Fq 'http://' "$WORKFLOW"; then
  fail "workflow contains a plaintext HTTP URL"
fi
if awk '/^  build:/{build=1} /^  release:/{build=0} build && /softprops\/action-gh-release|ssh-action|ssh-deploy/' "$WORKFLOW" | grep -q .; then
  fail "matrix build performs publishing"
fi

printf 'PASS: release packaging workflow\n'
