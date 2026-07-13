import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";

export const POLL_SNAPSHOT_EVENT = "router-poll-snapshot";

export const COMMANDS = {
  routerStatus: "router_status",
  routerStart: "router_start",
  routerStop: "router_stop",
  routerHealth: "router_health",
  pollSnapshot: "poll_snapshot",
  routerLogs: "router_logs",
  componentVersions: "component_versions",
  diagnosticsCollect: "diagnostics_collect",
  openLogLocation: "open_log_location",
  agentDetect: "agent_detect",
  agentPreview: "agent_preview",
  agentWrite: "agent_write",
  autostartGet: "autostart_get",
  autostartSet: "autostart_set",
  nativeLanguageSet: "set_native_language",
  desktopPaths: "desktop_paths",
  prepareForUninstall: "prepare_for_uninstall",
  windowVisibility: "window_visibility",
} as const;

export const MAX_LOG_LINES = 200;

export type RouterState =
  | "absent"
  | "starting"
  | "desktop_owned"
  | "external_compatible"
  | "degraded"
  | "stale"
  | "unknown_occupant"
  | "start_failed"
  | "stopping";

export interface RouterStatus {
  state: RouterState;
  owner?: "desktop" | "cli" | "external" | "none";
  listen_addr?: string;
  pid?: number;
  last_error?: string;
  recent_logs?: string[];
}

export interface RouterHealth {
  status: "ok" | "degraded" | "unknown";
  checked_at: string;
}

export interface PollError {
  code: string;
}

export interface PollSnapshot {
  revision: number;
  status?: RouterStatus;
  health?: RouterHealth;
  health_stale?: boolean;
  status_error?: PollError;
  health_error?: PollError;
}

export interface ComponentVersions {
  desktop: string;
  manager: string;
  router: string;
  management_protocol?: string;
}

export interface RouterLogs {
  lines: string[];
}

export interface Diagnostics {
  summary: string;
}

export interface DesktopPaths {
  data_dir: string;
  log_file: string;
  can_prepare_for_uninstall: boolean;
}

export type AgentId = "claude" | "opencode" | "codex";
export type NativeLanguage = "zh-CN" | "en";

export interface AgentState {
  agent: AgentId;
  name: string;
  detected: boolean;
  command?: string;
  path: string;
  auth_path?: string;
  format: string;
  exists: boolean;
  writable: boolean;
  configured: boolean;
  invalid: boolean;
}

export interface AgentDetection {
  agents: AgentState[];
}

export interface BackupPlan {
  required: boolean;
  pattern?: string;
  sensitive: boolean;
  warning?: string;
}

export interface AgentFilePreview {
  path: string;
  source_path?: string;
  format: string;
  operation: "create" | "replace" | "preserve";
  operations: Array<"create" | "replace" | "preserve">;
  contains_api_key: boolean;
  preserves?: string[];
  backup: BackupPlan;
  warning?: string;
}

export interface AgentPreviewItem {
  agent: AgentId;
  name: string;
  files: AgentFilePreview[];
  warnings?: string[];
}

export interface AgentPreview {
  revision_token: string;
  agents: AgentPreviewItem[];
  warnings?: string[];
}

export interface FileWriteStatus {
  path: string;
  backup_path?: string;
  rollback_backup_path?: string;
  replaced: boolean;
  restored?: boolean;
}

export interface AgentWriteStatus {
  agent: AgentId;
  success: boolean;
  files: FileWriteStatus[];
  changed?: string[];
  backups?: string[];
  rollback_backups?: string[];
  rolled_back?: boolean;
  error_code?: string;
}

export interface AgentWriteResult {
  transaction_id: string;
  agents: AgentWriteStatus[];
  sensitive_files: boolean;
  warning: string;
}

export interface DesktopApi {
  getPollSnapshot(): Promise<PollSnapshot>;
  subscribePollSnapshots(
    listener: (snapshot: PollSnapshot) => void,
  ): Promise<UnlistenFn>;
  setWindowVisibility(visible: boolean): Promise<void>;
  getRouterStatus(): Promise<RouterStatus>;
  startRouter(): Promise<RouterStatus>;
  stopRouter(): Promise<RouterStatus>;
  retryRouterHealth(): Promise<RouterHealth>;
  getComponentVersions(): Promise<ComponentVersions>;
  getRouterLogs(limit?: number): Promise<RouterLogs>;
  collectDiagnostics(): Promise<Diagnostics>;
  openLogLocation(): Promise<void>;
  detectAgents(): Promise<AgentDetection>;
  previewAgents(agents: AgentId[]): Promise<AgentPreview>;
  writeAgents(
    agents: AgentId[],
    revisionToken: string,
    apiKey: string,
  ): Promise<AgentWriteResult>;
  getAutostart(): Promise<boolean>;
  setAutostart(enabled: boolean): Promise<boolean>;
  setNativeLanguage(language: NativeLanguage): Promise<void>;
  getDesktopPaths(): Promise<DesktopPaths>;
  prepareForUninstall(): Promise<void>;
}

