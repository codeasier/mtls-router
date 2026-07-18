# Agent Model Configuration Tasks

Tasks are dependency-ordered. A task is complete only when its code, focused
tests, and required documentation satisfy the stated verification. Execution
must not begin until this specification package is approved.

## Phase 1: Protocol and Data Contracts

- [x] **1.1 Define management protocol v2 Agent contracts**
  - Bump the code-owned management protocol version to `2` and update all
    router, manager, setup receipt, desktop expected-value, build, and release
    references.
  - Add `agent.models` and `agent.render` methods and deadlines.
  - Replace v1 preview/write parameter types with the v2 catalog token,
    canonical model config, revision, approval, and transient-key shapes.
  - Add the model-specific stable error codes from `spec.md`.
  - Add bounded optional validation error details with JSON Pointer and stable
    rule identifiers.
  - Remove unused or misleading legacy protocol result types rather than
    creating a second representation.
  - Make setup scripts require an exact v2 receipt and non-sensitive
    manager-info handshake before any key-bearing request.
  - Add strict decode, exact result shape, validation-details, request-limit,
    deadline, valid-v1-receipt rejection, and mixed-v1/v2 protocol tests.
  - Verification: `go test ./internal/manager/protocol ./internal/version ./internal/manager/app`.

- [ ] **1.2 Implement the canonical model-config schema**
  - Add focused Go types for the versioned top-level document and Claude,
    opencode, and Codex sections.
  - Validate exact selected-Agent section matching, all required fields,
    catalog membership, typed enums, positive integer relationships, and model
    count/size limits.
  - Reject invalid UTF-8 and duplicate keys; implement RFC 8785 canonical JSON,
    safe integer/number limits, and checked-in cross-language test vectors.
  - Implement constrained `extra` and `options` validation, deep merge,
    recursive normalized protected-key checks, pinned Agent-schema allowlists,
    and depth/size limits.
  - Generate and check in a versioned JSON Schema from the authoritative Go
    model for Rust/TypeScript local validation.
  - Add exhaustive fixtures for valid minimal/full documents and every invalid
    boundary in the canonical schema.
  - Verification: `go test ./internal/manager/agent/...`.

- [x] **1.3 Add cross-process Agent transaction locking**
  - Add a cross-platform OS-backed lock in the shared `agent-transactions`
    directory with a five-second bounded acquisition.
  - Acquire it for startup recovery, signing-key/state reads, discovery prefill,
    token/state snapshots, preview, and write through commit/removal or
    rollback; do not retain it between desktop requests.
  - Return `AGENT_OPERATION_BUSY` without partial state on contention.
  - Add desktop-versus-CLI, two one-shot writers, concurrent recovery, timeout,
    crash, and lock-release tests.
  - Verification: `go test ./internal/manager/agent/...`.

- [x] **1.4 Implement cross-process catalog and revision token contracts**
  - Atomically create/load the private per-user random 256-bit signing key and
    fail closed for missing/corrupt/permission-invalid trust state.
  - Add versioned authenticated catalog envelopes binding normalized IDs,
    Agent set, owner, router address, deployment, protocol, canonicalization,
    and signing-key generation.
  - Extend preview revision tokens to bind canonical config, catalog identity,
    sidecar revision, router address, and drift state.
  - Replace persisted plain content hashes with context-separated keyed revision
    MACs for Agent files, journal entries, sidecar, and backup verification.
  - Enforce token encoding/size limits and never copy the transient key.
  - Add cross-process verification, concurrent key creation, public-data
    forgery, tampering, key loss/rotation, version, deployment, owner, Agent,
    address, config, and limit tests.
  - Verification: `go test ./internal/manager/agent/...`.

## Phase 2: Trusted Discovery and Rendering

- [x] **2.1 Implement the bounded model-catalog client**
  - Add a focused manager package for `GET /v1/models`.
  - Send one Bearer header, no query/body, bypass environment proxies, reject
    redirects, and enforce HTTP/body deadlines and limits.
  - Strictly parse `data[].id`; reject boundary whitespace instead of mutating
    IDs, then validate, deduplicate, sort, and bound the catalog without
    consuming non-standard metadata.
  - Map authentication, transport/status, invalid-response, and empty-catalog
    failures to stable sanitized errors.
  - Add local HTTP server tests for headers, proxy/redirect prevention,
    malformed/trailing/oversized responses, IDs, counts, timeouts, and
    key-free logs/errors.
  - Verification: `go test ./internal/manager/...`.

