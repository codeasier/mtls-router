#!/usr/bin/env bash
set -euo pipefail

metadata_dir="${1:?metadata directory is required}"
expected_protocol="${EXPECTED_MANAGEMENT_PROTOCOL_VERSION:-4}"
# One producer per desktop target; new releases carry no CLI producers.
expected_count="${EXPECTED_RELEASE_PRODUCERS:-6}"

metadata_count="$(find "$metadata_dir" -maxdepth 1 -type f -name 'release-metadata-*.json' | wc -l | tr -d ' ')"
[[ "$metadata_count" -eq "$expected_count" ]] || {
  printf 'release preflight requires %s protocol metadata files, found %s\n' "$expected_count" "$metadata_count" >&2
  exit 1
}

jq -se --arg protocol "$expected_protocol" --argjson count "$expected_count" '
  length == $count and
  all(
    .schema_version == 1 and
    .management_protocol_version == $protocol and
    (.producer | type == "string" and length > 0) and
    (.producer | startswith("desktop-"))
  ) and
  ([.[].producer] | unique | length == $count)
' "$metadata_dir"/release-metadata-*.json >/dev/null || {
  printf 'invalid, duplicate, mixed, or non-desktop release protocol metadata\n' >&2
  exit 1
}
