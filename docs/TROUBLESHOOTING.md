# Desktop Troubleshooting

[中文](zh-CN/TROUBLESHOOTING.md)

Use the Router, Logs, and Settings pages before deleting state. Diagnostic summaries and displayed logs are sanitized, but raw local files should still be handled as internal diagnostic data. Never attach Agent configuration or backup contents without reviewing them for API keys.

## Package is blocked or release status is unclear

The workflows build six native desktop packages and inspect each one on a matching target runner, including an initialization-only startup smoke test, but signing is conditional and package inspection does not install or normally launch the application.

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

The desktop uses exactly `127.0.0.1:19099`. It never selects an alternate port or terminates an occupant automatically. The regular Stop action, Quit, launch-at-login, and tray actions do not terminate unknown occupants.

1. Check whether a CLI-managed router is running with `./setup.sh router status` or `.\setup.ps1 router status` from a verified setup package.
2. If the desktop reports an external compatible router, reuse is intentional; the desktop will not own or stop it.
3. Read the structured diagnostic. `force_terminate` means the target is eligible for explicit confirmation. `manual_stop_required` means a service, insufficient privilege, or another user must be handled outside the app. `insufficient_privilege` includes terminate access denied by OS protection such as Windows PPL; access denial does not reliably identify PPL. `unavailable` means a lifecycle-known target is `protected_process` or its identity cannot be established safely. Blocked diagnostics contain no confirmation token.
4. By default on every platform, **Force terminate occupant** requires complete listener identity and current-user process ownership. Review the process name and PID, then verify the complete executable path in the confirmation. macOS and Linux have no PID-only exception.
5. On Windows, a readable different SID is rejected as `different_user` and never downgraded to PID-only. PID-only may be considered when the SID or complete process identity is unavailable, but only when the TCP4 owner table finds one unique PID for the exact listener, protected desktop/manager/managed-router targets are excluded, and a non-destructive terminate-access preflight succeeds. The view shows only the PID because identity, owner, start time, and executable remain unverified. Select Cancel unless you accept that residual risk.
6. If a Windows Service or Linux systemd service is identified, use the displayed bounded identifier and manual guidance. Windows examples use `services.msc` or `sc.exe stop`; user services use `systemctl --user stop`; system services use `sudo systemctl stop`. A copied Windows command is safely quoted for Administrator PowerShell only and must not be pasted into `cmd.exe`. The app only renders and copies this text: it does not execute commands, control SCM/systemd, or request elevation. For privilege or user blockers, stop the process from its owning identity and privilege context; do not run the desktop app as administrator or root.
7. macOS does not infer a launchd label. A verified ordinary same-user process may be terminated, but if it returns, use Activity Monitor or `launchctl` to identify the responsible job before stopping it manually.
8. Force termination is immediate, does not attempt graceful shutdown, and may lose unsaved data. Immediately before signaling, complete-identity mode revalidates the full identity; Windows PID-only mode repeats exact-owner and protection checks and reacquires terminate access. A disappeared, changed, duplicate, wildcard, malformed, ambiguous, or newly inaccessible target is refused without signaling. Unreadable Windows lifecycle state and PID reuse remain residual PID-only risks.
9. Manager success returns exactly `termination: process_terminated` and `port_state: released`, but the evidence depends on verification mode. Complete-identity mode proves the confirmed process identity became absent and the port was observed released. Windows PID-only proves the termination request succeeded and the original listener PID disappeared from the exact port; it does not independently prove that process fully ended. The desktop then samples status periodically for about 10 seconds. A released result means sampled checks found the port released and detected no reoccupation, not continuous observation or stable-release proof; a sampled new occupant is reported as reoccupation and inspected again. Starting the router cancels observation.
10. `OCCUPANT_PERMISSION_DENIED` means terminate access was lost or denied at confirmation. `OCCUPANT_TERMINATION_FAILED` means termination was attempted but the original process did not exit as required. `PORT_RELEASE_TIMEOUT` means the manager could not prove release within its short synchronous window. Inspect again after any of these errors; no replacement process is terminated.
11. If inspection or recovery is unavailable, you do not accept the warning or guidance, or termination fails, independently identify and stop or reconfigure the listener with operating-system tools, then retry from the Router page.

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

