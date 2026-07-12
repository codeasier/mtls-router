import { useCallback, useEffect, useRef, useState } from "react";

import { useI18n, type Translator } from "./i18n";
import type {
  ComponentVersions,
  DesktopApi,
  PollSnapshot,
  RouterHealth,
  RouterStatus,
} from "./ipc";
import { sanitizeSensitiveText } from "./ipc";

type Operation = "starting" | "stopping" | null;
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

function statusIdentity(status: RouterStatus | null): string {
  return [
    isAvailable(status) ? "available" : "unavailable",
    status?.owner ?? "none",
    status?.pid ?? "none",
  ].join("|");
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
  failed: boolean,
  reinstallRequired: boolean,
): ViewState {
  if (operation) return operation;
  if (reinstallRequired) return "reinstall";
  if (failed || status?.state === "start_failed" || status?.state === "stale") {
    return "failed";
  }
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

function ownerLabel(status: RouterStatus | null, t: Translator): string {
  if (status?.state === "external_compatible" || status?.owner === "cli") {
    return t("router.owner.external");
  }
  if (status?.owner === "desktop") return t("router.owner.desktop");
  return t("router.owner.none");
}

function safeActionError(
  action: "load" | "start" | "stop" | "health",
  t: Translator,
): string {
  const messages = {
    load: t("router.error.load"),
    start: t("router.error.start"),
    stop: t("router.error.stop"),
    health: t("router.error.health"),
  };
  return messages[action];
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

export function RouterPage({
  api,
  onNavigateToAgents,
}: {
  api: DesktopApi;
  onNavigateToAgents: () => void;
}) {
  const { t } = useI18n();
  const [status, setStatus] = useState<RouterStatus | null>(null);
  const [health, setHealth] = useState<RouterHealth | null>(null);
  const [versions, setVersions] = useState<ComponentVersions | null>(null);
  const [operation, setOperation] = useState<Operation>(null);
  const [failed, setFailed] = useState(false);
  const [message, setMessage] = useState("");
  const [checkingHealth, setCheckingHealth] = useState(false);
  const [snapshotRevision, setSnapshotRevision] = useState(-1);
  const [now, setNow] = useState(0);
  const [reinstallRequired, setReinstallRequired] = useState(false);
  const [healthFailed, setHealthFailed] = useState(false);
  const latestRevision = useRef(-1);
  const desiredVersionIdentity = useRef<string | null>(null);
  const loadedVersionIdentity = useRef<string | null>(null);
  const versionRefresh = useRef<Promise<void> | null>(null);

  const refreshComponentVersions = useCallback(
    (nextStatus: RouterStatus | null): Promise<void> => {
      desiredVersionIdentity.current = statusIdentity(nextStatus);
      if (versionRefresh.current) return versionRefresh.current;
      if (loadedVersionIdentity.current === desiredVersionIdentity.current) {
        return Promise.resolve();
      }

      const refresh = async () => {
        while (
          desiredVersionIdentity.current !== loadedVersionIdentity.current
        ) {
          const identity = desiredVersionIdentity.current;
          try {
            const value = await api.getComponentVersions();
            if (desiredVersionIdentity.current === identity) {
              setVersions(value);
            }
          } catch {
            // Component versions are informational; retry on the next identity change.
          }
          loadedVersionIdentity.current = identity;
        }
      };
      const pending = refresh().finally(() => {
        if (versionRefresh.current === pending) versionRefresh.current = null;
      });
      versionRefresh.current = pending;
      return pending;
    },
    [api],
  );

  const applySnapshot = useCallback(
    (snapshot: PollSnapshot) => {
      if (snapshot.revision <= latestRevision.current) return;
      latestRevision.current = snapshot.revision;
      setSnapshotRevision(snapshot.revision);
      setNow(Date.now());
      if (snapshot.status) {
        setStatus(snapshot.status);
        setFailed(false);
        void refreshComponentVersions(snapshot.status);
      } else if (desiredVersionIdentity.current === null) {
        void refreshComponentVersions(null);
      }
      if (snapshot.health) {
        setHealth(snapshot.health);
        setHealthFailed(false);
      }
      if (snapshot.status && !isAvailable(snapshot.status)) {
        setHealth(null);
        setHealthFailed(false);
      }
      const statusCode = snapshot.status_error?.code ?? "";
      if (sidecarError(statusCode)) {
        setReinstallRequired(true);
        setMessage(t("router.error.sidecarReinstall"));
      } else if (statusCode) {
        setFailed(true);
        setMessage(safeActionError("load", t));
      }
      if (snapshot.health_error) {
        setHealthFailed(true);
        setMessage(safeActionError("health", t));
      }
    },
    [refreshComponentVersions, t],
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
          setMessage(t("router.error.sidecarReinstall"));
        } else {
          setFailed(true);
          setMessage(safeActionError("load", t));
        }
      }
    }

    return () => {
      current = false;
      unlisten?.();
    };
  }, [api, applySnapshot, t]);

  useEffect(() => {
    if (!health) return;
    const checkedAt = Date.parse(health.checked_at);
    if (!Number.isFinite(checkedAt)) return;
    const remaining = checkedAt + 30_001 - Date.now();
    if (remaining <= 0) return;
    const timer = window.setTimeout(() => setNow(Date.now()), remaining);
    return () => window.clearTimeout(timer);
  }, [health, snapshotRevision]);

  const available = isAvailable(status);
  const observedHealth = checkingHealth
    ? "checking"
    : healthFailed
      ? "degraded"
      : healthState(health, now);
  const currentState = viewState(
    status,
    observedHealth,
    operation,
    failed,
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

  async function refreshSnapshot() {
    const snapshot = await api.getPollSnapshot();
    applySnapshot(snapshot);
    return snapshot;
  }

  async function start() {
    setOperation("starting");
    setFailed(false);
    setMessage("");
    try {
      const next = await api.startRouter();
      setStatus(next);
      await refreshComponentVersions(next);
      await refreshSnapshot();
      setOperation(null);
    } catch (error) {
      setOperation(null);
      if (errorCode(error) === "ROUTER_DEGRADED") {
        try {
          const refreshed = await refreshSnapshot();
          if (isAvailable(refreshed.status ?? null)) {
            setFailed(false);
            return;
          }
        } catch {
          // Use the safe action error below.
        }
      }
      if (sidecarError(errorCode(error))) {
        setReinstallRequired(true);
        setMessage(t("router.error.sidecarReinstall"));
      } else {
        setFailed(true);
        setMessage(safeActionError("start", t));
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
      await refreshComponentVersions(next);
      await refreshSnapshot();
      setOperation(null);
    } catch {
      setOperation(null);
      setMessage(safeActionError("stop", t));
    }
  }

  async function retryHealth() {
    setCheckingHealth(true);
    setMessage("");
    try {
      const next = await api.retryRouterHealth();
      setHealth(next);
      setHealthFailed(false);
      await refreshSnapshot();
    } catch {
      setHealthFailed(true);
      setMessage(safeActionError("health", t));
    } finally {
      setCheckingHealth(false);
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
            <dt>{t("router.owner")}</dt>
            <dd>{ownerLabel(status, t)}</dd>
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
            {message}
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
      </section>

      <aside className="status-rail" aria-label={t("router.componentsAria")}>
        <p className="overline">{t("router.componentVersions")}</p>
        <ol className="version-list">
          <li>
            <span className="status-index">A</span>
            <div>
              <strong>{t("router.desktop")}</strong>
              <small>{versions?.desktop ?? t("router.loading")}</small>
            </div>
          </li>
          <li>
            <span className="status-index">B</span>
            <div>
              <strong>{t("router.manager")}</strong>
              <small>{versions?.manager ?? t("router.loading")}</small>
            </div>
          </li>
          <li>
            <span className="status-index">C</span>
            <div>
              <strong>{t("router.router")}</strong>
              <small>{versions?.router || t("router.notRunning")}</small>
            </div>
          </li>
        </ol>
        {versions?.management_protocol && (
          <div className="protocol-readout">
            <span>{t("router.protocol")}</span>
            <strong>{versions.management_protocol}</strong>
          </div>
        )}
        <div className="notice">
          <span>{t("router.next")}</span>
          <p>{t("router.agentNotice")}</p>
          <button type="button" onClick={onNavigateToAgents}>
            {t("router.goToAgents")}
          </button>
        </div>
      </aside>
    </div>
  );
}
