# Agent Model Configuration Specification

## Change ID

`agent-models-config`

## Status

Proposed. Execution requires explicit approval of this specification package.

## Decision Source

This specification formalizes the model-configuration decisions approved in
the `understand-me` workflow on 2026-07-18.

This package supersedes the static-model and key-after-preview behavior in
FR-2, FR-8 through FR-11, and FR-15 of
`specs/tauri-desktop-app/spec.md`. Unrelated desktop, router lifecycle,
transaction, packaging, and security requirements remain in force.

The exact supersession matrix is:

| Existing item | v2 replacement |
|---|---|
| Desktop spec FR-2 Agent method list/key handling/deadlines | This spec Protocol v2, FR-1 through FR-6, and FR-15 |
| Desktop spec FR-8 `configured` semantics | This spec FR-11 |
| Desktop spec FR-9 preview without model catalog/config | This spec FR-4 and FR-5 |
| Desktop spec FR-10 key-after-preview and static write request | This spec FR-1, FR-6, FR-7, FR-13, and FR-16 |
| Desktop spec FR-11 whole Claude env, static opencode models, Codex custom provider, one-key auth file | This spec FR-8 through FR-10 |
| Desktop spec FR-15 old stdin automation | This spec FR-12 and Protocol Contract |
| Desktop tasks 1.1 method/deadline assertions, 1.6 configured semantics, 1.7, 1.8, 1.10, 2.3, 2.4, 3.2 through 3.4, and 4.5 only where they encode the rows above | Corresponding dependency tasks in this package |
| Desktop checklist lines 33-36, 94-98, 152-175, 200-215, and any key-after-preview stage assertions | Corresponding acceptance sections in this package |

Previously checked evidence for superseded assertions is historical only and
does not satisfy v2 acceptance. Implementation documentation must mark these
rows as superseded instead of presenting both behaviors as current.

## Motivation

The manager currently writes hard-coded model IDs for Claude Code and Codex,
and a hard-coded two-model opencode catalog with guessed metadata. The upstream
service already exposes an authenticated OpenAI-compatible `GET /v1/models`
endpoint. After a user supplies an API key, that endpoint is the authority for
the model IDs available to that key.

Every returned model is guaranteed by the service to support all endpoints
needed by the supported Agents, including Messages, Responses, and
Completions. Users therefore need one real candidate catalog, followed by
explicit Agent-native selection and configuration under the existing preview,
backup, atomic-write, rollback, and secret-handling boundaries.

## Goals

- Discover the authenticated model catalog through the trusted local router.
- Treat every returned model as compatible with every supported Agent without
  name filtering or inference probes.
- Configure each Agent according to its native model semantics.
- Support common settings through typed fields and additional safe model
  settings through constrained `extra` objects.
- Use one canonical model-config document across Go, Shell, PowerShell, Rust,
  TypeScript, preview, write, and automation.
- Produce an exact, API-key-redacted preview before writing.
- Revalidate selected models immediately before writing.
- Preserve unrelated user configuration and own only mtls-router settings.
- Provide no credential-designated field in model-config and never copy the
  transient `api_key` parameter into tokens, manager state, logs, diagnostics,
  or source files.
- Support `setup.sh`, `setup.ps1`, and the Tauri desktop in the same release.
- Upgrade the management protocol to v2 and reject mixed-version artifacts.

## Non-Goals

- Supporting model IDs absent from the current authenticated catalog.
- Inferring capabilities, limits, modalities, or compatibility from model IDs.
- Issuing Messages, Responses, or Completions requests during configuration.
- Depending on non-standard fields returned by `GET /v1/models`.
- Persisting the candidate catalog or raw models response.
- Reading an API key back from an existing Agent file.
- Continuously synchronizing Agent files in the background.
- Enabling Agent-specific runtime model discovery.
- Exposing router URLs, authentication headers, provider identity, transport
  retries, or other manager-owned connection settings in model config.
- Accepting arbitrary Agent JSON or TOML fragments.
- Adding Codex profiles or a synthetic Codex model catalog.
- Supporting management protocol v1 and v2 Agent behavior in parallel.
- Supporting paginated or provider-specific model-list response formats.

## Service Contract

The fixed upstream service contract is:

1. `GET /v1/models` accepts `Authorization: Bearer <api-key>`.
2. A successful response is an OpenAI-compatible object containing `data`, an
   array of objects with string `id` fields.
3. The response contains the complete catalog for that key in one response.
4. The same Bearer key authenticates inference requests for every returned
   model.
5. Every returned model supports the following exact client paths and request
   semantics:

| Client | Method and path | Required behavior |
|---|---|---|
| Catalog | `GET /v1/models` | Standard bounded JSON catalog |
| Claude Code | `POST /v1/messages` including `?beta=true` | Anthropic Messages request fields and `anthropic-version`/`anthropic-beta` headers preserved; SSE streamed without buffering |
| Claude Code | `POST /v1/messages/count_tokens` | Supported for exact token counting |
| opencode | `POST /v1/chat/completions` | OpenAI-compatible Chat Completions and streaming |
| Compatibility clients | `POST /v1/completions` | OpenAI-compatible Completions and streaming where requested |
| Codex | `POST /v1/responses` | OpenAI Responses and SSE streaming |

6. Claude tool schemas, including deferred tool fields used when tool search is
   enabled, and all future open-list `anthropic-*` request headers/body fields
   pass through unchanged.
7. Model visibility is sufficient for cross-Agent compatibility. Clients do
   not filter models by name and do not probe inference endpoints.

Local integration fixtures protect this exact matrix with one advertised
model. Production configuration does not issue inference requests.

## Supported Agent Surfaces

- Claude settings target Claude Code CLI, IDE integrations, and local Desktop
  Code sessions that consume the user's Claude Code settings. They do not
  configure Cowork or remote/cloud Claude sessions.
- opencode settings target current stable CLI and desktop builds that consume
  `opencode.json`/`opencode.jsonc`.
- Codex settings target current stable local CLI and IDE surfaces that consume
  `CODEX_HOME`; host-provided ephemeral authentication is outside this file
  workflow.

Implementation must check in a compatibility manifest pinning the exact
Claude Code, opencode schema revision, and Codex schema/source revision used by
tests. Release CI validates generated files with those current stable parsers
or binaries. Older versions are not implicitly supported.

## Architecture

### Router data plane

`mtls-router` remains a transparent data plane. It forwards `/v1/models` and
Agent inference paths to the fixed mTLS upstream while preserving the
`Authorization` header. It does not implement model selection or store API
keys. Access logs continue to exclude headers, queries, and bodies.

### Go manager control plane

The manager is the sole owner of:

