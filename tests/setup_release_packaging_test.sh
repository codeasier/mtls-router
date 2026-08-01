#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKFLOW="$ROOT/.github/workflows/release.yml"
RECOVERY="$ROOT/.github/workflows/recover-release.yml"
PACKAGE_SCRIPT="$ROOT/scripts/package-release.sh"
PROTOCOL_CHECK="$ROOT/scripts/check-release-protocol.sh"
COMPATIBILITY="$ROOT/internal/manager/agent/testdata/compatibility.json"

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
contains() { grep -Fq -- "$1" "$WORKFLOW" || grep -Fq -- "$1" "$PACKAGE_SCRIPT" || fail "release implementation missing: $1"; }
package_contains() { grep -Fq -- "$1" "$PACKAGE_SCRIPT" || fail "package script missing: $1"; }

[[ "$(grep -c '^  release:$' "$WORKFLOW")" -eq 1 ]] || fail "expected one aggregation/release job"
[[ "$(grep -c 'softprops/action-gh-release@' "$WORKFLOW")" -eq 1 ]] || fail "release must be published once"
[[ "$(grep -c 'easingthemes/ssh-deploy@' "$WORKFLOW")" -eq 1 ]] || fail "staged outputs must be mirrored once"
contains 'needs: [prepare, build, desktop]'
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
package_contains 'expected_desktop_assets=12'
package_contains 'if [[ "$online_update" == true ]]; then expected_desktop_assets=20; fi'
package_contains 'test "$(find release -maxdepth 1 -type f -name '\''signing-status-*'\'' | wc -l)" -eq 6'
package_contains 'expected_release_files=37'
package_contains 'expected_release_files=46'
contains './scripts/package-release.sh'
package_contains './scripts/check-release-protocol.sh protocol-metadata'
[[ -x "$PROTOCOL_CHECK" ]] || fail 'release protocol preflight is missing or not executable'
[[ "$(grep -Fc 'release-metadata-' "$WORKFLOW")" -ge 3 ]] || fail 'release producers/aggregation do not carry protocol metadata'
grep -Fq 'mv binaries/release-metadata-*.json desktop-packages/release-metadata-*.json protocol-metadata/' "$RECOVERY" || fail 'recovery release does not collect protocol metadata'

jq -e '
  .manifest_version == 1 and .retrieved == "2026-07-18" and
  ([.claude_code, .opencode, .codex] | all(
    (.tested_version | type == "string" and length > 0) and
    (.source | startswith("https://")) and
    (.revision | type == "string" and length > 0) and
    (.sha256 | test("^[0-9a-f]{64}$")) and
    (.integrity | startswith("sha512-"))
  )) and
  (.opencode.schema_source | startswith("https://")) and
  (.opencode.schema_revision | length > 0) and
  (.opencode.schema_sha256 | test("^[0-9a-f]{64}$")) and
  (.codex.config_revision | test("^[0-9a-f]{40}$")) and
  (.codex.config_source_archive_sha256 | test("^[0-9a-f]{64}$"))
' "$COMPATIBILITY" >/dev/null || fail 'Agent compatibility manifest pins are incomplete'

protocol_tmp="$(mktemp -d)"
trap 'rm -rf "$protocol_tmp"' EXIT
for kind in cli desktop; do
  for os_arch in linux-amd64 linux-arm64 darwin-amd64 darwin-arm64 windows-amd64 windows-arm64; do
    printf '{"schema_version":1,"producer":"%s-%s","management_protocol_version":"4"}\n' "$kind" "$os_arch" >"$protocol_tmp/release-metadata-$kind-$os_arch.json"
  done
done
"$PROTOCOL_CHECK" "$protocol_tmp" || fail 'matching protocol-v4 metadata was rejected'
jq '.management_protocol_version = "3"' "$protocol_tmp/release-metadata-cli-linux-amd64.json" >"$protocol_tmp/deliberate-protocol-v3-mismatch.json"
mv "$protocol_tmp/deliberate-protocol-v3-mismatch.json" "$protocol_tmp/release-metadata-cli-linux-amd64.json"
if "$PROTOCOL_CHECK" "$protocol_tmp" >/dev/null 2>&1; then
  fail 'deliberate mixed protocol-v3/v4 release metadata was accepted'
fi

package_tmp="$protocol_tmp/package-fixture"
mkdir -p "$package_tmp/bin" "$package_tmp/scripts" "$package_tmp/binaries" "$package_tmp/desktop-packages" "$package_tmp/protocol-metadata"
cat >"$package_tmp/bin/tar" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" != --sort=name ]]; then exec /usr/bin/tar "$@"; fi
output=
args=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --sort=name|--mtime=@0|--owner=0|--group=0|--numeric-owner) shift ;;
    -czf) output=$2; shift 2 ;;
    *) args+=("$1"); shift ;;
  esac
