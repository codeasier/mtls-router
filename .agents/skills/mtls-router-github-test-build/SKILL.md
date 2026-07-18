---
name: mtls-router-github-test-build
description: Use when building mtls-router validation packages from a GitHub commit, branch, or tag with GitHub Secrets, then downloading and verifying Actions artifacts.
---

# mtls-router GitHub Test Build

Build credential-injected test packages only through the repository's GitHub
`Release` workflow. Never request, copy, print, or pass secret values locally.

## When To Use

Use this Skill when the user asks to:

- build mtls-router packages from a branch, tag, or commit;
- use mTLS or signing credentials that exist only in GitHub Secrets;
- run a validation-only release build;
- download a platform artifact such as macOS arm64 after the build.

Do not use it to publish a release, move a tag, upload credentials, or build a
credential-injected binary locally.

## Workflow

1. Confirm `gh auth status` succeeds for the target repository.
2. Choose an explicit test SemVer such as `0.1.0-feature-name.1`.
3. Choose the exact branch, tag, or commit and desired artifact patterns.
4. Choose `all` or one paired CLI/desktop OS-architecture target. Optionally
   choose a non-sensitive HTTPS upstream override.
5. Run `scripts/build-and-download.sh` from this Skill directory.
6. Report the run URL, exact head SHA, downloaded paths, SHA-256 values, and
   signing status.

The script dispatches `.github/workflows/release.yml` in validation-only mode.
Branches are dispatched directly. Tags and commits are resolved to an exact SHA
and built through a temporary remote branch so a tag can never activate the
workflow's publication job. The temporary branch is deleted after success or
failure.

## Usage

```bash
scripts/build-and-download.sh \
  --ref <branch-or-tag-or-commit> \
  --version <test-semver> \
  [--target <all-or-os-arch>] \
  [--upstream <https-url>] \
  [--artifact <name-or-glob>]... \
  [--output <directory>] \
  [--repo <owner/name>]
```

Defaults:

- repository: the current GitHub repository;
- target: `all`; selecting one target builds its CLI and desktop artifacts;
- upstream: the repository `UPSTREAM_URL` variable;
- artifacts: all artifacts from the run;
- output: `desktop/release-artifacts/run-<run-id>` in the repository root.

Known artifact names:

- `mtls-router-desktop-darwin-arm64`
- `mtls-router-desktop-darwin-amd64`
- `mtls-router-desktop-windows-arm64`
- `mtls-router-desktop-windows-amd64`
- `mtls-router-desktop-linux-arm64`
- `mtls-router-desktop-linux-amd64`
- `mtls-router-cli-darwin-arm64`
- `mtls-router-cli-darwin-amd64`
- `mtls-router-cli-windows-arm64`
- `mtls-router-cli-windows-amd64`
- `mtls-router-cli-linux-arm64`
- `mtls-router-cli-linux-amd64`

## Examples

Build macOS arm64 from a branch:

```bash
.agents/skills/mtls-router-github-test-build/scripts/build-and-download.sh \
  --ref feature/agent-models-config \
  --version 0.1.0-agent-models.4 \
  --target darwin-arm64 \
  --artifact mtls-router-desktop-darwin-arm64
```

Build Windows amd64 against a validation upstream:

```bash
.agents/skills/mtls-router-github-test-build/scripts/build-and-download.sh \
  --ref main \
  --version 0.2.0-windows-test.1 \
  --target windows-amd64 \
  --upstream https://router.example.com \
  --artifact mtls-router-cli-windows-amd64 \
  --artifact mtls-router-desktop-windows-amd64
```

Build all packages from a tag without publishing that tag:

```bash
.agents/skills/mtls-router-github-test-build/scripts/build-and-download.sh \
  --ref v0.1.8 \
  --version 0.1.0-v018-validation.1
```

Build desktop artifacts from a commit into a custom directory:

```bash
.agents/skills/mtls-router-github-test-build/scripts/build-and-download.sh \
  --ref 41e729e214553533988566a6a4b950723f916826 \
  --version 0.1.0-commit-validation.1 \
  --artifact 'mtls-router-desktop-*' \
  --output /tmp/mtls-router-validation
```

## Safety Rules

- Never add secret flags or environment variables to the script invocation.
- Treat `--upstream` as public workflow metadata. Never include credentials,
  tokens, or sensitive query parameters in it. The upstream must accept the
  client certificate and CA material held by the repository Secrets.
- Never read local `secrets/` files for this workflow.
- Never dispatch an existing tag directly; doing so could publish a release.
- Never infer success from dispatch alone. Verify the run's exact `headSha`,
  wait for all jobs, and require conclusion `success`.
- Never overwrite an existing output directory.
- Treat mTLS credential injection and package signing as separate results.
  Report `unsigned` or `ad-hoc signed` status even when the build succeeds.
- Do not commit downloaded artifacts. The default output is under the
  repository's ignored `desktop/release-artifacts/` path.
- Do not trigger a second build merely to test the wrapper. Use static checks
  unless the user explicitly asks for a real build.

## Failure Handling

- If GitHub authentication fails, stop and ask the user to authenticate `gh`.
- If the ref cannot be resolved, stop without dispatching.
- If the run cannot be uniquely correlated to the expected SHA, stop and show
  candidate run IDs; do not download artifacts.
- If any requested artifact pattern has no match, fail before creating the
  output directory.
- If checksum verification fails, report the affected artifact and do not call
  the package verified.
- If temporary branch cleanup fails, report the exact
  `agent-test-build/...` branch for manual deletion.

OpenCode loads project Skills at startup. After adding or changing this Skill,
restart OpenCode before expecting automatic Skill discovery.