export type InvokeFn = <T>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export type ListenFn = <T>(
  event: string,
  handler: (event: { payload: T }) => void,
) => Promise<UnlistenFn>;

export function sanitizeSensitiveText(value: string): string {
  return value
    .replace(
      /-----BEGIN (?:RSA |EC |OPENSSH )?(?:PRIVATE KEY|CERTIFICATE)-----[\s\S]*?-----END (?:RSA |EC |OPENSSH )?(?:PRIVATE KEY|CERTIFICATE)-----/gi,
      "[REDACTED PEM]",
    )
    .replace(/\bBearer\s+[^\s,;]+/gi, "Bearer [REDACTED]")
    .replace(
      /\b(authorization|proxy-authorization|api[_-]?key|access[_-]?token|client[_-]?secret|password)(\s*[:=]\s*)([^\s,;]+)/gi,
      "$1$2[REDACTED]",
    )
    .replace(/\bsk-[A-Za-z0-9_-]{8,}\b/g, "[REDACTED KEY]")
    .replace(/\b[^\s"'=,;]*canary[^\s"',;]*\b/gi, "[REDACTED CANARY]");
}

function boundedSanitizedLines(lines: string[], limit: number): string[] {
  const bounded = lines.slice(-Math.min(Math.max(limit, 1), MAX_LOG_LINES));
  return sanitizeSensitiveText(bounded.join("\n")).split("\n");
}

export function createDesktopApi(
  invoke: InvokeFn = tauriInvoke,
  listen: ListenFn = tauriListen,
): DesktopApi {
  return {
    getPollSnapshot: () => invoke(COMMANDS.pollSnapshot),
    subscribePollSnapshots: (listener) =>
      listen<PollSnapshot>(POLL_SNAPSHOT_EVENT, (event) =>
        listener(event.payload),
      ),
    setWindowVisibility: (visible) =>
      invoke(COMMANDS.windowVisibility, { visible }),
    getRouterStatus: () => invoke(COMMANDS.routerStatus),
    startRouter: () => invoke(COMMANDS.routerStart, { owner: "desktop" }),
    stopRouter: () => invoke(COMMANDS.routerStop),
    retryRouterHealth: () => invoke(COMMANDS.routerHealth),
    getComponentVersions: () => invoke(COMMANDS.componentVersions),
    getRouterLogs: async (limit = MAX_LOG_LINES) => {
      const safeLimit = Math.min(Math.max(limit, 1), MAX_LOG_LINES);
      const result = await invoke<RouterLogs>(COMMANDS.routerLogs, {
        limit: safeLimit,
      });
      return { lines: boundedSanitizedLines(result.lines, safeLimit) };
    },
    collectDiagnostics: async () => {
      const result = await invoke<Diagnostics>(COMMANDS.diagnosticsCollect);
      return { summary: sanitizeSensitiveText(result.summary) };
    },
    openLogLocation: () => invoke(COMMANDS.openLogLocation),
    detectAgents: () => invoke(COMMANDS.agentDetect),
    previewAgents: (agents) =>
      invoke(COMMANDS.agentPreview, { request: { agents } }),
    writeAgents: (agents, revisionToken, apiKey) =>
      invoke(COMMANDS.agentWrite, {
        request: {
          agents,
          revision_token: revisionToken,
          api_key: apiKey,
        },
      }),
    getAutostart: () => invoke(COMMANDS.autostartGet),
    setAutostart: (enabled) => invoke(COMMANDS.autostartSet, { enabled }),
    setNativeLanguage: (language) =>
      invoke(COMMANDS.nativeLanguageSet, { language }),
    getDesktopPaths: () => invoke(COMMANDS.desktopPaths),
    prepareForUninstall: () => invoke(COMMANDS.prepareForUninstall),
  };
}

export const desktopApi = createDesktopApi();
