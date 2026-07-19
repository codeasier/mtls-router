# Agent Configuration Presets and Claude Context Specification

## Change ID

`agent-config-presets-claude-context`

## Status

Implemented with integrated verification complete on macOS. Executable
PowerShell verification remains pending because `pwsh` is unavailable in the
execution environment; see `checklist.md`.

## Relationship to Existing Specifications

This package extends `specs/agent-models-config/spec.md`. All unaffected
authenticated discovery, strict canonical validation, catalog-token,
write-time refresh, preview, approval, ownership, backup, rollback, atomic
write, and secret-handling requirements remain in force.

Where the existing specification prohibits automatic model selection, this
package distinguishes two behaviors:

- First-model selection, model-name heuristics, capability inference, model
  substitution, and cached/static fallback remain prohibited.
- A build-injected preset may become an editable initial value only after the
  manager validates its exact model IDs against the current authenticated
  catalog as specified here.

## Motivation

Claude Code supports custom model display names, but the desktop configuration
surface does not expose them. Claude Code also supports a 1M context variant
through an exact `[1m]` model-selection suffix, while the canonical model
configuration currently has no way to represent that choice without treating
the rendered suffix as part of the upstream catalog identity.

New installations also require users to enter every Agent-native field even
when a deployment has one known recommended configuration. A deployment-owned
preset can simplify first configuration, but it must not weaken authenticated
catalog validation or masquerade as existing user-owned configuration.

## Goals

- Expose optional Claude display names for the primary model and every explicit
  Haiku, Sonnet, or Opus selection in Shell, PowerShell, and desktop flows.
- Represent Claude Code 1M context mode independently for every explicit Claude
  selection.
- Keep the authenticated upstream base model ID separate from Claude Code's
  rendered `[1m]` selection value.
- Let release builds inject one key-free canonical preset into every manager
  binary.
- Validate preset structure at manager startup and validate each requested
  Agent section against the current authenticated catalog during discovery.
- Initialize every selected Agent independently using
  `existing > preset > empty` while keeping all values editable.
- Preserve the existing fail-closed and zero-secret-persistence contracts.

## Non-Goals

- Inferring 1M support from a model ID or catalog position.
- Adding context capability metadata to `GET /v1/models`.
- Writing a numeric Claude context-window value.
- Managing `CLAUDE_CODE_DISABLE_1M_CONTEXT`.
- Adding a static or cached model catalog.
- Choosing the first discovered model or matching models by name.
- Partially repairing a preset or substituting an unavailable model.
- Fetching presets from a remote service at runtime.
- Persisting a preset before the user approves and writes a resulting canonical
  configuration.
- Adding a per-model display name to Codex.
- Changing router lifecycle, inference routing, or Agent file transaction
  semantics.

## Architecture

### Authoritative manager

The Go manager remains the sole authority for canonical structure, catalog
membership, rendering, preview, and write validation. Shell, PowerShell, Rust,
and TypeScript mirror required types and UX behavior but cannot authorize a
model or preset independently.

### Catalog identity and Claude value

Canonical configuration and all catalog/revision checks use the base model ID:

```text
claude-sonnet-4-6
```

The Claude renderer appends `[1m]` only at the Agent configuration boundary:

```text
claude-sonnet-4-6[1m]
```

The suffix is not persisted as the canonical model identity and is not added to
the signed candidate catalog.

### Preset lifecycle

One complete key-free canonical preset is Base64-encoded and injected into the
manager at build time. The manager:

1. Strictly decodes and structurally validates it before serving requests.
2. Crops it to the Agents requested by `agent.models`.
3. Validates each requested Agent section independently against the current
   authenticated catalog.
4. Returns valid sections separately from existing configuration.
5. Reports unavailable preset models as nonfatal discovery metadata.

Clients use the result only as editable form initialization. Normal render,
preview, approval, refreshed-catalog validation, and transactional write still
apply.

## Functional Requirements

### FR-1: Canonical Claude context field

The canonical Claude model-selection object must accept an optional `context`
field:

```json
{
  "model": "claude-sonnet-4-6",
  "name": "Sonnet 4.6 1M",
  "context": "1m"
}
```

