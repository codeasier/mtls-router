#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKFLOW="$ROOT/.github/workflows/release.yml"
RECOVERY="$ROOT/.github/workflows/recover-release.yml"
PACKAGE_SCRIPT="$ROOT/scripts/package-release.sh"

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
contains() { grep -Fq -- "$1" "$WORKFLOW" || grep -Fq -- "$1" "$PACKAGE_SCRIPT" || fail "release implementation missing: $1"; }
package_contains() { grep -Fq -- "$1" "$PACKAGE_SCRIPT" || fail "package script missing: $1"; }

[[ "$(grep -c '^  release:$' "$WORKFLOW")" -eq 1 ]] || fail "expected one aggregation/release job"
[[ "$(grep -c 'softprops/action-gh-release@' "$WORKFLOW")" -eq 1 ]] || fail "release must be published once"
[[ "$(grep -c 'easingthemes/ssh-deploy@' "$WORKFLOW")" -eq 1 ]] || fail "staged outputs must be mirrored once"
contains 'needs: [build, desktop]'
contains 'actions/download-artifact@v4'
contains 'merge-multiple: true'
package_contains 'LC_ALL=C sha256sum release/mtls-router-*'
package_contains 'LC_ALL=C sort -k2 >release/SHA256SUMS'
package_contains 'packages/$package.tar.gz'
package_contains 'packages/$package.zip'
contains 'mtls-router-manager-${GOOS}-${GOARCH}${ext}'
contains './cmd/mtls-router-manager'
package_contains '"release/$manager"'
contains 'test -n "$CLIENT_CERT_PEM" && test -n "$CLIENT_KEY_PEM" && test -n "$UPSTREAM_CA_PEM"'
contains 'case "${UPSTREAM_URL:-}" in https://*)'
contains 'files: release/*'
contains 'SOURCE: release/'
contains 'TARGET: /home/codeasier/downloads/${{ github.event.repository.name }}/${{ github.ref_name }}/'
contains 'ARGS: -avz --delete'
package_contains "grep -Fxc 'DEFAULT_DOWNLOAD_BASE_URL=\"\"' setup.sh"
contains "grep -Fxc '\$DefaultDownloadBaseUrl = '\\'''\\''' setup.ps1"
contains "test \"\$(od -An -tx1 -N3 setup.ps1 | tr -d ' \\n')\" = efbbbf"
package_contains 'test "$(find release -maxdepth 1 -type f -name '\''mtls-router-*'\'' | wc -l)" -eq 12'
package_contains 'test "$(find release -maxdepth 1 -type f | wc -l)" -eq 19'
contains 'pattern: mtls-router-cli-*'
package_contains 'test "$(find release -maxdepth 1 -type f -name '\''CodeasierRouter-*'\'' | wc -l)" -eq 12'
package_contains 'test "$(find release -maxdepth 1 -type f -name '\''signing-status-*'\'' | wc -l)" -eq 6'
contains './scripts/package-release.sh'
[[ "$(grep -Fc 'version="${GITHUB_REF_NAME#v}"' "$WORKFLOW")" -eq 2 ]] || \
  fail 'CLI and desktop jobs must derive tag versions without the v prefix'
[[ "$(grep -Fc 'version="$DISPATCH_VERSION"' "$WORKFLOW")" -eq 2 ]] || \
  fail 'CLI and desktop jobs must use the dispatch version for validation builds'
[[ "$(grep -Fc "Version=\${RELEASE_VERSION}" "$WORKFLOW")" -eq 2 ]] || \
  fail 'both CLI binaries must use the derived release version'
if grep -Fq 'Version=${GITHUB_REF_NAME}' "$WORKFLOW"; then
  fail 'CLI binaries must not use the branch name as their validation version'
fi
[[ "$(grep -Fc 'desktop/release-artifacts/CodeasierRouter-windows-${{ matrix.arch }}.exe' "$WORKFLOW")" -eq 2 ]] || \
  fail 'Windows signing checks must use the normalized desktop package name'
if grep -Fq 'desktop/release-artifacts/mtls-router-desktop-windows-' "$WORKFLOW"; then
  fail 'Windows signing checks contain the obsolete desktop package name'
fi

desktop_upload_template="$(awk '
  /^[[:space:]]+name: / && /matrix\.os/ && /matrix\.arch/ {
    sub(/^[[:space:]]+name: /, "")
    print
    exit
  }
' "$WORKFLOW")"
[[ "$desktop_upload_template" == 'mtls-router-desktop-${{ matrix.os }}-${{ matrix.arch }}' ]] || \
  fail "desktop upload name template changed: $desktop_upload_template"

desktop_download_glob="$(awk '
  /^      - name: Download all desktop packages$/ { capture=1; next }
  capture && /^      - name:/ { exit }
  capture && /^[[:space:]]+pattern: / {
    sub(/^[[:space:]]+pattern: /, "")
    print
    exit
  }
' "$WORKFLOW")"
[[ "$desktop_download_glob" == 'mtls-router-desktop-*' ]] || \
  fail "desktop aggregation glob is not mtls-router-desktop-*: $desktop_download_glob"

desktop_matrix_entries="$(awk '
  /^  desktop:$/ { capture=1; next }
  capture && /^  release:$/ { exit }
  capture && /^[[:space:]]+os: (windows|darwin|linux)$/ { os=$2; next }
  capture && os != "" && /^[[:space:]]+arch: (amd64|arm64)$/ {
    print os, $2
    os=""
  }
' "$WORKFLOW")"
[[ "$(printf '%s\n' "$desktop_matrix_entries" | awk 'NF == 2 {n++} END{print n+0}')" -eq 6 ]] || \
  fail 'desktop release matrix must contain six os/arch producers'
for expected in \
  'windows amd64' 'windows arm64' \
  'darwin amd64' 'darwin arm64' \
  'linux amd64' 'linux arm64'; do
  if ! printf '%s\n' "$desktop_matrix_entries" | awk -v expected="$expected" '$0 == expected {found=1} END{exit !found}'; then
    fail "desktop release matrix is missing producer: $expected"
  fi
done

while read -r os arch; do
  producer_name="$(printf '%s\n' "$desktop_upload_template" | awk -v os="$os" -v arch="$arch" '{
    gsub(/\$\{\{ matrix\.os \}\}/, os)
    gsub(/\$\{\{ matrix\.arch \}\}/, arch)
    print
  }')"
  expected_name="mtls-router-desktop-${os}-${arch}"
  [[ "$producer_name" == "$expected_name" ]] || \
    fail "desktop upload template produced $producer_name, expected $expected_name"
  case "$producer_name" in
    $desktop_download_glob) ;;
    *) fail "desktop producer $producer_name does not match $desktop_download_glob" ;;
  esac
