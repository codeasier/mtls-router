# Targeted Validation Build Design

**Date:** 2026-07-19
**Status:** Approved design

## Goal

Allow maintainers to use `gh` to run a validation-only Release workflow for one OS/architecture target and an optional upstream URL while retaining the repository's GitHub-hosted mTLS and signing credentials.

The first intended invocation builds both CLI and desktop artifacts for Windows amd64. The same mechanism supports all six existing OS/architecture targets and preserves the current all-target default.

## Inputs

Extend `workflow_dispatch` with:

- `version`: existing required validation SemVer.
- `target`: a required choice with `all` as the default and these target values: `windows-amd64`, `windows-arm64`, `darwin-amd64`, `darwin-arm64`, `linux-amd64`, and `linux-arm64`.
- `upstream_url`: an optional string. An empty value uses the repository variable `UPSTREAM_URL`.

The upstream URL is not a secret. GitHub records workflow inputs, so callers must not include credentials, tokens, or sensitive query parameters. The selected URL must use HTTPS and remains subject to the embedded CA and client-certificate requirements.

## Matrix Selection

Add a matrix-selection job that emits JSON matrices for the CLI and desktop jobs.

- A tag-triggered run always emits all six CLI and all six desktop rows, regardless of dispatch defaults or inputs.
- A manually dispatched run with `target=all` emits the same complete matrices.
- A manually dispatched run with one target emits one matching CLI row and one matching desktop row.

The build jobs consume these outputs using `fromJSON`. Matrix rows retain the current fields, runner mappings, Rust targets, and bundle types. Matrix generation must use a fixed target mapping rather than interpolating user input into runner names or commands.

## Upstream Selection

Each CLI and desktop build resolves its effective upstream before building:

- Tag push: always use `${{ vars.UPSTREAM_URL }}`.
- Manual dispatch with non-empty `upstream_url`: use the input.
- Manual dispatch with empty `upstream_url`: use `${{ vars.UPSTREAM_URL }}`.

The effective value is validated as HTTPS by the existing build checks. The override must not mutate the repository variable and therefore cannot race with other release or validation runs.

## Publication Safety

Manual dispatch remains validation-only. The release aggregation and publication job continues to run only for tag refs.

Tag publication remains unchanged in substance:

- Full CLI and desktop matrices are mandatory.
- Production upstream comes only from the repository variable.
- Existing artifact-count, protocol-generation, checksum, GitHub Release, and mirror checks remain intact.

Targeted manual runs do not invoke aggregation, so incomplete matrices cannot be mistaken for publishable release sets.

## Wrapper CLI

Extend `.agents/skills/mtls-router-github-test-build/scripts/build-and-download.sh` with:

- `--target TARGET`, defaulting to `all`.
- `--upstream URL`, optional.

The wrapper validates target values and requires an HTTPS upstream when supplied. It sends only declared workflow inputs, continues to resolve commits and tags through temporary branches where necessary, verifies the exact run SHA, and downloads only requested artifacts.

The wrapper must never accept mTLS certificate, private-key, CA, or signing-secret values.

Example:

```bash
.agents/skills/mtls-router-github-test-build/scripts/build-and-download.sh \
  --ref main \
  --version 0.2.0-windows-test.1 \
  --target windows-amd64 \
  --upstream https://router.example.com \
  --artifact mtls-router-cli-windows-amd64 \
  --artifact mtls-router-desktop-windows-amd64
```

## Error Handling

- Reject unknown target values before dispatch.
- Reject a supplied non-HTTPS upstream before dispatch and again in the workflow.
- Fail matrix selection if its fixed mapping does not contain the selected target.
- Preserve existing failures for absent production variables, deployment identifiers, credentials, package checks, and signing checks.
- Continue cleaning temporary remote branches on success and failure.

## Testing

Extend shell-based workflow tests to assert:

- The dispatch input choices and defaults are exact.
- Tag runs cannot consume `inputs.upstream_url` or a targeted matrix.
- Every target maps to exactly one CLI and one desktop row.
- `all` and tag runs retain all six rows in both matrices.
- The release job remains tag-only and still depends on both build jobs.
- The wrapper accepts and dispatches validated target/upstream inputs and rejects invalid values.

Update English and Chinese build documentation and the project skill usage examples. Existing Go, shell, desktop workflow, and formatting checks remain the verification baseline.

## Non-Goals

- Publishing a partial release.
- Supplying or rotating GitHub Secrets through workflow inputs.
- Persistently changing repository variables.
- Selecting CLI-only or desktop-only builds.
- Adding arbitrary custom runner, Rust target, or bundle inputs.
