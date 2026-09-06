import {
  useEffect,
  useEffectEvent,
  useLayoutEffect,
  useRef,
  useState,
  type ChangeEvent,
} from "react";

import {
  AgentConfigFields,
  type AgentConfigFieldsHandle,
} from "./AgentConfigFields";
import { AgentPreviewPane } from "./AgentPreviewPane";
import {
  agentNames,
  configurationPresentation,
  recoveryReason,
  recoveryReasons,
} from "./agentPresentation";
import { useI18n } from "./i18n";
import {
  sanitizeSensitiveText,
  type AgentDetection,
  type AgentId,
  type DesktopApi,
} from "./ipc";
import { useAgentPanelController } from "./useAgentPanelController";

export interface AgentPanelGuardState {
  dirty: boolean;
  busy: boolean;
}

interface AgentPanelProps {
  api: DesktopApi;
  target: AgentId;
  onBack(): void;
  onNavigateToApiKeys(): void;
  onRetrySession?(): void;
  onReloaded?(detection: AgentDetection): void;
  onGuardStateChange?(state: AgentPanelGuardState): void;
  onDirtyChange?(dirty: boolean): void;
}

function phaseMessage(kind: string) {
  if (kind === "preview-loading") return "agents.panel.previewLoading" as const;
  if (kind === "writing") return "agents.executing" as const;
  if (kind === "reloading") return "agents.panel.reloading" as const;
  return "agents.discovering" as const;
}

function activePhase(kind: string, hasResult: boolean) {
  if (kind === "writing" || kind === "reloading") return "write";
  if (hasResult || kind === "reload-failed") return "result";
  if (kind === "loading") return "discover";
  if (kind === "readonly" || kind === "blocked-dirty" || kind === "editing")
    return "configure";
  if (kind === "preview-loading" || kind === "previewing") return "preview";
  return "configure";
}

