import { useI18n, type Translator } from "./i18n";
import {
  AgentLogo,
  agentNames,
  agentOrder,
  configurationPresentation,
  installationPresentation,
  recoveryReason,
  recoveryReasons,
  type AgentTarget,
  type ConfigurationPresentation,
} from "./agentPresentation";
import {
  sanitizeSensitiveText,
  type AgentDetection,
  type AgentState,
} from "./ipc";

export type OverviewIssue =
  | { kind: "credential" | "auth"; code: string }
  | { kind: "retry"; code: string; target: AgentTarget }
  | { kind: "detect"; code: string };

interface AgentOverviewProps {
  detection: AgentDetection;
  refreshing: boolean;
  stale: boolean;
  issue: OverviewIssue | null;
  onRefresh(): void;
  onConfigure(target: AgentTarget): void;
  onRetry(target: AgentTarget): void;
  onNavigateToApiKeys(): void;
}

function configurationLabel(
  state: ConfigurationPresentation["state"],
  t: Translator,
) {
  switch (state) {
    case "invalid":
      return t("agents.configuration.invalid");
    case "readonly":
      return t("agents.configuration.readonly");
    case "configured":
      return t("agents.configuration.configured");
    case "create":
      return t("agents.configuration.create");
    case "ready":
      return t("agents.configuration.ready");
  }
}

function configurationGuidance(
  agent: AgentState,
  state: ConfigurationPresentation["state"],
  t: Translator,
) {
  switch (state) {
    case "invalid":
      return t(
        agent.recovery.eligible
          ? "agents.recovery.guidance.eligible"
          : "agents.recovery.guidance.ineligible",
      );
    case "readonly":
      return t("agents.guidance.readonly");
    case "configured":
      return t("agents.guidance.configured");
    case "create":
    case "ready":
      return t("agents.guidance.ready");
  }
}

function actionLabel(
  agent: AgentState,
  presentation: ConfigurationPresentation,
  t: Translator,
) {
  const variables = { agent: agentNames[agent.agent] };
  if (presentation.state === "invalid")
    return t("agents.recovery.toggle", variables);
  if (presentation.state === "configured")
    return t("agents.action.edit", variables);
  if (presentation.state === "create")
    return t("agents.action.create", variables);
  return t("agents.action.configure", variables);
}

const discoveryIssueCodes = new Set([
  "MODEL_DISCOVERY_FAILED",
  "MODEL_RESPONSE_INVALID",
  "MODEL_CATALOG_EMPTY",
  "OPERATION_TIMEOUT",
  "AGENT_OPERATION_BUSY",
]);
const managerIssueCodes = new Set([
  "MANAGER_FAILED",
  "SIDECAR_MISSING",
  "SIDECAR_INVALID",
  "INVALID_RESPONSE",
]);

function retryIssueMessage(code: string, t: Translator) {
  if (discoveryIssueCodes.has(code)) return t("agents.issue.discovery");
  if (managerIssueCodes.has(code)) return t("agents.issue.manager");
  return t("agents.issue.unknown", {
    code: sanitizeSensitiveText(code || "UNKNOWN"),
  });
}

function credentialIssueMessage(code: string, t: Translator) {
  switch (code) {
    case "CREDENTIAL_NOT_FOUND":
      return t("agents.issue.credential.notFound");
    case "CREDENTIAL_INVALID":
      return t("agents.issue.credential.invalid");
    case "CREDENTIAL_IO_ERROR":
      return t("agents.issue.credential.io");
    case "CREDENTIAL_LOCK_TIMEOUT":
      return t("agents.issue.credential.locked");
    default:
      return t("agents.issue.credential.unavailable");
  }
}

