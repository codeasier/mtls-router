#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_setup() {
  # Source setup.sh without running main.
  local sourced
  sourced="$(mktemp)"
  sed '/^main "\$@"$/d' "$ROOT/setup.sh" >"$sourced"
  # shellcheck source=/dev/null
  source "$sourced"
  rm -f "$sourced"
}

test_latest_version_falls_back_to_web_redirect_when_api_rate_limited() {
  source_setup
  DOWNLOADER="curl"

  curl() {
    local url="${*: -1}"
    case "$url" in
      https://api.github.com/repos/codeasier/mtls-router/releases/latest)
        return 22
        ;;
      https://github.com/codeasier/mtls-router/releases/latest)
        printf '%s\n' 'location: https://github.com/codeasier/mtls-router/releases/tag/v0.1.0'
        return 0
        ;;
      *)
        printf 'unexpected url: %s\n' "$url" >&2
        return 99
        ;;
    esac
  }

  local got
  got="$(latest_version)"
  [[ "$got" == "v0.1.0" ]] || {
    printf 'got %q, want v0.1.0\n' "$got" >&2
    return 1
  }
}

test_latest_version_falls_back_to_web_redirect_when_api_rate_limited
