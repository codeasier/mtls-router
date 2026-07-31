# Agent Model Configuration

[中文](zh-CN/AGENT_MODELS.md)

This is the user and automation contract for management protocol v4. The Go
manager is authoritative when this guide and a client-side validation differ.

## Service Contract

The authenticated upstream `GET /v1/models` response is the only candidate
model source. The manager validates the complete bounded OpenAI-compatible
response and every `data[].id`, then deduplicates, enforces the unique-ID count
limit, and sorts by UTF-8 byte order before applying its immutable build filter.
With the default build policy `SIMPLIFY=True`, valid IDs containing ASCII `/`
are excluded; a manager built with `SIMPLIFY=False` retains every valid ID.
Full-width slash `／`, backslash `\`, and other non-ASCII lookalikes are not
ASCII `/` and are unaffected. The resulting filtered catalog is authoritative
for protocol results and tokens, existing and preset availability, imported
model config, render/preview validation, and write-time refresh. This filtering
does not change which routes the proxy supports:

| Client | Route | Contract |
|---|---|---|
| Catalog | `GET /v1/models` | One fully validated, bounded standard JSON response |
| Claude Code | `POST /v1/messages`, including `?beta=true` | Anthropic request fields and open-list `anthropic-*` headers pass through; SSE is unbuffered |
| Claude Code | `POST /v1/messages/count_tokens` | Exact token counting |
| opencode | `POST /v1/chat/completions` | OpenAI Chat Completions and streaming |
| Compatibility clients | `POST /v1/completions` | OpenAI Completions and optional streaming |
| Codex | `POST /v1/responses` | OpenAI Responses and SSE streaming |

Claude deferred tool fields and future open-list Anthropic request fields pass
through unchanged. Configuration never calls an inference endpoint and never
infers 1M context support from an ID, name, catalog position, or other
capability signal. The router preserves authorization but does not select
models or store keys. Every ID retained in the manager catalog is treated as
supported on all routes above; the build filter is a manager catalog policy,
not a proxy capability restriction or runtime model preference.

## Interactive Flow

The Shell, PowerShell, and desktop clients all preserve this protocol order:

1. Detect and select Claude Code, opencode, and/or Codex.
2. Read the API key without echo and establish a trusted protocol-v4 loopback router.
3. Call `agent.models`; only then show the common sorted catalog.
4. Initialize each selected Agent from a valid existing section, otherwise a visible authenticated preset section, otherwise an empty section; require the user to review or complete the editable Agent-native choices. Never select the first model, apply a model-name or capability heuristic, repair an unavailable preset, or substitute another model.
5. Render a redacted fragment for print, or preview exact file, backup, migration, ownership, and drift effects for write.
6. Re-fetch the catalog with the transient key immediately before writing, then perform one atomic multi-file transaction.

The Shell and PowerShell setup commands always omit recovery modes and therefore remain merge-only. The desktop opens one persistent single-Agent panel at a time and selects `merge` for a valid Agent or `rebuild` for an eligible invalid Agent. Each desktop preview and transaction contains exactly that Agent; rebuild follows the additional contract below. Management protocol remains v4 and adds `agent.cleanup.preview` and `agent.cleanup.write` for the separate cleanup contract below; these methods are not exposed as setup commands. As protocol-wide hardening, strict request parameter decoding now rejects duplicate JSON keys recursively at every object depth as well as unknown fields.

### Desktop persistent panel

Every panel entry runs `agent.detect`, reads the desktop credential summary, and calls authenticated `agent.models` only when a key is present and the target is editable or recovery-eligible. The desktop initializes `existing > preset > empty`, keeps the editor and preview rail on the same page, requires a preview for the current draft before every write, and remains in the panel after the transaction. A successful write consumes its flow, reports success immediately, and then repeats detection, credential summary, and discovery to load the actual on-disk configuration into a new flow. Failure of this reload is a separate panel state and does not turn the completed write into a failure.

During initial or full-reload discovery, a missing saved key, authentication failure, or catalog failure leaves the panel showing only key-free `agent.detect` metadata and recovery actions. It cannot safely show field-level prefill, import, export, render, or preview because those operations require a catalog token issued by authenticated `agent.models`. A failed background candidate refresh instead preserves the existing draft and active flow, reports that external state could not be verified, and keeps the operations that remain safe. The API key remains in Rust and the secret-bearing manager requests; it is not returned to the webview or placed in model config.

An editable panel listens for native window-focus signals and starts at most one candidate discovery every 15 seconds; manual refresh bypasses the interval but not the single-flight rule. A clean panel adopts the candidate. A dirty panel keeps its form baseline and draft: unchanged external state replaces only the active discovery, while changed external state requires an explicit keep-draft or load-disk decision. If detection becomes incompatible or unwritable, editing, import, preview, and write are blocked; export remains available only while a valid active flow exists. There is no polling, automatic merge, cached-catalog fallback, or Agent-file rewrite during refresh.

The frontend owns at most one active flow and one unresolved candidate request. A compatible successful candidate becomes active before the old flow is destroyed; a clean mode transition destroys the incompatible old flow before starting fresh discovery. Failed destroy requests remain deduplicated for retry. Obsolete, late, unmounted, or target-mismatched candidates are destroyed, and leaving or unmounting the panel destroys its active flow. A successful write consumes the active flow and is not destroyed again. `PREVIEW_STALE`, `MODEL_FLOW_EXPIRED`, and `MODEL_CATALOG_STALE` clear preview approvals, preserve the in-memory draft, and rediscover inside the panel; export stays disabled whenever no valid flow remains.

Filtering occurs only after the complete response and all IDs pass validation.
A malformed ID that contains ASCII `/` is therefore not hidden: the request
still fails with `MODEL_RESPONSE_INVALID`. If validation succeeds but filtering
removes every ID, discovery or refresh fails with `MODEL_CATALOG_EMPTY`.
Catalog tokens bind the manager's immutable build policy; changing that policy
makes an existing token stale and requires model rediscovery before render,
preview, or write.

`agent print-config` also requires a key because it validates choices against the
current catalog. It changes no Agent file, transaction journal, backup, or
last-applied sidecar. Discovery may start the router and first use may create the
private token-signing key.

The catalog is a configuration-time snapshot. Shell and PowerShell clients must
re-enter configuration and supply a key to refresh it. The desktop may replace
the snapshot through its explicit or throttled native-focus discovery described
above; this does not poll or rewrite an Agent file.

### Build preset

A release may inject one immutable, key-free canonical preset into its manager.
At startup the manager strictly decodes and structurally validates that preset;
during `agent.models`, it crops the preset to requested Agents and validates
each Agent section independently against the current authenticated catalog. A
section is returned complete only when every referenced base model ID is
available. If any ID is missing, the complete section is omitted and its sorted
missing base IDs are reported as nonfatal `MODEL_NOT_AVAILABLE` metadata. Other
valid Agent sections remain usable. The manager never partially repairs,
deep-merges, or substitutes a preset section.

Clients initialize each selected Agent independently with `existing > preset >
empty`. Preset values are visible editable defaults, not preview approval,
write confirmation, or proof of capability. Interactive edits override those
defaults. `--model-config=<path>` and desktop import are complete form
replacements and override existing/preset initialization rather than merging
with it. Existing and preset selections excluded by the build filter are
reported unavailable against the filtered catalog. An imported selection
outside that catalog remains invalid with `MODEL_CONFIG_INVALID` during
render/preview validation; filtering does not silently drop or substitute it.

## Canonical Model Config

All clients use one key-free JSON document. `version` is `1`; the `agents`
request array and present top-level sections must match exactly. Unknown fields
are rejected except in the constrained `extra`, `options`, and variant option
objects. Input is strict JSON: duplicate keys, invalid UTF-8, non-finite
numbers, unsafe integer ranges, and protected credential/connection paths are
rejected. Canonical bytes use RFC 8785 JCS without Unicode normalization.

Minimal three-Agent shape:

```json
{
  "version": 1,
  "claude": {
    "primary": {"model": "model-a"},
    "haiku": {"inherit_primary": true},
    "sonnet": {"inherit_primary": true},
    "opus": {"inherit_primary": true}
  },
  "opencode": {
    "default_model": "model-a",
    "models": {"model-a": {}}
  },
  "codex": {"model": "model-a"}
}
```

Use `--model-config=<path>` with `agent print-config` or `agent write-config` to
provide this document instead of answering model-setting prompts. The file must
be a regular, non-link JSON file no larger than 2 MiB. It must not contain an
API key, URL, provider identity, headers, fetched catalog, or arbitrary Agent
configuration.

### Claude Code

`primary` is required. `haiku`, `sonnet`, and `opus` each either inherit the
primary selection or select another catalog model. Optional `fable`, when
present, uses the same inherited or explicit-selection union. Its absence means
Fable is disabled and unmanaged; decoders and clients do not synthesize it.
Every explicit selection may have an optional display `name` and optional
`context`. Omission of `context` means Claude's standard/default behavior; the
only accepted value is the exact string `"1m"`. The canonical `model` is always
the authenticated base ID and must not end in `[1m]`. An inherited role contains
only `{"inherit_primary":true}` and inherits model, name, and context together.
For example, enabled Fable is either:

```json
{"inherit_primary": true}
```

or an explicit selection:

```json
{"model": "model-a", "name": "Optional display name", "context": "1m"}
```

Fable has no description key. Its explicit model must be in the authenticated
catalog, and explicit Fable `context: "1m"` conflicts with numeric
`context_window` just like the other explicit Claude selections.
`extra` is a string map limited to these description keys:

- `ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION`
- `ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION`
- `ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION`
- `ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION`

The Claude section may also contain independently optional `context_window` and
`max_output_tokens` fields. Each is a positive safe integer. When both are
present, `max_output_tokens` must be less than `context_window`.
`context_window` conflicts with `context: "1m"` on any explicit primary or role
selection; use either the numeric global budget or the selection-level `[1m]`
mechanism, not both. The manager renders configured values as exact base-10
decimal strings in `CLAUDE_CODE_MAX_CONTEXT_TOKENS` and
`CLAUDE_CODE_MAX_OUTPUT_TOKENS`, respectively.

The manager owns only its documented `env` keys, merges them into the existing
`env`, and preserves unrelated top-level and environment values. This ownership
includes both numeric-budget environment keys: omitting a previously managed
field removes its stale value. Existing files are projected back only when
these values are canonical positive decimal strings and the resulting Claude
section satisfies the same budget and context rules.

For standard context, the manager renders the base ID unchanged. For
`context: "1m"`, it appends exactly one terminal `[1m]` only at the Claude file
rendering boundary for `ANTHROPIC_MODEL`, `ANTHROPIC_CUSTOM_MODEL_OPTION`, and
the effective Haiku, Sonnet, Opus, and enabled Fable model values. Display names
are unchanged, and the manager does not write
`CLAUDE_CODE_DISABLE_1M_CONTEXT`. Existing values
with one exact terminal `[1m]` are projected back to base ID plus canonical
context; malformed, repeated, or middle markers are not repaired. Catalog and
write-time availability checks always use the base ID. Configuration does not
infer whether a model supports 1M context; a runtime rejection causes no model
fallback or configuration rewrite.

For enabled Fable, the manager always renders
`ANTHROPIC_DEFAULT_FABLE_MODEL` and renders
`ANTHROPIC_DEFAULT_FABLE_MODEL_NAME` only when the effective inherited or
explicit selection has a name. The Fable model key is also the projection
enablement signal: a name key by itself does not enable or project Fable. An
existing enabled Fable selection projects to inheritance only when its model,
name, and context exactly match primary; otherwise it remains explicit.
Malformed or numeric-context-conflicting enabled Fable makes the complete
existing Claude section unavailable. Fable remains optional for
`configured=true`, so legacy Claude files do not become incomplete.

Claude is the atomic availability and initialization unit. If an enabled Fable
model in a preset or projected existing configuration is unavailable or
invalid, the complete Claude section is unavailable; Fable is never dropped,
repaired, substituted, or merged independently. OpenCode and Codex sections
remain independently usable. Likewise, `existing > preset > empty` chooses one
complete Claude section: preset Fable is not deep-merged into an existing Claude
section that omits it.

Fable ownership is conditional and path-exact. While enabled, the manager owns
the rendered Fable model path and the name path when rendered. While absent, it
claims neither path and preserves never-owned manual Fable values. If the prior
sidecar proves a Fable path was manager-owned, disabling Fable removes that
stale path in the same recoverable transaction while preserving unrelated
environment values. Enabling Fable over an existing unowned value is a
collision/drift condition that requires preview-bound approval; it is never
silently overwritten. The sidecar stores the complete canonical Claude section
and records current Fable paths only while they are owned.

The enabled Fable alias requires Claude Code 2.1.170 or later. Older Claude Code
versions may ignore `ANTHROPIC_DEFAULT_FABLE_MODEL`; disable Fable or update
Claude Code rather than expecting manager fallback. This alias requirement is
separate from numeric context compatibility: the numeric `context_window`
override works directly for unknown custom model names on Claude Code 2.1.193
and later. Older versions remain supported and may ignore that numeric override;
there is no hard minimum Claude Code version when Fable is disabled. These
numeric values control Claude Code's local token budgeting and compaction
behavior. They do not enlarge, prove, or otherwise change the upstream model's
actual context or output capability.

### opencode

`models` contains one or more explicitly selected catalog IDs and
`default_model` names one of them. Per-model typed options include display name,
reasoning, attachments, tool calls, temperature, context/input/output limits,
input/output modalities, interleaved reasoning, and a constrained provider
`options` object. A model may also contain typed top-level `variants`. Variant
names are extensible but must be nonempty, contain no control characters, and
fit within 128 UTF-8 bytes; each name maps to a recursively bounded provider
option object that is subject to the same protected credential and connection
path checks. Legacy `extra.variants` input remains accepted, but defining
`variants` in both locations is a field conflict and is rejected. `extra`
otherwise accepts only fields valid under the pinned opencode schema and cannot
collide with typed or manager-owned paths.

Only display `name` defaults to the model ID. Every other optional field that is
unset is omitted, not guessed. The manager owns `provider.mtls-router` and an
owned root `model`, while preserving other providers, `small_model`, and
unrelated root settings. Typed variants render exactly at
`provider.mtls-router.models.<id>.variants` and are projected from that managed
top-level model field during discovery; arbitrary provider extras are not
extracted into canonical configuration. JSONC normalization can remove comments
and formatting and is shown in preview.

### Codex

`model` selects one catalog ID. Optional typed fields are `reasoning_effort`,
`reasoning_summary`, `verbosity`, `context_window`, and
`auto_compact_token_limit`. The only v1 `extra` key is
`model_auto_compact_token_limit_scope`, with `total` or `body_after_prefix`.
Unset optional keys are omitted or, when previously manager-owned, removed.

The dedicated provider is `model_providers.mtls-router`, uses `wire_api =
"responses"`, and uses the trusted loopback `/v1` URL. Codex CLI and IDE share
authentication. Preview separately requires approval before switching to
`cli_auth_credentials_store = "file"` and official `auth_mode = "apikey"` plus
`OPENAI_API_KEY` file authentication. OS keyring credentials are not deleted.

## Detection, Failures, and Refresh

`agent.detect` is key-free. `configured=true` means the local managed structure
is complete and internally consistent. It does not mean a model is currently
visible or authorized. Only successful `agent.models` discovery and write-time
refresh establish current authorization; no verified timestamp is persisted.

Discovery and write fail closed. `MODEL_AUTH_FAILED`,
`MODEL_DISCOVERY_FAILED`, `MODEL_RESPONSE_INVALID`, `MODEL_CATALOG_EMPTY`,
`MODEL_CATALOG_STALE`, `MODEL_CONFIG_INVALID`, `MODEL_NOT_AVAILABLE`,
`MANAGED_CONFIG_DRIFT`, `MODEL_STATE_INVALID`, `AGENT_OPERATION_BUSY`, and
`CODEX_AUTH_UNSUPPORTED` never trigger static models, cached catalogs, implicit
existing-model reuse, model substitution, or partial file changes. See
[Troubleshooting](TROUBLESHOOTING.md#model-configuration-errors) for actions.
If a selected ID disappears from the filtered catalog during write-time
refresh, write fails with `MODEL_NOT_AVAILABLE`; if refresh produces no retained
IDs at all, the earlier catalog fetch fails with `MODEL_CATALOG_EMPTY`.
Preset model unavailability is the exception only to discovery failure: it is
reported in `preset.unavailable_agents`, the unavailable complete preset section
is omitted, and discovery continues with existing configuration and other valid
preset sections. It still never causes substitution or partial preset use.

## Managed Configuration Cleanup

The desktop can clean exactly one Claude Code, OpenCode, or Codex configuration when `agent.detect` reports a trusted last-applied sidecar entry with `cleanup.managed=true` and `cleanup.available=true`. Cleanup is key-free and independent of router trust, model discovery, catalog tokens, canonical model configuration, and desktop model flows. The overview does not show the action for ordinary `not_managed` Agents. Invalid sidecar/signing state reports `model_state_invalid`; unresolved recovery or disabled writes reports `writes_disabled`. Base Agent file detection remains available in both cases.

Cleanup never infers ownership from provider names alone. It validates the sidecar, loads the saved Agent model section, reads only its recorded absolute file paths, and narrows older broad ownership records against the saved configuration and current structure:

- **Claude Code:** remove only recorded `env.*` paths. Remove the root `env` object if it becomes empty.
- **opencode:** remove `provider.mtls-router` after validating its object shape. Remove root `model` only when the sidecar owns it and its current string begins with exact ASCII `mtls-router/`. A preserved user default model is not newly claimed by later normal writes.
- **Codex:** from `config.toml`, remove `model_providers.mtls-router`, required `model_provider`, `model`, and `cli_auth_credentials_store`, plus only optional roots represented by the saved Codex model config. From `auth.json`, remove `auth_mode` and `OPENAI_API_KEY`; preserve other auth metadata and do not delete OS-keyring credentials or reconstruct competing auth fields replaced by an earlier write. The config/auth pair remains one transaction.

Preview returns path names and file/state effects only, never current values or contents. It creates no persistent filesystem state: no file, directory, backup, transaction journal, or coordination lock. For managed state it only opens and validates the already existing private transaction lock; a missing or unsafe lock fails closed without creating one. A cleanup-specific HMAC token binds the Agent, each source/target path and keyed revision, `replace`/`delete` operation, required backup source, sorted removed paths, whole-file drift flag, and sidecar revision/operation. It deliberately carries no router, catalog, API-key, or canonical-model claims.

Whole-file drift requires `approve_managed_overwrite=true`; approval does not widen the managed path set. Any Agent file or sidecar revision change after preview returns `PREVIEW_STALE`. Write first creates and verifies a private sibling backup for every existing Agent file and the sidecar, records a delete-capable journal v3, applies Agent files in order, and updates or deletes the sidecar last. A semantic empty JSON/TOML root becomes a `delete`; otherwise cleanup uses `replace`. Journal v3 records absent post-revisions for deletes, while startup recovery continues to decode legacy v1/v2 journals as replace-only. Rollback restores manager state first and then Agent files so ownership and files do not split.

Cleanup removes Agent-file authentication for the selected Agent but retains the desktop global credential and every historical or newly created backup. Backups may contain current or old keys. The stable cleanup failures are `INVALID_PARAMS`, `AGENT_NOT_MANAGED`, `MODEL_STATE_INVALID`, `CONFIG_INVALID`, `CONFIG_NOT_WRITABLE`, `PREVIEW_STALE`, and `MANAGED_CONFIG_DRIFT`, plus the existing `AGENT_OPERATION_BUSY`, `BACKUP_FAILED`, `WRITE_FAILED`, `ROLLBACK_FAILED`, and `OPERATION_TIMEOUT` transaction/protocol failures. `INVALID_PARAMS` covers unsupported Agents, malformed or duplicate/unknown request fields, and a missing or malformed revision token or required approval field. All fail without returning configuration values, key material, file content, URLs, or backup content.

## Destructive Rebuild Recovery

Normal `merge` parses the existing configuration, changes only manager-owned paths, and preserves supported unrelated data. `rebuild` does not parse or merge the malformed content: after a separate preview-bound approval, it replaces the complete approved Agent file set with freshly rendered managed-only files. **Rebuild discards all unrelated settings, comments, original formatting, and valid companion-file metadata. Review and protect every backup before proceeding.**

Rebuild is available only when detection finds at least one syntax-invalid file and every file in the complete managed set is otherwise safe: present targets must be readable, regular, non-linked, and writable, and each immediate parent must be an available writable directory. An unreadable, oversized, non-regular, linked, or non-writable file; an unavailable parent; an unsupported but syntactically valid structure; pending transaction recovery; or globally disabled writes blocks rebuild. Detection is advisory: preview and write repeat eligibility and exact path, format, existence, and revision checks under the transaction lock.

Valid syntax is never a reason to rebuild. A valid but unsupported structure must be repaired or migrated deliberately, and a parser compatibility problem must be fixed in the parser. For example, accepted BOM-prefixed JSON and quoted TOML keys stay on the preservation-merge path; they must not be discarded as recovery input.

The complete managed-only output is:

- **Claude Code:** one `settings.json` root object containing only `env`. The unconditional keys are `ANTHROPIC_BASE_URL`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_MODEL`, `ANTHROPIC_CUSTOM_MODEL_OPTION`, the Haiku/Sonnet/Opus `ANTHROPIC_DEFAULT_*_MODEL` keys, `ENABLE_TOOL_SEARCH`, and `DISABLE_AUTOUPDATER`. Only selected display-name keys (`ANTHROPIC_CUSTOM_MODEL_OPTION_NAME` and the `ANTHROPIC_DEFAULT_*_MODEL_NAME` keys), Fable model/name keys, `CLAUDE_CODE_MAX_CONTEXT_TOKENS`, `CLAUDE_CODE_MAX_OUTPUT_TOKENS`, and allowed description extras are added.
- **opencode:** one strict JSON root object containing only `model` and `provider.mtls-router`. The provider contains exactly `npm`, `name`, `options`, and `models`; `npm` is `"@ai-sdk/openai-compatible"`, `name` is `"mtls-router"`, `options` contains only `baseURL` and `apiKey`, and `models` contains the exact selected definitions. An approved `.jsonc` path is replaced in place as strict JSON; rebuild never performs the normal sibling `opencode.json` migration.
- **Codex:** the complete set is both `config.toml` and `auth.json`, even when only one is malformed. `config.toml` contains only `model_provider = "mtls-router"`, the selected `model`, `cli_auth_credentials_store = "file"`, selected optional model settings, and `model_providers.mtls-router` with exactly `name`, `wire_api = "responses"`, `requires_openai_auth = true`, and `base_url`. `auth.json` contains exactly `auth_mode: "apikey"` and `OPENAI_API_KEY`. A missing companion is created; every existing companion is replaced, so valid companion metadata is discarded.

