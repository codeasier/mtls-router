# Issues #48 and #49 Secure Installation and Process Identity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship platform packages whose setup scripts verify bundled or HTTPS-downloaded binaries, and prevent setup-managed router commands from signaling a process identified only by PID.

**Architecture:** Keep installation and process management in the existing cross-platform setup scripts. Installation resolves a sibling release payload first, validates it against `SHA256SUMS`, and only performs an explicitly authorized HTTPS fallback; process management persists and validates OS start identity plus executable path. A release aggregation job downloads all matrix artifacts, generates one checksum manifest, assembles platform archives, and publishes all outputs together.

**Tech Stack:** Bash, PowerShell 5.1+, GitHub Actions, standard SHA-256 tools, `jq`, shell integration tests.

---

## File Map

- Modify `setup.sh`: local payload discovery, strict checksum verification, HTTPS authorization, atomic installation, Unix process identity capture and validation.
- Modify `setup.ps1`: Windows-equivalent install and process identity behavior while preserving UTF-8 BOM.
- Modify `.github/workflows/release.yml`: aggregate binaries, generate `SHA256SUMS`, assemble six platform archives, publish and mirror verified artifacts.
- Create `tests/setup_secure_install_test.sh`: Unix local payload, fallback, checksum, protocol, and rollback tests.
- Modify `tests/setup_router_management_test.sh`: genuine, missing, stale, forged, and unrelated-process identity tests.
- Modify `tests/setup_powershell_router_test.sh`: PowerShell local payload and process identity integration coverage.
- Create `tests/setup_release_packaging_test.sh`: static release artifact and installer safety assertions.
- Modify `README.md` and `docs/zh-CN/README.md`: package-first installation, explicit fallback, checksum, and stale-state behavior.

### Task 1: Secure Unix Local Installation

**Files:**
- Create: `tests/setup_secure_install_test.sh`
- Modify: `setup.sh:4-150`

- [ ] **Step 1: Add failing local payload tests**

Create test helpers that copy `setup.sh` into a temporary package directory, create the detected `mtls-router-${os}-${arch}` payload, and generate `SHA256SUMS`. Assert that `router install` run from another working directory installs the payload, while missing, duplicate, malformed, and mismatched checksum entries fail and preserve a pre-existing installed binary.

```bash
expected='old-binary'
actual="$(cat "$install_dir/mtls-router")"
[[ "$actual" == "$expected" ]] || fail "checksum failure replaced installed binary"
```

- [ ] **Step 2: Run the focused test and verify failure**

Run: `bash tests/setup_secure_install_test.sh`

Expected: FAIL because `setup.sh` tries the network and has no sibling payload verification.

- [ ] **Step 3: Implement script directory and checksum helpers**

Add `SCRIPT_DIR`, `sha256_file`, `expected_checksum`, `verify_checksum`, and `install_verified_binary`. Parse only lines matching 64 hexadecimal characters, two spaces or ` *`, and the exact basename; require exactly one match. Use a temporary file inside `INSTALL_DIR`, apply mode `0755`, then `mv` it over `BINARY_PATH`.

```bash
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    fail "未找到 SHA-256 校验工具（sha256sum 或 shasum）。"
  fi
}
```

- [ ] **Step 4: Make `download_router` prefer verified sibling payloads**

After `detect_asset`, test `$SCRIPT_DIR/$ASSET`. If present, require `$SCRIPT_DIR/SHA256SUMS`, verify, and atomically install it. Do not invoke a downloader on any verification error. Preserve `MTLS_ROUTER_SKIP_DOWNLOAD=1` as an early bypass.

- [ ] **Step 5: Run focused tests**

Run: `bash tests/setup_secure_install_test.sh`

Expected: local payload cases PASS.

### Task 2: Secure Unix HTTPS Fallback

**Files:**
- Modify: `tests/setup_secure_install_test.sh`
- Modify: `setup.sh:29-150,850-1020`

- [ ] **Step 1: Add failing fallback tests**

Use fake `curl` and `wget` commands prepended to `PATH` to record requests and copy fixture files. Cover non-interactive failure without authorization, `--download` success, `MTLS_ROUTER_ALLOW_DOWNLOAD=1` success, remote checksum mismatch rollback, and rejection of `http://` before the fake downloader is called.

- [ ] **Step 2: Run the focused test and verify failure**

Run: `bash tests/setup_secure_install_test.sh`

Expected: FAIL because `--download` is unsupported and HTTP is accepted.

- [ ] **Step 3: Add explicit download authorization**

Introduce `ALLOW_DOWNLOAD="${MTLS_ROUTER_ALLOW_DOWNLOAD:-0}"`. Parse `--download` for `router install` and `router setup`, setting `ALLOW_DOWNLOAD=1`. If no sibling payload exists, prompt only when stdin and stderr are terminals; otherwise fail unless authorization equals `1`.

