# Desktop Troubleshooting

[中文](zh-CN/TROUBLESHOOTING.md)

Use the Router, Logs, and Settings pages before deleting state. Diagnostic summaries and displayed logs are sanitized, but raw local files should still be handled as internal diagnostic data. Never attach Agent configuration or backup contents without reviewing them for API keys.

## Package is blocked or release status is unclear

The workflows build six native desktop packages and inspect each one on a matching target runner, but signing is conditional and package inspection does not install or launch the application.

1. Match the package to the operating system and architecture: Windows x86_64/arm64 NSIS, macOS Intel/Apple Silicon DMG, or Linux x86_64/arm64 AppImage.
2. Verify the package with its `.sha256` file.
3. Read the matching `signing-status-<os>-<arch>.txt`. Windows and macOS may be unsigned when signing credentials were unavailable; macOS may be signed but not notarized when notarization credentials were unavailable. Linux status explicitly reports that package signing is not configured.
4. Ask the distributor for separate successful install/launch evidence from the matching target runner. A successful package-inspection job or status file is not launch evidence.

Do not bypass an operating-system warning unless the package checksum, recorded status, distribution policy, and required target-platform launch evidence have all been reviewed. See [Desktop Application](DESKTOP.md#install) and [Build and Release](BUILD.md#package-verification).

## Sidecar validation failed

Symptoms include `SIDECAR_MISSING`, `SIDECAR_INVALID`, a wrong-architecture report, or a request to reinstall.

1. Quit the application.
2. Confirm that the desktop package matches the operating system and CPU architecture.
3. Reinstall the complete package from the trusted release source.
4. Do not download a standalone manager/router, copy a CLI binary into the application, change executable permissions as a workaround, or disable integrity checks.

The desktop never repairs or downloads sidecars independently. If reinstalling the verified package fails, preserve the error and package identity for the maintainer.

## Port 19099 is occupied

The desktop uses exactly `127.0.0.1:19099`. It never selects an alternate port and never kills an unknown occupant.

1. Check whether a CLI-managed router is running with `./setup.sh router status` or `.\setup.ps1 router status` from a verified setup package.
2. If the desktop reports an external compatible router, reuse is intentional; the desktop will not own or stop it.
3. If it reports an unknown occupant, identify the listener with operating-system tools and stop or reconfigure it only after confirming ownership.
4. Retry from the Router page after the port is free.

A manually started router is intentionally unknown unless complete CLI setup state proves its process identity and deployment/protocol compatibility.

## Stale router state

Stale means recorded PID, process start identity, or executable identity no longer matches. The manager retains state for diagnosis and sends no signal.

1. Inspect the reported PID and executable independently.
2. If a real router is still running, stop it using the tool that owns it or stop it manually only after confirming its identity.
3. Preserve state and logs when the cause is unclear; do not edit a PID to make the state look current.
4. Restart the desktop after the verified process is gone. Contact the maintainer if stale state persists.

## Running with unavailable upstream

This state means the local router process is available but the latest upstream mTLS check failed. A health result older than 30 seconds is shown as stale rather than healthy.

1. Select Retry health check.
2. Check local network, DNS, proxy/VPN policy, clock, and upstream availability.
3. Open Logs and copy the sanitized diagnostic summary.
4. Do not replace embedded certificates manually. Credential or deployment rotation requires a complete replacement release.

The router process state and upstream health are independent. Stopping a healthy local process is not required merely because a transient upstream check is degraded.

## Router exited or manager is unavailable

An unexpected router exit is not restarted in an unlimited loop. If the manager exits, the desktop attempts at most one bounded recovery; failed recovery disables lifecycle commands until the application restarts.

Open Logs, preserve the diagnostic summary, then restart the desktop once. Reinstall if the error identifies a sidecar validation problem. Do not start another router on a different port as a workaround.

## Agent configuration is unavailable or not writable

- Claude Code, opencode, and Codex are always available as supported configuration targets; the desktop does not install or launch their CLIs.
- An empty `command` means only that the manager process cannot find the Agent CLI. It does not prevent creating or updating the configuration.
- Confirm `CLAUDE_CONFIG_DIR`, `OPENCODE_CONFIG`, or `CODEX_HOME` points to the intended current-user location before launching the desktop.
- Restore current-user write access to the configuration file and its directory. Do not run the desktop as administrator or root to bypass ownership problems.

## Invalid Agent configuration

`CONFIG_INVALID` means the existing JSON, JSONC, TOML, or Codex auth JSON cannot be safely interpreted for the requested operation. No file is changed.

1. Open the path shown by detection.
2. Fix its syntax with the owning Agent stopped if that Agent may write concurrently.
3. For canonical `~/.config/opencode/opencode.jsonc` with no explicit `OPENCODE_CONFIG`, check whether an existing sibling `opencode.json` conflicts with the proposed JSONC-to-JSON migration.
4. For an explicit `.jsonc` `OPENCODE_CONFIG`, check that the exact override path and its parent are writable; the sibling `opencode.json` is unrelated and is not a fallback.
5. Refresh detection and generate a new preview.

The manager preserves unrelated supported settings, but it does not attempt to guess repairs for invalid syntax.

## Preview became stale

`PREVIEW_STALE` means a selected target changed after preview. The write is rejected before mutation.

Return to configuration and generate a new preview from the retained key-free catalog/config when the client permits it. If the catalog token is also stale, enter the key and discover models again. For an explicit `OPENCODE_CONFIG`, verify that the exact override path was not changed after preview. Do not retry with an old revision token.

## Model configuration errors

All model errors fail closed: no Agent or last-applied sidecar file is changed,
and there is no static-model, cached-catalog, existing-model, or substitute-model
fallback.

| Code | Action |
|---|---|
| `MODEL_AUTH_FAILED` | Re-enter the API key; the catalog endpoint returned 401 or 403. |
| `MODEL_DISCOVERY_FAILED` | Check the trusted local router, network, and upstream service, then retry. Redirects and non-auth HTTP failures use this code. |
| `MODEL_RESPONSE_INVALID` | Report an upstream service-contract failure; the successful response was malformed, excessive, or not standard `data[].id` JSON. |
| `MODEL_CATALOG_EMPTY` | Confirm that the account/key has visible models, then retry discovery. |
| `MODEL_CATALOG_STALE` | Rediscover models. Router address, deployment, protocol, owner, or token trust state changed. |
| `MODEL_CONFIG_INVALID` | Correct the canonical model config at the reported JSON Pointer. Do not place credentials, URLs, provider/header fields, or arbitrary Agent config in `extra`/`options`. |
| `MODEL_NOT_AVAILABLE` | A selected model disappeared during the write-time refresh. Rediscover and select explicitly; the manager will not substitute one. |
| `MANAGED_CONFIG_DRIFT` | Generate a new preview, inspect only the listed managed namespaces, and explicitly approve overwrite or cancel. |
| `MODEL_STATE_INVALID` | Preserve Agent backups and resolve any transaction journal first. Move the entire invalid `agent-transactions` directory aside only after review; do not replace just the signing key or sidecar. |
| `AGENT_OPERATION_BUSY` | Wait for the other desktop/CLI Agent operation to finish, then retry. Do not delete the lock or transaction state. |
| `CODEX_AUTH_UNSUPPORTED` | Resolve forced ChatGPT login, managed policy, or incompatible credential-store policy before retrying. The manager will not weaken policy or delete OS keyring credentials. |

`configured=true` in detection is not an authorization result. It means only
that local managed structure is complete. Use model discovery to check current
key visibility. Catalog refresh is manual; re-enter Agent configuration and
supply the key. See [Agent Model Configuration](AGENT_MODELS.md).

## Write or rollback failed

A multi-Agent write is transactional. On a later failure, already replaced targets are restored and diagnostic backups are retained. `ROLLBACK_FAILED` disables further Agent writes because restoration could not be proven.

Do not delete backups or the manager transaction state while investigating. Record changed, backup, and rollback-backup paths from the result, quit the Agents that own those files, and contact the maintainer. Backups may contain old API keys and must not be attached unredacted.

## Autostart or uninstall preparation failed

Settings changes apply to the current user and should not require elevation. If **Prepare for uninstall** cannot verify that autostart is disabled, the application remains open and deletion must wait.

On macOS/Linux, retry **Prepare for uninstall**, verify the application exits, then delete it. On Windows, use the production installer uninstaller, which is responsible for removing the current-user registration. Uninstall does not delete Agent files, backups, logs, or state.
