# macOS Tray Template Icon Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the dense macOS `CR` tray monogram with a fixed, embedded route-node template image while preserving existing Windows and Linux status icons.

**Architecture:** Store the approved `40x40 px` transparent Retina PNG beside the existing desktop icons and decode it from embedded bytes through Tauri's PNG image support. Route tray creation and updates through a platform-aware helper: macOS ignores severity and returns the fixed template image, while other platforms retain the current runtime renderer.

**Tech Stack:** Rust 2021, Tauri 2.11, PNG, Cargo, npm desktop verification scripts

---

## File Structure

- Create `desktop/src-tauri/icons/tray-template-macos@2x.png`: approved monochrome route-node alpha mask with a transparent safety boundary.
- Modify `desktop/src-tauri/Cargo.toml`: enable Tauri's `image-png` feature for decoding the embedded PNG.
- Modify `desktop/src-tauri/src/tray.rs`: add platform-aware icon loading, propagate decode errors, and add resource-boundary regression tests.
- Modify `desktop/src-tauri/Cargo.lock`: record any feature-driven dependency changes resolved by Cargo.

### Task 1: Add And Validate The Template Resource

**Files:**
- Create: `desktop/src-tauri/icons/tray-template-macos@2x.png`
- Modify: `desktop/src-tauri/Cargo.toml:25`
- Modify: `desktop/src-tauri/Cargo.lock`
- Test: `desktop/src-tauri/src/tray.rs:653-721`

- [ ] **Step 1: Write the failing embedded-resource test**

Add a macOS resource constant near `status_icon` and add this test to the existing `tray::tests` module. At this stage the constant points at the intended but absent file, so compilation must fail and prove the test is connected to the production asset.

```rust
#[cfg(target_os = "macos")]
const MACOS_TRAY_ICON_PNG: &[u8] =
    include_bytes!("../icons/tray-template-macos@2x.png");

#[cfg(target_os = "macos")]
#[test]
fn macos_template_icon_has_retina_dimensions_and_safe_transparent_bounds() {
    let icon = Image::from_bytes(MACOS_TRAY_ICON_PNG).expect("template PNG must decode");
    assert_eq!((icon.width(), icon.height()), (40, 40));

    let alphas = icon.rgba().chunks_exact(4).map(|pixel| pixel[3]);
    assert!(alphas.clone().any(|alpha| alpha == 0));
    assert!(alphas.clone().any(|alpha| alpha != 0));

    for y in 0..40_usize {
        for x in 0..40_usize {
            if x < 2 || x >= 38 || y < 2 || y >= 38 {
                assert_eq!(icon.rgba()[(y * 40 + x) * 4 + 3], 0);
            }
        }
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm run rust:test -- tray::tests::macos_template_icon_has_retina_dimensions_and_safe_transparent_bounds`

Expected: FAIL during compilation because `icons/tray-template-macos@2x.png` does not exist or because `Image::from_bytes` is unavailable before the PNG feature is enabled.

- [ ] **Step 3: Add the approved PNG and enable decoding**

Create `desktop/src-tauri/icons/tray-template-macos@2x.png` as a `40x40 px` transparent RGBA PNG. Rasterize the approved route-node design with these constraints:

- Three circular nodes and one turning route path.
- Monochrome opaque marks on a transparent background.
- No visible pixel in rows or columns `0`, `1`, `38`, or `39`.
- No status badge or `CR` lettering.

Change the Tauri dependency to:

```toml
tauri = { version = "=2.11.5", features = ["image-png", "tray-icon"] }
```

Run `cargo check --manifest-path src-tauri/Cargo.toml` from `desktop/` to update `Cargo.lock` and validate PNG decoding support.

- [ ] **Step 4: Run the resource test to verify it passes**

Run: `npm run rust:test -- tray::tests::macos_template_icon_has_retina_dimensions_and_safe_transparent_bounds`

Expected: PASS with one matching test.

- [ ] **Step 5: Commit the resource and validation**

