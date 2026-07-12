import { useEffect, useState } from "react";

import { useI18n, type Translator } from "./i18n";
import {
  sanitizeSensitiveText,
  type AgentDetection,
  type AgentFilePreview,
  type AgentId,
  type AgentPreview,
  type AgentState,
  type AgentWriteResult,
  type DesktopApi,
} from "./ipc";

type Stage = "select" | "preview" | "key" | "result";

const agentOrder: AgentId[] = ["claude", "opencode", "codex"];
const agentNames: Record<AgentId, string> = {
  claude: "Claude Code",
  opencode: "opencode",
  codex: "Codex",
};

function operationLabel(
  operation: AgentFilePreview["operation"],
  t: Translator,
) {
  return t(`agents.operation.${operation}`);
}

function safe(value: string | undefined): string {
  return sanitizeSensitiveText(value ?? "");
}

function errorCode(error: unknown): string {
  if (typeof error === "object" && error !== null && "code" in error) {
    const code = (error as { code?: unknown }).code;
    return typeof code === "string" ? code : "";
  }
  return "";
}

function detectionLabel(agent: AgentState, t: Translator): string {
  if (!agent.detected) return t("agents.detection.absent");
  if (agent.invalid) return t("agents.detection.invalid");
  if (!agent.writable) return t("agents.detection.readonly");
  if (agent.configured) return t("agents.detection.configured");
  return agent.exists
    ? t("agents.detection.ready")
    : t("agents.detection.create");
}

function detectionTone(agent: AgentState): string {
  if (!agent.detected || !agent.writable) return "idle";
  if (agent.invalid) return "danger";
  return agent.configured ? "active" : "ready";
}

function canSelect(agent: AgentState): boolean {
  return agent.detected && !agent.invalid && agent.writable;
}

function initialSelection(detection: AgentDetection): AgentId[] {
  return detection.agents.filter(canSelect).map((agent) => agent.agent);
}

function actionMessage(agent: AgentState, t: Translator): string {
  if (!agent.detected) return t("agents.guidance.absent");
  if (agent.invalid) {
    return t("agents.guidance.invalid", {
      path: safe(agent.path),
      format: safe(agent.format).toUpperCase(),
    });
  }
  if (!agent.writable) return t("agents.guidance.readonly");
  return agent.configured
    ? t("agents.guidance.configured")
    : t("agents.guidance.ready");
}

function PreviewFile({ file }: { file: AgentFilePreview }) {
  const { t } = useI18n();
  const migration = Boolean(file.source_path && file.source_path !== file.path);
  return (
    <article className="agent-file">
      <div className="agent-file__head">
        <span className={`operation operation--${file.operation}`}>
          {operationLabel(file.operation, t)}
        </span>
        <strong>{safe(file.format).toUpperCase()}</strong>
      </div>
      <code title={safe(file.path)}>{safe(file.path)}</code>
      {migration && (
        <div className="migration-warning" role="note">
          <strong>{t("agents.migration")}</strong>
          <span>
            {t("agents.sourceFile", { path: safe(file.source_path) })}
          </span>
          <p>{t("agents.migrationWarning")}</p>
        </div>
      )}
      <div className="operation-list" aria-label={t("agents.fileOperations")}>
        {file.operations.map((operation) => (
          <span key={operation}>{operationLabel(operation, t)}</span>
        ))}
      </div>
      {file.preserves && file.preserves.length > 0 && (
        <p className="preserve-copy">
          {t("agents.preserve", {
            items: file.preserves.map((item) => safe(item)).join(", "),
          })}
        </p>
      )}
      {file.contains_api_key && (
        <p className="sensitive-copy">{t("agents.keyFile")}</p>
      )}
      {file.backup.required && (
        <div className="backup-plan">
          <span>{t("agents.sensitiveBackup")}</span>
          <code>{safe(file.backup.pattern)}</code>
          <p>{t("agents.backupWarning")}</p>
        </div>
      )}
      {file.warning && !migration && (
        <p className="file-warning">{safe(file.warning)}</p>
      )}
    </article>
  );
}

