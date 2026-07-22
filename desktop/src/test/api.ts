import { vi } from "vitest";

import type { DesktopApi, PollSnapshot } from "../ipc";

export function createMockApi(overrides: Partial<DesktopApi> = {}): DesktopApi {
  const api: DesktopApi = {
    getPollSnapshot: vi.fn(),
    subscribePollSnapshots: vi.fn().mockResolvedValue(() => undefined),
    setWindowVisibility: vi.fn().mockResolvedValue(undefined),
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
      confirmation_token: "opaque-token",
      expires_at: "2026-07-18T12:00:30Z",
    }),
    forceTerminateRouterOccupant: vi
      .fn()
      .mockResolvedValue({ state: "absent" }),
    retryRouterHealth: vi.fn().mockResolvedValue({
      status: "ok",
      checked_at: new Date().toISOString(),
    }),
    getComponentVersions: vi.fn().mockResolvedValue({
      desktop: "desktop-v1",
      manager: "manager-v1",
      router: "router-v1",
      management_protocol: "3",
    }),
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
    destroyAgentModelFlow: vi.fn().mockResolvedValue(undefined),
    importAgentModelConfig: vi
      .fn()
      .mockImplementation(async (content: string) => JSON.parse(content)),
    exportAgentModelConfig: vi
      .fn()
      .mockImplementation(async (config) => JSON.stringify(config, null, 2)),
    getAutostart: vi.fn().mockResolvedValue(true),
    setAutostart: vi
      .fn()
      .mockImplementation(async (enabled: boolean) => enabled),
    setNativeLanguage: vi.fn().mockResolvedValue(undefined),
    getDesktopPaths: vi.fn().mockResolvedValue({
      data_dir: "/safe/app-data",
      log_file: "/safe/app-data/mtls-router.log",
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
