import { useEffect, useRef } from "react";

import { AgentFileEffectCard } from "./AgentPreviewPane";
import { agentNames } from "./agentPresentation";
import { useI18n } from "./i18n";
import {
  sanitizeSensitiveText,
  type AgentId,
  type AgentWriteResult,
  type DesktopApi,
} from "./ipc";
import { useAgentCleanupController } from "./useAgentCleanupController";

interface AgentCleanupPanelProps {
  api: DesktopApi;
  agent: AgentId;
  onBack(): void;
  onBusyChange(busy: boolean): void;
  onComplete(): void;
}

function safe(value: string) {
  return sanitizeSensitiveText(value);
}

function CleanupResult({
  result,
  onFinish,
}: {
  result: AgentWriteResult;
  onFinish(): void;
}) {
  const { t } = useI18n();
  const headingRef = useRef<HTMLHeadingElement>(null);

  useEffect(() => {
    headingRef.current?.focus();
  }, []);

  return (
    <div className="agent-results cleanup-results">
      <div className="result-banner" role="status">
        <span>{t("agents.cleanup.completeOverline")}</span>
        <h3 ref={headingRef} tabIndex={-1}>
          {t("agents.cleanup.complete")}
        </h3>
        <p>{t("agents.cleanup.completeNote")}</p>
      </div>
      <div className="result-grid">
        {result.agents.map((agent) => (
          <article key={agent.agent}>
            <div className="result-card__heading">
              <h4>{agentNames[agent.agent]}</h4>
              <span className={agent.success ? "result-ok" : "result-fail"}>
                {agent.success ? t("agents.success") : t("agents.failure")}
              </span>
            </div>
            {agent.changed?.map((path) => (
              <div className="cleanup-result-path" key={`changed-${path}`}>
                <span>{t("agents.changed")}</span>
                <code>{safe(path)}</code>
              </div>
            ))}
            {agent.backups?.map((path) => (
              <div className="cleanup-result-path" key={`backup-${path}`}>
                <span>{t("agents.backups")}</span>
                <code className="backup-path">{safe(path)}</code>
              </div>
            ))}
            {agent.error_code && (
              <p>{t("agents.errorCode", { code: safe(agent.error_code) })}</p>
            )}
          </article>
        ))}
      </div>
      {(result.state_change || result.state_backup) && (
        <div className="preview-files result-state-effects">
          {result.state_change && (
            <AgentFileEffectCard effect={result.state_change} />
          )}
          {result.state_backup && (
            <AgentFileEffectCard effect={result.state_backup} />
          )}
        </div>
      )}
      <button type="button" className="control-button" onClick={onFinish}>
        {t("agents.cleanup.finish")}
      </button>
    </div>
  );
}

