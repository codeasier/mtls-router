# OpenCode Model Variants Design

## Goal

Add first-class OpenCode model `variants` support to the canonical Agent model
configuration and include the complete confirmed reasoning-effort matrix in the
local v1 model preset. Correct the Claude preset and canonical configuration so
Claude Code knows the numeric token budgets of the custom GPT 5.6 models.

This work also changes every explicit default reasoning effort in the v1 preset
to `medium`. That includes all five OpenCode model defaults and the Codex
default. Claude selections do not have a reasoning-effort field.

## Scope

The change covers:

- canonical OpenCode model types, validation, and JSON Schema;
- OpenCode rendering;
- compatibility with the existing `extra.variants` extension;
- canonical Claude context/output budgets and their rendering lifecycle;
- the local `tmp/agent-model-preset-v1.json` target preset;
- the local Chinese preset guide;
- focused tests and repository validation.

Claude Fable support remains a prerequisite for the complete target preset to
pass strict preflight. The Claude numeric-budget support in this design is an
additional prerequisite and does not remove or bypass the Fable blocker.

## Canonical Shape

Each OpenCode model gains an optional top-level `variants` field alongside
`options`, `limit`, and `modalities`:

```json
{
  "version": 1,
  "opencode": {
    "default_model": "gpt-5.6-sol",
    "models": {
      "gpt-5.6-sol": {
        "options": {
          "reasoningEffort": "medium"
        },
        "variants": {
          "none": {
            "reasoningEffort": "none"
          },
          "minimal": {
            "reasoningEffort": "minimal"
          },
          "low": {
            "reasoningEffort": "low"
          },
          "medium": {
            "reasoningEffort": "medium"
          },
          "high": {
            "reasoningEffort": "high"
          },
          "xhigh": {
            "reasoningEffort": "xhigh"
          },
          "max": {
            "reasoningEffort": "max"
          }
        }
      }
    }
  }
}
```

The canonical schema treats `variants` as a map from extensible variant names
to safe provider-option objects. It does not enumerate reasoning levels or
require `reasoningEffort`, because OpenCode supports provider-specific variant
names and option shapes.

Variant names must be non-empty configuration keys. Each variant value must be
an object and is validated with the canonical extension-tree limits and
protected-path rules. This preserves bounded recursion and rejects credentials,
authentication headers, URLs, and other protected configuration.

## Compatibility

Canonical model-config v1 already accepts `extra.variants`, and the renderer
currently promotes that value into the rendered OpenCode model object. Existing
valid v1 documents must continue to work.

The compatibility rules are:

- new documents should use top-level `variants`;
- documents containing only `extra.variants` remain valid and render as before;
- a model containing both top-level `variants` and `extra.variants` is invalid;
- the manager never merges the two locations or applies an implicit priority.

Rejecting dual definitions avoids ambiguous output while preserving existing
persisted or externally supplied canonical v1 documents.

## Claude Numeric Budgets

Claude Code supports numeric overrides for the context window and maximum output
tokens it assumes for the active model. These overrides are especially relevant
behind a custom `ANTHROPIC_BASE_URL`, where model IDs such as `gpt-5.6-sol` are
not recognized as native Claude model names.

The Claude canonical section gains two optional typed fields:

```json
{
  "claude": {
    "context_window": 353400,
    "max_output_tokens": 100000
  }
}
```

Both fields are positive safe integers and may be omitted independently. When
both are present, `max_output_tokens` must be less than `context_window`.

Numeric `context_window` and per-selection `context: "1m"` represent two
different Claude Code mechanisms and must not be mixed in one Claude section.
If `context_window` is present, the primary selection and every explicit role
selection must omit `context`. Inherited roles remain valid because they inherit
the primary selection, which is subject to the same rule.

The numeric values describe Claude Code's budgeting and auto-compaction model;
they do not change the upstream model's real capabilities. The manager does not
derive these values from model IDs, catalog order, or other metadata.

At the Claude rendering boundary, the typed fields produce:

```text
CLAUDE_CODE_MAX_CONTEXT_TOKENS=353400
CLAUDE_CODE_MAX_OUTPUT_TOKENS=100000
```

The manager owns these keys when their typed fields are configured. Existing
configuration extraction, preview drift detection, sidecar ownership, managed
replacement, and removal of stale owned keys all include them. They are not
accepted through Claude `extra`, which remains restricted to its documented
extension keys.

## Rendering

The OpenCode renderer emits canonical top-level `variants` directly into the
managed provider model entry. The output location is:

```text
provider.mtls-router.models.<model-id>.variants
```

The field is a peer of `options`, `limit`, and `modalities`. Existing deep merge
behavior for `extra` remains unchanged. Since validation rejects dual variant
definitions, rendering has no precedence rule to apply.

## V1 Preset Matrix

The target preset explicitly declares the supported reasoning variants. The
matrix is product metadata and is not mechanically limited to the variants
currently written in the local `unraid-wg` provider configuration.

