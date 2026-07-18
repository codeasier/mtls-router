# Agent Presets and Claude Context Configuration Design

**Date:** 2026-07-19

## Summary

Extend Agent configuration management in two related areas:

1. Complete Claude Code display-name configuration in the desktop client and
   add explicit per-selection support for Claude Code's 1M context variant.
2. Let release builds inject one key-free canonical Agent preset that the
   manager validates against each authenticated model catalog and uses as a
   modifiable initial configuration.

The manager remains authoritative. Presets never bypass authenticated model
discovery, preview, write approval, write-time catalog refresh, ownership
checks, or transactional writes. Existing managed configuration remains more
important than a preset.

## Goals

- Configure an optional Claude Code display name for the primary model and
  every explicit Haiku, Sonnet, or Opus selection in every supported client.
- Configure Claude Code's 1M context variant independently for the primary
  model and every explicit role selection.
- Inject one complete canonical preset into release manager binaries.
- Apply preset sections only after validating them against the current
  authenticated catalog.
- Initialize each selected Agent independently using
  `existing > preset > empty`.
- Preserve the current fail-closed model and key-handling guarantees.

## Non-Goals

- Do not infer 1M support from a model name.
- Do not add a static model catalog or choose the first discovered model.
- Do not substitute another model when a preset model is unavailable.
- Do not fetch presets from a remote service at runtime.
- Do not write Agent files during installation, startup, or discovery.
- Do not store presets in the last-applied ownership sidecar.
- Do not manage `CLAUDE_CODE_DISABLE_1M_CONTEXT`; it is a user policy setting,
  not a per-model context selection.
- Do not add per-model display names to Codex, whose current native config has
  no corresponding setting.

## Existing Behavior

The manager obtains the complete candidate catalog from authenticated
`GET /v1/models` and retains only `data[].id`. Canonical model selections must
match a signed catalog exactly. Shell, PowerShell, and desktop clients build or
import a key-free canonical configuration, then call manager render, preview,
and write methods.

Claude canonical selections already support an optional `name`. The renderer
writes the corresponding `ANTHROPIC_*_MODEL_NAME` variables, and setup scripts
already prompt for names. The desktop types preserve names, but its Claude UI
does not expose them and replaces a selection with a model-only object when the
model changes.

Claude canonical selections do not represent context mode. Claude Code enables
an extended context variant by appending `[1m]` to a selected model, rather than
through a numeric context-window setting. Treating that rendered value as the
catalog identity would violate the current exact catalog-membership rule when
the service catalog contains only the base model ID.

Discovery can return safe existing canonical selections recovered from the
last-applied sidecar or projected from current Agent files. That result has
ownership and drift semantics and must not be reused to represent a product
preset.

## Decisions

- A preset is injected into the manager at build time as Base64-encoded,
  key-free canonical JSON.
- One injected document may contain Claude, opencode, and Codex sections.
- The manager crops the document to the Agents selected by each request.
- Preset validity is assessed independently for each selected Agent.
- If any model in one Agent section is absent from the authenticated catalog,
  that entire section is unavailable. Other valid Agent sections remain usable.
- Clients initialize each Agent independently using
  `existing > preset > empty`.
- A valid preset is filled by default and remains editable.
- An explicit `--model-config` or desktop import replaces generated form
  defaults and remains the highest-precedence configuration input.
- Claude 1M mode belongs to each explicit selection. An inherited role inherits
  the primary model, display name, and context mode together.

## Canonical Model Changes

### Claude Selection

Add an optional `context` field to the Claude selection shape:

```json
{
  "model": "claude-sonnet-4-6",
  "name": "Sonnet 4.6 1M",
  "context": "1m"
}
```

The field has these rules:

- It may be omitted, which selects the normal model context behavior.
- The only accepted v1 value is the exact string `"1m"`.
- Numeric windows, booleans, and other context labels are invalid.
- The `model` field remains the base catalog identity and must match the
  authenticated catalog exactly.
- A canonical `model` value ending in `[1m]` is rejected. This prevents an
  ambiguous identity and duplicate suffix rendering.
- An inherited role cannot carry `model`, `name`, or `context` fields.
- An explicit role may configure its own display name and context mode.

The canonical document remains `version: 1`. The new field is optional, so all
existing v1 documents retain their behavior. The checked-in JSON Schema and the
Go, Rust, and TypeScript representations must be updated together.

Example:

```json
{
  "version": 1,
  "claude": {
    "primary": {
      "model": "claude-sonnet-4-6",
      "name": "Sonnet 4.6",
      "context": "1m"
    },
    "haiku": {
      "model": "claude-haiku-4-5",
      "name": "Haiku 4.5"
    },
    "sonnet": {"inherit_primary": true},
    "opus": {
      "model": "claude-opus-4-8",
      "name": "Opus 4.8",
      "context": "1m"
    }
  }
}
```

## Claude Rendering and Projection

Rendering maintains separate catalog and client values:

```text
catalog identity: claude-sonnet-4-6
rendered value:   claude-sonnet-4-6[1m]
```

When `context` is omitted, render the base ID unchanged. When it is `"1m"`,
append one exact `[1m]` suffix to all model variables associated with that
selection:

- `ANTHROPIC_MODEL`
- `ANTHROPIC_CUSTOM_MODEL_OPTION`
- `ANTHROPIC_DEFAULT_HAIKU_MODEL`
- `ANTHROPIC_DEFAULT_SONNET_MODEL`
- `ANTHROPIC_DEFAULT_OPUS_MODEL`

Display names continue to render through the existing custom and role-specific
`*_MODEL_NAME` variables. The manager does not append a context marker to the
display name; the user or preset supplies the desired label.

Existing Claude configuration projection recognizes only one exact,
case-sensitive terminal `[1m]` suffix. It removes that suffix and records
`context: "1m"`. It does not repair empty base IDs, repeated suffixes, or a
marker occurring in the middle of an ID.

A role projects as `inherit_primary` only when its effective model ID, display
name, and context mode are equivalent to the primary selection. This avoids
losing distinctions such as a standard-context role using the same base model
as a 1M primary, or a role-specific display name.

Write-time refreshed-catalog validation continues to use only base catalog IDs.
The existing zero-write `MODEL_NOT_AVAILABLE` behavior remains unchanged.

## Preset Build Input

Add `internal/manager/preset` with this link-time string variable:

```go
package preset

var Encoded string
```

Release builds inject a Base64 value:

```text
-X github.com/codeasier/mtls-router/internal/manager/preset.Encoded=<base64>
```

The release environment variable is `AGENT_MODEL_PRESET_BASE64`. Base64 avoids
JSON quoting, newline, and cross-shell differences in `-ldflags -X`. Every
released manager variant, including desktop sidecars, must receive the same
value. Local builds may omit it and then expose no preset.

The decoded value is limited to the canonical model config size limit. It must
be strict key-free JSON using the supported canonical schema. The manager must
not log the encoded or decoded value.

Build-input behavior:

| Input | Manager behavior |
|---|---|
| Not injected or empty | Start normally with no preset |
| Invalid Base64 | Fail manager startup |
| Decoded value too large | Fail manager startup |
| Invalid JSON or unsupported structure | Fail manager startup |
| Protected credential, URL, header, or provider path | Fail manager startup |
| Structurally valid model absent from a user's catalog | Start normally; mark that Agent preset section unavailable for that discovery |

This separates a broken release artifact from an authorization-specific model
catalog mismatch.

## Preset Validation

Preset validation has two stages:

1. Startup validates the complete decoded document without requiring a live
   catalog. It enforces strict JSON, canonical schema types and bounds, Agent
   section structure, protected extension keys, and base model-ID syntax.
2. `agent.models` crops the preset to requested Agents and validates each Agent
   section independently against the authenticated catalog.

The model-config package exposes one validation implementation with explicit
options for required Agents and catalog-membership enforcement, rather than
duplicating schema rules in the preset package. Startup calls it with every
section present in the preset and catalog membership disabled. Discovery calls
it once per requested Agent with membership enabled. Existing render, preview,
and write calls continue to require membership.

The injected document must have `version: 1` and at least one Agent section.
Unknown sections and an empty version-only document are invalid build inputs.

For each requested Agent during discovery:

1. If the preset has no section, return no preset for that Agent.
2. Wrap the section in a temporary versioned single-Agent document.
3. Validate it against the current catalog.
4. If valid, add it to the returned preset configuration.
5. If one or more selected models are absent, omit the entire section and
   report its unavailable model IDs.
6. Do not select a replacement, partially remove models, or repair defaults.

## Protocol Response

Extend `agent.models` with a separate stable `preset` object:

```json
{
  "models": ["claude-sonnet-4-6", "gpt-5.5"],
  "catalog_token": "...",
  "router_base_url": "http://127.0.0.1:19099",
  "api_base_url": "http://127.0.0.1:19099/v1",
  "existing": {
    "model_config": {},
    "unavailable_models": {},
    "drifted_agents": []
  },
  "preset": {
    "model_config": {
      "version": 1,
      "claude": {
        "primary": {
          "model": "claude-sonnet-4-6",
          "name": "Sonnet 4.6",
          "context": "1m"
        },
        "haiku": {"inherit_primary": true},
        "sonnet": {"inherit_primary": true},
        "opus": {"inherit_primary": true}
      }
    },
    "unavailable_agents": {
      "codex": {
        "code": "MODEL_NOT_AVAILABLE",
        "models": ["gpt-5.5-codex"]
      }
    }
  }
}
```

Suggested protocol types:

```go
type AgentModelsPresetUnavailable struct {
    Code   ErrorCode `json:"code"`
    Models []string  `json:"models,omitempty"`
}

type AgentModelsPreset struct {
    ModelConfig       json.RawMessage                          `json:"model_config"`
    UnavailableAgents map[string]AgentModelsPresetUnavailable `json:"unavailable_agents"`
}
```

`preset.model_config` is `{}` when no section is usable.
`preset.unavailable_agents` is `{}` when there are no catalog mismatches. Avoid
`null` so every client receives one stable shape.

Preset unavailability is discovery metadata, not a protocol failure. The
existing catalog, token, and existing-config results remain usable.

## Client Initialization and Precedence

After discovery, each client builds its initial form one Agent at a time:

```text
for every selected Agent:
    if a valid existing section is present:
        use existing
    else if a valid preset section is present:
        use preset
    else:
        use that Agent's empty form section
```

For example, if Claude has existing configuration, opencode has a valid preset,
and the Codex preset is unavailable, selecting all three produces:

```text
Claude   -> existing
opencode -> preset
Codex    -> empty
```

Overall input precedence is:

```text
explicit --model-config or desktop import
    > interactive user edits
    > per-Agent existing section
    > per-Agent preset section
    > empty section
```

The client exposes non-sensitive source information so the user can tell
which sections came from existing configuration or the recommended preset and
which presets were unavailable. A preset does not imply write approval.

## Shell and PowerShell UX

The setup scripts continue to require explicit Agent selection and hidden API
key input before discovery. Their interactive builders accept a per-Agent
initial section and use its values as prompt defaults. Pressing Enter accepts a
default; entering a value replaces it.

Claude prompts include:

```text
Claude primary model ID [claude-sonnet-4-6]:
Claude primary name [Sonnet 4.6]:
Claude primary context [standard/1m, default 1m]:
```

Explicit roles receive the same model, display-name, and context prompts.
Inherited roles need no independent metadata prompts.

The scripts print a short source summary before prompts. They do not print the
API key or any rendered unredacted config. An unavailable preset section falls
back to empty prompts only for that Agent.

`--model-config=PATH` continues to skip form construction and sends the imported
canonical document through normal manager validation.

## Desktop UX

The desktop configure stage remains one page. It gains:

- An optional display-name input for the Claude primary selection.
- A Standard/1M context selector for the Claude primary selection.
- The same controls for every non-inherited Haiku, Sonnet, or Opus selection.
- A concise notice listing sections initialized from the preset.
- A non-blocking notice for unavailable preset sections and their missing
  model IDs.

Changing a Claude model clears that selection's display name and context mode,
preventing metadata from silently moving to another model. Enabling inheritance
removes the complete explicit selection. Disabling inheritance creates one
empty explicit selection that must be completed before preview.

The desktop applies the same per-Agent initialization order as setup scripts.
Importing a canonical file replaces the current form as it does today.

## Ownership and Persistence

Presets are read-only build metadata. They are not persisted separately and do
not create ownership state. Only the canonical configuration actually approved
and written by the user enters the last-applied sidecar.

The new Claude `context` value is part of the canonical sidecar section. The
existing manager-owned Claude model environment paths remain unchanged because
1M is encoded in their rendered values. Preview and drift detection therefore
continue to operate through the existing ownership namespaces.

Discovery remains key-free after its authenticated catalog fetch result is
constructed. No preset contains or receives the API key, router URL, provider
identity, headers, or fetched catalog.

## Error Handling

| Condition | Result |
|---|---|
| No preset injected | Empty preset result; normal manual flow |
| Broken injected preset | Manager startup failure |
| One Agent preset references unavailable models | Omit that section and report `MODEL_NOT_AVAILABLE` metadata |
| Other Agent preset sections are valid | Return and use them normally |
| User puts `[1m]` in canonical `model` | `MODEL_CONFIG_INVALID` |
| Existing terminal `[1m]` has an available base model | Project to `context: "1m"` |
| Existing terminal `[1m]` has an unavailable base model | Report the base ID as the existing unavailable model |
| Upstream rejects a configured 1M request | Runtime Agent/upstream error; no configuration-time capability inference |
| Selected base model disappears before write | Existing zero-write `MODEL_NOT_AVAILABLE` behavior |

