import { useEffect, useRef, useState } from "react";

import { ConfirmDialog } from "./ConfirmDialog";
import { ReleaseHistoryDialog } from "./ReleaseHistoryDialog";
import { useI18n } from "./i18n";
import {
  localizedReleaseNotes,
  publishedDay,
  releaseHistory,
} from "./releaseNotes";
import {
  sanitizeSensitiveText,
  type ComponentVersions,
  type DesktopApi,
  type DesktopPaths,
  type UpdateCheckResult,
  type UpdateProgress,
} from "./ipc";
import type { TranslationKey } from "./locales/zh-CN";
import { THEMES, useTheme, type ThemeId } from "./theme";

function commandErrorCode(error: unknown): string {
  if (typeof error !== "object" || error === null || !("code" in error)) {
    return "";
  }
  return typeof error.code === "string" ? error.code : "";
}

const themeKeys: Record<ThemeId, TranslationKey> = {
  warm: "settings.theme.warm",
  light: "settings.theme.light",
  dark: "settings.theme.dark",
};

function GitHubMark() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z" />
    </svg>
  );
}

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
  const { theme, setTheme } = useTheme();
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [versions, setVersions] = useState<ComponentVersions | null>(null);
  const [paths, setPaths] = useState<DesktopPaths | null>(null);
  const [savingAutostart, setSavingAutostart] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [installState, setInstallState] = useState<
    "idle" | "downloading" | "restarting" | "error"
  >("idle");
  const [installErrorCode, setInstallErrorCode] = useState("");
  const [updateProgress, setUpdateProgress] = useState<UpdateProgress | null>(
    null,
  );
  const stopUpdateProgressRef = useRef<(() => void) | null>(null);
  const confirmTriggerRef = useRef<HTMLElement | null>(null);
  const [confirmAction, setConfirmAction] = useState<
    null | "install-update" | "uninstall"
  >(null);
  const [message, setMessage] = useState<
    | ""
    | "settings.error.load"
    | "settings.error.autostart"
    | "settings.error.autostartInit"
    | "settings.error.uninstall"
    | "settings.autostartChanged"
    | "update.githubOpenFailed"
  >("");

  useEffect(() => {
    let current = true;
    void Promise.allSettled([
      api.getAutostart(),
      api.getComponentVersions(),
      api.getDesktopPaths(),
    ]).then(([autostartResult, versionsResult, pathsResult]) => {
      if (!current) return;
      const autostartFailed =
        autostartResult.status === "rejected" ||
        (autostartResult.status === "fulfilled" &&
          Boolean(autostartResult.value.diagnostic));
      if (autostartResult.status === "fulfilled") {
        setAutostart(autostartResult.value.enabled);
      } else {
        setAutostart(false);
      }
      if (versionsResult.status === "fulfilled") {
        setVersions(versionsResult.value);
      }
      if (pathsResult.status === "fulfilled") {
        setPaths(pathsResult.value);
      }
      if (autostartFailed) {
        setMessage("settings.error.autostartInit");
      } else if (
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

  function openConfirm(action: "install-update" | "uninstall") {
    confirmTriggerRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    setConfirmAction(action);
  }

  function cancelConfirm() {
    confirmTriggerRef.current?.focus();
    closeConfirm();
  }

  function closeConfirm() {
    confirmTriggerRef.current = null;
    setConfirmAction(null);
  }

  async function prepareForUninstall() {
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

    setInstallState("downloading");
    setInstallErrorCode("");
    setUpdateProgress({ downloaded: 0 });
    try {
      const stop = await api.subscribeUpdateProgress((progress) => {
        setUpdateProgress(progress);
      });
      stopUpdateProgressRef.current = stop;
      await api.installUpdate(update.version);
      setInstallState("restarting");
    } catch (error) {
      const code = sanitizeSensitiveText(commandErrorCode(error) || "UNKNOWN");
      setInstallErrorCode(code);
      setInstallState("error");
    } finally {
      stopUpdateProgressRef.current?.();
      stopUpdateProgressRef.current = null;
    }
  }

  const pendingNotes = updateResult?.update?.notes
    ? localizedReleaseNotes(updateResult.update.notes, language)
    : "";
  const pendingNotesText = updateResult?.update
    ? pendingNotes || t("update.releaseNotesMissing")
    : "";
  const history = releaseHistory(language, updateResult?.update);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [pickedVersion, setPickedVersion] = useState<string | null>(null);

  async function openGithubReleases() {
    setMessage("");
    try {
      await api.openGithubReleases();
    } catch {
      setMessage("update.githubOpenFailed");
    }
  }

  // One binary, one version. A second row appears only while a router this
  // application does not own is being reused, because that is the only case
  // where the running router can carry a different version.
  const versionRows: [string, string, string | undefined][] = [
    ["APP", t("settings.appVersion"), versions?.version],
  ];
  if (versions?.external_router) {
    versionRows.push([
      "EXT",
      t("settings.externalRouter"),
      versions.external_router.version,
    ]);
  }
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
          <div className="setting-row setting-row--theme">
            <span>
              <strong id="settings-theme-label">{t("settings.theme")}</strong>
              <small>{t("settings.themeDescription")}</small>
            </span>
            <div
              className="theme-picker"
              role="radiogroup"
              aria-labelledby="settings-theme-label"
              onKeyDown={(event) => {
                const home = event.key === "Home";
                const end = event.key === "End";
                const nextArrow =
                  event.key === "ArrowRight" || event.key === "ArrowDown";
                const prevArrow =
                  event.key === "ArrowLeft" || event.key === "ArrowUp";
                if (!home && !end && !nextArrow && !prevArrow) {
                  return;
                }
                event.preventDefault();
                const next = home
                  ? THEMES[0]
                  : end
                    ? THEMES[THEMES.length - 1]
                    : THEMES[
                        (THEMES.indexOf(theme) +
                          (nextArrow ? 1 : -1) +
                          THEMES.length) %
                          THEMES.length
                      ];
                setTheme(next);
                event.currentTarget
                  .querySelector<HTMLElement>(`[data-theme-preview="${next}"]`)
                  ?.focus();
              }}
            >
              {THEMES.map((id) => (
                <button
                  key={id}
                  type="button"
                  role="radio"
                  data-theme-preview={id}
                  className={
                    theme === id ? "theme-option is-active" : "theme-option"
                  }
                  aria-checked={theme === id}
                  tabIndex={theme === id ? 0 : -1}
                  onClick={() => setTheme(id)}
                >
                  <span className="theme-option__swatch" aria-hidden="true" />
                  {t(themeKeys[id])}
                </button>
              ))}
            </div>
          </div>
        </section>

        <section className="settings-block settings-block--versions">
          <div className="settings-block__heading">
            <h3>{t("settings.components")}</h3>
            <div className="settings-block__actions">
              <button
                type="button"
                className="text-button"
                aria-haspopup="dialog"
                onClick={() => setHistoryOpen(true)}
              >
                {t("update.historyAction")}
              </button>
              <button
                type="button"
                className="settings-github"
                aria-label={t("update.viewOnGitHub")}
                onClick={() => void openGithubReleases()}
              >
                <GitHubMark />
              </button>
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
                      versions?.version ??
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
                    date: publishedDay(updateResult.update.published_at),
                  })}
                </p>
              )}

              <div className="settings-block__update-notes">
                <strong id="update-release-notes">
                  {t("update.releaseNotes")}
                </strong>
                <p
                  aria-labelledby="update-release-notes"
                  className={pendingNotes ? undefined : "is-missing"}
                  tabIndex={0}
                >
                  {pendingNotesText}
                </p>
              </div>

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
                  {installErrorCode === "UPDATE_RESTART_BLOCKED"
                    ? t("update.error.restartBlocked")
                    : t("update.error.install", {
                        code: installErrorCode || "UNKNOWN",
                      })}
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
                  onClick={() => openConfirm("install-update")}
                  disabled={installState === "downloading"}
                >
                  {installState === "downloading"
                    ? t("update.installing")
                    : t("update.installAction")}
                </button>
              )}
            </div>
          )}

          {updateCheckError &&
          !checkingForUpdate &&
          !updateResult?.available ? (
            <p className="settings-block__update-error" role="alert">
              {t("update.error.check")}
            </p>
          ) : (
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
                  : updateResult
                    ? t("update.current")
                    : t("update.statusUnavailable")}
            </p>
          )}
        </section>

        <section className="settings-block settings-block--locations">
          <h3>{t("settings.locations")}</h3>
          <dl>
            <div>
              <dt>{t("settings.dataLocation")}</dt>
              <dd title={paths?.data_dir}>
                {paths?.data_dir ?? t("settings.unavailable")}
              </dd>
            </div>
            <div>
              <dt>{t("settings.logLocation")}</dt>
              <dd title={paths?.log_directory}>
                {paths?.log_directory ?? t("settings.unavailable")}
              </dd>
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
            onClick={() => openConfirm("uninstall")}
            disabled={preparing}
          >
            {t("settings.prepareAction")}
          </button>
        </section>
      )}

      <p className="settings-status" role="status">
        {message ? t(message) : ""}
      </p>

      {historyOpen && (
        <ReleaseHistoryDialog
          title={t("update.historyAction")}
          closeLabel={t("update.historyClose")}
          emptyLabel={t("update.historyEmpty")}
          entries={history}
          selectedVersion={pickedVersion}
          onSelect={setPickedVersion}
          onClose={() => setHistoryOpen(false)}
        />
      )}

      {confirmAction === "install-update" && (
        <ConfirmDialog
          title={t("update.installConfirmTitle")}
          description={t("update.installConfirm", {
            version: updateResult?.update?.version ?? "",
          })}
          details={pendingNotesText}
          confirmLabel={t("update.installConfirmAction")}
          cancelLabel={t("dialog.cancel")}
          onConfirm={() => {
            closeConfirm();
            void installUpdate();
          }}
          onCancel={cancelConfirm}
        />
      )}
      {confirmAction === "uninstall" && (
        <ConfirmDialog
          danger
          title={t("settings.prepareConfirmTitle")}
          description={t("settings.prepareConfirm")}
          confirmLabel={t("settings.prepareConfirmAction")}
          cancelLabel={t("dialog.cancel")}
          onConfirm={() => {
            closeConfirm();
            void prepareForUninstall();
          }}
          onCancel={cancelConfirm}
        />
      )}
    </section>
  );
}
