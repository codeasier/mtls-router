#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CI="$ROOT/.github/workflows/ci.yml"
RELEASE="$ROOT/.github/workflows/release.yml"
PACKAGE="$ROOT/desktop/package.json"
LOCK="$ROOT/desktop/package-lock.json"
SIDECARS="$ROOT/desktop/scripts/build-sidecars.sh"
PREPARE="$ROOT/desktop/scripts/prepare-version.sh"
CONFIG="$ROOT/desktop/src-tauri/tauri.conf.json"
HOOKS="$ROOT/desktop/src-tauri/windows/uninstall-hooks.nsh"

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
contains() { grep -Fq -- "$2" "$1" || fail "$(basename "$1") missing: $2"; }

node - "$PACKAGE" "$LOCK" <<'NODE' || fail 'desktop package and lockfile metadata is inconsistent'
const fs = require('node:fs');

const [packagePath, lockPath] = process.argv.slice(2);
const packageJSON = JSON.parse(fs.readFileSync(packagePath, 'utf8'));
const lockJSON = JSON.parse(fs.readFileSync(lockPath, 'utf8'));
const expected = {
  '@emnapi/core': '1.11.2',
  '@emnapi/runtime': '1.11.2',
};

if (packageJSON.packageManager !== 'npm@11.6.2') {
  throw new Error('package.json must declare npm@11.6.2');
}
for (const [name, version] of Object.entries(expected)) {
  if (packageJSON.devDependencies?.[name] !== version) {
    throw new Error(`${name} is not an exact package.json devDependency`);
  }
  if (lockJSON.packages?.['']?.devDependencies?.[name] !== version) {
    throw new Error(`${name} is missing from root lockfile metadata`);
  }
  if (lockJSON.packages?.[`node_modules/${name}`]?.version !== version) {
    throw new Error(`${name} is missing its exact top-level lockfile record`);
  }
}
NODE

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
contains "$CI" 'sudo apt-get install -y libappindicator3-dev librsvg2-dev libwebkit2gtk-4.1-dev xdg-utils'
contains "$RELEASE" 'sudo apt-get install -y libappindicator3-dev librsvg2-dev libwebkit2gtk-4.1-dev xdg-utils'
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
contains "$ROOT/desktop/scripts/build-sidecars.sh" 'node -p "require('\''./package.json'\'').version"'
contains "$ROOT/desktop/scripts/verify-package.sh" 'node -p "require('\''./package.json'\'').version"'
contains "$PREPARE" 'const root = process.cwd();'
contains "$ROOT/desktop/scripts/verify-package.sh" 'expected_protocol="${MANAGEMENT_PROTOCOL_VERSION:-1}"'
contains "$ROOT/desktop/src-tauri/build.rs" 'BinaryFormat::Pe'
contains "$ROOT/desktop/src-tauri/src/sidecar.rs" 'BinaryFormat::Pe'
if grep -Fq 'BinaryFormat::Coff' "$ROOT/desktop/src-tauri/build.rs" "$ROOT/desktop/src-tauri/src/sidecar.rs"; then
  fail 'Windows sidecar validation must recognize PE executables'
fi
for script in "$ROOT/desktop/scripts/build-sidecars.sh" "$ROOT/desktop/scripts/verify-package.sh" "$PREPARE"; do
  if grep -Fq '$desktop_dir/package.json' "$script"; then
    fail "$(basename "$script") must not pass a Git Bash path to Node"
  fi
done
if grep -Fq 'DESKTOP_DIR="$desktop_dir"' "$PREPARE"; then
  fail 'prepare-version.sh must not pass a Git Bash path to Node'
fi

placeholder_req="$(awk '/openssl req -x509/{print; count++} END{if (count != 1) exit 1}' "$SIDECARS")" || \
  fail 'sidecar script must have one placeholder openssl req invocation'
[[ "$placeholder_req" == *"MSYS2_ARG_CONV_EXCL='/CN=' openssl req"* ]] || \
  fail 'MSYS2_ARG_CONV_EXCL must be command-scoped to placeholder openssl req'
[[ "$placeholder_req" == *'-keyout "$key" -out "$cert" -subj /CN=mtls-router-placeholder'* ]] || \
  fail 'placeholder openssl output paths or subject changed unexpectedly'
