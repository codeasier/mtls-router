# Agent Model Configuration Acceptance Checklist

Check an item only after automated evidence or recorded manual verification
demonstrates the behavior. This checklist applies together with the unaffected
requirements in `specs/tauri-desktop-app/checklist.md`.

## Scope and Contract

- [ ] The implementation matches the approved `spec.md` without static model
  fallback or undocumented compatibility behavior.
- [x] `GET /v1/models` is the only candidate model-ID source.
- [x] Every returned model is offered to Claude Code, opencode, and Codex.
- [x] No model is filtered or preferred by ID/name heuristics.
- [x] Configuration performs no inference probe.
- [x] Local fixtures protect the exact catalog, Messages, count_tokens, Chat
  Completions, Completions, and Responses method/path/header/query/SSE matrix.
- [x] Pagination and non-standard model metadata are not accidentally required.

## Protocol v2

- [x] Router, manager, desktop, setup receipt, and release metadata report
  management protocol v2.
- [x] Mixed v1/v2 artifacts are rejected before Agent configuration or key
  transmission.
- [x] Setup rejects a hash-valid v1 receipt/manager before serializing any
  key-bearing request and performs a v2 manager-info handshake.
- [x] Protocol exposes `agent.models`, `agent.render`, v2 `agent.preview`, and
  v2 `agent.write` with strict request decoding.
- [x] Old preview/write requests without canonical model config fail clearly
  and never trigger static behavior.
- [x] Every method has the specified manager and Rust watchdog deadline.
- [x] All new model error codes are stable and clients branch on codes only.
- [x] Catalog and revision tokens are bounded, versioned, authenticated, and
  verifiable across one-shot manager processes.
- [x] Token tampering, wrong Agent/owner/address/deployment/protocol/generation,
  and invalid size fail before mutation.
- [x] A private random per-user signing key, not public path/deployment data,
  authenticates cross-process tokens.
- [x] Signing-key create races converge; corruption, unsafe permissions, loss,
  and replacement fail according to documented state recovery rules.
- [x] Public state cannot forge a valid token.
- [x] Tokens never copy the transient API key or raw response.
- [x] Validation errors may include bounded JSON Pointer/rule details but never
  rejected values or file/response contents.
- [x] Desktop and one-shot managers serialize state/recovery/write with the
  OS-backed transaction lock and return `AGENT_OPERATION_BUSY` on timeout.

## Trusted Discovery

- [x] API key input is accepted only through transient manager stdin/IPC
  request bodies.
- [x] The manager validates router process, deployment, protocol, and loopback
  listener before sending the key.
- [x] Validation and `/v1/models` use one direct keep-alive connection with no
  redial, preventing a process-replacement race.
- [x] `owner=desktop` is rejected without verified desktop session/parent
  identity before key use.
- [x] An absent router is started safely under the requested CLI/desktop owner
  and validated before model discovery.
- [x] An unknown/stale/mismatched port occupant receives no key and no signal.
- [x] Discovery sends exactly one `Authorization: Bearer <api-key>` header.
- [x] Discovery sends no request body or query string.
- [x] Environment proxy settings cannot intercept the local request.
- [x] Redirects are not followed.
- [x] HTTP and response-body limits are enforced.
- [x] 401/403 returns `MODEL_AUTH_FAILED` without response content.
- [x] Other status/transport failures return `MODEL_DISCOVERY_FAILED` safely.
- [x] Discovery and all rendered Agent URLs use the same actual trusted
  listener rather than fixed port 19099.
- [x] Listener authority is an explicit numeric loopback and port `1..65535`;
  hostnames, port zero, non-loopback, and automatic port switching are rejected.
- [x] IPv4 and supported IPv6 loopback addresses render valid URLs.

## Catalog Parsing

- [x] A standard `{data:[{id:...}]}` response is accepted.
- [x] Unknown root/item fields are ignored without becoming configuration.
- [x] IDs with boundary whitespace are rejected rather than changed; valid IDs
  are validated, deduplicated, and deterministically sorted.
