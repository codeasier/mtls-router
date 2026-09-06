import { useEffect, useState } from "react";

import { useI18n, type Translator } from "./i18n";
import { MAX_LOG_LINES, type DesktopApi } from "./ipc";

function safeLogError(action: "load" | "open" | "copy", t: Translator): string {
  return {
    load: t("logs.error.load"),
    open: t("logs.error.open"),
    copy: t("logs.error.copy"),
  }[action];
}

export function LogsPage({ api }: { api: DesktopApi }) {
  const { t } = useI18n();
  const [lines, setLines] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [message, setMessage] = useState("");

  async function load() {
    setLoading(true);
    setMessage("");
    try {
      const result = await api.getRouterLogs(MAX_LOG_LINES);
      setLines(result.lines);
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
            onClick={load}
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

      <div
        className="log-screen log-screen--scroll"
        role="log"
        aria-live="polite"
        aria-busy={loading}
      >
        {loading && lines.length === 0 ? (
          <p className="log-empty">{t("logs.loading")}</p>
        ) : lines.length === 0 ? (
          <p className="log-empty">{t("logs.empty")}</p>
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
