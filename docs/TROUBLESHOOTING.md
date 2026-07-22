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

The desktop uses exactly `127.0.0.1:19099`. It never selects an alternate port or terminates an occupant automatically. The regular Stop action, Quit, launch-at-login, and tray actions do not terminate unknown occupants.

1. Check whether a CLI-managed router is running with `./setup.sh router status` or `.\setup.ps1 router status` from a verified setup package.
2. If the desktop reports an external compatible router, reuse is intentional; the desktop will not own or stop it.
3. By default on every platform, the Router page offers **Force terminate occupant** only after complete identity and current-user ownership are verified. Review the process name and PID, then verify the complete executable path in the confirmation dialog. macOS and Linux have no exception to this rule.
4. On Windows only, the page may instead show an **unverified** PID-only target when the TCP4 owner table finds one unique PID for the exact `127.0.0.1:19099` listener but complete identity is unavailable. This view shows only the PID. It does not prove identity, owner, start time, or executable; even a process with a readable other-user SID can enter this flow. Select Cancel unless you accept that degraded assurance.
5. Force termination is immediate, does not attempt graceful shutdown, and may lose unsaved data. Both modes require an explicit confirmation backed by a short-lived, single-use token. No mode requests administrator or root elevation.
6. For complete identity, the manager revalidates the full listener and process identity immediately before signaling. For Windows PID-only recovery, it immediately rechecks that the same exact port has the same unique PID and that the PID is not the desktop, manager, or a managed router found in readable desktop or CLI lifecycle state. It refuses without signaling if the listener disappeared, changed PID, or became duplicate, wildcard, malformed, or otherwise ambiguous.
7. Windows PID-only recovery skips unreadable lifecycle state, so a managed router may not be recognized as protected. Windows may deny termination, and PID reuse remains possible between the final owner check and termination. During the release wait, a disappeared listener succeeds; a changed or ambiguous listener is reported and no replacement is signaled.
8. After success, the port is released but the router remains stopped. Select Start router manually when ready; launch-at-login does not start it as part of this recovery.
9. If inspection is unavailable, you do not accept a PID-only warning, or termination fails, use operating-system tools to identify the listener and stop or reconfigure it only after independently confirming ownership and identity. Then retry from the Router page.

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

A multi-Agent write is transactional. On a later failure, already replaced targets are restored and diagnostic backups are retained. `ROLLBACK_FAILED` disables further Agent writes because restoration could not be proven.

Do not delete backups or the manager transaction state while investigating. Record changed, backup, and rollback-backup paths from the result, quit the Agents that own those files, and contact the maintainer. Backups may contain old API keys and must not be attached unredacted.

## Autostart or uninstall preparation failed

Settings changes apply to the current user and should not require elevation. If **Prepare for uninstall** cannot verify that autostart is disabled, the application remains open and deletion must wait.

On macOS/Linux, retry **Prepare for uninstall**, verify the application exits, then delete it. On Windows, use the production installer uninstaller, which is responsible for removing the current-user registration. Uninstall does not delete Agent files, backups, logs, or state.
