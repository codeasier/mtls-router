import type { TranslationKey } from "./zh-CN";

export const en: Record<TranslationKey, string> = {
  "app.homeAria": "CodeasierRouter home",
  "app.navigationAria": "Main navigation",
  "app.localMode": "Local mode",
  "app.safeControlPlane": "Safe control plane",
  "app.controlDesk": "CONTROL DESK",
  "app.sidebarCollapse": "Collapse sidebar",
  "app.sidebarExpand": "Expand sidebar",
  "app.sidebarExpandShort": "Expand",
  "app.updateBadgeAria": "A new version is available",
  "nav.router": "Router control",
  "nav.routerShort": "Router",
  "nav.agents": "Agent configuration",
  "nav.agentsShort": "Agents",
  "nav.apiKeys": "API key",
  "nav.apiKeysShort": "Key",
  "nav.logs": "Runtime logs",
  "nav.logsShort": "Logs",
  "nav.settings": "Settings",
  "nav.settingsShort": "Settings",
  "section.router.title": "Router control",
  "section.agents.title": "Agent configuration",
  "section.apiKeys.title": "API key",
  "section.logs.title": "Runtime logs",
  "section.settings.title": "Settings",
  "placeholder.overline": "STABLE MODULE BOUNDARY",
  "placeholder.heading": "Module not connected",
  "placeholder.description":
    "This area preserves stable navigation without performing reads, writes, or process operations.",
  "placeholder.waiting": "Pending a later task",
  "placeholder.claude": "Claude Code",
  "placeholder.opencode": "OpenCode",
  "placeholder.codex": "Codex",
  "router.state.notStarted.title": "Router is stopped",
  "router.state.notStarted.signal": "Stopped",
  "router.state.notStarted.detail":
    "The local listener is unavailable. Agent requests can be forwarded after startup.",
  "router.state.starting.title": "Starting router",
  "router.state.starting.signal": "Starting",
  "router.state.starting.detail":
    "Validating components and waiting for the local listener and upstream probe.",
  "router.state.healthy.title": "Router is healthy",
  "router.state.healthy.signal": "Running",
  "router.state.healthy.detail":
    "The local process is available and the upstream mTLS health check passed.",
  "router.state.degraded.title": "Upstream unavailable",
  "router.state.degraded.signal": "Degraded",
  "router.state.degraded.detail":
    "The router is still running, but the upstream health check failed or expired.",
  "router.state.external.title": "External router is running",
  "router.state.external.signal": "Externally managed",
  "router.state.external.detail":
    "A compatible external router was identified. The desktop application will not stop it.",
  "router.state.occupied.title": "Port is occupied",
  "router.state.occupied.signal": "Port conflict",
  "router.state.occupied.detail":
    "An unknown process owns the local port. Force termination requires either complete identity verification or an explicitly warned Windows PID-only confirmation.",
  "router.state.failed.title": "Router failed to start",
  "router.state.failed.signal": "Action required",
  "router.state.failed.detail":
    "A startup problem was identified. Follow the guidance below and retry; technical details remain available.",
  "router.state.unavailable.title": "Router status unavailable",
  "router.state.unavailable.signal": "Status unavailable",
  "router.state.unavailable.detail":
    "The desktop could not read the router status. It will retry automatically.",
  "router.state.reinstall.title": "Desktop components are invalid",
  "router.state.reinstall.signal": "Reinstall required",
  "router.state.reinstall.detail":
    "A packaged router component is missing or does not match this desktop build. Reinstall the desktop application from a trusted package.",
  "router.state.stopping.title": "Stopping router",
  "router.state.stopping.signal": "Stopping",
  "router.state.stopping.detail":
    "Verifying process identity and performing a safe shutdown.",
  "router.state.legacy.title": "A historical desktop router is running",
  "router.state.legacy.signal": "Migration required",
  "router.state.legacy.detail":
    "This installation is correlated with a still-running older desktop router. Start will reclaim or stop and recreate it using complete process identity, never PID-only termination.",
  "router.health.unavailable": "Unavailable",
  "router.health.unknown": "Awaiting check",
  "router.health.checking": "Checking",
  "router.health.healthy": "Healthy",
  "router.health.degraded": "Upstream unavailable",
  "router.health.stale": "Result expired",
  "router.error.load":
    "Unable to read router status. Restart the desktop app or view logs.",
  "router.error.start":
    "Startup failed. Review the safely filtered logs and retry.",
  "router.error.stop":
    "Stop failed. No signal was sent to an unverified process.",
  "router.error.health":
    "Health check failed. Router process state was not affected.",
  "router.error.sidecarReinstall":
    "A required packaged component is missing or invalid. Reinstall the desktop application; no component will be downloaded automatically.",
  "router.failureGuide.overline": "PROBLEM IDENTIFIED",
  "router.failureGuide.nextStep": "Recommended action",
  "router.failureGuide.upstream.title":
    "Cannot connect to the upstream service",
  "router.failureGuide.upstream.detail":
    "The pre-start mTLS connection check failed, so the router did not open its local address and will not accept Agent requests.",
  "router.failureGuide.upstream.action":
    "Confirm that the network, VPN, or proxy is available and retry. If it persists, ask the service administrator to verify the upstream and client credentials.",
  "router.failureGuide.credentials.title": "Router credentials cannot be used",
  "router.failureGuide.credentials.detail":
    "The packaged client certificate, private key, or trust chain cannot establish an mTLS connection.",
  "router.failureGuide.credentials.action":
    "Install the latest trusted package from the service administrator and retry. Do not replace individual credential files manually.",
  "router.failureGuide.configuration.title":
    "Router startup configuration is invalid",
  "router.failureGuide.configuration.detail":
    "The local listener, upstream address, or connection probe configuration does not meet startup requirements.",
  "router.failureGuide.configuration.action":
    "Restart the desktop application. If it happens again, reinstall the latest trusted package.",
  "router.failureGuide.log-storage.title": "Cannot prepare runtime logs",
  "router.failureGuide.log-storage.detail":
    "The router cannot create or open its local log location, so startup stopped rather than running without diagnostics.",
  "router.failureGuide.log-storage.action":
    "Check disk space and write access to the application data directory, close programs that may lock the log, and retry.",
  "router.failureGuide.local-port.title": "Cannot open the local port",
  "router.failureGuide.local-port.detail":
    "The router could not listen on its local address, so Agent requests cannot reach it.",
  "router.failureGuide.local-port.action":
    "Confirm that 127.0.0.1:19099 is not used by another program and retry. Restart the desktop application if needed.",
  "router.failureGuide.process-launch.title":
    "The system could not start the router component",
  "router.failureGuide.process-launch.detail":
    "The desktop application validated the component, but the operating system did not create the router process.",
  "router.failureGuide.process-launch.action":
    "Restart the desktop application and retry. If it persists, check security software or reinstall the trusted package.",
  "router.failureGuide.process-identity.title":
    "Cannot safely identify the router process",
  "router.failureGuide.process-identity.detail":
    "A router process was created, but the desktop application could not verify its complete identity and did not take ownership.",
  "router.failureGuide.process-identity.action":
    "Restart the desktop application and retry. If it repeats, reinstall the latest trusted package.",
  "router.failureGuide.readiness.title": "Router startup timed out",
  "router.failureGuide.readiness.detail":
    "The router process did not provide verifiable local status within the startup deadline.",
  "router.failureGuide.readiness.action":
    "Check the technical details for a connection error, verify the network, and retry. Contact the service administrator if timeouts continue.",
  "router.failureGuide.component-identity.title":
    "Router component identity does not match",
  "router.failureGuide.component-identity.detail":
    "The running process reported a version or deployment identity that differs from this desktop build, so ownership was refused.",
  "router.failureGuide.component-identity.action":
    "Exit other CodeasierRouter instances and reinstall the desktop application from one trusted package.",
  "router.failureGuide.state-reconcile.title":
    "Cannot confirm the existing router state",
  "router.failureGuide.state-reconcile.detail":
    "The desktop application could not safely verify or clear an existing state record, so it did not start another router.",
  "router.failureGuide.state-reconcile.action":
    "Exit other desktop instances and stop separately managed routers before retrying. Do not manually delete state files that may still be in use.",
  "router.failureGuide.state-storage.title": "Cannot save router runtime state",
  "router.failureGuide.state-storage.detail":
    "The router process started, but ownership state could not be stored safely, so startup was cancelled.",
  "router.failureGuide.state-storage.action":
    "Check disk space and write access to the application data directory, then restart the desktop application and retry.",
  "router.failureGuide.shutdown.title": "Router did not shut down cleanly",
  "router.failureGuide.shutdown.detail":
    "The router encountered an error while stopping service and has been marked unavailable.",
  "router.failureGuide.shutdown.action":
    "Restart the desktop application and review runtime logs. If it repeats, provide the technical details to the service administrator.",
  "router.failureGuide.unexpected-exit.title":
    "The running router exited unexpectedly",
  "router.failureGuide.unexpected-exit.detail":
    "A previously available router process ended, and the local address no longer accepts Agent requests.",
  "router.failureGuide.unexpected-exit.action":
    "Review runtime logs and retry startup. If it exits again, provide the technical details to the service administrator.",
  "router.failureGuide.process-exit.title": "Router exited during startup",
  "router.failureGuide.process-exit.detail":
    "The component was executed but ended before local listening and identity verification completed.",
  "router.failureGuide.process-exit.action":
    "Resolve the specific cause shown in the technical details and retry. If no cause is available, restart the desktop application.",
  "router.failureGuide.sidecar-resolution.title":
    "The management component cannot be located",
  "router.failureGuide.sidecar-resolution.detail":
    "The desktop could not resolve or validate the packaged manager, so the control plane did not start.",
  "router.failureGuide.sidecar-resolution.action":
    "Reinstall the desktop application from a trusted package; do not replace sidecars separately.",
  "router.failureGuide.spawn.title": "The management component could not start",
  "router.failureGuide.spawn.detail":
    "The operating system did not create the manager process, so the desktop cannot manage the router.",
  "router.failureGuide.spawn.action":
    "Restart the desktop application. If this persists, check security software and reinstall from a trusted package.",
  "router.failureGuide.handshake.title": "Manager handshake failed",
  "router.failureGuide.handshake.detail":
    "The manager started, but its version, deployment, or protocol identity does not match this desktop build.",
  "router.failureGuide.handshake.action":
    "Quit other instances and reinstall the desktop application from the same trusted package.",
  "router.failureGuide.protocol-parse.title":
    "The manager protocol response could not be parsed",
  "router.failureGuide.protocol-parse.detail":
    "The manager returned output that does not match the protocol, so the desktop discarded it.",
  "router.failureGuide.protocol-parse.action":
    "Restart the desktop application. If it repeats, provide the technical details to the service administrator.",
  "router.failureGuide.watchdog-timeout.title": "The manager timed out",
  "router.failureGuide.watchdog-timeout.detail":
    "The manager did not return within the protocol deadline. Requests with uncertain delivery were not replayed.",
  "router.failureGuide.watchdog-timeout.action":
    "Restart the desktop application and retry. Do not resubmit an in-flight write or terminate operation.",
  "router.failureGuide.internal.title": "Router startup did not complete",
  "router.failureGuide.internal.detail":
    "The desktop application did not receive enough information to classify the failure safely, so the router remains stopped.",
  "router.failureGuide.internal.action":
    "Review runtime logs and retry. If the problem persists, provide the technical details to the service administrator.",
  "router.failureDiagnostics": "Router failure diagnostics",
  "router.failureTechnicalDetails": "Technical details",
  "router.failureTechnicalHint": "Raw error codes and safely filtered logs",
  "router.failureLastError": "Last error",
  "router.failureRecentLogs": "Recent safely filtered logs",
  "router.viewFullRuntimeLogs": "View runtime logs",
  "router.panelOverline": "PROCESS / UPSTREAM",
  "router.instrumentNote":
    "Process availability and upstream health are monitored independently",
  "router.processStatus": "Process status",
  "router.upstreamHealth": "Upstream health",
  "router.localAddress": "Local address",
  "router.actionsAria": "Router actions",
  "router.start": "Start router",
  "router.stop": "Stop router",
  "router.retryHealth": "Retry health check",
  "router.occupant.overline": "PORT RECOVERY",
  "router.occupant.heading": "Inspect the port occupant",
  "router.occupant.inspecting": "Safely inspecting the occupant...",
  "router.occupant.process": "Process",
  "router.occupant.pid": "PID",
  "router.occupant.executable": "Complete executable path",
  "router.occupant.forceAction": "Force terminate occupant",
  "router.occupant.retry": "Retry inspection",
  "router.occupant.error.not-owned":
    "This process belongs to another user. The app will not request elevation or terminate it.",
  "router.occupant.error.unverifiable":
    "The occupant identity cannot be verified completely, so it cannot be terminated safely.",
  "router.occupant.error.protected":
    "This process is protected by the desktop lifecycle and cannot be terminated through port recovery.",
  "router.occupant.error.changed":
    "The port occupant changed or the confirmation expired. Inspect it again before confirming.",
  "router.occupant.error.permissionDenied":
    "Termination permission was denied after inspection. Stop the process from its original privilege context and inspect again.",
  "router.occupant.error.terminationFailed":
    "The termination request did not end the process. No router start was attempted.",
  "router.occupant.error.releaseTimeout":
    "The process changed, but the app could not confirm that the port was released. Inspect the current occupant before continuing.",
  "router.occupant.error.temporary":
    "The occupant cannot be inspected right now. No termination request was sent.",
  "router.occupant.reason.serviceManaged":
    "A system service manages this process. Stop every listed service through its supervisor before retrying.",
  "router.occupant.reason.insufficientPrivilege":
    "Terminate access was denied by the current privilege level or OS protection such as Windows PPL; the app cannot reliably distinguish these cases. Exit the process normally from its owning context or use the system process manager. Do not run the desktop app as administrator or root.",
  "router.occupant.reason.differentUser":
    "This process belongs to another user. Ask that user or an administrator to stop it; the desktop app will not elevate.",
  "router.occupant.reason.protectedProcess":
    "This target is protected by a known application lifecycle and cannot be terminated through port recovery. Resolve its owning application or lifecycle manager instead.",
  "router.occupant.reason.identityUnavailable":
    "The process identity cannot be verified reliably, so destructive recovery is unavailable.",
  "router.occupant.supervisor.windows":
    "Open services.msc or run each shown sc.exe command in Administrator PowerShell. Shared hosts require stopping every listed Windows Service separately.",
  "router.occupant.supervisor.systemdUser":
    "Stop the identified user service from the same login session.",
  "router.occupant.supervisor.systemdSystem":
    "Stop the identified system service from an authorized terminal.",
  "router.occupant.supervisor.generic":
    "Stop the service in its system supervisor, then retry inspection. No supervisor identifier was inferred.",
  "router.occupant.copyCommand": "Copy command",
  "router.occupant.commandCopied": "Command copied",
  "router.occupant.commandCopyFailed": "Unable to copy the command",
  "router.occupant.observation.observing":
    "Confirming that the port remains released",
  "router.occupant.observation.released": "The port remains released",
  "router.occupant.observation.reoccupied":
    "A replacement process or supervisor reoccupied the port",
  "router.occupant.observation.termination-ineffective":
    "The termination request did not end the original process",
  "router.occupant.observation.replacement":
    "A replacement process now occupies the port",
  "router.occupant.observation.supervisorGuidance":
    "If a supervisor restarted the process, identify and stop the responsible service in Service Control Manager (SCM), systemd unit, or launchd job before retrying. The app does not guess supervisor identifiers.",
  "router.occupant.dialogOverline": "IRREVERSIBLE ACTION",
  "router.occupant.dialogTitle": "Confirm force termination",
  "router.occupant.warning":
    "Termination is immediate and will not attempt a graceful exit first. Unsaved data in this process may be lost.",
  "router.occupant.pidOnlyWarning":
    "Windows did not verify this process's identity, owner, start time, or executable. The manager will recheck that the same port is still owned by the same PID immediately before termination, but PID reuse and unreadable managed-router state leave residual risk. Termination is immediate, and unsaved data may be lost.",
  "router.occupant.cancel": "Cancel",
  "router.occupant.confirm": "Force terminate",
  "router.occupant.terminating": "Terminating...",
  "router.desktop": "Desktop",
  "router.manager": "Manager",
  "router.router": "Router",
  "router.next": "NEXT",
  "router.agentNotice":
    "When the router is available, go to Agents to generate a configuration preview. Files are never changed automatically.",
  "router.goToAgents": "Go to Agent configuration",
  "logs.error.load": "Unable to read recent logs.",
  "logs.error.open": "Unable to open the log location.",
  "logs.error.copy": "Unable to copy the diagnostic summary.",
  "logs.opened": "Log location opened.",
  "logs.copied": "Safely filtered diagnostic summary copied.",
  "logs.overline": "BOUNDED EVENT STREAM",
  "logs.heading": "Most recent {count} lines",
  "logs.refresh": "Refresh",
  "logs.openLocation": "Open log location",
  "logs.copyDiagnostics": "Copy diagnostic summary",
  "logs.loading": "Reading bounded range...",
  "logs.empty": "No router logs",
  "logs.boundary":
    "Only the bounded log tail is shown; the full file is not read.",
  "settings.error.load": "Some settings could not be loaded. Try again later.",
  "settings.error.autostart": "Unable to change current-user startup settings.",
  "settings.error.uninstall":
    "Unable to prepare for uninstall. Startup settings were not changed.",
  "settings.autostartChanged": "Startup setting updated.",
  "settings.on": "ON",
  "settings.off": "OFF",
  "settings.overline": "CURRENT-USER PREFERENCES",
  "settings.heading": "Desktop control panel",
  "settings.general": "General",
  "settings.autostart": "Launch at login",
  "settings.autostartDescription":
    "Launch the desktop application after login. Enabled by default on first launch and available to disable at any time.",
  "settings.language": "Interface language",
  "settings.languageDescription":
    "Only the language preference is saved in local browser storage.",
  "settings.chinese": "简体中文",
  "settings.english": "English",
  "settings.components": "Component versions",
  "settings.locations": "Storage locations",
  "settings.dataLocation": "Application data",
  "settings.logLocation": "Log directory",
  "settings.unavailable": "Unavailable",
  "update.checkAction": "Check for updates",
  "update.checking": "Checking...",
  "update.currentVersion": "Current version",
  "update.latestVersion": "Latest version",
  "update.available": "Update available",
  "update.current": "You have the latest version",
  "update.statusUnavailable": "Update status is not available yet",
  "update.publishedAt": "Published: {date}",
  "update.releaseNotes": "Release notes",
  "update.installAction": "Install and restart",
  "update.installing": "Downloading...",
  "update.installConfirm":
    "CodeasierRouter {version} will be downloaded and installed, then the application will restart. Continue?",
  "update.downloadProgress": "Update download progress",
  "update.progressKnown": "Downloaded {downloaded} / {total} bytes",
  "update.progressUnknown": "Downloaded {downloaded} bytes",
  "update.restarting": "Update installed. Restarting...",
  "update.error.check": "Unable to check for updates. Try again later.",
  "update.error.install":
    "Unable to download or install the update. The current version was not changed; try again.",
  "settings.prepareTitle": "Prepare for uninstall",
  "settings.prepareDescription":
    "Remove current-user autostart and exit the application. Delete the application only after it exits. Agent configurations, backups, logs, and diagnostic state are not changed.",
  "settings.prepareAction": "Prepare and exit",
  "settings.prepareConfirm":
    "This will remove current-user autostart and exit CodeasierRouter. Continue?",
  "apikey.overline": "ONE PERSON / ONE DEVICE / ONE KEY",
  "apikey.heading": "Access credential",
  "apikey.status.saved": "Configured",
  "apikey.status.absent": "Not configured",
  "apikey.status.loading": "Reading credential status",
  "apikey.status.note":
    "The desktop interface reads only this summary. It cannot read the saved key back.",
  "apikey.fingerprint": "Fingerprint",
  "apikey.savedAt": "Saved at",
  "apikey.label": "API key",
  "apikey.input.placeholder": "Paste a new API key",
  "apikey.show": "Show",
  "apikey.hide": "Hide",
  "apikey.save": "Save key",
  "apikey.replace": "Replace key",
  "apikey.delete": "Delete saved key",
  "apikey.saving": "Saving...",
  "apikey.deleting": "Deleting...",
  "apikey.explainer.usage.heading": "Where it is used",
  "apikey.explainer.usage.agentFiles":
    "Writing credential configuration for Claude Code, OpenCode, and Codex.",
  "apikey.explainer.usage.catalog":
    "Fetching the available model catalog through the local router.",
  "apikey.storage.label": "Storage location",
  "apikey.explainer.storage.note":
    "Unix file permissions are fixed at 0600. Windows uses the current-user data directory access controls.",
  "apikey.path.loading": "Reading storage location...",
  "apikey.error.length": "The key must be non-empty and no larger than 16 KiB.",
  "apikey.error.io":
    "The credential file could not be read or written. Try again.",
  "apikey.error.invalid": "The credential file is corrupt. Save the key again.",
  "apikey.error.lock":
    "Another credential operation is active. Try again shortly.",
  "apikey.error.load": "The credential status could not be loaded. Try again.",
  "agents.overview.refresh": "Detect again",
  "agents.overview.refreshing": "Detecting again",
  "agents.overview.stale": "Status may be out of date. Detect again.",
  "agents.configuration.invalid": "Invalid configuration",
  "agents.configuration.readonly": "Not writable",
  "agents.configuration.configured": "Configured",
  "agents.configuration.create": "Ready to create",
  "agents.configuration.ready": "Ready to configure",
  "agents.action.configure": "Configure {agent}",
  "agents.action.edit": "Edit {agent} configuration",
  "agents.action.create": "Generate {agent} configuration",
  "agents.issue.credential.notFound": "No API key has been saved.",
  "agents.issue.credential.invalid":
    "The saved API key is malformed. Save it again.",
  "agents.issue.credential.io":
    "Credential storage is unavailable. Try again later.",
  "agents.issue.credential.locked":
    "Credential storage is in use. Wait for the current operation to finish and try again.",
  "agents.issue.credential.unavailable":
    "Credential storage is unavailable. Try again later.",
  "agents.issue.auth": "The saved API key was rejected by the model service.",
  "agents.issue.discovery":
    "The model catalog is temporarily unavailable. Try again.",
  "agents.issue.manager":
    "The local manager is temporarily unavailable. Try again or restart the app.",
  "agents.issue.unknown": "Unable to start configuration ({code}).",
  "agents.issue.toApiKeys": "Go to API key",
  "agents.issue.replaceApiKey": "Replace API key",
  "agents.issue.retry": "Retry {agent}",
  "agents.operation.create": "Create",
  "agents.operation.replace": "Replace",
  "agents.operation.delete": "Delete",
  "agents.operation.backup": "Backup",
  "agents.operation.preserve": "Preserve unmanaged configuration",
  "agents.mode.merge": "Merge",
  "agents.mode.rebuild": "Rebuild",
  "agents.detection.invalid": "Invalid configuration",
  "agents.detection.readonly": "Not writable",
  "agents.detection.configured": "Configured",
  "agents.detection.ready": "Ready",
  "agents.detection.create": "Ready to create",
  "agents.guidance.invalid": "Fix the {format} syntax in {path}, then refresh.",
  "agents.guidance.readonly":
    "Restore current-user write access to the configuration directory.",
  "agents.guidance.configured":
    "Local managed fields are complete; discover models to verify current authorization before updating.",
  "agents.guidance.ready":
    "Configuration can be generated; previewing does not write files.",
  "agents.recovery.guidance.eligible":
    "This invalid configuration can be backed up and rebuilt from a clean managed configuration.",
  "agents.recovery.guidance.ineligible":
    "Automatic rebuild is unavailable. Resolve the reason shown here, then refresh detection.",
  "agents.recovery.toggle": "Back up and rebuild {agent}",
  "agents.recovery.warning":
    "Unrelated settings, comments, formatting, and valid companion-file metadata will not be preserved.",
  "agents.recovery.reason.syntaxInvalid":
    "The configuration syntax is invalid.",
  "agents.recovery.reason.unsupportedStructure":
    "The configuration structure cannot be rebuilt safely.",
  "agents.recovery.reason.unreadable":
    "The configuration file cannot be read by the current user.",
  "agents.recovery.reason.oversized":
    "The configuration file exceeds the supported size limit.",
  "agents.recovery.reason.nonRegular":
    "The configuration path is not a regular file.",
  "agents.recovery.reason.linked":
    "A linked file cannot be rebuilt automatically.",
  "agents.recovery.reason.notWritable":
    "The configuration file is not writable by the current user.",
  "agents.recovery.reason.parentUnavailable":
    "The configuration directory is unavailable or not writable.",
  "agents.recovery.reason.transactionPending":
    "A previous configuration transaction requires recovery first.",
  "agents.recovery.reason.writesDisabled":
    "Agent configuration writes are disabled in this environment.",
  "agents.recovery.reason.unknown":
    "Automatic rebuild is unavailable for this configuration.",
  "agents.migration": "JSONC -> JSON migration",
  "agents.sourceFile": "Source file {path}",
  "agents.migrationWarning":
    "Comments and original formatting will not be preserved; the JSONC file is backed up as sensitive data first.",
  "agents.fileOperations": "File operations",
  "agents.preserve": "Preserve: {items}",
  "agents.keyFile":
    "This file will save the supplied API key; its value is never shown in the preview.",
  "agents.sensitiveBackup": "SENSITIVE BACKUP",
  "agents.backupWarning":
    "Backups may contain old keys. They remain beside the original and must be protected as sensitive files.",
  "agents.error.detect":
    "Agent detection failed. Confirm the manager is available and retry.",
  "agents.error.selection": "Select at least one configurable Agent.",
  "agents.previewRefreshed":
    "Files changed and the preview was refreshed. Review it again before continuing.",
  "agents.error.invalid":
    "The target configuration is invalid. No files were changed. Fix it and refresh detection.",
  "agents.error.preview":
    "Unable to generate a preview. No files were changed. Refresh detection and retry.",
  "agents.error.preview.configInvalid":
    "The existing Agent configuration cannot be read or is malformed. Fix the file and refresh detection ({code}).",
  "agents.error.preview.notWritable":
    "The Agent configuration path is not writable. Check file and directory permissions, then retry ({code}).",
  "agents.error.preview.agentNotFound":
    "The selected Agent is no longer available. Refresh detection and select it again ({code}).",
  "agents.error.preview.modelState":
    "The Agent model-management state is damaged. Restart the application and retry ({code}).",
  "agents.error.preview.busy":
    "Another Agent configuration operation is in progress. Retry shortly ({code}).",
  "agents.error.preview.timeout":
    "Generating the preview timed out. Confirm no other configuration operation is running, then retry ({code}).",
  "agents.error.preview.manager":
    "The local manager could not generate the preview. Restart the application and retry ({code}).",
  "agents.error.preview.unknown":
    "Unable to generate a preview. No files were changed. Refresh detection and retry ({code}).",
  "agents.error.write":
    "Write failed. The transaction did not complete; review the result and retry with the saved credential.",
  "agents.error.previewStale":
    "Files changed after preview. Detection was refreshed; select the Agents and review a new preview.",
  "agents.error.backupFailed":
    "Backup failed before replacement. No target file was changed.",
  "agents.error.rolledBack":
    "Writing failed, and all changed targets were restored from rollback backups.",
  "agents.error.rollbackFailed":
    "Rollback could not be completed. Agent writes are disabled until recovery is resolved.",
  "agents.overline": "SELECT / DISCOVER / MODEL ORCHESTRATION / WRITE",
  "agents.heading": "Model configuration workbench",
  "agents.currentStage": "Current stage {stage}",
  "agents.stage.select": "Select",
  "agents.stage.credential": "Credential",
  "agents.stage.discover": "Discover",
  "agents.stage.configure": "Configure",
  "agents.stage.preview": "Preview",
  "agents.stage.write": "Write",
  "agents.stage.result": "Result",
  "agents.continue": "Continue to model discovery",
  "agents.credentialHeading": "Discover models with the saved key",
  "agents.credentialNote":
    "Model discovery and configuration writes load the key temporarily from the desktop credential store. The page cannot read its plaintext value back.",
  "agents.discover": "Discover models",
  "agents.discovering":
    "Discovering models through the trusted local router...",
  "agents.error.auth":
    "The model service rejected the saved key. Replace it in API key management.",
  "agents.error.discovery":
    "Model discovery failed. Check the router and upstream, then retry.",
  "agents.error.catalogStale":
    "The catalog expired or a selected model disappeared. Discover models again with the saved key.",
  "agents.error.flowExpired":
    "The model-discovery session expired. Discover models again with the saved key.",
  "agents.error.config": "Model configuration is invalid: {detail}",
  "agents.error.config.catalogModel":
    "A selected model is not in the current catalog. Refresh discovery and select it again.",
  "agents.error.config.baseModel":
    "A selected model has an invalid format. Select it again.",
  "agents.error.config.name": "Model display names cannot be empty.",
  "agents.error.config.contextConflict":
    "Claude's numeric context window cannot be combined with a role's 1M context mode.",
  "agents.error.config.outputLimit":
    "Claude's maximum output tokens must be less than its context window.",
  "agents.error.config.integerRelationship":
    "The model token limits are inconsistent.",
  "agents.error.config.contextWindow":
    "The context window must be a positive integer.",
  "agents.error.config.positiveInteger":
    "The token limit must be a positive integer.",
  "agents.error.config.extra":
    "Advanced configuration contains a field that cannot be managed.",
  "agents.error.config.fallback":
    "The model configuration does not meet the requirements. Refresh discovery and select the models again.",
  "agents.error.import":
    "This canonical JSON model configuration could not be imported.",
  "agents.error.export":
    "The canonical model configuration could not be exported.",
  "agents.imported": "Canonical model configuration imported and validated.",
  "agents.commonCatalog": "Common model catalog",
  "agents.modelCount": "{count} available models",
  "agents.catalogSearch": "Search models",
  "agents.catalogLabel": "Available model catalog",
  "agents.catalogEmptySearch": "No matching models",
  "agents.existingDrift":
    "Managed existing configuration has drifted: {agents}",
  "agents.unavailableModels":
    "Existing {agent} models are unavailable and require a new selection: {models}",
  "agents.initializationSource": "{agent} initialized from {source}",
  "agents.source.existing": "existing configuration",
  "agents.source.preset": "recommended preset",
  "agents.presetUnavailable":
    "The recommended {agent} preset is unavailable because these models are not in the current catalog: {models}",
  "agents.opencodeModels": "Selected OpenCode models",
  "agents.primaryModel": "Primary model",
  "agents.roleModel": "{role} model",
  "agents.activeModel": "Active model",
  "agents.defaultModel": "Default model",
  "agents.chooseModel": "Choose a model explicitly",
  "agents.inheritPrimary": "{role} inherits primary",
  "agents.enableFable": "Enable Fable",
  "agents.advancedExtra": "Advanced constrained extra",
  "agents.extraJson": "{agent} extra JSON object",
  "agents.formatJson": "Format JSON",
  "agents.extraValid": "Object is valid; the manager remains authoritative.",
  "agents.unset": "Unset (omitted)",
  "agents.reasoningEffort": "Reasoning effort",
  "agents.reasoningSummary": "Reasoning summary",
  "agents.verbosity": "Verbosity",
  "agents.displayName": "Display name",
  "agents.contextMode": "Context mode",
  "agents.contextStandard": "Standard",
  "agents.context1m": "1M",
  "agents.claudeContextWindow": "Claude context window",
  "agents.claudeMaxOutputTokens": "Claude max output tokens",
  "agents.contextLimit": "Context limit",
  "agents.outputLimit": "Output limit",
  "agents.contextWindow": "Context window",
  "agents.compactLimit": "Auto compact token limit",
  "agents.compactScope": "Auto compact scope",
  "agents.modalities.input": "Input modalities",
  "agents.modalities.output": "Output modalities",
  "agents.interleaved": "Interleaved reasoning field",
  "agents.optionsJson": "Model options JSON",
  "agents.variantsJson": "Variants JSON",
  "agents.modelExtraJson": "Model extra JSON",
  "agents.importConfig": "Import canonical config",
  "agents.exportConfig": "Export canonical config",
  "agents.fragments": "Redacted managed fragments",
  "agents.effects": "File, backup, and state effects",
  "agents.effect.agent": "Agent",
  "agents.effect.role": "Role",
  "agents.effect.path": "Target path",
  "agents.effect.backupPattern": "Planned backup path",
  "agents.effect.preserves": "Preserves:",
  "agents.effect.warning": "Warning:",
  "agents.approveDrift": "Approve replacing drifted managed namespaces",
  "agents.approveCodexAuth":
    "Approve switching Codex to file-backed API-key auth",
  "agents.backToConfigure": "Back to configuration",
  "agents.selectionNote":
    "Only selected Agents enter the structured preview; detection does not write files.",
  "agents.detecting": "Detecting...",
  "agents.refresh": "Refresh detection",
  "agents.loading": "Reading detection state...",
  "agents.noResult": "No detection result returned",
  "agents.mainConfig": "Main configuration",
  "agents.notLocated": "Not located",
  "agents.authFile": "Authentication file",
  "agents.format": "Format",
  "agents.notApplicable": "N/A",
  "agents.file": "File",
  "agents.exists": "Exists",
  "agents.pendingCreate": "To be created",
  "agents.permission": "Permission",
  "agents.writable": "Writable by current user",
  "agents.routerConfig": "Router configuration",
  "agents.notConfigured": "Not configured",
  "agents.selected": "Selected",
  "agents.select": "Select this Agent",
  "agents.selectAgent": "Select {agent}",
  "agents.selectedCount": "{count} Agents selected",
  "agents.generatePreview": "Generate write preview",
  "agents.fileCount": "{count} files",
  "agents.approvalBoundary": "APPROVAL BOUNDARY",
  "agents.reviewScope": "Review change scope",
  "agents.oneTimeCredential": "Saved credential",
  "agents.reviewNote":
    "The saved API key is loaded only for the approved write. No files have been changed yet.",
  "agents.reviewSensitiveBackup": "Sensitive backups",
  "agents.reviewBackup":
    "Existing-file backups may contain old keys. Protect them like the original configuration.",
  "agents.approve": "I reviewed and approve",
  "agents.cancelDetection": "Cancel and return to detection",
  "agents.keyNote":
    "The saved key is loaded only for this controlled write request; the page does not reveal or log it.",
  "agents.executing": "Executing transaction...",
  "agents.write": "Write selected Agents",
  "agents.rebuildConfirm.overline": "DESTRUCTIVE CONFIRMATION",
  "agents.rebuildConfirm.title": "Confirm backup and rebuild",
  "agents.rebuildConfirm.description":
    "Only these Agents will be approved for destructive rebuild:",
  "agents.rebuildConfirm.cancel": "Cancel",
  "agents.rebuildConfirm.confirm": "Back up and rebuild",
  "agents.cancelKey": "Cancel write",
  "agents.transactionComplete": "TRANSACTION COMPLETE",
  "agents.resultHeading": "Agent configuration result",
  "agents.resultNote":
    "These paths come from the manager transaction result; no credential value is displayed.",
  "agents.success": "Success",
  "agents.failure": "Failed",
  "agents.rolledBack": "This change was rolled back",
  "agents.errorCode": "Error code: {code}",
  "agents.changed": "Changed",
  "agents.backups": "Backup created",
  "agents.none": "None",
  "agents.rollbackBackup": "Rollback diagnostic backups",
  "agents.finish": "Finish and refresh detection",
  "agents.panel.back": "Back to Agent overview",
  "agents.panel.previewLoading": "Generating write preview...",
  "agents.panel.reloading": "Reloading Agent configuration...",
  "agents.panel.readonly":
    "This Agent configuration cannot be edited right now.",
  "agents.panel.blockedDirty":
    "External configuration state changed and this draft can no longer be edited. Export or discard it first.",
  "agents.panel.retry": "Retry",
  "agents.panel.discard": "Discard draft",
  "agents.panel.reloadFailed": "Reload after write failed ({code}).",
  "agents.panel.operationFailed": "Operation failed ({code}).",
  "agents.panel.writeComplete": "Configuration was written successfully.",
  "agents.panel.refreshFailed":
    "Refresh failed ({code}); the current draft is preserved.",
  "agents.panel.refreshConflict":
    "The on-disk configuration changed. Keep this draft or load the on-disk configuration.",
  "agents.panel.keepDraft": "Keep current draft",
  "agents.panel.importedSource": "Imported configuration",
  "agents.panel.continueEditing": "Dismiss result and continue editing",
  "agents.panel.status": "PANEL STATUS",
  "agents.panel.unsaved": "Unsaved changes",
  "agents.panel.ready": "Configuration synchronized",
  "agents.panel.previewSteps":
    "Generate a preview, review file effects and approvals, then write the configuration.",
  "agents.panel.configurationState": "Configuration state",
  "agents.leave.overline": "UNSAVED AGENT DRAFT",
  "agents.leave.title": "Discard changes and leave?",
  "agents.leave.description":
    "This Agent configuration has unsaved changes. They will be lost if you leave.",
  "agents.leave.busy":
    "An Agent configuration operation is in progress. Wait for it to finish before leaving.",
  "agents.leave.cancel": "Keep editing",
  "agents.leave.confirm": "Discard and leave",
  "agents.cleanup.action": "Clean up managed {agent} configuration",
  "agents.cleanup.overline": "PER-AGENT CLEANUP / BACKUP FIRST / KEY-FREE",
  "agents.cleanup.heading": "Clean up managed {agent} configuration",
  "agents.cleanup.loading": "Generating cleanup preview...",
  "agents.cleanup.keyFree":
    "This operation does not read the API key or contact the model catalog or router service.",
  "agents.cleanup.review": "Confirm cleanup boundary",
  "agents.cleanup.removedPaths": "Managed settings to remove",
  "agents.cleanup.fileEffects": "File, backup, and state effects",
  "agents.cleanup.authRemoved":
    "Authentication settings in the Agent files will be removed.",
  "agents.cleanup.globalKeyRetained":
    "The global API key saved by the desktop will not be removed.",
  "agents.cleanup.backupsRetained":
    "Historical and new backups remain in place; cleanup does not delete them. Some Agent backups may contain credentials or old keys.",
  "agents.cleanup.approveDrift":
    "Approve cleanup of drifted managed namespaces",
  "agents.cleanup.write": "Back up and clean up",
  "agents.cleanup.writing": "Backing up and cleaning up...",
  "agents.cleanup.stale":
    "Files changed after preview. Cleanup has not run; generate a new preview.",
  "agents.cleanup.ambiguous":
    "Cleanup delivery could not be confirmed ({code}). Its outcome is unknown; generate a new preview before taking another action.",
  "agents.cleanup.repreview": "Re-preview cleanup",
  "agents.cleanup.failed": "Cleanup failed ({code}).",
  "agents.cleanup.retryPreview": "Retry preview",
  "agents.cleanup.retryWrite": "Retry cleanup",
  "agents.cleanup.completeOverline": "CLEANUP TRANSACTION COMPLETE",
  "agents.cleanup.complete": "Cleanup complete",
  "agents.cleanup.completeNote":
    "Cleanup is complete; historical backups remain in place.",
  "agents.cleanup.finish": "Finish and return to Agent overview",
  "agents.cleanup.diagnostic.modelStateInvalid":
    "The desktop-saved Agent management state cannot be verified. Restart the app; if this persists, inspect the configuration manually.",
  "agents.cleanup.diagnostic.writesDisabled":
    "Agent configuration writes are unavailable. Recover the pending transaction or restart the app first.",
};