- [x] Empty IDs, controls, invalid UTF-8, malformed items, trailing JSON,
  excessive IDs, and oversized bodies are rejected.
- [x] A catalog with no valid model returns `MODEL_CATALOG_EMPTY`.
- [x] The same complete catalog is returned for all selected Agents.
- [x] Existing available model selections can be safely prefilled.
- [x] Existing unavailable model IDs are reported but never selected.
- [x] Discovery results contain no stored key, unrelated Agent field, header,
  raw Agent file, or raw model response.

## Canonical Model Config

- [x] Canonical schema version is required and unknown versions are rejected.
- [x] Invalid UTF-8 and duplicate JSON keys are rejected before semantic decode.
- [x] Selected Agents and top-level sections match exactly.
- [x] Every selected model ID belongs to the bound catalog.
- [ ] RFC 8785 canonical encoding and checked-in vectors match across Go, Rust,
  TypeScript, jq, and PowerShell.
- [x] Integer/number bounds avoid cross-language precision divergence.
- [x] Claude primary is required and all role inheritance/explicit selection
  forms validate correctly.
- [x] opencode has at least one model and default references a selected model.
- [x] Codex has one active catalog model.
- [x] Every typed enum, integer relationship, modality, and interleaved shape is
  validated.
- [x] Unset optional values are omitted rather than guessed.
- [x] opencode display name alone defaults to its model ID.
- [x] Extra/options deep merge is deterministic and bounded.
- [x] Typed/managed path conflicts fail instead of overriding or being ignored.
- [x] Recursively normalized secret, connection, URL, auth, provider, header,
  transport, proxy, and fetch extension/option paths are rejected.
- [x] Dynamic JSON/TOML/terminal values are escaped safely.
- [x] Canonical model-config has no credential field; the manager never copies
  the transient key into it, and files are bounded regular JSON files.
- [x] Manager-generated JSON Schema and conformance fixtures keep Go, Rust, and
  TypeScript validation aligned while manager semantics remain authoritative.

## Render and Preview

- [ ] `agent.render` creates no state, backup, journal, or Agent file.
- [x] Render returns only manager-owned fragments, never complete user files.
- [x] Every fragment uses `<redacted-api-key>` and contains no real key.
- [x] `agent print-config` uses manager render output with no hard-coded snippets.
- [ ] Preview writes nothing, including state directories and sidecars.
- [x] Preview shows canonical selections, redacted fragments, file operations,
  backups, migration warnings, managed namespaces, and drift.
- [x] Revision token binds config, catalog, router, Agent files, sidecar, and
  drift state.
- [x] External changes after preview return `PREVIEW_STALE` before mutation.
- [x] Drift requires explicit managed-overwrite approval and cannot broaden the
  managed namespace.

## Claude Code

- [x] Primary, Haiku, Sonnet, and Opus IDs render from explicit/inherited
  canonical selections.
- [x] Typed display names render only when configured and stale managed name
  keys are removed safely.
- [x] Actual router URL and transient Bearer token render correctly.
- [x] Tool-search and updater policy remain fixed and the service-contract
  fixtures prove deferred tool/beta field pass-through.
- [x] Gateway runtime model discovery is not enabled.
- [x] Unrelated top-level settings and env entries are preserved.
- [x] Existing env is never replaced wholesale.
- [x] Prior owned extra keys can be removed; unowned fields are never silently
  claimed.

## opencode

- [x] `provider.mtls-router` uses the fixed compatible npm package, provider
  identity, actual API base URL, and transient key.
- [x] Models contain exactly the selected catalog subset.
- [x] Removed models do not remain in the managed provider.
- [x] Typed limits, modalities, capabilities, interleaved, options, and extras
  render exactly when set.
- [x] Omitted optional metadata remains omitted.
- [x] Root `model` is `mtls-router/<selected-default>` under ownership rules.
- [x] An unowned conflicting root model is never silently overwritten.
- [x] Other root fields, providers, and `small_model` remain unchanged.
- [x] Canonical JSONC migration and explicit JSONC normalization remain safe.