Error messages and protocol details must not contain credentials or raw preset
documents.

## Compatibility

- Existing canonical v1 documents without `context` are unchanged.
- Older managers continue to reject the new optional field through strict v1
  decoding. Manager and repository-managed clients are therefore released
  together; no client sends `context` to a manager that did not advertise the
  current packaged protocol implementation.
- Existing Claude display-name environment variables remain supported.
- Existing Agent ownership namespaces and file formats do not change.
- A manager with no injected preset behaves like the current implementation.
- `agent.models` gains one response field. Repository-managed Shell,
  PowerShell, TypeScript, and Rust consumers must be updated atomically.
- Protocol request strictness and protocol version 2 remain unchanged unless
  compatibility tests show an external response consumer requires a version
  bump.
- The pinned Claude Code compatibility evidence must be refreshed or extended
  to cover `[1m]` model selection and custom model-name variables.
- English and Chinese Agent, desktop, setup, troubleshooting, build, and release
  documentation must remain aligned where affected.

## Testing

### Go Model Configuration

- Accept omitted context and exact `"1m"`.
- Reject all other context values and canonical model IDs ending in `[1m]`.
- Preserve context through canonical JCS serialization and token claims.
- Keep old v1 fixtures valid.
- Regenerate and verify the checked-in schema.

### Go Claude Rendering and Existing Projection

- Render standard and 1M values for primary and every explicit role.
- Inherit model, display name, and context together.
- Never append `[1m]` twice.
- Project one exact terminal suffix to `context: "1m"`.
- Do not collapse a role to inheritance when model, name, or context differs.
- Preserve existing display names through projection and rendering.

### Go Preset and Protocol

- Cover absent, valid, malformed, oversized, and invalid-Base64 build inputs.
- Reject protected credential and connection fields.
- Crop one complete preset to selected Agents.
- Validate sections independently against the authenticated catalog.
- Omit an entire mismatched section without affecting valid sections.
- Return stable non-null preset fields.
- Keep catalog tokens and write-time refresh bound to base IDs.
- Verify no key or raw sensitive input appears in results, errors, or filesystem
  artifacts.

### Shell and PowerShell

- Use valid presets as editable prompt defaults.
- Prefer existing values over presets per Agent.
- Fall back to empty prompts only for unavailable sections.
- Let user input override defaults.
- Let `--model-config` override generated defaults.
- Round-trip Unicode display names and Claude `context: "1m"`.
- Preserve hidden-key and no-leakage guarantees.

### Desktop and Rust

- Decode and forward the new protocol shape.
- Merge `existing > preset > empty` independently per Agent.
- Edit and round-trip Claude display names and context modes.
- Clear selection metadata when changing models.
- Preserve complete inheritance behavior.
- Show preset source and unavailable notices.
- Preserve import, preview, revision-token, approval, write, and flow-destruction
  behavior.

### Release and Compatibility

- Build managers with and without `AGENT_MODEL_PRESET_BASE64`.
- Verify standalone and desktop-packaged managers receive the same preset.
- Verify invalid release input fails preflight or manager startup.
- Validate the pinned Claude Code package accepts custom names and `[1m]`
  selections.
- Run Go tests, vet, gofmt checks, shell tests, desktop tests, Rust tests, and
  release preflight relevant to modified paths.

## Acceptance Criteria

1. A user can set and preserve a Claude display name in setup scripts and the
   desktop for the primary and every explicit role selection.
2. A user can select standard or 1M context independently for the primary and
   every explicit role; inherited roles inherit the primary context.
3. The manager renders `[1m]` only at the Claude Code configuration boundary and
   validates the base model against authenticated discovery and write-time
   refresh.
4. A release can inject one complete key-free preset without embedding it in
   setup scripts or desktop code.
5. Discovery returns only preset sections valid for the selected Agents and the
   current authenticated catalog.
6. Clients initialize each Agent using `existing > preset > empty` and let the
   user modify all defaults before preview or write.
7. One unavailable preset section neither substitutes models nor blocks valid
   sections for other Agents.
8. Missing presets preserve current behavior; broken injected presets fail
   closed.
9. Presets never create files, sidecar state, drift, or write approval by
   themselves.
10. Existing key secrecy, exact preview, refreshed-catalog validation,
    ownership, backup, rollback, and atomic-write contracts remain intact.
