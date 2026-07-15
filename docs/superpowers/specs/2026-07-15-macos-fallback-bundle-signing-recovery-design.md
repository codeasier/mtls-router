# macOS Fallback Bundle Signing Recovery Design

## Context

The `v0.1.6` Release workflow failed while building the Intel macOS desktop package. The fallback packaging path attempted to ad-hoc sign the completed application bundle while its embedded router sidecar was unsigned, so `codesign` rejected the bundle. The tag must remain immutable and no incomplete GitHub Release was published.

## Decision

Publish the fix as `v0.1.7`. Keep `v0.1.6` at its existing commit and do not use release recovery because the failed run has no successful Intel macOS desktop artifact.

The unsigned macOS workflow will:

1. Ad-hoc sign the router and manager sidecars in Tauri's input directory.
2. Build the application bundle with Tauri signing disabled.
3. Ad-hoc sign the generated desktop executable.
4. Ad-hoc sign the completed application bundle.
5. Create the DMG and run the existing native package verification.

Each executable is signed explicitly. The workflow will not use recursive `codesign --deep` signing, which could modify packaged sidecars after their hashes are compared with the signed source files.

## Scope

- Update the fallback macOS block in `.github/workflows/release.yml`.
- Update workflow regression assertions in `tests/desktop_workflow_test.sh`.
- Add bilingual `v0.1.7` release notes that identify `v0.1.6` as unpublished.

No runtime, desktop UI, signed macOS path, or recovery workflow behavior changes.

## Verification

- Shell workflow assertions verify explicit sidecar, desktop executable, and app bundle signing order.
- The assertions continue to reject `codesign --deep` and Tauri DMG creation.
- Repository Go, shell, frontend, and formatting checks pass before the release PR is merged.
- The `v0.1.7` tag is created only from the merged release commit.
- The tag-triggered Release workflow must complete successfully and publish the full artifact manifest before the release is considered complete.
