# Warm Desktop UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the desktop application's layered industrial/blue styling with one responsive warm beige and orange card theme without changing application behavior.

**Architecture:** Keep all React markup, state, IPC, and accessibility semantics intact and consolidate `desktop/src/styles.css` into one theme. Use shared design tokens and shared surface/control rules first, then page-specific Router, Agent, Logs, and Settings rules, followed by the existing 800px and 540px responsive transformations.

**Tech Stack:** React 19, TypeScript 5.9, plain CSS, Vite 8, Vitest 4, Tauri 2

---

## File Structure

- Modify `desktop/src/styles.css`: own the complete visual system, shared shell, page-specific components, responsive behavior, focus states, and reduced-motion behavior.
- Do not modify `desktop/src/App.tsx`, `desktop/src/RouterPage.tsx`, `desktop/src/AgentPage.tsx`, `desktop/src/LogsPage.tsx`, or `desktop/src/SettingsPage.tsx` unless manual verification proves a reference-critical layout cannot be represented by the existing class API.
- Use existing tests under `desktop/src/*.test.tsx` as behavioral regression coverage. No style snapshot should be added because jsdom cannot validate layout, overflow, contrast, or rendered appearance reliably.

### Task 1: Establish The Warm Theme And Shared Shell

**Files:**
- Modify: `desktop/src/styles.css`
- Test: `desktop/src/App.test.tsx`

- [ ] **Step 1: Record the clean behavioral baseline**

Run:

```bash
cd desktop && npm test -- src/App.test.tsx
```

Expected: `src/App.test.tsx` passes before any styling changes.

- [ ] **Step 2: Replace the layered global theme with one token system**

Remove both the industrial base tokens and the later blue-theme override tokens. Start the consolidated stylesheet with one warm token set following this shape:

```css
:root {
  color: #302a25;
  background: #f1ece3;
  font-family:
    "Avenir Next", "PingFang SC", "Microsoft YaHei", "Segoe UI", sans-serif;
  font-synthesis: none;
  text-rendering: optimizeLegibility;
  --ink: #302a25;
  --paper: #f7f2ea;
  --paper-deep: #eee6dc;
  --surface: #fbf7f1;
  --surface-raised: #fffaf4;
  --line: #dfd3c5;
  --line-strong: #cdbba9;
  --signal: #dc5b2f;
  --signal-hover: #c94c25;
  --signal-soft: #f8e3d8;
  --warning: #a96f1d;
  --warning-soft: #f8ecd6;
  --danger: #a94737;
  --danger-soft: #f6e1dc;
  --good: #66804c;
  --good-soft: #e7eddf;
  --muted: #74695f;
  --log: #292521;
  --log-muted: #a99e92;
  --shadow-card: 0 12px 30px rgba(76, 57, 43, 0.08);
  --shadow-lifted: 0 16px 36px rgba(76, 57, 43, 0.13);
  --radius-card: 16px;
  --radius-control: 10px;
  --focus-ring: 0 0 0 3px rgba(220, 91, 47, 0.24);
}
```

Use these tokens consistently rather than introducing page-local versions of the same beige, orange, border, or shadow.

- [ ] **Step 3: Rebuild the shared viewport and shell rules**

Preserve `html`, `body`, and `#root` as fixed viewport containers and `main` as the page-level vertical scroller. Style `.app-frame`, `.sidebar`, `.brand`, `.brand-mark`, `.nav-item`, `.sidebar-foot`, `.topbar`, `.build-badge`, and `.content-intro` as the approved warm shell:

```css
.app-frame {
  display: grid;
  grid-template-columns: 236px minmax(0, 1fr);
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: var(--paper);
}

.sidebar {
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
  color: var(--ink);
  background: var(--paper-deep);
  border-right: 1px solid var(--line);
}

.nav-item {
  display: grid;
  grid-template-columns: 32px 1fr;
  gap: 8px;
  align-items: center;
  min-height: 48px;
  margin: 3px 12px;
  padding: 0 14px;
  color: var(--muted);
  text-align: left;
  border: 1px solid transparent;
  border-radius: var(--radius-control);
  background: transparent;
  cursor: pointer;
}

.nav-item:hover {
  color: var(--ink);
  background: var(--surface);
  border-color: var(--line);
}

.nav-item.is-active {
  color: var(--signal-hover);
  background: var(--signal-soft);
  border-color: rgba(220, 91, 47, 0.28);
}
```

