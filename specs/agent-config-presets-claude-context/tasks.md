# Agent Configuration Presets and Claude Context Tasks

Tasks are dependency-ordered. A task is complete only when its implementation,
focused tests, and stated verification pass. Execution must not begin until
this specification package is explicitly approved.

## Phase 1: Canonical Claude Contract

- [x] **1.1 Add canonical Claude context validation**
  - Add optional `context` to the Claude model-selection type.
  - Accept only exact `"1m"`; keep omission as standard/default behavior.
  - Reject canonical Claude model IDs ending in `[1m]` independently of catalog
    membership.
  - Add one shared structural-validation mode for build presets that enforces
    all ordinary schema rules without requiring a live catalog.
  - Require a structurally decoded preset to contain at least one Agent section.
  - Update generated JSON Schema and canonical/JCS fixtures.
  - Add valid, invalid enum/type, suffix, old-v1, structure-only, and schema
    parity tests.
  - Verification: `go test ./internal/manager/agent/modelconfig`.

- [x] **1.2 Render and recover Claude 1M selections**
  - Append one exact terminal `[1m]` only when rendering a Claude selection with
    `context: "1m"`.
  - Apply the selection to primary, custom option, and effective role model env
    values without modifying names.
  - Do not manage `CLAUDE_CODE_DISABLE_1M_CONTEXT`.
  - Parse one exact terminal suffix from existing settings into base ID plus
    context; reject empty/repeated/middle-marker repair.
  - Compare model, optional name, and context before projecting a role as
    inherited.
  - Report existing unavailable selections by base ID.
  - Add renderer, projection, inheritance, name, repeated-suffix, and
    unavailable-ID tests.
  - Verification: `go test ./internal/manager/agent -run 'Claude|DiscoverModels'`.

## Phase 2: Manager Preset and Protocol

- [x] **2.1 Implement the build-time preset loader**
  - Create `internal/manager/preset` with linker variable `Encoded`.
  - Strictly decode standard Base64 and enforce the canonical config size limit.
  - Use authoritative modelconfig structural validation rather than a second
    preset schema.
  - Return nil for empty input and sanitized startup errors for every malformed
    nonempty input.
  - Prove encoded/decoded canaries never appear in errors or logs.
  - Add absent, valid, malformed Base64, malformed JSON, oversized, version-only,
    unknown-field, protected-key, and no-leakage tests.
  - Verification: `go test ./internal/manager/preset`.

- [x] **2.2 Inject an immutable preset into the Agent service**
  - Add a typed preset option to `agent.NewService`.
  - Canonicalize and decode a private copy before storing it.
  - Keep preset data outside signer state, sidecars, revisions, journals,
    backups, and write approvals.
  - Add mutation-isolation and no-preset service tests.
  - Verification: `go test ./internal/manager/agent`.

- [x] **2.3 Return independently validated preset sections from discovery**
  - Add separate Agent service result types for preset config and unavailable
    Agents.
  - Crop the complete preset to selected Agents.
  - Gather bounded base IDs and validate each section independently against the
    authenticated catalog.
  - Omit an entire mismatched section, report sorted unique missing IDs, and
    preserve valid sections for other Agents.
  - Never partially repair, deep merge, substitute, or inspect Agent files while
    processing a preset.
  - Return canonical `{}` when no section is valid.
  - Add no-preset, selected-scope, mixed-validity, complete-section, ordering,
    and no-state-mutation tests.
  - Verification: `go test ./internal/manager/agent -run 'Preset|DiscoverModels'`.

- [x] **2.4 Extend protocol and manager app wiring**
  - Add strict stable `preset.model_config` and `preset.unavailable_agents`
    result types to protocol v2.
  - Keep both fields present as objects, never null.
  - Map service missing-model metadata to `MODEL_NOT_AVAILABLE` without turning
    discovery into a failure.
  - Load the linked preset during `app.New` before protocol serving or Agent
    transaction recovery.
  - Clear API-key request values under the existing discovery boundary and do
    not put preset data in tokens.
  - Add exact protocol shape, complete app result, malformed startup,
    subprocess, and secret-canary tests.
  - Verification:
    `go test ./internal/manager/protocol ./internal/manager/app ./cmd/mtls-router-manager`.

## Phase 3: Desktop Bridge and User Experience

- [x] **3.1 Update Rust canonical validation and manager bridge**
  - Add a strict Rust enum for Claude `"1m"` context.
  - Reject canonical Claude model IDs ending in `[1m]` before catalog checks.
  - Add strict preset result structs matching the Go protocol shape.
  - Validate Agent keys, missing-model array bounds, and model-ID syntax.
  - Forward preset data to TypeScript without storing it in secret-bearing
    `ModelFlow`.
  - Update import/export and shared canonical fixtures.
  - Add valid/invalid context, suffix, result-shape, malformed-response,
    flow-isolation, and no-secret tests.
  - Verification:
    `cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked`.

- [x] **3.2 Add TypeScript result types and per-Agent initialization**
  - Add optional `context?: "1m"` to Claude selection types.
  - Add typed preset and unavailable-Agent fields to discovery results.
  - Initialize each selected Agent independently with
    `existing > preset > empty`.
  - Keep desktop import as a complete form replacement.
  - Track non-sensitive initialization sources for presentation.
  - Add IPC, mixed-source, empty, unavailable, and import-precedence tests.
  - Verification: focused desktop IPC and Agent page Vitest suites plus
    `npm run typecheck` from `desktop/`.

