#!/usr/bin/env bash
set -euo pipefail

desktop_dir="$(cd "$(dirname "$0")/.." && pwd)"
target="${1:-${TAURI_ENV_TARGET_TRIPLE:-${TARGET:-$(rustc --print host-tuple)}}}"
expected_version="${VERSION:-$(cd "$desktop_dir" && node -p "require('./package.json').version")}"
expected_deployment="${DEPLOYMENT_ID:-dev}"
expected_protocol="${MANAGEMENT_PROTOCOL_VERSION:-3}"
bundle_root="$desktop_dir/src-tauri/target/$target/release/bundle"
output_dir="${PACKAGE_OUTPUT_DIR:-$desktop_dir/release-artifacts}"
work="$(mktemp -d "${TMPDIR:-/tmp}/mtls-router-package.XXXXXX")"
mounted_dmg=
cleanup() {
  if [[ -n "$mounted_dmg" ]]; then
    hdiutil detach "$mounted_dmg" -quiet 2>/dev/null || \
      hdiutil detach "$mounted_dmg" -force -quiet 2>/dev/null || true
  fi
  rm -rf "$work" 2>/dev/null || true
}
trap cleanup EXIT

case "$target" in
  aarch64-apple-darwin) os=darwin; arch=arm64; package_kind=dmg; extension= ;;
  x86_64-apple-darwin) os=darwin; arch=amd64; package_kind=dmg; extension= ;;
  aarch64-unknown-linux-gnu) os=linux; arch=arm64; package_kind=appimage; extension= ;;
  x86_64-unknown-linux-gnu) os=linux; arch=amd64; package_kind=appimage; extension= ;;
  aarch64-pc-windows-msvc) os=windows; arch=arm64; package_kind=nsis; extension=.exe ;;
  x86_64-pc-windows-msvc) os=windows; arch=amd64; package_kind=nsis; extension=.exe ;;
  *) printf 'unsupported desktop target: %s\n' "$target" >&2; exit 1 ;;
esac

host="$(rustc --print host-tuple)"
[[ "$host" == "$target" ]] || {
  printf 'package inspection requires a native runner: host=%s target=%s\n' "$host" "$target" >&2
  exit 1
}

router_source="$desktop_dir/src-tauri/binaries/mtls-router-$target$extension"
manager_source="$desktop_dir/src-tauri/binaries/mtls-router-manager-$target$extension"
[[ -f "$router_source" && -f "$manager_source" ]] || { printf 'target sidecars are missing\n' >&2; exit 1; }

actual_router_version="$($router_source -version | tr -d '\r\n')"
[[ "$actual_router_version" == "mtls-router $expected_version" ]] || {
  printf 'router version mismatch: %s\n' "$actual_router_version" >&2
  exit 1
}
manager_info="$(printf '{\"id\":\"package\",\"method\":\"manager.info\"}\n' | \
  MTLS_ROUTER_DESKTOP_DATA_DIR="$work/manager-data" \
  "$manager_source" serve)"
MANAGER_INFO="$manager_info" EXPECTED_VERSION="$expected_version" EXPECTED_DEPLOYMENT="$expected_deployment" EXPECTED_PROTOCOL="$expected_protocol" EXPECTED_TARGET="$os/$arch" node <<'NODE'
const response = JSON.parse(process.env.MANAGER_INFO);
if (response.error) throw new Error(`manager.info failed: ${response.error.code}`);
const result = response.result;
for (const [name, actual, expected] of [
  ["version", result.version, process.env.EXPECTED_VERSION],
  ["deployment", result.deployment_id, process.env.EXPECTED_DEPLOYMENT],
  ["target", result.target, process.env.EXPECTED_TARGET],
  ["protocol", result.management_protocol_version, process.env.EXPECTED_PROTOCOL],
]) {
  if (actual !== expected) throw new Error(`manager ${name} mismatch: ${actual} != ${expected}`);
}
NODE

case "$os" in
  darwin)
    package="$(set -- "$bundle_root/dmg"/*.dmg; [[ $# -eq 1 && -f "$1" ]] || exit 1; printf '%s' "$1")"
    mounted_dmg="$work/dmg"
    mkdir "$mounted_dmg"
    hdiutil attach -readonly -nobrowse -mountpoint "$mounted_dmg" "$package" >/dev/null
    app="$mounted_dmg/CodeasierRouter.app"
    [[ -d "$app" ]] || { printf 'DMG is missing the app bundle\n' >&2; exit 1; }
    applications_link="$mounted_dmg/Applications"
    [[ -L "$applications_link" ]] || { printf 'DMG is missing the Applications symbolic link\n' >&2; exit 1; }
    [[ "$(readlink "$applications_link")" == /Applications ]] || {
      printf 'DMG Applications symbolic link has an invalid target\n' >&2
      exit 1
    }
    codesign --verify --deep --strict --verbose=2 "$app"
    packaged_dir="$app/Contents/MacOS"
    packaged_desktop="$packaged_dir/mtls-router-desktop"
    plist_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")"
    [[ "$plist_version" == "$expected_version" ]] || { printf 'DMG app version mismatch\n' >&2; exit 1; }
    ;;
  linux)
    package="$(set -- "$bundle_root/appimage"/*.AppImage; [[ $# -eq 1 && -f "$1" ]] || exit 1; printf '%s' "$1")"
    (cd "$work" && "$package" --appimage-extract >/dev/null)
    packaged_dir="$work/squashfs-root/usr/bin"
    packaged_desktop="$(find "$work/squashfs-root" -type f -name mtls-router-desktop -print -quit)"
    ;;
  windows)
    package="$(set -- "$bundle_root/nsis"/*-setup.exe; [[ $# -eq 1 && -f "$1" ]] || exit 1; printf '%s' "$1")"
    7z x -y "-o$work/nsis" "$package" >/dev/null
    packaged_dir="$work/nsis"
    packaged_desktop="$(find "$packaged_dir" -type f -name mtls-router-desktop.exe -print -quit)"
    ;;
