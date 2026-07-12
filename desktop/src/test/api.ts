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
    retryRouterHealth: vi.fn().mockResolvedValue({
      status: "ok",
      checked_at: new Date().toISOString(),
    }),
    getComponentVersions: vi.fn().mockResolvedValue({
      desktop: "desktop-v1",
      manager: "manager-v1",
      router: "router-v1",
      management_protocol: "1",
    }),
    getRouterLogs: vi.fn().mockResolvedValue({ lines: [] }),
    collectDiagnostics: vi.fn().mockResolvedValue({ summary: "safe summary" }),
    openLogLocation: vi.fn().mockResolvedValue(undefined),
    detectAgents: vi.fn().mockResolvedValue({ agents: [] }),
    previewAgents: vi.fn().mockResolvedValue({
      revision_token: "revision",
      agents: [],
    }),
    writeAgents: vi.fn().mockResolvedValue({
      transaction_id: "transaction",
      agents: [],
      sensitive_files: false,
      warning: "",
    }),
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
