import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";

export const POLL_SNAPSHOT_EVENT = "router-poll-snapshot";
export const MAIN_WINDOW_FOCUSED_EVENT = "main-window-focused";
export const AGENT_DRAFT_QUIT_REQUESTED_EVENT = "agent-draft-quit-requested";

export const COMMANDS = {
  routerStatus: "router_status",
  routerStart: "router_start",
  routerStop: "router_stop",
  routerInspectOccupant: "router_inspect_occupant",
  routerForceTerminateOccupant: "router_force_terminate_occupant",
  routerCancelReleaseObservation: "router_cancel_release_observation",
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
  agentCleanupPreview: "agent_cleanup_preview",
  agentCleanupWrite: "agent_cleanup_write",
  agentFlowDestroy: "agent_model_flow_destroy",
  agentModelConfigImport: "agent_model_config_import",
  agentModelConfigExport: "agent_model_config_export",
  credentialGet: "get_credential",
  credentialSave: "save_credential",
  credentialDelete: "delete_credential",
  autostartGet: "autostart_get",
  autostartSet: "autostart_set",
  nativeLanguageSet: "set_native_language",
  desktopPaths: "desktop_paths",
  prepareForUninstall: "prepare_for_uninstall",
  windowVisibility: "window_visibility",
  setAgentDraftDirty: "set_agent_draft_dirty",
  resolveAppQuit: "resolve_app_quit",
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
}

type VerifiedOccupantIdentity<Recovery> =
  | {
      verification_mode: "verified_identity";
      process_name: string;
      executable: string;
    }
  | (Recovery extends {
      recovery: {
        action: "manual_stop_required";
        reason: "different_user";
      };
    }
      ? {
          verification_mode: "verified_identity";
          process_name?: never;
          executable?: never;
        }
      : never);

type OccupantIdentity<Recovery> =
  | VerifiedOccupantIdentity<Recovery>
  | {
      verification_mode: "windows_pid_only";
      process_name?: never;
      executable?: never;
    };

export type OccupantSupervisor =
  | {
      kind: "windows_service";
      scope: "system";
      identifiers: string[];
    }
  | {
      kind: "systemd_user";
      scope: "user";
      identifiers: string[];
    }
  | {
      kind: "systemd_system";
      scope: "system";
      identifiers: string[];
    };

type OccupantRecovery =
  | {
      recovery: { action: "force_terminate" };
      supervisor?: never;
      confirmation_token: string;
      expires_at: string;
    }
  | {
      recovery: {
        action: "manual_stop_required";
        reason: "service_managed";
      };
      supervisor?: OccupantSupervisor;
      confirmation_token?: never;
      expires_at?: never;
    }
  | {
      recovery: {
        action: "manual_stop_required";
        reason: "insufficient_privilege";
      };
      supervisor?: never;
      confirmation_token?: never;
      expires_at?: never;
    }
  | {
      recovery: {
        action: "manual_stop_required";
        reason: "different_user";
      };
      supervisor?: never;
      confirmation_token?: never;
      expires_at?: never;
    }
  | {
      recovery: {
        action: "unavailable";
        reason: "protected_process" | "identity_unavailable";
      };
      supervisor?: never;
      confirmation_token?: never;
      expires_at?: never;
    };

type OccupantInspectionVariant<Recovery> = Recovery extends OccupantRecovery
  ? OccupantInspectionBase & OccupantIdentity<Recovery> & Recovery
  : never;

export type OccupantInspection = OccupantInspectionVariant<OccupantRecovery>;

export interface OccupantTerminationResult {
  termination: "process_terminated";
  port_state: "released";
}

function exactKeys(value: Record<string, unknown>, allowed: readonly string[]) {
  const keys = Object.keys(value);
  return (
    keys.length === allowed.length && keys.every((key) => allowed.includes(key))
  );
}

function validManagerTimestamp(value: unknown): value is string {
  if (typeof value !== "string") return false;
  const match =
    /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):([0-5]\d)(?:\.\d+)?(?:Z|([+-])(\d{2}):(\d{2}))$/.exec(
      value,
    );
  if (!match) return false;
  const [, year, month, day, hour, minute, , , offsetHour, offsetMinute] =
    match;
  const numericYear = Number(year);
  const numericMonth = Number(month);
  const daysInMonth = [
    31,
    numericYear % 4 === 0 &&
    (numericYear % 100 !== 0 || numericYear % 400 === 0)
      ? 29
      : 28,
    31,
    30,
    31,
    30,
    31,
    31,
    30,
    31,
    30,
    31,
  ];
  return (
    numericMonth >= 1 &&
    numericMonth <= 12 &&
    Number(day) >= 1 &&
    Number(day) <= daysInMonth[numericMonth - 1] &&
    Number(hour) <= 23 &&
    Number(minute) <= 59 &&
    (offsetHour === undefined ||
      (Number(offsetHour) <= 23 && Number(offsetMinute) <= 59))
  );
}

