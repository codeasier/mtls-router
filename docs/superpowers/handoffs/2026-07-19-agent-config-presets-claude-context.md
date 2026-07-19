# Agent Configuration Presets and Claude Context Handoff

## Handoff Summary

The `agent-config-presets-claude-context` change is implemented, committed, and
pushed to `main`. A GitHub validation-only build for macOS arm64 completed
successfully and its CLI and desktop artifacts were downloaded and verified.

The only known acceptance gap is executable PowerShell verification. The local
host does not have `pwsh`, so PowerShell source, static checks, UTF-8 BOM checks,
and Go integration checks pass, but the PowerShell v2 interactive flow has not
run on this host.

## Original Goal

Extend Agent model configuration without weakening the existing authenticated
catalog, transaction, ownership, or secret-handling contracts.

The requested behavior was:

- Let Claude Code users configure optional display names for the primary model
  and every explicit Haiku, Sonnet, and Opus selection.
- Let every explicit Claude selection independently choose standard context or
  1M context.
- Store only the authenticated base model ID in canonical configuration and
  append `[1m]` only when rendering Claude Code settings.
- Recover one exact terminal `[1m]` suffix from existing Claude settings into a
  base model ID plus canonical `context: "1m"`.
- Inject one key-free canonical Agent model preset into manager builds through
  `AGENT_MODEL_PRESET_BASE64`.
- Validate the injected preset structurally at manager startup and validate each
  requested Agent section independently against the current authenticated model
  catalog during discovery.
- Initialize each selected Agent using `existing > preset > empty`, without
  deep-merging existing and preset sections.
- Keep presets editable and require the normal preview, approval, refreshed
  catalog validation, and transactional write flow.
- Keep preset data out of router binaries, API keys, tokens, sidecars, journals,
  backups, logs, diagnostics, and secret-bearing desktop flow state.
- Preserve equivalent Shell, PowerShell, and desktop behavior and update English
  and Chinese documentation together.

The feature explicitly did not include model capability inference, first-model
selection, fuzzy name matching, model substitution, partial preset repair,
cached catalog fallback, runtime 1M fallback, or management of
`CLAUDE_CODE_DISABLE_1M_CONTEXT`.

## Completed Work

### Canonical Claude contract

- Added optional Claude selection `context` with the only accepted value
  `"1m"`; omission retains standard behavior.
- Rejected canonical Claude model IDs ending in `[1m]`, independently of catalog
  membership.
- Kept canonical schema version `1` and compatibility with existing documents
  that omit `context`.
- Updated Go types and validation, generated JSON Schema, Rust types, TypeScript
  types, and shared canonical/JCS vectors.
- Added a shared structural decode path for presets that enforces canonical
  structure without requiring a live catalog.
- Added a consistent limit of 1000 referenced model IDs per Agent so Go protocol
  results and Rust response validation have the same bound.

### Claude rendering and existing projection

- Appended exactly one terminal `[1m]` at the Claude rendering boundary for the
  primary, custom option, and effective role model variables.
- Kept display-name variables unchanged and optional.
- Did not add `CLAUDE_CODE_DISABLE_1M_CONTEXT` to manager-owned settings.
- Parsed one exact terminal `[1m]` from existing settings into base ID plus
  context.
- Kept malformed, repeated, middle-marker, and alternate-case forms from being
  silently repaired.
- Required model, optional name, and context to all match before projecting an
  existing role as `inherit_primary`.
- Used base IDs for catalog availability and unavailable-model reporting.

### Build-time preset and manager protocol

- Added `internal/manager/preset` with linker symbol:

  ```text
  github.com/codeasier/mtls-router/internal/manager/preset.Encoded
  ```

- Added strict standard Base64 decoding, UTF-8/JSON validation, canonical size
  and model-reference bounds, duplicate/trailing-data rejection, protected-field
  rejection, and sanitized startup failures.
- Empty or unset `AGENT_MODEL_PRESET_BASE64` remains a valid no-preset build.
- Manager startup loads and validates the preset before protocol serving or
  Agent transaction recovery.
- The Agent service stores a canonicalized private preset copy outside signer,
  transaction, sidecar, revision, and approval state.
- Discovery crops the preset to requested Agents and validates each section
  atomically against the authenticated catalog.
- A missing model omits the complete Agent section and reports sorted unique base
  IDs without blocking valid sections for other Agents.
- Added stable protocol v2 response objects:

  ```json
  {
    "preset": {
      "model_config": {},
      "unavailable_agents": {}
    }
  }
  ```

- Preset unavailability uses `MODEL_NOT_AVAILABLE` metadata and does not turn an
  otherwise valid discovery request into an error.

### Desktop bridge and UI