export function AgentPanel({
  api,
  target,
  onBack,
  onNavigateToApiKeys,
  onRetrySession = () => undefined,
  onReloaded = () => undefined,
  onGuardStateChange = () => undefined,
  onDirtyChange = () => undefined,
}: AgentPanelProps) {
  const { t } = useI18n();
  const controller = useAgentPanelController({
    api,
    target,
    onDirtyChange: () => undefined,
  });
  const dirty = controller.dirty;
  const fieldsRef = useRef<AgentConfigFieldsHandle>(null);
  const editorRef = useRef<HTMLDivElement>(null);
  const previewHeadingRef = useRef<HTMLHeadingElement>(null);
  const reportedReloadRef = useRef("");
  const pendingReloadTransactionRef = useRef("");
  const discoveryFlowRef = useRef<string | null>(null);
  const restoreEditorFocusRef = useRef(false);
  const [resetToken, setResetToken] = useState(0);
  const reportGuardState = useEffectEvent(onGuardStateChange);
  const reportDirty = useEffectEvent(onDirtyChange);
  const reportReloaded = useEffectEvent(onReloaded);
  const busy = ["preview-loading", "writing", "reloading"].includes(
    controller.phase.kind,
  );
  const hasFields = Boolean(
    controller.target && controller.discovery && controller.config,
  );
  const showProcessing = controller.phase.kind === "loading" || busy;
  const showWorkspaceProcessing = showProcessing && !hasFields;
  const showRailProcessing = showProcessing && hasFields;
  const showIdle = controller.phase.kind === "editing" && !controller.result;
  const showPreview =
    controller.phase.kind === "previewing" && Boolean(controller.target);
  const showResult = Boolean(controller.result && controller.target);
  const showReadonly = controller.phase.kind === "readonly";
  const showBlocked = controller.phase.kind === "blocked-dirty";
  const showReloadFailed = controller.phase.kind === "reload-failed";
  const showRail =
    Boolean(controller.issue) ||
    showResult ||
    showPreview ||
    showIdle ||
    showReadonly ||
    showBlocked ||
    showReloadFailed ||
    showRailProcessing;
  const statusOnly = showWorkspaceProcessing && !showRail;
  const fieldResetToken =
    resetToken +
    (controller.preview ? 100_000 : 0) +
    (controller.result ? 1_000_000 : 0);
  const targetState = controller.detection?.agents.find(
    (agent) => agent.agent === target,
  );
  const readonlyCredential =
    controller.phase.kind === "readonly" &&
    (controller.phase.reason.kind === "credential" ||
      ("code" in controller.phase.reason &&
        (controller.phase.reason.code.startsWith("CREDENTIAL_") ||
          controller.phase.reason.code === "MODEL_AUTH_FAILED")));

  useLayoutEffect(() => {
    reportGuardState({ dirty, busy });
    reportDirty(dirty);
  }, [busy, dirty]);

  useEffect(() => {
    if (controller.phase.kind !== "previewing") return;
    if (window.matchMedia?.("(max-width: 900px)").matches) {
      previewHeadingRef.current?.focus();
    }
  }, [controller.phase.kind]);

  useEffect(() => {
    const flow = controller.discovery?.flow_id ?? null;
    if (discoveryFlowRef.current === null) {
      discoveryFlowRef.current = flow;
      return;
    }
    if (flow && flow !== discoveryFlowRef.current && !dirty) {
      setResetToken((value) => value + 1);
    }
    discoveryFlowRef.current = flow;
  }, [controller.discovery?.flow_id, dirty]);

  useEffect(() => {
    if (controller.result || !restoreEditorFocusRef.current) return;
    restoreEditorFocusRef.current = false;
    editorRef.current?.focus();
  }, [controller.result]);

  useEffect(() => {
    const successfulResult = controller.result?.agents.some(
      (agent) => agent.agent === target && agent.success,
    );
    if (successfulResult && controller.result) {
      pendingReloadTransactionRef.current = controller.result.transaction_id;
    }
    const transaction = pendingReloadTransactionRef.current;
    if (
      controller.phase.kind !== "editing" ||
      !transaction ||
      !controller.detection ||
      reportedReloadRef.current === transaction
    )
      return;
    reportedReloadRef.current = transaction;
    pendingReloadTransactionRef.current = "";
    reportReloaded(controller.detection);
  }, [controller.detection, controller.phase.kind, controller.result, target]);

  function synchronizeFields() {
    const snapshot = fieldsRef.current?.getSnapshot();
    if (!snapshot) return false;
    controller.setConfig(snapshot.config);
    controller.setDraftState({
      error: snapshot.error,
      hasLocalDraft: snapshot.hasLocalDraft,
    });
    return !snapshot.error;
  }

  function generatePreview() {
    if (!synchronizeFields()) return;
    void controller.generatePreview();
  }

  async function importConfig(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) return;
    await controller.importConfig(file);
    setResetToken((value) => value + 1);
  }

  async function discardBlockedDraft() {
    await controller.discardBlockedDraft();
    setResetToken((value) => value + 1);
  }

  const fields =
    controller.target && controller.discovery && controller.config ? (
      <AgentConfigFields
        ref={fieldsRef}
        target={controller.target}
        discovery={controller.discovery}
        config={controller.config}
        disabled={!controller.operations.edit}
        resetToken={fieldResetToken}
        onChange={controller.setConfig}
        onDraftStateChange={controller.setDraftState}
        beforeFields={
          <>
            {controller.source && controller.source !== "empty" && (
              <p className="source-note" role="note">
                {t("agents.initializationSource", {
                  agent: agentNames[target],
                  source:
                    controller.source === "existing"
                      ? t("agents.source.existing")
                      : controller.source === "preset"
                        ? t("agents.source.preset")
                        : t("agents.panel.importedSource"),
                })}
              </p>
            )}
            {controller.target?.mode === "rebuild" && (
              <p className="drift-note" role="alert">
                {t("agents.recovery.guidance.eligible")}{" "}
                {t("agents.recovery.warning")}
              </p>
            )}
            {controller.phase.kind === "editing" &&
              controller.phase.refresh.kind === "failed" && (
                <p className="drift-note" role="alert">
                  {t("agents.panel.refreshFailed", {
                    code: sanitizeSensitiveText(controller.phase.refresh.code),
                  })}
                </p>
              )}
            {controller.phase.kind === "editing" &&
              controller.phase.refresh.kind === "conflict" && (
                <div className="agent-panel__conflict" role="alert">
                  <p>{t("agents.panel.refreshConflict")}</p>
                  <button
                    type="button"
                    className="text-button"
                    onClick={() => controller.resolveConflict("preserve")}
                  >
                    {t("agents.panel.keepDraft")}
                  </button>
                  <button
                    type="button"
                    className="control-button control-button--danger"
                    onClick={() => {
                      controller.resolveConflict("discard");
                      setResetToken((value) => value + 1);
                    }}
                  >
                    {t("agents.panel.discard")}
                  </button>
                </div>
              )}
          </>
        }
        afterFields={
          controller.phase.kind === "editing" ? (
            <>
              <div className="config-actions agent-panel__actions">
                <label className="text-button file-button">
                  {t("agents.importConfig")}
                  <input
                    type="file"
                    accept="application/json,.json"
                    disabled={!controller.operations.import}
                    onChange={(event) => void importConfig(event)}
                  />
                </label>
                <button
                  type="button"
                  className="text-button"
                  disabled={
                    !controller.operations.export ||
                    Boolean(controller.draftState.error)
                  }
                  onClick={() => {
                    if (!synchronizeFields()) return;
                    void controller.exportConfig();
                  }}
                >
                  {t("agents.exportConfig")}
                </button>
                <button
                  type="button"
                  className="text-button"
                  disabled={controller.phase.refresh.kind === "checking"}
                  onClick={() => void controller.refresh()}
                >
                  {controller.phase.refresh.kind === "checking"
                    ? t("agents.overview.refreshing")
                    : t("agents.overview.refresh")}
                </button>
                <button type="button" className="text-button" onClick={onBack}>
                  {t("agents.panel.back")}
                </button>
              </div>
              {controller.draftState.error && (
                <p className="validation-path" role="alert">
                  {controller.draftState.error}
                </p>
              )}
            </>
          ) : undefined
        }
      />
    ) : null;

  return (
    <section
      className="agent-panel agents-workbench"
      aria-labelledby="agent-panel-heading"
      data-phase={controller.phase.kind}
      data-busy={busy}
      aria-busy={busy}
    >
      <header className="agents-workbench__header agent-panel__header">
        <div>
          <p className="overline">{t("agents.overline")}</p>
          <h2 id="agent-panel-heading">{agentNames[target]}</h2>
        </div>
        <button type="button" className="text-button" onClick={onBack}>
          {t("agents.panel.back")}
        </button>
      </header>
      <ol
        className="agent-phase-rail"
        aria-label={t("agents.panel.previewSteps")}
      >
        {(
          [
            ["discover", "agents.stage.discover"],
            ["configure", "agents.stage.configure"],
            ["preview", "agents.stage.preview"],
            ["write", "agents.stage.write"],
            ["result", "agents.stage.result"],
          ] as const
        ).map(([phase, label]) => (
          <li
            key={phase}
            className={
              activePhase(controller.phase.kind, Boolean(controller.result)) ===
              phase
                ? "is-current"
                : undefined
            }
          >
            {t(label)}
          </li>
        ))}
      </ol>

      <div
        className={
          statusOnly
            ? "agent-panel__workspace agent-panel__workspace--status"
            : "agent-panel__workspace"
        }
      >
        {hasFields && (
          <div ref={editorRef} className="agent-panel__editor" tabIndex={-1}>
            {fields}
          </div>
        )}

        {showWorkspaceProcessing && (
          <div
            className="processing-stage agent-panel__processing"
            role="status"
          >
            <h3>{t(phaseMessage(controller.phase.kind))}</h3>
          </div>
        )}

        {showRail && (
          <aside className="agent-panel__rail" aria-live="polite">
            {controller.issue && (
              <p
                className={`agent-alert agent-panel__notice agent-panel__notice--${controller.issue.kind}`}
                role={controller.issue.kind === "success" ? "status" : "alert"}
              >
                {controller.issue.kind === "success"
                  ? t("agents.panel.writeComplete")
                  : t("agents.panel.operationFailed", {
                      code: sanitizeSensitiveText(controller.issue.code),
                    })}
              </p>
            )}

            {showRailProcessing && (
              <div
                className="processing-stage agent-panel__processing"
                role="status"
              >
                <h3>{t(phaseMessage(controller.phase.kind))}</h3>
              </div>
            )}

            {showResult && controller.target && controller.result && (
              <AgentPreviewPane
                target={controller.target}
                preview={null}
                result={controller.result}
                busy={busy}
                onGenerate={generatePreview}
                onBackToEdit={controller.returnToEditing}
                onWrite={(approvals) => void controller.write(approvals)}
                onCancel={onBack}
                onFinish={() => {
                  restoreEditorFocusRef.current = true;
                  controller.dismissResult();
                }}
              />
            )}

            {showPreview && controller.target && (
              <div className="agent-panel__preview">
                <h3 ref={previewHeadingRef} tabIndex={-1}>
                  {t("agents.reviewScope")}
                </h3>
                <AgentPreviewPane
                  target={controller.target}
                  preview={controller.preview}
                  result={null}
                  busy={false}
                  onGenerate={generatePreview}
                  onBackToEdit={controller.returnToEditing}
                  onWrite={(approvals) => void controller.write(approvals)}
                  onCancel={onBack}
                  onFinish={controller.dismissResult}
                />
              </div>
            )}

            {showIdle && (
              <div
                className="agent-panel__state agent-panel__state--idle"
                role="status"
              >
                <p className="overline">{t("agents.panel.status")}</p>
                <strong>
                  {dirty ? t("agents.panel.unsaved") : t("agents.panel.ready")}
                </strong>
                <p>{t("agents.panel.previewSteps")}</p>
                <button
                  type="button"
                  className="control-button"
                  disabled={
                    !controller.operations.preview ||
                    Boolean(controller.draftState.error)
                  }
                  onClick={generatePreview}
                >
                  {t("agents.generatePreview")}
                </button>
              </div>
            )}

            {showReadonly && controller.phase.kind === "readonly" && (
              <div className="agent-panel__state" role="alert">
                <p>{t("agents.panel.readonly")}</p>
                {targetState && (
                  <dl className="agent-panel__metadata">
                    <div>
                      <dt>{t("agents.effect.path")}</dt>
                      <dd>{sanitizeSensitiveText(targetState.path)}</dd>
                    </div>
                    <div>
                      <dt>{t("agents.format")}</dt>
                      <dd>{sanitizeSensitiveText(targetState.format)}</dd>
                    </div>
                    <div>
                      <dt>{t("agents.exists")}</dt>
                      <dd>
                        {targetState.exists
                          ? t("agents.exists")
                          : t("agents.pendingCreate")}
                      </dd>
                    </div>
                    <div>
                      <dt>{t("agents.panel.configurationState")}</dt>
                      <dd>
                        {t(
                          `agents.configuration.${configurationPresentation(targetState).state}`,
                        )}
                      </dd>
                    </div>
                    <div>
                      <dt>{t("agents.permission")}</dt>
                      <dd>
                        {targetState.writable
                          ? t("agents.writable")
                          : t("agents.configuration.readonly")}
                      </dd>
                    </div>
                  </dl>
                )}
                {targetState && recoveryReasons(targetState).length > 0 && (
                  <ul className="agent-recovery-reasons">
                    {recoveryReasons(targetState).map((reason) => (
                      <li key={reason}>{recoveryReason(reason, t)}</li>
                    ))}
                  </ul>
                )}
                {"code" in controller.phase.reason && (
                  <p>
                    {t("agents.panel.operationFailed", {
                      code: sanitizeSensitiveText(controller.phase.reason.code),
                    })}
                  </p>
                )}
                {readonlyCredential ? (
                  <button
                    type="button"
                    className="control-button"
                    onClick={onNavigateToApiKeys}
                  >
                    {t("agents.issue.toApiKeys")}
                  </button>
                ) : (
                  <button
                    type="button"
                    className="control-button"
                    onClick={onRetrySession}
                  >
                    {t("agents.panel.retry")}
                  </button>
                )}
                <button type="button" className="text-button" onClick={onBack}>
                  {t("agents.panel.back")}
                </button>
              </div>
            )}

            {showBlocked && controller.phase.kind === "blocked-dirty" && (
              <div className="agent-panel__state" role="alert">
                <p>{t("agents.panel.blockedDirty")}</p>
                {controller.phase.errorCode && (
                  <p>
                    {t("agents.panel.operationFailed", {
                      code: sanitizeSensitiveText(controller.phase.errorCode),
                    })}
                  </p>
                )}
                <div className="config-actions">
                  <button
                    type="button"
                    className="text-button"
                    disabled={!controller.operations.export}
                    onClick={() => void controller.exportConfig()}
                  >
                    {t("agents.exportConfig")}
                  </button>
                  <button
                    type="button"
                    className="text-button"
                    onClick={() => void controller.retryBlockedDraft()}
                  >
                    {t("agents.panel.retry")}
                  </button>
                  <button
                    type="button"
                    className="control-button control-button--danger"
                    onClick={() => void discardBlockedDraft()}
                  >
                    {t("agents.panel.discard")}
                  </button>
                </div>
              </div>
            )}

            {showReloadFailed && controller.phase.kind === "reload-failed" && (
              <div className="agent-panel__state" role="alert">
                <p>
                  {t("agents.panel.reloadFailed", {
                    code: sanitizeSensitiveText(controller.phase.code),
                  })}
                </p>
                <button
                  type="button"
                  className="control-button"
                  onClick={() => void controller.refresh()}
                >
                  {t("agents.panel.retry")}
                </button>
              </div>
            )}
          </aside>
        )}
      </div>
    </section>
  );
}
