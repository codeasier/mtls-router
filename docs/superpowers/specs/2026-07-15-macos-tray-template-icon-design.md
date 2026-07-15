# macOS Tray Template Icon Design

## Summary

Replace the dense, runtime-rendered `CR` monogram used by the macOS desktop
tray with a dedicated monochrome template image. The new icon uses three nodes
and a turning path to represent local routing while matching the spacing,
stroke weight, and visual scale of adjacent macOS menu bar symbols.

This change is limited to macOS. Windows and Linux retain their existing
status-aware tray icons.

## Problem

`status_icon` currently draws a `20x20` RGBA pixel image in Rust. The `CR`
monogram nearly fills that canvas, and the normal, warning, and error badges
add more visual weight to its lower-right corner. Although the tray is already
configured with `icon_as_template(true)` on macOS, the source alpha mask is too
dense to look native at menu bar size.

The icon therefore needs a simpler silhouette and a predictable transparent
boundary. Template rendering itself is already the correct macOS light, dark,
and selected-state adaptation mechanism and will remain enabled.

## Decisions

### Visual Direction

Use the approved route-node symbol:

- Three circular nodes establish endpoints and a destination.
- A light turning path communicates routing between the nodes.
- The design avoids letters, a containing shape, and a status badge.
- The image uses only opaque monochrome marks and transparent background;
  macOS supplies the displayed color through template rendering.

The resource is a `40x40 px` Retina PNG representing a `20x20 pt` menu bar
canvas. All visible pixels remain within a `36x36 px` visual boundary, leaving
at least `2 px` of transparent padding on every edge.

### Status Presentation

Normal, warning, and error states use the same macOS template image. Router
state remains available through:

- The tray tooltip.
- The first menu item.
- The enabled or disabled Start and Stop menu actions.

This avoids reintroducing crowded state badges at menu bar size. Severity
calculation and all lifecycle behavior remain unchanged.

### Platform Scope

Only macOS uses the embedded route-node resource. Windows and Linux continue
to call the existing runtime `status_icon(severity)` implementation, including
their current status indicators. No non-macOS visual behavior changes as part
of this issue.

## Implementation

Add the PNG under `desktop/src-tauri/icons/` and embed it into the Rust binary
with `include_bytes!`. Enable Tauri's `image-png` Cargo feature and decode the
embedded bytes with `Image::from_bytes`.

Introduce a platform-aware icon selection function:

- On macOS, decode and return the fixed embedded template image regardless of
  severity.
- On Windows and Linux, return the existing runtime-rendered status icon.

Tray creation and status updates both use this selection function. Because PNG
decoding can fail, icon selection returns `tauri::Result<Image<'static>>` and
propagates an invalid embedded resource as a Tauri error. There is no fallback
to the old macOS monogram; a damaged bundled resource should fail visibly in
tests or application initialization rather than silently restoring the visual
bug.

`icon_as_template(cfg!(target_os = "macos"))` remains unchanged.

## Testing

Add regression coverage for the embedded template resource:

- It decodes successfully as PNG.
- Its dimensions are exactly `40x40 px`.
- It contains both transparent and non-transparent pixels, preventing an empty
  image or an opaque background from passing.
- The outer `2 px` on every side are fully transparent.
- Every non-transparent pixel remains within the specified visual boundary.
- macOS icon selection is independent of severity.

Keep the existing non-macOS status icon assertions so their current behavior
remains protected.

Run the tray-focused Rust tests first, followed by the complete Desktop
verification command, `npm run verify`. On macOS with a Retina display,
perform an application-level visual check in both light and dark menu bar
appearances, confirming balanced spacing beside system symbols and correct
template recoloring.

## Out Of Scope

- Changing router state semantics or lifecycle actions.
- Adding color-based tray status.
- Redesigning Windows or Linux tray assets.
- Changing the application, installer, or DMG icon.
- Adding new tooltip or menu text.
