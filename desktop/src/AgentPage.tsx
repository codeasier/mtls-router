import { useCallback, useEffect, useRef, useState } from "react";

import { AgentOverview, type OverviewIssue } from "./AgentOverview";
import { AgentWorkflow } from "./AgentWorkflow";
import { completeAgentDetection, type AgentTarget } from "./agentPresentation";
import { useI18n } from "./i18n";
import type { AgentDetection, AgentModelsResult, DesktopApi } from "./ipc";

interface WorkflowSession {
  target: AgentTarget;
  discovery: AgentModelsResult;
}

function errorCode(error: unknown) {
  return typeof error === "object" &&
    error !== null &&
    "code" in error &&
    typeof (error as { code?: unknown }).code === "string"
    ? (error as { code: string }).code
    : "";
}

function classifyStartError(
  error: unknown,
  target: AgentTarget,
): OverviewIssue {
  const code = errorCode(error);
  if (
    [
      "CREDENTIAL_NOT_FOUND",
      "CREDENTIAL_INVALID",
      "CREDENTIAL_IO_ERROR",
      "CREDENTIAL_LOCK_TIMEOUT",
    ].includes(code)
  ) {
    return { kind: "credential", code };
  }
  if (code === "MODEL_AUTH_FAILED") return { kind: "auth", code };
  return { kind: "retry", code: code || "UNKNOWN", target };
}

function requireCompleteDetection(value: AgentDetection) {
  const complete = completeAgentDetection(value);
  if (!complete) throw { code: "AGENT_DETECT_FAILED" };
  return complete;
}

export function AgentPage({
  api,
  onNavigateToApiKeys,
}: {
  api: DesktopApi;
  onNavigateToApiKeys: () => void;
}) {
  const { t } = useI18n();
  const [detection, setDetection] = useState<AgentDetection | null>(null);
  const [session, setSession] = useState<WorkflowSession | null>(null);
  const [issue, setIssue] = useState<OverviewIssue | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [stale, setStale] = useState(false);
  const flowRef = useRef("");
  const flowApiRef = useRef<DesktopApi | null>(null);
  const requestRef = useRef(0);
  const detectionGenerationRef = useRef(0);
  const startingRef = useRef(false);

  const destroyFlow = useCallback(async () => {
    const flow = flowRef.current;
    const owner = flowApiRef.current;
    flowRef.current = "";
    flowApiRef.current = null;
    if (flow && owner) {
      await owner.destroyAgentModelFlow(flow).catch(() => undefined);
    }
  }, []);

  function consumeFlow() {
    flowRef.current = "";
    flowApiRef.current = null;
  }

  const refreshDetection = useCallback(async () => {
    const generation = ++detectionGenerationRef.current;
    try {
      const value = requireCompleteDetection(await api.detectAgents());
      if (generation === detectionGenerationRef.current) {
        setDetection(value);
        setStale(false);
      }
      return value;
    } catch (error) {
      if (generation !== detectionGenerationRef.current) {
        throw { code: "REQUEST_SUPERSEDED" };
      }
      throw error;
    }
  }, [api]);

  useEffect(() => {
    let active = true;
    requestRef.current += 1;
    startingRef.current = false;
    void Promise.resolve()
      .then(() => {
        if (!active) return Promise.reject({ code: "REQUEST_CANCELLED" });
        setLoading(true);
        setDetection(null);
        setSession(null);
        setIssue(null);
        setStale(false);
        return refreshDetection();
      })
      .catch((error) => {
        if (active && errorCode(error) !== "REQUEST_SUPERSEDED") {
          setIssue({ kind: "detect", code: "AGENT_DETECT_FAILED" });
        }
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
      requestRef.current += 1;
      detectionGenerationRef.current += 1;
      startingRef.current = false;
      void destroyFlow();
    };
  }, [destroyFlow, refreshDetection]);

  async function refreshOverview() {
    if (refreshing) return;
    setRefreshing(true);
    setIssue(null);
    try {
      await refreshDetection();
    } catch (error) {
      if (errorCode(error) === "REQUEST_SUPERSEDED") return;
      setStale(Boolean(detection));
      setIssue({
        kind: "detect",
        code: errorCode(error) || "AGENT_DETECT_FAILED",
      });
    } finally {
      setRefreshing(false);
    }
  }

  async function refreshWorkflowDetection() {
    try {
      return await refreshDetection();
    } catch (error) {
      if (errorCode(error) !== "REQUEST_SUPERSEDED") {
        setStale(Boolean(detection));
      }
      throw error;
    }
  }

  async function startTarget(target: AgentTarget) {
    if (startingRef.current) return;
    startingRef.current = true;
    const request = ++requestRef.current;
    setIssue(null);
    setLoading(true);
    try {
      const value = await api.discoverModels([target.agent]);
      if (!value.flow_id.trim()) throw { code: "MODEL_RESPONSE_INVALID" };
      if (request !== requestRef.current) {
        await api.destroyAgentModelFlow(value.flow_id).catch(() => undefined);
        return;
      }
      flowRef.current = value.flow_id;
      flowApiRef.current = api;
      setSession({ target, discovery: value });
    } catch (error) {
      if (request === requestRef.current) {
        setIssue(classifyStartError(error, target));
      }
    } finally {
      if (request === requestRef.current) {
        startingRef.current = false;
        setLoading(false);
      }
    }
  }

  function returnToOverview(nextIssue?: OverviewIssue) {
    void destroyFlow();
    setSession(null);
    setIssue(nextIssue ?? null);
  }

  function navigateToApiKeys() {
    void destroyFlow();
    setSession(null);
    onNavigateToApiKeys();
  }

  if (session) {
    return (
      <AgentWorkflow
        api={api}
        target={session.target}
        discovery={session.discovery}
        onBack={() => returnToOverview()}
        onFlowConsumed={consumeFlow}
        onReturnToOverview={returnToOverview}
        refreshDetection={refreshWorkflowDetection}
      />
    );
  }

  return (
    <section className="agents-workbench" aria-labelledby="agents-heading">
      <header className="agents-workbench__header">
        <div>
          <p className="overline">{t("agents.overline")}</p>
          <h2 id="agents-heading">{t("agents.heading")}</h2>
        </div>
      </header>
      {loading ? (
        <div className="processing-stage" role="status">
          <span className="instrument__dial">GET</span>
          <h3>{t("agents.discovering")}</h3>
        </div>
      ) : detection ? (
        <AgentOverview
          detection={detection}
          refreshing={refreshing}
          stale={stale}
          issue={issue}
          onRefresh={() => void refreshOverview()}
          onConfigure={(target) => void startTarget(target)}
          onRetry={(target) => void startTarget(target)}
          onNavigateToApiKeys={navigateToApiKeys}
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
