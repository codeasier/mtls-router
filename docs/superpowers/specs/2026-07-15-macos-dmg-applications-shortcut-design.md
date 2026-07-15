# macOS DMG Applications Shortcut Design

## Problem

The controlled macOS DMG helper passes `CodeasierRouter.app` directly to
`hdiutil create` as its source folder. The resulting volume contains the app
bundle but cannot also contain the conventional `Applications` symbolic link.

## Design

`desktop/scripts/create-macos-dmg.sh` will create a temporary staging directory,
copy `CodeasierRouter.app` into it, and add an `Applications` symbolic link whose
target is `/Applications`. It will then use that staging directory as the
`hdiutil create` source folder. The temporary directory will be removed by an
exit trap on both success and failure.

No signing, artifact naming, target selection, or release workflow behavior will
change.

## Verification

`desktop/scripts/verify-package.sh` will inspect a mounted macOS DMG and reject
it unless `Applications` is a symbolic link with the exact target
`/Applications`.

`tests/desktop_workflow_test.sh` will assert that the helper stages the app and
shortcut, creates the DMG from the staging directory, and that package
verification checks the shortcut type and target. The existing workflow test
and relevant repository checks will be run after implementation.