`gpt-5.4` and `gpt-5.5` contain six variants:

```text
none, minimal, low, medium, high, xhigh
```

`gpt-5.6-sol`, `gpt-5.6-terra`, and `gpt-5.6-luna` contain seven variants:

```text
none, minimal, low, medium, high, xhigh, max
```

Every variant maps to a same-named `reasoningEffort` value. The mapping is
written explicitly in the preset; the manager does not infer it from model IDs
or default options.

Every OpenCode model changes its default `options.reasoningEffort` to `medium`.
Codex also changes from `reasoning_effort: "max"` to
`reasoning_effort: "medium"`. The OpenCode default model remains
`gpt-5.6-sol`.

The Claude preset changes Haiku from `gpt-5.5` to `gpt-5.6-luna`, with display
name `GPT 5.6 Luna`. The resulting Claude mapping is:

| Claude position | Model | Display name |
|---|---|---|
| primary | `gpt-5.6-sol` | `GPT 5.6 Sol` |
| fable | `gpt-5.6-sol` | `GPT 5.6 Sol` |
| opus | `gpt-5.6-terra` | `GPT 5.6 Terra` |
| sonnet | `gpt-5.6-luna` | `GPT 5.6 Luna` |
| haiku | `gpt-5.6-luna` | `GPT 5.6 Luna` |

All Claude selections therefore use models with a 353,400-token context window
and 100,000-token output limit. The Claude preset sets `context_window: 353400`
and `max_output_tokens: 100000`. `gpt-5.5` remains in the complete OpenCode
model list but is no longer selected by Claude.

Claude Code 2.1.193 or newer applies `CLAUDE_CODE_MAX_CONTEXT_TOKENS` directly
to unrecognized custom model names. Older versions may ignore this numeric
override, but that does not make the preset or Claude Code unusable; Claude Code
falls back to its own context-window assumptions and compaction behavior. The
numeric-budget feature therefore does not impose a minimum Claude Code version.
Fable selection remains independently dependent on Claude Code 2.1.170 or
newer.

## Validation Errors

Invalid input reports canonical validation paths without rejected values. Tests
must cover:

- a non-object `variants` value;
- a non-object individual variant value;
- invalid or empty variant names;
- protected keys nested in a variant;
- recursive-size and depth limits inherited from extension-tree validation;
- simultaneous top-level `variants` and `extra.variants`.

Claude validation tests must also cover:

- non-integer, non-positive, and unsafe integer token budgets;
- `max_output_tokens >= context_window` when both fields are present;
- numeric `context_window` combined with primary or explicit-role
  `context: "1m"`;
- independent omission of either numeric field.

The dual-definition failure should use a stable validation rule that identifies
a field conflict rather than silently selecting one value.

## Schema And Tests

The generated and checked-in model-config v1 JSON Schema must expose top-level
OpenCode model `variants`. The schema describes an object whose additional
properties are safe option objects; authoritative recursive and protected-key
checks remain manager validation responsibilities.

Tests cover:

- typed decode and canonical round-trip of top-level variants;
- schema generation synchronization;
- exact OpenCode render output;
- compatibility rendering for legacy `extra.variants`;
- rejection of dual definitions and unsafe variant trees;
- preset loading with the new field;
- exact six-level matrix for GPT 5.4 and GPT 5.5;
- exact seven-level matrix for all GPT 5.6 models;
- `medium` OpenCode defaults for all five models;
- `medium` Codex default;
- Claude Haiku mapped to `gpt-5.6-luna`;
- Claude numeric budget canonical round-trip and exact environment rendering;
- existing-config extraction and managed ownership of both budget variables;
- stale budget-variable cleanup and drift detection;
- numeric budget relationship and `[1m]` conflict validation.

Repository verification runs:

```bash
go test ./...
go vet ./...
test -z "$(gofmt -l .)"
```

The preset JSON is checked with `jq`. Strict preset preflight must no longer
reject OpenCode `variants` or Claude numeric budgets. If Claude Fable support
has not yet been completed, the complete target preset remains expected to fail
only on the independent unknown `fable` field.

## Documentation

`tmp/AGENT_MODEL_PRESET_GUIDE.zh-CN.md` is updated to:

- list the complete six-level and seven-level OpenCode matrices;
- state that these variants are confirmed product metadata rather than a blind
  copy of the currently explicit local provider variants;
- change all OpenCode and Codex default reasoning efforts to `medium`;
- change Claude Haiku to `gpt-5.6-luna`;
- document `context_window: 353400`, `max_output_tokens: 100000`, their rendered
  environment variables, and that numeric context overrides become effective
  for unrecognized custom models on Claude Code 2.1.193 or newer without making
  that version a preset minimum;
- explain that numeric overrides affect Claude Code budgeting and compaction but
  do not change upstream model capability;
- replace the old claim that Claude cannot configure a numeric context window;
- remove the statement that variants are omitted because canonical config does
  not accept them;
- retain the Claude Fable preflight blocker and existing secret-handling rules.
