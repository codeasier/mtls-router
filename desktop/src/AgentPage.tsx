import { useCallback, useEffect, useRef, useState } from "react";

import { AgentOverview, type OverviewIssue } from "./AgentOverview";
import { AgentCleanupPanel } from "./AgentCleanupPanel";
import { AgentPanel, type AgentPanelGuardState } from "./AgentPanel";
import { completeAgentDetection, type AgentTarget } from "./agentPresentation";
import { useI18n } from "./i18n";
import type { AgentDetection, AgentId, DesktopApi } from "./ipc";

export type LeaveDecision = "allow" | "confirm" | "block";
export type LeaveGuard = () => LeaveDecision;

export interface AgentPageProps {
  api: DesktopApi;
  onNavigateToApiKeys(): void;
  onRequestLeave?(action: () => void): void;
  onDirtyChange?(dirty: boolean): void;
  registerLeaveGuard?(guard: LeaveGuard | null): void;
}

const requestLeaveDirectly = (action: () => void) => action();
const ignoreDirtyChange = () => undefined;
const ignoreLeaveGuard = () => undefined;

function requireCompleteDetection(value: AgentDetection) {
  const complete = completeAgentDetection(value);
  if (!complete) throw { code: "AGENT_DETECT_FAILED" };
  return complete;
}

export function AgentPage({
  api,
  onNavigateToApiKeys,
  onRequestLeave = requestLeaveDirectly,
  onDirtyChange = ignoreDirtyChange,
  registerLeaveGuard = ignoreLeaveGuard,
}: AgentPageProps) {
  const { t } = useI18n();
  const [detection, setDetection] = useState<AgentDetection | null>(null);
  const [target, setTarget] = useState<AgentTarget | null>(null);
  const [cleanupTarget, setCleanupTarget] = useState<AgentId | null>(null);
  const [issue, setIssue] = useState<OverviewIssue | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [stale, setStale] = useState(false);
  const [panelSession, setPanelSession] = useState(0);
  const guardStateRef = useRef<AgentPanelGuardState>({
    dirty: false,
    busy: false,
  });
  const cleanupBusyRef = useRef(false);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const restoreFocusRef = useRef<{
    agent: AgentId;
    preferred: "configure" | "cleanup";
  } | null>(null);
  const detectionGenerationRef = useRef(0);

  const refreshDetection = useCallback(async () => {
    const generation = ++detectionGenerationRef.current;
    const value = requireCompleteDetection(await api.detectAgents());
    if (generation !== detectionGenerationRef.current)
      throw { code: "REQUEST_SUPERSEDED" };
    setDetection(value);
    setStale(false);
    return value;
  }, [api]);

  useEffect(() => {
    let active = true;
    void Promise.resolve()
      .then(() => {
        if (!active) throw { code: "REQUEST_CANCELLED" };
        setLoading(true);
        setIssue(null);
        return refreshDetection();
      })
      .catch(() => {
        if (active) setIssue({ kind: "detect", code: "AGENT_DETECT_FAILED" });
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
      detectionGenerationRef.current += 1;
    };
  }, [refreshDetection]);

  useEffect(() => {
    if (!target && !cleanupTarget) return;
    registerLeaveGuard(() => {
      if (cleanupTarget) return cleanupBusyRef.current ? "block" : "allow";
      if (guardStateRef.current.busy) return "block";
      return guardStateRef.current.dirty ? "confirm" : "allow";
    });
    return () => {
      registerLeaveGuard(null);
      guardStateRef.current = { dirty: false, busy: false };
      cleanupBusyRef.current = false;
      onDirtyChange(false);
    };
  }, [cleanupTarget, onDirtyChange, registerLeaveGuard, target]);

  useEffect(() => {
    const restore = restoreFocusRef.current;
    if (target || cleanupTarget || loading || !detection || !restore) return;
    restoreFocusRef.current = null;
    const ids =
      restore.preferred === "cleanup"
        ? [`agent-${restore.agent}-cleanup`, `agent-${restore.agent}-action`]
        : [`agent-${restore.agent}-action`];
    const action = ids
      .map((id) => document.getElementById(id))
      .find(
        (element): element is HTMLButtonElement =>
          element instanceof HTMLButtonElement && !element.disabled,
      );
    if (action) action.focus();
    else headingRef.current?.focus();
  }, [cleanupTarget, detection, loading, target]);

  async function refreshOverview() {
    if (refreshing) return;
    setRefreshing(true);
    setIssue(null);
    try {
      await refreshDetection();
    } catch {
      setStale(Boolean(detection));
    } finally {
      setRefreshing(false);
    }
  }

  function leavePanel() {
    if (!target) return;
    const previous = target.agent;
    onRequestLeave(() => {
      restoreFocusRef.current = { agent: previous, preferred: "configure" };
      setTarget(null);
    });
  }

  function leaveCleanup() {
    if (!cleanupTarget) return;
    const previous = cleanupTarget;
    onRequestLeave(() => {
      restoreFocusRef.current = { agent: previous, preferred: "cleanup" };
      setCleanupTarget(null);
    });
  }

  async function completeCleanup() {
    if (!cleanupTarget) return;
    const previous = cleanupTarget;
    restoreFocusRef.current = { agent: previous, preferred: "cleanup" };
    try {
      await refreshDetection();
    } catch {
      setStale(Boolean(detection));
    } finally {
      setCleanupTarget(null);
    }
  }

  if (target) {
    return (
      <AgentPanel
        key={`${target.agent}:${panelSession}`}
        api={api}
        target={target.agent}
        onBack={leavePanel}
        onGuardStateChange={(state) => {
          guardStateRef.current = state;
          // Keep native quit routed through the live panel; its guard still skips
          // the dialog for clean drafts and blocks active operations.
          onDirtyChange(true);
        }}
        onNavigateToApiKeys={onNavigateToApiKeys}
        onRetrySession={() => setPanelSession((value) => value + 1)}
        onReloaded={setDetection}
      />
    );
  }

  if (cleanupTarget) {
    return (
      <AgentCleanupPanel
        api={api}
        agent={cleanupTarget}
        onBack={leaveCleanup}
        onBusyChange={(busy) => {
          cleanupBusyRef.current = busy;
        }}
        onComplete={() => void completeCleanup()}
      />
    );
  }

  return (
    <section className="agents-workbench" aria-labelledby="agents-heading">
      <header className="agents-workbench__header">
        <div>
          <p className="overline">{t("agents.overline")}</p>
          <h2 ref={headingRef} id="agents-heading" tabIndex={-1}>
            {t("agents.heading")}
          </h2>
        </div>
      </header>
      {loading ? (
        <div className="processing-stage" role="status">
          <h3>{t("agents.detecting")}</h3>
        </div>
      ) : detection ? (
        <AgentOverview
          detection={detection}
          refreshing={refreshing}
          stale={stale}
          issue={issue}
          onRefresh={() => void refreshOverview()}
          onConfigure={setTarget}
          onCleanup={setCleanupTarget}
          onRetry={setTarget}
          onNavigateToApiKeys={onNavigateToApiKeys}
        />
      ) : (
        <div className="agent-alert" role="alert">
          <span>{t("agents.error.detect")}</span>{" "}
          <button
            type="button"
            className="text-button"
            disabled={refreshing}
            onClick={() => void refreshOverview()}
          >
            {t("agents.overview.refresh")}
          </button>
        </div>
      )}
    </section>
  );
}