Keep the existing responsive visibility hooks `.nav-label--full` and `.nav-label--short`. Use warm orange focus rings for `button`, `a`, `input`, and `select` focus-visible states.

- [ ] **Step 4: Run shell regression tests**

Run:

```bash
cd desktop && npm test -- src/App.test.tsx
```

Expected: all App tests pass; navigation still changes mounted pages and Router-to-Agent navigation still works.

- [ ] **Step 5: Commit the shared theme**

```bash
git add desktop/src/styles.css
git commit -m "style: establish warm desktop theme"
```

### Task 2: Restyle Router Status Surfaces

**Files:**
- Modify: `desktop/src/styles.css`
- Test: `desktop/src/RouterPage.test.tsx`

- [ ] **Step 1: Confirm Router behavior before page-specific CSS**

Run:

```bash
cd desktop && npm test -- src/RouterPage.test.tsx
```

Expected: Router tests pass.

- [ ] **Step 2: Convert the Router container into rounded cards**

Style `.panel-grid` as a cream rounded card with a quiet sand border and card shadow. Keep its two-column grid above 800px. Style `.primary-panel` and `.status-rail` without industrial hard-black dividers. Use an inset beige surface for `.instrument`, `.readout-grid > div`, `.protocol-readout`, and `.notice`.

The central rules should follow this geometry:

```css
.panel-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 250px;
  overflow: hidden;
  border: 1px solid var(--line);
  border-radius: var(--radius-card);
  background: var(--surface);
  box-shadow: var(--shadow-card);
}

.primary-panel {
  min-width: 0;
  padding: 26px;
  border-right: 1px solid var(--line);
}

.instrument {
  display: flex;
  gap: 20px;
  align-items: center;
  margin: 30px 0;
  padding: 20px;
  border: 1px solid var(--line);
  border-radius: 14px;
  background: var(--paper-deep);
}

.instrument__dial {
  display: grid;
  flex: 0 0 68px;
  height: 68px;
  place-items: center;
  color: var(--signal-hover);
  border: 1px solid rgba(220, 91, 47, 0.24);
  border-radius: 18px;
  background: var(--signal-soft);
  font-weight: 800;
  transform: none;
}

.instrument__dial span {
  transform: none;
}
```

- [ ] **Step 3: Preserve every Router semantic tone**

Map `.panel-grid--active`, `.signal--active`, and `.health-value--healthy` to `--good`; warning/degraded/stale to `--warning`; danger/occupied/failed/reinstall to `--danger`; idle/unknown/checking to neutral or orange. Use soft backgrounds plus text or border differences so status is not represented by color alone.

Restyle `.signal` as a rounded status pill, `.status-index` as an orange-tinted badge, `.failure-diagnostics` as a brick-red soft card, and `.inline-alert` as a warning/danger message without changing DOM or content.

- [ ] **Step 4: Restyle shared action controls**

Define `.control-button` as a filled orange rounded button, `.text-button` as a cream or transparent bordered button, and `.control-button--stop` as a brick-red destructive button. Include hover, active, focus-visible, and disabled rules. Disabled controls must reduce contrast and remove lift without becoming unreadable.

- [ ] **Step 5: Run Router regression tests**

Run:

```bash
cd desktop && npm test -- src/RouterPage.test.tsx
```

Expected: all Router state, action, diagnostics sanitization, and navigation tests pass.

- [ ] **Step 6: Commit the Router treatment**

```bash
git add desktop/src/styles.css
git commit -m "style: soften router status surfaces"
```

### Task 3: Restyle Agent Workflow Surfaces

**Files:**
- Modify: `desktop/src/styles.css`
- Test: `desktop/src/AgentPage.test.tsx`

- [ ] **Step 1: Confirm Agent workflow behavior before page-specific CSS**

Run:

```bash
cd desktop && npm test -- src/AgentPage.test.tsx
```

Expected: Agent tests pass.

- [ ] **Step 2: Apply the shared warm card language**

Style `.agents-workbench` as the page card, `.stage-meter span` as quiet rounded step badges, and `.stage-meter .is-current` with the orange primary treatment. Convert `.agent-card`, `.preview-agent`, `.agent-file`, `.approval-rail`, `.result-banner`, and `.result-grid > article` to cream cards with sand borders and 12-16px radii.

Selected Agent cards use orange border/ring and a slight lift:

```css
.agent-card {
  min-width: 0;
  padding: 20px;
  border: 1px solid var(--line);
  border-radius: 14px;
  background: var(--surface-raised);
  transition: border-color 160ms ease, box-shadow 160ms ease, transform 160ms ease;
}

.agent-card.is-selected {
  border-color: rgba(220, 91, 47, 0.55);
  box-shadow: var(--focus-ring), var(--shadow-lifted);
  transform: translateY(-2px);
}
```

Keep native checkboxes and style their accent with `accent-color: var(--signal)`.

- [ ] **Step 3: Preserve security and operation distinctions**

Use separate soft semantic treatments for `.migration-warning`, `.backup-plan`, `.sensitive-copy`, `.file-warning`, `.preview-warnings`, `.risk-box`, `.rollback-note`, `.result-ok`, and `.result-fail`. Migration and backup warnings use ochre; write failures and rollback use brick red; success uses olive; ordinary create/replace operation pills use orange or neutral tones.

Style `.key-field input` and Agent file/path code surfaces with cream or inset beige backgrounds, visible orange focus rings, and safe wrapping. Do not hide `.backup-plan`, warning copy, native input labels, or disabled states.

- [ ] **Step 4: Keep workflow layouts flexible**

Retain three Agent selection columns and split preview/approval columns above 800px. Preserve the existing stacked forms below 800px and the single-column file cards and full-width actions below 540px. Do not add fixed card heights or `white-space: nowrap` to translated status pills.

- [ ] **Step 5: Run Agent regression tests**

Run:

```bash
cd desktop && npm test -- src/AgentPage.test.tsx
```

Expected: all selection, preview, credential, stale-preview, backup disclosure, sanitization, and result tests pass.

- [ ] **Step 6: Commit the Agent treatment**

```bash
git add desktop/src/styles.css
git commit -m "style: unify agent workflow cards"
```

### Task 4: Restyle Logs And Settings

**Files:**
- Modify: `desktop/src/styles.css`
- Test: `desktop/src/LogsPage.test.tsx`
- Test: `desktop/src/SettingsPage.test.tsx`

- [ ] **Step 1: Confirm Logs and Settings behavior before page-specific CSS**

Run:

```bash
cd desktop && npm test -- src/LogsPage.test.tsx src/SettingsPage.test.tsx
```

Expected: both test files pass.

- [ ] **Step 2: Build the warm Logs card with a dark body**

Style `.logs-panel` as a rounded cream card and `.logs-toolbar`/`.logs-foot` as warm light surfaces. Keep `.log-screen` dark and internally scrollable:

```css
.logs-panel {
  overflow: hidden;
  border: 1px solid var(--line);
  border-radius: var(--radius-card);
  background: var(--surface);
  box-shadow: var(--shadow-card);
}

.log-screen {
  min-height: 260px;
  max-height: calc(100dvh - 340px);
  overflow: auto;
  color: #eee6dc;
  background: var(--log);
  border-block: 1px solid #403a34;
}

.log-screen li > span {
  color: var(--log-muted);
}
```

Use orange-muted hover and scrollbar accents without reducing code readability. Keep the current `role="log"`, loading/empty content, and footer status behavior untouched.

- [ ] **Step 3: Separate Settings into warm cards**

Keep `.settings-grid` as a two-column grid above 800px but add gaps rather than shared hard dividers. Style each `.settings-block` as an independent rounded cream card. Make `.toggle.is-on` orange with a cream thumb, `.language-select select` a rounded native select with an orange focus ring, and version indices orange-tinted badges.

Style `.settings-block--danger` with `--danger-soft`, a brick-red border, and the destructive button variant. Ensure long paths in `.settings-block--locations dd` wrap safely.

- [ ] **Step 4: Run Logs and Settings regression tests**

Run:

```bash
cd desktop && npm test -- src/LogsPage.test.tsx src/SettingsPage.test.tsx
```

Expected: log loading/actions and Settings switch/language/version/uninstall tests pass. The language select remains directly wrapped by `.language-select`.

- [ ] **Step 5: Commit Logs and Settings**

```bash
git add desktop/src/styles.css
git commit -m "style: refresh logs and settings cards"
```

### Task 5: Consolidate Responsive And Interaction States