- Trusted-router validation and safe automatic startup.
- Authenticated model fetching and normalization.
- Catalog-token creation and validation.
- Canonical model-config validation and normalization.
- Existing-config and last-applied-state inspection.
- Claude Code JSON, opencode JSON/JSONC, and Codex TOML/JSON rendering.
- Redacted render and preview results.
- Drift detection, revision tokens, backups, atomic writes, and rollback.

Setup scripts and desktop clients collect input and display manager results.
They do not construct Agent configuration snippets.

### Management protocol v2

`internal/version.ManagementProtocolVersion` changes from `1` to `2`. Router,
manager, setup receipts, release metadata, desktop expected values, `/version`,
and `manager.info` must agree on v2. Existing identity rules reject mixed v1/v2
deployments.

Protocol v2 retains line-delimited JSON and adds:

- `agent.models`
- `agent.render`

It changes required parameters and results for `agent.preview` and
`agent.write`. There is no protocol v1 fallback.

### Canonical model config

All clients and manager methods use one versioned JSON document. It contains no
API key, router address, provider identity, or fetched catalog:

```json
{
  "version": 1,
  "claude": {},
  "opencode": {},
  "codex": {}
}
```

The request's `agents` array is authoritative. Each selected Agent must have
exactly one corresponding section. A section for an unselected Agent or a
missing selected section is `MODEL_CONFIG_INVALID`; it is never ignored.

Unknown fields are rejected except inside documented `extra` and `options`
objects. JSON input must be valid UTF-8 and contain no duplicate object keys.
Canonical form is RFC 8785 JSON Canonicalization Scheme (JCS), with no Unicode
normalization. Typed integers are limited to `1..9007199254740991`; extension
numbers must be finite IEEE-754 values exactly representable by JCS. Catalog
ordering is lexicographic by raw UTF-8 bytes. Checked-in cross-language vectors
must produce identical bytes in Go, Rust, TypeScript, jq, and PowerShell flows.

## Canonical Schema

### Example

The IDs below demonstrate fields and are not defaults:

```json
{
  "version": 1,
  "claude": {
    "primary": {"model": "model-a", "name": "Primary model"},
    "haiku": {"inherit_primary": true},
    "sonnet": {"model": "model-b", "name": "Sonnet workload model"},
    "opus": {"inherit_primary": true},
    "extra": {
      "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION": "Long context"
    }
  },
  "opencode": {
    "default_model": "model-a",
    "models": {
      "model-a": {
        "name": "Model A",
        "reasoning": true,
        "attachment": true,
        "tool_call": true,
        "temperature": true,
        "limit": {"context": 272000, "input": 244800, "output": 27200},
        "modalities": {"input": ["text", "image"], "output": ["text"]},
        "interleaved": {"field": "reasoning_details"},
        "options": {"reasoningEffort": "medium"},
        "extra": {"status": "active"}
      },
      "model-b": {"name": "Model B"}
    }
  },
  "codex": {
    "model": "model-a",
    "reasoning_effort": "medium",
    "reasoning_summary": "auto",
    "verbosity": "medium",
    "context_window": 272000,
    "auto_compact_token_limit": 244800,
    "extra": {"model_auto_compact_token_limit_scope": "body_after_prefix"}
  }
}
```

### Claude Code section

`claude.primary` is required and contains a required catalog `model` plus an
optional non-empty display `name`.

`claude.haiku`, `claude.sonnet`, and `claude.opus` are required. Each is
exactly one of:

```json
{"inherit_primary": true}
```

or:

```json
{"model": "catalog-model-id", "name": "optional display name"}
```

The UI initializes all three roles to inherit primary. Inheritance remains in
the canonical document and is resolved during rendering.

`claude.extra` is an optional string map. Its exact v1 allowlist is:

- `ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION`
- `ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION`
- `ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION`
- `ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION`

Model-ID, name, authentication, connection, custom-header, and supported-
capability variables are not extensions. Selection/name fields use typed
schema fields; supported-capability declarations do not affect an
`ANTHROPIC_BASE_URL` gateway and are therefore not exposed.

### opencode section

`opencode.models` is a required object with at least one model. Its keys are
catalog model IDs. `opencode.default_model` is required and must name one of
those keys.

Each model supports these optional typed fields:

- `name`: non-empty display name; omitted means the model ID.
- `reasoning`, `attachment`, `tool_call`, and `temperature`: booleans.
- `limit`: required positive integer `context` and `output`, with optional
  positive integer `input`; `input` must not exceed `context`.
- `modalities`: optional `input` and `output` arrays containing unique members
  of `text`, `audio`, `image`, `video`, and `pdf`.
- `interleaved`: `true` or an object whose `field` is `reasoning`,
  `reasoning_content`, or `reasoning_details`.
- `options`: provider model-options object. At every nesting depth, key names
  are normalized by lowercasing and removing `_`, `-`, and `.`; names
  containing credentials, auth, token, secret, password, bearer, header, URL,
  endpoint, provider, transport, proxy, or fetch semantics are rejected.
- `extra`: object deep-merged into the model entry.

Model `extra` cannot contain `id`, `name`, `reasoning`, `attachment`,
`tool_call`, `temperature`, `limit`, `modalities`, `interleaved`, or `options`,
which are typed fields. It cannot contain `headers` or `provider`, which remain
manager-owned. Native fields such as `family`, `release_date`, `cost`,
`status`, `experimental`, or `variants` are allowed only when they satisfy the
checked-in pinned opencode schema snapshot. The snapshot source URL, revision
or digest, and retrieval date are recorded in the compatibility manifest.

### Codex section

`codex.model` is required and is a catalog model ID. Optional typed fields map
to Codex root keys:

| Canonical field | Codex key | Accepted values |
|---|---|---|
| `reasoning_effort` | `model_reasoning_effort` | non-empty lowercase token up to 64 bytes; UI offers pinned parser values `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, `ultra` |
| `reasoning_summary` | `model_reasoning_summary` | `auto`, `concise`, `detailed`, `none` |
| `verbosity` | `model_verbosity` | `low`, `medium`, `high` |
| `context_window` | `model_context_window` | positive integer |
| `auto_compact_token_limit` | `model_auto_compact_token_limit` | positive integer, not above `context_window` when both exist |

`codex.extra` is not an open TOML map. Each key must exist in the pinned current
Codex `ConfigToml` schema, be model-behavior related, and not be a typed or
manager-owned key. For the v1 canonical schema, the allowed extension is
`model_auto_compact_token_limit_scope` with `total` or `body_after_prefix`.
Expanding this allowlist requires a canonical-schema/compatibility-manifest
update and parser tests; unknown `model_*` keys are rejected.

### Shared extension rules

- Values are JSON-compatible and finite; `null` is rejected.
- Objects deep-merge recursively; arrays and scalars are complete leaf values.
- Typed or manager-owned path collisions are errors, never overrides.
- Nesting is limited to 16, keys to 128 UTF-8 bytes, strings to 16 KiB, and
  arrays to 1024 elements.
- Secret/connection-like keys are rejected recursively after the normalized-key
  transform defined for opencode options. This includes credential, auth,
  token, secret, password, bearer, header, URL, endpoint, provider, transport,
  proxy, and fetch variants.
- Control characters are rejected in IDs, names, object keys, and TOML keys.
- JSON strings are escaped; TOML values use complete TOML encoding rather than
  string interpolation.

The canonical schema has no credential-designated field. The manager never
copies the transient key into canonical config/state and, at write, rejects a
canonical string exactly equal to that key as defense in depth. Arbitrary
display/option values are user input and must not contain secrets; the product
does not claim semantic detection of every possible credential string.

## Functional Requirements

### FR-1: Trusted model discovery

`agent.models` accepts:

```json
{
  "owner": "cli",
  "agents": ["claude", "opencode", "codex"],
  "api_key": "transient secret"
}
```

`owner` is `cli` for setup scripts and direct CLI automation, and `desktop` for
the Tauri client. It controls ownership only when a router must be started.

Before sending the key, the manager must establish a trusted local router:

1. Run existing router discovery and identity validation.
2. Reuse only a desktop-owned or setup-owned router whose process identity,
   deployment ID, and management protocol v2 identity are valid.
3. If no router runs, start the verified bundled or installed pair under the
   requested owner and validate the new process before sending the key.
4. If an unknown, stale, or identity-mismatched process occupies the port, fail
   without sending the key, starting a second router, or signaling the process.
5. Require an actual loopback IP listener. Never resolve or send the key to a
   non-loopback host.

`owner=desktop` is valid only for a manager session started with a verified
desktop parent identity and desktop session ID. Other callers receive
`INVALID_PARAMS` before the key is used. A trusted degraded router may still
attempt `/v1/models`; the models request result is authoritative. Unknown,
stale, identity-mismatched, or latched unexpected-exit states do not auto-start.
A free port with absent state may start once; a user must explicitly resolve a
desktop unexpected-exit latch through the existing lifecycle controls.

The trusted router's actual listener is the manager's explicit `--listen`
value after canonical validation and exact state/endpoint verification. It must
be a numeric `127.0.0.0/8` or `::1` address with port `1..65535`; hostnames and
port zero are rejected. Setup and desktop continue to pass the default and
never auto-select a port; direct manager automation may use another valid
loopback address. This listener is the single source for model discovery and
rendered Agent base URLs. Claude receives `http://<listen>`;
opencode and Codex receive `http://<listen>/v1`. IPv4 and IPv6 URL syntax must
be normalized correctly. Fixed `127.0.0.1:19099` rendering is removed.

The manager issues a direct local `GET /v1/models` with:

- `Authorization: Bearer <api-key>`.
- No body or query string.
- No environment-configured proxy.
- Redirect following disabled.
- A five-second HTTP deadline within the method deadline.
- A 1 MiB response-body limit.

Validation and key transmission must be channel-bound. The manager opens one
direct TCP connection to the numeric loopback listener, validates `/version`
and deployment/process-state correlation, and sends `/v1/models` only over the
same keep-alive connection. The transport must not redial between validation
and the key-bearing request. If connection reuse cannot be proven, discovery
fails before key transmission.

Only HTTP 200 is success. HTTP 401 and 403 are `MODEL_AUTH_FAILED`. Redirects,
429, 5xx, connection failures, and other statuses are
`MODEL_DISCOVERY_FAILED`. Errors and logs never include response bodies,
headers, a redirect target, or the key.

#### Scenario: router is absent

Given a verified installation and free loopback port, when `agent.models` runs,
then the manager starts and validates the router under the requested owner and
only then sends the key to `/v1/models`. The router remains running under normal
lifecycle rules after discovery.

#### Scenario: unknown local process

Given an unknown process occupies the configured port, when discovery runs,
then the manager returns the existing safe identity/port error, sends no key,
starts no second router, and signals no process.

#### Scenario: redirect response

Given the trusted router returns a redirect for `/v1/models`, when discovery
runs, then the manager does not follow it and returns
`MODEL_DISCOVERY_FAILED` without disclosing the target or key.

### FR-2: Catalog parsing and token

A successful body must contain exactly one JSON value whose root is an object
and whose `data` field is an array. Every item must be an object with a
non-empty UTF-8 string `id`. Unknown root and item fields are ignored.

IDs are limited to 256 UTF-8 bytes and cannot contain controls or leading or
trailing Unicode whitespace. Invalid boundary whitespace is rejected rather
than trimmed, so the manager never invents an ID the service did not advertise.
The service contract requires every ID to be directly usable as a Claude model
string, an opencode provider model key/reference, and a Codex model string.
Identical IDs are deduplicated. The final catalog is sorted lexicographically
by UTF-8 bytes and limited to 1000 unique models. Malformed fields, trailing
JSON, oversized values, or excessive model count are
`MODEL_RESPONSE_INVALID`. No valid models is `MODEL_CATALOG_EMPTY`.

The result is:

```json
{
  "models": ["model-a", "model-b"],
  "catalog_token": "opaque-key-free-token",
  "router_base_url": "http://127.0.0.1:19099",
  "api_base_url": "http://127.0.0.1:19099/v1",
  "existing": {
    "model_config": {},
    "unavailable_models": {},
    "drifted_agents": []
  }
}
```

`existing` contains only model-related, key-free data safe for prefilling:

- If last-applied state exists and all recorded revisions match, return that
  canonical section.
- Otherwise parse only supported typed model fields from current Agent files,
  omit unknown/extra values, and mark the Agent drifted.
- Report configured IDs absent from the new catalog under
  `unavailable_models`; never select replacements.
- Never return stored keys, unrelated env/auth fields, headers, arbitrary file
  content, or raw models response fields.

The catalog token is a self-contained authenticated envelope that binds the
normalized model IDs, normalized selected-Agent set, requested owner, trusted
router address, deployment ID, management protocol version, canonicalization
version, and signing-key generation. It contains no copied API key and clients
treat it as opaque. It is verifiable across one-shot manager processes without
persisting the fetched catalog. Its encoded size is limited to 512 KiB. It has
no time expiry; write-time trusted-router and model refresh is the freshness
boundary.

The catalog token is a consistency mechanism, not a replacement for trusted
router validation or write-time model refresh. Invalid, incompatible, or
address-mismatched tokens are `MODEL_CATALOG_STALE`.

