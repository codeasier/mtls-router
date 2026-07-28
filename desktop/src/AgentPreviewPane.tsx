import { useEffect, useRef, useState } from "react";

import type { AgentTarget } from "./agentPresentation";
import { agentNames } from "./agentPresentation";
import { useI18n } from "./i18n";
import {
  sanitizeSensitiveText,
  type AgentFileEffect,
  type AgentPreview,
  type AgentWriteResult,
} from "./ipc";
import type { WriteApprovals } from "./agentPanelState";
import { validateSingleTargetConfig } from "./AgentConfigFields";

function safe(value: string | undefined) {
  return sanitizeSensitiveText(value ?? "");
}

function FileEffect({ effect }: { effect: AgentFileEffect }) {
  const { t } = useI18n();
  const operation =
    effect.operation === "create"
      ? t("agents.operation.create")
      : effect.operation === "replace"
        ? t("agents.operation.replace")
        : effect.operation === "preserve"
          ? t("agents.operation.preserve")
          : safe(effect.operation);
  return (
    <article
      className={`effect-card${effect.mode === "rebuild" ? " effect-card--rebuild" : ""}`}
    >
      <div className="effect-card__badges">
        <span>{operation}</span>
        {effect.mode && <span>{t(`agents.mode.${effect.mode}`)}</span>}
        {effect.backup_sensitive && (
          <span className="effect-card__sensitive">
            {t("agents.sensitiveBackup")}
          </span>
        )}
      </div>
      <dl className="effect-card__details">
        {effect.agent && (
          <div>
            <dt>{t("agents.effect.agent")}</dt>
            <dd>{agentNames[effect.agent]}</dd>
          </div>
        )}
        <div>
          <dt>{t("agents.effect.role")}</dt>
          <dd>{safe(effect.role)}</dd>
        </div>
      </dl>
      <span className="effect-card__label">{t("agents.effect.path")}</span>
      <code>{safe(effect.path)}</code>
      {effect.backup_required && effect.backup_pattern && (
        <>
          <span className="effect-card__label">
            {t("agents.effect.backupPattern")}
          </span>
          <code className="backup-path">{safe(effect.backup_pattern)}</code>
        </>
      )}
      {effect.backup_path && (
        <code className="backup-path">{safe(effect.backup_path)}</code>
      )}
      {effect.preserves && effect.preserves.length > 0 && (
        <p>
          <strong>{t("agents.effect.preserves")}</strong>{" "}
          {effect.preserves.map(safe).join(", ")}
        </p>
      )}
      {effect.warning && (
        <p className="effect-card__warning">
          <strong>{t("agents.effect.warning")}</strong> {safe(effect.warning)}
        </p>
      )}
    </article>
  );
}

// The workflow rechecks this immediately before dispatching the write request.
// eslint-disable-next-line react-refresh/only-export-components
export function validateRebuildPreview(
  preview: AgentPreview | null,
  target: AgentTarget,
) {
  if (
    !preview ||
    !validateSingleTargetConfig(preview.model_config, target.agent)
  )
    return false;
  if (preview.fragments.some((fragment) => fragment.agent !== target.agent))
    return false;
  if (preview.drifted_agents.some((agent) => agent !== target.agent))
    return false;
  if (
    preview.managed_collisions.some(
      (collision) => collision.agent !== target.agent,
    )
  )
    return false;
  if (preview.files.some((effect) => effect.agent !== target.agent))
    return false;
  if (
    [preview.state_change, preview.state_backup].some(
      (effect) =>
        effect && ((effect.agent ?? "") !== "" || (effect.mode ?? "") !== ""),
    )
  )
    return false;
  const rebuildEffects = preview.files.filter(
    (effect) => effect.mode === "rebuild",
  );
  const unique = new Set(
    rebuildEffects.map((effect) => `${effect.agent}\0${effect.path}`),
  );
  if (unique.size !== rebuildEffects.length) return false;
  if (target.mode === "merge") return rebuildEffects.length === 0;
  return (
    rebuildEffects.length > 0 &&
    rebuildEffects.every((effect) => effect.agent === target.agent) &&
    !preview.files.some(
      (effect) => effect.agent === target.agent && effect.mode === "merge",
    )
  );
}

export interface AgentPreviewPaneProps {
  target: AgentTarget;
  preview: AgentPreview | null;
  result: AgentWriteResult | null;
  busy: boolean;
  onGenerate(): void;
  onBackToEdit(): void;
  onWrite(approvals: WriteApprovals): void;
  onCancel(): void;
  onFinish(): void;
}