esac

find_packaged() {
  local name="$1" match
  match="$(find "$packaged_dir" -type f -name "$name" -print -quit)"
  [[ -n "$match" ]] || { printf 'package is missing %s\n' "$name" >&2; exit 1; }
  printf '%s' "$match"
}

sha256() {
  node -e 'const fs=require("node:fs"),crypto=require("node:crypto"); process.stdout.write(crypto.createHash("sha256").update(fs.readFileSync(process.argv[1])).digest("hex"))' "$1"
}

packaged_router="$(find_packaged "mtls-router$extension")"
packaged_manager="$(find_packaged "mtls-router-manager$extension")"
[[ -n "$packaged_desktop" && -f "$packaged_desktop" ]] || { printf 'package is missing the desktop executable\n' >&2; exit 1; }
[[ "$(basename "$package")" == *"$expected_version"* ]] || { printf 'package filename does not contain the desktop version\n' >&2; exit 1; }
[[ "$(sha256 "$packaged_router")" == "$(sha256 "$router_source")" ]]
[[ "$(sha256 "$packaged_manager")" == "$(sha256 "$manager_source")" ]]

if [[ "$os" != windows ]]; then
  [[ -x "$packaged_desktop" && -x "$packaged_router" && -x "$packaged_manager" ]] || { printf 'packaged executables have invalid permissions\n' >&2; exit 1; }
fi

"$packaged_desktop" --verify-manager-handshake

PACKAGED_DESKTOP="$packaged_desktop" PACKAGED_ROUTER="$packaged_router" PACKAGED_MANAGER="$packaged_manager" \
  EXPECTED_DEPLOYMENT="$expected_deployment" EXPECTED_TARGET="$target" EXPECTED_VERSION="$expected_version" node <<'NODE'
import fs from "node:fs";

const target = process.env.EXPECTED_TARGET;
const expectedArch = target.startsWith("aarch64-") ? "arm64" : "amd64";
const expectedFormat = target.includes("windows") ? "pe" : target.includes("apple") ? "macho" : "elf";
const binaries = [
  ["desktop", process.env.PACKAGED_DESKTOP],
  ["router", process.env.PACKAGED_ROUTER],
  ["manager", process.env.PACKAGED_MANAGER],
];

function identify(bytes) {
  if (bytes.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) {
    const machine = bytes.readUInt16LE(18);
    return ["elf", machine === 0xb7 ? "arm64" : machine === 0x3e ? "amd64" : "unknown"];
  }
  if (bytes.readUInt32LE(0) === 0xfeedfacf) {
    const cpu = bytes.readUInt32LE(4);
    return ["macho", cpu === 0x0100000c ? "arm64" : cpu === 0x01000007 ? "amd64" : "unknown"];
  }
  if (bytes.subarray(0, 2).toString("ascii") === "MZ") {
    const pe = bytes.readUInt32LE(0x3c);
    if (bytes.subarray(pe, pe + 4).toString("binary") !== "PE\0\0") return ["invalid", "unknown"];
    const machine = bytes.readUInt16LE(pe + 4);
    return ["pe", machine === 0xaa64 ? "arm64" : machine === 0x8664 ? "amd64" : "unknown"];
  }
  return ["invalid", "unknown"];
}

function peSubsystem(bytes) {
  const pe = bytes.readUInt32LE(0x3c);
  if (bytes.subarray(pe, pe + 4).toString("binary") !== "PE\0\0") return null;
  const optionalHeader = pe + 24;
  const magic = bytes.readUInt16LE(optionalHeader);
  if (magic !== 0x10b && magic !== 0x20b) return null;
  return bytes.readUInt16LE(optionalHeader + 68);
}

for (const [name, path] of binaries) {
  const bytes = fs.readFileSync(path);
  const [format, arch] = identify(bytes);
  if (format !== expectedFormat || arch !== expectedArch) {
    throw new Error(`${name} format/architecture mismatch: ${format}/${arch}`);
  }
  if (!bytes.includes(Buffer.from(process.env.EXPECTED_DEPLOYMENT))) {
    throw new Error(`${name} deployment identity is missing`);
  }
  if (!bytes.includes(Buffer.from(process.env.EXPECTED_VERSION))) {
    throw new Error(`${name} version identity is missing`);
  }
  if (name === "desktop" && !bytes.includes(Buffer.from(target))) {
    throw new Error("desktop target identity is missing");
  }
  if (name === "desktop" && expectedFormat === "pe" && peSubsystem(bytes) !== 2) {
    throw new Error("desktop PE subsystem is not IMAGE_SUBSYSTEM_WINDOWS_GUI");
  }
}
NODE

mkdir -p "$output_dir"
case "$package_kind" in dmg) suffix=dmg ;; appimage) suffix=AppImage ;; nsis) suffix=exe ;; esac
artifact="$output_dir/CodeasierRouter-$os-$arch.$suffix"
cp "$package" "$artifact"
hash="$(sha256 "$artifact")"
printf '%s  %s\n' "$hash" "$(basename "$artifact")" >"$output_dir/CodeasierRouter-$os-$arch.sha256"
printf 'validated target=%s version=%s deployment_id=%s protocol=%s package=%s\n' "$target" "$expected_version" "$expected_deployment" "$expected_protocol" "$(basename "$artifact")"
