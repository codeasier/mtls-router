# Warm Desktop UI Design

**Issue:** [#70](https://github.com/codeasier/mtls-router/issues/70)

## Goal

Replace the desktop application's remaining industrial and blue native-client styling with the warm, minimal card language shown in the supplied reference. The redesign must make the interface feel calmer and more approachable without changing its information architecture, behavior, security boundaries, or user-facing copy.

## Scope

The redesign covers the shared shell and all four desktop pages:

- Sidebar, brand, navigation, page header, and content introduction
- Router status, health, version, diagnostics, and action surfaces
- Agent selection, preview, approval, credential, and result surfaces
- Log toolbar, code view, and status footer
- Settings controls, versions, paths, and uninstall warning

The left sidebar remains the primary navigation. The log body remains a dark code surface inside a warm light card. No system-theme switching, new icon library, component framework, or application behavior is added.

## Implementation Strategy

Consolidate `desktop/src/styles.css` into one coherent warm theme while retaining the existing class API and responsive layout behavior. The current file contains an industrial base followed by a light-blue override layer; adding another override would make conditional and responsive styles harder to reason about. A single stylesheet prevents old colors, square geometry, and high-contrast borders from leaking into less common states.

Existing React markup already exposes sufficient semantic class names. `App.tsx`, `RouterPage.tsx`, `AgentPage.tsx`, `LogsPage.tsx`, and `SettingsPage.tsx` should remain unchanged unless a reference-critical detail cannot be expressed safely in CSS. Any markup adjustment must preserve accessible names, semantic elements, test selectors, page mounting behavior, and control order.

## Visual System

Use a small set of CSS custom properties for the complete visual system:

- Warm beige application background
- Slightly deeper beige sidebar and inset surfaces
- Cream or off-white cards
- Deep brown-gray primary text
- Warm taupe secondary text
- Burnt orange primary action and selected-state color
- Sand-colored low-contrast borders
- Muted olive success, ochre warning, and brick-red danger colors
- Warm charcoal log background

Primary cards use 10-16px corner radii. Buttons, inputs, selects, status labels, and navigation items use related rounded geometry. Borders remain visible but quiet. Soft shadows are limited to major cards, selected surfaces, and raised interaction states so the application does not become visually noisy.

Typography uses the existing local system-oriented font stack and does not fetch web fonts. Monospace remains reserved for paths, versions, state codes, and log output rather than serving as a general visual motif.

## Shared Shell

Keep the fixed viewport and `main` as the sole page-level vertical scroller. The sidebar remains fixed-width on normal desktop windows and keeps its existing compact forms at the 800px and 540px breakpoints.

The sidebar becomes a warm light surface separated from the content by a sand border. The brand mark uses the orange accent. Navigation items become rounded controls with a cream hover state and a soft orange selected state. Active and focus states remain distinguishable without depending solely on text color. The local-mode footer remains visually subordinate but readable.

The top bar and introduction lose hard industrial dividers and use spacing and restrained borders for hierarchy. The build badge becomes a quiet pill rather than an instrument label.

## Router Page

Convert the joined industrial panel into a rounded primary card with a responsive main area and status rail. The state code remains available but the rotated gauge treatment becomes a calm status medallion. State tone appears through the medallion, status pill, and a restrained accent rather than thick rails or glowing instrumentation.

The definition-list readouts remain semantic and become individually separated inset cards. The version rail becomes a light secondary card with compact orange-tinted indices. Diagnostics, protocol notices, and action feedback preserve distinct neutral, warning, and danger treatments.

Start, stop, retry, reinstall, and Agent navigation behavior remains unchanged. External ownership, stale health, occupied ports, start failures, and sidecar reinstall states must remain visually distinguishable.

## Agent Page

The workbench becomes a warm card container with a rounded stage meter. Selection cards use a cream background, sand border, soft hover elevation, and an orange selected ring or inset border. Detection states retain neutral, ready, active, and danger semantics.

Preview file cards, the approval rail, and result cards use the same card geometry while keeping security-relevant regions distinct. Migration warnings, sensitive backups, preview warnings, credential entry, rollback notices, and write failures must not be flattened into the primary orange style. Their labels, borders, and semantic colors remain clearly differentiated.

Native checkboxes and the password input remain in the DOM and retain their current labels, order, disabled state, and focus behavior. The API key lifecycle and all sanitization boundaries remain untouched.

## Logs Page

Use a rounded warm outer card for the toolbar, log screen, and footer. Keep the log body as a warm-charcoal code surface because it provides useful contrast for dense output. Line numbers, hover rows, empty/loading messages, scrollbars, and focus treatment should use muted warm colors rather than the existing blue or acid accents.

Log loading, opening, diagnostic copying, sanitization, live-region semantics, and internal scrolling remain unchanged.

## Settings Page

Convert the currently joined settings grid into separate cards with consistent gaps. General controls, component versions, locations, and uninstall preparation each retain their semantic grouping. The switch, language select, version indices, and path readouts adopt the rounded warm component language.

The uninstall section keeps a clearly destructive brick-red treatment. Autostart loading and disabled states, native-language synchronization, confirmation, partial-load handling, and the `.language-select` wrapper remain unchanged.

## Responsive Behavior

Preserve the existing breakpoints and minimum supported Tauri window size:

- Above 800px: full sidebar, split Router panel, multi-column Agent and Settings layouts
- At or below 800px: compact sidebar, stacked Router rail, single-column Settings, stacked Agent selection and approval areas
- At or below 540px: narrower sidebar, hidden secondary header metadata, stacked actions and readouts, full-width form controls
- At 360px width and 580px height: no page-level horizontal overflow and no inaccessible controls

Avoid fixed card heights and one-line assumptions. Chinese and English labels, warnings, paths, and status pills must wrap or truncate only where the existing semantics permit it. Shadows and focus rings must not be clipped by rounded containers.

## Accessibility

Preserve native controls, ARIA roles, heading order, definition lists, live regions, and keyboard order. Focus indicators must remain clearly visible against beige, cream, orange, and charcoal surfaces. Hover, focus, selected, disabled, loading, success, warning, and danger states must use more than color alone where the existing content allows it.

Choose final token values with sufficient contrast for small secondary text, orange controls, status pills, and disabled states. Reduced-motion behavior remains supported.

## Testing And Verification

Automated verification from `desktop/`:

```bash
npm run static:check
npm run typecheck
npm test
npm run build
npm run rust:format
npm run rust:test
```

Also run `bash tests/desktop_workflow_test.sh` from the repository root.

Manual verification covers Router, Agents, Logs, and Settings in Chinese and English at 1120px, around both responsive breakpoints, and at the 360px minimum width. Check keyboard focus, long content, internal scrolling, loading and disabled controls, and all available healthy, warning, failure, and destructive states.

## Non-Goals

- Changing navigation structure from sidebar to top navigation
- Changing IPC calls, state machines, persistence, security behavior, or copy
- Adding dark mode or system-theme support
- Adding external fonts, an icon package, or a component framework
- Refactoring React components solely for style organization
- Updating historical planning documents that are not current product documentation
