import { useCallback, useEffect, useRef, useState } from "react";

import { useI18n, type Translator } from "./i18n";
import type {
  DesktopApi,
  OccupantInspection,
  OccupantSupervisor,
  PollSnapshot,
  ReleaseObservation,
  RouterHealth,
  RouterStatus,
} from "./ipc";
import { sanitizeSensitiveText, validOccupantInspection } from "./ipc";
import type { TranslationKey } from "./locales/zh-CN";

type Operation = "starting" | "stopping" | null;
type RouterMessage =
  | ""
  | "router.error.load"
  | "router.error.start"
  | "router.error.stop"
  | "router.error.health"
  | "router.error.sidecarReinstall";
type OccupantError =
  | "not-owned"
  | "unverifiable"
  | "protected"
  | "changed"
  | "permission-denied"
  | "termination-failed"
  | "release-timeout"
  | "temporary";
type ReoccupationOutcome = "termination-ineffective" | "replacement";
export const MAX_FAILURE_LOG_LINES = 20;
type HealthState = "unknown" | "checking" | "healthy" | "degraded" | "stale";
type ViewState =
  | "not-started"
  | "starting"
  | "healthy"
  | "degraded"
  | "external"
  | "occupied"
  | "failed"
  | "unavailable"
  | "reinstall"
  | "stopping";

interface StateCopy {
  title: string;
  signal: string;
  detail: string;
  tone: "idle" | "active" | "warning" | "danger";
  light: "off" | "green" | "yellow" | "red";
}

interface FailureDiagnostics {
  lastError: string;
  recentLogs: string[];
}

type FailureKind =
  | "upstream"
  | "credentials"
  | "configuration"
  | "log-storage"
  | "local-port"
  | "process-launch"
  | "process-identity"
  | "readiness"
  | "component-identity"
  | "state-reconcile"
  | "state-storage"
  | "shutdown"
  | "unexpected-exit"
  | "process-exit"
  | "internal";

interface FailureGuide {
  title: string;
  detail: string;
  action: string;
}

const failureReasonKinds: Record<string, FailureKind> = {
  arguments_invalid: "configuration",
  config_invalid: "configuration",
  backend_start_failed: "internal",
  log_open_failed: "log-storage",
  tls_material_invalid: "credentials",
  probe_setup_failed: "configuration",
  upstream_probe_failed: "upstream",
  listen_failed: "local-port",
  shutdown_failed: "shutdown",
  router_failure: "internal",
};

const failureStageKinds: Record<string, FailureKind> = {
  log_directory: "log-storage",
  log_open: "log-storage",
  process_launch: "process-launch",
  process_inspect: "process-identity",
  readiness: "readiness",
  identity_validate: "component-identity",
  state_reconcile: "state-reconcile",
  state_persist: "state-storage",
  process_exit: "process-exit",
};

function knownDiagnosticKind(
  lines: string[],
  pattern: RegExp,
  kinds: Record<string, FailureKind>,
): FailureKind | null {
  for (let index = lines.length - 1; index >= 0; index -= 1) {
    const value = pattern.exec(lines[index])?.[1];
    if (value && kinds[value]) return kinds[value];
  }
  return null;
}

function failureKind(diagnostics: FailureDiagnostics): FailureKind {
  const lines = [diagnostics.lastError, ...diagnostics.recentLogs].filter(
    Boolean,
  );
  const reasonKind = knownDiagnosticKind(
    lines,
    /(?:^|\s)reason=([a-z_]+)(?=\s|$)/,
    failureReasonKinds,
  );
  if (reasonKind) return reasonKind;

  const stageKind = knownDiagnosticKind(
    lines,
    /(?:^|\s)stage=([a-z_]+)(?=\s|$)/,
    failureStageKinds,
  );
  if (stageKind) return stageKind;
  if (lines.some((line) => line.includes("exited unexpectedly"))) {
    return "unexpected-exit";
  }
  return "internal";
}

function failureGuide(
  t: Translator,
  diagnostics: FailureDiagnostics,
): FailureGuide {
  const kind = failureKind(diagnostics);
  const key = (part: "title" | "detail" | "action") =>
    `router.failureGuide.${kind}.${part}` as TranslationKey;
  return {
    title: t(key("title")),
    detail: t(key("detail")),
    action: t(key("action")),
  };
}