Catalog and revision authentication uses a random 256-bit per-user HMAC key at
`<agent-transactions>/token-signing-key.json`. The file contains schema version,
random key generation ID, and base64 key material; it is atomically created
with exclusive create semantics and user-private permissions by the first
`agent.models` call. Concurrent creators converge on one valid file. Public
deployment metadata or state paths are never the signing secret.

If the signing key is missing while no journal/sidecar exists, it can be
created. If it is malformed, permission-invalid, or missing while journal or
last-applied state exists, Agent model discovery/preview/write fails with
`MODEL_STATE_INVALID`; the manager never silently replaces the trust root or
claims existing ownership. Recovery guidance requires preserving Agent
backups, resolving any journal first, and moving the entire invalid
`agent-transactions` directory aside before starting a fresh ownership state.
Deleting or replacing the key invalidates every outstanding token.

#### Scenario: duplicate and unsorted IDs

Given repeated, unsorted model IDs, when discovery succeeds, then the manager
returns one lexicographically sorted occurrence and binds exactly that catalog
into the token.

#### Scenario: configured model disappeared

Given an existing Agent refers to an absent model, when discovery succeeds,
then the ID is reported unavailable, is not a valid prefill, and the user must
choose another model.

### FR-3: Agent-native model selection

Every selected model in canonical config must be a member of the token catalog:

- Claude primary is required. Haiku, Sonnet, and Opus inherit primary or select
  a catalog model explicitly.
- opencode selects one or more models and names one selected model as default.
- Codex selects one active model.

Clients may prefill existing IDs only while they remain available. They must
not select the first result, infer preferences from names, or copy a selection
between Agents without user action. All three Agents receive the same complete
catalog because all-endpoint compatibility is guaranteed.

### FR-4: Key-free managed render

`agent.render` accepts `agents`, `catalog_token`, and `model_config`. It
validates and canonicalizes config, verifies catalog membership, and returns
manager-rendered managed fragments with the API key represented by the fixed
placeholder `<redacted-api-key>`.

Render performs no merge-dependent file read, write, backup, state creation, or
revision-token creation. It implements `agent print-config`. Results contain
only manager-owned fragments, not complete merged user files, so unrelated
stored secrets cannot enter protocol output.

Each fragment includes Agent, target path, format, and escaped content.
User-controlled names and paths must not permit terminal escape injection.

### FR-5: Exact preview and revision binding

`agent.preview` accepts:

```json
{
  "agents": ["claude", "opencode", "codex"],
  "catalog_token": "...",
  "model_config": {}
}
```

Preview validates the catalog token and canonical config before reading target
files. It returns the existing file and backup plan plus:

- Canonical model config.
- API-key-redacted manager-owned fragments.
- Drift status and exact managed namespaces that will be replaced.
- A revision token bound to selected Agents, canonical config, catalog
  identity, source and target file revisions, sidecar revision, router address,
  and drift state.

Preview remains key-free and creates no directory, backup, sidecar, journal,
or temporary file.

All operations over shared Agent transaction state use an OS-backed per-user
lock in `agent-transactions`. Startup recovery acquires it before journal
inspection. Discovery prefill, render token verification, and preview hold it
for their state read/snapshot; write holds it from preflight through journal
commit/removal or rollback. The long-running desktop manager does not hold it
between requests. A five-second acquisition timeout returns
`AGENT_OPERATION_BUSY`. This lock serializes desktop and one-shot CLI managers;
the existing in-process mutex remains an additional local guard.

If last-applied revisions differ from current files, preview does not attempt a
three-way merge. It sets `managed_config_drift=true`, lists affected Agents,
and requires explicit managed-overwrite approval at write.

### FR-6: Write-time catalog revalidation

`agent.write` accepts:

```json
{
  "agents": ["claude", "opencode", "codex"],
  "catalog_token": "...",
  "model_config": {},
  "revision_token": "...",
  "approve_managed_overwrite": false,
  "approve_codex_auth_change": false,
  "api_key": "transient secret"
}
```

Before creating state, backups, journal, or temporary target files, write must:

1. Revalidate request shape, token, canonical config, and revision token.
2. Re-run trusted-router validation and safely restart a verified stopped
   router under the owner bound to the token.
3. Require the same trusted listener address and deployment identity bound to
   preview; otherwise return `MODEL_CATALOG_STALE`.
4. Fetch and normalize `/v1/models` again using the supplied key.
5. Verify every selected model still appears in the refreshed catalog.
6. Re-read and verify all target and sidecar revisions.

If a selected model disappeared, return `MODEL_NOT_AVAILABLE` and require new
discovery and preview. Never substitute a model. Any preflight failure leaves
all Agent and sidecar files unchanged.

Desktop and interactive setup ask for the key once and reuse the in-memory
value for discovery and write. The protocol does not persist a key fingerprint
or claim separately submitted values are identical; the write key is
authoritative if it currently authorizes every selected model.

After preflight, write uses the existing multi-file transaction, sensitive
backups, journal, atomic replacement, rollback, crash recovery, and result
rules. The last-applied sidecar participates in the same transaction.

If preview reported drift, write requires
`approve_managed_overwrite=true`. Approval permits replacement only inside the
documented managed namespaces; unrelated settings remain preserved.
If preview reported a Codex authentication migration, write separately requires
`approve_codex_auth_change=true`; the flag is rejected when no such migration
was bound into preview.

### FR-7: Last-applied state and ownership

The manager stores a user-private sidecar in the existing shared
`agent-transactions` state directory:

```text
<agent-transactions>/last-applied-model-config.json
```

The sidecar contains only:

- State schema version.
- Canonical model-config sections last applied per Agent.
- Target paths and keyed post-write revision MACs.
- Manager-owned extension paths needed to remove obsolete values.

It must not contain an API key, unkeyed key-dependent hash, candidate catalog, raw response,
rendered Agent content, unrelated user fields, or upstream details.

When a subset is selected, the transaction updates those sidecar sections and
preserves unselected sections. The sidecar is planned, privately backed up when
present, journaled, replaced, and rolled back with Agent files. Recovery must
never leave the sidecar claiming a rolled-back configuration.

Ownership rules are:

- If recorded revisions match, prior owned extension fields omitted by the new
  config are removed.
- If revisions drift, removal or replacement requires FR-6 approval.
- Without ownership evidence, an unknown existing field is preserved; a new
  extension colliding with it is `MODEL_CONFIG_INVALID` rather than silently
  taking ownership.

When no valid sidecar ownership exists, fixed namespaces use this bootstrap
collision matrix:

| Existing state | Behavior |
|---|---|
| Fixed managed path absent | Create it without overwrite approval |
| Complete exact v1 mtls-router signature | Treat as migratable ownership and show migration in preview |
| Any other value at a fixed managed path | Preserve it and require preview-bound `approve_managed_overwrite` |
| Unknown extension path | Preserve it; a requested colliding extension is `MODEL_CONFIG_INVALID` and cannot be claimed by broad overwrite approval |

Exact v1 signatures are code-owned fixtures:

- Claude: loopback `ANTHROPIC_BASE_URL`, non-empty auth token, old fixed primary
  and Haiku/Opus `gpt-5.5`, Sonnet `gpt-5.4[1M]`, tool search true, updater
  disabled, and only the known optional `_MODEL_NAME` additions.
- opencode: `provider.mtls-router` with the compatible npm/name/options shape,
  loopback `/v1` URL, non-empty key, and exactly the old `gpt-5.5`/`gpt-5.4`
  static model definitions.
- Codex: the exact `custom` provider signature already defined in FR-10, old
  fixed root model/provider values, and recognized file auth shape.

Any partial or modified signature is a collision, not an assumed migration.
Codex authentication still requires its separate approval even for exact v1.

The exact sidecar shape is:

```json
{
  "version": 1,
  "key_generation": "opaque-generation-id",
  "agents": {
    "claude": {
      "model_config": {},
      "files": [
        {"role": "config", "path": "/absolute/path", "revision_mac": "..."}
      ],
      "owned_paths": ["env.ANTHROPIC_MODEL"]
    }
  }
}
```

It is canonical JSON, limited to 2 MiB, and strictly rejects duplicate/unknown
fields and unsupported versions. Malformed, permission-invalid, generation-
mismatched, or internally inconsistent state is `MODEL_STATE_INVALID` for
preview/write while key-free Agent detection remains available.

Persisted or token-embedded whole-file revisions are HMACs using a key derived
from the per-user signing key and a distinct revision context. Plain SHA-256 of
credential-bearing file contents is not persisted or placed in a decodable
token. Journal pre/post revisions, sidecar revisions, backup verification, and
preview tokens follow the same rule.

The journal adds an internal `manager_state` scope for the sidecar rather than
pretending it belongs to an Agent. Agent files are replaced first and sidecar
last; reverse rollback restores sidecar first. Preview/write results report the
sidecar separately as `state_change`/`state_backup`, never inside a per-Agent
file list.

### FR-8: Claude Code rendering and merge

The manager preserves every top-level Claude setting and every unrelated
existing `env` entry. It manages only approved prior/new Claude `extra` keys
and these fixed keys:

- `ANTHROPIC_BASE_URL`
- `ANTHROPIC_AUTH_TOKEN`
- `ANTHROPIC_MODEL`
- `ANTHROPIC_CUSTOM_MODEL_OPTION`
- `ANTHROPIC_CUSTOM_MODEL_OPTION_NAME`
- `ANTHROPIC_DEFAULT_HAIKU_MODEL`
- `ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME`
- `ANTHROPIC_DEFAULT_SONNET_MODEL`
- `ANTHROPIC_DEFAULT_SONNET_MODEL_NAME`
- `ANTHROPIC_DEFAULT_OPUS_MODEL`
- `ANTHROPIC_DEFAULT_OPUS_MODEL_NAME`
- `ENABLE_TOOL_SEARCH`
- `DISABLE_AUTOUPDATER`

Rendering rules are:

- `ANTHROPIC_BASE_URL` is the trusted router URL without `/v1`.
- `ANTHROPIC_AUTH_TOKEN` is the supplied key.
- `ANTHROPIC_MODEL` is the primary model.
- `ANTHROPIC_CUSTOM_MODEL_OPTION` is always the primary model.
  `ANTHROPIC_CUSTOM_MODEL_OPTION_NAME` is written only when its typed name is
  present.
- Each role model is its explicit selection or the resolved primary model.
- A role `_MODEL_NAME` is its typed name, or the primary name when inherited;
  it is omitted when no name is configured.
- `ENABLE_TOOL_SEARCH=true` and `DISABLE_AUTOUPDATER=1` remain fixed policy.
- Gateway runtime discovery is not enabled; the managed snapshot is explicit.

The manager removes fixed optional name/custom-option keys when the new
canonical config omits them. It removes prior owned `extra` keys under FR-7.
It never replaces the entire `env` object.

#### Scenario: unrelated Claude environment

Given settings contain unrelated env values, when Claude config is written,
then those values remain byte-equivalent as JSON values while only managed
keys change.

### FR-9: opencode rendering and merge

The manager preserves unrelated root fields and other providers. It replaces
`provider.mtls-router` exactly with:

- `npm: "@ai-sdk/openai-compatible"`
- `name: "mtls-router"`
- `options.baseURL` set to the actual trusted API base URL.
- `options.apiKey` set to the supplied key.
- `models` containing exactly the selected canonical models.

Each selected model is rendered under its exact catalog ID. Omitted optional
capability, limit, modality, and options fields remain omitted. Only `name`
defaults to the model ID.

The manager also sets root `model` to
`mtls-router/<opencode.default_model>`. It replaces a prior root `model` only
when last-applied state proves that value was manager-owned. If a different
unowned root model already exists, preview marks a managed collision and
requires explicit managed-overwrite approval. Other root fields, including
`small_model`, remain untouched.

Existing canonical JSONC migration and explicit JSONC normalization rules stay
in force.

#### Scenario: models removed from the selection

Given an old mtls-router provider contains several models, when a new config
selects fewer, then the provider contains exactly the new selection. Removed
mtls-router models do not survive as stale entries.

### FR-10: Codex rendering, merge, and migration

Codex uses the dedicated provider ID `mtls-router`:

```toml
model_provider = "mtls-router"
model = "selected-model"
cli_auth_credentials_store = "file"

[model_providers.mtls-router]
name = "mtls-router"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://actual-loopback-listen/v1"
```

Typed and extension model keys are written at the TOML root. Omitted optional
model keys are removed only when prior state proves manager ownership. The
manager uses a real TOML parser/encoder or a focused lossless mutation
implementation with complete TOML string escaping; dynamic values are never
interpolated into quoted strings.

`disable_response_storage` is not emitted or managed in v2 because it is absent
from the pinned current Codex `ConfigToml` schema. Generated config must parse
with the pinned current Codex parser; unknown root model keys are invalid.