export function AgentCleanupPanel({
  api,
  agent,
  onBack,
  onBusyChange,
  onComplete,
}: AgentCleanupPanelProps) {
  const { t } = useI18n();
  const controller = useAgentCleanupController({ api, agent });
  const statusHeadingRef = useRef<HTMLHeadingElement>(null);
  const finishRequestedRef = useRef(false);

  useEffect(() => {
    onBusyChange(controller.busy);
  }, [controller.busy, onBusyChange]);

  useEffect(() => {
    if (controller.phase.kind === "loading-preview") {
      statusHeadingRef.current?.focus();
    }
  }, [controller.phase.kind]);

  const phase = controller.phase;
  const preview =
    phase.kind === "previewing" ||
    phase.kind === "writing" ||
    phase.kind === "repreview-required" ||
    (phase.kind === "failed" && phase.preview)
      ? phase.preview
      : null;

  return (
    <section
      className="agents-workbench agent-cleanup"
      aria-labelledby="cleanup-heading"
    >
      <header className="agents-workbench__header cleanup-header">
        <div>
          <p className="overline">{t("agents.cleanup.overline")}</p>
          <h2 id="cleanup-heading">
            {t("agents.cleanup.heading", { agent: agentNames[agent] })}
          </h2>
        </div>
        {phase.kind !== "result" && (
          <button
            type="button"
            className="text-button"
            disabled={controller.busy}
            onClick={onBack}
          >
            {t("agents.panel.back")}
          </button>
        )}
      </header>

      {phase.kind === "loading-preview" && (
        <div className="processing-stage" role="status">
          <h3 ref={statusHeadingRef} tabIndex={-1}>
            {t("agents.cleanup.loading")}
          </h3>
          <p>{t("agents.cleanup.keyFree")}</p>
        </div>
      )}

      {phase.kind === "result" && (
        <CleanupResult
          result={phase.result}
          onFinish={() => {
            if (finishRequestedRef.current) return;
            finishRequestedRef.current = true;
            onComplete();
          }}
        />
      )}

      {phase.kind === "failed" && (
        <div className="agent-alert cleanup-alert" role="alert">
          <span>{t("agents.cleanup.failed", { code: safe(phase.code) })}</span>
          <button
            type="button"
            className="text-button"
            onClick={() => void controller.retry()}
          >
            {t(
              phase.preview
                ? "agents.cleanup.retryWrite"
                : "agents.cleanup.retryPreview",
            )}
          </button>
        </div>
      )}

      {phase.kind === "repreview-required" && (
        <div className="agent-alert cleanup-alert" role="alert">
          <span>
            {phase.code === "PREVIEW_STALE"
              ? t("agents.cleanup.stale")
              : t("agents.cleanup.ambiguous", { code: safe(phase.code) })}
          </span>
          <button
            type="button"
            className="text-button"
            onClick={() => void controller.repreview()}
          >
            {t("agents.cleanup.repreview")}
          </button>
        </div>
      )}

      {preview && (
        <div className="preview-layout cleanup-preview-layout">
          <div className="preview-main">
            <section className="preview-agent">
              <div className="preview-agent__heading">
                <h3>{t("agents.cleanup.removedPaths")}</h3>
              </div>
              <ul className="cleanup-removed-paths">
                {preview.removed_paths.map((path) => (
                  <li key={path}>
                    <code>{safe(path)}</code>
                  </li>
                ))}
              </ul>
            </section>
            <section className="preview-agent">
              <div className="preview-agent__heading">
                <h3>{t("agents.cleanup.fileEffects")}</h3>
              </div>
              <div className="preview-files">
                {preview.files.map((effect) => (
                  <AgentFileEffectCard
                    key={`${effect.role}-${effect.path}`}
                    effect={effect}
                  />
                ))}
                {preview.state_change && (
                  <AgentFileEffectCard effect={preview.state_change} />
                )}
                {preview.state_backup && (
                  <AgentFileEffectCard effect={preview.state_backup} />
                )}
              </div>
            </section>
          </div>
          <aside className="approval-rail cleanup-approval-rail">
            <p className="overline">{t("agents.approvalBoundary")}</p>
            <h3>{t("agents.cleanup.review")}</h3>
            <div className="cleanup-retention-notes">
              <p>{t("agents.cleanup.authRemoved")}</p>
              <p>{t("agents.cleanup.globalKeyRetained")}</p>
              <p>{t("agents.cleanup.backupsRetained")}</p>
            </div>
            {preview.managed_config_drift && (
              <label className="approval-check cleanup-drift-approval">
                <input
                  type="checkbox"
                  checked={
                    phase.kind === "previewing" ? phase.driftApproved : true
                  }
                  disabled={phase.kind !== "previewing"}
                  onChange={(event) =>
                    controller.setDriftApproved(event.target.checked)
                  }
                />
                {t("agents.cleanup.approveDrift")}
              </label>
            )}
            {(phase.kind === "previewing" || phase.kind === "writing") && (
              <button
                type="button"
                className="control-button control-button--danger"
                disabled={!controller.canWrite || controller.busy}
                onClick={() => void controller.write()}
              >
                {phase.kind === "writing"
                  ? t("agents.cleanup.writing")
                  : t("agents.cleanup.write")}
              </button>
            )}
          </aside>
        </div>
      )}
    </section>
  );
}