done <<< "$desktop_matrix_entries"

if grep -Fq 'http://' "$WORKFLOW"; then
  fail "workflow contains a plaintext HTTP URL"
fi
if awk '/^  build:/{build=1} /^  release:/{build=0} build && /softprops\/action-gh-release|ssh-action|ssh-deploy/' "$WORKFLOW" | grep -q .; then
  fail "matrix build performs publishing"
fi

[[ -f "$PACKAGE_SCRIPT" ]] || fail 'shared release packaging script is missing'
grep -Fq '"../../packages/$package.zip"' "$PACKAGE_SCRIPT" || \
  fail 'Windows archives must resolve to the repository packages directory'
if grep -Fq '"../../../packages/$package.zip"' "$PACKAGE_SCRIPT"; then
  fail 'shared release packaging script contains the broken Windows archive path'
fi

[[ -f "$RECOVERY" ]] || fail 'release recovery workflow is missing'
for value in \
  'release_tag:' \
  'source_run_id:' \
  'actions: read' \
  'contents: write' \
  'group: release-publication' \
  'persist-credentials: false' \
  'workflow_id == ".github/workflows/release.yml"' \
  'event == "push"' \
  'head_branch == release_tag' \
  'head_sha == tag_sha' \
  'conclusion == "failure"' \
  'assert release["draft"] is True' \
  'run-id: ${{ inputs.source_run_id }}' \
  'github-token: ${{ github.token }}' \
  './scripts/package-release.sh' \
  'gh release create "$RELEASE_TAG" --draft' \
  'gh release upload "$RELEASE_TAG" release/* --clobber' \
  'releases/assets/$asset_id' \
  "-eq 31" \
  'SOURCE: release/' \
  'Update latest symlink' \
  '--draft=false --latest'; do
  grep -Fq -- "$value" "$RECOVERY" || fail "recovery workflow missing: $value"
done

printf 'PASS: release packaging workflow\n'
