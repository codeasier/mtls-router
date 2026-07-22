# Desktop Application

[中文](zh-CN/DESKTOP.md)

The Tauri desktop application is a current-user control panel for the fixed-service `mtls-router`. It packages architecture-matching `mtls-router-manager` and `mtls-router` sidecars; it does not install either sidecar into `PATH` and does not provide certificate, upstream, or sidecar replacement controls.

> The checked-in CI and release workflows build six native desktop packages: Windows x86_64/arm64 NSIS installers, macOS Intel/Apple Silicon DMGs, and Linux x86_64/arm64 AppImages. Each package job runs inspection on a matching target runner. Release signing is conditional on platform credentials, and macOS notarization/stapling additionally requires complete Apple notarization credentials; the per-target status file records the result. Inspection does not install or launch the package, so separate successful target-runner launch evidence remains required. See [Build and Release](BUILD.md).

## Install

Obtain the package for the operating system and CPU architecture from the trusted internal release channel. Do not move, replace, or run a sidecar separately from its desktop package.

- Windows: run the current-user installer matching x86_64 or arm64. Administrator elevation is not required by the application design.
- macOS: open the DMG matching Intel or Apple Silicon, then drag `CodeasierRouter.app` onto the Applications shortcut.
- Linux: make the matching x86_64 or arm64 AppImage executable and launch it as the current user.

If the operating system reports that the package is unsigned, unnotarized, damaged, or from an unknown publisher, stop and verify the release status with the distributor. Do not bypass platform security based only on the package filename.

