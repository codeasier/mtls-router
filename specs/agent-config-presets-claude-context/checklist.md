# Agent Configuration Presets and Claude Context Acceptance Checklist

Check an item only after automated evidence or recorded manual verification
demonstrates the behavior. Unaffected requirements in
`specs/agent-models-config/checklist.md` continue to apply.

## Scope and Compatibility

- [x] The implementation matches `spec.md` without unrelated router, Agent
  target-path, transaction-format, or desktop workflow redesign.
- [x] Existing canonical v1 documents without `context` retain their behavior.
- [x] Manager and repository-managed clients ship compatible protocol/result
  types together.
- [x] No first-model, model-name heuristic, capability inference, cached catalog,
  partial repair, or model substitution is introduced.
- [x] No new runtime network call or dependency is introduced for presets.

## Canonical Claude Context

- [x] Claude selection accepts omitted `context`.
- [x] Claude selection accepts only exact `context: "1m"` when present.
- [x] Numeric, boolean, empty, and unsupported context values are rejected.
- [x] Canonical Claude model IDs ending in `[1m]` are rejected even if present
  in the catalog.
- [x] Explicit roles support independent model, name, and context.
- [x] Inherited roles contain no explicit selection metadata.
- [x] Go JSON Schema, Rust types, TypeScript types, and canonical vectors agree.
- [x] RFC 8785 canonical bytes and revision claims include configured context.

## Claude Rendering and Existing Projection

- [x] Standard selections render unsuffixed base model IDs.
- [x] 1M selections render exactly one terminal `[1m]`.
- [x] Primary, custom option, Haiku, Sonnet, and Opus model variables use the
  correct effective selection.
- [x] Display names render unchanged and only when configured.
- [x] `CLAUDE_CODE_DISABLE_1M_CONTEXT` is not manager-owned or written.
- [x] Existing one-terminal-suffix settings project to base ID plus
  `context: "1m"`.
- [x] Empty-base, repeated-suffix, middle-marker, and alternate-case values are
  not silently repaired.
- [x] Existing catalog availability and unavailable reporting use base IDs.
- [x] A role projects as inherited only when model, name, and context all equal
  the primary selection.

## Preset Build Input

- [x] The linker symbol is exactly
  `github.com/codeasier/mtls-router/internal/manager/preset.Encoded`.
- [x] Empty or unset `AGENT_MODEL_PRESET_BASE64` starts normally with no preset.
- [x] Valid standard Base64 decodes to a strict key-free canonical document.
- [x] Decoded size is bounded by the canonical config limit.
- [x] Invalid Base64, JSON, UTF-8, duplicate keys, trailing data, version,
  structure, or protected fields fail manager startup.
- [x] A version-only preset with no Agent section fails startup.
- [x] Startup fails before protocol serving or transaction recovery.
- [x] Preset errors/logs do not expose encoded or decoded content.
- [x] Standalone and desktop manager builds receive the same value.
- [x] The router binary does not receive the preset linker value.

## Preset Discovery

- [x] Only requested Agent sections are assessed and returned.
- [x] Each Agent section is validated independently against the current
  authenticated catalog.
- [x] A fully valid section is returned without modification.
- [x] Any missing model omits that Agent's entire section.
- [x] Missing IDs are sorted, unique base IDs.
- [x] One unavailable Agent does not block another valid Agent section.
- [x] No unavailable section is partially filtered, repaired, deep-merged, or
  substituted.
- [x] Preset processing reads or writes no Agent file or transaction state.
- [x] No-preset and no-valid-section results use `{}`, not null or omission.

## Protocol and Security

- [x] `agent.models` returns stable `preset.model_config` and
  `preset.unavailable_agents` objects.
- [x] Unavailable entries use `MODEL_NOT_AVAILABLE` and bounded model arrays.
- [x] Preset unavailability is metadata and does not fail otherwise valid
  discovery.
- [x] Preset results contain no API key, raw upstream response, raw Agent file,
  router credential, or unvalidated extension.
- [x] Preset data is not copied into catalog or revision tokens.
- [x] Rust result decoding rejects malformed, unknown, unbounded, or invalid
  preset result shapes.
- [x] Preset data is not stored in Rust secret-bearing `ModelFlow`.

## Client Precedence

