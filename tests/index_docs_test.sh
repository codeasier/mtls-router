#!/usr/bin/env bash
# Guards the INDEX.md convention documented in AGENTS.md:
#   1. every Go package under internal/ has its own INDEX.md
#   2. every internal/ top-level package is listed in the root INDEX.md
#   3. every internal/manager/ subpackage is listed in internal/manager/INDEX.md
#   4. every relative link in AGENTS.md and any INDEX.md resolves
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

failures=0
fail() {
  printf 'FAIL: %s\n' "$1" >&2
  failures=$((failures + 1))
}

# Package directories are those holding at least one .go file. testdata holds
# fixtures only and is deliberately excluded.
package_dirs="$(find internal -name '*.go' -not -path '*/testdata/*' | sed 's|/[^/]*$||' | sort -u)"

while read -r dir; do
  [ -n "$dir" ] || continue
  [ -f "$dir/INDEX.md" ] || fail "$dir has Go sources but no INDEX.md (see AGENTS.md 「INDEX 层级与同步规则」)"

  case "$dir" in
    internal/manager/*)
      # Subpackages are registered in the manager index, not the root one.
      name="${dir#internal/manager/}"
      grep -Fq "]($name/INDEX.md)" internal/manager/INDEX.md ||
        fail "internal/manager/INDEX.md 子包表 is missing a link to $name/INDEX.md"
      ;;
    internal/*)
      grep -Fq "]($dir/INDEX.md)" INDEX.md ||
        fail "root INDEX.md 包索引 is missing a row linking $dir/INDEX.md"
      ;;
  esac
done <<EOF
$package_dirs
EOF

# Relative links must resolve. Absolute URLs and pure anchors are skipped.
docs="$(
  {
    printf 'AGENTS.md\n'
    find . -name 'INDEX.md' \
      -not -path './node_modules/*' \
      -not -path './desktop/node_modules/*' \
      -not -path './.worktrees/*' |
      sed 's|^\./||'
  } | sort -u
)"
while read -r doc; do
  [ -n "$doc" ] || continue
  dir="$(dirname "$doc")"
  targets="$(grep -o '](\([^)]*\))' "$doc" | sed 's|^](||; s|)$||' || true)"
  while read -r target; do
    [ -n "$target" ] || continue
    case "$target" in
      http://* | https://* | mailto:* | '#'*) continue ;;
    esac
    path="${target%%#*}"
    [ -n "$path" ] || continue
    [ -e "$dir/$path" ] || fail "$doc links to $target but $dir/$path does not exist"
  done <<EOF
$targets
EOF
done <<EOF
$docs
EOF

if [ "$failures" -ne 0 ]; then
  printf '%s INDEX documentation check(s) failed\n' "$failures" >&2
  exit 1
fi

printf 'PASS: INDEX coverage and documentation links\n'