The provider-table fields `name=9router`, wire API `responses`, OpenAI auth
required, and a loopback `/v1` base URL are necessary but not sufficient for
historical migration. The manager removes `[model_providers.custom]` only when
the complete Codex v1 signature in FR-7 matches, including fixed root values
and recognized auth shape. Any partial match or other `custom` provider is
preserved and treated as a collision. Existing
`model_providers.mtls-router` content outside the manager-owned shape is drift
and needs explicit approval.

Codex local CLI and IDE surfaces share authentication. To make the supplied key
effective with `requires_openai_auth=true`, the manager sets
`cli_auth_credentials_store = "file"` and renders the official API-key login
shape:

```json
{
  "auth_mode": "apikey",
  "OPENAI_API_KEY": "<supplied-key>"
}
```

Unknown non-auth metadata is preserved only if the pinned Codex auth schema
accepts it. Competing known credentials (`tokens`, `last_refresh`,
`agent_identity`, `personal_access_token`, and `bedrock_api_key`) are removed.
Existing `keyring`, `auto`, ChatGPT, access-token, agent-identity, Bedrock, or
ephemeral auth is a `codex_auth` collision. Preview warns that local Codex
CLI/IDE login will switch to file-backed API-key auth and binds a separate
approval requirement.

The manager does not read, delete, or overwrite OS keyring credentials. It
changes the active config to file storage and backs up/replaces `auth.json` when
approved. Rollback restores the previous config/auth file, after which an old
keyring/auto selection can use its untouched credential again. If
`forced_login_method = "chatgpt"`, managed policy, or another effective setting
forbids file-backed API-key auth, the operation returns
`CODEX_AUTH_UNSUPPORTED` without changing the policy or any credential.
Without explicit `approve_codex_auth_change`, nothing changes.

#### Scenario: user-owned custom provider

Given `[model_providers.custom]` does not exactly match the historical
mtls-router signature, when Codex is configured, then `custom` remains
unchanged and the new dedicated provider is added.

### FR-11: Detection semantics

`agent.detect` remains key-free. `configured=true` means the local managed
structure is complete and internally consistent, not that the model is still
authorized by a current key.

- Claude requires actual router base URL shape, non-empty token, primary and
  all three role model IDs.
- opencode requires the managed provider, non-empty key, at least one model,
  and root default model referencing that provider and one declared model.
- Codex requires the dedicated provider, file credential store, API-key auth
  mode, non-empty auth key, model selection, fixed provider fields, and valid
  optional model settings.
- The historical Codex `custom` signature is recognized for migration but is
  not considered fully configured under v2.

Current authorization is reported only by successful `agent.models` and
write-time revalidation. No verified flag or timestamp is persisted.

### FR-12: Setup CLI experience

`setup.sh` and `setup.ps1` keep:

```text
agent print-config [--agent=claude,opencode,codex]
agent write-config --agent=claude,opencode,codex
```

Both add:

```text
--model-config=<path>
```

Interactive behavior is:

1. Detect and select Agents.
2. Read the API key once without echo.
3. Call `agent.models` and show the common catalog.
4. Run Agent-specific prompts for required selection and common typed fields.
5. Call `agent.render` for print, or `agent.preview` for write.
6. Print exact redacted managed fragments and file/backup/drift effects.
7. For write, require confirmation and call `agent.write` with the in-memory
   key.
8. Clear shell variables holding the key and request payload promptly.

The common-field wizard covers all required model selections and the typed
fields defined by this specification. It may offer an explicit "keep optional
fields unset" shortcut. It must not silently select defaults.

When `--model-config` is provided, the file supplies canonical config and the
wizard does not ask model-setting questions. The file must be a regular,
bounded JSON file, is never copied into manager state as a raw document, and
must not contain an API key. The key is still read securely and separately.

When stdin is not a terminal, setup scripts fail with guidance to invoke the
manager protocol directly. They never accept keys through flags, model-config,
or environment variables. Direct automation calls `agent.models`, constructs a
model config, then calls preview/write with the returned token.

`agent print-config` now requires a key because strict catalog validation is
required. It makes no Agent-config, last-applied sidecar, transaction journal,
backup, or temporary config change and prints only manager-returned redacted
fragments and target paths. Discovery may start the router and create ordinary
lifecycle state/logs, and first use may create the private signing key. Print
does not claim or display backup, migration, collision, or drift effects; those
require the merge-aware `agent.preview` write flow. All hard-coded snippets are
removed from both scripts.

Compatibility aliases route into the same v2 flow. Old noninteractive
`agent.write` requests without model config fail with `INVALID_PARAMS`; there
is no static or existing-config fallback.

`setup.ps1` must retain its UTF-8 BOM.

### FR-13: Desktop experience

The Agent page flow becomes:

```text
Select Agents -> Enter Key -> Discover Models -> Configure -> Preview -> Write -> Result
```

- React submits the key once to a focused discovery command and clears the
  password field immediately. Rust stores it in zeroizing transient flow state
  under an unguessable `flow_id`; discovery returns that ID to React, never the
  key. Render/preview carry no key. Write sends `flow_id` to Rust, which consumes
  the same in-memory key for one `agent.write` call and destroys the flow on
  every terminal path.
- The same sorted catalog is shown for every selected Agent.
- Search/filter may reduce the visible list but cannot change catalog content.
- No model is automatically selected. Valid existing selections may be
  prefilled; unavailable values require a new choice.
- Claude shows primary plus inheritable Haiku/Sonnet/Opus controls.
- opencode supports multi-select, a default among selected models, and
  per-model typed settings.
- Codex shows its active model and typed settings.
- Optional typed fields are unset by default and omission is visible.
- Each Agent has an advanced `extra` JSON object editor with formatting,
  immediate schema/conflict validation, path-specific errors, and no arbitrary
  whole-file editing.
- Import/export of the key-free canonical model-config file is allowed through
  focused Tauri commands. Arbitrary file access remains unavailable.
- Preview shows canonical choices, redacted fragments, files, backups,
  migrations, and drift/overwrite approval.
- `MODEL_NOT_AVAILABLE` and `MODEL_CATALOG_STALE` return to discovery;
  `PREVIEW_STALE` regenerates preview from the same in-memory catalog/config
  only when the catalog token remains valid.

`agent.models` and `agent.write` are secret-bearing and non-replayable in the
Rust manager supervisor. A timeout, malformed response, manager exit/restart,
or uncertain delivery destroys the flow and returns the UI to credential entry;
the supervisor never retransmits either request automatically. Ordinary
validation errors before write serialization may retain the flow. A
`PREVIEW_STALE` response proves write completed without mutation and may retain
the flow for key-free re-preview; every discovery/auth/catalog/transport or
unknown error destroys it.

The feature does not display an endpoint-compatibility warning because full
endpoint compatibility is a service contract.

### FR-14: Refresh and failure behavior