- [x] **2.2 Integrate trusted router validation and automatic startup**
  - Reuse discovery and lifecycle identity rules before every key-bearing
    request.
  - Start an absent verified router under the request's CLI or desktop owner,
    validate readiness/deployment/protocol, and leave it under normal lifecycle
    ownership.
  - Reject unknown, stale, non-loopback, identity-mismatched, and mixed-v1/v2
    targets before transmitting a key.
  - Enforce desktop-owner eligibility, explicit restartable states, numeric
    loopback/port validation, and no automatic alternate-port selection.
  - Bind `/version` validation and `/v1/models` key transmission to one direct
    keep-alive TCP connection with no redial, proxy, or redirect.
  - Derive normalized Claude and OpenAI-compatible base URLs from the validated
    explicit listener, including IPv6 loopback syntax.
  - Revalidate the same deployment/address at write and return stale catalog
    rather than silently switching routers.
  - Add absent/start, owner-ineligible, existing CLI/desktop, degraded, unknown
    port, stale/unexpected-exit state, wrong deployment/protocol, process swap,
    non-loopback, port zero, hostname, changed address, and restart-before-write
    tests.
  - Verification: `go test ./internal/manager/...`.

- [x] **2.3 Implement key-free model discovery results**
  - Wire catalog fetch, catalog token, normalized URLs, selected-Agent scope,
    existing-config prefill, unavailable IDs, and drift reporting.
  - Use last-applied state only when recorded revisions match; otherwise parse
    supported typed model fields without returning unrelated content.
  - Never return existing keys, headers, raw Agent files, or raw response data.
  - Add same-catalog-for-all-Agents, valid prefill, unavailable model, drift,
    no-state, and secret-canary tests.
  - Verification: `go test ./internal/manager/agent ./internal/manager/app`.

## Phase 3: Agent-Native Configuration

- [x] **3.1 Refactor render planning into focused Agent renderers**
  - Keep path detection, JSONC migration, file revisions, and transaction
    planning separate from canonical validation and Agent-specific rendering.
  - Add managed-fragment rendering with the fixed redacted key placeholder for
    `agent.render` and preview.
  - Enforce bounded output and safe JSON/TOML/terminal escaping for dynamic IDs,
    names, paths, options, and extensions.
  - Remove all hard-coded model IDs and guessed model metadata.
  - Verification: `go test ./internal/manager/agent`.

- [x] **3.2 Implement Claude Code model mapping and safe merge**
  - Render primary and inherited/explicit Haiku, Sonnet, and Opus IDs and
    optional names exactly as specified.
  - Use the actual trusted base URL, transient auth token, and fixed policy
    flags without enabling runtime gateway discovery.
  - Merge only managed env keys and preserve all unrelated env/top-level data.
  - Remove obsolete owned optional-name and extension keys safely.
  - Add minimal/full/inherited/name-omitted/extension/drift and unrelated-env
    preservation tests.
  - Verification: `go test ./internal/manager/agent`.

- [x] **3.3 Implement opencode model catalog and safe merge**
  - Render exactly the selected model subset, default root model, typed fields,
    options, and validated extensions under `provider.mtls-router`.
  - Omit unspecified capabilities/limits/modalities and default only display
    name to the model ID.
  - Preserve unrelated root fields/providers and enforce ownership/drift rules
    for root `model`.
  - Preserve canonical and explicit JSONC migration/normalization behavior.
  - Add exact subset/removal/default/collision/extension/escaping and JSONC
    tests.
  - Verification: `go test ./internal/manager/agent`.