function IssueNotice({
  issue,
  onRefresh,
  onRetry,
  onNavigateToApiKeys,
}: {
  issue: OverviewIssue;
  onRefresh(): void;
  onRetry(target: AgentTarget): void;
  onNavigateToApiKeys(): void;
}) {
  const { t } = useI18n();
  if (issue.kind === "credential" || issue.kind === "auth") {
    return (
      <div className="agent-alert agent-overview__error" role="alert">
        <span>
          {issue.kind === "auth"
            ? t("agents.issue.auth")
            : credentialIssueMessage(issue.code, t)}
        </span>{" "}
        <button
          type="button"
          className="text-button"
          onClick={onNavigateToApiKeys}
        >
          {t(
            issue.kind === "auth"
              ? "agents.issue.replaceApiKey"
              : "agents.issue.toApiKeys",
          )}
        </button>
      </div>
    );
  }
  if (issue.kind === "retry") {
    return (
      <div className="agent-alert agent-overview__error" role="alert">
        <span>{retryIssueMessage(issue.code, t)}</span>{" "}
        <button
          type="button"
          className="text-button"
          onClick={() => onRetry(issue.target)}
        >
          {t("agents.issue.retry", {
            agent: agentNames[issue.target.agent],
          })}
        </button>
      </div>
    );
  }
  return (
    <div className="agent-alert agent-overview__error" role="alert">
      <span>{retryIssueMessage(issue.code, t)}</span>{" "}
      <button type="button" className="text-button" onClick={onRefresh}>
        {t("agents.overview.refresh")}
      </button>
    </div>
  );
}

export function AgentOverview({
  detection,
  refreshing,
  stale,
  issue,
  onRefresh,
  onConfigure,
  onRetry,
  onNavigateToApiKeys,
}: AgentOverviewProps) {
  const { t } = useI18n();
  const byAgent = new Map(
    detection.agents.map((agent) => [agent.agent, agent]),
  );

  return (
    <>
      {issue && (
        <IssueNotice
          issue={issue}
          onRefresh={onRefresh}
          onRetry={onRetry}
          onNavigateToApiKeys={onNavigateToApiKeys}
        />
      )}
      <div className="agent-toolbar">
        {refreshing ? (
          <p className="agent-overview__loading" role="status">
            {t("agents.overview.refreshing")}
          </p>
        ) : stale ? (
          <p className="agent-overview__stale" role="note">
            {t("agents.overview.stale")}
          </p>
        ) : (
          <span />
        )}
        <button
          type="button"
          className="text-button"
          disabled={refreshing}
          onClick={onRefresh}
        >
          {t(
            refreshing
              ? "agents.overview.refreshing"
              : "agents.overview.refresh",
          )}
        </button>
      </div>
      <ul
        className="agent-card-grid"
        aria-label={t("agents.heading")}
        aria-busy={refreshing}
      >
        {agentOrder.map((id) => {
          const agent = byAgent.get(id);
          if (!agent) return null;
          const installation = installationPresentation(agent);
          const configuration = configurationPresentation(agent);
          const reasons = recoveryReasons(agent);
          const label = actionLabel(agent, configuration, t);
          const target: AgentTarget = {
            agent: id,
            mode: configuration.action === "rebuild" ? "rebuild" : "merge",
            installedAtEntry: installation.state === "installed",
          };

          return (
            <li className="agent-card" key={id}>
              <div className="agent-card__head">
                <AgentLogo agent={id} />
                <h3>{agentNames[id]}</h3>
              </div>
              <div className="agent-card__states">
                <span
                  className={`agent-state agent-state--installation agent-state--${installation.state.replace("_", "-")}`}
                  aria-label={`CLI: ${t(
                    installation.state === "installed"
                      ? "agents.installation.installed"
                      : "agents.installation.notInstalled",
                  )}`}
                >
                  {t(
                    installation.state === "installed"
                      ? "agents.installation.installed"
                      : "agents.installation.notInstalled",
                  )}
                </span>
                <span
                  className={`agent-state agent-state--configuration agent-state--${configuration.state}`}
                  aria-label={`${t("agents.stage.configure")}: ${configurationLabel(configuration.state, t)}`}
                >
                  {configurationLabel(configuration.state, t)}
                </span>
              </div>
              <p
                className="agent-card__config-path"
                title={sanitizeSensitiveText(agent.path)}
              >
                {sanitizeSensitiveText(agent.path)}
              </p>
              <p className="agent-card__guidance">
                {configurationGuidance(agent, configuration.state, t)}
              </p>
              {configuration.state === "invalid" && (
                <ul className="agent-recovery-reasons">
                  {(reasons.length ? reasons : [""]).map((reason) => (
                    <li key={reason || "unknown"}>
                      {recoveryReason(reason, t)}
                    </li>
                  ))}
                </ul>
              )}
              <button
                type="button"
                id={`agent-${id}-action`}
                className={
                  configuration.action === "rebuild"
                    ? "agent-card__action agent-rebuild-toggle"
                    : "agent-card__action control-button"
                }
                disabled={refreshing || configuration.action === "disabled"}
                onClick={() => onConfigure(target)}
              >
                {label}
              </button>
            </li>
          );
        })}
      </ul>
    </>
  );
}
