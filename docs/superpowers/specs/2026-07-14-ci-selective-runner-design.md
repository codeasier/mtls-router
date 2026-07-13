# Selective CI Runner Design

**Date:** 2026-07-14
**Status:** Approved design

## Problem

The CI workflow currently runs the same 12 runner jobs for every pull request and for
the resulting push to `main`. The workload includes three host operating-system
Rust test jobs and six unsigned desktop package jobs. Even documentation-only
changes therefore consume macOS, Windows, Linux, x86_64, and ARM runners twice.

The desktop application is not independent from the repository root. Its
sidecars are built from the root Go router and manager, so selective execution
must represent those dependencies instead of dividing changes into only
`desktop/**` and non-desktop paths.

## Goals

- Run pull-request checks only for the areas affected by the complete PR diff.
- Keep Linux, macOS, and Windows Rust tests for relevant desktop changes.
- Validate pull-request packaging on Linux ARM64 and Windows x86_64.
- Keep all six target packages in the release workflow.
- Avoid expensive desktop matrices after a PR is merged to `main`.
- Retain low-cost validation for direct pushes to `main`.
- Cancel obsolete CI runs when a newer commit is pushed to the same PR.
- Fail closed when change classification or workflow assertions fail.

## Non-Goals

- Changing release triggers, signing, notarization, or publishing behavior.
- Removing any release target.
- Making the desktop sidecars independent from the root Go modules.
- Adding scheduled full-matrix CI.
- Configuring repository rules or required status checks.

## Current Constraints

- `desktop/scripts/build-sidecars.sh` builds the router from the repository root
  and the manager from `cmd/mtls-router-manager`.
- The packaged desktop verification exercises the manager handshake, so manager
  and protocol changes affect package validation.
- The active `main` ruleset prevents deletion and non-fast-forward updates but
  does not require status checks and does not prevent direct pushes.
- Workflow-level path filtering can leave a required check pending when the
  complete workflow is skipped. Selective execution will therefore use job
  conditions inside an always-triggered workflow.

## Architecture

Keep one `.github/workflows/ci.yml` triggered by pull requests targeting `main`
and pushes to `main`. Add an initial `scope` job that classifies changed paths
with `dorny/paths-filter` and runs the existing low-cost workflow assertions.
The other jobs depend on `scope` and use its outputs in job-level `if`
conditions.

The workflow will not use event-level `paths` or `paths-ignore`. A failed scope
job blocks its dependants and fails the workflow instead of treating an error as
an empty diff.

Add workflow concurrency keyed by workflow name and pull-request number, with
the Git ref as the fallback for non-PR events. Enable `cancel-in-progress` so a
new PR commit stops obsolete matrix jobs.

## Change Scopes

The scope job emits these boolean outputs:

| Output | Paths | Purpose |
| --- | --- | --- |
| `ci` | `.github/workflows/ci.yml`, `Makefile` | Validate CI orchestration changes with every PR job. |
| `go` | Root and `cmd/**`/`internal/**` Go files, `go.mod`, `go.sum`, `setup.sh`, `setup.ps1`, `scripts/**`, `tests/setup_*.sh`, `Dockerfile`, `systemd/**` | Run Go formatting, shell tests, Go tests, and vet. |
| `frontend` | `desktop/src/**`, `desktop/package.json`, `desktop/package-lock.json`, `desktop/.npmrc`, `desktop/.prettierignore`, `desktop/index.html`, `desktop/app-icon.svg`, and desktop frontend tool configuration | Run frontend static checks, type checks, tests, and build. |
| `rust` | `desktop/src-tauri/**`, `desktop/scripts/build-sidecars.sh`, `cmd/mtls-router-manager/**`, `internal/manager/**`, `internal/version/**`, `go.mod`, `go.sum` | Run Rust formatting and tests on the three host operating systems. |
| `package` | `desktop/**`, `cmd/mtls-router-manager/**`, `internal/manager/**`, `internal/version/**`, `go.mod`, `go.sum` | Build and inspect representative unsigned desktop packages. |

The filters should list concrete repository paths rather than broad negative
patterns. A CI workflow or Makefile change sets `ci`, and every conditional PR
job treats `ci` as an override. This deliberately causes full PR validation
when the orchestration itself changes.

Changing ordinary router/proxy Go code does not trigger package validation.
Changing the manager, management protocol, shared version metadata, Go module
definition, or any desktop path does trigger it because those inputs can affect
packaged sidecars or the package verification handshake.

## Event Behavior

### Pull Requests

The scope job always runs. Other jobs use these conditions:

