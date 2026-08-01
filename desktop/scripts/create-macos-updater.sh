#!/usr/bin/env bash
set -euo pipefail

desktop_dir="$(cd "$(dirname "$0")/.." && pwd)"
target="${1:?Tauri target is required}"
bundle_root="$desktop_dir/src-tauri/target/$target/release/bundle"
app="$bundle_root/macos/CodeasierRouter.app"
archive="$bundle_root/macos/CodeasierRouter.app.tar.gz"

[[ "${ONLINE_UPDATE:-false}" == true ]] || exit 0
[[ -d "$app" ]] || { printf 'macOS updater app bundle is missing\n' >&2; exit 1; }
: "${TAURI_SIGNING_PRIVATE_KEY:?TAURI_SIGNING_PRIVATE_KEY is required}"
: "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:?TAURI_SIGNING_PRIVATE_KEY_PASSWORD is required}"

rm -f "$bundle_root/macos"/*.app.tar.gz "$bundle_root/macos"/*.app.tar.gz.sig
COPYFILE_DISABLE=1 tar -czf "$archive" -C "$bundle_root/macos" CodeasierRouter.app
(cd "$desktop_dir" && npm exec tauri -- signer sign "$archive")
[[ -s "$archive.sig" ]] || { printf 'macOS updater signature is missing\n' >&2; exit 1; }
