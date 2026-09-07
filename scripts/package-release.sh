#!/usr/bin/env bash
# Assembles the release directory for a new version. Only the desktop
# application, its updater artifacts, per-target signing status, checksums,
# and (for stable tags) the updater feed are published. CLI router/manager
# binaries, setup scripts, and service wrappers are frozen at their historical
# tags and must never re-enter a new release; any file outside the allowlist
# fails packaging.
set -euo pipefail

release_tag="${RELEASE_TAG:-}"
online_update=false
if [[ "$release_tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then online_update=true; fi
if [[ "$online_update" == true ]]; then
  : "${DESKTOP_DOWNLOAD_BASE_URL:?DESKTOP_DOWNLOAD_BASE_URL is required for stable releases}"
  case "$DESKTOP_DOWNLOAD_BASE_URL" in https://*) ;; *) printf 'DESKTOP_DOWNLOAD_BASE_URL must use HTTPS\n' >&2; exit 1 ;; esac
fi

./scripts/check-release-protocol.sh protocol-metadata

# Every published file must match exactly one allowlisted shape.
allowed_asset() {
  case "$1" in
    CodeasierRouter-darwin-amd64.dmg|CodeasierRouter-darwin-arm64.dmg) return 0 ;;
    CodeasierRouter-linux-amd64.AppImage|CodeasierRouter-linux-arm64.AppImage) return 0 ;;
    CodeasierRouter-windows-amd64.exe|CodeasierRouter-windows-arm64.exe) return 0 ;;
    CodeasierRouter-darwin-amd64.app.tar.gz|CodeasierRouter-darwin-arm64.app.tar.gz) return 0 ;;
    CodeasierRouter-*-*.sha256) return 0 ;;
    CodeasierRouter-*.sig) return 0 ;;
    signing-status-*-*.txt) return 0 ;;
    SHA256SUMS|latest.json) return 0 ;;
    *) return 1 ;;
  esac
}

forbidden_asset() {
  case "$1" in
    mtls-router*|setup.sh|setup.ps1|*.zip|*.tar.gz|*.service|Dockerfile|*.nsh) return 0 ;;
    *) return 1 ;;
  esac
}

mkdir -p release
(cd desktop-packages && sha256sum -c CodeasierRouter-*.sha256)
while IFS= read -r -d '' path; do
  name="$(basename "$path")"
  if forbidden_asset "$name" && [[ "$name" != CodeasierRouter-darwin-*.app.tar.gz ]]; then
    printf 'CLI lifecycle artifact must not be published: %s\n' "$name" >&2
    exit 1
  fi
  allowed_asset "$name" || { printf 'artifact is not on the release allowlist: %s\n' "$name" >&2; exit 1; }
  cp "$path" release/
done < <(find desktop-packages -maxdepth 1 -type f ! -name 'release-metadata-*.json' -print0)

expected_desktop_assets=12
if [[ "$online_update" == true ]]; then expected_desktop_assets=20; fi
test "$(find release -maxdepth 1 -type f -name 'CodeasierRouter-*' | wc -l)" -eq "$expected_desktop_assets"
test "$(find release -maxdepth 1 -type f -name 'signing-status-*' | wc -l)" -eq 6
test "$(find release -maxdepth 1 -type f -name 'mtls-router*' | wc -l)" -eq 0

if [[ "$online_update" == true ]]; then
  test "$(find release -maxdepth 1 -type f -name 'CodeasierRouter-*.sig' | wc -l)" -eq 6
  test "$(find release -maxdepth 1 -type f -name 'CodeasierRouter-darwin-*.app.tar.gz' | wc -l)" -eq 2
  RELEASE_TAG="$release_tag" DESKTOP_DOWNLOAD_BASE_URL="$DESKTOP_DOWNLOAD_BASE_URL" python3 - <<'PY'
import json
import os
from pathlib import Path

release = Path("release")
tag = os.environ["RELEASE_TAG"]
version = tag.removeprefix("v")
base_url = os.environ["DESKTOP_DOWNLOAD_BASE_URL"].rstrip("/")
targets = {
    "linux-x86_64": "CodeasierRouter-linux-amd64.AppImage",
    "linux-aarch64": "CodeasierRouter-linux-arm64.AppImage",
    "windows-x86_64": "CodeasierRouter-windows-amd64.exe",
    "windows-aarch64": "CodeasierRouter-windows-arm64.exe",
    "darwin-x86_64": "CodeasierRouter-darwin-amd64.app.tar.gz",
    "darwin-aarch64": "CodeasierRouter-darwin-arm64.app.tar.gz",
}
platforms = {}
for target, name in targets.items():
    artifact = release / name
    signature = release / f"{name}.sig"
    if not artifact.is_file() or not signature.is_file():
        raise SystemExit(f"missing updater artifact pair: {name}")
    signature_text = signature.read_text(encoding="utf-8").strip()
    if not signature_text:
        raise SystemExit(f"empty updater signature: {signature.name}")
    platforms[target] = {
        "signature": signature_text,
        "url": f"{base_url}/{name}",
    }

feed = {"version": version, "platforms": platforms}
(release / "latest.json").write_text(json.dumps(feed, indent=2) + "\n", encoding="utf-8")
PY
  jq -e --arg version "${release_tag#v}" '
    .version == $version and
    (.platforms | keys | sort) == ([
      "darwin-aarch64", "darwin-x86_64", "linux-aarch64",
      "linux-x86_64", "windows-aarch64", "windows-x86_64"
    ] | sort) and
    ([.platforms[] | select((.signature | length) == 0 or (.url | startswith("https://") | not))] | length) == 0
  ' release/latest.json >/dev/null
fi

find release -maxdepth 1 -type f ! -name SHA256SUMS ! -name 'signing-status-*' -print0 | LC_ALL=C sort -z | xargs -0 sha256sum | sed 's#  release/#  #' >release/SHA256SUMS
expected_checksums=12
expected_release_files=19
if [[ "$online_update" == true ]]; then
  expected_checksums=21
  expected_release_files=28
fi
test "$(awk '$1 ~ /^[0-9a-f]{64}$/ { print $2 }' release/SHA256SUMS | sort -u | wc -l)" -eq "$expected_checksums"
test "$(find release -maxdepth 1 -type f | wc -l)" -eq "$expected_release_files"
while IFS= read -r -d '' path; do
  allowed_asset "$(basename "$path")" || { printf 'unexpected release file: %s\n' "$path" >&2; exit 1; }
done < <(find release -maxdepth 1 -type f -print0)