The exact rules are:

- Omission means standard/default context behavior.
- The only accepted v1 value is the exact string `"1m"`.
- Numeric values, booleans, empty strings, and other strings are invalid.
- `model` remains required and must match the authenticated base catalog ID.
- A canonical Claude `model` ending in `[1m]` is invalid even if that exact
  string appears in the catalog.
- An inherited role contains only `{"inherit_primary":true}`.
- An explicit role may define its own `model`, optional `name`, and optional
  `context`.
- Canonical schema version remains `1`; documents without `context` retain
  their previous behavior.

The Go JSON Schema, Rust types, and TypeScript types must encode the same field
and enum.

### FR-2: Claude rendering

When a selection omits `context`, the manager must render its base model ID
unchanged. When `context` is `"1m"`, the manager must append exactly one
terminal `[1m]` to each model environment value produced from that selection.

The affected managed variables are:

- `ANTHROPIC_MODEL`
- `ANTHROPIC_CUSTOM_MODEL_OPTION`
- `ANTHROPIC_DEFAULT_HAIKU_MODEL`
- `ANTHROPIC_DEFAULT_SONNET_MODEL`
- `ANTHROPIC_DEFAULT_OPUS_MODEL`

The manager must not alter display-name values or write
`CLAUDE_CODE_DISABLE_1M_CONTEXT`.

An inherited role must inherit the primary model, display name, and context
mode together. An explicit role must render its own complete selection.

### FR-3: Existing Claude configuration projection

When inspecting existing Claude Code settings, the manager must recognize one
exact, case-sensitive terminal `[1m]` suffix and project it to:

```json
{"model":"<base-id>","context":"1m"}
```

Projection must not repair:

- an empty base ID
- repeated suffixes such as `model[1m][1m]`
- a marker in the middle of an ID
- alternate capitalization or spelling

Catalog availability and unavailable-model reporting must use the base ID.

A role may project as `inherit_primary` only when model ID, optional display
name, and context mode all equal the primary selection. Equal base IDs with a
different name or context remain explicit roles.

### FR-4: Claude display-name client support

Shell, PowerShell, and desktop flows must let users set or clear the optional
display name for the primary selection and every explicit role selection.

The desktop must preserve imported and existing display names, expose editable
name controls, and clear stale name/context metadata when the user changes a
selection's model. Enabling inheritance removes the entire explicit selection;
disabling inheritance creates an incomplete explicit selection that must be
completed before preview.

### FR-5: Build-time preset input

The manager linker symbol must be:

```text
github.com/codeasier/mtls-router/internal/manager/preset.Encoded
```

Build tooling reads the optional environment variable:

```text
AGENT_MODEL_PRESET_BASE64
```

The decoded document must:

- be no larger than the canonical model-config limit
- be strict UTF-8 JSON with no duplicate keys or trailing data
- have `version: 1`
- contain at least one Claude, opencode, or Codex section
- satisfy the same structural, type, bound, extension, and protected-key rules
  as ordinary canonical model configuration
- contain no key, URL, provider identity, header, fetched catalog, or arbitrary
  Agent configuration

Unset or empty input means no preset and is valid. Invalid Base64, oversized
input, invalid JSON, an empty version-only document, or invalid canonical
structure must fail manager startup before protocol serving. Errors and logs
must not expose encoded or decoded preset content.

Standalone release managers and desktop-packaged manager sidecars must receive
the same injected value. The router binary must not receive it.

### FR-6: Per-Agent preset catalog validation

After authenticated catalog discovery, the manager must assess only requested
Agent sections. Each Agent section is atomic:

- If every referenced model exists, return the complete validated section.
- If any referenced model is missing, omit the complete section and report the
  sorted unique missing base IDs for that Agent.
- A mismatch in one Agent section must not block valid sections for other
  requested Agents.
- The manager must not remove individual models, change defaults, repair role
  relationships, infer a replacement, or select another catalog model.
- Sections for unrequested Agents must not be returned or reported.

Preset processing must not read or mutate Agent files, create transaction
state, or alter drift/ownership results.

