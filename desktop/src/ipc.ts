import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";

export const POLL_SNAPSHOT_EVENT = "router-poll-snapshot";

export const COMMANDS = {
  routerStatus: "router_status",
  routerStart: "router_start",
  routerStop: "router_stop",
  routerInspectOccupant: "router_inspect_occupant",
  routerForceTerminateOccupant: "router_force_terminate_occupant",
  routerHealth: "router_health",
  pollSnapshot: "poll_snapshot",
  routerLogs: "router_logs",
  componentVersions: "component_versions",
  diagnosticsCollect: "diagnostics_collect",
  openLogLocation: "open_log_location",
  agentDetect: "agent_detect",
  agentModels: "agent_models",
  agentRender: "agent_render",
  agentPreview: "agent_preview",
  agentWrite: "agent_write",
  agentFlowDestroy: "agent_model_flow_destroy",
  agentModelConfigImport: "agent_model_config_import",
  agentModelConfigExport: "agent_model_config_export",
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

interface OccupantInspectionBase {
  pid: number;
  listen_addr: string;
  confirmation_token: string;
  expires_at: string;
}

export type OccupantInspection = OccupantInspectionBase &
  (
    | {
        verification_mode: "verified_identity";
        process_name: string;
        executable: string;
      }
    | {
        verification_mode: "windows_pid_only";
        process_name?: never;
        executable?: never;
      }
  );

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
export type JsonObject = Record<string, unknown>;

export interface ModelSelection {
  model: string;
  name?: string;
  context?: "1m";
}
export type ClaudeRole = { inherit_primary: true } | ModelSelection;
export interface ClaudeModelConfig {
  primary: ModelSelection;
  haiku: ClaudeRole;
  sonnet: ClaudeRole;
  opus: ClaudeRole;
  fable?: ClaudeRole;
  context_window?: number;
  max_output_tokens?: number;
  extra?: Record<string, string>;
}
export interface ModelLimit {
  context: number;
  input?: number;
  output: number;
}
export interface Modalities {
  input?: Array<"text" | "audio" | "image" | "video" | "pdf">;
  output?: Array<"text" | "audio" | "image" | "video" | "pdf">;
}
export interface OpenCodeModelConfig {
  name?: string;
  reasoning?: boolean;
  attachment?: boolean;
  tool_call?: boolean;
  temperature?: boolean;
  limit?: ModelLimit;
  modalities?: Modalities;
  interleaved?:
    true | { field: "reasoning" | "reasoning_content" | "reasoning_details" };
  options?: JsonObject;
  variants?: Record<string, JsonObject>;
  extra?: JsonObject;
}
export interface OpenCodeConfig {
  default_model: string;
  models: Record<string, OpenCodeModelConfig>;
}
export interface CodexConfig {
  model: string;
  reasoning_effort?: string;
  reasoning_summary?: "auto" | "concise" | "detailed" | "none";
  verbosity?: "low" | "medium" | "high";
  context_window?: number;
  auto_compact_token_limit?: number;
  extra?: {
    model_auto_compact_token_limit_scope?: "total" | "body_after_prefix";
  };
}
export interface ModelConfig {
  version: 1;
  claude?: ClaudeModelConfig;
  opencode?: OpenCodeConfig;
  codex?: CodexConfig;
}

export type InitializationSource = "existing" | "preset" | "empty";

export function initializeAgentConfig(
  agents: AgentId[],
  existing: Partial<ModelConfig>,
  preset: Partial<ModelConfig>,
): { config: ModelConfig; sources: Record<AgentId, InitializationSource> } {
  const config: ModelConfig = { version: 1 };
  const sources: Record<AgentId, InitializationSource> = {
    claude: "empty",
    opencode: "empty",
    codex: "empty",
  };
  for (const agent of agents) {
    sources[agent] = existing[agent]
      ? "existing"
      : preset[agent]
        ? "preset"
        : "empty";
    if (agent === "claude")
      config.claude = structuredClone(
        existing.claude ??
          preset.claude ?? {
            primary: { model: "" },
            haiku: { inherit_primary: true },
            sonnet: { inherit_primary: true },
            opus: { inherit_primary: true },
          },
      );
    if (agent === "opencode")
      config.opencode = structuredClone(
        existing.opencode ??
          preset.opencode ?? { default_model: "", models: {} },
      );
    if (agent === "codex")
      config.codex = structuredClone(
        existing.codex ?? preset.codex ?? { model: "" },
      );
  }
  return { config, sources };
}

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
export interface AgentModelsResult {
  flow_id: string;
  models: string[];
  catalog_token: string;
  router_base_url: string;
  api_base_url: string;
  existing: {
    model_config: Partial<ModelConfig>;
    unavailable_models: Partial<Record<AgentId, string[]>>;
    drifted_agents: AgentId[];
  };
  preset: {
    model_config: Partial<ModelConfig>;
    unavailable_agents: Partial<
      Record<AgentId, { code: "MODEL_NOT_AVAILABLE"; models: string[] }>
    >;
  };
}
export interface AgentFragment {
  agent: AgentId;
  role: string;
  path: string;
  format: string;
  content: string;
}
export interface AgentFileEffect {
  path: string;
  role: string;
  format: string;
  operation: string;
  backup_path?: string;
}
export interface AgentPreview {
  revision_token: string;
  model_config: ModelConfig;
  fragments: AgentFragment[];
  files: AgentFileEffect[];
  managed_config_drift: boolean;
  drifted_agents: AgentId[];
  managed_collisions: Array<{
    agent: AgentId;
    path: string;
    type: string;
    action: string;
  }>;
  requires_codex_auth_approval: boolean;
  state_change?: AgentFileEffect;
  state_backup?: AgentFileEffect;
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
  changed?: string[];
  backups?: string[];
  error_code?: string;
}

export interface AgentWriteResult {
  transaction_id: string;
  agents: AgentWriteStatus[];
  state_change?: AgentFileEffect;
  state_backup?: AgentFileEffect;
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
  inspectRouterOccupant(): Promise<OccupantInspection>;
  forceTerminateRouterOccupant(
    confirmationToken: string,
  ): Promise<RouterStatus>;
  retryRouterHealth(): Promise<RouterHealth>;
  getComponentVersions(): Promise<ComponentVersions>;
  getRouterLogs(limit?: number): Promise<RouterLogs>;
  collectDiagnostics(): Promise<Diagnostics>;
  openLogLocation(): Promise<void>;
  detectAgents(): Promise<AgentDetection>;
  discoverModels(agents: AgentId[], apiKey: string): Promise<AgentModelsResult>;
  renderAgentConfig(
    agents: AgentId[],
    flowId: string,
    catalogToken: string,
    modelConfig: ModelConfig,
  ): Promise<{ model_config: ModelConfig; fragments: AgentFragment[] }>;
  previewAgents(
    agents: AgentId[],
    flowId: string,
    catalogToken: string,
    modelConfig: ModelConfig,
  ): Promise<AgentPreview>;
  writeAgents(
    agents: AgentId[],
    flowId: string,
    catalogToken: string,
    modelConfig: ModelConfig,
    revisionToken: string,
    approveManagedOverwrite: boolean,
    approveCodexAuthChange: boolean,
  ): Promise<AgentWriteResult>;
  destroyAgentModelFlow(flowId: string): Promise<void>;
  importAgentModelConfig(
    content: string,
    agents: AgentId[],
    flowId: string,
  ): Promise<ModelConfig>;
  exportAgentModelConfig(
    modelConfig: ModelConfig,
    agents: AgentId[],
    flowId: string,
  ): Promise<string>;
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
    inspectRouterOccupant: () => invoke(COMMANDS.routerInspectOccupant),
    forceTerminateRouterOccupant: (confirmationToken) =>
      invoke(COMMANDS.routerForceTerminateOccupant, {
        request: { confirmation_token: confirmationToken },
      }),
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
    discoverModels: (agents, apiKey) =>
      invoke(COMMANDS.agentModels, { request: { agents, api_key: apiKey } }),
    renderAgentConfig: (agents, flowId, catalogToken, modelConfig) =>
      invoke(COMMANDS.agentRender, {
        request: {
          agents,
          flow_id: flowId,
          catalog_token: catalogToken,
          model_config: modelConfig,
        },
      }),
    previewAgents: (agents, flowId, catalogToken, modelConfig) =>
      invoke(COMMANDS.agentPreview, {
        request: {
          agents,
          flow_id: flowId,
          catalog_token: catalogToken,
          model_config: modelConfig,
        },
      }),
    writeAgents: (
      agents,
      flowId,
      catalogToken,
      modelConfig,
      revisionToken,
      approveManagedOverwrite,
      approveCodexAuthChange,
    ) =>
      invoke(COMMANDS.agentWrite, {
        request: {
          agents,
          flow_id: flowId,
          catalog_token: catalogToken,
          model_config: modelConfig,
          revision_token: revisionToken,
          approve_managed_overwrite: approveManagedOverwrite,
          approve_codex_auth_change: approveCodexAuthChange,
        },
      }),
    destroyAgentModelFlow: (flowId) =>
      invoke(COMMANDS.agentFlowDestroy, { flowId }),
    importAgentModelConfig: (content, agents, flowId) =>
      invoke(COMMANDS.agentModelConfigImport, { content, agents, flowId }),
    exportAgentModelConfig: (modelConfig, agents, flowId) =>
      invoke(COMMANDS.agentModelConfigExport, { modelConfig, agents, flowId }),
    getAutostart: () => invoke(COMMANDS.autostartGet),
    setAutostart: (enabled) => invoke(COMMANDS.autostartSet, { enabled }),
    setNativeLanguage: (language) =>
      invoke(COMMANDS.nativeLanguageSet, { language }),
    getDesktopPaths: () => invoke(COMMANDS.desktopPaths),
    prepareForUninstall: () => invoke(COMMANDS.prepareForUninstall),
  };
}

export const desktopApi = createDesktopApi();