function validUtf8(value: string): boolean {
  return !/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(^|[^\uD800-\uDBFF])[\uDC00-\uDFFF]/.test(
    value,
  );
}

function compareUtf8(left: string, right: string): number {
  const leftBytes = new TextEncoder().encode(left);
  const rightBytes = new TextEncoder().encode(right);
  const length = Math.min(leftBytes.length, rightBytes.length);
  for (let index = 0; index < length; index += 1) {
    if (leftBytes[index] !== rightBytes[index])
      return leftBytes[index] - rightBytes[index];
  }
  return leftBytes.length - rightBytes.length;
}

function validWindowsServiceName(value: string): boolean {
  return (
    value.trim() !== "" &&
    !/\p{Cc}/u.test(value) &&
    !["/", "\\", ",", '"'].some((character) => value.includes(character))
  );
}

function validSystemdServiceName(value: string): boolean {
  if (
    new TextEncoder().encode(value).length > 255 ||
    !value.endsWith(".service")
  )
    return false;
  const stem = value.slice(0, -".service".length);
  return (
    stem !== "" &&
    !stem.startsWith("@") &&
    (stem.match(/@/g)?.length ?? 0) <= 1 &&
    /^(?:[A-Za-z0-9:_.@-]|\\x[0-9A-Fa-f]{2})+$/.test(stem)
  );
}

function validOccupantSupervisor(value: unknown): value is OccupantSupervisor {
  if (!value || typeof value !== "object") return false;
  const supervisor = value as Record<string, unknown>;
  if (!exactKeys(supervisor, ["kind", "scope", "identifiers"])) return false;
  if (!Array.isArray(supervisor.identifiers)) return false;
  const identifiers = supervisor.identifiers;
  if (identifiers.length === 0 || identifiers.length > 16) return false;
  for (let index = 0; index < identifiers.length; index += 1) {
    const identifier = identifiers[index];
    if (
      typeof identifier !== "string" ||
      identifier === "" ||
      !validUtf8(identifier) ||
      new TextEncoder().encode(identifier).length > 256 ||
      (index > 0 && compareUtf8(identifiers[index - 1], identifier) >= 0)
    ) {
      return false;
    }
    if (
      supervisor.kind === "windows_service"
        ? !validWindowsServiceName(identifier)
        : !validSystemdServiceName(identifier)
    ) {
      return false;
    }
  }
  const kindAndScopeMatch =
    (supervisor.kind === "windows_service" && supervisor.scope === "system") ||
    (supervisor.kind === "systemd_user" && supervisor.scope === "user") ||
    (supervisor.kind === "systemd_system" && supervisor.scope === "system");
  return (
    kindAndScopeMatch &&
    new TextEncoder().encode(JSON.stringify(supervisor)).length <= 4 * 1024
  );
}

export function validOccupantInspection(
  value: unknown,
): value is OccupantInspection {
  if (!value || typeof value !== "object") return false;
  const inspection = value as Record<string, unknown>;
  if (
    !Number.isInteger(inspection.pid) ||
    (inspection.pid as number) <= 0 ||
    (inspection.pid as number) > 0xffffffff ||
    inspection.listen_addr !== "127.0.0.1:19099"
  ) {
    return false;
  }

  if (!inspection.recovery || typeof inspection.recovery !== "object")
    return false;
  const recovery = inspection.recovery as Record<string, unknown>;
  const identityKeys: string[] = [];
  if (inspection.verification_mode === "verified_identity") {
    const hasProcessName = Object.hasOwn(inspection, "process_name");
    const hasExecutable = Object.hasOwn(inspection, "executable");
    if (hasProcessName !== hasExecutable) return false;
    if (hasProcessName) {
      if (
        typeof inspection.process_name !== "string" ||
        inspection.process_name.trim() === "" ||
        typeof inspection.executable !== "string" ||
        inspection.executable.trim() === ""
      ) {
        return false;
      }
      identityKeys.push("process_name", "executable");
    } else if (
      recovery.action !== "manual_stop_required" ||
      recovery.reason !== "different_user"
    ) {
      return false;
    }
  } else if (inspection.verification_mode !== "windows_pid_only") {
    return false;
  }

  const baseKeys = [
    "pid",
    "verification_mode",
    ...identityKeys,
    "listen_addr",
    "recovery",
  ];
  if (recovery.action === "force_terminate") {
    return (
      exactKeys(recovery, ["action"]) &&
      exactKeys(inspection, [
        ...baseKeys,
        "confirmation_token",
        "expires_at",
      ]) &&
      typeof inspection.confirmation_token === "string" &&
      inspection.confirmation_token.trim() !== "" &&
      validManagerTimestamp(inspection.expires_at)
    );
  }
  if (recovery.action === "manual_stop_required") {
    const reason = recovery.reason;
    if (
      !exactKeys(recovery, ["action", "reason"]) ||
      !["service_managed", "insufficient_privilege", "different_user"].includes(
        reason as string,
      )
    ) {
      return false;
    }
    if (reason !== "service_managed") return exactKeys(inspection, baseKeys);
    return (
      exactKeys(inspection, baseKeys) ||
      (exactKeys(inspection, [...baseKeys, "supervisor"]) &&
        validOccupantSupervisor(inspection.supervisor))
    );
  }
  return (
    recovery.action === "unavailable" &&
    exactKeys(recovery, ["action", "reason"]) &&
    ["protected_process", "identity_unavailable"].includes(
      recovery.reason as string,
    ) &&
    exactKeys(inspection, baseKeys)
  );
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
  release_observation?: ReleaseObservation;
}