- [ ] **3.4 Implement Codex model settings, provider migration, and auth merge**
  - Render `model_providers.mtls-router`, active model, file credential store,
    typed current-schema root model keys, and the pinned extension allowlist;
    remove obsolete `disable_response_storage` output.
  - Use complete TOML encoding for all dynamic values.
  - Remove `model_providers.custom` only for an exact historical mtls-router
    full signature across provider, root model/provider values, and auth shape;
    preserve every partial match and user-owned custom provider.
  - Detect effective authentication conflicts and require a separate preview-
    bound approval to switch local CLI/IDE to official file-backed
    `auth_mode=apikey` and `OPENAI_API_KEY` shape.
  - Remove competing known auth material only after approval, keep OS keyring
    credentials untouched, and reject forced/managed ChatGPT-only policy.
  - Add complete historical migration, partial root/provider/auth combinations,
    user-owned custom provider, existing dedicated provider drift, optional
    removal, special-character, pinned parser, API-key migration, keyring
    rollback, policy rejection, and approval tests.
  - Verification: `go test ./internal/manager/agent`.

- [x] **3.5 Update key-free Agent detection semantics**
  - Report local structural completeness for dynamic model config without
    claiming current authorization.
  - Recognize the old Codex signature as migratable but not v2-configured.
  - Use actual managed URL shape rather than fixed port/model IDs.
  - Keep all stored key values and model extensions out of detection results.
  - Add configured/incomplete/migratable/invalid tests for all Agents.
  - Verification: `go test ./internal/manager/agent`.

## Phase 4: Preview, Transaction, and Manager Wiring

- [x] **4.1 Add transactional last-applied model state**
  - Add the private sidecar under the existing `agent-transactions` state
    directory with schema version, canonical per-Agent sections, managed paths,
    target paths, owned paths, key generation, and keyed revision MACs only.
  - Update selected sections while preserving unselected Agent state.
  - Include the sidecar in planning, private backup, journal, replacement,
    rollback, deadline handling, and startup recovery.
  - Add strict sidecar schema/limit/corruption behavior, an internal journal
    state scope, Agent-files-first/sidecar-last replace order, and separate
    state result fields.
  - Prove that transient keys, unkeyed credential-dependent hashes, catalogs,
    raw responses, rendered content, and unrelated settings never enter
    sidecar or journal.
  - Add create/update/subset, permission, partial failure, rollback, crash-point,
    corrupt-state, and secret-canary tests.
  - Verification: `go test ./internal/manager/agent`.

- [x] **4.2 Implement drift-aware preview and write preflight**
  - Extend preview with canonical config, redacted fragments, exact managed
    namespaces/collisions, sidecar revision, drift/auth state, and bound token.
  - Apply the fixed bootstrap collision matrix and exact v1 migration fixtures.
  - Require managed-overwrite and Codex-auth approvals exactly when bound.
  - Before any write artifact, revalidate trusted router and refresh catalog
    with the transient key; reject changed router/token identity and missing
    selected IDs, but allow unrelated catalog additions/removals.
  - Keep all current backup, atomic replace, rollback, deadline, and recovery
    guarantees after preflight.
  - Add no-write-on-every-preflight-failure assertions and success/stale/drift/
    removed-model/rollback tests.
  - Verification: `go test ./internal/manager/agent`.

- [x] **4.3 Wire complete protocol v2 Agent handlers**
  - Connect lifecycle, discovery, model client, Agent service, and metadata in
    the manager app without duplicating validation.
  - Implement `agent.models`, `agent.render`, v2 preview, and v2 write mapping.
  - Clear request-held API-key values promptly and keep stdout protocol-pure.
  - Add multi-request subprocess tests proving cross-process catalog-token use,
    write refresh, stable errors, deadlines, and no secret output.
  - Verification: `go test ./internal/manager/... ./cmd/mtls-router-manager`.

## Phase 5: CLI and Desktop Clients

- [x] **5.1 Implement the Shell model flow**
  - Add key-before-discovery and the Agent-native common-field wizard.
  - Add bounded regular-file `--model-config` parsing and manager validation.
  - Replace static print snippets with `agent.render` output.
  - Show exact preview, backup, and drift approval before v2 write.
  - Keep key input hidden and transient; direct noninteractive users to v2
    manager stdin automation.
  - Update help and compatibility aliases without changing no-argument router
    setup behavior.
  - Verification: `make test-shell`.