function getStateCopy(t: Translator): Record<ViewState, StateCopy> {
  return {
    "not-started": {
      title: t("router.state.notStarted.title"),
      signal: t("router.state.notStarted.signal"),
      detail: t("router.state.notStarted.detail"),
      tone: "idle",
      light: "off",
    },
    starting: {
      title: t("router.state.starting.title"),
      signal: t("router.state.starting.signal"),
      detail: t("router.state.starting.detail"),
      tone: "active",
      light: "yellow",
    },
    healthy: {
      title: t("router.state.healthy.title"),
      signal: t("router.state.healthy.signal"),
      detail: t("router.state.healthy.detail"),
      tone: "active",
      light: "green",
    },
    degraded: {
      title: t("router.state.degraded.title"),
      signal: t("router.state.degraded.signal"),
      detail: t("router.state.degraded.detail"),
      tone: "warning",
      light: "yellow",
    },
    external: {
      title: t("router.state.external.title"),
      signal: t("router.state.external.signal"),
      detail: t("router.state.external.detail"),
      tone: "active",
      light: "green",
    },
    occupied: {
      title: t("router.state.occupied.title"),
      signal: t("router.state.occupied.signal"),
      detail: t("router.state.occupied.detail"),
      tone: "danger",
      light: "red",
    },
    failed: {
      title: t("router.state.failed.title"),
      signal: t("router.state.failed.signal"),
      detail: t("router.state.failed.detail"),
      tone: "danger",
      light: "red",
    },
    unavailable: {
      title: t("router.state.unavailable.title"),
      signal: t("router.state.unavailable.signal"),
      detail: t("router.state.unavailable.detail"),
      tone: "warning",
      light: "yellow",
    },
    reinstall: {
      title: t("router.state.reinstall.title"),
      signal: t("router.state.reinstall.signal"),
      detail: t("router.state.reinstall.detail"),
      tone: "danger",
      light: "red",
    },
    stopping: {
      title: t("router.state.stopping.title"),
      signal: t("router.state.stopping.signal"),
      detail: t("router.state.stopping.detail"),
      tone: "warning",
      light: "yellow",
    },
  };
}

function isAvailable(status: RouterStatus | null): boolean {
  return (
    status?.state === "desktop_owned" ||
    status?.state === "external_compatible" ||
    status?.state === "degraded"
  );
}

function sanitizedFailureDiagnostics(
  status: RouterStatus | null,
): FailureDiagnostics {
  if (status?.state !== "start_failed") {
    return { lastError: "", recentLogs: [] };
  }
  const recentLogs = sanitizeSensitiveText(
    (status.recent_logs ?? []).join("\n"),
  )
    .split("\n")
    .slice(-MAX_FAILURE_LOG_LINES);
  return {
    lastError: status.last_error
      ? sanitizeSensitiveText(status.last_error)
      : "",
    recentLogs:
      recentLogs.length === 1 && recentLogs[0] === "" ? [] : recentLogs,
  };
}

function healthState(health: RouterHealth | null, now: number): HealthState {
  if (!health) return "unknown";
  if (health.status === "unknown") return "unknown";
  const checkedAt = Date.parse(health.checked_at);
  if (!Number.isFinite(checkedAt) || now - checkedAt > 30_000) return "stale";
  return health.status === "ok" ? "healthy" : "degraded";
}

function viewState(
  status: RouterStatus | null,
  health: HealthState,
  operation: Operation,
  actionFailed: boolean,
  statusReadFailed: boolean,
  reinstallRequired: boolean,
): ViewState {
  if (operation) return operation;
  if (reinstallRequired) return "reinstall";
  if (
    actionFailed ||
    status?.state === "start_failed" ||
    status?.state === "stale"
  ) {
    return "failed";
  }
  if (!status && statusReadFailed) return "unavailable";
  switch (status?.state) {
    case "starting":
      return "starting";
    case "stopping":
      return "stopping";
    case "desktop_owned":
      return health === "healthy" ? "healthy" : "degraded";
    case "external_compatible":
      return health === "healthy" ? "external" : "degraded";
    case "degraded":
      return "degraded";
    case "unknown_occupant":
      return "occupied";
    default:
      return "not-started";
  }
}

function healthLabel(
  value: HealthState,
  available: boolean,
  t: Translator,
): string {
  if (!available) return t("router.health.unavailable");
  const labels: Record<HealthState, string> = {
    unknown: t("router.health.unknown"),
    checking: t("router.health.checking"),
    healthy: t("router.health.healthy"),
    degraded: t("router.health.degraded"),
    stale: t("router.health.stale"),
  };
  return labels[value];
}

function actionErrorKey(
  action: "load" | "start" | "stop" | "health",
): RouterMessage {
  const messages = {
    load: "router.error.load",
    start: "router.error.start",
    stop: "router.error.stop",
    health: "router.error.health",
  } as const;
  return messages[action];
}

function clearRecoveredStatusMessage(
  current: RouterMessage,
  status: RouterStatus,
): RouterMessage {
  if (current === "router.error.load") return "";
  if (current === "router.error.start" && isAvailable(status)) return "";
  if (current === "router.error.stop" && status.state === "absent") return "";
  return current;
}

