# Personal-device Linux CLI

[中文](zh-CN/LINUX-CLI.md) · [Enrollment operations](ENROLLMENT-OPERATIONS.md)

`codeasier-router` is the new headless Rust client for Linux amd64/arm64. It
shares device identity and streaming transport with the desktop, needs no
Tauri/WebKit, display or keyring, and has no API-key/token/private-key argument.
The historical Go `mtls-router`, manager and setup scripts remain frozen; their
archives are not replacements for this client.

## Download and verify

Choose a published version and architecture. The release supplies exactly one
`CodeasierRouter-cli-linux-<arch>.tar.gz`, its `.sha256` sidecar and a
`.tar.gz.sigstore.json` provenance bundle per architecture. The archive contains
only `codeasier-router`. CLI provenance uses GitHub OIDC/Sigstore; the desktop
retains its separate Tauri updater signature and six-platform feed.

In a download directory, using a recent GitHub CLI:

```sh
version=vX.Y.Z
arch=amd64
gh release download "$version" --repo codeasier/mtls-router --pattern "CodeasierRouter-cli-linux-$arch.*"
sha256sum -c "CodeasierRouter-cli-linux-$arch.sha256"
sha=$(gh api "repos/codeasier/mtls-router/commits/$version" --jq .sha)
gh attestation verify "CodeasierRouter-cli-linux-$arch.tar.gz" --repo codeasier/mtls-router --signer-workflow codeasier/mtls-router/.github/workflows/release.yml --source-ref "refs/tags/$version" --source-digest "$sha" --bundle "CodeasierRouter-cli-linux-$arch.tar.gz.sigstore.json" --deny-self-hosted-runners
tar -xzf "CodeasierRouter-cli-linux-$arch.tar.gz"
```

Do not execute an unverified archive or replace repository/workflow identity with
values supplied by a mirror. Install the verified binary at `~/.local/bin/codeasier-router`.
`--version` prints public build/protocol metadata and requires no certificate.

## Log in without handling certificates

```sh
codeasier-router login --no-browser
codeasier-router identity status
codeasier-router serve
```

Open the printed **public HTTPS link** on a phone or another device. Sign into
GitCode, verify the account, organization, device and short code, then approve.
The original Linux process generates its private key/CSR and installs the bound
certificate automatically. No private key is copied through the browser.
Without `--no-browser`, a GUI session may launch `xdg-open`; headless sessions
still print the link. Model API keys and Agent configuration stay independent.
`MTLS_ROUTER_NO_BROWSER=true` is the environment twin; an explicit boolean flag
such as `--no-browser=false` takes precedence and affects login only.

Default state is `${XDG_STATE_HOME:-$HOME/.local/state}/codeasier-router/`.
`--data-dir` / `MTLS_ROUTER_DATA_DIR` explicitly select another private directory;
`--listen` / `MTLS_ROUTER_LISTEN` select `127.0.0.1:<port>` (default `19099`).
Flags take precedence. Login, status, logout and the service must use the **same
user and directory**. The desktop discovers CLI processes only at its expected
XDG state location, with matching OS identity, deployment and authentication
generation; an arbitrary HTTP-advertised state path is not trusted.

The private `identity/` directory and atomic state contain keys and pending
proofs. Do not copy them into logs/support bundles. The separate `router-state.json`
contains only process/public protocol metadata and is removed only by its owner.

## Renewal, recovery and logout

`identity renew` checks the 7-day renewal window. The foreground service renews
automatically with backoff, swaps complete generations and preserves in-flight
SSE. A lost committed renewal response is recoverable for 24 hours through the
fixed public enrollment origin using the original device key, even if the old
certificate expires; that endpoint cannot issue a replacement operation.
Keep the private pending state until completion. Expired/unrecoverable
authorization requires a fresh browser login.

`logout` first disables the local identity, then attempts remote revocation.
Offline or not-yet-published revocation remains `revocation_pending`, stored
privately for retry by `serve` or another logout. It does not remove an API key or
rewrite Agent files. Only actual gateway evidence is reported as confirmed.

Without a usable certificate, `/version` and `/health` remain available on
loopback (`/health` HTTP 200); model calls return a closed certificate error.
Port conflicts fail without selecting another port or terminating its owner.
`serve` stays in the foreground and handles SIGTERM with bounded shutdown.

## User systemd service and replacement

Use the [user unit](../deploy/cli/codeasier-router.service), not a system service
with a different account. Its explicit state directory is `~/.local/state/codeasier-router`;
adjust both the unit and interactive commands together for a custom XDG location.

```sh
codeasier-router --data-dir "$HOME/.local/state/codeasier-router" login --no-browser
systemctl --user daemon-reload
systemctl --user enable --now codeasier-router.service
```

Place the unit at `~/.config/systemd/user/codeasier-router.service` before these
commands. Do not pass a background flag. For an update, stop this owned user
service, verify the new archive/signature, replace its binary, then restart the
same unit; preserve `identity/` and all existing Agent/API-key files. Review any
old Go service explicitly before stopping it. The new CLI never kills an unknown
port occupant or reuses a shared embedded credential.

Native Linux amd64/arm64 workflows run without GUI packages. Configured workflows
are not evidence of a completed target run; real no-browser approval, live
re-login, upgrade and service replacement still require target-runner acceptance.
