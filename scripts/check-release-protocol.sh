#!/usr/bin/env bash
set -euo pipefail

metadata_dir="${1:?metadata directory is required}"
expected_protocol="${EXPECTED_MANAGEMENT_PROTOCOL_VERSION:-2}"

metadata_count="$(find "$metadata_dir" -maxdepth 1 -type f -name 'release-metadata-*.json' | wc -l | tr -d ' ')"
[[ "$metadata_count" -eq 12 ]] || {
  printf 'release preflight requires 12 protocol metadata files, found %s\n' "$metadata_count" >&2
  exit 1
}

jq -se --arg protocol "$expected_protocol" '
  length == 12 and
  all(
    .schema_version == 1 and
    .management_protocol_version == $protocol and
    (.producer | type == "string" and length > 0)
  ) and
  ([.[].producer] | unique | length == 12)
' "$metadata_dir"/release-metadata-*.json >/dev/null || {
  printf 'invalid, duplicate, or mixed release protocol metadata\n' >&2
  exit 1
}