function errorCode(error: unknown): string {
  if (typeof error !== "object" || error === null || !("code" in error)) {
    return "";
  }
  return typeof error.code === "string" ? error.code : "";
}

function sidecarError(code: string): boolean {
  return code === "SIDECAR_MISSING" || code === "SIDECAR_INVALID";
}

function occupantError(code: string): OccupantError {
  switch (code) {
    case "OCCUPANT_NOT_OWNED":
      return "not-owned";
    case "OCCUPANT_IDENTITY_UNAVAILABLE":
      return "unverifiable";
    case "OCCUPANT_PROTECTED":
      return "protected";
    case "OCCUPANT_CHANGED":
    case "OCCUPANT_NOT_FOUND":
    case "CONFIRMATION_EXPIRED":
      return "changed";
    case "OCCUPANT_PERMISSION_DENIED":
      return "permission-denied";
    case "OCCUPANT_TERMINATION_FAILED":
      return "termination-failed";
    case "PORT_RELEASE_TIMEOUT":
      return "release-timeout";
    default:
      return "temporary";
  }
}

const occupantErrorKeys = {
  "not-owned": "router.occupant.error.not-owned",
  unverifiable: "router.occupant.error.unverifiable",
  protected: "router.occupant.error.protected",
  changed: "router.occupant.error.changed",
  "permission-denied": "router.occupant.error.permissionDenied",
  "termination-failed": "router.occupant.error.terminationFailed",
  "release-timeout": "router.occupant.error.releaseTimeout",
  temporary: "router.occupant.error.temporary",
} as const;

const recoveryReasonKeys = {
  service_managed: "router.occupant.reason.serviceManaged",
  insufficient_privilege: "router.occupant.reason.insufficientPrivilege",
  different_user: "router.occupant.reason.differentUser",
  protected_process: "router.occupant.reason.protectedProcess",
  identity_unavailable: "router.occupant.reason.identityUnavailable",
} as const;

function supervisorCommand(
  supervisor: OccupantSupervisor,
  identifier: string,
): string {
  if (supervisor.kind === "windows_service") {
    const powerShellLiteral = `'${identifier.replaceAll("'", "''")}'`;
    return `sc.exe stop ${powerShellLiteral}`;
  }
  const posixLiteral = `'${identifier.replaceAll("'", `'"'"'`)}'`;
  if (supervisor.kind === "systemd_user") {
    return `systemctl --user stop -- ${posixLiteral}`;
  }
  return `sudo systemctl stop -- ${posixLiteral}`;
}

function supervisorGuidanceKey(supervisor: OccupantSupervisor) {
  if (supervisor.kind === "windows_service") {
    return "router.occupant.supervisor.windows" as const;
  }
  if (supervisor.kind === "systemd_user") {
    return "router.occupant.supervisor.systemdUser" as const;
  }
  return "router.occupant.supervisor.systemdSystem" as const;
}

