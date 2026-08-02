import { describe, expect, it, vi } from "vitest";

import {
  COMMANDS,
  createDesktopApi,
  MAX_LOG_LINES,
  POLL_SNAPSHOT_EVENT,
  UPDATE_PROGRESS_EVENT,
  sanitizeSensitiveText,
  validOccupantInspection,
  type AgentDetection,
  type AgentCleanupPreview,
  type AgentModes,
  type AgentPreview,
  type InvokeFn,
  type ListenFn,
  type OccupantInspection,
  type OccupantTerminationResult,
} from "./ipc";

const detectionFixture = {
  agents: [
    {
      agent: "claude",
      name: "Claude Code",
      detected: true,
      path: "/home/example/.claude/settings.json",
      format: "json",
      exists: true,
      writable: false,
      configured: false,
      invalid: true,
      migratable: false,
      recovery: {
        eligible: true,
        reasons: ["syntax_invalid"],
        files: [
          {
            role: "config",
            path: "/home/example/.claude/settings.json",
            format: "json",
            exists: true,
            reasons: ["syntax_invalid"],
          },
        ],
      },
      cleanup: {
        managed: false,
        available: false,
        reason: "not_managed",
      },
    },
  ],
} satisfies AgentDetection;

const previewFixture = {
  revision_token: "revision-1",
  model_config: { version: 1 },
  fragments: [],
  files: [
    {
      agent: "claude",
      mode: "rebuild",
      path: "/home/example/.claude/settings.json",
      role: "config",
      format: "json",
      operation: "replace",
      backup_required: true,
      backup_pattern: "/home/example/.claude/settings.json.bak.*",
      backup_sensitive: true,
      preserves: ["unrelated files"],
      warning: "Existing invalid configuration will be rebuilt",
    },
  ],
  managed_config_drift: false,
  drifted_agents: [],
  managed_collisions: [],
  requires_codex_auth_approval: false,
} satisfies AgentPreview;

const cleanupPreviewFixture = {
  revision_token: "cleanup-revision-1",
  agent: "opencode",
  files: [
    {
      path: "/home/example/.config/opencode/opencode.json",
      role: "config",
      format: "json",
      operation: "delete",
      backup_required: true,
    },
  ],
  removed_paths: ["model", "provider.mtls-router"],
  managed_config_drift: true,
} satisfies AgentCleanupPreview;

if (
  cleanupPreviewFixture.files.some(
    (file) => "preserves" in file || "warning" in file,
  )
) {
  throw new Error("cleanup fixture must remain free of manager prose");
}

const forceableOccupant = {
  pid: 7,
  verification_mode: "windows_pid_only",
  listen_addr: "127.0.0.1:19099",
  recovery: { action: "force_terminate" },
  confirmation_token: "token",
  expires_at: "2026-07-25T00:00:30Z",
} satisfies OccupantInspection;

const blockedOccupants = [
  {
    pid: 8,
    verification_mode: "windows_pid_only",
    listen_addr: "127.0.0.1:19099",
    recovery: { action: "manual_stop_required", reason: "service_managed" },
    supervisor: {
      kind: "windows_service",
      scope: "system",
      identifiers: ["Router Service"],
    },
  },
  {
    pid: 8,
    verification_mode: "windows_pid_only",
    listen_addr: "127.0.0.1:19099",
    recovery: {
      action: "manual_stop_required",
      reason: "insufficient_privilege",
    },
  },
  {
    pid: 8,
    verification_mode: "windows_pid_only",
    listen_addr: "127.0.0.1:19099",
    recovery: { action: "manual_stop_required", reason: "different_user" },
  },
  {
    pid: 8,
    verification_mode: "windows_pid_only",
    listen_addr: "127.0.0.1:19099",
    recovery: { action: "unavailable", reason: "protected_process" },
  },
  {
    pid: 8,
    verification_mode: "windows_pid_only",
    listen_addr: "127.0.0.1:19099",
    recovery: { action: "unavailable", reason: "identity_unavailable" },
  },
  {
    pid: 8,
    verification_mode: "verified_identity",
    listen_addr: "127.0.0.1:19099",
    recovery: { action: "manual_stop_required", reason: "different_user" },
  },
] satisfies OccupantInspection[];

const terminationFixture = {
  termination: "process_terminated",
  port_state: "released",
} satisfies OccupantTerminationResult;