export interface ReleaseObservation {
  state: "observing" | "released" | "reoccupied";
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
  credentials_path: string;
  can_prepare_for_uninstall: boolean;
}

export interface CredentialSummary {
  present: boolean;
  fingerprint: string;
  saved_at: string | null;
}

export type AgentId = "claude" | "opencode" | "codex";
export type AgentMode = "merge" | "rebuild";
export type AgentModes = Partial<Record<AgentId, AgentMode>>;
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

export interface AgentRecoveryFileState {
  role: string;
  path: string;
  format: string;
  exists: boolean;
  reasons?: string[];
}

export interface AgentRecoveryState {
  eligible: boolean;
  reasons?: string[];
  files: AgentRecoveryFileState[];
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
  migratable?: boolean;
  recovery: AgentRecoveryState;
  cleanup: AgentCleanupState;
}

export interface AgentCleanupState {
  managed: boolean;
  available: boolean;
  reason?: string | null;
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
export type AgentFileOperation =
  "create" | "replace" | "preserve" | "delete" | "backup";
export interface AgentFileEffect {
  agent?: AgentId;
  mode?: AgentMode;
  path: string;
  role: string;
  format: string;
  operation: AgentFileOperation;
  backup_path?: string;
  backup_required?: boolean;
  backup_pattern?: string;
  backup_sensitive?: boolean;
  preserves?: string[];
  warning?: string;
}
export interface AgentBackupFileEffect extends AgentFileEffect {
  operation: "backup";
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
  state_backup?: AgentBackupFileEffect;
}

export interface AgentCleanupFileEffect extends AgentFileEffect {
  operation: "replace" | "delete";
}

export interface AgentCleanupPreview {
  revision_token: string;
  agent: AgentId;
  files: AgentCleanupFileEffect[];
  removed_paths: string[];
  managed_config_drift: boolean;
  state_change?: AgentCleanupFileEffect | null;
  state_backup?: AgentBackupFileEffect | null;
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
  state_backup?: AgentBackupFileEffect;
}

export interface DesktopApi {
  getPollSnapshot(): Promise<PollSnapshot>;
  subscribePollSnapshots(
    listener: (snapshot: PollSnapshot) => void,
  ): Promise<UnlistenFn>;
  subscribeMainWindowFocused(listener: () => void): Promise<UnlistenFn>;
  subscribeAgentDraftQuitRequested(listener: () => void): Promise<UnlistenFn>;
  setWindowVisibility(visible: boolean): Promise<void>;
  setAgentDraftDirty(dirty: boolean): Promise<void>;
  resolveAppQuit(confirmed: boolean): Promise<void>;
  getRouterStatus(): Promise<RouterStatus>;
  startRouter(): Promise<RouterStatus>;
  stopRouter(): Promise<RouterStatus>;
  inspectRouterOccupant(): Promise<OccupantInspection>;
  forceTerminateRouterOccupant(
    confirmationToken: string,
  ): Promise<OccupantTerminationResult>;
  cancelRouterReleaseObservation(): Promise<void>;
  retryRouterHealth(): Promise<RouterHealth>;
  getComponentVersions(): Promise<ComponentVersions>;
  getRouterLogs(limit?: number): Promise<RouterLogs>;
  collectDiagnostics(): Promise<Diagnostics>;
  openLogLocation(): Promise<void>;
  detectAgents(): Promise<AgentDetection>;
  discoverModels(agents: AgentId[]): Promise<AgentModelsResult>;
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
    modes: AgentModes,
  ): Promise<AgentPreview>;
  writeAgents(
    agents: AgentId[],
    flowId: string,
    catalogToken: string,
    modelConfig: ModelConfig,
    revisionToken: string,
    approveManagedOverwrite: boolean,
    approveCodexAuthChange: boolean,
    approveRebuild: AgentId[],
  ): Promise<AgentWriteResult>;
  previewAgentCleanup(agent: AgentId): Promise<AgentCleanupPreview>;
  writeAgentCleanup(
    agent: AgentId,
    revisionToken: string,
    approveManagedOverwrite: boolean,
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
  getCredential(): Promise<CredentialSummary>;
  saveCredential(apiKey: string): Promise<CredentialSummary>;
  deleteCredential(): Promise<CredentialSummary>;
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
    subscribeMainWindowFocused: (listener) =>
      listen<void>(MAIN_WINDOW_FOCUSED_EVENT, () => listener()),
    subscribeAgentDraftQuitRequested: (listener) =>
      listen<void>(AGENT_DRAFT_QUIT_REQUESTED_EVENT, () => listener()),
    setWindowVisibility: (visible) =>
      invoke(COMMANDS.windowVisibility, { visible }),
    setAgentDraftDirty: (dirty) =>
      invoke(COMMANDS.setAgentDraftDirty, { request: { dirty } }),
    resolveAppQuit: (confirmed) =>
      invoke(COMMANDS.resolveAppQuit, { request: { confirmed } }),
    getRouterStatus: () => invoke(COMMANDS.routerStatus),
    startRouter: () => invoke(COMMANDS.routerStart, { owner: "desktop" }),
    stopRouter: () => invoke(COMMANDS.routerStop),
    inspectRouterOccupant: () => invoke(COMMANDS.routerInspectOccupant),
    forceTerminateRouterOccupant: (confirmationToken) =>
      invoke(COMMANDS.routerForceTerminateOccupant, {
        request: { confirmation_token: confirmationToken },
      }),
    cancelRouterReleaseObservation: () =>
      invoke(COMMANDS.routerCancelReleaseObservation),
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
    discoverModels: (agents) =>
      invoke(COMMANDS.agentModels, { request: { agents } }),
    renderAgentConfig: (agents, flowId, catalogToken, modelConfig) =>
      invoke(COMMANDS.agentRender, {
        request: {
          agents,
          flow_id: flowId,
          catalog_token: catalogToken,
          model_config: modelConfig,
        },
      }),
    previewAgents: (agents, flowId, catalogToken, modelConfig, modes) =>
      invoke(COMMANDS.agentPreview, {
        request: {
          agents,
          flow_id: flowId,
          catalog_token: catalogToken,
          model_config: modelConfig,
          modes,
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
      approveRebuild,
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
          approve_rebuild: approveRebuild,
        },
      }),
    previewAgentCleanup: (agent) =>
      invoke(COMMANDS.agentCleanupPreview, { request: { agent } }),
    writeAgentCleanup: (agent, revisionToken, approveManagedOverwrite) =>
      invoke(COMMANDS.agentCleanupWrite, {
        request: {
          agent,
          revision_token: revisionToken,
          approve_managed_overwrite: approveManagedOverwrite,
        },
      }),
    destroyAgentModelFlow: (flowId) =>
      invoke(COMMANDS.agentFlowDestroy, { flowId }),
    importAgentModelConfig: (content, agents, flowId) =>
      invoke(COMMANDS.agentModelConfigImport, { content, agents, flowId }),
    exportAgentModelConfig: (modelConfig, agents, flowId) =>
      invoke(COMMANDS.agentModelConfigExport, { modelConfig, agents, flowId }),
    getCredential: () => invoke(COMMANDS.credentialGet),
    saveCredential: (apiKey) => invoke(COMMANDS.credentialSave, { apiKey }),
    deleteCredential: () => invoke(COMMANDS.credentialDelete),
    getAutostart: () => invoke(COMMANDS.autostartGet),
    setAutostart: (enabled) => invoke(COMMANDS.autostartSet, { enabled }),
    setNativeLanguage: (language) =>
      invoke(COMMANDS.nativeLanguageSet, { language }),
    getDesktopPaths: () => invoke(COMMANDS.desktopPaths),
    prepareForUninstall: () => invoke(COMMANDS.prepareForUninstall),
  };
}

export const desktopApi = createDesktopApi();
