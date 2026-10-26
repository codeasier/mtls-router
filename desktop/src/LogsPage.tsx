import { useEffect, useState } from "react";

import { useI18n, type Translator } from "./i18n";
import { MAX_LOG_LINES, type AppLogRecord, type DesktopApi } from "./ipc";

const APP_COMPONENTS = [
  "lifecycle",
  "updater",
  "router",
  "command",
  "settings",
  "credential",
  "identity",
  "agent",
  "workbench",
] as const;

const APP_LEVELS = ["error", "warn", "info", "debug"] as const;

function formatAppRecord(record: AppLogRecord): string {
  const detail = record.detail
    ? Object.entries(record.detail)
        .map(([key, value]) => `${key}=${value}`)
        .join(" ")
    : "";
  return [
    record.ts,
    record.level,
    record.component,
    record.event,
    record.code,
    record.stage,
    record.outcome,
    detail,
  ]
    .filter((part) => part && part.length > 0)
    .join(" ");
}

function safeLogError(action: "load" | "open" | "copy", t: Translator): string {
  return {
    load: t("logs.error.load"),
    open: t("logs.error.open"),
    copy: t("logs.error.copy"),
  }[action];
}

export function LogsPage({ api }: { api: DesktopApi }) {
  const { t } = useI18n();
  const [source, setSource] = useState<"router" | "app">("router");
  const [level, setLevel] = useState("all");
  const [component, setComponent] = useState("all");
  const [lines, setLines] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [message, setMessage] = useState("");

  async function load(
    nextSource = source,
    nextLevel = level,
    nextComponent = component,
  ) {
    setLoading(true);
    setMessage("");
    try {
      if (nextSource === "router") {
        const result = await api.getRouterLogs(MAX_LOG_LINES);
        setLines(result.lines);
      } else {
        const result = await api.getAppLogs(MAX_LOG_LINES);
        setLines(
          result.records
            .filter(
              (record) => nextLevel === "all" || record.level === nextLevel,
            )
            .filter(
              (record) =>
                nextComponent === "all" || record.component === nextComponent,
            )
            .map(formatAppRecord),
        );
      }
    } catch {
      setMessage(safeLogError("load", t));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    let current = true;
    void api
      .getRouterLogs(MAX_LOG_LINES)
      .then((result) => {
        if (current) setLines(result.lines);
      })
      .catch(() => {
        if (current) setMessage(safeLogError("load", t));
      })
      .finally(() => {
        if (current) setLoading(false);
      });
    return () => {
      current = false;
    };
  }, [api, t]);

  async function openLocation() {
    setMessage("");
    try {
      await api.openLogLocation();
      setMessage(t("logs.opened"));
    } catch {
      setMessage(safeLogError("open", t));
    }
  }

  async function copyDiagnostics() {
    setMessage("");
    try {
      const diagnostics = await api.collectDiagnostics();
      await navigator.clipboard.writeText(diagnostics.summary);
      setMessage(t("logs.copied"));
    } catch {
      try {
        const snapshot = await api.getDiagnosticSnapshot();
        await navigator.clipboard.writeText(snapshot.summary);
        setMessage(t("logs.copiedSnapshot"));
      } catch {
        setMessage(safeLogError("copy", t));
      }
    }
  }

  async function exportBundle() {
    setMessage("");
    try {
      await api.exportSupportBundle();
    } catch (error) {
      if (
        typeof error === "object" &&
        error !== null &&
        "code" in error &&
        error.code === "DIALOG_CANCELLED"
      ) {
        setMessage(t("router.exportCancelled"));
      } else {
        setMessage(t("router.exportFailed"));
      }
    }
  }

  return (
    <section className="logs-panel" aria-labelledby="logs-heading">
      <div className="logs-toolbar">
        <div>
          <p className="overline">{t("logs.overline")}</p>
          <h2 id="logs-heading">
            {t("logs.heading", { count: MAX_LOG_LINES })}
          </h2>
        </div>
        <div className="logs-actions">
          <button
            type="button"
            className="text-button"
            onClick={() => void load()}
            disabled={loading}
          >
            {t("logs.refresh")}
          </button>
          <button type="button" className="text-button" onClick={openLocation}>
            {t("logs.openLocation")}
          </button>
          <button
            type="button"
            className="control-button"
            onClick={copyDiagnostics}
          >
            {t("logs.copyDiagnostics")}
          </button>
          <button
            type="button"
            className="text-button"
            onClick={() => void exportBundle()}
          >
            {t("logs.exportBundle")}
          </button>
        </div>
      </div>

      <div className="logs-filters">
        <label>
          {t("logs.source")}
          <select
            value={source}
            onChange={(event) => {
              const next = event.target.value === "app" ? "app" : "router";
              setSource(next);
              void load(next, level, component);
            }}
          >
            <option value="router">{t("logs.source.router")}</option>
            <option value="app">{t("logs.source.app")}</option>
          </select>
        </label>
        <label>
          {t("logs.filter.level")}
          <select
            value={level}
            disabled={source !== "app"}
            onChange={(event) => {
              setLevel(event.target.value);
              void load(source, event.target.value, component);
            }}
          >
            <option value="all">{t("logs.filter.all")}</option>
            {APP_LEVELS.map((item) => (
              <option key={item} value={item}>
                {t(`logs.level.${item}`)}
              </option>
            ))}
          </select>
        </label>
        <label>
          {t("logs.filter.component")}
          <select
            value={component}
            disabled={source !== "app"}
            onChange={(event) => {
              setComponent(event.target.value);
              void load(source, level, event.target.value);
            }}
          >
            <option value="all">{t("logs.filter.all")}</option>
            {APP_COMPONENTS.map((item) => (
              <option key={item} value={item}>
                {t(`logs.component.${item}`)}
              </option>
            ))}
          </select>
        </label>
      </div>

      <div
        className="log-screen log-screen--scroll"
        role="log"
        aria-live="polite"
        aria-busy={loading}
      >
        {loading && lines.length === 0 ? (
          <p className="log-empty">{t("logs.loading")}</p>
        ) : lines.length === 0 ? (
          <p className="log-empty">
            {t(source === "app" ? "logs.emptyApp" : "logs.empty")}
          </p>
        ) : (
          <ol>
            {lines.map((line, index) => (
              <li key={`${index}-${line}`}>
                <span>{String(index + 1).padStart(3, "0")}</span>
                <code>{line}</code>
              </li>
            ))}
          </ol>
        )}
      </div>

      <footer className="logs-foot">
        <span>{t("logs.boundary")}</span>
        <span role="status">{message}</span>
      </footer>
    </section>
  );
}
