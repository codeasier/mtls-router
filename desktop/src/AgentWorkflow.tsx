import { useRef, useState } from "react";

import {
  AgentConfigFields,
  validateAgentConfig,
  validateSingleTargetConfig,
  type AgentConfigDraftState,
  type AgentConfigFieldsHandle,
} from "./AgentConfigFields";
import type { OverviewIssue } from "./AgentOverview";
import { AgentPreviewPane, validateRebuildPreview } from "./AgentPreviewPane";
import { agentNames, type AgentTarget } from "./agentPresentation";
import { useI18n, type Translator } from "./i18n";
import {
  initializeAgentConfig,
  sanitizeSensitiveText,
  type AgentDetection,
  type AgentId,
  type AgentModes,
  type AgentModelsResult,
  type AgentPreview,
  type AgentWriteResult,
  type DesktopApi,
  type InitializationSource,
  type ModelConfig,
} from "./ipc";

interface AgentWorkflowProps {
  api: DesktopApi;
  target: AgentTarget;
  discovery: AgentModelsResult;
  onBack(): void;
  onFlowConsumed(): void;
  onReturnToOverview(issue?: OverviewIssue): void;
  refreshDetection(): Promise<AgentDetection>;
}

type WorkflowStage = "configure" | "preview" | "write" | "result";

function safe(value: string | undefined) {
  return sanitizeSensitiveText(value ?? "");
}

function errorCode(error: unknown) {
  return typeof error === "object" &&
    error !== null &&
    "code" in error &&
    typeof (error as { code?: unknown }).code === "string"
    ? (error as { code: string }).code
    : "";
}

function retryIssue(code: string, target: AgentTarget): OverviewIssue {
  return { kind: "retry", code: code || "UNKNOWN", target };
}

function writeIssue(code: string, target: AgentTarget): OverviewIssue {
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
  return retryIssue(code, target);
}

function targetIsReusable(detection: AgentDetection, target: AgentTarget) {
  const state = detection.agents.find((agent) => agent.agent === target.agent);
  return target.mode === "rebuild"
    ? Boolean(state?.invalid && state.recovery.eligible)
    : Boolean(state?.writable && !state.invalid);
}

function modelConfigErrorMessage(error: unknown, t: Translator) {
  const details =
    typeof error === "object" && error !== null && "details" in error
      ? (error as { details?: unknown }).details
      : undefined;
  const path =
    typeof details === "object" &&
    details !== null &&
    "path" in details &&
    typeof (details as { path?: unknown }).path === "string"
      ? (details as { path: string }).path
      : "";
  const rule =
    typeof details === "object" &&
    details !== null &&
    "rule" in details &&
    typeof (details as { rule?: unknown }).rule === "string"
      ? (details as { rule: string }).rule
      : "";

  if (rule === "catalog_model" || rule === "catalog")
    return t("agents.error.config.catalogModel");
  if (rule === "base_model" || rule === "model_id")
    return t("agents.error.config.baseModel");
  if (rule === "non_empty_name" || rule === "name")
    return t("agents.error.config.name");
  if (rule === "context_conflict")
    return t("agents.error.config.contextConflict");
  if (rule === "integer_relationship") {
    return path === "/claude/max_output_tokens"
      ? t("agents.error.config.outputLimit")
      : t("agents.error.config.integerRelationship");
  }
  if (rule === "positive_integer")
    return path.endsWith("/context_window")
      ? t("agents.error.config.contextWindow")
      : t("agents.error.config.positiveInteger");
  if (rule === "allowlist" || rule === "protected_path")
    return t("agents.error.config.extra");
  return t("agents.error.config.fallback");
}

function previewErrorMessage(code: string, t: Translator) {
  const key =
    code === "CONFIG_INVALID"
      ? "agents.error.preview.configInvalid"
      : code === "CONFIG_NOT_WRITABLE"
        ? "agents.error.preview.notWritable"
        : code === "AGENT_NOT_FOUND"
          ? "agents.error.preview.agentNotFound"
          : code === "MODEL_STATE_INVALID"
            ? "agents.error.preview.modelState"
            : code === "AGENT_OPERATION_BUSY"
              ? "agents.error.preview.busy"
              : code === "OPERATION_TIMEOUT"
                ? "agents.error.preview.timeout"
                : [
                      "MANAGER_FAILED",
                      "SIDECAR_MISSING",
                      "SIDECAR_INVALID",
                      "INVALID_RESPONSE",
                    ].includes(code)
                  ? "agents.error.preview.manager"
                  : "agents.error.preview.unknown";
  return t(key, { code: code || "UNKNOWN" });
}

function validateWriteResult(
  result: unknown,
  target: AgentId,
): result is AgentWriteResult {
  if (!result || typeof result !== "object") return false;
  const statuses = (result as { agents?: unknown }).agents;
  if (!Array.isArray(statuses) || statuses.length !== 1) return false;
  const status = statuses[0];
  return (
    Boolean(status) &&
    typeof status === "object" &&
    (status as { agent?: unknown }).agent === target &&
    typeof (status as { success?: unknown }).success === "boolean"
  );
}

