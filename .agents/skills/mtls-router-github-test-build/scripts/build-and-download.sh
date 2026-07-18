#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Build mtls-router validation packages on GitHub and download artifacts.

Usage:
  build-and-download.sh --ref REF --version VERSION [options]

Required:
  --ref REF              GitHub branch, tag, or commit
  --version VERSION      Validation package SemVer

Options:
  --target TARGET        all or one OS-architecture target (default: all)
  --upstream URL         HTTPS upstream override for this validation build
  --artifact GLOB        Artifact name or shell glob; repeatable (default: all)
  --output DIRECTORY     Download destination (default: repository artifacts dir)
  --repo OWNER/NAME      GitHub repository (default: current repository)
  -h, --help             Show this help

The script never accepts secret values. GitHub Actions injects repository
Secrets and Variables during the validation-only Release workflow.
EOF
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

need_value() {
  [ "$#" -ge 2 ] && [ -n "$2" ] || die "$1 requires a value"
}

ref_input=""
version=""
target="all"
upstream=""
repo=""
output=""
artifact_patterns_file="$(mktemp "${TMPDIR:-/tmp}/mtls-router-artifacts.XXXXXX")"
selected_artifacts_file="$(mktemp "${TMPDIR:-/tmp}/mtls-router-selected.XXXXXX")"
all_artifacts_file=""
temporary_branch=""
cleanup_failed=0

cleanup() {
  local status=${1:-$?}
  rm -f "$artifact_patterns_file" "$selected_artifacts_file"
  if [ -n "$all_artifacts_file" ]; then
    rm -f "$all_artifacts_file"
  fi
  if [ -n "$temporary_branch" ]; then
    if ! gh api --method DELETE "repos/$repo/git/refs/heads/$temporary_branch" >/dev/null 2>&1; then
      printf 'warning: failed to delete temporary branch %s; delete it manually\n' "$temporary_branch" >&2
      cleanup_failed=1
    fi
  fi
  if [ "$status" -eq 0 ] && [ "$cleanup_failed" -ne 0 ]; then
    status=1
  fi
  exit "$status"
}

interrupted() {
  trap - EXIT INT TERM
  cleanup 130
}

trap cleanup EXIT
trap interrupted INT TERM

while [ "$#" -gt 0 ]; do
  case "$1" in
    --ref)
      need_value "$@"
      ref_input=$2
      shift 2
      ;;
    --version)
      need_value "$@"
      version=$2
      shift 2
      ;;
    --target)
      need_value "$@"
      target=$2
      shift 2
      ;;
    --upstream)
      need_value "$@"
      upstream=$2
      shift 2
      ;;
    --artifact)
      need_value "$@"
      printf '%s\n' "$2" >>"$artifact_patterns_file"
      shift 2
      ;;
    --output)
      need_value "$@"
      output=$2
      shift 2
      ;;
    --repo)
      need_value "$@"
      repo=$2
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

[ -n "$ref_input" ] || die "--ref is required"
[ -n "$version" ] || die "--version is required"
case "$version" in
  *[!0-9A-Za-z.-]*|.*|*..*|*.) die "invalid validation version: $version" ;;
esac
printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$' || die "invalid validation version: $version"
case "$target" in
  all|windows-amd64|windows-arm64|darwin-amd64|darwin-arm64|linux-amd64|linux-arm64) ;;
  *) die "invalid validation target: $target" ;;
esac
case "$upstream" in
  ""|https://*) ;;
  *) die "validation upstream must use HTTPS" ;;
esac
case "$upstream" in
  *[[:space:]\'\"]*) die "validation upstream must not contain whitespace or quotes" ;;
esac
if [ -n "$output" ]; then
  [ ! -e "$output" ] || die "output already exists: $output"
fi

command -v gh >/dev/null 2>&1 || die "gh is required"
command -v jq >/dev/null 2>&1 || die "jq is required"
gh auth status >/dev/null 2>&1 || die "gh is not authenticated"

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) || die "run from an mtls-router checkout"
workflow="$repo_root/.github/workflows/release.yml"
[ -f "$workflow" ] || die "missing .github/workflows/release.yml"