If a freshly built or installed manager exits before accepting protocol requests
with `invalid embedded Agent model preset`, its nonempty build-time
`AGENT_MODEL_PRESET_BASE64` is invalid. The manager deliberately reports no raw
encoded or decoded preset content and fails before Agent transaction recovery.
Users should reinstall a corrected complete release; maintainers should correct
or clear the repository variable and rebuild both standalone and desktop
manager artifacts. Do not patch the packaged sidecar or inject the preset into
the router.

If it exits with `invalid embedded simplify value`, the manager was directly
linked with an invalid or empty `modelcatalog.Simplify` value. Users should
reinstall a corrected complete release. Maintainers should use the repository
build scripts with unset/empty `SIMPLIFY` or an ASCII-case spelling of `true` or
`false`, then rebuild both standalone and desktop manager artifacts with the
same normalized value. The scripts reject all other values before compilation;
do not patch the sidecar or add `SIMPLIFY` to router runtime configuration.

## Agent configuration is unavailable or not writable

- Claude Code, opencode, and Codex are always available as supported configuration targets; the desktop does not install or launch their CLIs.
- Agent detection does not search `PATH` or report CLI installation. Protocol v4 keeps the compatibility fields and now returns fixed `detected=true` and `command=""` values.
- Confirm `CLAUDE_CONFIG_DIR`, `OPENCODE_CONFIG`, or `CODEX_HOME` points to the intended current-user location before launching the desktop.
- Restore current-user write access to the configuration file and its directory. Do not run the desktop as administrator or root to bypass ownership problems.

## Invalid Agent configuration

`CONFIG_INVALID` means the existing JSON, JSONC, TOML, or Codex auth JSON cannot be safely interpreted for the requested operation. No file is changed.

Prefer manual repair and normal preservation merge. Stop the owning Agent, open every path shown by detection, fix the syntax, refresh detection, and generate a new preview. A syntactically valid but unsupported root structure is not recoverable by rebuild. Neither is a parser compatibility defect: valid forms such as accepted BOM-prefixed JSON or quoted TOML keys must remain valid and merge normally rather than being discarded.

The desktop offers **Back up and rebuild** only when at least one file is syntax-invalid and every file in that Agent's complete managed set has no other recovery blocker. Unreadable, oversized, non-regular, linked, or non-writable files; unavailable parents; unsupported valid structures; pending transaction recovery; and disabled writes are ineligible. For Codex, this assessment always covers both `config.toml` and `auth.json`.

**Rebuild is destructive.** It discards unrelated settings, comments, formatting, and valid companion metadata. Claude is replaced with managed `env` only; opencode is replaced at the approved path with strict managed-only JSON, including when that path ends in `.jsonc`; Codex replaces both files. There is no automatic or global force-overwrite fallback.

If that loss is acceptable:

1. Stop every selected Agent and refresh detection.
2. Select **Back up and rebuild** only for the eligible invalid Agent; valid Agents may independently remain on **Merge**.
3. Enter the key, configure models, and review every redacted managed fragment, affected path, create/replace operation, and planned backup pattern.
4. Confirm the separate destructive prompt. Write rechecks syntax, safety, complete file sets, revisions, and the catalog before creating artifacts.
5. On success, record every changed and actual backup path. Each existing approved target was backed up byte-for-byte before any replacement. On failure, preserve the error even if no artifact paths are displayed.

Backups are sensitive and may contain API keys or other credentials. Never attach or share them unredacted. To restore, stop the owning Agent and preserve any transaction journal and current files needed for diagnosis. Verify that the original path and parent still have the expected current-user, non-link identity, then write the backup through a private same-directory temporary file and atomically replace the target; never copy through a link. For Codex, restore each file that existed before the transaction. Remove a companion only when the reviewed preview/result proves that the transaction created it; if its prior state is uncertain or recovery is unresolved, contact the maintainer instead. Refresh detection and preview again, and do not delete remaining backups or transaction state until recovery is proven complete.

## Preview became stale

`PREVIEW_STALE` means a selected target changed after preview. The write is rejected before mutation.

Return to configuration and generate a new preview from the retained key-free catalog/config when the client permits it. If the catalog token is also stale, enter the key and discover models again. For rebuild, a manually repaired source, changed companion existence, new blocker, or any path/revision change also makes the approval stale and may make rebuild ineligible. Do not retry with an old revision token or switch from merge to rebuild without a new preview and destructive confirmation.

For cleanup, no key or catalog rediscovery is needed. Select **Repreview cleanup** to rebuild the plan from the current Agent files and sidecar. Do not retry the old cleanup token or replay a cleanup write after an ambiguous transport failure.