Preview creates no file, directory, journal, or backup. It shows redacted managed fragments, each exact affected path, whether the operation creates or replaces it, and a planned sibling backup pattern. Write requires the signed preview revision plus an exact `approve_rebuild` set; omitted modes default to merge, and missing, extra, or duplicate rebuild approvals are rejected. There is no automatic recovery, parse bypass, `force`, global overwrite, or fallback from failed merge to rebuild.

Before any replacement, write creates a private sibling backup for every existing file in the approved managed set, syncs it, reopens it, and verifies byte-for-byte equality. Actual backup paths appear in a successful write result; preview shows patterns only and never backup contents. A failed operation may report only its error even when diagnostic artifacts remain. Backups can contain current or old API keys and other credentials. Keep them private and do not attach them unredacted. Never manually restore while a transaction journal or unresolved recovery exists; stop and ask the maintainer because changing a target can prevent recovery from proving its identity. After recovery is resolved, stop the owning Agent, preserve current files, verify that each original path and parent still have the expected current-user, non-link identity, and use a private same-directory temporary file plus atomic replacement rather than copying through a link. For Codex, restore each file that existed before the transaction and remove a companion only when the preview/result proves that transaction created it; otherwise stop and ask the maintainer.

Any changed file, repaired syntax, new blocker, catalog change, or stale revision before replacement rejects the write and requires a fresh detection and preview. A backup failure changes no target. A later write failure rolls back every changed Agent file and manager sidecar together and retains diagnostic backups. If rollback or startup recovery cannot prove restoration, writes are disabled and rebuild remains unavailable until recovery is resolved; do not delete the journal or backups while investigating.

