import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

import { AgentPage, type LeaveGuard } from "./AgentPage";
import { ApiKeysPage } from "./ApiKeysPage";
import { I18nProvider, useI18n } from "./i18n";
import { desktopApi, type DesktopApi, type UpdateCheckResult } from "./ipc";
import { LogsPage } from "./LogsPage";
import type { TranslationKey } from "./locales/zh-CN";
import { navigationItems, type SectionId } from "./model";
import { NavIcon } from "./NavIcon";
import { RouterPage } from "./RouterPage";
import { SettingsPage } from "./SettingsPage";
import { ThemeProvider } from "./theme";
import { UsagePage } from "./UsagePage";

const sectionKeys: Record<SectionId, string> = {
  router: "section.router",
  agents: "section.agents",
  "api-keys": "section.apiKeys",
  usage: "section.usage",
  logs: "section.logs",
  settings: "section.settings",
};

const navigationKeys: Record<SectionId, TranslationKey> = {
  router: "nav.router",
  agents: "nav.agents",
  "api-keys": "nav.apiKeys",
  usage: "nav.usage",
  logs: "nav.logs",
  settings: "nav.settings",
};

const shortNavigationKeys: Record<SectionId, TranslationKey> = {
  router: "nav.routerShort",
  agents: "nav.agentsShort",
  "api-keys": "nav.apiKeysShort",
  usage: "nav.usageShort",
  logs: "nav.logsShort",
  settings: "nav.settingsShort",
};

const SIDEBAR_COLLAPSED_STORAGE_KEY = "mtls-router.sidebar.collapsed";