The catalog is a configuration-time snapshot. Agent files are never rewritten
in the background. Users refresh by entering configuration again and supplying
a key.

Discovery auth failure, transport failure, invalid response, empty catalog,
stale token, removed model, or validation failure is fail-closed:

- No Agent or sidecar file changes.
- No static model fallback.
- No old-catalog cache fallback.
- No implicit reuse of existing models.
- Sanitized actionable error and retry guidance.

### FR-15: Deadlines and limits

Manager deadlines are:

- `agent.detect`: five seconds.
- `agent.models`: thirty seconds, including bounded router start and fetch.
- `agent.render`: five seconds.
- `agent.preview`: five seconds.
- `agent.write`: thirty seconds, including refresh and rollback.

Rust watchdog deadlines remain one second longer. Existing per-request protocol
input limit remains 4 MiB. API keys remain limited to 16 KiB. Canonical config
is limited to 2 MiB after strict decoding. Redacted render output is limited to
2 MiB. Limit failures use stable sanitized errors and perform no writes.

### FR-16: Security and logging

- The key appears only in transient `agent.models`/`agent.write` request bodies
  over manager stdin/IPC, local `/v1/models` authorization, private
  same-directory temporary files used for atomic replacement, final Agent
  files, and approved sensitive backups that may contain current or previous
  keys.
- Models and write request bodies are never logged.
- Redirects are disabled and environment proxies are bypassed for the local
  key-bearing request.
- API keys never enter catalog/revision tokens, sidecar, journal, responses,
  errors, logs, diagnostics, command arguments, or environment variables.
- Temporary and final secret-bearing files retain existing private-permission
  and sensitive-backup guarantees.
- Rust best-effort zeroization and frontend logical clearing remain in force.
- Model IDs, names, config paths, and upstream error text are sanitized before
  terminal or UI display.

## Protocol Contract

Protocol v2 uses strict parameter decoding and rejects unknown fields.

### `agent.models`

Request fields:

- `owner`: required `cli` or `desktop`.
- `agents`: required unique array in supported Agent vocabulary.
- `api_key`: required non-empty transient string.

Result fields:

- `models`: normalized model-ID array.
- `catalog_token`: opaque authenticated envelope.
- `router_base_url` and `api_base_url`: trusted normalized local URLs.
- `existing.model_config`: key-free prefill document containing only selected
  Agent sections that are complete and valid against the current catalog.
- `existing.unavailable_models`: Agent-to-model-ID arrays.
- `existing.drifted_agents`: Agent array.

If a section has any unavailable required ID, `model_config` omits that entire
Agent section; clients use `unavailable_models` only for explanation and require
fresh complete selection. There is no partial-canonical prefill type.

### `agent.render`

Request fields are `agents`, `catalog_token`, and `model_config`. The result
is exactly:

```json
{
  "model_config": {},
  "fragments": [
    {
      "agent": "claude",
      "role": "config",
      "path": "/absolute/target",
      "format": "json",
      "content": "redacted managed fragment"
    }
  ]
}
```

Claude and opencode have one config fragment. Codex has `config` and `auth`
fragments. Empty fragment arrays are invalid because at least one Agent is
required.

### `agent.preview`

Request fields are `agents`, `catalog_token`, and `model_config`. The result
extends the existing structured preview with canonical config, redacted
managed fragments, `managed_config_drift`, `drifted_agents`,
`managed_collisions`, `requires_codex_auth_approval`, and optional
`state_change`/`state_backup`. `managed_collisions` entries contain Agent,
canonical JSON-pointer/TOML path, collision type, and proposed action, but no
current secret value. Codex returns separate config and auth fragments/files.

### `agent.write`

Request fields are `agents`, `catalog_token`, `model_config`,
`revision_token`, `approve_managed_overwrite`,
`approve_codex_auth_change`, and transient `api_key`. Results retain the
existing transaction/per-Agent status shape and add optional separate
`state_change` and `state_backup` fields, but no secret or catalog data.

### Tokens

Catalog and revision tokens are authenticated, versioned, URL-safe envelopes
using keys derived from the private per-user signing key with distinct HMAC
contexts. Revision tokens are deterministic for identical snapshots; catalog
tokens need not be. Tokens never copy the transient key. Invalid version,
signature, limits, generation, deployment, protocol, owner, Agent scope, router
address, or model config is rejected before file mutation.

Protocol errors retain stable `code` and sanitized `message` and add optional
bounded `details` for validation only:

```json
{"path":"/opencode/models/model-a/options/apiKey","rule":"protected_path"}
```

`path` is a JSON Pointer and `rule` is a stable non-secret identifier. Details
never contain rejected values, file contents, response data, or API keys.
Clients still branch on top-level error code.

## Error Codes

Protocol v2 retains all existing stable codes and adds:

| Code | Meaning | Client action |
|---|---|---|
| `MODEL_AUTH_FAILED` | `/v1/models` rejected the key with 401/403 | Re-enter key |
| `MODEL_DISCOVERY_FAILED` | Trusted request could not complete successfully | Check router/upstream and retry |
| `MODEL_RESPONSE_INVALID` | Successful response violated the bounded standard schema | Report service contract failure |
| `MODEL_CATALOG_EMPTY` | Successful response contained no usable model IDs | Report account/catalog issue |
| `MODEL_CATALOG_STALE` | Catalog token no longer matches trusted deployment/address/protocol | Rediscover models |
| `MODEL_CONFIG_INVALID` | Canonical config, typed fields, extension, or Agent-section scope is invalid | Correct model config |
| `MODEL_NOT_AVAILABLE` | A selected model is absent during write-time refresh | Rediscover and select again |
| `MANAGED_CONFIG_DRIFT` | Drift exists but overwrite approval is absent or mismatched | Review preview and approve or cancel |
| `MODEL_STATE_INVALID` | Signing key, sidecar, or ownership state is unsafe/corrupt | Follow state recovery guidance |
| `AGENT_OPERATION_BUSY` | Another manager owns the shared Agent transaction lock | Retry after the other operation finishes |
| `CODEX_AUTH_UNSUPPORTED` | Effective Codex auth policy/store cannot be safely migrated | Resolve Codex login/store policy and retry |

Error messages are sanitized diagnostics. Clients branch only on codes.

## Migration

### Protocol and release migration

- Bump the management protocol constant and all expected/build metadata to v2.
- Release router, manager, setup scripts, and desktop together.
- Existing install receipt and external-router compatibility checks reject a
  mixed generation and direct users to update the full pair/application.
- Setup scripts embed expected protocol `2`, require receipt protocol exactly
  `2`, and perform a non-sensitive `manager.info` deployment/protocol handshake
  before serializing the first `agent.models` request.
