# Agent Responsive Controls Acceptance Checklist

Check an item only after automated evidence or recorded manual verification
demonstrates the behavior.

## Scope

- [x] The implementation matches `spec.md` without unrelated Agent workflow or
  desktop-page redesign.
- [x] No Rust, IPC, Manager, router, protocol, persisted configuration, or
  localization contract changes are introduced.
- [x] Select styling changes are scoped to the Agent page.
- [x] The Settings language select retains its existing markup and styling.

## Configuration Paths

- [x] Every detected Agent path uses a dedicated containment class.
- [x] Every detected Agent path exposes its exact complete value through
  `title`.
- [x] Long paths render on one line with overflow hidden and an ellipsis.
- [x] Short paths remain fully visible.
- [x] No-result content remains contained.
- [x] Path elements and relevant grid/flex ancestors can shrink with
  `min-width: 0` or an equivalent rule.
- [ ] Long paths do not widen an Agent card or create page-level horizontal
  overflow at existing responsive breakpoints.
- [x] Agent selection cards retain stable heights rather than wrapping paths to
  different row counts.

## Agent Select Structure

- [x] One reusable Agent-local wrapper renders the themed selects.
- [x] The wrapper renders a native `<select>`, not a custom listbox.
- [x] The wrapper does not create a second interactive element.
- [x] Every configuration-stage model select uses the wrapper.
- [x] Every configuration-stage typed-option select uses the wrapper.
- [x] Every configuration-stage omission-state select uses the wrapper.
- [x] Existing option children and values are unchanged.
- [x] Existing `onChange` handlers and model configuration state updates are
  unchanged.
- [x] Existing labels, `aria-label` values, focus order, and `combobox` roles
  are preserved.
- [x] Existing disabled behavior is preserved.

## Agent Select Styling

- [x] The wrapper and select fit the width supplied by existing grid/flex
  parents and can shrink without overflow.
- [x] The platform arrow is hidden in supported desktop WebView engines.
- [x] Exactly one theme-controlled chevron is visible.
- [x] The chevron does not intercept pointer events.
- [x] Select text has enough trailing space to avoid overlapping the chevron.
- [x] Default styling uses existing warm surface, strong border, and control
  radius variables or equivalent existing theme tokens.
- [x] Hover styling uses existing signal-theme treatment and does not apply to
  disabled controls.
- [x] Focus-visible styling retains a clear outline and existing focus ring.
- [x] Disabled styling is visibly distinct and does not imply interactivity.
- [x] The select popup remains the platform-native option UI.

## Responsive Behavior

- [x] Agent cards continue to switch to one column at the existing breakpoint.
- [x] The configuration workbench continues to switch to one column at the
  existing breakpoint.
- [x] Typed and omission grids continue to collapse at the existing narrow
  breakpoint.
- [ ] The new wrappers introduce no horizontal page overflow at 800 px, 540 px,
  or the application's supported minimum width.
- [x] Long selected model IDs remain contained within their select controls.

## Automated Verification

- [x] Focused tests assert path class and exact `title` values.
- [x] Focused tests assert Agent selects are inside the themed wrapper.
- [x] Focused tests assert wrapped controls retain native `combobox` roles.
- [x] Existing Agent model selection and preview tests pass unchanged in
  behavior.
- [x] `npm run static:check` passes in `desktop/`.
- [x] `npm run typecheck` passes in `desktop/`.
- [x] `npm test` passes in `desktop/`.
- [x] `npm run build` passes in `desktop/`.

## Manual Visual Acceptance

- [ ] A real desktop application build has been checked at a constrained window
  size with realistic long macOS, Linux, or Windows paths.
- [ ] Long paths remain inside all three Agent cards and expose the complete
  value on hover.
- [ ] Primary/default/active model selects match the warm desktop theme.
- [ ] Role, typed-option, and omission-state selects match the same theme.
- [ ] Default, hover, focus-visible, and disabled states are visually usable.
- [ ] Keyboard selection and focus navigation work in the packaged application.
- [ ] No unintended select-style change is visible outside the Agent page.
- [ ] Manual evidence records the date, platform, application build, and tested
  window dimensions.
