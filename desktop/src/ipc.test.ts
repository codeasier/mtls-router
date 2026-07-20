import { describe, expect, it, vi } from "vitest";

import {
  COMMANDS,
  createDesktopApi,
  MAX_LOG_LINES,
  POLL_SNAPSHOT_EVENT,
  sanitizeSensitiveText,
  type InvokeFn,
  type ListenFn,
} from "./ipc";

describe("typed desktop API", () => {
  it("centralizes lifecycle command names and typed arguments", async () => {
    const invoke = vi.fn().mockResolvedValue({ state: "desktop_owned" });
    const api = createDesktopApi(invoke as InvokeFn);

    await api.getRouterStatus();
    await api.startRouter();
    await api.stopRouter();
    await api.retryRouterHealth();

    expect(invoke).toHaveBeenNthCalledWith(1, COMMANDS.routerStatus);
    expect(invoke).toHaveBeenNthCalledWith(2, COMMANDS.routerStart, {
      owner: "desktop",
    });
    expect(invoke).toHaveBeenNthCalledWith(3, COMMANDS.routerStop);
    expect(invoke).toHaveBeenNthCalledWith(4, COMMANDS.routerHealth);
  });

  it("exposes one typed snapshot command and event subscription", async () => {
    const invoke = vi.fn().mockResolvedValue({ revision: 3 });
    const unlisten = vi.fn();
    const listen = vi.fn().mockResolvedValue(unlisten);
    const api = createDesktopApi(invoke as InvokeFn, listen as ListenFn);
    const observer = vi.fn();

    await api.getPollSnapshot();
    const stop = await api.subscribePollSnapshots(observer);
    const handler = listen.mock.calls[0][1];
    handler({ payload: { revision: 4, status: { state: "absent" } } });
    await api.setWindowVisibility(false);
    stop();

    expect(invoke).toHaveBeenNthCalledWith(1, COMMANDS.pollSnapshot);
    expect(listen).toHaveBeenCalledWith(
      POLL_SNAPSHOT_EVENT,
      expect.any(Function),
    );
    expect(observer).toHaveBeenCalledWith({
      revision: 4,
      status: { state: "absent" },
    });
    expect(invoke).toHaveBeenNthCalledWith(2, COMMANDS.windowVisibility, {
      visible: false,
    });
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it("submits only the opaque token to the occupant termination command", async () => {
    const invoke = vi.fn().mockResolvedValue({ state: "absent" });
    const api = createDesktopApi(invoke as InvokeFn);

    await api.inspectRouterOccupant();
    await api.forceTerminateRouterOccupant("opaque-token");

    expect(invoke).toHaveBeenNthCalledWith(1, COMMANDS.routerInspectOccupant);
    expect(invoke).toHaveBeenNthCalledWith(
      2,
      COMMANDS.routerForceTerminateOccupant,
      { request: { confirmation_token: "opaque-token" } },
    );
    expect(JSON.stringify(invoke.mock.calls)).not.toContain("executable");
    expect(JSON.stringify(invoke.mock.calls)).not.toContain("pid");
  });

  it("uses focused v2 Agent commands and never sends a key at write", async () => {
    const invoke = vi.fn().mockResolvedValue({ agents: [] });
    const api = createDesktopApi(invoke as InvokeFn);
    const transientKey = "fixture-sensitive-value";

    await api.detectAgents();
    await api.discoverModels(["claude"], transientKey);
    const modelConfig = {
      version: 1 as const,
      claude: {
        primary: { model: "m" },
        haiku: { inherit_primary: true as const },
        sonnet: { inherit_primary: true as const },
        opus: { inherit_primary: true as const },
        fable: { model: "m", name: "Fable", context: "1m" as const },
      },
    };
    await api.previewAgents(["claude"], "flow", "catalog", modelConfig);
    await api.renderAgentConfig(["claude"], "flow", "catalog", modelConfig);
    await api.writeAgents(
      ["claude"],
      "flow",
      "catalog",
      modelConfig,
      "revision-1",
      false,
      false,
    );

    expect(invoke).toHaveBeenNthCalledWith(1, COMMANDS.agentDetect);
    expect(invoke).toHaveBeenNthCalledWith(2, COMMANDS.agentModels, {
      request: { agents: ["claude"], api_key: transientKey },
    });
    expect(invoke).toHaveBeenNthCalledWith(3, COMMANDS.agentPreview, {
      request: {
        agents: ["claude"],
        flow_id: "flow",
        catalog_token: "catalog",
        model_config: modelConfig,
      },
    });
    expect(invoke).toHaveBeenNthCalledWith(4, COMMANDS.agentRender, {
      request: {
        agents: ["claude"],
        flow_id: "flow",
        catalog_token: "catalog",
        model_config: modelConfig,
      },
    });
    expect(invoke).toHaveBeenNthCalledWith(5, COMMANDS.agentWrite, {
      request: {
        agents: ["claude"],
        flow_id: "flow",
        catalog_token: "catalog",
        model_config: modelConfig,
        revision_token: "revision-1",
        approve_managed_overwrite: false,
        approve_codex_auth_change: false,
      },
    });
    expect(JSON.stringify(invoke.mock.calls.slice(2))).not.toContain(
      transientKey,
    );
  });

  it("uses only narrow settings and uninstall commands", async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce(true)
      .mockResolvedValueOnce(false)
      .mockResolvedValueOnce({
        data_dir: "/safe/app-data",
        log_file: "/safe/app-data/mtls-router.log",
        can_prepare_for_uninstall: true,
      })
      .mockResolvedValueOnce(undefined);
    const api = createDesktopApi(invoke as InvokeFn);

    await api.getAutostart();
    await api.setAutostart(false);
    await api.setNativeLanguage("en");
    await api.getDesktopPaths();
    await api.prepareForUninstall();

    expect(invoke).toHaveBeenNthCalledWith(1, COMMANDS.autostartGet);
    expect(invoke).toHaveBeenNthCalledWith(2, COMMANDS.autostartSet, {
      enabled: false,
    });
    expect(invoke).toHaveBeenNthCalledWith(3, COMMANDS.nativeLanguageSet, {
      language: "en",
    });
    expect(invoke).toHaveBeenNthCalledWith(4, COMMANDS.desktopPaths);
    expect(invoke).toHaveBeenNthCalledWith(5, COMMANDS.prepareForUninstall);
  });

  it("caps the backend log request and response at the frontend boundary", async () => {
    const invoke = vi.fn().mockResolvedValue({
      lines: Array.from(
        { length: MAX_LOG_LINES + 50 },
        (_, index) => `line ${index}`,
      ),
    });
    const api = createDesktopApi(invoke as InvokeFn);

    const result = await api.getRouterLogs(10_000);

    expect(invoke).toHaveBeenCalledWith(COMMANDS.routerLogs, {
      limit: MAX_LOG_LINES,
    });
    expect(result.lines).toHaveLength(MAX_LOG_LINES);
    expect(result.lines[0]).toBe("line 50");
  });

  it("redacts credentials, key-shaped canaries, and PEM blocks from logs and diagnostics", async () => {
    const secretKey = "sk-uiBoundaryCanary123456";
    const pem =
      "-----BEGIN PRIVATE KEY-----\nprivate-canary-material\n-----END PRIVATE KEY-----";
    const invoke = vi.fn(async (command: string) => {
      if (command === COMMANDS.routerLogs) {
        return { lines: [`Authorization: Bearer ${secretKey}`, pem] };
      }
      return { summary: `api_key=${secretKey}\n${pem}` };
    });
    const api = createDesktopApi(invoke as InvokeFn);

    const logs = await api.getRouterLogs();
    const diagnostics = await api.collectDiagnostics();

    expect(logs.lines.join("\n")).not.toContain(secretKey);
    expect(logs.lines.join("\n")).not.toContain("private-canary-material");
    expect(diagnostics.summary).not.toContain(secretKey);
    expect(diagnostics.summary).not.toContain("private-canary-material");
    expect(diagnostics.summary).toContain("[REDACTED");
  });
});

describe("sanitizeSensitiveText", () => {
  it("preserves ordinary operational log content", () => {
    expect(
      sanitizeSensitiveText(
        "time=2026-07-12 level=INFO method=POST path=/v1/messages status=200",
      ),
    ).toBe(
      "time=2026-07-12 level=INFO method=POST path=/v1/messages status=200",
    );
  });
});