if [ -z "$repo" ]; then
  repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
fi
printf '%s\n' "$repo" | grep -Eq '^[^/[:space:]]+/[^/[:space:]]+$' || die "invalid repository: $repo"

target_sha=""
dispatch_ref=""
if target_sha=$(gh api "repos/$repo/git/ref/heads/$ref_input" --jq '.object.sha' 2>/dev/null); then
  dispatch_ref=$ref_input
else
  commit_json=$(gh api "repos/$repo/commits/$ref_input" 2>/dev/null) || die "GitHub ref cannot be resolved: $ref_input"
  target_sha=$(printf '%s' "$commit_json" | jq -r '.sha')
  [ -n "$target_sha" ] && [ "$target_sha" != null ] || die "resolved ref has no commit SHA: $ref_input"
  short_sha=$(printf '%.12s' "$target_sha")
  timestamp=$(date -u +%Y%m%d%H%M%S)
  temporary_branch="agent-test-build/$short_sha-$timestamp-$$"
  gh api --method POST "repos/$repo/git/refs" \
    -f ref="refs/heads/$temporary_branch" \
    -f sha="$target_sha" >/dev/null
  dispatch_ref=$temporary_branch
fi

case "$target_sha" in
  '') die "resolved ref has no commit SHA: $ref_input" ;;
esac

remote_workflow=$(gh api --method GET \
  -H 'Accept: application/vnd.github.raw+json' \
  "repos/$repo/contents/.github/workflows/release.yml" \
  -f ref="$target_sha") || die "target ref has no release workflow: $ref_input"
for reference in 'workflow_dispatch' 'target:' 'upstream_url:' 'secrets.CLIENT_CERT_PEM' 'secrets.CLIENT_KEY_PEM' 'secrets.UPSTREAM_CA_PEM' 'vars.UPSTREAM_URL' 'vars.DEPLOYMENT_ID'; do
  grep -Fq "$reference" <<<"$remote_workflow" || die "target release workflow is missing GitHub reference: $reference"
done

created_after=$(date -u +%Y-%m-%dT%H:%M:%SZ)
dispatch_args=(
  --method POST
  "repos/$repo/actions/workflows/release.yml/dispatches"
  -f ref="$dispatch_ref"
  -f "inputs[version]=$version"
  -f "inputs[target]=$target"
)
if [ -n "$upstream" ]; then
  dispatch_args+=(-f "inputs[upstream_url]=$upstream")
fi
gh api "${dispatch_args[@]}" >/dev/null

printf 'Dispatched validation build for %s at %s\n' "$ref_input" "$target_sha"

run_id=""
run_url=""
attempt=0
while [ "$attempt" -lt 60 ]; do
  runs=$(gh api --method GET "repos/$repo/actions/workflows/release.yml/runs" \
    -f event=workflow_dispatch -f branch="$dispatch_ref" -F per_page=20)
  matches=$(printf '%s' "$runs" | jq -r \
    --arg sha "$target_sha" --arg after "$created_after" \
    '.workflow_runs[] | select(.head_sha == $sha and .created_at >= $after) | [.id, .html_url] | @tsv')
  count=$(printf '%s\n' "$matches" | awk 'NF { count++ } END { print count+0 }')
  if [ "$count" -eq 1 ]; then
    run_id=$(printf '%s\n' "$matches" | awk 'NF { print $1 }')
    run_url=$(printf '%s\n' "$matches" | awk 'NF { print $2 }')
    break
  fi
  if [ "$count" -gt 1 ]; then
    printf '%s\n' "$matches" >&2
    die "multiple workflow runs matched the dispatch"
  fi
  sleep 2
  attempt=$((attempt + 1))