export function RouterPage({
  api,
  onNavigateToAgents,
  onNavigateToLogs,
}: {
  api: DesktopApi;
  onNavigateToAgents: () => void;
  onNavigateToLogs: () => void;
}) {
  const { t } = useI18n();
  const [status, setStatus] = useState<RouterStatus | null>(null);
  const [health, setHealth] = useState<RouterHealth | null>(null);
  const [operation, setOperation] = useState<Operation>(null);
  const [actionFailed, setActionFailed] = useState(false);
  const [statusReadFailed, setStatusReadFailed] = useState(false);
  const [message, setMessage] = useState<RouterMessage>("");
  const [checkingHealth, setCheckingHealth] = useState(false);
  const [snapshotRevision, setSnapshotRevision] = useState(-1);
  const [now, setNow] = useState(0);
  const [reinstallRequired, setReinstallRequired] = useState(false);
  const [occupant, setOccupant] = useState<OccupantInspection | null>(null);
  const [occupantFailure, setOccupantFailure] = useState<OccupantError | null>(
    null,
  );
  const [inspectingOccupant, setInspectingOccupant] = useState(false);
  const [occupantDialogOpen, setOccupantDialogOpen] = useState(false);
  const [terminatingOccupant, setTerminatingOccupant] = useState(false);
  const [releaseObservation, setReleaseObservation] = useState<
    ReleaseObservation["state"] | null
  >(null);
  const [reoccupiedRevision, setReoccupiedRevision] = useState(-1);
  const [reoccupationOutcome, setReoccupationOutcome] =
    useState<ReoccupationOutcome | null>(null);
  const [copyResult, setCopyResult] = useState<"copied" | "failed" | null>(
    null,
  );
  const [postForceFocus, setPostForceFocus] = useState<{
    target: "start" | "retry";
    generation: number;
  } | null>(null);
  const latestRevision = useRef(-1);
  const occupantGeneration = useRef(0);
  const statusState = useRef<RouterStatus | null>(null);
  const observationState = useRef<ReleaseObservation["state"] | null>(null);
  const reoccupiedRevisionState = useRef(-1);
  const forcedOccupantPid = useRef<number | null>(null);
  const autoInspectionKey = useRef<string | null>(null);
  const copyRequest = useRef(0);
  const focusGeneration = useRef(0);
  const startRouterRef = useRef<HTMLButtonElement>(null);
  const occupantRetryRef = useRef<HTMLButtonElement>(null);
  const forceTriggerRef = useRef<HTMLButtonElement>(null);
  const cancelOccupantRef = useRef<HTMLButtonElement>(null);
  const confirmOccupantRef = useRef<HTMLButtonElement>(null);

  const applySnapshot = useCallback((snapshot: PollSnapshot) => {
    if (snapshot.revision <= latestRevision.current) return;
    latestRevision.current = snapshot.revision;
    setSnapshotRevision(snapshot.revision);
    setNow(Date.now());
    const nextObservation = snapshot.release_observation?.state ?? null;
    if (nextObservation !== observationState.current) {
      observationState.current = nextObservation;
      setReleaseObservation(nextObservation);
      if (nextObservation === "reoccupied") {
        reoccupiedRevisionState.current = snapshot.revision;
        setReoccupiedRevision(snapshot.revision);
      } else {
        reoccupiedRevisionState.current = -1;
        setReoccupationOutcome(null);
        if (nextObservation === "released") forcedOccupantPid.current = null;
      }
    }
    if (snapshot.status) {
      statusState.current = snapshot.status;
      setStatus(snapshot.status);
      if (!snapshot.status_error) {
        setActionFailed(false);
        setStatusReadFailed(false);
        setMessage((current) =>
          clearRecoveredStatusMessage(current, snapshot.status!),
        );
      }
    }
    if (snapshot.health) {
      setHealth(snapshot.health);
      if (!snapshot.health_error) {
        setMessage((current) =>
          current === "router.error.health" ? "" : current,
        );
      }
    }
    if (snapshot.status && !isAvailable(snapshot.status)) {
      setHealth(null);
    }
    const statusCode = snapshot.status_error?.code ?? "";
    if (sidecarError(statusCode)) {
      setReinstallRequired(true);
      setMessage("router.error.sidecarReinstall");
    } else if (statusCode) {
      setStatusReadFailed(true);
      setMessage(actionErrorKey("load"));
    }
    if (snapshot.health_error && !statusCode) {
      setMessage(actionErrorKey("health"));
    }
  }, []);

  const inspectOccupant = useCallback(
    async (reoccupation?: { originalPid: number; revision: number }) => {
      const generation = ++occupantGeneration.current;
      setInspectingOccupant(true);
      setOccupant(null);
      setOccupantFailure(null);
      copyRequest.current += 1;
      setCopyResult(null);
      try {
        const inspected = await api.inspectRouterOccupant();
        const currentReoccupation =
          !reoccupation ||
          (observationState.current === "reoccupied" &&
            reoccupiedRevisionState.current === reoccupation.revision);
        if (
          generation === occupantGeneration.current &&
          statusState.current?.state === "unknown_occupant" &&
          currentReoccupation
        ) {
          if (validOccupantInspection(inspected)) {
            setOccupant(inspected);
            if (reoccupation) {
              setReoccupationOutcome(
                inspected.pid === reoccupation.originalPid
                  ? "termination-ineffective"
                  : "replacement",
              );
              forcedOccupantPid.current = null;
            }
          } else {
            setOccupantFailure("unverifiable");
            if (reoccupation) forcedOccupantPid.current = null;
          }
        }
      } catch (error) {
        const currentReoccupation =
          !reoccupation ||
          (observationState.current === "reoccupied" &&
            reoccupiedRevisionState.current === reoccupation.revision);
        if (
          generation === occupantGeneration.current &&
          statusState.current?.state === "unknown_occupant" &&
          currentReoccupation
        ) {
          setOccupantFailure(occupantError(errorCode(error)));
          if (reoccupation) forcedOccupantPid.current = null;
        }
      } finally {
        if (generation === occupantGeneration.current) {
          setInspectingOccupant(false);
        }
      }
    },
    [api],
  );

  useEffect(() => {
    let current = true;
    let unlisten: (() => void) | undefined;

    void api
      .subscribePollSnapshots((snapshot) => {
        if (current) applySnapshot(snapshot);
      })
      .then((stopListening) => {
        if (current) {
          unlisten = stopListening;
          void api.getPollSnapshot().then(applySnapshot).catch(handleLoadError);
        } else {
          stopListening();
        }
      })
      .catch(handleLoadError);

    function handleLoadError(error: unknown) {
      if (current) {
        const code = errorCode(error);
        if (sidecarError(code)) {
          setReinstallRequired(true);
          setMessage("router.error.sidecarReinstall");
        } else {
          setStatusReadFailed(true);
          setMessage(actionErrorKey("load"));
        }
      }
    }

    return () => {
      current = false;
      unlisten?.();
      void api.cancelRouterReleaseObservation().catch(() => undefined);
    };
  }, [api, applySnapshot]);

  useEffect(() => {
    if (!health) return;
    const checkedAt = Date.parse(health.checked_at);
    if (!Number.isFinite(checkedAt)) return;
    const remaining = checkedAt + 30_001 - Date.now();
    if (remaining <= 0) return;
    const timer = window.setTimeout(() => setNow(Date.now()), remaining);
    return () => window.clearTimeout(timer);
  }, [health, snapshotRevision]);

  useEffect(() => {
    const inspectionKey =
      status?.state === "unknown_occupant"
        ? releaseObservation === "reoccupied"
          ? `reoccupied:${reoccupiedRevision}`
          : "occupied"
        : null;
    const timer = window.setTimeout(() => {
      if (inspectionKey && autoInspectionKey.current !== inspectionKey) {
        autoInspectionKey.current = inspectionKey;
        const originalPid = forcedOccupantPid.current;
        void inspectOccupant(
          releaseObservation === "reoccupied" && originalPid !== null
            ? { originalPid, revision: reoccupiedRevision }
            : undefined,
        );
        return;
      }
      if (inspectionKey) return;
      autoInspectionKey.current = null;
      occupantGeneration.current += 1;
      setOccupant(null);
      setOccupantFailure(null);
      copyRequest.current += 1;
      setCopyResult(null);
      setInspectingOccupant(false);
      setOccupantDialogOpen(false);
    }, 0);
    return () => window.clearTimeout(timer);
  }, [status?.state, releaseObservation, reoccupiedRevision, inspectOccupant]);

  useEffect(() => {
    if (!occupantDialogOpen) return;
    cancelOccupantRef.current?.focus();
    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape" && !terminatingOccupant) {
        closeOccupantDialog();
      }
    }
    window.addEventListener("keydown", handleEscape);
    return () => window.removeEventListener("keydown", handleEscape);
  }, [occupantDialogOpen, terminatingOccupant]);

  function closeOccupantDialog() {
    forceTriggerRef.current?.focus();
    setOccupantDialogOpen(false);
  }

  function requestPostForceFocus(target: "start" | "retry") {
    const generation = ++focusGeneration.current;
    setPostForceFocus({ target, generation });
  }

  const available = isAvailable(status);
  const observedHealth = checkingHealth ? "checking" : healthState(health, now);
  const currentState = viewState(
    status,
    observedHealth,
    operation,
    actionFailed,
    statusReadFailed,
    reinstallRequired,
  );
  const copy = getStateCopy(t)[currentState];
  const failureDiagnostics = sanitizedFailureDiagnostics(status);
  const failureGuidance =
    failureDiagnostics.lastError || failureDiagnostics.recentLogs.length > 0
      ? failureGuide(t, failureDiagnostics)
      : null;
  const canStart =
    !operation &&
    !reinstallRequired &&
    (currentState === "not-started" || currentState === "failed");
  const canStop =
    (!operation &&
      status?.state === "desktop_owned" &&
      status.owner === "desktop") ||
    (!operation && status?.state === "degraded" && status.owner === "desktop");
  const canRetryHealth = available && !operation && !checkingHealth;
  const pidOnlyOccupant = occupant?.verification_mode === "windows_pid_only";

  useEffect(() => {
    if (
      !postForceFocus ||
      postForceFocus.generation !== focusGeneration.current
    )
      return;
    const target =
      postForceFocus.target === "start"
        ? startRouterRef.current
        : occupantRetryRef.current;
    if (!target || target.disabled) return;
    target.focus();
    setPostForceFocus(null);
  }, [postForceFocus, currentState, occupantFailure]);

  async function refreshSnapshot() {
    const snapshot = await api.getPollSnapshot();
    applySnapshot(snapshot);
    return snapshot;
  }

  async function start() {
    setOperation("starting");
    setActionFailed(false);
    setMessage("");
    try {
      try {
        await api.cancelRouterReleaseObservation();
      } catch {
        // Router start also cancels scheduler observation on the native side.
      }
      observationState.current = null;
      setReleaseObservation(null);
      const next = await api.startRouter();
      setStatus(next);
      await refreshSnapshot();
      setOperation(null);
    } catch (error) {
      setOperation(null);
      let refreshed: PollSnapshot | null = null;
      try {
        refreshed = await refreshSnapshot();
      } catch {
        // Use the safe action error below.
      }
      if (
        errorCode(error) === "ROUTER_DEGRADED" &&
        isAvailable(refreshed?.status ?? null)
      ) {
        setActionFailed(false);
        return;
      }
      if (sidecarError(errorCode(error))) {
        setReinstallRequired(true);
        setMessage("router.error.sidecarReinstall");
      } else {
        setActionFailed(true);
        setMessage(actionErrorKey("start"));
      }
    }
  }

  async function stop() {
    setOperation("stopping");
    setMessage("");
    try {
      const next = await api.stopRouter();
      setStatus(next);
      setHealth(null);
      await refreshSnapshot();
      setOperation(null);
    } catch {
      setOperation(null);
      setMessage(actionErrorKey("stop"));
    }
  }

  async function retryHealth() {
    setCheckingHealth(true);
    setMessage("");
    try {
      const next = await api.retryRouterHealth();
      setHealth(next);
      await refreshSnapshot();
    } catch {
      setMessage(actionErrorKey("health"));
    } finally {
      setCheckingHealth(false);
    }
  }

  async function forceTerminateOccupant() {
    if (
      !occupant ||
      occupant.recovery.action !== "force_terminate" ||
      terminatingOccupant
    )
      return;
    const confirmationToken = occupant.confirmation_token;
    if (typeof confirmationToken !== "string") return;
    forcedOccupantPid.current = occupant.pid;
    setReoccupationOutcome(null);
    setTerminatingOccupant(true);
    setMessage("");
    try {
      await api.forceTerminateRouterOccupant(confirmationToken);
    } catch (error) {
      const failure = occupantError(errorCode(error));
      setOccupantDialogOpen(false);
      setOccupant(null);
      setOccupantFailure(failure);
      forcedOccupantPid.current = null;
      requestPostForceFocus("retry");
      setTerminatingOccupant(false);
      return;
    }
    setOccupantDialogOpen(false);
    setOccupant(null);
    setHealth(null);
    try {
      await refreshSnapshot();
    } catch {
      // Scheduler snapshots will continue reconciling the successful release.
    }
    requestPostForceFocus("start");
    setTerminatingOccupant(false);
  }

  async function copyCommand(command: string) {
    const request = ++copyRequest.current;
    try {
      await navigator.clipboard.writeText(command);
      if (request === copyRequest.current) setCopyResult("copied");
    } catch {
      if (request === copyRequest.current) setCopyResult("failed");
    }
  }

  return (
    <div className={`panel-grid panel-grid--${copy.tone}`}>
      <section className="primary-panel" aria-labelledby="router-state-heading">
        <div className="panel-heading">
          <div>
            <p className="overline">{t("router.panelOverline")}</p>
            <h2 id="router-state-heading">{copy.title}</h2>
          </div>
          <span className={`signal signal--${copy.tone}`}>{copy.signal}</span>
        </div>

        <div className="instrument">
          <div
            className="traffic-light"
            data-state={copy.light}
            aria-hidden="true"
          >
            <span className="traffic-light__red" />
            <span className="traffic-light__yellow" />
            <span className="traffic-light__green" />
          </div>
          <div>
            <p>{copy.detail}</p>
            <span>{t("router.instrumentNote")}</span>
          </div>
        </div>

        <dl className="readout-grid">
          <div>
            <dt>{t("router.processStatus")}</dt>
            <dd>{copy.signal}</dd>
          </div>
          <div>
            <dt>{t("router.upstreamHealth")}</dt>
            <dd className={`health-value health-value--${observedHealth}`}>
              {healthLabel(observedHealth, available, t)}
            </dd>
          </div>
          <div>
            <dt>{t("router.localAddress")}</dt>
            <dd title={status?.listen_addr ?? "127.0.0.1:19099"}>
              {status?.listen_addr ?? "127.0.0.1:19099"}
            </dd>
          </div>
        </dl>

        {message && !failureGuidance && (
          <p className="inline-alert" role="alert">
            {t(message)}
          </p>
        )}

        {releaseObservation && (
          <section
            className={`release-observation release-observation--${releaseObservation}`}
            role="status"
          >
            <strong>
              {t(
                releaseObservation === "reoccupied" && reoccupationOutcome
                  ? `router.occupant.observation.${reoccupationOutcome}`
                  : `router.occupant.observation.${releaseObservation}`,
              )}
            </strong>
            {releaseObservation === "reoccupied" && (
              <p>{t("router.occupant.observation.supervisorGuidance")}</p>
            )}
          </section>
        )}

        {failureGuidance && (
          <section className="failure-guidance" role="alert">
            <span className="failure-guidance__marker" aria-hidden="true">
              !
            </span>
            <div>
              <p className="overline">{t("router.failureGuide.overline")}</p>
              <h3>{failureGuidance.title}</h3>
              <p className="failure-guidance__detail">
                {failureGuidance.detail}
              </p>
              <div className="failure-guidance__action">
                <strong>{t("router.failureGuide.nextStep")}</strong>
                <p>{failureGuidance.action}</p>
              </div>
            </div>
          </section>
        )}

        {failureGuidance && (
          <details
            className="failure-diagnostics"
            aria-label={t("router.failureDiagnostics")}
          >
            <summary>
              <span>{t("router.failureTechnicalDetails")}</span>
              <small>{t("router.failureTechnicalHint")}</small>
            </summary>
            <div className="failure-diagnostics__body">
              {failureDiagnostics.lastError && (
                <div>
                  <strong>{t("router.failureLastError")}</strong>
                  <code>{failureDiagnostics.lastError}</code>
                </div>
              )}
              {failureDiagnostics.recentLogs.length > 0 && (
                <div>
                  <strong>{t("router.failureRecentLogs")}</strong>
                  <ol>
                    {failureDiagnostics.recentLogs.map((line, index) => (
                      <li key={`${index}-${line}`}>
                        <code>{line}</code>
                      </li>
                    ))}
                  </ol>
                </div>
              )}
              <button
                type="button"
                className="text-button failure-diagnostics__action"
                onClick={onNavigateToLogs}
              >
                {t("router.viewFullRuntimeLogs")}
              </button>
            </div>
          </details>
        )}

        {currentState === "occupied" && (
          <section
            className="occupant-panel"
            aria-labelledby="occupant-recovery-heading"
          >
            <div>
              <p className="overline">{t("router.occupant.overline")}</p>
              <h3 id="occupant-recovery-heading">
                {t("router.occupant.heading")}
              </h3>
            </div>
            {inspectingOccupant && (
              <p role="status">{t("router.occupant.inspecting")}</p>
            )}
            {occupant?.recovery.action === "force_terminate" &&
              !inspectingOccupant && (
                <div className="occupant-target">
                  <dl>
                    {!pidOnlyOccupant && (
                      <div>
                        <dt>{t("router.occupant.process")}</dt>
                        <dd>{occupant.process_name}</dd>
                      </div>
                    )}
                    <div>
                      <dt>{t("router.occupant.pid")}</dt>
                      <dd>{occupant.pid}</dd>
                    </div>
                  </dl>
                  {pidOnlyOccupant && (
                    <p className="danger-dialog__warning">
                      {t("router.occupant.pidOnlyWarning")}
                    </p>
                  )}
                  <button
                    ref={forceTriggerRef}
                    type="button"
                    className="control-button control-button--danger"
                    onClick={() => setOccupantDialogOpen(true)}
                  >
                    {t("router.occupant.forceAction")}
                  </button>
                </div>
              )}
            {occupant &&
              occupant.recovery.action !== "force_terminate" &&
              !inspectingOccupant && (
                <div className="occupant-guidance">
                  <dl className="occupant-guidance__target">
                    {!pidOnlyOccupant && occupant.process_name && (
                      <div>
                        <dt>{t("router.occupant.process")}</dt>
                        <dd>{occupant.process_name}</dd>
                      </div>
                    )}
                    <div>
                      <dt>{t("router.occupant.pid")}</dt>
                      <dd>{occupant.pid}</dd>
                    </div>
                  </dl>
                  <p className="occupant-guidance__reason">
                    {t(recoveryReasonKeys[occupant.recovery.reason])}
                  </p>
                  {occupant.recovery.reason === "service_managed" &&
                    !occupant.supervisor && (
                      <p>{t("router.occupant.supervisor.generic")}</p>
                    )}
                  {occupant.recovery.reason === "service_managed" &&
                    occupant.supervisor && (
                      <div className="occupant-commands">
                        <p>{t(supervisorGuidanceKey(occupant.supervisor))}</p>
                        <ul>
                          {occupant.supervisor.identifiers.map((identifier) => {
                            const command = supervisorCommand(
                              occupant.supervisor!,
                              identifier,
                            );
                            return (
                              <li key={identifier}>
                                <strong>{identifier}</strong>
                                <div className="occupant-command">
                                  <code>{command}</code>
                                  <button
                                    type="button"
                                    className="text-button"
                                    onClick={() => void copyCommand(command)}
                                  >
                                    {t("router.occupant.copyCommand")}
                                  </button>
                                </div>
                              </li>
                            );
                          })}
                        </ul>
                      </div>
                    )}
                  <button
                    type="button"
                    className="text-button occupant-guidance__retry"
                    onClick={() => void inspectOccupant()}
                  >
                    {t("router.occupant.retry")}
                  </button>
                </div>
              )}
            {copyResult && (
              <p
                className="occupant-copy-result"
                role={copyResult === "failed" ? "alert" : "status"}
              >
                {t(
                  copyResult === "copied"
                    ? "router.occupant.commandCopied"
                    : "router.occupant.commandCopyFailed",
                )}
              </p>
            )}
            {occupantFailure && !inspectingOccupant && (
              <div className="occupant-blocked" role="alert">
                <p>{t(occupantErrorKeys[occupantFailure])}</p>
                <button
                  ref={occupantRetryRef}
                  type="button"
                  className="text-button"
                  onClick={() => void inspectOccupant()}
                >
                  {t("router.occupant.retry")}
                </button>
              </div>
            )}
          </section>
        )}

        <div className="action-row" aria-label={t("router.actionsAria")}>
          <button
            ref={startRouterRef}
            type="button"
            className="control-button"
            onClick={start}
            disabled={!canStart}
          >
            {t("router.start")}
          </button>
          <button
            type="button"
            className="control-button control-button--stop"
            onClick={stop}
            disabled={!canStop}
          >
            {t("router.stop")}
          </button>
          <button
            type="button"
            className="text-button"
            onClick={retryHealth}
            disabled={!canRetryHealth}
          >
            {t("router.retryHealth")}
          </button>
        </div>

        <div className="router-next">
          <div>
            <p className="overline">{t("router.next")}</p>
            <p>{t("router.agentNotice")}</p>
          </div>
          <button type="button" onClick={onNavigateToAgents}>
            {t("router.goToAgents")}
          </button>
        </div>
      </section>
      {occupantDialogOpen &&
        occupant?.recovery.action === "force_terminate" && (
          <div
            className="dialog-backdrop"
            onMouseDown={(event) => {
              if (
                event.target === event.currentTarget &&
                !terminatingOccupant
              ) {
                closeOccupantDialog();
              }
            }}
          >
            <section
              className="danger-dialog"
              role="dialog"
              aria-modal="true"
              aria-labelledby="occupant-dialog-title"
              aria-describedby="occupant-dialog-warning"
              onKeyDown={(event) => {
                if (event.key !== "Tab") return;
                if (terminatingOccupant) {
                  event.preventDefault();
                  cancelOccupantRef.current?.focus();
                  return;
                }
                if (
                  event.shiftKey &&
                  document.activeElement === cancelOccupantRef.current
                ) {
                  event.preventDefault();
                  confirmOccupantRef.current?.focus();
                } else if (
                  !event.shiftKey &&
                  document.activeElement === confirmOccupantRef.current
                ) {
                  event.preventDefault();
                  cancelOccupantRef.current?.focus();
                }
              }}
            >
              <p className="overline">{t("router.occupant.dialogOverline")}</p>
              <h2 id="occupant-dialog-title">
                {t("router.occupant.dialogTitle")}
              </h2>
              <dl className="danger-dialog__details">
                {!pidOnlyOccupant && (
                  <div>
                    <dt>{t("router.occupant.process")}</dt>
                    <dd>{occupant.process_name}</dd>
                  </div>
                )}
                <div>
                  <dt>{t("router.occupant.pid")}</dt>
                  <dd>{occupant.pid}</dd>
                </div>
                {!pidOnlyOccupant && (
                  <div className="danger-dialog__path">
                    <dt>{t("router.occupant.executable")}</dt>
                    <dd>{occupant.executable}</dd>
                  </div>
                )}
              </dl>
              <p
                id="occupant-dialog-warning"
                className="danger-dialog__warning"
              >
                {t(
                  pidOnlyOccupant
                    ? "router.occupant.pidOnlyWarning"
                    : "router.occupant.warning",
                )}
              </p>
              <div className="danger-dialog__actions">
                <button
                  ref={cancelOccupantRef}
                  type="button"
                  className="text-button"
                  aria-disabled={terminatingOccupant || undefined}
                  onClick={() => {
                    if (!terminatingOccupant) closeOccupantDialog();
                  }}
                >
                  {t("router.occupant.cancel")}
                </button>
                <button
                  ref={confirmOccupantRef}
                  type="button"
                  className="control-button control-button--danger"
                  disabled={terminatingOccupant}
                  onClick={() => void forceTerminateOccupant()}
                >
                  {terminatingOccupant
                    ? t("router.occupant.terminating")
                    : t("router.occupant.confirm")}
                </button>
              </div>
            </section>
          </div>
        )}
    </div>
  );
}