- No v1 Agent request is interpreted as a v2 request.

### Existing Agent configuration

- Existing hard-coded model IDs are merely prefill candidates and are usable
  only if returned by the new key's catalog.
- Claude changes from whole-`env` replacement to managed-key merge.
- opencode changes from fixed models to the exact selected catalog subset and
  manages root default `model` under ownership rules.
- Codex changes provider ID from `custom` to `mtls-router`; only exact old
  mtls-router signatures are removed.
- Codex switches to approved file-backed official API-key auth and removes
  competing known auth material, with sensitive backup/rollback and a separate
  approval.
- Existing backups are never deleted or rewritten by migration.

### CLI migration

- Interactive users now enter the key before model selection and preview.
- Noninteractive users must use protocol v2 and supply canonical model config.
- `--model-config` never contains a key.
- Existing static examples and hard-coded setup snippets are removed.

## Testing Requirements

### Go unit and integration tests

- Model client: Bearer header, no body/query, no proxy, no redirects, timeout,
  status mapping, body limit, and secret-free failures.
- Parser: malformed root/data/items, trailing JSON, empty/duplicate/unsorted
  IDs, whitespace, UTF-8, controls, size/count limits, and deterministic order.
- Tokens: cross-process verification, tampering, protocol/deployment/address/
  owner/Agent mismatch, signing-key creation/corruption/rotation, size bounds,
  public-data forgery attempts, and no copied transient key.
- Shared lock: desktop-versus-CLI, two one-shot managers, concurrent preview/
  write, concurrent startup recovery, timeout, and release after failure.
- Canonical config: every typed field, enum, integer relationship, exact Agent
  section matching, deterministic encoding, extension deep merge, collision,
  secret-like keys, and depth/size limits.
- Renderers: omitted optional fields, correct escaping, actual listener URL,
  same shared catalog, and exact Agent-native output.
- Claude merge: unrelated env preservation and obsolete owned-key removal.
- opencode merge: exact model subset/default, unrelated provider preservation,
  JSONC behavior, and root model collision/drift.
- Codex merge: dedicated provider, exact historical migration, unrelated
  custom provider preservation, dynamic TOML escaping, root model fields,
  file-backed API-key auth migration, separate approval, keyring preservation,
  forced-login rejection, and rollback.
- Detection: local structural completeness independent of current model auth.
- Write: model refresh before any write, disappeared model, router-address
  change, stale preview, drift approval, sidecar update, partial failure,
  rollback, deadline, and crash recovery.
- Router contract fixtures cover the exact method/path/header/query/SSE matrix
  in the Service Contract with authorization preserved and no sensitive logging.

### Setup script tests

- Shell and PowerShell interactive catalog and selection flows.
- `--model-config` path, strict validation, and no key in file/arguments/env.
- Print uses manager redacted render and creates no Agent/transaction config;
  tests account for permitted router lifecycle and signing-key state.
- Write requires v2 config and has no static fallback.
- Exact model IDs and special characters are passed safely.
- Discovery, auth, empty catalog, removed model, stale preview, and drift errors.
- No hard-coded model/provider snippets remain in either script.
- `setup.ps1` retains its UTF-8 BOM.

### Desktop tests

- TypeScript/Rust v2 IPC shape and strict validation.
- Stage transitions, one-time key entry, cancel/navigation/error clearing.
- Shared searchable catalog and no automatic selection.
- Claude inheritance, opencode multi-model/default/settings, and Codex settings.
- Advanced JSON editor formatting and path-specific validation.
- Valid prefill, unavailable models, drift approval, stale catalog, removed
  model, preview, transaction result, and retry paths.
- Import/export of canonical config with no credential-designated fields
  through focused commands.
- No endpoint-compatibility warning.
- No key in DOM snapshots, logs, errors, persisted state, or diagnostics.

### Schema conformance tests

- The manager publishes a versioned JSON Schema generated from the canonical
  Go model and checked-in JCS test vectors.
- Rust and TypeScript local validation is generated from or tested against that
  schema; manager validation remains authoritative.
- Go, Rust, TypeScript, jq, and PowerShell fixtures agree on duplicate-key
  rejection, UTF-8, integers, Unicode, number encoding, key ordering, and exact
  canonical bytes.
- Generated Agent files parse with compatibility-manifest-pinned current stable
  Claude/opencode/Codex parsers or binaries.

### Cross-platform acceptance

- Windows, macOS, and Linux preserve private permissions for Agent files,
  backups, journal, and last-applied sidecar.
- IPv4 and IPv6 loopback URL rendering is valid where supported.
- Shell and PowerShell produce equivalent canonical requests and results.
- Desktop and CLI can share last-applied state and detect each other's drift.

## Documentation Impact

Update together:

- `README.md` and `docs/zh-CN/README.md`
- `docs/DESKTOP.md` and `docs/zh-CN/DESKTOP.md`
- `docs/TROUBLESHOOTING.md` and `docs/zh-CN/TROUBLESHOOTING.md`
- `CHANGELOG.md` and `docs/zh-CN/CHANGELOG.md`
- Setup help in both scripts
- Desktop Chinese and English locale resources
- Maintainer protocol/release documentation
- `AGENTS.md`, removing or correcting the nonexistent `docs/agent-conf/`
  reference unless real generated golden fixtures are added

Documentation must describe:

- The all-endpoint model compatibility service contract.
- The key-before-discovery workflow.
- Agent-native model settings and omission defaults.
- Manual refresh and hard-failure behavior.
- v2 automation and model-config schema.
- The distinction between local `configured` and current authorization.
- Exact preservation/ownership boundaries and sensitive backups.

## Repository Impact

Expected implementation areas include:

- `internal/version/`
- `internal/manager/protocol/`
- `internal/manager/app/`
- `internal/manager/discovery/` and lifecycle integration
- A focused model-catalog client package under `internal/manager/`
- `internal/manager/agent/`
- `setup.sh`, `setup.ps1`, and shell tests
- `desktop/src-tauri/`
- `desktop/src/` and locale/test files
- Build/release protocol metadata and documentation

The preferred implementation creates focused model-schema, catalog, and
renderer units rather than expanding the existing `preview.go` into a single
large component. Existing transactional write machinery should be extended,
not replaced.

## Verification Commands

The final implementation must pass:

```bash
test -z "$(gofmt -l .)"
go test ./...
go vet ./...
make test-shell
```

Desktop verification must pass:

```bash
npm run static:check
npm run typecheck
npm test
npm run build
npm run rust:format
npm run rust:test
```

Run desktop commands from `desktop/`. Supported-platform package and smoke
checks remain required by the existing desktop specification.
