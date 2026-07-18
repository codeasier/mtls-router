#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/.agents/skills/mtls-router-github-test-build/scripts/build-and-download.sh"

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
contains() { grep -Fq -- "$2" "$1" || fail "$(basename "$1") missing: $2"; }

[[ -x "$SCRIPT" ]] || fail 'GitHub test-build wrapper is missing or not executable'
help="$($SCRIPT --help)"
for text in '--target TARGET' '--upstream URL' 'default: all'; do
  [[ "$help" == *"$text"* ]] || fail "wrapper help missing: $text"
done

set +e
invalid_target="$($SCRIPT --ref main --version 0.2.0-test.1 --target freebsd-amd64 2>&1)"
invalid_target_status=$?
invalid_upstream="$($SCRIPT --ref main --version 0.2.0-test.1 --upstream http://router.example.com 2>&1)"
invalid_upstream_status=$?
unsafe_upstream="$($SCRIPT --ref main --version 0.2.0-test.1 --upstream "https://router.example.com/' -X unsafe=value" 2>&1)"
unsafe_upstream_status=$?
set -e
[[ "$invalid_target_status" -ne 0 && "$invalid_target" == *'invalid validation target'* ]] || \
  fail 'wrapper must reject unknown targets before GitHub access'
[[ "$invalid_upstream_status" -ne 0 && "$invalid_upstream" == *'validation upstream must use HTTPS'* ]] || \
  fail 'wrapper must reject non-HTTPS upstreams before GitHub access'
[[ "$unsafe_upstream_status" -ne 0 && "$unsafe_upstream" == *'must not contain whitespace or quotes'* ]] || \
  fail 'wrapper must reject linker-unsafe upstreams before GitHub access'

for value in \
  'all|windows-amd64|windows-arm64|darwin-amd64|darwin-arm64|linux-amd64|linux-arm64' \
  "'target:' 'upstream_url:'" \
  '-f "inputs[target]=$target"' \
  'dispatch_args+=(-f "inputs[upstream_url]=$upstream")'; do
  contains "$SCRIPT" "$value"
done

printf 'PASS: GitHub validation-build wrapper\n'