## Managed cleanup is unavailable or failed

Automatic cleanup is shown only for an Agent with a valid last-applied sidecar ownership entry. It never guesses ownership from a `mtls-router` provider name and never uses the router, model catalog, or global API key.

| Code or state | Action |
|---|---|
| `not_managed` / `AGENT_NOT_MANAGED` | No trusted ownership entry exists. Do not create or edit the sidecar to enable cleanup. Stop the Agent, preserve current files and backups, then manually remove only fields whose ownership you can independently establish. |
| `model_state_invalid` / `MODEL_STATE_INVALID` | Preserve the complete `agent-transactions` directory and Agent files. Resolve any journal/recovery problem; do not replace only the signing key or sidecar. |
| `writes_disabled` / `ROLLBACK_FAILED` | Cleanup and normal writes remain blocked until transaction recovery is proven. Restarting or deleting the journal does not bypass the failure. |
| `CONFIG_INVALID` | The recorded file, path identity, JSON/JSONC/TOML structure, or managed provider container cannot be cleaned safely. Repair it manually, refresh detection, and request a new cleanup preview. No backup or mutation begins. |
| `CONFIG_NOT_WRITABLE` | Restore current-user write access to every recorded target and parent. Do not elevate the desktop application. |
| `MANAGED_CONFIG_DRIFT` | Review the listed managed paths and file effects. Confirm the dedicated drift checkbox only if those namespaces should be removed; confirmation does not authorize unrelated deletion. |
| `PREVIEW_STALE` | A recorded Agent file or the sidecar changed after preview. Repreview; do not reuse the token. |
| `AGENT_OPERATION_BUSY` / `OPERATION_TIMEOUT` | Wait for the other operation or bounded deadline to finish, refresh detection, and preview again. Do not delete lock or transaction state. |
| `BACKUP_FAILED` / `WRITE_FAILED` / `ROLLBACK_FAILED` | Follow [Write or rollback failed](#write-or-rollback-failed). Cleanup uses the same verified backup, journal, rollback, and fail-closed recovery engine as configuration writes. |

Cleanup removes only the selected Agent's proven managed provider/model/authentication paths. It deliberately retains the desktop global API key, all historical backups, and new cleanup backups. A recorded file that is already absent is treated as cleaned after drift approval and gets no backup or mutation; other read errors remain `CONFIG_INVALID`. A present file shown as `delete` is backed up and removed because its semantic root is already empty or would otherwise be empty; a `replace` preserves remaining user data. Backups can contain keys or other credentials and must not be shared unredacted. For Codex, cleanup removes file authentication fields but does not remove OS-keyring credentials or reconstruct auth fields displaced by an earlier write.

## Model configuration errors

All model errors fail closed: no Agent or last-applied sidecar file is changed,
and there is no static-model, cached-catalog, existing-model, or substitute-model
fallback.

An unavailable build preset is reported differently: `agent.models` remains
successful, omits the complete affected Agent preset section, and lists its
missing base IDs under `preset.unavailable_agents` with
`MODEL_NOT_AVAILABLE`. Existing sections and valid preset sections for other
Agents remain usable. Select the model explicitly or ask the distributor for an
updated release; the manager will not partially use, repair, or substitute the
unavailable section. A preset notice is an editable recommendation, not proof
that a model supports Claude 1M context.

The manager's build-filtered catalog is the availability boundary. With the
default build policy, valid IDs containing ASCII `/` are intentionally absent;
existing and preset sections that reference them are reported unavailable, and
an imported config that selects one fails `MODEL_CONFIG_INVALID`. Use a model
shown by the current manager, or ask the distributor for a release built with
`SIMPLIFY=False`; this cannot be changed as a runtime preference. Full-width
slash and backslash do not trigger the filter, and proxy route support is
unchanged.

| Code | Action |
|---|---|
| `MODEL_AUTH_FAILED` | Re-enter the API key; the catalog endpoint returned 401 or 403. |
| `MODEL_DISCOVERY_FAILED` | Check the trusted local router, network, and upstream service, then retry. Redirects and non-auth HTTP failures use this code. |
| `MODEL_RESPONSE_INVALID` | Report an upstream service-contract failure; the successful response was malformed, excessive, or not standard `data[].id` JSON. The complete response is validated before filtering, so even a malformed ID that would otherwise be filtered remains this error. |
| `MODEL_CATALOG_EMPTY` | The response had no valid IDs retained by this manager. Confirm account/key visibility; if all visible IDs contain ASCII `/`, use a release built with `SIMPLIFY=False` or ask the service to expose suitable IDs, then rediscover. The same error during write-time refresh means no write began. |
| `MODEL_CATALOG_STALE` | Rediscover models. Router address, deployment, protocol, owner, or token trust state changed. |
| `MODEL_CONFIG_INVALID` | Correct the canonical model config at the reported JSON Pointer. An imported selection outside the filtered catalog remains invalid. Do not place credentials, URLs, provider/header fields, or arbitrary Agent config in `extra`/`options`. |
| `MODEL_NOT_AVAILABLE` | A selected model disappeared from the filtered catalog during the write-time refresh. Rediscover and select explicitly; the manager will not substitute one. If refresh retained no IDs at all, the result is `MODEL_CATALOG_EMPTY` instead. |
| `MANAGED_CONFIG_DRIFT` | Generate a new preview, inspect only the listed managed namespaces, and explicitly approve overwrite or cancel. |
| `MODEL_STATE_INVALID` | Preserve Agent backups and resolve any transaction journal first. Move the entire invalid `agent-transactions` directory aside only after review; do not replace just the signing key or sidecar. |
| `AGENT_OPERATION_BUSY` | Wait for the other desktop/CLI Agent operation to finish, then retry. Do not delete the lock or transaction state. |
| `CODEX_AUTH_UNSUPPORTED` | Resolve forced ChatGPT login, managed policy, or incompatible credential-store policy before retrying. The manager will not weaken policy or delete OS keyring credentials. |

`configured=true` in detection is not an authorization result. It means only
that local managed structure is complete. Use model discovery to check current
key visibility. Catalog refresh is manual; re-enter Agent configuration and
supply the key. See [Agent Model Configuration](AGENT_MODELS.md).

For Claude, canonical `context` accepts only exact `"1m"`; put the authenticated
base ID in `model`, never an ID ending in `[1m]`. The manager appends that suffix
only when rendering Claude settings and does not infer capability. If Claude or
the upstream rejects 1M at runtime, choose Standard or another explicitly
validated selection and write a new preview; there is no automatic fallback or
configuration rewrite.

Fable is optional. If it is enabled, its explicit model must remain in the
authenticated catalog; an unavailable Fable model makes the complete Claude
section unavailable rather than dropping only Fable. If an existing manual
`ANTHROPIC_DEFAULT_FABLE_MODEL` or `ANTHROPIC_DEFAULT_FABLE_MODEL_NAME` causes a
collision, review the preview and approve ownership only if the manager should
replace that exact value. Disabling Fable preserves never-owned manual keys and
removes only stale paths proven manager-owned by the prior sidecar. Use Claude
Code 2.1.170 or newer for the Fable alias. This is separate from numeric context
override compatibility: direct numeric overrides for unknown custom model names
require Claude Code 2.1.193 or newer and may be ignored by older versions.

## Write or rollback failed

Agent merge/rebuild writes and per-Agent cleanup writes are transactional:

- `BACKUP_FAILED` occurs before replacement. No target changed, created backup artifacts were cleaned up, and writes remain available after the underlying file/directory problem is corrected. If cleanup cannot be proven, the result is `ROLLBACK_FAILED` instead.
- `WRITE_FAILED` with successful rollback means every changed target and manager sidecar was restored. Diagnostic and original backups remain for review.
- `ROLLBACK_FAILED` means restoration or startup transaction recovery could not be proven. Further Agent writes and rebuild eligibility are disabled until the unresolved recovery is repaired; restarting does not bypass it.

Do not delete backups or the manager transaction state while investigating. Record any changed, backup, and rollback-backup paths shown by a successful result; a failed desktop operation may expose only the error even when local diagnostic artifacts remain. Quit the Agents that own those files and contact the maintainer. Backups may contain old API keys and must not be attached unredacted. Never force another write, remove only part of a Codex pair, or replace the signing key/sidecar to bypass unresolved recovery.

## Autostart or uninstall preparation failed

Settings changes apply to the current user and should not require elevation. If **Prepare for uninstall** cannot verify that autostart is disabled, the application remains open and deletion must wait.

On macOS/Linux, retry **Prepare for uninstall**, verify the application exits, then delete it. On Windows, use the production installer uninstaller, which is responsible for removing the current-user registration. Uninstall does not delete Agent files, backups, logs, or state.