done
exec /usr/bin/tar -czf "$output" "${args[@]}"
SH
cat >"$package_tmp/bin/touch" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == -d ]]; then shift 2; fi
exec /usr/bin/touch "$@"
SH
chmod +x "$package_tmp/bin/tar" "$package_tmp/bin/touch"
cp "$PACKAGE_SCRIPT" "$PROTOCOL_CHECK" "$package_tmp/scripts/"
cp "$ROOT/setup.sh" "$ROOT/setup.ps1" "$package_tmp/"
for kind in cli desktop; do
  for os_arch in linux-amd64 linux-arm64 darwin-amd64 darwin-arm64 windows-amd64 windows-arm64; do
    printf '{"schema_version":1,"producer":"%s-%s","management_protocol_version":"4"}\n' "$kind" "$os_arch" >"$package_tmp/protocol-metadata/release-metadata-$kind-$os_arch.json"
  done
done
for os_arch in linux-amd64 linux-arm64 darwin-amd64 darwin-arm64; do
  printf 'router %s\n' "$os_arch" >"$package_tmp/binaries/mtls-router-$os_arch"
  printf 'manager %s\n' "$os_arch" >"$package_tmp/binaries/mtls-router-manager-$os_arch"
done
for os_arch in windows-amd64 windows-arm64; do
  printf 'router %s\n' "$os_arch" >"$package_tmp/binaries/mtls-router-$os_arch.exe"
  printf 'manager %s\n' "$os_arch" >"$package_tmp/binaries/mtls-router-manager-$os_arch.exe"
done
for os_arch in linux-amd64 linux-arm64 windows-amd64 windows-arm64 darwin-amd64 darwin-arm64; do
  os=${os_arch%-*}
  arch=${os_arch#*-}
  case "$os" in darwin) suffix=dmg ;; linux) suffix=AppImage ;; windows) suffix=exe ;; esac
  package="CodeasierRouter-$os-$arch.$suffix"
  printf 'desktop package %s\n' "$os_arch" >"$package_tmp/desktop-packages/$package"
  (cd "$package_tmp/desktop-packages" && sha256sum "$package" >"CodeasierRouter-$os-$arch.sha256")
  printf 'signed\n' >"$package_tmp/desktop-packages/signing-status-$os-$arch.txt"
  if [[ "$os" == darwin ]]; then
    updater="CodeasierRouter-$os-$arch.app.tar.gz"
    printf 'desktop updater %s\n' "$os_arch" >"$package_tmp/desktop-packages/$updater"
  else
    updater="$package"
  fi
  printf 'trusted updater signature %s\n' "$os_arch" >"$package_tmp/desktop-packages/$updater.sig"
done
(cd "$package_tmp" && PATH="$package_tmp/bin:$PATH" RELEASE_TAG=v1.2.3 DOWNLOAD_BASE_URL=https://downloads.codeasier.top/mtls-router/v1.2.3 SOURCE_DATE_EPOCH=0 ./scripts/package-release.sh) || \
  fail 'stable updater release fixture failed to package'
[[ "$(find "$package_tmp/release" -maxdepth 1 -type f | wc -l | tr -d ' ')" -eq 46 ]] || \
  fail 'stable updater release fixture has the wrong exact asset count'
jq -e '
  .version == "1.2.3" and
  (.platforms | length) == 6 and
  .platforms["darwin-aarch64"].url == "https://downloads.codeasier.top/mtls-router/v1.2.3/CodeasierRouter-darwin-arm64.app.tar.gz" and
  .platforms["linux-x86_64"].url == "https://downloads.codeasier.top/mtls-router/v1.2.3/CodeasierRouter-linux-amd64.AppImage" and
  .platforms["windows-x86_64"].url == "https://downloads.codeasier.top/mtls-router/v1.2.3/CodeasierRouter-windows-amd64.exe"
' "$package_tmp/release/latest.json" >/dev/null || fail 'stable updater latest.json is invalid'
(cd "$package_tmp/release" && sha256sum -c SHA256SUMS >/dev/null) || fail 'stable updater release checksums are invalid'

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
[[ "$desktop_upload_template" == 'CodeasierRouter-desktop-${{ matrix.os }}-${{ matrix.arch }}' ]] || \
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
[[ "$desktop_download_glob" == 'CodeasierRouter-desktop-*' ]] || \
  fail "desktop aggregation glob is not CodeasierRouter-desktop-*: $desktop_download_glob"

for os_arch in windows-amd64 windows-arm64 darwin-amd64 darwin-arm64 linux-amd64 linux-arm64; do
  os=${os_arch%-*}
  arch=${os_arch#*-}
  grep -Fq "{\"name\":\"$os_arch\"" "$WORKFLOW" || fail "release matrix is missing target: $os_arch"
  producer_name="$(printf '%s\n' "$desktop_upload_template" | awk -v os="$os" -v arch="$arch" '{
    gsub(/\$\{\{ matrix\.os \}\}/, os)
    gsub(/\$\{\{ matrix\.arch \}\}/, arch)
    print
  }')"
  expected_name="CodeasierRouter-desktop-${os}-${arch}"
  [[ "$producer_name" == "$expected_name" ]] || \
    fail "desktop upload template produced $producer_name, expected $expected_name"
  case "$producer_name" in
    $desktop_download_glob) ;;
    *) fail "desktop producer $producer_name does not match $desktop_download_glob" ;;
  esac
  grep -Fq "CodeasierRouter-desktop-${os}-${arch}" "$RECOVERY" || \
    fail "recovery workflow is missing CodeasierRouter desktop artifact $os-$arch"
