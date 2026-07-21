# Simplified Agent Model Catalog Design

**Date:** 2026-07-20

## Summary

Add one manager-only build-time `simplify` setting. It defaults to enabled and,
when enabled, excludes every authenticated `/v1/models` entry whose ID contains
the ASCII slash character (`/`) from Agent configuration.

The filtered catalog is authoritative for discovery, presets, existing
configuration projection, imports, catalog-token claims, preview, and
write-time revalidation. Setting `SIMPLIFY=False` at build time preserves the
current complete-catalog behavior.

## Goals

- Default released and locally scripted manager builds to simplified catalogs.
- Let builders explicitly disable simplification with `SIMPLIFY=False`.
- Hide slash-containing model IDs from every Agent configuration client.
- Treat hidden IDs as unavailable in existing, preset, and imported model
  configurations.
- Apply exactly the same catalog rule during initial discovery and write-time
  refresh.
- Reject malformed build values instead of silently choosing a behavior.

## Non-Goals

- Do not change the router binary or request proxy behavior.
- Do not change upstream `/v1/models` responses sent through the router.
- Do not classify slash-containing IDs as malformed upstream data.
- Do not filter full-width slashes, reverse slashes, spaces, Unicode, or other
  model-name patterns.
- Do not add a runtime flag, manager protocol field, or client-side preference.
- Do not let existing configuration bypass the simplified catalog.
- Do not add a new management protocol error code.

## Existing Behavior

The manager authenticates `GET /v1/models` through a trusted router connection.
`modelcatalog.Parse` validates the bounded OpenAI-compatible response, extracts
every valid `data[].id`, deduplicates exact strings, enforces the model-count
bound, and sorts the resulting catalog by UTF-8 bytes. Slash-containing IDs are
currently valid and are returned to Shell, PowerShell, and Desktop clients.

The same trusted-router path is used during initial `agent.models` discovery
and immediately before `agent.write`. The normalized catalog is included in a
signed catalog token and is the exact membership boundary for current Agent
configuration, build presets, imports, previews, and writes.

## Build Contract

Add this manager link-time string variable:

```go
package modelcatalog

var Simplify = "True"
```

The code initializer makes unscripted `go build` invocations simplify by
default. Builders may inject a normalized value directly:

```text
-X github.com/codeasier/mtls-router/internal/manager/modelcatalog.Simplify=False
```

Repository build entry points accept the environment variable `SIMPLIFY`:

| Input | Result |
|---|---|
| Unset or empty | Normalize and inject `True` |
| `true` or `True`, in any letter case | Normalize and inject `True` |
| `false` or `False`, in any letter case | Normalize and inject `False` |
| Any other value, including `1`, `0`, `yes`, or `no` | Fail the build |

The affected entry points are:

- `scripts/build.sh`
- `desktop/scripts/build-sidecars.sh`
- `.github/workflows/release.yml`

Release builds source `SIMPLIFY` from the repository variable of the same name.
An absent GitHub variable still produces `True`. Both standalone and desktop
manager binaries receive the normalized value. Router binaries never receive
it because the router does not configure Agents.

## Startup Validation

The manager parses the linked value before it begins protocol serving. This
second validation boundary is required because callers can invoke `go build`
with `-ldflags -X` directly and bypass repository shell validation.

Only case-insensitive `true` and `false` are accepted. A directly linked empty
string is invalid because it explicitly replaces the initialized default.
Invalid values fail manager startup with the fixed error
`invalid embedded simplify value`. The error does not include the supplied
value.

The parsed boolean is passed through manager construction to the trusted-router
catalog channel and then to the model-catalog client. It is immutable for the
lifetime of the process.

## Catalog Processing

Catalog processing retains the current response validation contract:

1. Validate the complete response body as bounded UTF-8 JSON.
2. Validate every `data` item and every model ID with the existing syntax and
   byte limits.
3. Deduplicate exact IDs and enforce the existing maximum catalog size.
4. Sort the complete normalized catalog using the existing byte order.
5. If simplification is enabled, remove IDs for which
   `strings.Contains(id, "/")` is true.
6. Return the remaining catalog, or `MODEL_CATALOG_EMPTY` when none remains.

Filtering after validation ensures malformed hidden entries cannot make an
otherwise invalid upstream response appear valid. Enforcing the count bound
before filtering also preserves the existing resource limit rather than
allowing an arbitrarily large hidden catalog.

Filtering occurs after sorting, as listed above, so the implementation has one
fixed normalization pipeline.

## Consistency Semantics

The trusted-router channel owns the simplify setting and constructs the catalog
client with it for every fetch. Consequently, the setting applies identically
to:

- initial `agent.models` discovery;
- the `models` protocol response;
- signed catalog claims;
- build-preset availability;
- existing Agent configuration projection and availability metadata;
- imported canonical model configuration validation;
- render and preview validation; and
- refreshed-catalog validation immediately before write.

