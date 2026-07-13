import { useEffect, useState } from "react";

import { useI18n } from "./i18n";
import type { ComponentVersions, DesktopApi, DesktopPaths } from "./ipc";

export function SettingsPage({ api }: { api: DesktopApi }) {
  const { language, setLanguage, t } = useI18n();
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [versions, setVersions] = useState<ComponentVersions | null>(null);
  const [paths, setPaths] = useState<DesktopPaths | null>(null);
  const [savingAutostart, setSavingAutostart] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [message, setMessage] = useState<
    | ""
    | "settings.error.load"
    | "settings.error.autostart"
    | "settings.error.uninstall"
    | "settings.autostartChanged"
  >("");

  useEffect(() => {
    let current = true;
    void Promise.allSettled([
      api.getAutostart(),
      api.getComponentVersions(),
      api.getDesktopPaths(),
    ]).then(([autostartResult, versionsResult, pathsResult]) => {
      if (!current) return;
      if (autostartResult.status === "fulfilled") {
        setAutostart(autostartResult.value);
      }
      if (versionsResult.status === "fulfilled") {
        setVersions(versionsResult.value);
      }
      if (pathsResult.status === "fulfilled") {
        setPaths(pathsResult.value);
      }
      if (
        autostartResult.status === "rejected" ||
        versionsResult.status === "rejected" ||
        pathsResult.status === "rejected"
      ) {
        setMessage("settings.error.load");
      }
    });
    return () => {
      current = false;
    };
  }, [api]);

  async function changeAutostart() {
    if (autostart === null || savingAutostart) return;
    setSavingAutostart(true);
    setMessage("");
    try {
      setAutostart(await api.setAutostart(!autostart));
      setMessage("settings.autostartChanged");
    } catch {
      setMessage("settings.error.autostart");
    } finally {
      setSavingAutostart(false);
    }
  }

  async function prepareForUninstall() {
    if (!window.confirm(t("settings.prepareConfirm"))) return;
    setPreparing(true);
    setMessage("");
    try {
      await api.prepareForUninstall();
    } catch {
      setPreparing(false);
      setMessage("settings.error.uninstall");
    }
  }

  const versionRows = [
    ["A", t("router.desktop"), versions?.desktop],
    ["B", t("router.manager"), versions?.manager],
  ];

  return (
    <section className="settings-panel" aria-labelledby="settings-heading">
      <div className="settings-heading">
        <p className="overline">{t("settings.overline")}</p>
        <h2 id="settings-heading">{t("settings.heading")}</h2>
      </div>

      <div className="settings-grid">
        <section className="settings-block">
          <h3>{t("settings.general")}</h3>
          <div className="setting-row">
            <div>
              <strong>{t("settings.autostart")}</strong>
              <p>{t("settings.autostartDescription")}</p>
            </div>
            <button
              type="button"
              className={autostart ? "toggle is-on" : "toggle"}
              role="switch"
              aria-label={t("settings.autostart")}
              aria-checked={autostart ?? false}
              onClick={changeAutostart}
              disabled={autostart === null || savingAutostart}
            >
              <span aria-hidden="true" />
              {autostart ? t("settings.on") : t("settings.off")}
            </button>
          </div>
          <label className="setting-row setting-row--language">
            <span>
              <strong>{t("settings.language")}</strong>
              <small>{t("settings.languageDescription")}</small>
            </span>
            <span className="language-select">
              <select
                value={language}
                onChange={(event) =>
                  setLanguage(event.target.value === "en" ? "en" : "zh-CN")
                }
              >
                <option value="zh-CN">{t("settings.chinese")}</option>
                <option value="en">{t("settings.english")}</option>
              </select>
            </span>
          </label>
        </section>

        <section className="settings-block settings-block--versions">
          <h3>{t("settings.components")}</h3>
          <ol
            className="settings-version-list"
            aria-label={t("settings.components")}
          >
            {versionRows.map(([index, label, version]) => (
              <li key={index}>
                <span>{index}</span>
                <strong>{label}</strong>
                <code>{version || t("settings.unavailable")}</code>
              </li>
            ))}
          </ol>
        </section>

        <section className="settings-block settings-block--locations">
          <h3>{t("settings.locations")}</h3>
          <dl>
            <div>
              <dt>{t("settings.dataLocation")}</dt>
              <dd>{paths?.data_dir ?? t("settings.unavailable")}</dd>
            </div>
            <div>
              <dt>{t("settings.logLocation")}</dt>
              <dd>{paths?.log_file ?? t("settings.unavailable")}</dd>
            </div>
          </dl>
        </section>

        {paths?.can_prepare_for_uninstall && (
          <section className="settings-block settings-block--danger">
            <div>
              <h3>{t("settings.prepareTitle")}</h3>
              <p>{t("settings.prepareDescription")}</p>
            </div>
            <button
              type="button"
              className="control-button control-button--stop"
              onClick={prepareForUninstall}
              disabled={preparing}
            >
              {t("settings.prepareAction")}
            </button>
          </section>
        )}
      </div>

      <p className="settings-status" role="status">
        {message ? t(message) : ""}
      </p>
    </section>
  );
}