```bash
if [[ "$ALLOW_DOWNLOAD" != "1" ]]; then
  if [[ -t 0 && -t 2 ]]; then
    read -r -p "未找到随包二进制，是否通过 HTTPS 下载？[y/N] " answer
    [[ "$answer" =~ ^[Yy]$ ]] || fail "已取消联网下载。"
  else
    fail "未找到随包二进制；非交互安装需显式传入 --download。"
  fi
fi
```

- [ ] **Step 4: Enforce HTTPS and verify both remote files**

Resolve the binary URL and checksum URL from the same release/base URL. Validate both begin with `https://` before `require_downloader` and before applying credentials. Download into one temporary directory, verify the binary against the downloaded manifest, and call `install_verified_binary` only after success.

- [ ] **Step 5: Run Unix secure-install tests**

Run: `bash tests/setup_secure_install_test.sh`

Expected: PASS.

### Task 3: Secure PowerShell Installation

**Files:**
- Modify: `setup.ps1:1-96,850-980`
- Modify: `tests/setup_powershell_router_test.sh`

- [ ] **Step 1: Add failing PowerShell package and rollback tests**

Create a temporary package containing `setup.ps1`, `mtls-router-windows-amd64.exe`, and `SHA256SUMS`. Assert sibling installation succeeds and checksum mismatch preserves an existing `$BinaryPath`. Add static assertions for `--download` and HTTPS rejection so CI still covers intent when `pwsh` is unavailable.

- [ ] **Step 2: Run and verify failure**

Run: `bash tests/setup_powershell_router_test.sh`

Expected: FAIL because PowerShell downloads directly to the destination.

- [ ] **Step 3: Implement PowerShell verification and atomic replacement**

Use `$PSScriptRoot`, `Get-FileHash -Algorithm SHA256`, strict exact-name manifest matching, and a temporary file in `$InstallDir`. Move the temporary file over `$BinaryPath` only after verification.

```powershell
$actual = (Get-FileHash -LiteralPath $SourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) { Write-Fail "mtls-router SHA-256 校验失败：$Asset" }
```

- [ ] **Step 4: Implement PowerShell authorization and HTTPS-only fallback**

Parse `--download`, honor `MTLS_ROUTER_ALLOW_DOWNLOAD=1`, use `$Host.UI.RawUI` only for an interactive prompt, and otherwise fail closed. Require `[Uri]$url` to have scheme `https` before invoking `Invoke-WebRequest` or constructing credentials. Download the binary and `SHA256SUMS` to temporary paths and verify before replacement.

- [ ] **Step 5: Preserve and verify BOM**

Ensure the first three bytes of `setup.ps1` remain `EF BB BF` after editing.

Run: `bash tests/setup_powershell_router_test.sh && go test ./...`

Expected: PASS, including the existing BOM assertion.

### Task 4: Unix Process Identity Safety

**Files:**
- Modify: `setup.sh:152-264`
- Modify: `tests/setup_router_management_test.sh`

- [ ] **Step 1: Replace the fake router with an identifiable child**

Make the fake background process execute the installed fake router itself in a child mode rather than spawning plain `sleep`, so its executable identity can match the managed binary. Add tests that state contains `process_started_at` and `process_executable`.

- [ ] **Step 2: Add stale and unrelated-PID tests**

Create valid JSON state pointing to a live unrelated `sleep` process, with forged or missing identity. Assert status contains `stale`, does not contain `router running`, and stop leaves the unrelated process alive. Add a missing PID test and verify genuine status/stop still work.

- [ ] **Step 3: Run focused tests and verify failure**

Run: `bash tests/setup_router_management_test.sh`

Expected: FAIL because status and stop still trust `kill -0`.

- [ ] **Step 4: Implement Unix identity capture**

Add helpers to normalize paths and return an OS-stable start identity and executable. Linux reads `/proc/$pid/stat` field 22 and `/proc/$pid/exe`; macOS reads a stable `ps -o lstart=` value and verifies the command path against the normalized managed binary. Write both identity values atomically into state.

- [ ] **Step 5: Implement tri-state identity validation**

Replace `router_pid_running` in status/stop with a validator whose result distinguishes genuine, absent, and stale. Require stored start identity and executable, compare both to live values, and compare the stored executable to `binary_path`.

- [ ] **Step 6: Revalidate throughout stop**

Send TERM only for a genuine identity. During waiting, stop when the original identity disappears. Immediately before SIGKILL, validate the complete identity again; if it differs, report stale/reused PID and send no signal. Remove state only after confirmed genuine process termination.

- [ ] **Step 7: Run focused tests**

Run: `bash tests/setup_router_management_test.sh`

Expected: PASS.

### Task 5: PowerShell Process Identity Safety

**Files:**
- Modify: `setup.ps1:97-197`
- Modify: `tests/setup_powershell_router_test.sh`

- [ ] **Step 1: Add failing PowerShell stale-state tests**