export function AgentPreviewPane(props: AgentPreviewPaneProps) {
  const identity = props.preview
    ? `${props.target.agent}:${props.target.mode}:${props.preview.revision_token}`
    : props.result
      ? `${props.target.agent}:result:${props.result.transaction_id}`
      : `${props.target.agent}:empty`;
  return <AgentPreviewPaneContent key={identity} {...props} />;
}

function AgentPreviewPaneContent({
  target,
  preview,
  result,
  busy,
  onGenerate,
  onBackToEdit,
  onWrite,
  onCancel,
  onFinish,
}: AgentPreviewPaneProps) {
  const { t } = useI18n();
  const [approveDrift, setApproveDrift] = useState(false);
  const [approveAuth, setApproveAuth] = useState(false);
  const [rebuildDialogOpen, setRebuildDialogOpen] = useState(false);
  const cancelRebuildRef = useRef<HTMLButtonElement>(null);
  const confirmRebuildRef = useRef<HTMLButtonElement>(null);
  const rebuildTriggerRef = useRef<HTMLButtonElement>(null);
  const previewValid = validateRebuildPreview(preview, target);

  useEffect(() => {
    if (!rebuildDialogOpen) return;
    cancelRebuildRef.current?.focus();
    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape" && !busy) {
        rebuildTriggerRef.current?.focus();
        setRebuildDialogOpen(false);
      }
    }
    window.addEventListener("keydown", handleEscape);
    return () => window.removeEventListener("keydown", handleEscape);
  }, [busy, rebuildDialogOpen]);

  function approvals(rebuild: boolean): WriteApprovals {
    return {
      managedOverwrite: approveDrift,
      codexAuthChange: approveAuth,
      rebuild: rebuild ? [target.agent] : [],
    };
  }

  function closeRebuildDialog() {
    rebuildTriggerRef.current?.focus();
    setRebuildDialogOpen(false);
  }

  if (result) {
    const targetResult = result.agents.find(
      (agent) => agent.agent === target.agent,
    );
    const showInstallLater =
      !target.installedAtEntry && targetResult?.success === true;
    return (
      <div className="agent-results">
        <div className="result-banner">
          <span>{t("agents.transactionComplete")}</span>
          <h3>{t("agents.resultHeading")}</h3>
        </div>
        {showInstallLater && (
          <p className="result-install-note" role="note">
            {t("agents.result.installLater", {
              agent: agentNames[target.agent],
            })}
          </p>
        )}
        <div className="result-grid">
          {result.agents.map((agent) => (
            <article key={agent.agent}>
              <div className="result-card__heading">
                <h4>{agentNames[agent.agent]}</h4>
                {target.mode === "rebuild" && (
                  <span className="result-mode">
                    {t("agents.mode.rebuild")}
                  </span>
                )}
                <span className={agent.success ? "result-ok" : "result-fail"}>
                  {agent.success ? t("agents.success") : t("agents.failure")}
                </span>
              </div>
              {agent.changed?.map((path) => (
                <div className="result-path" key={path}>
                  <span>{t("agents.changed")}</span>
                  <code>{safe(path)}</code>
                </div>
              ))}
              {agent.backups?.map((path) => (
                <div className="result-path result-path--backup" key={path}>
                  <span>{t("agents.backups")}</span>
                  <code>{safe(path)}</code>
                </div>
              ))}
              {agent.error_code && (
                <p className="result-path result-path--error">
                  {t("agents.errorCode", { code: safe(agent.error_code) })}
                </p>
              )}
            </article>
          ))}
        </div>
        {(result.state_change || result.state_backup) && (
          <div className="preview-files result-state-effects">
            {result.state_change && <FileEffect effect={result.state_change} />}
            {result.state_backup && <FileEffect effect={result.state_backup} />}
          </div>
        )}
        <button className="control-button" disabled={busy} onClick={onFinish}>
          {t("agents.panel.continueEditing")}
        </button>
      </div>
    );
  }

  if (!preview) {
    return (
      <button className="control-button" disabled={busy} onClick={onGenerate}>
        {t("agents.generatePreview")}
      </button>
    );
  }

  return (
    <>
      <div className="preview-layout">
        <div className="preview-main">
          <section className="preview-agent">
            <div className="preview-agent__heading">
              <h3>{t("agents.fragments")}</h3>
            </div>
            {preview.fragments.map((fragment) => (
              <article
                className="fragment-card"
                key={`${fragment.agent}-${fragment.role}`}
              >
                <strong>
                  {agentNames[fragment.agent]} / {safe(fragment.role)}
                </strong>
                <code>{safe(fragment.path)}</code>
                <pre>{safe(fragment.content)}</pre>
              </article>
            ))}
          </section>
          <section className="preview-agent">
            <div className="preview-agent__heading">
              <h3>{t("agents.effects")}</h3>
            </div>
            <div className="preview-files">
              {preview.files.map((effect) => (
                <FileEffect
                  key={`${effect.path}-${effect.role}`}
                  effect={effect}
                />
              ))}
              {preview.state_change && (
                <FileEffect effect={preview.state_change} />
              )}
              {preview.state_backup && (
                <FileEffect effect={preview.state_backup} />
              )}
            </div>
          </section>
        </div>
        <aside className="approval-rail">
          <p className="overline">{t("agents.approvalBoundary")}</p>
          <h3>{t("agents.reviewScope")}</h3>
          {preview.managed_collisions.map((collision) => (
            <p
              className="collision"
              key={`${collision.agent}-${collision.path}`}
            >
              {safe(collision.agent)} <code>{safe(collision.path)}</code>{" "}
              {safe(collision.action)}
            </p>
          ))}
          {preview.drifted_agents.length > 0 && (
            <label className="approval-check">
              <input
                type="checkbox"
                checked={approveDrift}
                onChange={(event) => setApproveDrift(event.target.checked)}
              />
              {t("agents.approveDrift")}
            </label>
          )}
          {preview.requires_codex_auth_approval && (
            <label className="approval-check">
              <input
                type="checkbox"
                checked={approveAuth}
                onChange={(event) => setApproveAuth(event.target.checked)}
              />
              {t("agents.approveCodexAuth")}
            </label>
          )}
          <button
            ref={rebuildTriggerRef}
            className="control-button"
            disabled={
              busy ||
              (preview.drifted_agents.length > 0 && !approveDrift) ||
              (preview.requires_codex_auth_approval && !approveAuth) ||
              !previewValid
            }
            onClick={() => {
              if (!previewValid) return;
              if (target.mode === "rebuild") setRebuildDialogOpen(true);
              else onWrite(approvals(false));
            }}
          >
            {t("agents.write")}
          </button>
          <button className="text-button" onClick={onBackToEdit}>
            {t("agents.backToConfigure")}
          </button>
          <button className="text-button" onClick={onCancel}>
            {t("agents.cancelDetection")}
          </button>
        </aside>
      </div>
      {rebuildDialogOpen && previewValid && (
        <div
          className="dialog-backdrop"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget && !busy)
              closeRebuildDialog();
          }}
        >
          <section
            className="danger-dialog rebuild-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="rebuild-dialog-title"
            aria-describedby="rebuild-dialog-warning"
            onKeyDown={(event) => {
              if (event.key !== "Tab") return;
              if (
                event.shiftKey &&
                document.activeElement === cancelRebuildRef.current
              ) {
                event.preventDefault();
                confirmRebuildRef.current?.focus();
              } else if (
                !event.shiftKey &&
                document.activeElement === confirmRebuildRef.current
              ) {
                event.preventDefault();
                cancelRebuildRef.current?.focus();
              }
            }}
          >
            <p className="overline">{t("agents.rebuildConfirm.overline")}</p>
            <h2 id="rebuild-dialog-title">
              {t("agents.rebuildConfirm.title")}
            </h2>
            <p>{t("agents.rebuildConfirm.description")}</p>
            <ul className="rebuild-dialog__agents">
              <li>{agentNames[target.agent]}</li>
            </ul>
            <p id="rebuild-dialog-warning" className="danger-dialog__warning">
              {t("agents.recovery.warning")}
            </p>
            <div className="danger-dialog__actions">
              <button
                ref={cancelRebuildRef}
                type="button"
                className="text-button"
                disabled={busy}
                onClick={closeRebuildDialog}
              >
                {t("agents.rebuildConfirm.cancel")}
              </button>
              <button
                ref={confirmRebuildRef}
                type="button"
                className="control-button control-button--danger"
                disabled={busy}
                onClick={() => {
                  setRebuildDialogOpen(false);
                  onWrite(approvals(true));
                }}
              >
                {t("agents.rebuildConfirm.confirm")}
              </button>
            </div>
          </section>
        </div>
      )}
    </>
  );
}