## Codex

- [x] New config uses `model_providers.mtls-router` and provider ID
  `mtls-router`.
- [x] Provider uses Responses, OpenAI auth, actual API base URL, and current
  pinned Codex schema; obsolete `disable_response_storage` is not emitted.
- [x] Active model and every optional typed model setting render correctly.
- [x] Codex extensions are restricted to the pinned current model-key allowlist;
  arbitrary `model_*` keys are rejected.
- [x] Dynamic TOML values survive quotes, slashes, Unicode, and control tests
  through valid encoding.
- [x] Exact historical mtls-router `custom` provider is migrated.
- [x] Partial provider/root/auth signature matches and all user-owned or
  ambiguous `custom` providers are preserved.
- [x] Unrelated root keys and sections are preserved.
- [x] Preview separately warns and requires approval before switching shared
  local CLI/IDE auth to file-backed API-key mode.
- [x] Approved migration writes official `auth_mode="apikey"` plus
  `OPENAI_API_KEY` and removes competing known auth material.
- [x] OS keyring credentials remain untouched and rollback restores the prior
  config/auth-file selection.
- [x] ChatGPT-only forced/managed policy returns `CODEX_AUTH_UNSUPPORTED` with
  zero credential or policy changes.
- [ ] Generated Codex config parses with the pinned current Codex parser.

## State, Write, and Recovery

- [x] Last-applied sidecar contains only schema, canonical selected config,
  managed paths, target paths, signing generation, and keyed revision MACs.
- [x] Sidecar contains no copied key, unkeyed credential-dependent hash,
  catalog, raw response, rendered content, upstream detail, or unrelated setting.
- [x] Malformed/unsafe/mismatched sidecar is `MODEL_STATE_INVALID`, not silent
  ownership reset.
- [ ] Sidecar and backups use user-private permissions.
- [x] Updating a subset preserves unselected Agent sidecar sections.
- [x] Sidecar is included in the same backup/journal/replace/rollback transaction
  as Agent files.
- [x] Crash recovery cannot leave sidecar ownership inconsistent with files.
- [x] Journal uses an internal state scope, writes sidecar last, rolls it back
  first, and reports state changes separately from Agents.
- [x] Write revalidates trusted router and current catalog before any write
  artifact is created.
- [x] A changed router identity/address returns `MODEL_CATALOG_STALE` with zero
  writes.
- [x] A disappeared model returns `MODEL_NOT_AVAILABLE` with zero writes and no
  substitution.
- [x] Auth, transport, invalid catalog, invalid config, missing approval, and
  stale preview all leave files unchanged.
- [x] Successful multi-Agent writes retain current backup, atomicity, result,
  rollback, deadline, and recovery guarantees.

## Detection

- [x] `configured` reports local managed structural completeness only.
- [x] Detection never claims current model authorization.
- [x] Claude detection checks dynamic required model slots and managed URL/key.
- [x] opencode detection checks provider models and valid managed root default.
- [x] Codex detection checks dedicated provider, model settings, and auth.
- [x] Historical Codex custom config is recognized for migration but is not v2
  configured.
- [x] Detection results contain no stored key or extension values.

## Setup Shell and PowerShell

- [ ] Both scripts use key-before-discovery and Agent-native selection flows.
- [ ] Both scripts support all required and typed common fields.
- [ ] Neither script automatically selects a model.
- [ ] `--model-config` bypasses setting prompts but not key discovery or manager
  validation.
- [ ] Print is dynamic, exact, and redacted; it changes no Agent/transaction
  config, while documenting that discovery may start router lifecycle state.
- [ ] Write displays preview and requires explicit confirmation/drift approval.
- [ ] Noninteractive use directs users to manager protocol v2 and has no hidden
  static/existing-config fallback.
- [ ] Keys never enter flags, environment variables, model-config, logs, or
  output.