[[ "$(awk '/MSYS2_ARG_CONV_EXCL=/{n++} END{print n+0}' "$SIDECARS")" -eq 1 ]] || \
  fail 'sidecar script must set MSYS2_ARG_CONV_EXCL only once'
if awk '/MSYS2_ARG_CONV_EXCL=/ && /\*/{found=1} END{exit !found}' "$SIDECARS"; then
  fail 'sidecar script must not use wildcard MSYS2 conversion exclusion'
fi

job_block() {
  local workflow=$1
  local job=$2
  awk -v job="$job" '
    $0 == "  " job ":" { capture=1; start=NR }
    capture && NR > start && $0 ~ /^  [A-Za-z0-9_-]+:/ { exit }
    capture { print }
  ' "$workflow"
}

assert_npm_job() {
  local label=$1
  local block=$2
  local setup_line activation_line ci_line activation_block

  [[ "$(printf '%s\n' "$block" | awk '/uses: actions\/setup-node@/{n++} END{print n+0}')" -eq 1 ]] || \
    fail "$label must set up Node exactly once"
  [[ "$(printf '%s\n' "$block" | awk '/name: Use declared npm version/{n++} END{print n+0}')" -eq 1 ]] || \
    fail "$label must activate npm exactly once"
  [[ "$(printf '%s\n' "$block" | awk '/^[[:space:]]+- run: npm ci$/{n++} END{print n+0}')" -eq 1 ]] || \
    fail "$label must run npm ci exactly once"

  setup_line="$(printf '%s\n' "$block" | awk '/uses: actions\/setup-node@/{print NR; exit}')"
  activation_line="$(printf '%s\n' "$block" | awk '/name: Use declared npm version/{print NR; exit}')"
  ci_line="$(printf '%s\n' "$block" | awk '/^[[:space:]]+- run: npm ci$/{print NR; exit}')"
  [[ "$setup_line" -lt "$activation_line" && "$activation_line" -lt "$ci_line" ]] || \
    fail "$label npm activation must be after setup-node and before npm ci"

  activation_block="$(printf '%s\n' "$block" | awk '
    /- name: Use declared npm version$/ { capture=1; seen=0 }
    capture && seen && /^[[:space:]]+- name:/ { exit }
    capture { print; seen=1 }
  ')"
  [[ "$activation_block" == *'packageManager'* ]] || \
    fail "$label npm activation must derive its version from packageManager"
  [[ "$activation_block" == *'npm install --global "npm@${npm_version}"'* ]] || \
    fail "$label npm activation must install the declared version exactly"
  [[ "$activation_block" == *'test "$(npm --version)" = "$npm_version"'* ]] || \
    fail "$label npm activation must verify its exact version"

  if [[ "$label" == frontend ]]; then
    [[ "$block" == *'working-directory: desktop'* ]] || \
      fail 'frontend npm activation must run in the desktop context'
  else
    [[ "$activation_block" == *'working-directory: desktop'* ]] || \
      fail "$label npm activation must set working-directory: desktop"
  fi
}

ci_frontend_block="$(job_block "$CI" frontend)"
ci_package_block="$(job_block "$CI" desktop-package)"
release_desktop_block="$(job_block "$RELEASE" desktop)"
[[ "$(awk '/^[[:space:]]+- run: npm ci$/{n++} END{print n+0}' "$CI")" -eq 2 ]] || \
  fail 'CI must contain exactly two npm ci commands'
[[ "$(awk '/^[[:space:]]+- run: npm ci$/{n++} END{print n+0}' "$RELEASE")" -eq 1 ]] || \
  fail 'release must contain exactly one npm ci command'
assert_npm_job frontend "$ci_frontend_block"
assert_npm_job desktop-package "$ci_package_block"
assert_npm_job release-desktop "$release_desktop_block"

for metadata in 'VERSION: 0.1.0' 'DEPLOYMENT_ID: dev' "MANAGEMENT_PROTOCOL_VERSION: '1'"; do
  [[ "$ci_package_block" == *"$metadata"* ]] || fail "desktop-package job missing inherited $metadata"
done
if [[ "$(printf '%s' "$ci_package_block" | grep -Fc 'VERSION: 0.1.0')" -ne 1 ]]; then
  fail 'CI desktop VERSION must be defined once at package job scope'
fi

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