export function AgentWorkflow({
  api,
  target,
  discovery,
  onBack,
  onFlowConsumed,
  onReturnToOverview,
  refreshDetection,
}: AgentWorkflowProps) {
  const { t } = useI18n();
  const agents: AgentId[] = [target.agent];
  const modes: AgentModes = { [target.agent]: target.mode };
  const initialized = initializeAgentConfig(
    agents,
    discovery.existing.model_config,
    discovery.preset.model_config,
  );
  const [stage, setStage] = useState<WorkflowStage>("configure");
  const [config, setConfig] = useState<ModelConfig>(initialized.config);
  const [sources, setSources] = useState<Record<AgentId, InitializationSource>>(
    initialized.sources,
  );
  const [preview, setPreview] = useState<AgentPreview | null>(null);
  const [result, setResult] = useState<AgentWriteResult | null>(null);
  const [draftState, setDraftState] = useState<AgentConfigDraftState>(() => ({
    error: validateAgentConfig(initialized.config, target.agent),
    hasLocalDraft: false,
  }));
  const [fieldsKey, setFieldsKey] = useState(0);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const writeInFlightRef = useRef(false);
  const fieldsRef = useRef<AgentConfigFieldsHandle>(null);

  async function loadPreview() {
    const snapshot = fieldsRef.current?.getSnapshot();
    if (!snapshot || snapshot.error) {
      setMessage(snapshot?.error ?? draftState.error);
      return;
    }
    setBusy(true);
    setMessage("");
    try {
      const value = await api.previewAgents(
        agents,
        discovery.flow_id,
        discovery.catalog_token,
        snapshot.config,
        modes,
      );
      if (!validateSingleTargetConfig(value.model_config, target.agent)) {
        onReturnToOverview(retryIssue("MODEL_RESPONSE_INVALID", target));
        return;
      }
      if (!value.revision_token.trim()) {
        onReturnToOverview(retryIssue("MODEL_RESPONSE_INVALID", target));
        return;
      }
      setPreview(value);
      setConfig(value.model_config);
      setStage("preview");
    } catch (error) {
      const code = errorCode(error);
      if (code === "MODEL_CATALOG_STALE" || code === "MODEL_FLOW_EXPIRED") {
        onReturnToOverview(retryIssue(code, target));
      } else if (code === "MODEL_CONFIG_INVALID") {
        setMessage(
          t("agents.error.config", {
            detail: modelConfigErrorMessage(error, t),
          }),
        );
      } else {
        setMessage(previewErrorMessage(code, t));
      }
    } finally {
      setBusy(false);
    }
  }

  async function recoverStalePreview() {
    setPreview(null);
    try {
      const detection = await refreshDetection();
      if (targetIsReusable(detection, target)) {
        setStage("configure");
        setMessage(t("agents.error.previewStale"));
      } else {
        onReturnToOverview(retryIssue("PREVIEW_STALE", target));
      }
    } catch {
      onReturnToOverview(retryIssue("PREVIEW_STALE", target));
    }
  }

  async function write(
    managedOverwrite: boolean,
    codexAuthChange: boolean,
    approveRebuild: AgentId[],
  ) {
    if (
      !preview ||
      writeInFlightRef.current ||
      !validateRebuildPreview(preview, target)
    )
      return;
    writeInFlightRef.current = true;
    setBusy(true);
    setStage("write");
    setMessage("");
    try {
      const value = await api.writeAgents(
        agents,
        discovery.flow_id,
        discovery.catalog_token,
        config,
        preview.revision_token,
        managedOverwrite,
        codexAuthChange,
        approveRebuild,
      );
      if (!validateWriteResult(value, target.agent)) {
        onReturnToOverview(retryIssue("INVALID_RESPONSE", target));
        return;
      }
      if (value.agents[0].success) onFlowConsumed();
      setResult(value);
      setStage("result");
    } catch (error) {
      const code = errorCode(error);
      if (code === "PREVIEW_STALE") {
        await recoverStalePreview();
      } else if (code === "ROLLBACK_FAILED") {
        try {
          const detection = await refreshDetection();
          if (targetIsReusable(detection, target))
            onReturnToOverview(retryIssue(code, target));
          else onReturnToOverview();
        } catch {
          onReturnToOverview({ kind: "detect", code });
        }
      } else {
        onReturnToOverview(writeIssue(code, target));
      }
    } finally {
      writeInFlightRef.current = false;
      setBusy(false);
    }
  }

  async function importConfig(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) return;
    if (
      file.size > 2 * 1024 * 1024 ||
      !file.name.toLowerCase().endsWith(".json")
    ) {
      setMessage(t("agents.error.import"));
      return;
    }
    try {
      const value = await api.importAgentModelConfig(
        await file.text(),
        agents,
        discovery.flow_id,
      );
      if (!validateSingleTargetConfig(value, target.agent))
        throw new Error("invalid target");
      setConfig(value);
      setFieldsKey((current) => current + 1);
      setDraftState({
        error: validateAgentConfig(value, target.agent),
        hasLocalDraft: false,
      });
      setSources({ claude: "empty", opencode: "empty", codex: "empty" });
      setMessage(t("agents.imported"));
    } catch {
      setMessage(t("agents.error.import"));
    }
  }

  async function exportConfig() {
    const snapshot = fieldsRef.current?.getSnapshot();
    if (!snapshot || snapshot.error) {
      setMessage(snapshot?.error ?? draftState.error);
      return;
    }
    try {
      const content = await api.exportAgentModelConfig(
        snapshot.config,
        agents,
        discovery.flow_id,
      );
      const url = URL.createObjectURL(
        new Blob([content], { type: "application/json" }),
      );
      const link = document.createElement("a");
      link.href = url;
      link.download = "mtls-router-model-config.json";
      link.click();
      URL.revokeObjectURL(url);
    } catch {
      setMessage(t("agents.error.export"));
    }
  }

  async function finish() {
    setBusy(true);
    try {
      await refreshDetection();
      onReturnToOverview();
    } catch {
      onReturnToOverview({ kind: "detect", code: "AGENT_DETECT_FAILED" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="agents-workbench" aria-labelledby="agents-heading">
      <header className="agents-workbench__header">
        <div>
          <p className="overline">{t("agents.overline")}</p>
          <h2 id="agents-heading">{agentNames[target.agent]}</h2>
        </div>
      </header>
      {message && (
        <p className="agent-alert" role="alert">
          {message}
        </p>
      )}
      {stage === "write" && (
        <div className="processing-stage" role="status">
          <span className="instrument__dial">TX</span>
          <h3>{t("agents.executing")}</h3>
        </div>
      )}
      {stage === "configure" && (
        <div className="config-workbench">
          <AgentConfigFields
            ref={fieldsRef}
            target={target}
            discovery={discovery}
            config={config}
            disabled={false}
            resetToken={fieldsKey}
            onChange={setConfig}
            onDraftStateChange={setDraftState}
            beforeFields={
              <>
                {discovery.existing.drifted_agents.includes(target.agent) && (
                  <p className="drift-note" role="note">
                    {t("agents.existingDrift", { agents: target.agent })}
                  </p>
                )}
                {(discovery.existing.unavailable_models[target.agent]?.length ??
                  0) > 0 && (
                  <p className="drift-note" role="note">
                    {t("agents.unavailableModels", {
                      agent: target.agent,
                      models:
                        discovery.existing.unavailable_models[
                          target.agent
                        ]!.map(safe).join(", "),
                    })}
                  </p>
                )}
                {sources[target.agent] !== "empty" && (
                  <p className="source-note" role="note">
                    {t("agents.initializationSource", {
                      agent: agentNames[target.agent],
                      source: t(
                        sources[target.agent] === "existing"
                          ? "agents.source.existing"
                          : "agents.source.preset",
                      ),
                    })}
                  </p>
                )}
                {(discovery.preset.unavailable_agents[target.agent]?.models
                  .length ?? 0) > 0 && (
                  <p className="drift-note" role="note">
                    {t("agents.presetUnavailable", {
                      agent: agentNames[target.agent],
                      models:
                        discovery.preset.unavailable_agents[
                          target.agent
                        ]!.models.map(safe).join(", "),
                    })}
                  </p>
                )}
              </>
            }
            afterFields={
              <>
                <div className="config-actions">
                  <label className="text-button file-button">
                    {t("agents.importConfig")}
                    <input
                      type="file"
                      accept="application/json,.json"
                      onChange={(event) => void importConfig(event)}
                    />
                  </label>
                  <button
                    className="text-button"
                    disabled={Boolean(draftState.error)}
                    onClick={() => void exportConfig()}
                  >
                    {t("agents.exportConfig")}
                  </button>
                  <button
                    className="control-button"
                    disabled={Boolean(draftState.error) || busy}
                    onClick={() => void loadPreview()}
                  >
                    {t("agents.generatePreview")}
                  </button>
                  <button className="text-button" onClick={onBack}>
                    {t("agents.cancelDetection")}
                  </button>
                </div>
                {draftState.error && (
                  <p className="validation-path" role="alert">
                    {draftState.error}
                  </p>
                )}
              </>
            }
          />
        </div>
      )}
      {(stage === "preview" || stage === "result") && (
        <AgentPreviewPane
          key={`${discovery.flow_id}:${preview?.revision_token ?? stage}`}
          target={target}
          preview={stage === "preview" ? preview : null}
          result={stage === "result" ? result : null}
          busy={busy}
          onGenerate={() => void loadPreview()}
          onBackToEdit={() => setStage("configure")}
          onWrite={(approvals) =>
            void write(
              approvals.managedOverwrite,
              approvals.codexAuthChange,
              approvals.rebuild,
            )
          }
          onCancel={onBack}
          onFinish={() => void finish()}
        />
      )}
    </section>
  );
}
