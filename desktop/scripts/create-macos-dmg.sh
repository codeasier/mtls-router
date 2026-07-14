#!/usr/bin/env bash
set -euo pipefail

desktop_dir="$(cd "$(dirname "$0")/.." && pwd)"
target="${1:?usage: create-macos-dmg.sh <target> <version>}"
version="${2:?usage: create-macos-dmg.sh <target> <version>}"

[[ "$(uname -s)" == Darwin ]] || {
  printf 'macOS DMG creation requires a macOS host\n' >&2
  exit 1
}
command -v hdiutil >/dev/null || {
  printf 'hdiutil is required to create a macOS DMG\n' >&2
  exit 1
}

bundle_root="$desktop_dir/src-tauri/target/$target/release/bundle"
app="$bundle_root/macos/CodeasierRouter.app"
dmg_dir="$bundle_root/dmg"
dmg="$dmg_dir/CodeasierRouter_${version}.dmg"

[[ -d "$app" ]] || {
  printf 'macOS application bundle is missing: %s\n' "$app" >&2
  exit 1
}
rm -rf "$dmg_dir"
mkdir -p "$dmg_dir"
hdiutil create -volname CodeasierRouter -srcfolder "$app" -ov -format UDZO "$dmg"
[[ -f "$dmg" ]] || {
  printf 'DMG creation produced no output: %s\n' "$dmg" >&2
  exit 1
}