**Files:**
- Modify: `desktop/src/styles.css`
- Test: `desktop/src/App.test.tsx`
- Test: `desktop/src/RouterPage.test.tsx`
- Test: `desktop/src/AgentPage.test.tsx`
- Test: `desktop/src/LogsPage.test.tsx`
- Test: `desktop/src/SettingsPage.test.tsx`

- [ ] **Step 1: Add one 800px responsive block**

Retain the established behavior in one `@media (max-width: 800px)` block:

- Sidebar width becomes 82px.
- Brand copy and sidebar footer copy hide.
- Full navigation labels hide and short labels show.
- Router main/status rail stack and the primary-panel right border disappears.
- Agent selection, preview/approval, preview files, and result grids stack.
- Settings becomes one column.
- Logs actions wrap without leaving the card.

- [ ] **Step 2: Add one 540px responsive block**

Retain the established behavior in one `@media (max-width: 540px)` block:

- Sidebar width becomes 64px and navigation padding contracts.
- Main padding contracts without removing the content gutter.
- Build badge and page counter hide.
- Router heading, action row, readouts, and version rows stack.
- Log actions and footer stack.
- Agent headers, toolbar, footer actions, and preview files stack.
- Settings rows and destructive section stack; select and buttons can fill available width.

- [ ] **Step 3: Complete interaction and reduced-motion states**

Audit every button, link, input, select, switch, Agent card, and scrollable surface for hover, focus-visible, active, and disabled styling. Add a single reduced-motion block:

```css
@media (prefers-reduced-motion: reduce) {
  *,
  *::before,
  *::after {
    scroll-behavior: auto !important;
    transition-duration: 0.01ms !important;
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
  }
}
```

- [ ] **Step 4: Confirm the stylesheet contains one theme and one copy of each breakpoint**

Run:

```bash
rg -n '^:root|^@media \(max-width: 800px\)|^@media \(max-width: 540px\)' desktop/src/styles.css
```

Expected: one `:root`, one 800px block, and one 540px block. No later blue-theme override comment or blue primary tokens remain.

- [ ] **Step 5: Run the complete frontend suite**

Run:

```bash
cd desktop && npm run static:check && npm run typecheck && npm test && npm run build
```

Expected: ESLint and Prettier pass, TypeScript emits no errors, all Vitest tests pass, and Vite builds successfully.

- [ ] **Step 6: Commit responsive consolidation**

```bash
git add desktop/src/styles.css
git commit -m "style: finalize responsive warm UI"
```

### Task 6: Verify Native Desktop Integration And Visual Acceptance

**Files:**
- Verify: `desktop/src/styles.css`
- Verify: `docs/superpowers/specs/2026-07-15-warm-desktop-ui-design.md`

- [ ] **Step 1: Run Rust and workflow verification**

Run:

```bash
cd desktop && npm run rust:format && npm run rust:test
```

Expected: Rust formatting check and locked Cargo tests pass.

Run from the repository root:

```bash
bash tests/desktop_workflow_test.sh
```

Expected: desktop workflow checks pass.

- [ ] **Step 2: Launch the desktop development application**

Run:

```bash
cd desktop && npm run tauri dev
```

Expected: the Tauri window opens at the configured desktop size with the warm theme and no runtime console or rendering error.

- [ ] **Step 3: Inspect shared and responsive layouts**

Manually check widths 1120px, 800px, 540px, and 360px and minimum height 580px. Confirm:

- Sidebar remains usable in full and compact forms.
- `main` remains the page-level scroller.
- No horizontal overflow appears.
- Shadows and focus rings are not clipped.
- Chinese and English labels wrap without hiding controls.

- [ ] **Step 4: Inspect critical states on all pages**

Confirm Router neutral/healthy/degraded/failure states, Agent loading/selection/preview/key/result states, Logs loading/empty/populated states, and Settings on/off/disabled/danger states. Verify warning, failure, selected, disabled, and primary-action treatments remain distinct.

- [ ] **Step 5: Review the final diff for scope**

Run:

```bash
git status --short
```

Expected: product changes are limited to the approved visual stylesheet; no IPC, state, security, or copy behavior changed. If a TSX change became necessary, the diff explicitly preserves its semantics and has matching tests.

- [ ] **Step 6: Record final verification without creating an empty commit**

If manual inspection requires CSS corrections, apply them, repeat Tasks 5 and 6, then commit:

```bash
git add desktop/src/styles.css
git commit -m "fix: polish warm desktop UI"
```

If no correction is needed, leave the verified commits unchanged.
