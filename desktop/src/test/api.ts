import { vi } from "vitest";

import type { AgentId, DesktopApi, PollSnapshot } from "../ipc";

export function createMockApi(overrides: Partial<DesktopApi> = {}): DesktopApi {
  const api: DesktopApi = {
    getPollSnapshot: vi.fn(),
    subscribePollSnapshots: vi.fn().mockResolvedValue(() => undefined),
    subscribeMainWindowFocused: vi.fn().mockResolvedValue(() => undefined),
    subscribeAgentDraftQuitRequested: vi
      .fn()
      .mockResolvedValue(() => undefined),
    setWindowVisibility: vi.fn().mockResolvedValue(undefined),
    setAgentDraftDirty: vi.fn().mockResolvedValue(undefined),
    resolveAppQuit: vi.fn().mockResolvedValue(undefined),
    getRouterStatus: vi.fn().mockResolvedValue({ state: "absent" }),
    startRouter: vi.fn().mockResolvedValue({
      state: "desktop_owned",
      owner: "desktop",
      listen_addr: "127.0.0.1:19099",
    }),
    stopRouter: vi.fn().mockResolvedValue({ state: "absent" }),
    inspectRouterOccupant: vi.fn().mockResolvedValue({
      pid: 4242,
      verification_mode: "verified_identity",
      process_name: "example-server",
      executable: "/usr/local/bin/example-server",
      listen_addr: "127.0.0.1:19099",
      recovery: { action: "force_terminate" },
      confirmation_token: "opaque-token",
      expires_at: "2026-07-18T12:00:30Z",
    }),
    forceTerminateRouterOccupant: vi.fn().mockResolvedValue({
      termination: "process_terminated",
      port_state: "released",
    }),
    cancelRouterReleaseObservation: vi.fn().mockResolvedValue(undefined),
    retryRouterHealth: vi.fn().mockResolvedValue({
      status: "ok",
      checked_at: new Date().toISOString(),
    }),
    getComponentVersions: vi.fn().mockResolvedValue({
      desktop: "desktop-v1",
      manager: "manager-v1",
      router: "router-v1",
      management_protocol: "4",
    }),
    checkForUpdate: vi.fn().mockResolvedValue({
      available: false,
      current_version: "desktop-v1",
    }),
    installUpdate: vi.fn().mockResolvedValue(undefined),
    subscribeUpdateProgress: vi.fn().mockResolvedValue(() => undefined),
    getRouterLogs: vi.fn().mockResolvedValue({ lines: [] }),
    collectDiagnostics: vi.fn().mockResolvedValue({ summary: "safe summary" }),
    openLogLocation: vi.fn().mockResolvedValue(undefined),
    detectAgents: vi.fn().mockResolvedValue({ agents: [] }),
    discoverModels: vi.fn().mockResolvedValue({
      flow_id: "flow",
      models: ["model-a"],
      catalog_token: "catalog",
      router_base_url: "http://127.0.0.1:19099",
      api_base_url: "http://127.0.0.1:19099/v1",
      existing: {
        model_config: {},
        unavailable_models: {},
        drifted_agents: [],
      },
      preset: { model_config: {}, unavailable_agents: {} },
    }),
    renderAgentConfig: vi
      .fn()
      .mockResolvedValue({ model_config: { version: 1 }, fragments: [] }),
    previewAgents: vi.fn().mockResolvedValue({
      revision_token: "revision",
      model_config: { version: 1 },
      fragments: [],
      files: [],
      managed_config_drift: false,
      drifted_agents: [],
      managed_collisions: [],
      requires_codex_auth_approval: false,
    }),
    writeAgents: vi.fn().mockResolvedValue({
      transaction_id: "transaction",
      agents: [],
    }),
    previewAgentCleanup: vi.fn().mockImplementation(async (agent: AgentId) => ({
      revision_token: `cleanup-revision-${agent}`,
      agent,
      files: [],
      removed_paths: [],
      managed_config_drift: false,
    })),
    writeAgentCleanup: vi.fn().mockImplementation(async (agent: AgentId) => ({
      transaction_id: "cleanup-transaction",
      agents: [{ agent, success: true }],
    })),
    destroyAgentModelFlow: vi.fn().mockResolvedValue(undefined),
    importAgentModelConfig: vi
      .fn()
      .mockImplementation(async (content: string) => JSON.parse(content)),
    exportAgentModelConfig: vi
      .fn()
      .mockImplementation(async (config) => JSON.stringify(config, null, 2)),
    getCredential: vi.fn().mockResolvedValue({
      present: true,
      fingerprint: "ABCD",
      saved_at: "2026-07-26T00:00:00Z",
    }),
    saveCredential: vi.fn().mockResolvedValue({
      present: true,
      fingerprint: "ABCD",
      saved_at: "2026-07-26T00:00:00Z",
    }),
    deleteCredential: vi.fn().mockResolvedValue({
      present: false,
      fingerprint: "",
      saved_at: null,
    }),
    getAutostart: vi.fn().mockResolvedValue(true),
    setAutostart: vi
      .fn()
      .mockImplementation(async (enabled: boolean) => enabled),
    setNativeLanguage: vi.fn().mockResolvedValue(undefined),
    getDesktopPaths: vi.fn().mockResolvedValue({
      data_dir: "/safe/app-data",
      log_directory: "/safe/app-data/mtls-router-logs",
      credentials_path: "/safe/app-data/credentials.json",
      can_prepare_for_uninstall: true,
    }),
    prepareForUninstall: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };

  if (!overrides.getPollSnapshot) {
    let revision = 0;
    api.getPollSnapshot = vi.fn(async (): Promise<PollSnapshot> => {
      const status = await api.getRouterStatus();
      return {
        revision: ++revision,
        status,
        health:
          status.state === "desktop_owned" ||
          status.state === "external_compatible" ||
          status.state === "degraded"
            ? await api.retryRouterHealth()
            : undefined,
      };
    });
  }

  return api;
}
