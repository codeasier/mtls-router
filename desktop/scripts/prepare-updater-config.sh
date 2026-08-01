#!/usr/bin/env bash
set -euo pipefail

output="${1:?output config path is required}"
enabled="${ONLINE_UPDATE:-false}"

if [[ "$enabled" == true ]]; then
  : "${TAURI_UPDATER_PUBKEY:?TAURI_UPDATER_PUBKEY is required for stable releases}"
  : "${TAURI_SIGNING_PRIVATE_KEY:?TAURI_SIGNING_PRIVATE_KEY is required for stable releases}"
  : "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:?TAURI_SIGNING_PRIVATE_KEY_PASSWORD is required for stable releases}"
  : "${UPDATER_ENDPOINT:?UPDATER_ENDPOINT is required for stable releases}"
  : "${TAURI_UPDATER_PUBKEY_SHA256:?TAURI_UPDATER_PUBKEY_SHA256 is required for stable releases}"
  [[ "$GITHUB_REF" =~ ^refs/tags/v[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
    printf 'online updates require a stable vX.Y.Z tag\n' >&2
    exit 1
  }
  [[ "$UPDATER_ENDPOINT" == https://* ]] || { printf 'UPDATER_ENDPOINT must use HTTPS\n' >&2; exit 1; }
fi

OUTPUT="$output" ENABLED="$enabled" node <<'NODE'
const fs = require("node:fs");
const crypto = require("node:crypto");

const config = { bundle: { createUpdaterArtifacts: process.env.ENABLED === "true" } };
if (process.env.ENABLED === "true") {
  const publicKey = process.env.TAURI_UPDATER_PUBKEY.trim();
  const fingerprint = crypto.createHash("sha256").update(publicKey).digest("hex");
  if (fingerprint !== process.env.TAURI_UPDATER_PUBKEY_SHA256) {
    throw new Error("TAURI_UPDATER_PUBKEY does not match the pinned fingerprint");
  }
  config.plugins = {
    updater: {
      pubkey: publicKey,
      endpoints: [process.env.UPDATER_ENDPOINT],
    },
  };
}
fs.writeFileSync(process.env.OUTPUT, `${JSON.stringify(config)}\n`, { mode: 0o600 });
NODE
