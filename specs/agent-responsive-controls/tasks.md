# Agent Responsive Controls Tasks

Tasks are dependency-ordered. A task is complete only when its implementation,
focused tests, and stated verification pass. Execution must not begin until
this specification package is explicitly approved.

## Phase 1: Responsive Path Containment

- [x] **1.1 Add a dedicated Agent-card path contract**
  - Mark the detected path element with a path-specific class.
  - Expose the complete detected path through `title`.
  - Ensure the element and relevant layout ancestors can shrink.
  - Apply single-line overflow hiding and ellipsis without changing card data.
  - Keep no-result content contained.
  - Verification: focused `AgentPage` component test plus desktop static check.

## Phase 2: Agent Select Theme

- [x] **2.1 Add a minimal native-select wrapper**
  - Introduce one Agent-local wrapper that accepts normal select props and
    renders a native `<select>` plus a decorative chevron.
  - Preserve children, values, handlers, labels, ARIA attributes, and disabled
    state without transforming model data.
  - Use the wrapper for every select in the Agent configuration stage.
  - Do not modify selects on other pages.
  - Verification: `npm run typecheck` and focused component tests.

- [x] **2.2 Implement scoped themed select styling**
  - Add full-width, shrink-safe wrapper and select rules.
  - Hide the platform arrow and reserve text padding for one custom chevron.
  - Reuse existing surface, border, radius, signal, and focus variables.
  - Add default, hover, focus-visible, and disabled treatments.
  - Prevent the decorative chevron from intercepting pointer input.
  - Preserve existing flex and grid layouts at all responsive breakpoints.
  - Verification: `npm run static:check` and `npm run build`.

## Phase 3: Regression Evidence

- [x] **3.1 Add focused Agent page regressions**
  - Assert each detected path has the containment class and exact full `title`.
  - Assert configuration-stage selects are wrapped by the Agent-local theme.
  - Assert wrapped controls retain native `combobox` roles.
  - Retain existing assertions that model changes update preview input.
  - Verification: `npm test -- AgentPage.test.tsx`.

- [x] **3.2 Run the desktop verification suite**
  - Run formatting/static checks and TypeScript type checking.
  - Run the desktop test suite.
  - Build the desktop frontend for production.
  - Record any unavailable command rather than marking it complete without
    evidence.
  - Verification: `npm run static:check`, `npm run typecheck`, `npm test`, and
    `npm run build` from `desktop/`.

- [ ] **3.3 Record real narrow-window visual acceptance**
  - Open the Agent selection stage at a constrained desktop width with long
    paths and confirm no card or page horizontal overflow.
  - Open the configuration stage and inspect model, typed-option, and omission
    selects in default, hover, focus, and disabled states.
  - Confirm Settings and other pages have no unintended select-style change.
  - Record platform, application build, viewport/window size, and result.
  - Verification: dated manual evidence linked or written into the acceptance
    record used for this change.