Record a live unrelated process in state and assert `router status` reports stale and `router stop` does not terminate it. Assert genuine start writes `process_started_at` and `process_executable` and can still be stopped.

- [ ] **Step 2: Run and verify failure**

Run: `bash tests/setup_powershell_router_test.sh`

Expected: FAIL because `Test-RouterProcess` checks only PID existence.

- [ ] **Step 3: Capture Windows process identity**

Use `Get-Process -Id`, `StartTime.ToUniversalTime().ToString('o')`, and `.Path`, normalized with `[IO.Path]::GetFullPath`. Persist both values in `Write-RouterState`; fail startup state creation if either cannot be read.

- [ ] **Step 4: Validate before every signal**

Return an identity result that distinguishes missing and stale processes. Require PID, start time, and executable matches before `Stop-Process`; repeat validation in the wait loop and immediately before `Stop-Process -Force`. Keep stale state for diagnosis.

- [ ] **Step 5: Run PowerShell tests**

Run: `bash tests/setup_powershell_router_test.sh`

Expected: PASS when `pwsh` is installed; static checks PASS otherwise.

### Task 6: Release Checksums and Platform Packages

**Files:**
- Create: `tests/setup_release_packaging_test.sh`
- Modify: `.github/workflows/release.yml`

- [ ] **Step 1: Add failing workflow assertions**

Assert the workflow contains a single aggregation/release job that downloads all build artifacts, creates `SHA256SUMS`, produces `.tar.gz` and `.zip` packages, publishes the manifest and packages, contains no plaintext binary download base URL, and validates setup placeholder count and PowerShell BOM.

- [ ] **Step 2: Run and verify failure**

Run: `bash tests/setup_release_packaging_test.sh`

Expected: FAIL on the current HTTP installer staging workflow.

- [ ] **Step 3: Make matrix builds artifact-only**

Keep each build job responsible for one binary and `upload-artifact`. Remove per-matrix server and GitHub release uploads so no release is published before the checksum manifest is complete.

- [ ] **Step 4: Add the aggregation job**

Download all artifacts with merge enabled, sort filenames under a stable locale, run `sha256sum`, and stage one package directory per target. Copy the matching setup script, binary, and complete `SHA256SUMS`; preserve the executable bit for Unix setup and binary.

```bash
LC_ALL=C sha256sum mtls-router-* | LC_ALL=C sort -k2 > SHA256SUMS
tar -czf "packages/mtls-router-${os}-${arch}.tar.gz" -C "$stage" .
zip -j "packages/mtls-router-${os}-${arch}.zip" "$stage/setup.ps1" "$stage/$asset" "$stage/SHA256SUMS"
```

- [ ] **Step 5: Add release preflight and publish**

Verify `setup.sh` has exactly one empty default placeholder, `setup.ps1` has exactly one equivalent placeholder and starts with UTF-8 BOM, `SHA256SUMS` has six unique valid entries, and every package contains exactly three expected files. Publish binaries, manifest, and packages in one `softprops/action-gh-release` invocation. Mirror the same staged outputs only after preflight passes.

- [ ] **Step 6: Run workflow tests**

Run: `bash tests/setup_release_packaging_test.sh`

Expected: PASS.

### Task 7: User Documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/zh-CN/README.md`

- [ ] **Step 1: Update package-first installation instructions**

Document selecting the OS/architecture archive, extracting it, and running its setup script. Explain that the sibling binary must match `SHA256SUMS` and that verification failure never triggers fallback.

- [ ] **Step 2: Document explicit network fallback**

Add `router install --download`, `MTLS_ROUTER_ALLOW_DOWNLOAD=1`, HTTPS-only custom sources, and non-interactive fail-closed behavior to both languages.

- [ ] **Step 3: Document stale process state**

Explain that status/stop validate PID, process start identity, and executable path; stale state is retained and never signaled.

- [ ] **Step 4: Check documentation parity**

Run searches for `--download`, `SHA256SUMS`, and stale-state terminology in both files and confirm both language versions cover the same behavior.

### Task 8: Full Verification

**Files:**
- Modify only files required by failures introduced by this plan.

- [ ] **Step 1: Run shell integration tests**

Run: `make test-shell`

Expected: all setup tests PASS or explicitly skip only unavailable `pwsh` execution.

- [ ] **Step 2: Run Go tests and vet**

Run: `go test ./...`

Expected: PASS.

Run: `go vet ./...`

Expected: PASS.

- [ ] **Step 3: Check formatting and encoding**

Run: `test -z "$(gofmt -l .)"`

Expected: exit 0.

Verify `setup.ps1` begins with bytes `EF BB BF` and all shell scripts pass `bash -n`.

- [ ] **Step 4: Inspect final diff**

Run: `git diff --check && git status --short`

Expected: no whitespace errors; only intended tracked files plus pre-existing unrelated untracked files appear.
