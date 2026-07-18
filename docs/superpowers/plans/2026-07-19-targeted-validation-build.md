# Targeted Validation Build Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add safe validation-only GitHub Actions inputs for one paired CLI/desktop target and an optional HTTPS upstream override.

**Architecture:** A matrix-selection job owns a fixed mapping for the six supported targets and emits filtered CLI and desktop matrices. Build jobs resolve the upstream from a dispatch override or repository variable, while tag runs always select the complete matrices and repository upstream; the existing tag-only publication job remains unchanged.

**Tech Stack:** GitHub Actions YAML, Bash, `jq`, GitHub CLI, repository shell regression tests, Markdown.

---

## File Structure

- Modify `.github/workflows/release.yml`: declare validation inputs, generate safe matrices, and resolve the effective upstream.
- Modify `tests/desktop_workflow_test.sh`: statically verify matrix wiring and formal release invariants.
- Modify `tests/setup_release_packaging_test.sh`: verify tag publication still depends on complete producer jobs and repository upstream behavior.
- Create `tests/setup_github_test_build_skill_test.sh`: exercise wrapper argument validation with stubbed commands and inspect dispatch fields without contacting GitHub.
- Modify `.agents/skills/mtls-router-github-test-build/scripts/build-and-download.sh`: add `--target` and `--upstream` validation and dispatch inputs.
- Modify `.agents/skills/mtls-router-github-test-build/SKILL.md`: document targeted builds and upstream input safety.
- Modify `docs/BUILD.md` and `docs/zh-CN/BUILD.md`: document direct `gh` validation dispatch and artifact behavior.

### Task 1: Lock Down Workflow Contracts

**Files:**
- Modify: `tests/desktop_workflow_test.sh`
- Modify: `tests/setup_release_packaging_test.sh`

- [ ] Add assertions for exact `target` choices, optional `upstream_url`, `prepare` matrix outputs, `fromJSON` consumption, and tag-only release behavior.
- [ ] Run `make test-workflows` and verify it fails because the workflow has no targeted inputs or dynamic matrices.
- [ ] Keep assertions structural: fixed mappings must cover all six targets, each selected row must pair one CLI and one desktop producer, and `release` must remain tag-only.

### Task 2: Implement Safe Dynamic Matrices

**Files:**
- Modify: `.github/workflows/release.yml`

- [ ] Add `target` choice and optional `upstream_url` dispatch inputs.
- [ ] Add a `prepare` job that defines immutable CLI/desktop JSON arrays, validates the selected target, filters with `jq`, and exports compact matrices.
- [ ] Make `build` and `desktop` depend on `prepare` and consume `fromJSON(needs.prepare.outputs.*)`.
- [ ] Resolve `UPSTREAM_URL` with `${{ github.event_name == 'workflow_dispatch' && inputs.upstream_url || vars.UPSTREAM_URL }}` in both producer jobs.
- [ ] Preserve `release` as tag-only with `needs: [build, desktop]`; tag runs force `target=all` in the prepare script.
- [ ] Run `make test-workflows` and verify workflow contract tests pass.

### Task 3: Extend the Safe Build Wrapper

**Files:**
- Create: `tests/setup_github_test_build_skill_test.sh`
- Modify: `.agents/skills/mtls-router-github-test-build/scripts/build-and-download.sh`

- [ ] Add a stubbed shell test that checks help text, rejects unknown targets and non-HTTPS upstreams before GitHub access, and verifies dispatch includes `inputs[target]` plus `inputs[upstream_url]` only when supplied.
- [ ] Run `bash tests/setup_github_test_build_skill_test.sh` and verify it fails against the current wrapper.
- [ ] Add parser defaults `target=all` and empty `upstream`, fixed target validation, HTTPS validation, remote workflow input checks, and conditional API dispatch fields.
- [ ] Run the wrapper test and all shell tests; no real workflow dispatch is permitted during verification.

### Task 4: Document Maintainer Usage

**Files:**
- Modify: `.agents/skills/mtls-router-github-test-build/SKILL.md`
- Modify: `docs/BUILD.md`
- Modify: `docs/zh-CN/BUILD.md`

- [ ] Document `--target`, `--upstream`, defaults, input visibility, credential compatibility, and paired CLI/desktop outputs.
- [ ] Add direct `gh workflow run` examples for Windows amd64 validation and explain that tag releases ignore overrides and build all targets.
- [ ] Keep English and Chinese release contracts equivalent.

### Task 5: Full Verification

**Files:**
- Verify all modified files.

- [ ] Run `make test-workflows`.
- [ ] Run `make test-shell`.
- [ ] Run `go test ./...`.
- [ ] Run `go vet ./...`.
- [ ] Run `test -z "$(gofmt -l .)"`.
- [ ] Run `git diff --check` and inspect `git diff` to ensure unrelated untracked files remain untouched.
- [ ] Do not trigger a real GitHub build unless the user explicitly requests it.
