#!/usr/bin/env bash
set -euo pipefail

case "${SIMPLIFY:-}" in
  '' | [Tt][Rr][Uu][Ee]) printf '%s\n' True ;;
  [Ff][Aa][Ll][Ss][Ee]) printf '%s\n' False ;;
  *)
    printf '%s\n' 'invalid SIMPLIFY value' >&2
    exit 1
    ;;
esac