### FR-7: `agent.models` preset result

Protocol v2 `agent.models` must add this stable result object:

```json
{
  "preset": {
    "model_config": {},
    "unavailable_agents": {}
  }
}
```

When populated, `model_config` is a versioned canonical document containing
only valid requested Agent sections. Each unavailable entry has this shape:

```json
{
  "codex": {
    "code": "MODEL_NOT_AVAILABLE",
    "models": ["missing-base-id"]
  }
}
```

`model_config` and `unavailable_agents` must be objects, never `null` or omitted.
Preset unavailability is metadata, not an `agent.models` failure. Existing
catalog, catalog token, URL, and existing-config results remain usable.

The preset result must contain no API key, raw upstream response, raw Agent
file, router credential, or unvalidated extension.

### FR-8: Client initialization precedence

After discovery, Shell, PowerShell, and desktop clients must initialize each
selected Agent independently:

```text
valid existing section > valid preset section > empty section
```

The overall user-input precedence is:

```text
explicit --model-config or desktop import
    > interactive user edits
    > per-Agent existing section
    > per-Agent preset section
    > empty section
```

Clients must not deep-merge existing and preset data inside one Agent section.
They must expose a non-sensitive source summary and unavailable preset model IDs
so users can distinguish existing, recommended, and empty initialization.

A preset is an editable default, not preview approval or write confirmation.

### FR-9: Shell and PowerShell behavior

Setup scripts must retain explicit Agent selection and hidden API-key input
before model discovery. Their Agent-native prompts must use existing/preset
values as defaults, with an empty response accepting the displayed default.

Claude prompts must cover model ID, optional display name, and
`standard`/`1m` context for the primary and every explicit role. `standard`
omits canonical `context`; `1m` emits `"context":"1m"`. Any other context value
must fail before a manager configuration request.

`--model-config=PATH` must continue to bypass interactive field defaults while
still requiring authenticated discovery and manager validation.

Shell and PowerShell must produce equivalent canonical requests, preserve key
secrecy, and retain PowerShell 5.1 compatibility and the `setup.ps1` UTF-8 BOM.

### FR-10: Desktop behavior

The desktop configure stage must add:

- one optional display-name input for the Claude primary selection
- one Standard/1M selector for the primary selection
- equivalent controls for every non-inherited role selection
- a notice identifying Agent sections initialized from the preset
- a nonblocking notice identifying unavailable preset sections and missing IDs

The desktop must apply per-Agent precedence rather than choosing one whole
existing, preset, or empty document. Preset data must not enter the Rust
secret-bearing flow state. Import continues to replace the current form.

The controls must preserve native accessibility, existing responsive behavior,
and the Agent page's established visual language.

### FR-11: Ownership, state, and security

The injected preset is immutable build metadata. It must not be written to the
last-applied sidecar, journal, backup, revision claims, logs, diagnostics, or an
Agent file merely because discovery returned it.

Only the exact canonical configuration approved through normal preview/write
may enter existing sidecar and transaction flows. The new Claude `context`
field becomes part of that approved canonical section, while managed Claude env
paths remain unchanged.

API-key handling remains unchanged: the key is accepted only in transient
`agent.models` and `agent.write` request bodies and final Agent authentication
configuration. It must not enter the preset, model config, catalog/revision
tokens, command-line arguments, environment variables, logs, or diagnostics.

### FR-12: Write-time validation and failures

Preview and write must continue to validate canonical base IDs. Immediately
before a write, the manager must refresh the authenticated catalog and reject a
base model that disappeared with `MODEL_NOT_AVAILABLE` before creating any
backup, journal, temporary file, directory, sidecar change, or Agent-file
change.

Configuration-time 1M capability is not inferred. If Claude Code or the
upstream rejects a validly configured 1M request, that remains a runtime error
and must not cause configuration fallback or rewriting.

## Observable Scenarios

### Scenario 1: Claude primary uses 1M context

Given the authenticated catalog contains `claude-sonnet-4-6`, when the user
selects that base model with `context: "1m"`, then canonical validation succeeds
and Claude settings render `claude-sonnet-4-6[1m]` without changing its display
name or catalog identity.