- [x] **3.3 Add Claude display-name and context controls**
  - Add optional primary and explicit-role display-name inputs.
  - Add Standard/1M primary and explicit-role selectors.
  - Clear name and context when a model changes.
  - Remove the complete explicit selection when inheritance is enabled; create
    an incomplete explicit selection when inheritance is disabled.
  - Add preset-source and unavailable-model notices without a new wizard stage.
  - Preserve native accessibility, responsive layout, and established Agent
    page styling.
  - Add localization keys in English and Chinese with parity tests.
  - Add canonical preview, metadata-clearing, inheritance, source-notice, and
    responsive-control regression tests.
  - Verification: `npm run verify` from `desktop/`.

## Phase 4: Setup Clients

- [x] **4.1 Add Shell preset defaults and Claude context prompts**
  - Build one per-Agent initial document from existing first and preset second.
  - Use safe jq object operations; never evaluate preset text as shell code.
  - Display source summaries and unavailable base IDs without credentials.
  - Use existing/preset values as editable prompt defaults.
  - Add primary and explicit-role `standard`/`1m` prompts and reject other
    values before manager calls.
  - Keep `--model-config` as a complete replacement and keep hidden key input.
  - Add existing-over-preset, mixed-source, unavailable, blank-default,
    override, import, Unicode, context, and key-leakage tests.
  - Verification: `bash tests/setup_agent_v2_flow_test.sh` and
    `make test-shell`.

- [ ] **4.2 Add equivalent PowerShell behavior**
  - Mirror Shell per-Agent precedence, editable defaults, source notices, and
    Claude context semantics with typed PowerShell objects.
  - Preserve PowerShell 5.1 APIs, transient key behavior, and canonical request
    parity.
  - Preserve the existing UTF-8 BOM in `setup.ps1`.
  - Add flow, static wizard, Unicode, context, override, import, secrecy, and BOM
    tests.
  - Verification: focused PowerShell shell tests, `go test . -run PowerShell`,
    and `make test-shell`.
  - Recorded gap: static, BOM, and Go checks pass, but executable PowerShell
    flow verification is pending because `pwsh` is unavailable on this host.

## Phase 5: Build and Release Integration

- [x] **5.1 Inject the preset into every manager build**
  - Read optional `AGENT_MODEL_PRESET_BASE64` in local and desktop sidecar build
    scripts.
  - Add the exact preset linker symbol only to manager builds.
  - Keep empty local values valid and avoid injecting the value into the router.
  - Add static workflow/build tests for symbol, source variable, and binary
    scope.
  - Verification: `make test-workflows` and an empty-preset local build.

- [x] **5.2 Add release parity and preflight**
  - Source release preset input from `vars.AGENT_MODEL_PRESET_BASE64`.
  - Pass the same value to standalone manager and desktop sidecar matrix builds.
  - Validate configured input before publication without printing decoded JSON.
  - Keep an unset/empty release variable as a valid no-preset release.
  - Add release workflow assertions for standalone/desktop equality and invalid
    input failure.
  - Verification: `make test-workflows`.

## Phase 6: Compatibility, Documentation, and Integrated Verification

- [x] **6.1 Refresh Claude compatibility evidence**
  - Verify the pinned Claude Code artifact supports custom model-name variables
    and exact `[1m]` selection syntax.
  - Record reproducible source URL, revision/version, integrity, and digest.
  - Do not update only the retrieval date.
  - Add automated manifest/evidence assertions.
  - Verification: `go test ./internal/manager/agent -run Compatibility`.

- [x] **6.2 Update English and Chinese user contracts**
  - Update README, Agent model, desktop, build, troubleshooting, and changelog
    documents in both languages.
  - Document canonical context, rendering boundary, no capability inference,
    preset protocol shape, precedence, unavailability, import override,
    build-time Base64 input, startup failure, and sidecar exclusion.
  - Revise no-auto-selection wording to permit only visible authenticated
    presets while continuing to prohibit first-model/name heuristics and
    substitution.
  - Verification: documentation link/parity checks plus `make test-shell` and
    `make test-workflows`.

- [ ] **6.3 Run integrated verification and security review**
  - Run gofmt cleanliness, all Go tests, and Go vet.
  - Run setup and workflow test suites.
  - Run complete desktop verification.
  - Confirm base IDs are revalidated immediately before write.
  - Confirm preset/key data does not enter sidecars, journals, revisions, logs,
    diagnostics, or router builds.
  - Confirm no first-model, fuzzy-name, partial-preset, substitution, or cached
    fallback exists.
  - Confirm `setup.ps1` retains its UTF-8 BOM.
  - Verification:
    `test -z "$(gofmt -l .)"`, `go test ./...`, `go vet ./...`,
    `make test-shell`, `make test-workflows`, and `make desktop-verify`.
  - Recorded gap: all listed commands pass, but `make test-shell` reports the
    executable PowerShell flow as skipped because `pwsh` is unavailable.

## Completion Rule

Do not mark this package complete until every applicable item in
`checklist.md` has automated or recorded evidence. If a platform-specific check
cannot run in the execution environment, leave it unchecked and record the
gap.