- [x] Shell initializes each selected Agent with `existing > preset > empty`.
- [ ] PowerShell initializes each selected Agent with the same precedence.
- [x] Desktop initializes each selected Agent with the same precedence.
- [x] Clients do not deep-merge existing and preset within one Agent section.
- [x] Mixed existing, preset, and empty sections work in one flow.
- [x] Clients visibly identify existing/preset sources without sensitive data.
- [x] Clients show unavailable Agent/model information without blocking valid
  sections.
- [x] Preset values remain editable before preview and write.
- [x] `--model-config` and desktop import replace generated defaults.
- [x] A preset never implies preview approval, drift approval, auth approval, or
  write confirmation.

## Shell and PowerShell

- [x] Claude primary prompts support model ID, optional name, and
  standard/1M context.
- [x] Every explicit role supports the same fields.
- [x] Inherited roles emit only `inherit_primary`.
- [x] Empty interactive input accepts a displayed existing/preset default.
- [x] User input can replace every default.
- [x] `standard` omits canonical context and `1m` emits exact context.
- [x] Unsupported context input fails before manager configuration calls.
- [x] Unicode model names survive canonical requests.
- [x] Key input remains hidden and absent from flags, environment, model config,
  output, logs, and temporary files.
- [ ] Shell and PowerShell produce equivalent canonical protocol requests.
- [ ] `setup.ps1` remains PowerShell 5.1 compatible and retains its UTF-8 BOM.

## Desktop

- [x] Primary Claude selection exposes optional display-name and Standard/1M
  controls.
- [x] Every explicit Claude role exposes equivalent controls.
- [x] Imported and existing display names/context values are preserved.
- [x] Changing a model clears stale name and context metadata.
- [x] Enabling inheritance removes the complete explicit selection.
- [x] Disabling inheritance creates an incomplete explicit selection that must
  be completed before preview.
- [x] Preset-applied and preset-unavailable notices are localized in English and
  Chinese.
- [x] Controls preserve native accessibility, keyboard use, responsive layout,
  and established Agent-page styling.
- [x] Import, preview, revision, approvals, write, cancel, and flow destruction
  retain their prior behavior.

## Ownership, Write, and Failure Behavior

- [x] Discovery alone writes no Agent file, sidecar, journal, backup, temporary
  file, target directory, or ownership state.
- [x] The injected preset is absent from sidecars, journals, backups, revision
  claims, logs, and diagnostics.
- [x] Only user-approved canonical configuration enters normal sidecar and
  transaction flows.
- [x] Claude `context` is preserved in an approved canonical sidecar section.
- [x] Existing manager-owned Claude env paths remain unchanged.
- [x] Write refresh validates base model IDs against the current authenticated
  catalog.
- [x] A disappeared base model returns `MODEL_NOT_AVAILABLE` before every write
  artifact.
- [x] A runtime upstream rejection of 1M does not trigger fallback or config
  rewriting.
- [x] Existing drift, backup, rollback, atomicity, recovery, and approval
  guarantees remain intact.

## Build, Compatibility, and Documentation

- [x] Local manager builds work with empty and valid preset values.
- [x] Release workflow reads `vars.AGENT_MODEL_PRESET_BASE64`.
- [x] Release preflight rejects malformed configured input without printing it.
- [x] Standalone and desktop release managers have identical preset behavior.
- [x] Pinned Claude Code evidence verifies custom model names and `[1m]` syntax
  with reproducible source, revision, integrity, and digest.
- [x] English and Chinese README documents are aligned.
- [x] English and Chinese Agent model documents are aligned.
- [x] English and Chinese desktop, build, troubleshooting, and changelog
  documents are aligned.
- [x] Documentation clearly distinguishes validated presets from prohibited
  first-model/name-based automatic selection.

## Automated Verification

- [x] `test -z "$(gofmt -l .)"` passes.
- [x] `go test ./...` passes.
- [x] `go vet ./...` passes.
- [ ] `make test-shell` passes.
- [x] `make test-workflows` passes.
- [x] `make desktop-verify` passes.
- [x] Any unavailable platform-specific verification is recorded and remains
  unchecked rather than being claimed complete.

Recorded gap: `pwsh` is not installed on the verification host. The
PowerShell runtime flow is skipped, so PowerShell precedence, interactive
request parity, and PowerShell 5.1 execution remain unchecked above. Static
PowerShell checks, Go checks, and the UTF-8 BOM check pass.
