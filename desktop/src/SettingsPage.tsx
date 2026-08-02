import { useEffect, useRef, useState } from "react";

import { useI18n } from "./i18n";
import type {
  ComponentVersions,
  DesktopApi,
  DesktopPaths,
  UpdateCheckResult,
  UpdateProgress,
} from "./ipc";

interface SettingsPageProps {
  api: DesktopApi;
  updateResult: UpdateCheckResult | null;
  checkingForUpdate: boolean;
  updateCheckError: boolean;
  onCheckForUpdate(): Promise<void>;
}

export function SettingsPage({
  api,
  updateResult,
  checkingForUpdate,
  updateCheckError,
  onCheckForUpdate,
}: SettingsPageProps) {
  const { language, setLanguage, t } = useI18n();
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [versions, setVersions] = useState<ComponentVersions | null>(null);
  const [paths, setPaths] = useState<DesktopPaths | null>(null);
  const [savingAutostart, setSavingAutostart] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [installState, setInstallState] = useState<
    "idle" | "downloading" | "restarting" | "error"
  >("idle");
  const [updateProgress, setUpdateProgress] = useState<UpdateProgress | null>(
    null,
  );
  const stopUpdateProgressRef = useRef<(() => void) | null>(null);
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

  useEffect(
    () => () => {
      stopUpdateProgressRef.current?.();
    },
    [],
  );

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

  async function installUpdate() {
    const update = updateResult?.update;
    if (
      !updateResult?.available ||
      !update ||
      installState === "downloading" ||
      installState === "restarting"
    ) {
      return;
    }
    if (
      !window.confirm(
        t("update.installConfirm", {
          version: update.version,
        }),
      )
    ) {
      return;
    }

    setInstallState("downloading");
    setUpdateProgress({ downloaded: 0 });
    try {
      const stop = await api.subscribeUpdateProgress((progress) => {
        setUpdateProgress(progress);
      });
      stopUpdateProgressRef.current = stop;
      await api.installUpdate(update.version);
      setInstallState("restarting");
    } catch {
      setInstallState("error");
    } finally {
      stopUpdateProgressRef.current?.();
      stopUpdateProgressRef.current = null;
    }
  }

  const versionRows = [
    ["A", t("router.desktop"), versions?.desktop],
    ["B", t("router.manager"), versions?.manager],
    ["C", t("router.router"), versions?.router],
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
          <div className="settings-block__heading">
            <h3>{t("settings.components")}</h3>
            <button
              type="button"
              className="text-button"
              onClick={() => void onCheckForUpdate()}
              disabled={
                checkingForUpdate ||
                installState === "downloading" ||
                installState === "restarting"
              }
            >
              {checkingForUpdate
                ? t("update.checking")
                : t("update.checkAction")}
            </button>
          </div>
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

          {updateResult?.available && updateResult.update && (
            <div className="settings-block__update">
              <header className="settings-block__update-head">
                <strong>{t("update.available")}</strong>
                <span className="settings-block__update-version">
                  {updateResult.update.version}
                </span>
              </header>

              <div className="settings-block__update-versions">
                <div>
                  <span>{t("update.currentVersion")}</span>
                  <code>
                    {updateResult?.current_version ??
                      versions?.desktop ??
                      t("settings.unavailable")}
                  </code>
                </div>
                <div>
                  <span>{t("update.latestVersion")}</span>
                  <code>{updateResult.update.version}</code>
                </div>
              </div>

              {updateResult.update.published_at && (
                <p className="settings-block__update-published">
                  {t("update.publishedAt", {
                    date: updateResult.update.published_at,
                  })}
                </p>
              )}

              {updateResult.update.notes && (
                <div className="settings-block__update-notes">
                  <strong>{t("update.releaseNotes")}</strong>
                  <p>{updateResult.update.notes}</p>
                </div>
              )}

              {installState === "downloading" && updateProgress && (
                <div className="update-progress" role="status">
                  <div
                    className={
                      updateProgress.total
                        ? "update-progress__track"
                        : "update-progress__track is-indeterminate"
                    }
                    role="progressbar"
                    aria-label={t("update.downloadProgress")}
                    aria-valuemin={0}
                    aria-valuenow={updateProgress.downloaded}
                    aria-valuemax={updateProgress.total}
                  >
                    <span
                      style={
                        updateProgress.total
                          ? {
                              width: `${Math.min(
                                (updateProgress.downloaded /
                                  updateProgress.total) *
                                  100,
                                100,
                              )}%`,
                            }
                          : undefined
                      }
                    />
                  </div>
                  <span>
                    {updateProgress.total
                      ? t("update.progressKnown", {
                          downloaded: updateProgress.downloaded,
                          total: updateProgress.total,
                        })
                      : t("update.progressUnknown", {
                          downloaded: updateProgress.downloaded,
                        })}
                  </span>
                </div>
              )}

              {installState === "error" && (
                <p className="settings-block__update-error" role="alert">
                  {t("update.error.install")}
                </p>
              )}
              {installState === "restarting" && (
                <p className="settings-block__update-state" role="status">
                  {t("update.restarting")}
                </p>
              )}

              {installState !== "restarting" && (
                <button
                  type="button"
                  className="control-button settings-block__update-action"
                  onClick={() => void installUpdate()}
                  disabled={installState === "downloading"}
                >
                  {installState === "downloading"
                    ? t("update.installing")
                    : t("update.installAction")}
                </button>
              )}
            </div>
          )}

          {updateCheckError && !updateResult?.available && (
            <p className="settings-block__update-error" role="alert">
              {t("update.error.check")}
            </p>
          )}

          <p
            className={
              updateResult?.available
                ? "settings-block__status settings-block__status--update"
                : "settings-block__status"
            }
            role="status"
          >
            {checkingForUpdate
              ? t("update.checking")
              : updateResult?.available
                ? t("update.available")
                : updateCheckError
                  ? t("update.error.check")
                  : updateResult
                    ? t("update.current")
                    : t("update.statusUnavailable")}
          </p>
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
      </div>

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

      <p className="settings-status" role="status">
        {message ? t(message) : ""}
      </p>
    </section>
  );
}