done
grep -Fq 'pattern: CodeasierRouter-desktop-*' "$RECOVERY" || \
  fail 'recovery workflow does not download CodeasierRouter desktop artifacts'
[[ "$(grep -Fc 'matrix: ${{ fromJSON(needs.prepare.outputs.' "$WORKFLOW")" -eq 2 ]] || \
  fail 'release producer jobs must consume prepared dynamic matrices'
grep -Fq 'SELECTED_TARGET: ${{ github.event_name == '\''workflow_dispatch'\'' && inputs.target || '\''all'\'' }}' "$WORKFLOW" || \
  fail 'tag releases must select all build targets'
[[ "$(grep -Fc 'UPSTREAM_URL: ${{ github.event_name == '\''workflow_dispatch'\'' && inputs.upstream_url || vars.UPSTREAM_URL }}' "$WORKFLOW")" -eq 2 ]] || \
  fail 'validation upstream override must be isolated to producer jobs'

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
  'if release["draft"]:' \
  'releases?per_page=100" --slurp' \
  'run-id: ${{ inputs.source_run_id }}' \
  'github-token: ${{ github.token }}' \
  './scripts/package-release.sh' \
  'gh release create "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY"' \
  'https://uploads.github.com/repos/$GITHUB_REPOSITORY' \
  'releases/$release_id/assets?name=$(basename "$asset")' \
  'releases/assets/$asset_id' \
  'cmp expected-assets.txt actual-assets.txt' \
  'if [[ "$release_is_draft" == true ]]' \
  'test "$release_is_draft" = false' \
  'RELEASE_ID: ${{ steps.prepare-release.outputs.release_id }}' \
  'SOURCE: release/' \
  'Validate recovered updater assets' \
  '(cd release && sha256sum -c SHA256SUMS)' \
  'Update latest symlink' \
  '-F draft=false -F make_latest=true'; do
  grep -Fq -- "$value" "$RECOVERY" || fail "recovery workflow missing: $value"
done
for value in \
  'RELEASE_TAG: ${{ inputs.release_tag }}' \
  'release/latest.json' \
  'test ! -e "$base/latest" || test -L "$base/latest"' \
  'current="$(basename "$(readlink "$base/latest")")"' \
  'sort -V | tail -n1' \
  'mv -Tf "$base/latest.tmp" "$base/latest"'; do
  grep -Fq -- "$value" "$RECOVERY" || fail "recovery updater handling missing: $value"
done
for value in \
  'RELEASE_TAG: ${{ github.ref_name }}' \
  'if: needs.prepare.outputs.online-update == '\''true'\''' \
  'test ! -e "$base/latest" || test -L "$base/latest"' \
  'current="$(basename "$(readlink "$base/latest")")"' \
  'sort -V | tail -n1' \
  'mv -Tf "$base/latest.tmp" "$base/latest"'; do
  grep -Fq -- "$value" "$WORKFLOW" || fail "release updater publication missing: $value"
done
for target in linux-x86_64 linux-aarch64 windows-x86_64 windows-aarch64 darwin-x86_64 darwin-aarch64; do
  grep -Fq -- "\"$target\"" "$PACKAGE_SCRIPT" || fail "latest feed is missing target $target"
done
if grep -Fq -- '--hostname uploads.github.com' "$RECOVERY"; then
  fail 'gh api must use the full uploads.github.com URL'
fi

printf 'PASS: release packaging workflow\n'