When simplification is enabled, a slash-containing model referenced by an
existing Agent file or build preset is reported through the existing
`MODEL_NOT_AVAILABLE` behavior. It is not retained as a hidden exception and
is never written through a newly approved configuration.

When simplification is disabled, all currently valid IDs, including IDs with
one or more ASCII slashes, remain available. This is the compatibility mode for
deployments that intentionally expose provider-qualified model IDs.

## Error Handling

| Condition | Result |
|---|---|
| Missing build input | Simplification enabled |
| Valid `True` build input | Simplification enabled |
| Valid `False` build input | Complete current catalog behavior |
| Invalid script environment value | Build fails before `go build` |
| Invalid directly linked value | Manager fails before protocol serving |
| Mixed slash and non-slash catalog while enabled | Return only non-slash IDs |
| Nonempty upstream catalog filtered to zero IDs | `MODEL_CATALOG_EMPTY` |
| Malformed slash-containing entry | `MODEL_RESPONSE_INVALID` |
| Existing/preset/imported slash ID while enabled | Existing `MODEL_NOT_AVAILABLE` behavior |

No new protocol error code or response field is necessary.

## Compatibility

- The manager protocol remains version 2 because request and response shapes do
  not change.
- Router lifecycle, proxy routes, credentials, and runtime configuration are
  unchanged.
- Default behavior intentionally changes from complete catalogs to simplified
  catalogs.
- `SIMPLIFY=False` restores the existing catalog behavior for new builds.
- The setting is fixed per manager artifact; clients cannot change it at
  runtime.
- Existing tests that use slash-containing IDs to exercise Unicode or shell
  quoting must use non-slash special IDs unless they explicitly build or
  construct a simplify-disabled manager.

## Documentation

Update English and Chinese documentation together:

- `docs/BUILD.md` and `docs/zh-CN/BUILD.md` describe `SIMPLIFY`, accepted values,
  the default, and direct linker injection.
- `docs/AGENT_MODELS.md` and `docs/zh-CN/AGENT_MODELS.md` replace the current
  unconditional no-name-filtering contract with the build-dependent catalog
  rule.
- README and Desktop documentation mention that displayed candidate models are
  the manager's build-filtered authenticated catalog where they currently call
  it complete or unfiltered.

The setting is a build-time policy, not a user configuration-precedence input,
so it does not belong in the router flag/environment precedence table.

## Testing

### Go Unit Tests

- Parse default `Simplify` as enabled.
- Accept case-insensitive `true` and `false`.
- Reject an empty direct-link override and every other malformed linked value
  without echoing it.
- Keep slash-containing IDs when simplification is disabled.
- Remove slash-containing IDs from a mixed catalog when enabled.
- Match only ASCII `/` and retain full-width slash and reverse slash.
- Preserve existing ID validation, deduplication, sorting, body limits, and
  pre-filter model-count bounds.
- Return `MODEL_CATALOG_EMPTY` when every valid ID is filtered.
- Reject malformed slash-containing IDs before filtering.

### Manager Flow Tests

- Verify filtered models populate the protocol response and signed token.
- Verify a slash-containing preset or existing model is unavailable when
  enabled.
- Verify a slash-containing imported selection cannot preview or write when
  enabled.
- Verify write-time revalidation uses the same setting as discovery.
- Verify disabled mode preserves slash-containing models end to end.
- Verify an invalid linked value fails startup before serving requests and does
  not expose the value in stderr.

### Build and Client Tests

- Verify local and desktop build scripts default to `True`.
- Verify case-insensitive `SIMPLIFY=False` produces a manager that retains slash
  IDs.
- Verify unsupported environment values fail before compilation.
- Verify release and desktop manager linker flags receive the same normalized
  value while router linker flags do not.
- Update Shell and PowerShell fixtures so default-build tests no longer depend
  on slash IDs; retain explicit simplify-disabled coverage for compatibility.

Run the repository checks after implementation:

```bash
test -z "$(gofmt -l .)"
go test ./...
go vet ./...
make test-shell
```

## Acceptance Criteria

1. A manager built without an explicit simplify value excludes every model ID
   containing ASCII `/` from Agent configuration.
2. `SIMPLIFY=False` in supported build entry points produces a manager that
   retains slash-containing model IDs.
3. Only case-insensitive `true` and `false` are accepted; invalid script or
   direct-link values fail closed.
4. Filtering governs the displayed list, catalog token, existing and preset
   availability, imports, preview, and write-time refresh consistently.
5. A catalog containing only slash IDs returns `MODEL_CATALOG_EMPTY`.
6. Simplification does not weaken complete upstream response validation or
   existing resource limits.
7. Router binaries and runtime proxy behavior remain unchanged.
8. English and Chinese build and Agent-model contracts describe the same
   behavior.
