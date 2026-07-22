#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
file_mode() {
  case "$(uname -s)" in
    Darwin) stat -f '%Lp' "$1" ;;
    *) stat -c '%a' "$1" ;;
  esac
}

platform="$(case "$(uname -s)" in Linux) printf linux;; Darwin) printf darwin;; *) fail unsupported;; esac)-$(case "$(uname -m)" in x86_64|amd64) printf amd64;; arm64|aarch64) printf arm64;; *) fail unsupported;; esac)"
router_asset="mtls-router-$platform"
manager_asset="mtls-router-manager-$platform"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
package="$tmp/package"; home="$tmp/home"; install="$tmp/install"; bin="$tmp/bin"
clean="$tmp/clean"
mkdir -p "$package" "$clean" "$home" "$install" "$bin" "$home/codex"
export CODEX_HOME="$tmp/hostile-codex-home"
cp "$ROOT/setup.sh" "$package/setup.sh"
cat >"$package/$router_asset" <<'ROUTER'
#!/usr/bin/env bash
[[ "${1:-}" == --version || "${1:-}" == -version ]] && { printf 'mtls-router wrapper-test\n'; exit; }
exit 0
ROUTER
chmod +x "$package/setup.sh" "$package/$router_asset"
go build -trimpath -ldflags "-X github.com/codeasier/mtls-router/internal/version.Version=wrapper-test -X github.com/codeasier/mtls-router/internal/version.DeploymentID=wrapper-test-deployment" -o "$package/$manager_asset" "$ROOT/cmd/mtls-router-manager"
chmod 0555 "$package/$router_asset" "$package/$manager_asset"
for command in claude opencode codex; do printf '#!/usr/bin/env bash\nexit 0\n' >"$bin/$command"; chmod +x "$bin/$command"; done
{
  printf '%s  %s\n' "$(sha256 "$package/$router_asset")" "$router_asset"
  printf '%s  %s\n' "$(sha256 "$package/$manager_asset")" "$manager_asset"
} >"$package/SHA256SUMS"

common=(env PATH="$bin:$PATH" HOME="$home" CODEX_HOME="$home/codex" CLAUDE_CONFIG_DIR="$home/claude" OPENCODE_CONFIG="$home/opencode.json" MTLS_ROUTER_INSTALL_DIR="$install" MTLS_ROUTER_STATE_DIR="$home/state" MTLS_ROUTER_DESKTOP_DATA_DIR="$home/desktop")

# No-argument setup installs the verified pair, can skip startup, and never writes Agent files.
MTLS_ROUTER_SKIP_START=1 "${common[@]}" bash "$package/setup.sh" >/dev/null
[[ -x "$install/mtls-router" && -x "$install/mtls-router-manager" ]] || fail "no-arg setup did not install pair"
[[ ! -e "$home/claude/settings.json" && ! -e "$home/opencode.json" && ! -e "$home/codex/config.toml" ]] || fail "no-arg setup changed Agent files"
[[ "$(file_mode "$package/$router_asset")" == 555 && "$(file_mode "$package/$manager_asset")" == 555 ]] || fail "package payloads are not mode 0555 after install"

cp "$ROOT/setup.sh" "$clean/setup.sh"

# Agent commands are interactive only and compatibility aliases use current protocol guidance.
for command in 'agent print-config --agent=claude' 'agent write-config --agent=claude' '--print-config --agent=claude' '--write-config --agent=claude'; do
  if MTLS_ROUTER_OPENAI_API_KEY=environment-canary "${common[@]}" bash "$clean/setup.sh" $command </dev/null >/dev/null 2>"$tmp/noninteractive-error"; then fail "$command accepted noninteractive key input"; fi
  grep -Fq 'protocol v3' "$tmp/noninteractive-error" || fail "$command omitted v3 automation guidance"
done
[[ ! -e "$home/claude/settings.json" ]] || fail "noninteractive command changed Agent config"

# Agent commands with no sibling or verified receipt fail without touching a downloader.
mkdir -p "$tmp/uninstalled-home"
if env PATH="$bin:$PATH" HOME="$tmp/uninstalled-home" CODEX_HOME="$tmp/uninstalled-home/codex" \
  MTLS_ROUTER_INSTALL_DIR="$tmp/uninstalled" MTLS_ROUTER_STATE_DIR="$tmp/uninstalled-state" \
  MTLS_ROUTER_ALLOW_DOWNLOAD=1 bash "$clean/setup.sh" agent print-config --agent=claude >/dev/null 2>"$tmp/uninstalled-error"; then
  fail "Agent command without verified manager should fail"
fi
grep -Fq '不会隐式下载' "$tmp/uninstalled-error" || fail "Agent missing-manager guidance is absent"

# A hostile inherited CODEX_HOME is not touched when the test fixes its isolated value.
[[ ! -e "$CODEX_HOME" ]] || fail "hostile outer CODEX_HOME was touched"

printf 'PASS: setup manager wrapper and v3 noninteractive guidance\n'