- Added strict Rust decoding and validation for Claude context and preset result
  metadata.
- Aligned Rust model-ID validation with Go, including leading/trailing Unicode
  whitespace rejection.
- Forwarded preset data to TypeScript without storing it in secret-bearing
  `ModelFlow`.
- Added per-Agent `existing > preset > empty` initialization without deep merge.
- Preserved desktop import as a complete replacement of generated defaults.
- Added optional Claude display-name fields and Standard/1M controls for the
  primary and all explicit roles.
- Model changes clear stale display-name and context metadata.
- Enabling inheritance removes the complete explicit role selection; disabling
  inheritance creates an incomplete explicit selection that must be completed
  before preview.
- Added localized existing/preset source notices and unavailable preset notices
  in English and Chinese.
- Added responsive and accessibility regression coverage.

### Shell and PowerShell clients

- Added per-Agent initialization from existing configuration first, preset
  second, and empty configuration last.
- Added non-sensitive source summaries and unavailable preset model notices.
- Existing and preset values are editable prompt defaults; blank input accepts
  a displayed default and `-` clears optional values.
- Added primary and explicit-role Claude model, display-name, and
  `standard`/`1m` prompts.
- Unsupported context input fails before render, preview, or write.
- Preserved `--model-config` as a complete replacement while still requiring
  authenticated discovery and manager validation.
- Preserved hidden transient API-key input.
- Preserved PowerShell 5.1-compatible APIs and the `setup.ps1` UTF-8 BOM.
- Added executable Shell coverage for independent Haiku, Sonnet, and Opus names
  and contexts.
- Added executable PowerShell fixtures and static assertions, but did not execute
  them because `pwsh` is unavailable on the current host.

### Build and release integration

- Added manager-only preset linker injection to local builds, desktop sidecar
  builds, standalone release managers, and desktop release manager sidecars.
- Confirmed router build paths do not receive the preset linker symbol.
- Added release preflight using the real manager startup validation path without
  printing encoded or decoded preset content.
- Release input is sourced from `vars.AGENT_MODEL_PRESET_BASE64`.
- Added workflow tests for symbol accuracy, source variable use, standalone and
  desktop parity, invalid-input failure, and router exclusion.
- Locally built empty and valid preset variants for the manager and native
  desktop sidecar on macOS arm64.
- Verified at dependency, binary-membership, startup, and runtime levels that the
  router does not contain or consume the preset.

### Compatibility evidence and documentation

- Pinned Claude Code compatibility evidence to version `2.1.214` with source
  URL, npm revision, integrity, archive digest, binary digest, and reproducible
  extraction metadata.
- Recorded support for all managed Claude custom model-name variables and exact
  `[1m]` selection syntax.
- Updated these English and Chinese document pairs:

  ```text
  README.md / docs/zh-CN/README.md
  docs/AGENT_MODELS.md / docs/zh-CN/AGENT_MODELS.md
  docs/DESKTOP.md / docs/zh-CN/DESKTOP.md
  docs/BUILD.md / docs/zh-CN/BUILD.md
  docs/TROUBLESHOOTING.md / docs/zh-CN/TROUBLESHOOTING.md
  docs/CHANGELOG.md / docs/zh-CN/CHANGELOG.md
  ```

- Documented canonical context, rendering boundaries, preset protocol and
  precedence, unavailable sections, imports, build injection, startup failures,
  sidecar exclusion, and prohibited automatic-selection behavior.

## Source and Specification State

The implementation commit is:

```text
affa57874c6808576fa172e016ad2abd71873eec
feat: add agent model presets and Claude context
```

It has been pushed to `origin/main`. At handoff time, local `HEAD` and
`origin/main` both resolve to that exact SHA.

The specification package is:

```text
specs/agent-config-presets-claude-context/spec.md
specs/agent-config-presets-claude-context/tasks.md
specs/agent-config-presets-claude-context/checklist.md
```

The detailed implementation plan is:

```text
docs/superpowers/plans/2026-07-19-agent-config-presets-claude-context.md
```

The specification progress reflects actual evidence. PowerShell runtime items
and final unqualified completion remain unchecked because the executable
PowerShell suite was skipped.

## Verification Completed

The final integrated verification passed:

```bash
test -z "$(gofmt -l .)"
go test ./...
go vet ./...
make test-shell
make test-workflows
make desktop-verify
```

Relevant results:

- All Go packages passed.
- Go vet passed.
- Shell v2 Agent flow passed, including explicit Claude role name/context cases.
- Workflow tests passed.
- Desktop verification passed with 82 Vitest tests and 59 Rust tests.
- TypeScript type checking, ESLint, Prettier, Vite production build, Cargo
  formatting, and Cargo tests passed.