done
[ -n "$run_id" ] || die "timed out correlating the dispatched workflow run"

printf 'Run: %s\n' "$run_url"
gh run watch "$run_id" --repo "$repo" --exit-status

run_json=$(gh api "repos/$repo/actions/runs/$run_id")
[ "$(printf '%s' "$run_json" | jq -r '.conclusion')" = success ] || die "workflow run did not succeed"
[ "$(printf '%s' "$run_json" | jq -r '.head_sha')" = "$target_sha" ] || die "workflow run head SHA changed"

artifacts_json=$(gh api --paginate "repos/$repo/actions/runs/$run_id/artifacts?per_page=100" --slurp)
all_artifacts_file=$(mktemp "${TMPDIR:-/tmp}/mtls-router-all-artifacts.XXXXXX")
printf '%s' "$artifacts_json" | jq -r '.[].artifacts[] | select(.expired == false) | .name' | sort -u >"$all_artifacts_file"
[ -s "$all_artifacts_file" ] || die "successful run has no available artifacts"

if [ -s "$artifact_patterns_file" ]; then
  while IFS= read -r pattern; do
    matched=0
    while IFS= read -r artifact; do
      # The caller-facing artifact selector intentionally uses shell globs.
      # shellcheck disable=SC2053
      if [[ $artifact == $pattern ]]; then
        printf '%s\n' "$artifact" >>"$selected_artifacts_file"
        matched=1
      fi
    done <"$all_artifacts_file"
    [ "$matched" -eq 1 ] || die "artifact pattern matched nothing: $pattern"
  done <"$artifact_patterns_file"
  sort -u "$selected_artifacts_file" -o "$selected_artifacts_file"
else
  cp "$all_artifacts_file" "$selected_artifacts_file"
fi
rm -f "$all_artifacts_file"
all_artifacts_file=""

if [ -z "$output" ]; then
  output="$repo_root/desktop/release-artifacts/run-$run_id"
fi
[ ! -e "$output" ] || die "output already exists: $output"
mkdir -p "$output"

while IFS= read -r artifact; do
  gh run download "$run_id" --repo "$repo" --name "$artifact" --dir "$output/$artifact"
done <"$selected_artifacts_file"

checksum_count=0
while IFS= read -r checksum; do
  checksum_count=$((checksum_count + 1))
  checksum_dir=${checksum%/*}
  checksum_name=${checksum##*/}
  (cd "$checksum_dir" && shasum -a 256 -c "$checksum_name")
done < <(find "$output" -type f -name '*.sha256' -print)
if [ "$checksum_count" -eq 0 ]; then
  printf 'warning: no checksum manifests were downloaded\n' >&2
fi

printf '\nDownloaded artifacts:\n'
while IFS= read -r artifact; do
  printf '  %s/%s\n' "$output" "$artifact"
done <"$selected_artifacts_file"

printf '\nPackage SHA-256:\n'
find "$output" -type f \( -name '*.dmg' -o -name '*.exe' -o -name '*.AppImage' -o -name 'mtls-router-*' \) ! -name '*.sha256' -exec shasum -a 256 {} \;

while IFS= read -r status_file; do
  status=$(tr '\n' ' ' <"$status_file")
  printf 'Signing status (%s): %s\n' "${status_file#"$output"/}" "$status"
  case "$status" in
    *unsigned*|*ad-hoc*) printf 'warning: package is not trusted-signed: %s\n' "${status_file#"$output"/}" >&2 ;;
  esac
done < <(find "$output" -type f -name 'signing-status-*.txt' -print)

printf '\nValidation build complete\n'
printf 'Run ID: %s\nRun URL: %s\nHead SHA: %s\nVersion: %s\nTarget: %s\nOutput: %s\n' \
  "$run_id" "$run_url" "$target_sha" "$version" "$target" "$output"
