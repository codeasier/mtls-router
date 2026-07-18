# Agent Model Configuration

[中文](zh-CN/AGENT_MODELS.md)

This is the user and automation contract for management protocol v2. The Go
manager is authoritative when this guide and a client-side validation differ.

## Service Contract

The authenticated upstream `GET /v1/models` response is the only candidate
model source. It returns one complete OpenAI-compatible `data[].id` catalog for
the supplied Bearer key. Every returned ID is supported, without name filtering
or inference probing, on all required routes:

| Client | Route | Contract |
|---|---|---|
| Catalog | `GET /v1/models` | One bounded standard JSON response |
| Claude Code | `POST /v1/messages`, including `?beta=true` | Anthropic request fields and open-list `anthropic-*` headers pass through; SSE is unbuffered |
| Claude Code | `POST /v1/messages/count_tokens` | Exact token counting |
| opencode | `POST /v1/chat/completions` | OpenAI Chat Completions and streaming |
| Compatibility clients | `POST /v1/completions` | OpenAI Completions and optional streaming |
| Codex | `POST /v1/responses` | OpenAI Responses and SSE streaming |

Claude deferred tool fields and future open-list Anthropic request fields pass
through unchanged. Configuration never calls an inference endpoint. The router
preserves authorization but does not select models or store keys.

## Interactive Flow

The Shell, PowerShell, and desktop flows all use this order:

1. Detect and select Claude Code, opencode, and/or Codex.
2. Read the API key without echo and establish a trusted protocol-v2 loopback router.
3. Call `agent.models`; only then show the common sorted catalog.
4. Require explicit Agent-native model choices. No first model or name-based preference is selected automatically.
5. Render a redacted fragment for print, or preview exact file, backup, migration, ownership, and drift effects for write.
6. Re-fetch the catalog with the transient key immediately before writing, then perform one atomic multi-file transaction.

`agent print-config` also requires a key because it validates choices against the
current catalog. It changes no Agent file, transaction journal, backup, or
last-applied sidecar. Discovery may start the router and first use may create the
private token-signing key.

The catalog is a configuration-time snapshot. There is no background refresh or
Agent-file rewrite. Re-enter configuration and supply a key to refresh it.

## Canonical Model Config

All clients use one key-free JSON document. `version` is `1`; the `agents`
request array and present top-level sections must match exactly. Unknown fields
are rejected except in the constrained `extra` and `options` objects. Input is
strict JSON: duplicate keys, invalid UTF-8, non-finite numbers, unsafe integer
ranges, and protected credential/connection paths are rejected. Canonical bytes
use RFC 8785 JCS without Unicode normalization.

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
primary model or select another catalog model. Every explicit model may have an
optional display `name`. `extra` is a string map limited to these description
keys:

- `ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION`
- `ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION`
- `ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION`
- `ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION`

The manager owns only its documented `env` keys, merges them into the existing
`env`, and preserves unrelated top-level and environment values.

### opencode

`models` contains one or more explicitly selected catalog IDs and
`default_model` names one of them. Per-model typed options include display name,
reasoning, attachments, tool calls, temperature, context/input/output limits,
input/output modalities, interleaved reasoning, and a constrained provider
`options` object. `extra` accepts only fields valid under the pinned opencode
schema and cannot collide with typed or manager-owned paths.

Only display `name` defaults to the model ID. Every other optional field that is
unset is omitted, not guessed. The manager owns `provider.mtls-router` and an
owned root `model`, while preserving other providers, `small_model`, and
unrelated root settings. JSONC normalization can remove comments and formatting
and is shown in preview.

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

## Ownership, Migration, and Backups

The manager records only canonical selected model sections, owned paths, target
paths, and keyed revision MACs in the private
`agent-transactions/last-applied-model-config.json` sidecar. It stores no key,
catalog, rendered file, raw response, or unrelated Agent setting. An OS-backed
per-user lock serializes desktop and CLI operations.

Known manager-owned paths can be updated or removed. Unrelated settings are
preserved. Unknown extension collisions are rejected; drift in a managed
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

## Protocol v2 Automation

Automation must use a receipt-verified `mtls-router-manager serve`. First call
`manager.info` and require management protocol `2`. Then call:

1. `agent.models` with `owner`, `agents`, and transient `api_key`.
2. `agent.render` for key-redacted managed fragments, or `agent.preview` with `agents`, `catalog_token`, and `model_config`.
3. `agent.write` with those fields, the preview `revision_token`, both explicit approval booleans, and transient `api_key`.

The key belongs only in the two secret-bearing stdin/IPC request bodies. Never
put it in arguments, environment variables, model config, logs, shell history,
or temporary request files. Protocol v1 requests and mixed v1/v2 router,
manager, setup receipt, or desktop artifacts are rejected; update the complete
release together.

Exact request/result schemas and stable errors are defined by the checked-in
canonical JSON Schema and manager protocol types. The tested Agent revisions
and source digests are recorded in
[`internal/manager/agent/testdata/compatibility.json`](../internal/manager/agent/testdata/compatibility.json).