- `setup.ps1` retained its UTF-8 BOM.
- `make test-shell` exited successfully but reported the executable PowerShell
  flow as skipped because `pwsh` is not installed.

## GitHub Validation Build

A real validation-only GitHub Actions build used repository Secrets and
Variables without reading or passing credentials locally.

Build details:

```text
Version: 0.1.0-agent-presets.1
Target: darwin-arm64
Run ID: 29676444092
Run URL: https://github.com/codeasier/mtls-router/actions/runs/29676444092
Head SHA: affa57874c6808576fa172e016ad2abd71873eec
Conclusion: success
```

The temporary `agent-test-build/...` branch was deleted after the run. The
release publication job was skipped as required by validation-only mode.

The first desktop artifact download ended with a network `unexpected EOF` after
the successful build. No second build was triggered. Both artifacts were
downloaded again from the same run into:

```text
desktop/release-artifacts/run-29676444092-retry/
```

Downloaded packages and SHA-256 values:

```text
acea1b696b1bb0249562d03b8491ee64c8436cad8c8231bfb3302c456688e86d  mtls-router-darwin-arm64
bc5449a8279d9cc0d36f0026129017986a494c7b9ea3994b0136ea323e5a5e6b  mtls-router-manager-darwin-arm64
2af5d71895ed1891479845885f77f570b898b0c8b402292f8ee147dc349e8f83  CodeasierRouter-darwin-arm64.dmg
```

The DMG's included checksum manifest passed. Its signing status is:

```text
ad-hoc signed (not trusted by Gatekeeper)
```

This is expected for the validation fallback path. It is not an Apple Developer
ID trusted signature and is not notarized.

The incomplete first download directory remains at:

```text
desktop/release-artifacts/run-29676444092/
```

It contains only the CLI artifact. Both artifact directories are under the
repository's ignored release-artifact path and must not be committed.

## Remaining Work

### Required to close the current specification

Run the executable PowerShell v2 flow in an environment with `pwsh` available:

```bash
bash tests/setup_powershell_v2_flow_test.sh
make test-shell
```

The evidence must confirm:

- Per-Agent `existing > preset > empty` precedence.
- No deep merge between existing and preset sections.
- Existing and preset blank-default behavior.
- User overrides and Unicode display-name preservation.
- Standard/1M canonical serialization.
- Unsupported context rejection before manager configuration calls.
- Hidden API-key behavior and no key leakage.
- `--model-config` complete replacement behavior.
- Shell and PowerShell canonical request equivalence.

For PowerShell 5.1 acceptance, also run the relevant setup flow on native Windows
PowerShell 5.1 rather than relying only on `pwsh`. Record the OS and PowerShell
versions with the result.

After that evidence passes:

1. Update task 4.2 and the corresponding PowerShell checklist items in
   `specs/agent-config-presets-claude-context/`.
2. Rerun all six integrated verification commands.
3. If no checks are skipped, mark task 6.3 and the final `make test-shell`
   acceptance item complete.
4. Commit the evidence/progress update separately.

### Optional follow-up

- Run validation-only GitHub builds for Windows and Linux targets if release
  confidence is needed beyond the verified macOS arm64 target.
- Run a trusted macOS signing and notarization validation when Apple signing
  credentials are configured. Do not describe the current ad-hoc DMG as trusted
  or notarized.
- Remove the incomplete ignored download directory
  `desktop/release-artifacts/run-29676444092/` only if it is no longer useful.

## Important Safety Constraints for Follow-up Work

- Do not read, copy, print, or pass local mTLS/signing secrets when using the
  GitHub validation build workflow.
- Do not trigger a second validation build merely to retry an artifact download;
  download again from the existing successful run.
- Do not dispatch an existing tag directly for validation because that could
  activate release publication behavior.
- Do not add preset content to router linker flags, runtime environment, logs,
  sidecars, journals, backups, revision claims, or diagnostics.
- Do not infer 1M support from model names or catalog order.
- Do not substitute, partially repair, or select another model when a preset
  model is unavailable.
- Continue to validate base model IDs against a refreshed authenticated catalog
  immediately before write artifacts are created.
- Preserve the `setup.ps1` UTF-8 BOM and PowerShell 5.1 compatibility.

## Workspace Notes

At handoff time, two unrelated untracked documents already exist:

```text
docs/superpowers/plans/2026-07-19-github-test-build-skill.md
docs/superpowers/specs/2026-07-19-github-test-build-skill-design.md
```

They were deliberately excluded from commit `affa578` and must not be modified,
deleted, or committed as part of this feature unless explicitly requested.

This handoff document is tracked separately from implementation commit
`affa578`, so its review history does not alter the completed feature commit.
