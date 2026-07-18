# Agent Responsive Controls Specification

## Change ID

`agent-responsive-controls`

## Status

Execution approved on 2026-07-18. Implementation and automated verification
are complete; packaged-application visual acceptance remains pending.

## Motivation

Low-resolution desktop testing exposed two Agent-page presentation defects:

1. Detected Agent configuration paths can extend beyond their selection cards.
2. Native platform select styling in the model configuration workbench is
   visually inconsistent with the warm-paper desktop theme.

These defects reduce readability and make the Agent flow appear unfinished,
but do not require changes to model configuration behavior or protocol data.

## Goals

- Keep every detected configuration path within its Agent selection card.
- Preserve a stable, equal-height card layout at constrained desktop widths.
- Make Agent workbench selects consistent with the existing desktop theme.
- Preserve native select semantics, keyboard interaction, labels, and state.
- Limit the change to the Agent desktop page.

## Non-Goals

- Replacing native selects with a custom listbox or option popup.
- Globally restyling selects on Settings or other desktop pages.
- Changing model choices, defaults, validation, or configuration state.
- Changing localization text.
- Changing Rust, IPC, Manager, router, or management protocol behavior.
- Redesigning the Agent workflow, cards, model catalog, or responsive shell.

## Functional Requirements

### FR-1: Contained configuration paths

Each detected path shown in an Agent selection card must remain within the
card's content width. The visible value must use a single line with overflow
hidden and an ellipsis when the complete path does not fit.

The complete path must remain available in the rendered document through the
path element's `title` attribute. The path and each relevant flex/grid ancestor
must be allowed to shrink rather than increasing the card or grid width.

No-result text must remain safe and contained under the same narrow layout.

### FR-2: Agent-local themed native select

Every native `<select>` in the Agent model configuration workbench must render
inside one Agent-local visual wrapper. The wrapper must:

- occupy the width supplied by its existing flex or grid layout
- hide the platform select arrow
- draw one non-interactive theme-controlled chevron
- use the existing warm raised surface, strong border, and control radius
- use the existing signal palette for interactive emphasis
- preserve legible text and sufficient room for the chevron

The wrapper must not impose a fixed width that can overflow a narrow panel.

### FR-3: Interaction states

Agent workbench selects must provide theme-consistent default, hover,
focus-visible, and disabled states. Focus-visible behavior must retain the
repository's existing visible outline and focus ring. Disabled controls must
remain distinguishable and must not present an active cursor or hover effect.

The decorative chevron must not intercept pointer events.

### FR-4: Native behavior and accessibility

The implementation must retain native `<select>` controls. Existing values,
change handlers, labels, `aria-label` attributes, disabled state, keyboard
navigation, focus order, and `combobox` accessibility semantics must remain
unchanged.

The implementation must not create a second interactive control or manually
implement option selection behavior.

### FR-5: Scope containment

The themed wrapper must cover all selects rendered in the Agent configuration
stage, including:

- primary, role, default, and active model selects
- typed model option selects
- omission-state selects

Selects outside the Agent page, including the Settings language select, must
retain their current markup and behavior.

### FR-6: Narrow-layout compatibility

At the existing responsive breakpoints, Agent cards, select wrappers, and
native selects must fit their parent width without creating horizontal page
overflow. Existing one-column Agent card and configuration workbench behavior
must remain intact.

## Observable Scenarios

### Scenario 1: Long detected path

Given a detected Agent path longer than the available card width, when the
selection stage is displayed at a constrained desktop width, then the visible
path is truncated with an ellipsis, the card is not widened, and the complete
path is present in `title`.

### Scenario 2: Short detected path

Given a path that fits in the card, when the selection stage is displayed,
then the full path remains visible and the same `title` value is available.

### Scenario 3: Model selection

Given the configuration stage, when a user focuses and changes any model
select with mouse or keyboard, then the control uses the themed wrapper while
the existing React state update and native select behavior remain unchanged.

### Scenario 4: Optional and disabled fields

Given typed option or omission-state selects, when controls are enabled or
disabled, then their state is visually clear and their existing values and
disabled behavior remain unchanged.

### Scenario 5: Other desktop pages

Given the Settings page language select, when this change is present, then its
existing page-specific wrapper and styling remain unchanged.

## Impact

Expected product-code impact is limited to:

- `desktop/src/AgentPage.tsx`
- `desktop/src/styles.css`
- focused tests in `desktop/src/AgentPage.test.tsx`

No persisted format, external interface, dependency, build metadata, or
localized contract changes.

## Verification Strategy

Automated verification must assert:

- detected path elements use the dedicated containment class
- complete detected paths are exposed through `title`
- Agent configuration selects are inside the themed wrapper
- wrapped controls remain native elements with `combobox` roles
- existing model-selection tests continue to exercise state updates
- current narrow-layout source assertions continue to pass

The desktop static check, type check, test suite, and production build must
pass. A real application narrow-window check should confirm that no Agent card
path or workbench select causes horizontal overflow and that the controls match
the desktop theme on the target platform.

## Acceptance Boundary

The change is complete when every checklist item has automated or recorded
manual evidence. Visual similarity alone is insufficient if native select
behavior, accessibility semantics, or narrow-layout containment regresses.
