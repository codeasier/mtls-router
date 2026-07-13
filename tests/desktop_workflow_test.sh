#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CI="$ROOT/.github/workflows/ci.yml"
RELEASE="$ROOT/.github/workflows/release.yml"
CONFIG="$ROOT/desktop/src-tauri/tauri.conf.json"
HOOKS="$ROOT/desktop/src-tauri/windows/uninstall-hooks.nsh"

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
contains() { grep -Fq -- "$2" "$1" || fail "$(basename "$1") missing: $2"; }

for target in \
  x86_64-pc-windows-msvc aarch64-pc-windows-msvc \
  x86_64-apple-darwin aarch64-apple-darwin \
  x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
  contains "$CI" "target: $target"
  contains "$RELEASE" "target: $target"
done

for runner in windows-2025 windows-11-arm macos-15-intel macos-15 ubuntu-24.04 ubuntu-24.04-arm; do
  contains "$CI" "runner: $runner"
  contains "$RELEASE" "runner: $runner"
done

for command in 'npm run static:check' 'npm run typecheck' 'npm test' 'npm run build' \
  'cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all -- --check' \
  'cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked'; do
  contains "$CI" "$command"
done
contains "$CI" 'npm exec tauri -- build --target ${{ matrix.target }} --bundles ${{ matrix.bundles }} --no-sign --ci'
contains "$CI" './desktop/scripts/verify-package.sh ${{ matrix.target }}'
for metadata in 'VERSION: 0.1.0' 'DEPLOYMENT_ID: dev' "MANAGEMENT_PROTOCOL_VERSION: '1'"; do
  contains "$CI" "$metadata"
done
contains "$RELEASE" "MANAGEMENT_PROTOCOL_VERSION: '1'"
contains "$RELEASE" "printf 'VERSION=%s\\n' \"\$version\" >>\"\$GITHUB_ENV\""
contains "$ROOT/desktop/scripts/verify-package.sh" '"$packaged_desktop" --verify-manager-handshake'
contains "$ROOT/desktop/src-tauri/src/main.rs" '"--verify-manager-handshake"'
contains "$ROOT/desktop/src-tauri/src/main.rs" 'verify_manager_handshake()'
contains "$ROOT/desktop/scripts/build-sidecars.sh" 'management_protocol_version="${MANAGEMENT_PROTOCOL_VERSION:-1}"'
contains "$ROOT/desktop/scripts/build-sidecars.sh" 'version="${VERSION:-$(node -p'
contains "$ROOT/desktop/scripts/verify-package.sh" 'expected_protocol="${MANAGEMENT_PROTOCOL_VERSION:-1}"'

ci_package_block="$(awk '/^  desktop-package:/{capture=1} capture{print} /^  workflow-static:/{exit}' "$CI")"
for metadata in 'VERSION: 0.1.0' 'DEPLOYMENT_ID: dev' "MANAGEMENT_PROTOCOL_VERSION: '1'"; do
  [[ "$ci_package_block" == *"$metadata"* ]] || fail "desktop-package job missing inherited $metadata"
done
if [[ "$(printf '%s' "$ci_package_block" | grep -Fc 'VERSION: 0.1.0')" -ne 1 ]]; then
  fail 'CI desktop VERSION must be defined once at package job scope'
fi

release_desktop_block="$(awk '/^  desktop:/{capture=1} capture{print} /^  release:/{exit}' "$RELEASE")"
for metadata in 'DEPLOYMENT_ID: ${{ vars.DEPLOYMENT_ID }}' "MANAGEMENT_PROTOCOL_VERSION: '1'"; do
  [[ "$release_desktop_block" == *"$metadata"* ]] || fail "release desktop job missing inherited $metadata"
done
[[ "$release_desktop_block" == *"printf 'VERSION=%s\\n' \"\$version\" >>\"\$GITHUB_ENV\""* ]] || \
  fail 'release desktop version is not propagated to later build and verification steps'

pull_request_block="$(awk '/^  pull_request:/{capture=1} capture{print} /^jobs:/{exit}' "$CI")"
[[ "$pull_request_block" == *'pull_request:'* ]] || fail 'CI is not enabled for pull requests'
if grep -Eq 'softprops/action-gh-release|ssh-deploy|appleboy/ssh-action' "$CI"; then
  fail 'pull-request CI contains publishing actions'
fi

contains "$CONFIG" '"createUpdaterArtifacts": false'
contains "$CONFIG" '"installMode": "currentUser"'
contains "$CONFIG" '"installerHooks": "./windows/uninstall-hooks.nsh"'
contains "$HOOKS" 'DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "mtls-router-desktop"'
contains "$HOOKS" 'DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "mtls-router-desktop"'
if grep -Eqi 'agent|backup|logs?|state' "$HOOKS"; then
  fail 'Windows uninstall hook must not touch retained user data'
fi

contains "$RELEASE" 'Create signed macOS package'
contains "$RELEASE" 'Build signed Windows package'
contains "$RELEASE" 'status="unsigned (credentials unavailable)"'
contains "$RELEASE" 'status="signed (notarization credentials unavailable)"'
contains "$RELEASE" 'status="signed and notarized"'
contains "$RELEASE" 'status="signed"'
contains "$RELEASE" 'status="unsigned (credentials unavailable)"'
contains "$RELEASE" 'needs: [build, desktop]'
contains "$RELEASE" '(cd desktop-packages && sha256sum -c CodeasierRouter-*.sha256)'

if grep -Eqi 'dmg.*(uninstall hook|delete hook)|appimage.*(uninstall hook|delete hook)' "$RELEASE" "$CONFIG"; then
  fail 'DMG/AppImage configuration claims an unavailable uninstall hook'
fi
if grep -Eqi 'updater|update endpoint' "$CI" "$RELEASE"; then
  fail 'workflow enables or references an updater'
fi
if grep -Fq 'tauri-plugin-updater' "$ROOT/desktop/src-tauri/Cargo.toml" "$ROOT/desktop/src-tauri/Cargo.lock"; then
  fail 'desktop workspace enables the Tauri updater plugin'
fi

printf 'PASS: desktop CI and release workflow configuration\n'
