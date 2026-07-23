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

function AppContent({ api }: { api: DesktopApi }) {
  const { t } = useI18n();
  const [activeSection, setActiveSection] = useState<SectionId>("router");
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

  return (
    <div className="app-frame">
      <aside className="sidebar">
        <a
          className="brand"
          href="#main-content"
          aria-label={t("app.homeAria")}
        >
          <span className="brand-mark" aria-hidden="true">
            CR
          </span>
          <span>
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
          <span className="connection-light" aria-hidden="true" />
          <div>
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
