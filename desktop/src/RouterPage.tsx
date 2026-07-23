import { useCallback, useEffect, useRef, useState } from "react";

import { useI18n, type Translator } from "./i18n";
import type {
  DesktopApi,
  OccupantInspection,
  PollSnapshot,
  RouterHealth,
  RouterStatus,
} from "./ipc";
import { sanitizeSensitiveText } from "./ipc";

type Operation = "starting" | "stopping" | null;
type RouterMessage =
  | ""
  | "router.error.load"
  | "router.error.start"
  | "router.error.stop"
  | "router.error.health"
  | "router.error.sidecarReinstall"
  | "router.occupant.released";
type OccupantError =
  "not-owned" | "unverifiable" | "protected" | "changed" | "temporary";
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
  code: string;
  title: string;
  signal: string;
  detail: string;
  tone: "idle" | "active" | "warning" | "danger";
}

function getStateCopy(t: Translator): Record<ViewState, StateCopy> {
  return {
    "not-started": {
      code: "00",
      title: t("router.state.notStarted.title"),
      signal: t("router.state.notStarted.signal"),
      detail: t("router.state.notStarted.detail"),
      tone: "idle",
    },
    starting: {
      code: "01",
      title: t("router.state.starting.title"),
      signal: t("router.state.starting.signal"),
      detail: t("router.state.starting.detail"),
      tone: "active",
    },
    healthy: {
      code: "10",
      title: t("router.state.healthy.title"),
      signal: t("router.state.healthy.signal"),
      detail: t("router.state.healthy.detail"),
      tone: "active",
    },
    degraded: {
      code: "11",
      title: t("router.state.degraded.title"),
      signal: t("router.state.degraded.signal"),
      detail: t("router.state.degraded.detail"),
      tone: "warning",
    },
    external: {
      code: "EX",
      title: t("router.state.external.title"),
      signal: t("router.state.external.signal"),
      detail: t("router.state.external.detail"),
      tone: "active",
    },
    occupied: {
      code: "PC",
      title: t("router.state.occupied.title"),
      signal: t("router.state.occupied.signal"),
      detail: t("router.state.occupied.detail"),
      tone: "danger",
    },
    failed: {
      code: "ER",
      title: t("router.state.failed.title"),
      signal: t("router.state.failed.signal"),
      detail: t("router.state.failed.detail"),
      tone: "danger",
    },
    unavailable: {
      code: "NA",
      title: t("router.state.unavailable.title"),
      signal: t("router.state.unavailable.signal"),
      detail: t("router.state.unavailable.detail"),
      tone: "warning",
    },
    reinstall: {
      code: "SC",
      title: t("router.state.reinstall.title"),
      signal: t("router.state.reinstall.signal"),
      detail: t("router.state.reinstall.detail"),
      tone: "danger",
    },
    stopping: {
      code: "09",
      title: t("router.state.stopping.title"),
      signal: t("router.state.stopping.signal"),
      detail: t("router.state.stopping.detail"),
      tone: "warning",
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

function sanitizedFailureDiagnostics(status: RouterStatus | null): {
  lastError: string;
  recentLogs: string[];
} {
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
    default:
      return "temporary";
  }
}

function validOccupantInspection(value: unknown): value is OccupantInspection {
  if (!value || typeof value !== "object") return false;
  const inspection = value as Record<string, unknown>;
  const keys = Object.keys(inspection);
  const hasExactKeys = (allowed: readonly string[]) =>
    keys.length === allowed.length &&
    keys.every((key) => allowed.includes(key));
  const rfc3339 =
    /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:[0-5]\d(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/;
  const validBase =
    Number.isInteger(inspection.pid) &&
    (inspection.pid as number) > 0 &&
    (inspection.pid as number) <= 0xffffffff &&
    inspection.listen_addr === "127.0.0.1:19099" &&
    typeof inspection.confirmation_token === "string" &&
    inspection.confirmation_token.trim() !== "" &&
    typeof inspection.expires_at === "string" &&
    rfc3339.test(inspection.expires_at) &&
    Number.isFinite(Date.parse(inspection.expires_at));
  if (!validBase) return false;

  if (inspection.verification_mode === "verified_identity") {
    return (
      hasExactKeys([
        "pid",
        "verification_mode",
        "process_name",
        "executable",
        "listen_addr",
        "confirmation_token",
        "expires_at",
      ]) &&
      typeof inspection.process_name === "string" &&
      inspection.process_name.trim() !== "" &&
      typeof inspection.executable === "string" &&
      inspection.executable.trim() !== ""
    );
  }
  if (inspection.verification_mode === "windows_pid_only") {
    return hasExactKeys([
      "pid",
      "verification_mode",
      "listen_addr",
      "confirmation_token",
      "expires_at",
    ]);
  }
  return false;
}

const occupantErrorKeys = {
  "not-owned": "router.occupant.error.not-owned",
  unverifiable: "router.occupant.error.unverifiable",
  protected: "router.occupant.error.protected",
  changed: "router.occupant.error.changed",
  temporary: "router.occupant.error.temporary",
} as const;

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
  const latestRevision = useRef(-1);
  const occupantGeneration = useRef(0);
  const statusState = useRef<RouterStatus | null>(null);
  const cancelOccupantRef = useRef<HTMLButtonElement>(null);

  const applySnapshot = useCallback((snapshot: PollSnapshot) => {
    if (snapshot.revision <= latestRevision.current) return;
    latestRevision.current = snapshot.revision;
    setSnapshotRevision(snapshot.revision);
    setNow(Date.now());
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

  const inspectOccupant = useCallback(async () => {
    const generation = ++occupantGeneration.current;
    setInspectingOccupant(true);
    setOccupant(null);
    setOccupantFailure(null);
    try {
      const inspected = await api.inspectRouterOccupant();
      if (
        generation === occupantGeneration.current &&
        statusState.current?.state === "unknown_occupant"
      ) {
        if (validOccupantInspection(inspected)) {
          setOccupant(inspected);
        } else {
          setOccupantFailure("unverifiable");
        }
      }
    } catch (error) {
      if (
        generation === occupantGeneration.current &&
        statusState.current?.state === "unknown_occupant"
      ) {
        setOccupantFailure(occupantError(errorCode(error)));
      }
    } finally {
      if (generation === occupantGeneration.current) {
        setInspectingOccupant(false);
      }
    }
  }, [api]);

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
    const timer = window.setTimeout(() => {
      if (status?.state === "unknown_occupant") {
        void inspectOccupant();
        return;
      }
      occupantGeneration.current += 1;
      setOccupant(null);
      setOccupantFailure(null);
      setInspectingOccupant(false);
      setOccupantDialogOpen(false);
    }, 0);
    return () => window.clearTimeout(timer);
  }, [status?.state, inspectOccupant]);

  useEffect(() => {
    if (!occupantDialogOpen) return;
    cancelOccupantRef.current?.focus();
    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape" && !terminatingOccupant) {
        setOccupantDialogOpen(false);
      }
    }
    window.addEventListener("keydown", handleEscape);
    return () => window.removeEventListener("keydown", handleEscape);
  }, [occupantDialogOpen, terminatingOccupant]);

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
      const next = await api.startRouter();
      setStatus(next);
      await refreshSnapshot();
      setOperation(null);
    } catch (error) {
      setOperation(null);
      if (errorCode(error) === "ROUTER_DEGRADED") {
        try {
          const refreshed = await refreshSnapshot();
          if (isAvailable(refreshed.status ?? null)) {
            setActionFailed(false);
            return;
          }
        } catch {
          // Use the safe action error below.
        }
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
    if (!occupant || terminatingOccupant) return;
    setTerminatingOccupant(true);
    setMessage("");
    try {
      const next = await api.forceTerminateRouterOccupant(
        occupant.confirmation_token,
      );
      statusState.current = next;
      setStatus(next);
      setOccupantDialogOpen(false);
      setOccupant(null);
      setHealth(null);
      await refreshSnapshot();
      setMessage("router.occupant.released");
    } catch (error) {
      const failure = occupantError(errorCode(error));
      if (failure === "changed") {
        setOccupantDialogOpen(false);
        setOccupant(null);
      }
      setOccupantFailure(failure);
    } finally {
      setTerminatingOccupant(false);
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
          <div className="instrument__dial" aria-hidden="true">
            <span>{copy.code}</span>
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

        {message && (
          <p className="inline-alert" role="alert">
            {t(message)}
          </p>
        )}

        {(failureDiagnostics.lastError ||
          failureDiagnostics.recentLogs.length > 0) && (
          <section
            className="failure-diagnostics"
            aria-label={t("router.failureDiagnostics")}
          >
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
          </section>
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
            {occupant && !inspectingOccupant && (
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
                  type="button"
                  className="control-button control-button--danger"
                  onClick={() => setOccupantDialogOpen(true)}
                >
                  {t("router.occupant.forceAction")}
                </button>
              </div>
            )}
            {occupantFailure && !inspectingOccupant && (
              <div className="occupant-blocked" role="alert">
                <p>{t(occupantErrorKeys[occupantFailure])}</p>
                <button
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
      {occupantDialogOpen && occupant && (
        <div
          className="dialog-backdrop"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget && !terminatingOccupant) {
              setOccupantDialogOpen(false);
            }
          }}
        >
          <section
            className="danger-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="occupant-dialog-title"
            aria-describedby="occupant-dialog-warning"
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
            <p id="occupant-dialog-warning" className="danger-dialog__warning">
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
                disabled={terminatingOccupant}
                onClick={() => setOccupantDialogOpen(false)}
              >
                {t("router.occupant.cancel")}
              </button>
              <button
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