function readSidebarCollapsed() {
  try {
    return window.localStorage.getItem(SIDEBAR_COLLAPSED_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function writeSidebarCollapsed(collapsed: boolean) {
  try {
    window.localStorage.setItem(
      SIDEBAR_COLLAPSED_STORAGE_KEY,
      collapsed ? "1" : "0",
    );
  } catch {
    // Storage can be unavailable (private mode, disabled storage). In that
    // case the sidebar still collapses for the current session only.
  }
}

function CollapseIcon({ collapsed }: { collapsed: boolean }) {
  return (
    <svg
      aria-hidden="true"
      className="sidebar-collapse__icon"
      viewBox="0 0 16 16"
      focusable="false"
    >
      {collapsed ? (
        <path d="M6 3.5 10.5 8 6 12.5" fill="none" strokeLinecap="round" />
      ) : (
        <path d="M10 3.5 5.5 8 10 12.5" fill="none" strokeLinecap="round" />
      )}
    </svg>
  );
}

function AppContent({ api }: { api: DesktopApi }) {
  const { t } = useI18n();
  const [activeSection, setActiveSection] = useState<SectionId>("router");
  const [sidebarCollapsed, setSidebarCollapsed] =
    useState(readSidebarCollapsed);
  const [agentExitGuarded, setAgentExitGuarded] = useState(false);
  const [blockedLeaveAttempt, setBlockedLeaveAttempt] = useState(0);
  const [updateResult, setUpdateResult] = useState<UpdateCheckResult | null>(
    null,
  );
  const [checkingForUpdate, setCheckingForUpdate] = useState(false);
  const [updateCheckError, setUpdateCheckError] = useState(false);
  const [pendingLeave, setPendingLeave] = useState<{
    kind: "navigation" | "native-quit";
    confirm(): void;
    cancel(): void;
    restoreFocus: HTMLElement | null;
  } | null>(null);
  const pendingLeaveRef = useRef<typeof pendingLeave>(null);
  const leaveGuardRef = useRef<LeaveGuard | null>(null);
  const leaveCancelRef = useRef<HTMLButtonElement>(null);
  const leaveConfirmRef = useRef<HTMLButtonElement>(null);
  const sectionHeadingRef = useRef<HTMLHeadingElement>(null);
  const focusSectionHeadingRef = useRef(false);
  const startupUpdateCheckStartedRef = useRef(false);
  const updateCheckInFlightRef = useRef(false);
  const sectionKey = sectionKeys[activeSection];

  const registerLeaveGuard = useCallback((guard: LeaveGuard | null) => {
    leaveGuardRef.current = guard;
  }, []);

  const checkForUpdate = useCallback(async () => {
    if (updateCheckInFlightRef.current) return;
    updateCheckInFlightRef.current = true;
    setCheckingForUpdate(true);
    setUpdateCheckError(false);
    try {
      setUpdateResult(await api.checkForUpdate());
    } catch {
      setUpdateCheckError(true);
    } finally {
      updateCheckInFlightRef.current = false;
      setCheckingForUpdate(false);
    }
  }, [api]);

  const requestLeave = useCallback(
    (
      action: () => void,
      cancel = () => undefined,
      restoreFocus?: HTMLElement,
      kind: "navigation" | "native-quit" = "navigation",
    ) => {
      if (pendingLeaveRef.current) {
        if (
          kind === "native-quit" &&
          pendingLeaveRef.current.kind !== "native-quit"
        ) {
          cancel();
        }
        return;
      }
      const decision = leaveGuardRef.current?.() ?? "allow";
      if (decision === "block") {
        setBlockedLeaveAttempt((attempt) => attempt + 1);
        cancel();
        return;
      }
      setBlockedLeaveAttempt(0);
      if (decision === "allow") {
        action();
        return;
      }
      const pending = {
        kind,
        confirm: action,
        cancel,
        restoreFocus:
          restoreFocus ??
          (document.activeElement instanceof HTMLElement
            ? document.activeElement
            : null),
      };
      pendingLeaveRef.current = pending;
      setPendingLeave(pending);
    },
    [],
  );

  const navigate = useCallback(
    (section: SectionId, restoreFocus?: HTMLElement) => {
      if (section === activeSection) return;
      requestLeave(
        () => {
          focusSectionHeadingRef.current = true;
          setActiveSection(section);
        },
        undefined,
        restoreFocus,
      );
    },
    [activeSection, requestLeave],
  );

  useEffect(() => {
    if (startupUpdateCheckStartedRef.current) return;
    startupUpdateCheckStartedRef.current = true;
    void checkForUpdate();
  }, [checkForUpdate]);

  useEffect(() => {
    function synchronizeVisibility() {
      void api.setWindowVisibility(document.visibilityState === "visible");
    }

    synchronizeVisibility();
    document.addEventListener("visibilitychange", synchronizeVisibility);
    return () =>
      document.removeEventListener("visibilitychange", synchronizeVisibility);
  }, [api]);

  useLayoutEffect(() => {
    void api.setAgentDraftDirty(agentExitGuarded);
  }, [agentExitGuarded, api]);

  useEffect(
    () => () => {
      void api.setAgentDraftDirty(false);
    },
    [api],
  );

  useEffect(() => {
    if (!blockedLeaveAttempt) return;
    const timeout = window.setTimeout(() => setBlockedLeaveAttempt(0), 5000);
    return () => window.clearTimeout(timeout);
  }, [blockedLeaveAttempt]);

  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | null = null;
    void api
      .subscribeAgentDraftQuitRequested(() =>
        requestLeave(
          () => void api.resolveAppQuit(true),
          () => void api.resolveAppQuit(false),
          undefined,
          "native-quit",
        ),
      )
      .then((stop) => {
        if (disposed) stop();
        else unsubscribe = stop;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [api, requestLeave]);

  useEffect(() => {
    if (!pendingLeave) return;
    leaveCancelRef.current?.focus();
    const pending = pendingLeave;
    function escape(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      pendingLeaveRef.current = null;
      setPendingLeave(null);
      pending.cancel();
      pending.restoreFocus?.focus();
    }
    window.addEventListener("keydown", escape);
    return () => window.removeEventListener("keydown", escape);
  }, [pendingLeave]);

  useEffect(() => {
    if (!focusSectionHeadingRef.current) return;
    focusSectionHeadingRef.current = false;
    sectionHeadingRef.current?.focus();
  }, [activeSection]);

  function cancelLeave() {
    if (!pendingLeave) return;
    const pending = pendingLeave;
    pendingLeaveRef.current = null;
    setPendingLeave(null);
    pending.cancel();
    pending.restoreFocus?.focus();
  }

  function confirmLeave() {
    if (!pendingLeave) return;
    const pending = pendingLeave;
    pendingLeaveRef.current = null;
    setPendingLeave(null);
    pending.confirm();
  }

  function toggleSidebar() {
    setSidebarCollapsed((current) => {
      const next = !current;
      writeSidebarCollapsed(next);
      return next;
    });
  }

  const updateAvailable =
    Boolean(updateResult?.available) && Boolean(updateResult?.update);

  return (
    <div
      className="app-frame"
      data-sidebar={sidebarCollapsed ? "collapsed" : "expanded"}
    >
      <aside className="sidebar">
        <a
          className="brand"
          href="#main-content"
          aria-label={t("app.homeAria")}
        >
          <span className="brand-mark" aria-hidden="true">
            CR
          </span>
          <span className="brand-copy">
            <strong>CodeasierRouter</strong>
            <small>{t("app.controlDesk")}</small>
          </span>
        </a>

        <nav aria-label={t("app.navigationAria")}>
          {navigationItems.map((id) => (
            <button
              key={id}
              type="button"
              className={
                activeSection === id ? "nav-item is-active" : "nav-item"
              }
              aria-label={
                id === "settings" && updateAvailable
                  ? `${t(navigationKeys[id])} - ${t("app.updateBadgeAria")}`
                  : t(navigationKeys[id])
              }
              aria-current={activeSection === id ? "page" : undefined}
              onClick={(event) => navigate(id, event.currentTarget)}
            >
              <span className="nav-marker" aria-hidden="true">
                <NavIcon id={id} />
              </span>
              <strong className="nav-label--full">
                {t(navigationKeys[id])}
              </strong>
              <strong className="nav-label--short">
                {t(shortNavigationKeys[id])}
              </strong>
              {id === "settings" && updateAvailable && (
                <span className="nav-badge" aria-hidden="true">
                  1
                </span>
              )}
            </button>
          ))}
        </nav>

        <div className="sidebar-foot">
          <button
            type="button"
            className="sidebar-collapse"
            aria-expanded={!sidebarCollapsed}
            aria-label={
              sidebarCollapsed
                ? t("app.sidebarExpand")
                : t("app.sidebarCollapse")
            }
            onClick={toggleSidebar}
          >
            <CollapseIcon collapsed={sidebarCollapsed} />
            <span className="sidebar-collapse__label">
              {sidebarCollapsed
                ? t("app.sidebarExpandShort")
                : t("app.sidebarCollapse")}
            </span>
          </button>
          <span className="connection-light" aria-hidden="true" />
          <div className="sidebar-foot__status">
            <strong>{t("app.localMode")}</strong>
            <small>{t("app.safeControlPlane")}</small>
          </div>
        </div>
      </aside>

      <main id="main-content">
        <header className="topbar">
          <h1 ref={sectionHeadingRef} tabIndex={-1}>
            {t(`${sectionKey}.title` as TranslationKey)}
          </h1>
        </header>

        <div className="main-scroll">
          {blockedLeaveAttempt > 0 && (
            <p
              key={blockedLeaveAttempt}
              className="agent-alert leave-blocked-notice"
              role="status"
            >
              {t("agents.leave.busy")}
            </p>
          )}

          {activeSection === "router" && (
            <RouterPage
              api={api}
              onNavigateToAgents={() => navigate("agents")}
              onNavigateToLogs={() => navigate("logs")}
            />
          )}
          {activeSection === "logs" && <LogsPage api={api} />}
          {activeSection === "agents" && (
            <AgentPage
              api={api}
              onNavigateToApiKeys={() => navigate("api-keys")}
              onRequestLeave={requestLeave}
              onDirtyChange={setAgentExitGuarded}
              registerLeaveGuard={registerLeaveGuard}
            />
          )}
          {activeSection === "api-keys" && <ApiKeysPage api={api} />}
          {activeSection === "usage" && (
            <UsagePage
              api={api}
              onNavigateToApiKeys={() => navigate("api-keys")}
            />
          )}
          {activeSection === "settings" && (
            <SettingsPage
              api={api}
              updateResult={updateResult}
              checkingForUpdate={checkingForUpdate}
              updateCheckError={updateCheckError}
              onCheckForUpdate={checkForUpdate}
            />
          )}
        </div>
      </main>
      {pendingLeave && (
        <div className="dialog-backdrop">
          <section
            className="danger-dialog leave-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="leave-dialog-title"
            aria-describedby="leave-dialog-description"
            onKeyDown={(event) => {
              if (event.key !== "Tab") return;
              if (
                event.shiftKey &&
                document.activeElement === leaveCancelRef.current
              ) {
                event.preventDefault();
                leaveConfirmRef.current?.focus();
              } else if (
                !event.shiftKey &&
                document.activeElement === leaveConfirmRef.current
              ) {
                event.preventDefault();
                leaveCancelRef.current?.focus();
              }
            }}
          >
            <p className="overline">{t("agents.leave.overline")}</p>
            <h2 id="leave-dialog-title">{t("agents.leave.title")}</h2>
            <p id="leave-dialog-description">{t("agents.leave.description")}</p>
            <div className="danger-dialog__actions">
              <button
                ref={leaveCancelRef}
                type="button"
                className="text-button"
                onClick={cancelLeave}
              >
                {t("agents.leave.cancel")}
              </button>
              <button
                ref={leaveConfirmRef}
                type="button"
                className="control-button control-button--danger"
                onClick={confirmLeave}
              >
                {t("agents.leave.confirm")}
              </button>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}

export function App({ api = desktopApi }: { api?: DesktopApi }) {
  return (
    <ThemeProvider>
      <I18nProvider synchronizeNativeLanguage={api.setNativeLanguage}>
        <AppContent api={api} />
      </I18nProvider>
    </ThemeProvider>
  );
}
