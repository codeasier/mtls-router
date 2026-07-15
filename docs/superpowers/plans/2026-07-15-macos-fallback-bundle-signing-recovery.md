# macOS Fallback Bundle Signing Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix fallback Intel macOS application sealing and publish the correction as `v0.1.7` without moving the failed `v0.1.6` tag.

**Architecture:** Explicitly ad-hoc sign Tauri's two input sidecars before bundling, then explicitly sign the generated desktop executable and top-level app bundle before deterministic DMG creation. Shell assertions enforce this order and continue to prohibit recursive signing.

**Tech Stack:** GitHub Actions YAML, Bash, Tauri 2, macOS `codesign`, Go and shell repository checks

---

### Task 1: Enforce the fallback signing contract

**Files:**
- Modify: `tests/desktop_workflow_test.sh:309-325`

- [ ] **Step 1: Replace the fallback signing assertion with ordered explicit-signing assertions**

Require the unsigned macOS workflow block to contain, in order:

```bash
codesign --force --sign - "src-tauri/binaries/mtls-router-${{ matrix.target }}"
codesign --force --sign - "src-tauri/binaries/mtls-router-manager-${{ matrix.target }}"
npm exec tauri -- build --target ${{ matrix.target }} --bundles app --no-sign --ci
codesign --force --sign - "$app/Contents/MacOS/mtls-router-desktop"
codesign --force --sign - "$app"
./scripts/create-macos-dmg.sh ${{ matrix.target }} "$VERSION"
```

Keep the existing rejection of `codesign --force --deep` and `--bundles dmg`.

- [ ] **Step 2: Run the workflow assertion test and verify it fails**

Run: `bash tests/desktop_workflow_test.sh`

Expected: FAIL because `.github/workflows/release.yml` does not yet sign the sidecars and desktop executable explicitly.

### Task 2: Implement explicit fallback signing

**Files:**
- Modify: `.github/workflows/release.yml:230-237`
- Test: `tests/desktop_workflow_test.sh`

- [ ] **Step 1: Update the unsigned macOS package block**

Use this order in the desktop working directory:

```bash
codesign --force --sign - "src-tauri/binaries/mtls-router-${{ matrix.target }}"
codesign --force --sign - "src-tauri/binaries/mtls-router-manager-${{ matrix.target }}"
npm exec tauri -- build --target ${{ matrix.target }} --bundles app --no-sign --ci
app=src-tauri/target/${{ matrix.target }}/release/bundle/macos/CodeasierRouter.app
codesign --force --sign - "$app/Contents/MacOS/mtls-router-desktop"
codesign --force --sign - "$app"
./scripts/create-macos-dmg.sh ${{ matrix.target }} "$VERSION"
```

- [ ] **Step 2: Run the focused workflow assertion test**

Run: `bash tests/desktop_workflow_test.sh`

Expected: PASS.

### Task 3: Document `v0.1.7`

**Files:**
- Modify: `docs/CHANGELOG.md`
- Modify: `docs/zh-CN/CHANGELOG.md`

- [ ] **Step 1: Add aligned `v0.1.7` entries dated 2026-07-15**

State that `v0.1.6` did not publish, that its tag remains unchanged, and that fallback macOS packaging now explicitly signs embedded executables before sealing the app bundle.

- [ ] **Step 2: Check documentation whitespace**

Run: `git diff --check`

Expected: no output and exit status 0.

### Task 4: Verify and deliver the release fix

**Files:**
- Verify all modified files

- [ ] **Step 1: Run repository verification**

Run: `go test ./...`

Expected: all packages pass.

Run: `go vet ./...`

Expected: no diagnostics.

Run: `make test-shell`

Expected: all shell tests pass.

Run: `test -z "$(gofmt -l .)"`

Expected: exit status 0.

Run from `desktop`: `npm ci && npm run static:check && npm run typecheck && npm test -- --run && npm run build`

Expected: static checks, 56 tests, and production build pass.

- [ ] **Step 2: Commit and open a PR**

```bash
git add .github/workflows/release.yml tests/desktop_workflow_test.sh docs/CHANGELOG.md docs/zh-CN/CHANGELOG.md docs/superpowers/specs/2026-07-15-macos-fallback-bundle-signing-recovery-design.md docs/superpowers/plans/2026-07-15-macos-fallback-bundle-signing-recovery.md
git commit -m "fix: sign fallback macOS bundle contents"
git push -u origin release/v0.1.7
```

Create the PR against `main`, wait for required checks, and squash merge it.

- [ ] **Step 3: Tag and verify publication**

Fast-forward local `main`, create annotated tag `v0.1.7` at the merged commit, and push only that tag. Wait for the tag-triggered Release workflow to succeed, then verify the GitHub Release and full asset manifest exist. Do not modify or delete `v0.1.6`.