export function AgentPage({ api }: { api: DesktopApi }) {
  const { t } = useI18n();
  const [detection, setDetection] = useState<AgentDetection | null>(null);
  const [selected, setSelected] = useState<AgentId[]>([]);
  const [preview, setPreview] = useState<AgentPreview | null>(null);
  const [result, setResult] = useState<AgentWriteResult | null>(null);
  const [stage, setStage] = useState<Stage>("select");
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(true);
  const [message, setMessage] = useState("");

  function clearKey() {
    setApiKey("");
  }

  async function refreshDetection() {
    clearKey();
    setBusy(true);
    setMessage("");
    setPreview(null);
    setResult(null);
    setStage("select");
    try {
      const next = await api.detectAgents();
      setDetection(next);
      setSelected(initialSelection(next));
    } catch {
      setDetection(null);
      setSelected([]);
      setMessage(t("agents.error.detect"));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    let current = true;
    void api
      .detectAgents()
      .then((next) => {
        if (!current) return;
        setDetection(next);
        setSelected(initialSelection(next));
      })
      .catch(() => {
        if (!current) return;
        setMessage(t("agents.error.detect"));
      })
      .finally(() => {
        if (current) setBusy(false);
      });
    return () => {
      current = false;
    };
  }, [api, t]);

  function toggle(agent: AgentId) {
    setSelected((current) =>
      current.includes(agent)
        ? current.filter((item) => item !== agent)
        : agentOrder.filter((item) => [...current, agent].includes(item)),
    );
    setMessage("");
  }

  async function loadPreview(stale = false) {
    if (selected.length === 0) {
      setMessage(t("agents.error.selection"));
      return;
    }
    clearKey();
    setBusy(true);
    setMessage("");
    try {
      const next = await api.previewAgents(selected);
      setPreview(next);
      setResult(null);
      setStage("preview");
      if (stale) setMessage(t("agents.previewRefreshed"));
    } catch (error) {
      const code = errorCode(error);
      setMessage(
        code === "CONFIG_INVALID"
          ? t("agents.error.invalid")
          : t("agents.error.preview"),
      );
      setStage("select");
    } finally {
      setBusy(false);
    }
  }

  function cancelToSelection() {
    clearKey();
    setPreview(null);
    setResult(null);
    setMessage("");
    setStage("select");
  }

  function cancelKeyEntry() {
    clearKey();
    setMessage("");
    setStage("preview");
  }

  async function write() {
    if (!preview || apiKey.length === 0) return;
    const transientKey = apiKey;
    clearKey();
    setBusy(true);
    setMessage("");
    try {
      const next = await api.writeAgents(
        selected,
        preview.revision_token,
        transientKey,
      );
      setResult(next);
      setStage("result");
    } catch (error) {
      if (errorCode(error) === "PREVIEW_STALE") {
        await loadPreview(true);
      } else {
        setMessage(t("agents.error.write"));
        setStage("preview");
      }
    } finally {
      setBusy(false);
    }
  }

  const detectionByAgent = new Map(
    detection?.agents.map((agent) => [agent.agent, agent]) ?? [],
  );
  const stageLabel = t(`agents.stage.${stage}`);

  return (
    <section className="agents-workbench" aria-labelledby="agents-heading">
      <header className="agents-workbench__header">
        <div>
          <p className="overline">{t("agents.overline")}</p>
          <h2 id="agents-heading">{t("agents.heading")}</h2>
        </div>
        <div
          className="stage-meter"
          aria-label={t("agents.currentStage", { stage: stageLabel })}
        >
          {(["select", "preview", "key", "result"] as Stage[]).map(
            (item, index) => (
              <span key={item} className={item === stage ? "is-current" : ""}>
                {String(index + 1).padStart(2, "0")}
              </span>
            ),
          )}
        </div>
      </header>

      {message && (
        <p className="agent-alert" role="alert">
          {message}
        </p>
      )}

      {stage === "select" && (
        <>
          <div className="agent-toolbar">
            <p>{t("agents.selectionNote")}</p>
            <button
              type="button"
              className="text-button"
              onClick={() => void refreshDetection()}
              disabled={busy}
            >
              {busy ? t("agents.detecting") : t("agents.refresh")}
            </button>
          </div>
          <div className="agent-card-grid" aria-busy={busy}>
            {agentOrder.map((id, index) => {
              const agent = detectionByAgent.get(id);
              if (!agent) {
                return (
                  <article className="agent-card agent-card--loading" key={id}>
                    <span>{String(index + 1).padStart(2, "0")}</span>
                    <h3>{agentNames[id]}</h3>
                    <p>{busy ? t("agents.loading") : t("agents.noResult")}</p>
                  </article>
                );
              }
              const selectable = canSelect(agent);
              const checked = selected.includes(id);
              return (
                <article
                  className={`agent-card agent-card--${detectionTone(agent)}${checked ? " is-selected" : ""}`}
                  key={id}
                >
                  <div className="agent-card__topline">
                    <span>{String(index + 1).padStart(2, "0")}</span>
                    <span className="agent-state">
                      {detectionLabel(agent, t)}
                    </span>
                  </div>
                  <h3>{safe(agent.name) || agentNames[id]}</h3>
                  <dl>
                    <div className="agent-card__path">
                      <dt>{t("agents.mainConfig")}</dt>
                      <dd title={safe(agent.path)}>
                        {safe(agent.path) || t("agents.notLocated")}
                      </dd>
                    </div>
                    {agent.auth_path && (
                      <div className="agent-card__path">
                        <dt>{t("agents.authFile")}</dt>
                        <dd title={safe(agent.auth_path)}>
                          {safe(agent.auth_path)}
                        </dd>
                      </div>
                    )}
                    <div>
                      <dt>{t("agents.format")}</dt>
                      <dd>
                        {safe(agent.format).toUpperCase() ||
                          t("agents.notApplicable")}
                      </dd>
                    </div>
                    <div>
                      <dt>{t("agents.file")}</dt>
                      <dd>
                        {agent.exists
                          ? t("agents.exists")
                          : t("agents.pendingCreate")}
                      </dd>
                    </div>
                    <div>
                      <dt>{t("agents.permission")}</dt>
                      <dd>
                        {agent.writable
                          ? t("agents.writable")
                          : t("agents.detection.readonly")}
                      </dd>
                    </div>
                    <div>
                      <dt>{t("agents.routerConfig")}</dt>
                      <dd>
                        {agent.configured
                          ? t("agents.detection.configured")
                          : t("agents.notConfigured")}
                      </dd>
                    </div>
                  </dl>
                  <p className="agent-card__guidance">
                    {actionMessage(agent, t)}
                  </p>
                  <label className="agent-select">
                    <input
                      type="checkbox"
                      checked={checked}
                      disabled={!selectable}
                      onChange={() => toggle(id)}
                    />
                    <span>
                      {checked ? t("agents.selected") : t("agents.select")}
                    </span>
                  </label>
                </article>
              );
            })}
          </div>
          <div className="agent-footer-action">
            <span>{t("agents.selectedCount", { count: selected.length })}</span>
            <button
              type="button"
              className="control-button"
              disabled={busy || selected.length === 0}
              onClick={() => void loadPreview()}
            >
              {t("agents.generatePreview")}
            </button>
          </div>
        </>
      )}

      {(stage === "preview" || stage === "key") && preview && (
        <div className="preview-layout">
          <div className="preview-main">
            {preview.agents.map((agent) => (
              <section className="preview-agent" key={agent.agent}>
                <div className="preview-agent__heading">
                  <span>{safe(agent.agent).toUpperCase()}</span>
                  <h3>{safe(agent.name)}</h3>
                  <small>
                    {t("agents.fileCount", { count: agent.files.length })}
                  </small>
                </div>
                <div className="preview-files">
                  {agent.files.map((file) => (
                    <PreviewFile
                      key={`${agent.agent}-${file.path}`}
                      file={file}
                    />
                  ))}
                </div>
              </section>
            ))}
          </div>
          <aside className="approval-rail">
            <p className="overline">{t("agents.approvalBoundary")}</p>
            <h3>
              {stage === "preview"
                ? t("agents.reviewScope")
                : t("agents.oneTimeCredential")}
            </h3>
            {stage === "preview" ? (
              <>
                <p>{t("agents.reviewNote")}</p>
                {preview.warnings && preview.warnings.length > 0 && (
                  <div className="preview-warnings" role="note">
                    {preview.warnings.map((warning) => (
                      <p key={warning}>{safe(warning)}</p>
                    ))}
                  </div>
                )}
                <div className="risk-box">
                  <strong>{t("agents.reviewSensitiveBackup")}</strong>
                  <p>{t("agents.reviewBackup")}</p>
                </div>
                <button
                  type="button"
                  className="control-button"
                  onClick={() => {
                    clearKey();
                    setMessage("");
                    setStage("key");
                  }}
                >
                  {t("agents.approve")}
                </button>
                <button
                  type="button"
                  className="text-button"
                  onClick={cancelToSelection}
                >
                  {t("agents.cancelDetection")}
                </button>
              </>
            ) : (
              <form
                onSubmit={(event) => {
                  event.preventDefault();
                  void write();
                }}
              >
                <p>{t("agents.keyNote")}</p>
                <label className="key-field">
                  <span>{t("agents.apiKey")}</span>
                  <input
                    type="password"
                    name="agent-api-key"
                    autoComplete="off"
                    spellCheck={false}
                    value={apiKey}
                    disabled={busy}
                    onChange={(event) => setApiKey(event.target.value)}
                  />
                </label>
                <button
                  type="submit"
                  className="control-button"
                  disabled={busy || apiKey.length === 0}
                >
                  {busy ? t("agents.executing") : t("agents.write")}
                </button>
                <button
                  type="button"
                  className="text-button"
                  disabled={busy}
                  onClick={cancelKeyEntry}
                >
                  {t("agents.cancelKey")}
                </button>
              </form>
            )}
          </aside>
        </div>
      )}

      {stage === "result" && result && (
        <div className="agent-results">
          <div className="result-banner">
            <span>{t("agents.transactionComplete")}</span>
            <h3>{t("agents.resultHeading")}</h3>
            <p>{t("agents.resultNote")}</p>
          </div>
          <div className="result-grid">
            {result.agents.map((agent) => (
              <article key={agent.agent}>
                <div className="result-card__heading">
                  <h4>{agentNames[agent.agent] ?? safe(agent.agent)}</h4>
                  <span className={agent.success ? "result-ok" : "result-fail"}>
                    {agent.success ? t("agents.success") : t("agents.failure")}
                  </span>
                </div>
                {agent.rolled_back && (
                  <p className="rollback-note">{t("agents.rolledBack")}</p>
                )}
                {agent.error_code && (
                  <p>
                    {t("agents.errorCode", { code: safe(agent.error_code) })}
                  </p>
                )}
                <dl>
                  <dt>{t("agents.changed")}</dt>
                  <dd>
                    {(agent.changed ?? []).length > 0
                      ? agent.changed?.map((path) => (
                          <code key={path}>{safe(path)}</code>
                        ))
                      : t("agents.none")}
                  </dd>
                  <dt>{t("agents.sensitiveBackup")}</dt>
                  <dd>
                    {(agent.backups ?? []).length > 0
                      ? agent.backups?.map((path) => (
                          <code key={path}>{safe(path)}</code>
                        ))
                      : t("agents.none")}
                  </dd>
                  {(agent.rollback_backups ?? []).length > 0 && (
                    <>
                      <dt>{t("agents.rollbackBackup")}</dt>
                      <dd>
                        {agent.rollback_backups?.map((path) => (
                          <code key={path}>{safe(path)}</code>
                        ))}
                      </dd>
                    </>
                  )}
                </dl>
              </article>
            ))}
          </div>
          <button
            type="button"
            className="control-button"
            onClick={() => void refreshDetection()}
          >
            {t("agents.finish")}
          </button>
        </div>
      )}
    </section>
  );
}