- [x] Static model/provider snippets no longer exist in either script.
- [ ] No-argument setup still changes no Agent file.
- [ ] Compatibility aliases use the same v2 behavior.
- [ ] `setup.ps1` retains its UTF-8 BOM and PowerShell 5.1 compatibility.

## Desktop

- [x] Flow order is select, credential, discover, configure, preview, write,
  result.
- [x] React clears key input immediately after discovery submission; Rust stores
  it only in zeroizing unguessable flow state and consumes it once at write.
- [x] Secret-bearing models/write requests are never automatically replayed
  after timeout, malformed response, manager exit/restart, or uncertain delivery.
- [x] Flow state is destroyed on all specified terminal/error/navigation paths.
- [x] The same searchable catalog appears for every Agent.
- [x] No model is automatically selected.
- [x] Valid existing selections prefill and unavailable values require action.
- [x] Claude role inheritance UI is complete.
- [x] opencode multi-model, default, and per-model typed controls are complete.
- [x] Codex typed controls are complete.
- [x] Unset optional fields remain visibly unset.
- [x] Per-Agent extra editors format and report path-specific validation errors.
- [x] No arbitrary whole-Agent configuration editor or file API is exposed.
- [x] Focused canonical model-config import/export contains no key.
- [x] Preview shows redacted exact fragments and drift approval.
- [x] Stale catalog, removed model, stale preview, auth, and discovery errors
  return to the correct recoverable stage.
- [x] UI contains no endpoint-compatibility warning.
- [ ] Desktop and narrow window layouts remain usable.
- [x] Chinese and English strings are complete and tested.
- [x] DOM, snapshots, errors, logs, diagnostics, and persisted UI state contain
  no API key.

## Security and Logging

- [x] Router and manager raw logs contain no authorization header, query,
  request/response body, Agent payload, certificate material, or key canary.
- [x] Models/write protocol request bodies are never logged.
- [x] Errors never include upstream response bodies or redirect targets.
- [ ] API keys exist persistently only in required Agent files and approved
  sensitive backups, and transiently only in specified request/memory/private
  atomic-write temporary-file boundaries.
- [ ] Temporary secret-bearing files remain private.
- [x] Rust performs best-effort zeroization and frontend state is cleared.
- [ ] No real credential is present in source, fixtures, sidecar, journal,
  package, or documentation.

## Documentation and Release

- [x] English and Chinese README describe equivalent model workflows.
- [x] English and Chinese desktop docs describe equivalent desktop behavior.
- [x] English and Chinese troubleshooting cover every stable model error.
- [x] English and Chinese changelogs record protocol v2 and migration.
- [x] Setup help documents interactive flow, `--model-config`, and v2
  automation.
- [x] Documentation states all returned models support all required endpoints.
- [x] Documentation distinguishes local configured state from authorization.
- [x] Documentation describes omission defaults, manual refresh, hard failures,
  merge ownership, drift, and backups.
- [x] `AGENTS.md` no longer references a nonexistent unchecked docs directory.
- [x] Old desktop spec/tasks/checklist contradictions are marked superseded and
  their historical checked evidence is not used for v2 acceptance.
- [x] Compatibility manifest pins current tested Claude Code, opencode schema,
  and Codex parser/binary revisions.
- [x] Release preflight rejects mixed protocol generations.

## Verification

- [x] `test -z "$(gofmt -l .)"` passes.
- [x] `go test ./...` passes.
- [x] `go vet ./...` passes.
- [x] `make test-shell` passes.
- [x] Desktop `npm run static:check` passes.
- [x] Desktop `npm run typecheck` passes.
- [x] Desktop `npm test` passes.
- [x] Desktop `npm run build` passes.
- [x] Desktop `npm run rust:format` passes.
- [x] Desktop `npm run rust:test` passes.
- [ ] Windows, macOS, and Linux permission and transaction checks pass.
- [ ] Supported package builds contain matching v2 router/manager/desktop
  artifacts and pass launch smoke tests.