The release asset set contains a `.sha256` file and a `signing-status-<os>-<arch>.txt` file for each desktop package. These prove the recorded checksum and signing/notarization result, not successful installation or launch. Obtain the separate target-platform launch evidence required by [Package verification](BUILD.md#package-verification).

## First launch

On first launch the application:

1. Verifies the packaged manager and router sidecars against the target architecture and build-time SHA-256 values, then checks the manager target, version, deployment ID, and management protocol.
2. Inspects `127.0.0.1:19099`.
3. Reuses a trusted, compatible router started by the CLI setup scripts, or starts the packaged router when the port is free.
4. Opens the Router page and checks process availability separately from upstream mTLS health.
5. Enables current-user launch-at-login by default. It can be disabled immediately in Settings without administrator privileges.

First launch never changes Claude Code, opencode, or Codex files. A second application launch activates the existing window instead of starting a second manager or router.

If either sidecar is missing, non-executable, altered, or for the wrong architecture, startup fails closed and reports that reinstallation is required. The application does not download or replace a sidecar by itself. Reinstall the complete package from the trusted source.

## Router ownership and state

The Router page distinguishes the local process from upstream health. A running process can therefore be healthy, degraded, or based on a health result older than 30 seconds.

- **Desktop router:** the application supervises a foreground child. Stop and Quit may stop it only after PID, start identity, executable identity, and ownership checks pass.
- **External router:** reuse is allowed only for a CLI setup-managed router whose recorded process identity, `deployment_id`, and `management_protocol_version` match the desktop build. A manually started process is not trusted from `/version` response shape alone.
- **Unknown port occupant:** the application never terminates it automatically and never switches to another port. Complete identity is the default recovery requirement on every platform: the manager verifies current-user ownership, process name, PID, start identity, complete executable path, and listener identity before the Router page offers **Force terminate occupant**. The confirmation dialog shows the verified process details and warns that termination is immediate and may lose unsaved data. macOS and Linux behavior is unchanged and always requires this complete identity.
- **Windows PID-only exception:** when complete identity cannot be established, Windows may still offer the recovery action only if its TCP4 owner table identifies one unique PID for the exact `127.0.0.1:19099` listener. The panel and confirmation dialog show only the PID and explicitly warn that identity, owner, start time, and executable are unverified. This means a process with a readable other-user SID can still enter this degraded flow. Confirmation uses a short-lived, single-use token; immediately before termination the manager requires the same exact port to have the same unique PID and rechecks that it is not the desktop, manager, or a managed router identified by readable desktop or CLI lifecycle state. A listener that disappeared, changed PID, or became duplicate, wildcard, malformed, or otherwise ambiguous is refused without a signal. Unreadable lifecycle state is skipped, so an unrecognized managed router remains a risk; Windows may deny termination, and PID reuse between the owner recheck and termination cannot be eliminated. The action never requests elevation and never starts the router after releasing the port.
- **Manual fallback:** if no inspection mode is eligible, both modes are unavailable, the warning is unacceptable, or termination fails, identify and stop or reconfigure the listener with operating-system tools, then retry.
- **Stale state:** a PID or executable mismatch is reported as stale and no signal is sent. Inspect the process and state before making a manual cleanup.
- **Degraded or stale health:** the router process may still accept local connections, but upstream service is not currently proven reachable. Use Retry health check and inspect Logs; do not treat stale health as healthy.

See [Troubleshooting](TROUBLESHOOTING.md) for recovery steps.

## Tray, close, and quit

Closing the main window hides it in the system tray and leaves the router unchanged. Use the tray icon to open the window, start or stop an eligible router, open logs, or quit.

Quit is different from closing the window. Quit stops a verified desktop-owned router and exits the application. It never stops a compatible external router or an unverified process. The regular Stop action remains unavailable for an unknown occupant, and the tray provides no force-termination action; occupant termination is available only through explicit confirmation on the Router page. Launch-at-login starts the desktop application on the next user login and applies the same discovery and ownership rules; it never terminates an occupant or blindly starts a second router.

## Configure Agents

The Agents page supports Claude Code, opencode, and Codex. Detection is metadata-only: it reports path, format, existence, writability, and configured or invalid state without returning stored API-key values. It respects `CLAUDE_CONFIG_DIR`, `OPENCODE_CONFIG`, and `CODEX_HOME`; an existing Codex home directory also identifies a Codex desktop installation.

Normal **Merge** preserves supported unrelated configuration. **Back up and rebuild** is a separate destructive recovery action shown only for eligible syntax-invalid files. It replaces the complete approved Agent file set with managed-only output; unrelated settings, comments, formatting, and valid companion-file metadata are lost. Its backups may contain API keys. Review this loss and protect every backup before continuing.

Agent configuration is an explicit key-before-discovery sequence:

1. Refresh detection. Select **Merge** for valid, writable Agents. For an invalid Agent, select **Back up and rebuild** only when detection marks it eligible; other invalid configurations must be repaired manually.
2. Enter the API key. React clears the field immediately; Rust retains it only in one transient, non-replayable flow.
3. Discover the manager's authenticated, build-filtered model catalog through the trusted local router. By default, valid IDs containing ASCII `/` are excluded; a release built with `SIMPLIFY=False` retains them. The same filtered catalog is authoritative for every selected Agent, imported config, preset, preview, and refresh. This is an immutable manager build policy, not a runtime preference or a restriction on proxy route support. The desktop never selects the first model, infers a choice from model names or capabilities, or substitutes a model. A visible build preset may supply an editable initial section only after the manager validates all of that section's exact IDs against this catalog.
4. Initialize each Agent independently with `existing > preset > empty`, showing whether a section came from existing configuration or the recommended preset and listing missing base IDs for any unavailable complete preset section. Configure Agent-native choices: Claude primary/role inheritance plus optional display names and Standard/1M context, an opencode model subset/default/options, and one Codex model/options. Claude includes a localized **Enable Fable** control. Fable starts disabled for an empty Claude form; enabling it creates an inherit-primary selection and exposes the same inherit/explicit model, display-name, and Standard/1M controls, while disabling it removes the complete Fable selection and its metadata. Existing Claude wins over preset Claude as one complete section, so preset Fable is not merged into an existing Claude section that omits it. Preset values remain editable and imply no approval. Unset optional fields remain omitted. A key-free canonical model config can be imported or exported; import replaces the complete current form rather than merging with existing or preset values and preserves enabled or absent Fable through export, preview, and write.
5. Generate a structured preview and review redacted fragments plus every create, replace, preserve, migration, drift approval, state, and backup operation. Normal merge without explicit `OPENCODE_CONFIG` migrates canonical `~/.config/opencode/opencode.jsonc` to sibling `opencode.json`; an existing sibling is a migration collision. An explicit `.jsonc` override is normalized in place. Rebuild instead replaces its approved JSON or JSONC path in place as strict JSON and performs no sibling migration. Codex rebuild always replaces both `config.toml` and `auth.json`, creating a missing companion and discarding valid companion metadata; every existing file in that pair requires a backup.
6. Approve and write. Rebuild Agents appear in a separate destructive confirmation and must exactly match the preview. The manager consumes the in-memory key to refresh the catalog before creating any write artifact, then review changed and actual backup paths.

Rebuild eligibility requires at least one syntax-invalid file and no other blocker across the complete managed file set. Unsupported but valid structures, unreadable or oversized files, non-regular files, links, non-writable paths, unavailable parents, pending transaction recovery, and disabled writes are ineligible. Valid syntax and parser compatibility problems must be fixed rather than rebuilt. Merge and rebuild can be selected independently for different Agents in one transaction; there is no automatic recovery, parse bypass, global force overwrite, or fallback from merge failure to rebuild.

Rebuild output is deliberately managed-only: Claude `settings.json` contains only the managed `env`; opencode strict JSON contains only root `model` and `provider.mtls-router`; Codex `config.toml` contains only its managed model/provider settings and `auth.json` contains only `auth_mode` and `OPENAI_API_KEY`. See [Agent Model Configuration](AGENT_MODELS.md#destructive-rebuild-recovery) for the exact output and eligibility contract.

Immediately before writing, the manager verifies that files still match the approved revision. `PREVIEW_STALE` means a target changed; no write begins and a fresh preview is required. Existing files are backed up before replacement. A multi-Agent write is one transaction: failure causes already changed files to be rolled back, while diagnostic backups are retained. If rollback cannot be proven, further Agent writes fail closed.

Backups remain beside the original configuration and may contain an old API key. They are sensitive recovery artifacts: protect, retain, or remove them with the same care as the original Agent file. Preview shows only planned sibling backup patterns; a successful result shows actual paths after verified byte-for-byte backups are created. Neither displays backup contents, and a failed operation may show only its error. Never manually restore while a transaction journal or unresolved recovery exists; contact the maintainer because changing a target can prevent recovery from proving its identity. After recovery is resolved, stop the owning Agent, preserve current files, verify that the original path and parent retain their expected current-user, non-link identity, then use a private same-directory temporary file and atomic replacement. For Codex, restore only files that existed before the transaction and remove a newly created companion only when the reviewed operation proves its prior absence; otherwise contact the maintainer.

Detection's `configured` state means only that local managed fields are structurally complete and internally consistent. It does not prove the selected models are currently authorized. Re-enter configuration and supply a key for a manual refresh; there is no background catalog synchronization or Agent-file rewrite. Catalog/auth/validation failures, a disappeared model, drift without approval, or invalid ownership state fail closed with no static/cached fallback or partial write. See [Agent Model Configuration](AGENT_MODELS.md) for the service contract, canonical schema, omission, migration, and ownership rules.

For Claude, canonical configuration stores the authenticated base model ID and optional exact `context: "1m"` separately. Enabled Fable renders `ANTHROPIC_DEFAULT_FABLE_MODEL` and an optional `ANTHROPIC_DEFAULT_FABLE_MODEL_NAME`; absent Fable renders and claims neither. Preview shows collisions before an enabled Fable claims an existing unowned value, and disabling Fable removes only stale paths proven previously manager-owned while preserving never-owned manual keys. The manager appends `[1m]` only while rendering Claude's model environment values; it does not infer 1M support or write `CLAUDE_CODE_DISABLE_1M_CONTEXT`. A runtime rejection does not cause fallback or rewrite. The Fable alias requires Claude Code 2.1.170 or newer. Separately, numeric custom-model context overrides work directly for unknown model names on Claude Code 2.1.193 or newer; older versions may ignore them. Preset discovery itself writes no Agent file or manager transaction state, and preset data is not stored in the desktop's secret-bearing flow.

## API-key boundary and limitations

The desktop submits the entered key for `agent.models`, clears the password field immediately, and keeps the key only in zeroizing Rust transient flow state until one `agent.write`. Secret-bearing calls are never automatically replayed after timeout, malformed response, manager restart, or uncertain delivery. The key is not intentionally placed in desktop or manager persistent state, process arguments, environment variables, logs, diagnostics, model config, catalog/revision tokens, preview responses, or write responses.

The selected Agent configuration must persist the key where that Agent requires it. User-approved recovery backups may also persist an older key. Clearing JavaScript and Rust application references is best effort and is not a guarantee of forensic erasure from process or operating-system memory.

`MTLS_ROUTER_OPENAI_API_KEY` has been removed and no longer supplies a key. For exact noninteractive manager automation, see [stdin manager automation](#stdin-manager-automation).

## Stdin manager automation

Automation must invoke the receipt-verified installed `mtls-router-manager serve`, require `manager.info` protocol `3`, call `agent.models` with the transient key, construct canonical model config, call key-free `agent.render` or `agent.preview`, and finally call `agent.write` with the revision token, explicit approvals, and transient key. The key must not be an argument, exported environment variable, model-config value, log value, or temporary request file. Catalog tokens are intentionally verifiable across one-shot manager processes. See the exact [protocol v3 automation contract](AGENT_MODELS.md#protocol-v3-automation).

## Credential model

Each production package is bound to one service environment. The router sidecar contains a shared client certificate, shared private key, upstream CA, and default upstream URL. A user who can obtain the package can extract those embedded values; packaging and the desktop UI cannot prevent this. Distribute packages only to trusted internal users.

Revocation or rotation requires a replacement release built with new credential material, distribution of the complete replacement package, and server-side rejection of the old credential. There is no runtime certificate import, profile switch, sidecar update, or automatic application update.

The local listener is plain HTTP on trusted localhost. Do not expose the management endpoints or listener publicly.

## Uninstall

- Windows: quit the application, then uninstall it from Windows Settings. The production installer must remove the desktop application's current-user launch-at-login registration during uninstall.
- macOS and Linux: open Settings, select **Prepare for uninstall**, and confirm. Wait for the application to remove current-user launch-at-login and exit. Only then delete the macOS application or Linux AppImage.

No uninstall path restores, deletes, or rewrites Agent configurations, sensitive backups, router logs, application state, or diagnostic state. Remove any of those separately only after reviewing what must be retained for recovery or diagnosis.