describe("occupant wire validation", () => {
  it("accepts every discriminated recovery branch and strict termination type", () => {
    expect(validOccupantInspection(forceableOccupant)).toBe(true);
    for (const fixture of blockedOccupants) {
      expect(validOccupantInspection(fixture)).toBe(true);
    }
    expect(
      validOccupantInspection({
        ...blockedOccupants[0],
        supervisor: undefined,
      }),
    ).toBe(false);
    const serviceWithoutMetadata = { ...blockedOccupants[0] } as Record<
      string,
      unknown
    >;
    delete serviceWithoutMetadata.supervisor;
    expect(validOccupantInspection(serviceWithoutMetadata)).toBe(true);
    expect(terminationFixture).toEqual({
      termination: "process_terminated",
      port_state: "released",
    });
  });

  it("rejects unknown fields, enums, invalid matrices, and blocked token fields", () => {
    const invalid = [
      { ...forceableOccupant, extra: true },
      { ...forceableOccupant, verification_mode: "unknown" },
      { ...forceableOccupant, recovery: { action: "unknown" } },
      {
        ...forceableOccupant,
        recovery: { action: "force_terminate", reason: "different_user" },
      },
      {
        ...blockedOccupants[1],
        recovery: {
          action: "manual_stop_required",
          reason: "protected_process",
        },
      },
      {
        ...blockedOccupants[3],
        recovery: { action: "unavailable", reason: "service_managed" },
      },
      { ...blockedOccupants[2], confirmation_token: "token" },
      { ...blockedOccupants[2], expires_at: "2026-07-25T00:00:30Z" },
      { ...forceableOccupant, confirmation_token: undefined },
      { ...forceableOccupant, expires_at: "not-a-timestamp" },
    ];
    for (const fixture of invalid) {
      expect(validOccupantInspection(fixture)).toBe(false);
    }
  });

  it("enforces exact endpoint, PID, and verification metadata", () => {
    const verified = {
      ...forceableOccupant,
      verification_mode: "verified_identity",
      process_name: "example-server",
      executable: "/usr/local/bin/example-server",
    };
    expect(validOccupantInspection(verified)).toBe(true);
    const redactedDifferentUser = blockedOccupants[5];
    for (const fixture of [
      { ...forceableOccupant, pid: 0 },
      { ...forceableOccupant, pid: 0x1_0000_0000 },
      { ...forceableOccupant, listen_addr: "127.0.0.1:19100" },
      { ...forceableOccupant, process_name: "unexpected" },
      { ...forceableOccupant, verification_mode: "verified_identity" },
      { ...blockedOccupants[0], verification_mode: "verified_identity" },
      { ...blockedOccupants[1], verification_mode: "verified_identity" },
      { ...blockedOccupants[3], verification_mode: "verified_identity" },
      { ...blockedOccupants[4], verification_mode: "verified_identity" },
      { ...verified, process_name: " " },
      { ...verified, executable: undefined },
      { ...redactedDifferentUser, process_name: "example-server" },
      {
        ...redactedDifferentUser,
        executable: "/usr/local/bin/example-server",
      },
    ]) {
      expect(validOccupantInspection(fixture)).toBe(false);
    }
  });

  it("enforces supervisor bounds, ordering, uniqueness, and total size", () => {
    const service = (identifiers: string[]) => ({
      ...blockedOccupants[0],
      supervisor: {
        kind: "windows_service",
        scope: "system",
        identifiers,
      },
    });
    const overStructureLimit = Array.from(
      { length: 16 },
      (_, index) => `${index.toString().padStart(2, "0")}${"x".repeat(254)}`,
    );
    for (const fixture of [
      service([]),
      service(Array.from({ length: 17 }, (_, index) => `Svc${index}`)),
      service(["Beta", "Alpha"]),
      service(["Alpha", "Alpha"]),
      service(["é".repeat(129)]),
      service(overStructureLimit),
    ]) {
      expect(validOccupantInspection(fixture)).toBe(false);
    }
  });

  it("accepts only safe Windows names and canonical systemd service units", () => {
    const supervised = (kind: string, scope: string, identifier: string) => ({
      ...blockedOccupants[0],
      supervisor: { kind, scope, identifiers: [identifier] },
    });
    expect(
      validOccupantInspection(
        supervised(
          "systemd_user",
          "user",
          String.raw`demo\x2dworker@blue.service`,
        ),
      ),
    ).toBe(true);
    expect(
      validOccupantInspection(
        supervised("systemd_system", "system", `${"x".repeat(247)}.service`),
      ),
    ).toBe(true);
    for (const identifier of [
      "Svc/name",
      String.raw`Svc\name`,
      "Svc,name",
      'Svc"name',
      "Svc\nname",
    ]) {
      expect(
        validOccupantInspection(
          supervised("windows_service", "system", identifier),
        ),
      ).toBe(false);
    }
    for (const identifier of [
      "@demo.service",
      "a@b@c.service",
      String.raw`demo\q20.service`,
      String.raw`demo\x2.service`,
      "demo scope.service",
      `${"x".repeat(248)}.service`,
    ]) {
      expect(
        validOccupantInspection(
          supervised("systemd_system", "system", identifier),
        ),
      ).toBe(false);
    }
  });
});