### Scenario 2: Mixed Claude role contexts

Given a 1M primary, an inherited Sonnet role, and a standard explicit Haiku role,
then Sonnet renders the primary `[1m]` value while Haiku renders its unsuffixed
explicit model.

### Scenario 3: Existing 1M settings are recovered

Given existing Claude settings contain a single terminal `[1m]`, when discovery
inspects them, then the safe prefill contains the base model plus
`context: "1m"`. A same-base standard-context role remains explicit.

### Scenario 4: Valid preset fills a new Agent

Given no existing opencode section and a preset opencode section whose models
all exist in the authenticated catalog, when discovery completes, then the
client initializes opencode from that preset and permits edits before preview.

### Scenario 5: Existing and preset sections are mixed

Given existing Claude configuration, valid preset opencode configuration, and
an unavailable Codex preset, when all three Agents are selected, then Claude
uses existing, opencode uses preset, and Codex starts empty with an unavailable
notice.

### Scenario 6: One preset section is unavailable

Given the preset Codex model is absent while Claude preset models are present,
then `agent.models` returns Claude's complete preset section, omits Codex, and
reports Codex's missing base ID without failing discovery or selecting a
replacement.

### Scenario 7: Broken injected preset

Given the manager contains malformed Base64 or invalid canonical preset JSON,
when it starts, then startup fails before reading protocol requests and no raw
preset content appears in stderr.

### Scenario 8: No preset is injected

Given an empty linker value, when discovery completes, then the stable preset
result contains empty objects and clients retain existing manual behavior.

### Scenario 9: Model disappears before write

Given a preset-backed selection passed preview but its base model disappears
from the refreshed catalog, when write is requested, then the manager returns
`MODEL_NOT_AVAILABLE` with zero write artifacts.

### Scenario 10: Explicit import wins

Given existing and preset defaults are available, when a user supplies
`--model-config` or imports a desktop canonical document, then that explicit
document replaces generated defaults and proceeds through normal manager
validation.

## Impact

Expected implementation impact includes:

- canonical Go model types, validation, schema, renderer, and existing projection
- a focused internal manager preset loader
- Agent service discovery and protocol result types
- manager startup and release linker metadata
- Rust canonical validation and desktop IPC bridge
- TypeScript types, Agent page controls, localization, and tests
- Shell and PowerShell interactive builders and integration tests
- standalone, desktop-sidecar, and GitHub Actions manager builds
- English and Chinese Agent, desktop, build, troubleshooting, README, and
  changelog documentation
- pinned Claude Code compatibility evidence

No new runtime dependency, external network call, router endpoint, Agent target
path, or transaction file is required.

## Verification Strategy

Automated evidence must cover:

- canonical context acceptance/rejection, JCS round-trip, and schema parity
- standard/1M Claude rendering and exact existing projection
- inheritance comparison across model, name, and context
- absent, valid, malformed, oversized, protected, and invalid-Base64 presets
- per-Agent crop, catalog mismatch, stable empty objects, and no secret leakage
- protocol/app/subprocess result shape and startup failure
- Rust/TypeScript strict types and cross-language canonical vectors
- desktop per-Agent precedence, metadata clearing, controls, and notices
- Shell/PowerShell defaults, overrides, imports, Unicode names, context, and key
  secrecy
- identical manager linker injection in local, standalone release, and desktop
  sidecar builds
- write-time refreshed base-ID validation with zero artifacts on failure
- English/Chinese documentation parity and pinned Claude compatibility evidence

Required integrated commands are:

```bash
test -z "$(gofmt -l .)"
go test ./...
go vet ./...
make test-shell
make test-workflows
make desktop-verify
```

Any unavailable platform-specific verification must be recorded rather than
marked complete without evidence.

## Acceptance Boundary

The change is complete only when every applicable item in `checklist.md` has
automated or recorded evidence. A visible preset or 1M control is insufficient
if catalog identity, write-time refresh, key secrecy, sidecar isolation,
cross-client precedence, or release-build parity is not proven.