```bash
git add desktop/src-tauri/Cargo.toml desktop/src-tauri/Cargo.lock desktop/src-tauri/icons/tray-template-macos@2x.png desktop/src-tauri/src/tray.rs
git commit -m "test: validate macOS tray template asset"
```

### Task 2: Select The Platform-Specific Tray Icon

**Files:**
- Modify: `desktop/src-tauri/src/tray.rs:226-232`
- Modify: `desktop/src-tauri/src/tray.rs:436-447`
- Modify: `desktop/src-tauri/src/tray.rs:593-651`
- Test: `desktop/src-tauri/src/tray.rs:653-721`

- [ ] **Step 1: Write a failing severity-independence test**

Add this macOS-only test to the existing test module:

```rust
#[cfg(target_os = "macos")]
#[test]
fn macos_template_icon_is_independent_of_severity() {
    let normal = tray_icon(Severity::Normal).expect("normal icon");
    let warning = tray_icon(Severity::Warning).expect("warning icon");
    let error = tray_icon(Severity::Error).expect("error icon");

    assert_eq!(normal.rgba(), warning.rgba());
    assert_eq!(warning.rgba(), error.rgba());
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm run rust:test -- tray::tests::macos_template_icon_is_independent_of_severity`

Expected: FAIL because `tray_icon` does not exist.

- [ ] **Step 3: Add the platform-aware helper**

Add these helpers next to the existing runtime renderer:

```rust
#[cfg(target_os = "macos")]
fn tray_icon(_severity: Severity) -> tauri::Result<Image<'static>> {
    Ok(Image::from_bytes(MACOS_TRAY_ICON_PNG)?.to_owned())
}

#[cfg(not(target_os = "macos"))]
fn tray_icon(severity: Severity) -> tauri::Result<Image<'static>> {
    Ok(status_icon(severity))
}
```

Keep `status_icon` unchanged for non-macOS platforms.

- [ ] **Step 4: Route tray creation and updates through the helper**

Resolve the initial icon before constructing `TrayIconBuilder`, then pass it to the builder:

```rust
let initial_icon = tray_icon(Severity::Warning)?;
let tray = TrayIconBuilder::with_id("main")
    // existing builder calls
    .icon(initial_icon)
```

Update `apply_presentation_text` to propagate image-selection errors:

```rust
state.tray.set_icon(Some(tray_icon(value.severity)?))?;
```

Leave `icon_as_template(cfg!(target_os = "macos"))` unchanged.

- [ ] **Step 5: Run focused tray tests**

Run: `npm run rust:test -- tray::tests`

Expected: all tray tests PASS, including the new resource and severity-independence tests.

- [ ] **Step 6: Commit platform selection behavior**

```bash
git add desktop/src-tauri/src/tray.rs
git commit -m "fix: use native macOS tray template icon"
```

### Task 3: Verify The Complete Change

**Files:**
- Verify: `desktop/src-tauri/icons/tray-template-macos@2x.png`
- Verify: `desktop/src-tauri/Cargo.toml`
- Verify: `desktop/src-tauri/Cargo.lock`
- Verify: `desktop/src-tauri/src/tray.rs`

- [ ] **Step 1: Check Rust formatting**

Run: `npm run rust:format`

Expected: PASS with no formatting differences.

- [ ] **Step 2: Run complete Desktop verification**

Run: `npm run verify`

Expected: ESLint, Prettier, TypeScript, Vitest, Vite build, Rust formatting, and all Rust tests PASS.

- [ ] **Step 3: Inspect the final diff**

Run: `git diff --check HEAD~2..HEAD && git status --short && git diff HEAD~2..HEAD -- desktop/src-tauri`

Expected: no whitespace errors; only the planned asset, Cargo feature/lockfile, and tray implementation are changed. `.superpowers/` may remain untracked and must not be committed.

- [ ] **Step 4: Perform macOS visual verification when an app run is available**

Run: `npm run tauri dev`

Expected: the menu bar shows the route-node symbol with balanced spacing at Retina scale; it recolors correctly in light and dark appearances; changing router state updates tooltip/menu text without changing or crowding the icon. If runtime credentials or local environment prevent startup, record this manual check as not performed rather than weakening automated verification.
