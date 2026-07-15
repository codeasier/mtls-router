# macOS DMG Applications Shortcut Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Include a conventional `Applications -> /Applications` shortcut in every macOS DMG and reject packages that omit or misdirect it.

**Architecture:** Build the DMG from an ephemeral staging directory containing only the copied app bundle and Applications symlink. Extend native DMG inspection to validate the mounted shortcut, while the existing shell workflow test statically guards both creation and verification behavior.

**Tech Stack:** Bash, macOS `hdiutil`, POSIX symbolic links, repository shell tests

---

## File Structure

- Modify `tests/desktop_workflow_test.sh`: assert the controlled helper stages the app and exact symlink, builds from staging, cleans staging, and package verification validates the mounted link.
- Modify `desktop/scripts/create-macos-dmg.sh`: create and clean an ephemeral staging directory, copy the app, create the shortcut, and pass staging to `hdiutil`.
- Modify `desktop/scripts/verify-package.sh`: require the mounted `Applications` entry to be a symlink targeting `/Applications`.

### Task 1: Add Regression Assertions

**Files:**
- Modify: `tests/desktop_workflow_test.sh:296-302`
- Test: `tests/desktop_workflow_test.sh`

- [ ] **Step 1: Replace the old direct-source assertion with staging and verification assertions**

Add assertions for these exact implementation fragments:

```bash
contains "$CREATE_DMG" 'staging="$(mktemp -d "${TMPDIR:-/tmp}/mtls-router-dmg.XXXXXX")"'
contains "$CREATE_DMG" 'trap cleanup EXIT'
contains "$CREATE_DMG" 'cp -R "$app" "$staging/CodeasierRouter.app"'
contains "$CREATE_DMG" 'ln -s /Applications "$staging/Applications"'
contains "$CREATE_DMG" 'hdiutil create -volname CodeasierRouter -srcfolder "$staging" -ov -format UDZO "$dmg"'
contains "$ROOT/desktop/scripts/verify-package.sh" '[[ -L "$applications_link" ]]'
contains "$ROOT/desktop/scripts/verify-package.sh" '[[ "$(readlink "$applications_link")" == /Applications ]]'
```

Remove the obsolete assertion requiring `-srcfolder "$app"`.

- [ ] **Step 2: Run the workflow test to verify it fails**

Run: `bash tests/desktop_workflow_test.sh`

Expected: FAIL because the helper does not yet create a staging directory and package verification does not inspect the shortcut.

### Task 2: Stage the DMG Contents

**Files:**
- Modify: `desktop/scripts/create-macos-dmg.sh:17-32`
- Test: `tests/desktop_workflow_test.sh`

- [ ] **Step 1: Add temporary staging cleanup**

After the output path declarations, create staging and install an exit trap:

```bash
staging="$(mktemp -d "${TMPDIR:-/tmp}/mtls-router-dmg.XXXXXX")"
cleanup() {
  rm -rf "$staging"
}
trap cleanup EXIT
```

- [ ] **Step 2: Populate staging and build from it**

Replace the direct app source with:

```bash
cp -R "$app" "$staging/CodeasierRouter.app"
ln -s /Applications "$staging/Applications"
hdiutil create -volname CodeasierRouter -srcfolder "$staging" -ov -format UDZO "$dmg"
```

Keep the existing app existence check, DMG directory reset, output name, and output existence check unchanged.

### Task 3: Verify the Mounted Shortcut

**Files:**
- Modify: `desktop/scripts/verify-package.sh:64-76`
- Test: `tests/desktop_workflow_test.sh`

- [ ] **Step 1: Validate link type and target in the Darwin package branch**

Immediately after validating the mounted app bundle, add:

```bash
applications_link="$mounted_dmg/Applications"
[[ -L "$applications_link" ]] || { printf 'DMG is missing the Applications symbolic link\n' >&2; exit 1; }
[[ "$(readlink "$applications_link")" == /Applications ]] || {
  printf 'DMG Applications symbolic link has an invalid target\n' >&2
  exit 1
}
```

- [ ] **Step 2: Run the focused regression test**

Run: `bash tests/desktop_workflow_test.sh`

Expected: PASS and print `desktop workflow checks passed`.

### Task 4: Run Relevant Full Verification

**Files:**
- Verify: `desktop/scripts/create-macos-dmg.sh`
- Verify: `desktop/scripts/verify-package.sh`
- Verify: `tests/desktop_workflow_test.sh`

- [ ] **Step 1: Check shell syntax**

Run: `bash -n desktop/scripts/create-macos-dmg.sh desktop/scripts/verify-package.sh tests/desktop_workflow_test.sh`

Expected: PASS with no output.

- [ ] **Step 2: Run repository shell tests**

Run: `make test-shell`

Expected: PASS.

- [ ] **Step 3: Run Go tests to detect repository-wide regressions**

Run: `go test ./...`

Expected: PASS.

- [ ] **Step 4: Inspect the final diff**

Run: `git diff --check && git status --short && git diff -- desktop/scripts/create-macos-dmg.sh desktop/scripts/verify-package.sh tests/desktop_workflow_test.sh docs/superpowers/specs/2026-07-15-macos-dmg-applications-shortcut-design.md docs/superpowers/plans/2026-07-15-macos-dmg-applications-shortcut.md`

Expected: no whitespace errors; only the issue-scoped files are changed.