## Ownership, Migration, and Backups

The manager records only canonical selected model sections, owned paths, target
paths, and keyed revision MACs in the private
`agent-transactions/last-applied-model-config.json` sidecar. It stores no key,
catalog, rendered file, raw response, or unrelated Agent setting. An OS-backed
per-user lock serializes desktop and CLI operations.

The injected preset is build metadata and discovery alone never writes it to an
Agent file, last-applied sidecar, journal, revision claim, backup, log, or
diagnostic. Only the exact canonical configuration the user approves through
the normal preview/write flow can enter Agent files and transactional state.

Known manager-owned paths can be updated or removed. Normal merge preserves
unrelated settings; explicitly approved rebuild follows the destructive contract
above. Unknown extension collisions are rejected; drift in a managed
namespace requires preview-bound overwrite approval. A write rechecks Agent
files, sidecar revision, router identity, and current model availability before
creating any write artifact.

Exact historical v1 signatures can migrate: Claude changes from whole-`env`
replacement to managed-key merge, opencode changes from fixed models to the
selected catalog subset, and Codex changes from `custom` to `mtls-router` with a
separately approved auth migration. Partial or modified historical signatures
are not claimed. Existing backups are never deleted or rewritten by migration.

Existing files and the sidecar are backed up and changed in one journaled
transaction. Backups stay beside their source where applicable, use private
permissions, and may contain current or old API keys. Treat them as sensitive.
Rollback restores files and sidecar together; do not remove transaction state
or backups while recovery is unresolved.

