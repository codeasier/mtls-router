#!/usr/bin/env bash
set -euo pipefail

desktop_dir="$(cd "$(dirname "$0")/.." && pwd)"
version="${1:-}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]] || {
  printf 'desktop version must be a SemVer value without a v prefix: %s\n' "$version" >&2
  exit 1
}

VERSION="$version" DESKTOP_DIR="$desktop_dir" node <<'NODE'
import fs from "node:fs";
import path from "node:path";

const root = process.env.DESKTOP_DIR;
const version = process.env.VERSION;
for (const name of ["package.json", "package-lock.json", "src-tauri/tauri.conf.json"]) {
  const file = path.join(root, name);
  const value = JSON.parse(fs.readFileSync(file, "utf8"));
  value.version = version;
  if (name === "package-lock.json" && value.packages?.[""]) {
    value.packages[""].version = version;
  }
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

for (const name of ["src-tauri/Cargo.toml", "src-tauri/Cargo.lock"]) {
  const file = path.join(root, name);
  let content = fs.readFileSync(file, "utf8");
  const pattern = name.endsWith("Cargo.toml")
    ? /^(\[package\]\nname = "mtls-router-desktop"\nversion = ")[^"]+/m
    : /(\[\[package\]\]\nname = "mtls-router-desktop"\nversion = ")[^"]+/;
  if (!pattern.test(content)) throw new Error(`desktop package entry not found in ${name}`);
  const updated = content.replace(pattern, `$1${version}`);
  fs.writeFileSync(file, updated);
}
NODE

(cd "$desktop_dir" && npm exec prettier -- --write package.json package-lock.json src-tauri/tauri.conf.json >/dev/null)

grep -Fq "version = \"$version\"" "$desktop_dir/src-tauri/Cargo.toml"
grep -Fq "version = \"$version\"" "$desktop_dir/src-tauri/Cargo.lock"
