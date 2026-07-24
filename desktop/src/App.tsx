import { useEffect, useState } from "react";

import { AgentPage } from "./AgentPage";
import { I18nProvider, useI18n } from "./i18n";
import { desktopApi, type DesktopApi } from "./ipc";
import { LogsPage } from "./LogsPage";
import type { TranslationKey } from "./locales/zh-CN";
import { navigationItems, type SectionId } from "./model";
import { RouterPage } from "./RouterPage";
import { SettingsPage } from "./SettingsPage";

const sectionKeys: Record<SectionId, string> = {
  router: "section.router",
  agents: "section.agents",
  logs: "section.logs",
  settings: "section.settings",
};

const navigationKeys: Record<SectionId, TranslationKey> = {
  router: "nav.router",
  agents: "nav.agents",
  logs: "nav.logs",
  settings: "nav.settings",
};

const shortNavigationKeys: Record<SectionId, TranslationKey> = {
  router: "nav.routerShort",
  agents: "nav.agentsShort",
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
  const sectionKey = sectionKeys[activeSection];

  useEffect(() => {
    function synchronizeVisibility() {
      void api.setWindowVisibility(document.visibilityState === "visible");
    }

    synchronizeVisibility();
    document.addEventListener("visibilitychange", synchronizeVisibility);
    return () =>
      document.removeEventListener("visibilitychange", synchronizeVisibility);
  }, [api]);

  function toggleSidebar() {
    setSidebarCollapsed((current) => {
      const next = !current;
      writeSidebarCollapsed(next);
      return next;
    });
  }

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
          {navigationItems.map((item) => (
            <button
              key={item.id}
              type="button"
              className={
                activeSection === item.id ? "nav-item is-active" : "nav-item"
              }
              aria-label={t(navigationKeys[item.id])}
              aria-current={activeSection === item.id ? "page" : undefined}
              onClick={() => setActiveSection(item.id)}
            >
              <span>{item.index}</span>
              <strong className="nav-label--full">
                {t(navigationKeys[item.id])}
              </strong>
              <strong className="nav-label--short">
                {t(shortNavigationKeys[item.id])}
              </strong>
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
          <div>
            <p>{t(`${sectionKey}.eyebrow` as TranslationKey)}</p>
            <h1>{t(`${sectionKey}.title` as TranslationKey)}</h1>
          </div>
          <div className="build-badge">
            <span>{t("app.ui")}</span>
            <strong>{t("app.phase")}</strong>
          </div>
        </header>

        <div className="main-scroll">
          <div className="content-intro">
            <p>{t(`${sectionKey}.description` as TranslationKey)}</p>
            <span aria-hidden="true">
              {navigationItems.findIndex((item) => item.id === activeSection) +
                1}
              /4
            </span>
          </div>

          {activeSection === "router" && (
            <RouterPage
              api={api}
              onNavigateToAgents={() => setActiveSection("agents")}
              onNavigateToLogs={() => setActiveSection("logs")}
            />
          )}
          {activeSection === "logs" && <LogsPage api={api} />}
          {activeSection === "agents" && <AgentPage api={api} />}
          {activeSection === "settings" && <SettingsPage api={api} />}
        </div>
      </main>
    </div>
  );
}

export function App({ api = desktopApi }: { api?: DesktopApi }) {
  return (
    <I18nProvider synchronizeNativeLanguage={api.setNativeLanguage}>
      <AppContent api={api} />
    </I18nProvider>
  );
}