## Protocol v4 Automation

Automation must use a receipt-verified `mtls-router-manager serve`. First call
`manager.info` and require management protocol `4`. Then call:

1. `agent.models` with `owner`, `agents`, and transient `api_key`.
2. `agent.render` for key-redacted managed fragments, or `agent.preview` with `agents`, `catalog_token`, `model_config`, and a per-Agent `modes` map when requesting rebuild.
3. `agent.write` with those fields, the same `modes`, the preview `revision_token`, both explicit approval booleans, an `approve_rebuild` array exactly matching rebuild-mode Agents, and transient `api_key`.

Cleanup automation is a separate two-call sequence: call `agent.cleanup.preview` with exactly one `agent`, then call `agent.cleanup.write` with that Agent, the cleanup `revision_token`, and explicit `approve_managed_overwrite`. These requests reject API keys, Agent arrays, catalog/model configuration, flow IDs, and unknown fields. Cleanup write is non-replayable after ambiguous delivery; rediscover the cleanup state and generate a new preview instead of resending an uncertain write.

`agent.models` always includes stable preset objects, including when no preset
or no valid requested section exists:

```json
{
  "preset": {
    "model_config": {},
    "unavailable_agents": {}
  }
}
```

`model_config` is a versioned canonical document containing only complete valid
requested sections. An unavailable entry is
`{"code":"MODEL_NOT_AVAILABLE","models":["missing-base-id"]}`. Both fields
are objects, never `null` or omitted; this metadata contains no key and does not
turn otherwise successful discovery into a failure.

The key belongs only in the two secret-bearing stdin/IPC request bodies. Never
put it in arguments, environment variables, model config, logs, shell history,
or temporary request files. Protocol v1-v3 requests and mixed v3/v4 router,
manager, setup receipt, or desktop artifacts are rejected; update the complete
release together.

Exact request/result schemas and stable errors are defined by the checked-in
canonical JSON Schema and manager protocol types. The tested Agent revisions
and source digests are recorded in
[`internal/manager/agent/testdata/compatibility.json`](../internal/manager/agent/testdata/compatibility.json).
