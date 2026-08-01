#!/usr/bin/env bash
set -euo pipefail

: "${DOWNLOAD_BASE_URL:?DOWNLOAD_BASE_URL is required}"
case "$DOWNLOAD_BASE_URL" in https://*) ;; *) printf 'DOWNLOAD_BASE_URL must use HTTPS\n' >&2; exit 1 ;; esac
release_tag="${RELEASE_TAG:-}"
online_update=false
if [[ "$release_tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then online_update=true; fi

test "$(grep -Fxc 'DEFAULT_DOWNLOAD_BASE_URL=""' setup.sh)" -eq 1
test "$(grep -Fxc '$DefaultDownloadBaseUrl = '\'''\''' setup.ps1)" -eq 1
test "$(od -An -tx1 -N3 setup.ps1 | tr -d ' \n')" = efbbbf
./scripts/check-release-protocol.sh protocol-metadata

mkdir -p release packages stage
cp binaries/mtls-router-* release/
test "$(find release -maxdepth 1 -type f -name 'mtls-router-*' | wc -l)" -eq 12
LC_ALL=C sha256sum release/mtls-router-* | sed 's#  release/#  #' | LC_ALL=C sort -k2 >release/SHA256SUMS
test "$(awk '$1 ~ /^[0-9a-f]{64}$/ && $2 ~ /^mtls-router(-manager)?-(linux|darwin)-(amd64|arm64)$/ || $1 ~ /^[0-9a-f]{64}$/ && $2 ~ /^mtls-router(-manager)?-windows-(amd64|arm64)\.exe$/ { print $2 }' release/SHA256SUMS | sort -u | wc -l)" -eq 12

cp setup.sh stage/setup.sh
cp setup.ps1 stage/setup.ps1
DOWNLOAD_BASE_URL="$DOWNLOAD_BASE_URL" python3 - <<'PY'
import os
from pathlib import Path

url = os.environ['DOWNLOAD_BASE_URL']
sh = Path('stage/setup.sh')
ps1 = Path('stage/setup.ps1')
sh_bytes = sh.read_bytes()
ps1_bytes = ps1.read_bytes()
sh_placeholder = b'DEFAULT_DOWNLOAD_BASE_URL=""'
ps1_placeholder = b"$DefaultDownloadBaseUrl = ''"
if sh_bytes.count(sh_placeholder) != 1 or ps1_bytes.count(ps1_placeholder) != 1:
    raise SystemExit('installer placeholder count changed')
sh.write_bytes(sh_bytes.replace(sh_placeholder, f'DEFAULT_DOWNLOAD_BASE_URL="{url}"'.encode()))
ps1.write_bytes(ps1_bytes.replace(ps1_placeholder, f"$DefaultDownloadBaseUrl = '{url}'".encode()))
PY
test "$(od -An -tx1 -N3 stage/setup.ps1 | tr -d ' \n')" = efbbbf
chmod 0755 stage/setup.sh

while read -r os arch router manager; do
  package="mtls-router-${os}-${arch}"
  rm -rf "stage/$package"
  mkdir "stage/$package"
  cp "release/$router" "release/$manager" release/SHA256SUMS "stage/$package/"
  if [ "$os" = windows ]; then
    cp stage/setup.ps1 "stage/$package/"
    touch -d @0 "stage/$package"/*
    (cd "stage/$package" && LC_ALL=C zip -X -q "../../packages/$package.zip" SHA256SUMS setup.ps1 "$router" "$manager")
    test "$(unzip -Z1 "packages/$package.zip" | wc -l)" -eq 4
  else
    cp stage/setup.sh "stage/$package/"
    chmod 0755 "stage/$package/setup.sh" "stage/$package/$router" "stage/$package/$manager"
    tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -czf "packages/$package.tar.gz" -C "stage/$package" .
    test "$(tar -tzf "packages/$package.tar.gz" | grep -Evc '^\./?$')" -eq 4
  fi
done <<'TARGETS'
linux amd64 mtls-router-linux-amd64 mtls-router-manager-linux-amd64
linux arm64 mtls-router-linux-arm64 mtls-router-manager-linux-arm64
darwin amd64 mtls-router-darwin-amd64 mtls-router-manager-darwin-amd64
darwin arm64 mtls-router-darwin-arm64 mtls-router-manager-darwin-arm64
windows amd64 mtls-router-windows-amd64.exe mtls-router-manager-windows-amd64.exe
windows arm64 mtls-router-windows-arm64.exe mtls-router-manager-windows-arm64.exe
TARGETS
cp packages/* release/
test "$(find release -maxdepth 1 -type f | wc -l)" -eq 19
(cd desktop-packages && sha256sum -c CodeasierRouter-*.sha256)
cp desktop-packages/CodeasierRouter-* release/
cp desktop-packages/signing-status-* release/
expected_desktop_assets=12
if [[ "$online_update" == true ]]; then expected_desktop_assets=20; fi
test "$(find release -maxdepth 1 -type f -name 'CodeasierRouter-*' | wc -l)" -eq "$expected_desktop_assets"
test "$(find release -maxdepth 1 -type f -name 'signing-status-*' | wc -l)" -eq 6

if [[ "$online_update" == true ]]; then
  test "$(find release -maxdepth 1 -type f -name 'CodeasierRouter-*.sig' | wc -l)" -eq 6
  test "$(find release -maxdepth 1 -type f -name 'CodeasierRouter-darwin-*.app.tar.gz' | wc -l)" -eq 2
  RELEASE_TAG="$release_tag" DOWNLOAD_BASE_URL="$DOWNLOAD_BASE_URL" python3 - <<'PY'
import json
import os
from pathlib import Path

release = Path("release")
tag = os.environ["RELEASE_TAG"]
version = tag.removeprefix("v")
base_url = os.environ["DOWNLOAD_BASE_URL"].rstrip("/")
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
expected_checksums=30
expected_release_files=37
if [[ "$online_update" == true ]]; then
  expected_checksums=39
  expected_release_files=46
fi
test "$(awk '$1 ~ /^[0-9a-f]{64}$/ { print $2 }' release/SHA256SUMS | sort -u | wc -l)" -eq "$expected_checksums"
test "$(find release -maxdepth 1 -type f | wc -l)" -eq "$expected_release_files"