- [ ] **5.2 Implement the PowerShell model flow**
  - Match Shell behavior and canonical protocol requests using PowerShell 5.1
    compatible APIs.
  - Preserve SecureString handling where available and clear transient values.
  - Preserve the UTF-8 BOM and add executable flow coverage rather than only
    source-string assertions.
  - Verification: `make test-shell && go test ./...`.

- [x] **5.3 Upgrade the Rust manager and Tauri command layer**
  - Add typed v2 model config, discovery, render, preview, and write requests.
  - Hardcode `owner=desktop` at the command boundary and validate all sizes,
    enums, IDs, tokens, and structured extra values.
  - Move the key from React immediately into Rust zeroizing flow state under an
    unguessable ID; consume it once at write and destroy it on every defined
    terminal path.
  - Mark models/write non-replayable across manager exit, timeout, malformed
    response, or uncertain delivery; never auto-resend secret-bearing calls.
  - Add focused model-config import/export commands with no arbitrary file API.
  - Update watchdog deadlines and protocol version handshake.
  - Add fake-manager, strict-validation, timeout, cancellation, and zeroization
    tests.
  - Verification: `npm run rust:format && npm run rust:test` from `desktop/`.

- [ ] **5.4 Implement the React Agent model workbench**
  - Replace the old stages with select, credential, discover, configure,
    preview, write, and result states.
  - Show one searchable catalog without automatic selection.
  - Add Claude role inheritance, opencode multi-model/default/per-model fields,
    Codex fields, omission controls, and existing-value prefill.
  - Add per-Agent constrained JSON editors for `extra` with formatting and
    path-specific validation.
  - Add import/export of canonical model config with no credential-designated
    fields.
  - Show redacted fragments, file/backup effects, drift approval, and stable
    error recovery without endpoint-compatibility warnings.
  - Keep desktop and narrow layouts usable and preserve established visual
    language.
  - Add component, state-transition, accessibility, localization, secret-DOM,
    and IPC tests.
  - Verification: `npm run static:check && npm run typecheck && npm test && npm run build` from `desktop/`.

## Phase 6: Contract Coverage, Documentation, and Acceptance

- [x] **6.1 Add all-endpoint service contract fixtures**
  - Advertise one model and exercise the exact method/path/query/header matrix
    for models, Messages, count_tokens, Chat Completions, Completions, and
    Responses with Bearer authorization preserved.
  - Verify required SSE streaming remains unbuffered and Claude open-list beta/
    version/tool fields pass through unchanged.
  - Assert raw access/error logs contain no authorization value, request body,
    response body, query, certificate material, or key-shaped canary.
  - Verification: `go test ./internal/proxy ./internal/log ./...`.

- [x] **6.2 Update release metadata and English/Chinese documentation**
  - Document the service contract, key-before-discovery flow, canonical schema,
    Agent-native options, omission behavior, manual refresh, hard failures,
    detection semantics, v2 automation, migration, and ownership boundaries.
  - Update README, desktop, troubleshooting, changelogs, setup help, desktop
    locale resources, and maintainer/release docs in English and Chinese.
  - Correct the nonexistent `docs/agent-conf/` repository-map reference or add
    generated golden fixtures with equivalence tests.
  - Mark every contradictory old desktop spec/task/checklist assertion in the
    supersession matrix, including old protocol/deadline and configured-state
    evidence, as superseded so historical checks are not v2 acceptance.
  - Record and test the compatibility manifest for current stable Agent/schema
    versions.
  - Verify release preflight rejects mixed protocol generations.
  - Verification: documentation parity/link review and release test coverage.

- [ ] **6.3 Run the complete acceptance matrix**
  - Complete every item in `checklist.md` with automated or recorded evidence.
  - Run Go formatting, unit/integration tests, vet, shell tests, frontend static
    checks/tests/build, Rust format/tests, and supported package checks.
  - Verify Windows, macOS, and Linux permissions, loopback behavior, migration,
    rollback/recovery, CLI/desktop shared state, and no real secrets.
  - Verification: all commands in `spec.md` pass and release blockers are
    recorded explicitly.
