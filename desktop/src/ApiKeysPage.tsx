import { useEffect, useRef, useState } from "react";

import { useI18n } from "./i18n";
import type { CredentialSummary, DesktopApi, DesktopPaths } from "./ipc";
import type { TranslationKey } from "./locales/zh-CN";

// Keep in sync with the authoritative Rust validation in credential.rs.
const MAX_KEY_BYTES = 16 * 1024;

function errorTranslation(error: unknown): TranslationKey {
  const code =
    error && typeof error === "object" && "code" in error
      ? String(error.code)
      : "";
  if (code === "CREDENTIAL_INVALID") return "apikey.error.invalid";
  if (code === "CREDENTIAL_IO_ERROR") return "apikey.error.io";
  if (code === "CREDENTIAL_LOCK_TIMEOUT") return "apikey.error.lock";
  return "apikey.error.load";
}

export function ApiKeysPage({ api }: { api: DesktopApi }) {
  const { language, t } = useI18n();
  const inputRef = useRef<HTMLInputElement>(null);
  const [summary, setSummary] = useState<CredentialSummary | null>(null);
  const [paths, setPaths] = useState<DesktopPaths | null>(null);
  const [show, setShow] = useState(false);
  const [operation, setOperation] = useState<"" | "save" | "delete">("");
  const [error, setError] = useState<TranslationKey | "">("");

  function clearInput() {
    if (inputRef.current) inputRef.current.value = "";
    setShow(false);
  }

  useEffect(() => {
    let current = true;
    void Promise.allSettled([api.getCredential(), api.getDesktopPaths()]).then(
      ([summaryResult, pathsResult]) => {
        if (!current) return;
        if (summaryResult.status === "fulfilled") {
          setSummary(summaryResult.value);
        } else {
          setSummary({ present: false, fingerprint: "", saved_at: null });
          setError(errorTranslation(summaryResult.reason));
        }
        if (pathsResult.status === "fulfilled") {
          setPaths(pathsResult.value);
        } else {
          setError("apikey.error.load");
        }
      },
    );
    return () => {
      current = false;
    };
  }, [api]);

  async function save(event: React.FormEvent) {
    event.preventDefault();
    if (operation || !inputRef.current) return;
    const key = inputRef.current.value.trim();
    if (!key || new TextEncoder().encode(key).length > MAX_KEY_BYTES) {
      clearInput();
      setError("apikey.error.length");
      return;
    }
    setOperation("save");
    setError("");
    try {
      setSummary(await api.saveCredential(key));
    } catch (saveError) {
      setError(errorTranslation(saveError));
    } finally {
      clearInput();
      setOperation("");
    }
  }

  async function remove() {
    if (operation) return;
    setOperation("delete");
    setError("");
    try {
      setSummary(await api.deleteCredential());
      clearInput();
    } catch (deleteError) {
      setError(errorTranslation(deleteError));
    } finally {
      setOperation("");
    }
  }

  const savedAt = summary?.saved_at
    ? new Intl.DateTimeFormat(language, {
        dateStyle: "medium",
        timeStyle: "short",
      }).format(new Date(summary.saved_at))
    : "";

  return (
    <section className="apikey-panel" aria-labelledby="apikey-heading">
      <header className="apikey-heading">
        <p className="overline">{t("apikey.overline")}</p>
        <h2 id="apikey-heading">{t("apikey.heading")}</h2>
      </header>

      {error && (
        <p className="apikey-alert" role="alert">
          {t(error)}
        </p>
      )}

      <section
        className="apikey-card"
        data-state={
          summary ? (summary.present ? "saved" : "absent") : "loading"
        }
      >
        <header className="apikey-card__status" aria-live="polite">
          <span className="apikey-card__indicator" aria-hidden="true" />
          <div className="apikey-card__status-text">
            <strong>
              {summary
                ? t(
                    summary.present
                      ? "apikey.status.saved"
                      : "apikey.status.absent",
                  )
                : t("apikey.status.loading")}
            </strong>
            <span className="apikey-card__status-note">
              {t("apikey.status.note")}
            </span>
          </div>
          {summary?.present && (
            <dl className="apikey-card__meta">
              <div>
                <dt>{t("apikey.fingerprint")}</dt>
                <dd>...{summary.fingerprint}</dd>
              </div>
              <div>
                <dt>{t("apikey.savedAt")}</dt>
                <dd>{savedAt}</dd>
              </div>
            </dl>
          )}
        </header>

        <form className="apikey-card__form" onSubmit={save}>
          <label htmlFor="apikey-input">{t("apikey.label")}</label>
          <div className="apikey-card__input-row">
            <input
              ref={inputRef}
              id="apikey-input"
              type={show ? "text" : "password"}
              autoComplete="off"
              spellCheck={false}
              placeholder={t("apikey.input.placeholder")}
              disabled={Boolean(operation)}
            />
            <button
              type="button"
              className="text-button"
              aria-pressed={show}
              onClick={() => setShow((current) => !current)}
              disabled={Boolean(operation)}
            >
              {t(show ? "apikey.hide" : "apikey.show")}
            </button>
          </div>
          <div className="apikey-card__actions">
            <button className="control-button" disabled={Boolean(operation)}>
              {operation === "save"
                ? t("apikey.saving")
                : t(summary?.present ? "apikey.replace" : "apikey.save")}
            </button>
            {summary?.present && (
              <button
                type="button"
                className="text-button apikey-delete"
                onClick={() => void remove()}
                disabled={Boolean(operation)}
              >
                {operation === "delete"
                  ? t("apikey.deleting")
                  : t("apikey.delete")}
              </button>
            )}
          </div>
        </form>

        <dl className="apikey-card__location">
          <dt>{t("apikey.storage.label")}</dt>
          <dd>
            <code>{paths?.credentials_path ?? t("apikey.path.loading")}</code>
          </dd>
        </dl>

        <aside className="apikey-card__tips">
          <strong>{t("apikey.explainer.usage.heading")}</strong>
          <ul>
            <li>{t("apikey.explainer.usage.agentFiles")}</li>
            <li>{t("apikey.explainer.usage.catalog")}</li>
          </ul>
          <p className="apikey-card__tips-note">
            {t("apikey.explainer.storage.note")}
          </p>
        </aside>
      </section>
    </section>
  );
}