describe("typed desktop API", () => {
  it("uses content-free native lifecycle events and strict commands", async () => {
    const invoke = vi.fn().mockResolvedValue(undefined);
    const unlistenFocus = vi.fn();
    const unlistenQuit = vi.fn();
    const listen = vi
      .fn()
      .mockResolvedValueOnce(unlistenFocus)
      .mockResolvedValueOnce(unlistenQuit);
    const api = createDesktopApi(invoke as InvokeFn, listen as ListenFn);
    const focused = vi.fn();
    const quitRequested = vi.fn();

    await api.setAgentDraftDirty(true);
    await api.resolveAppQuit(false);
    const stopFocus = await api.subscribeMainWindowFocused(focused);
    const stopQuit = await api.subscribeAgentDraftQuitRequested(quitRequested);
    listen.mock.calls[0][1]({ payload: { ignored: true } });
    listen.mock.calls[1][1]({ payload: "ignored" });
    stopFocus();
    stopQuit();

    expect(invoke).toHaveBeenNthCalledWith(1, "set_agent_draft_dirty", {
      request: { dirty: true },
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "resolve_app_quit", {
      request: { confirmed: false },
    });
    expect(listen).toHaveBeenNthCalledWith(
      1,
      "main-window-focused",
      expect.any(Function),
    );
    expect(listen).toHaveBeenNthCalledWith(
      2,
      "agent-draft-quit-requested",
      expect.any(Function),
    );
    expect(focused).toHaveBeenCalledWith();
    expect(quitRequested).toHaveBeenCalledWith();
    expect(unlistenFocus).toHaveBeenCalledOnce();
    expect(unlistenQuit).toHaveBeenCalledOnce();
  });

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
    handler({
      payload: {
        revision: 4,
        status: { state: "absent" },
        release_observation: { state: "observing" },
      },
    });
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
      release_observation: { state: "observing" },
    });
    expect(invoke).toHaveBeenNthCalledWith(2, COMMANDS.windowVisibility, {
      visible: false,
    });
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it("uses the update commands and forwards typed download progress", async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce({ available: false, current_version: "1.0.0" })
      .mockResolvedValueOnce(undefined);
    const unlisten = vi.fn();
    const listen = vi.fn().mockResolvedValue(unlisten);
    const api = createDesktopApi(invoke as InvokeFn, listen as ListenFn);
    const observer = vi.fn();

    await api.checkForUpdate();
    const stop = await api.subscribeUpdateProgress(observer);
    listen.mock.calls[0][1]({ payload: { downloaded: 64, total: 128 } });
    await api.installUpdate("1.1.0");
    stop();

    expect(invoke).toHaveBeenNthCalledWith(1, COMMANDS.updateCheck);
    expect(listen).toHaveBeenCalledWith(
      UPDATE_PROGRESS_EVENT,
      expect.any(Function),
    );
    expect(observer).toHaveBeenCalledWith({ downloaded: 64, total: 128 });
    expect(invoke).toHaveBeenNthCalledWith(2, COMMANDS.updateInstall, {
      version: "1.1.0",
    });
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it("submits only the opaque token to the occupant termination command", async () => {
    const invoke = vi.fn().mockResolvedValue({ state: "absent" });
    const api = createDesktopApi(invoke as InvokeFn);

    await api.inspectRouterOccupant();
    await api.forceTerminateRouterOccupant("opaque-token");
    await api.cancelRouterReleaseObservation();

    expect(invoke).toHaveBeenNthCalledWith(1, COMMANDS.routerInspectOccupant);
    expect(invoke).toHaveBeenNthCalledWith(
      2,
      COMMANDS.routerForceTerminateOccupant,
      { request: { confirmation_token: "opaque-token" } },
    );
    expect(JSON.stringify(invoke.mock.calls)).not.toContain("executable");
    expect(JSON.stringify(invoke.mock.calls)).not.toContain("pid");
    expect(invoke).toHaveBeenNthCalledWith(
      3,
      COMMANDS.routerCancelReleaseObservation,
    );
  });

  it("uses focused v2 Agent commands without accepting a key", async () => {
    const invoke = vi.fn().mockResolvedValue({ agents: [] });
    const api = createDesktopApi(invoke as InvokeFn);

    await api.detectAgents();
    await api.discoverModels(["claude"]);
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
    const modes = { claude: "rebuild" } satisfies AgentModes;
    await api.previewAgents(["claude"], "flow", "catalog", modelConfig, modes);
    await api.renderAgentConfig(["claude"], "flow", "catalog", modelConfig);
    await api.writeAgents(
      ["claude"],
      "flow",
      "catalog",
      modelConfig,
      "revision-1",
      false,
      false,
      ["claude"],
    );

    expect(invoke).toHaveBeenNthCalledWith(1, COMMANDS.agentDetect);
    expect(invoke).toHaveBeenNthCalledWith(2, COMMANDS.agentModels, {
      request: { agents: ["claude"] },
    });
    expect(invoke).toHaveBeenNthCalledWith(3, COMMANDS.agentPreview, {
      request: {
        agents: ["claude"],
        flow_id: "flow",
        catalog_token: "catalog",
        model_config: modelConfig,
        modes,
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
        approve_rebuild: ["claude"],
      },
    });
    expect(invoke.mock.calls[4][1]).not.toHaveProperty("request.modes");
    expect(JSON.stringify(invoke.mock.calls)).not.toContain("api_key");
  });

  it("uses exact key-free cleanup command payloads", async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce(cleanupPreviewFixture)
      .mockResolvedValueOnce({ transaction_id: "cleanup-tx", agents: [] });
    const api = createDesktopApi(invoke as InvokeFn);

    await api.previewAgentCleanup("opencode");
    await api.writeAgentCleanup("opencode", "cleanup-revision-1", true);

    expect(invoke).toHaveBeenNthCalledWith(1, COMMANDS.agentCleanupPreview, {
      request: { agent: "opencode" },
    });
    expect(invoke).toHaveBeenNthCalledWith(2, COMMANDS.agentCleanupWrite, {
      request: {
        agent: "opencode",
        revision_token: "cleanup-revision-1",
        approve_managed_overwrite: true,
      },
    });
    const payload = JSON.stringify(invoke.mock.calls);
    for (const forbidden of [
      "api_key",
      "flow_id",
      "catalog_token",
      "model_config",
    ]) {
      expect(payload).not.toContain(forbidden);
    }
    expect(cleanupPreviewFixture.files[0]).not.toHaveProperty("preserves");
    expect(cleanupPreviewFixture.files[0]).not.toHaveProperty("warning");
  });

  it("exposes credential management without a credential readback API", async () => {
    const invoke = vi.fn().mockResolvedValue({
      present: true,
      fingerprint: "ABCD",
      saved_at: "2026-07-26T00:00:00Z",
    });
    const api = createDesktopApi(invoke as InvokeFn);

    await api.getCredential();
    await api.saveCredential("fixture-secret");
    await api.deleteCredential();

    expect(invoke).toHaveBeenNthCalledWith(1, COMMANDS.credentialGet);
    expect(invoke).toHaveBeenNthCalledWith(2, COMMANDS.credentialSave, {
      apiKey: "fixture-secret",
    });
    expect(invoke).toHaveBeenNthCalledWith(3, COMMANDS.credentialDelete);
    expect("useCredential" in (api as unknown as Record<string, unknown>)).toBe(
      false,
    );
  });

  it("preserves recovery detection and rebuild preview fields", () => {
    expect(detectionFixture.agents[0].recovery.files[0]).toMatchObject({
      role: "config",
      reasons: ["syntax_invalid"],
    });
    expect(previewFixture.files[0]).toMatchObject({
      agent: "claude",
      mode: "rebuild",
      backup_required: true,
      backup_sensitive: true,
      preserves: ["unrelated files"],
    });
  });

  it("uses only narrow settings and uninstall commands", async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce(true)
      .mockResolvedValueOnce(false)
      .mockResolvedValueOnce({
        data_dir: "/safe/app-data",
        log_file: "/safe/app-data/mtls-router.log",
        credentials_path: "/safe/app-data/credentials.json",
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