| Job | Condition |
| --- | --- |
| `go-shell` | `go` or `ci` changed |
| `frontend` | `frontend` or `ci` changed |
| `rust` | `rust` or `ci` changed |
| `desktop-package` | `package` or `ci` changed |

The Rust matrix remains:

- Linux on `ubuntu-24.04`
- macOS on `macos-15`
- Windows on `windows-2025`

The PR package matrix becomes:

- Linux ARM64: `ubuntu-24.04-arm`, target
  `aarch64-unknown-linux-gnu`, AppImage
- Windows x86_64: `windows-2025`, target
  `x86_64-pc-windows-msvc`, NSIS

The release workflow continues to build both architectures for Linux, macOS,
and Windows. It remains the full six-target package gate.

### Pushes to `main`

The scope job always runs and includes workflow static assertions. `go-shell`
runs only when the `go` or `ci` scope changed. Frontend, Rust, and package jobs
are restricted to the `pull_request` event and do not run after merge.

Consequences for common changes:

| Change | PR | Push after merge |
| --- | --- | --- |
| Documentation only | Scope assertions | Scope assertions |
| Proxy or setup code | Scope + Go/shell | Scope + Go/shell |
| Frontend only | Scope + frontend + two packages | Scope only |
| Rust desktop only | Scope + three-system Rust + two packages | Scope only |
| CI workflow | All PR jobs | Scope + Go/shell |

Direct pushes to `main` retain the low-cost checks described above. They do not
receive expensive desktop validation; preventing unvalidated direct pushes is
a repository-rules concern and is outside this change.

## Permissions And Dependency

Keep `contents: read` and add `pull-requests: read`. `dorny/paths-filter@v4` reads
the pull-request file list through the GitHub API, so explicitly declaring the
second read-only permission is required once the workflow defines its own token
permissions. The major-version reference is consistent with the repository's
existing pinning convention for GitHub Actions.

No secrets are exposed to pull-request jobs. The package jobs continue to use
placeholder credentials and unsigned packages.

## Failure Semantics

- If checkout, path classification, or workflow assertions fail in `scope`, the
  workflow fails and conditional jobs do not run.
- Jobs omitted by a false job-level condition appear as skipped and do not
  consume their configured matrix runners.
- Matrix jobs retain `fail-fast: false` so independent platform failures remain
  visible once the matrix has started.
- No extra summary runner is added. The workflow run result is the aggregate
  result, while `scope` provides a stable check that always exists.

If required checks are configured later, `scope` is safe to require because it
always runs. Individual conditional jobs should only be required after adding
an always-present aggregate check; event-level path filtering must not be used
for required workflows.

## Static Tests

Update `tests/desktop_workflow_test.sh` to assert:

- CI still supports pull requests and pushes to `main`.
- CI grants only `contents: read` and `pull-requests: read`.
- CI has concurrency cancellation for obsolete runs.
- A scope job uses the path-filter action, exposes all five outputs, and runs
  `make test-workflows`.
- Conditional jobs depend on `scope` and use the intended event and scope
  conditions.
- A CI workflow change enables every PR validation area.
- Expensive frontend, Rust, and package jobs are restricted to pull requests.
- The Rust matrix still contains Linux, macOS, and Windows.
- The CI package matrix contains exactly Linux ARM64 and Windows x86_64 targets
  and their corresponding runners and bundle types.
- The release workflow still contains all six targets and all six runners.
- Existing npm version, sidecar, metadata, package verification, signing status,
  no-updater, and no-publishing-in-CI assertions remain intact.

Because existing tests currently assume all six package targets and runners are
present in both CI and release workflows, split those assertions so CI and
release have independent expected matrices.

## Verification

Run:

```bash
make test-workflows
```

Also parse or lint `.github/workflows/ci.yml` with an available workflow-aware
or YAML validation tool. Review the resulting job conditions against this event
table, because shell string assertions alone cannot prove GitHub expression
semantics.

The first pull request using the change should confirm these observable cases:

- a documentation-only commit starts only `scope`;
- a frontend commit starts frontend and the two package jobs, but not Go/shell
  or Rust;
- a Rust desktop commit starts the three Rust jobs and two package jobs;
- a CI workflow commit starts all PR jobs;
- a newer commit cancels an in-progress run for the same PR;
- after merge, the push run does not start desktop matrices.

## Expected Impact

A documentation-only PR and its merge fall from 24 total runner jobs to two
short scope jobs. A relevant desktop PR falls from nine desktop matrix runners
to the affected frontend and/or three Rust hosts plus two representative package
runners; its merge does not repeat those matrices. Full six-target packaging is
deferred to the release workflow, where it directly validates release output.
