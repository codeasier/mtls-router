import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AgentWorkflow } from "./AgentWorkflow";
import type { AgentTarget } from "./agentPresentation";
import {
  type AgentDetection,
  type AgentModelsResult,
  type AgentPreview,
  type DesktopApi,
  type ModelConfig,
} from "./ipc";
import { createMockApi } from "./test/api";

const detection: AgentDetection = {
  agents: ["claude", "opencode", "codex"].map((agent) => ({
    agent: agent as "claude" | "opencode" | "codex",
    name: agent,
    detected: true,
    command: `/safe/bin/${agent}`,
    path: `/safe/${agent}`,
    format: agent === "codex" ? "toml" : "json",
    exists: true,
    writable: true,
    configured: false,
    invalid: false,
    recovery: { eligible: false, files: [] },
  })),
};

const claudeConfig: ModelConfig = {
  version: 1,
  claude: {
    primary: { model: "model-a" },
    haiku: { inherit_primary: true },
    sonnet: { inherit_primary: true },
    opus: { inherit_primary: true },
  },
};

const opencodeConfig: ModelConfig = {
  version: 1,
  opencode: { default_model: "model-a", models: { "model-a": {} } },
};

const codexConfig: ModelConfig = {
  version: 1,
  codex: { model: "model-b" },
};

const baseDiscovery: AgentModelsResult = {
  flow_id: "6e9cee5f-3e9d-42d9-a88e-9e0677edb806",
  models: ["model-a", "model-b"],
  catalog_token: "catalog-token",
  router_base_url: "http://127.0.0.1:19099",
  api_base_url: "http://127.0.0.1:19099/v1",
  existing: {
    model_config: {},
    unavailable_models: {},
    drifted_agents: [],
  },
  preset: { model_config: {}, unavailable_agents: {} },
};

const targets = {
  claude: {
    agent: "claude",
    mode: "merge",
    installedAtEntry: true,
  } satisfies AgentTarget,
  opencode: {
    agent: "opencode",
    mode: "merge",
    installedAtEntry: true,
  } satisfies AgentTarget,
  codex: {
    agent: "codex",
    mode: "merge",
    installedAtEntry: true,
  } satisfies AgentTarget,
  rebuild: {
    agent: "opencode",
    mode: "rebuild",
    installedAtEntry: true,
  } satisfies AgentTarget,
};

interface RenderOptions {
  api?: DesktopApi;
  target?: AgentTarget;
  discovery?: AgentModelsResult;
  refreshDetection?: () => Promise<AgentDetection>;
}

function renderWorkflow({
  api = createMockApi(),
  target = targets.claude,
  discovery = baseDiscovery,
  refreshDetection = vi.fn().mockResolvedValue(detection),
}: RenderOptions = {}) {
  const callbacks = {
    onBack: vi.fn(),
    onFlowConsumed: vi.fn(),
    onReturnToOverview: vi.fn(),
    refreshDetection,
  };
  render(
    <AgentWorkflow
      api={api}
      target={target}
      discovery={discovery}
      {...callbacks}
    />,
  );
  return { api, ...callbacks };
}

function previewFor(
  agent: "claude" | "opencode" | "codex",
  modelConfig: ModelConfig,
  overrides: Partial<AgentPreview> = {},
): AgentPreview {
  return {
    revision_token: "revision",
    model_config: modelConfig,
    fragments: [
      {
        agent,
        role: "config",
        path: `/safe/${agent}/config`,
        format: agent === "codex" ? "toml" : "json",
        content: '{"api_key":"<redacted-api-key>"}',
      },
    ],
    files: [
      {
        agent,
        mode: "merge",
        path: `/safe/${agent}/config`,
        role: "config",
        format: agent === "codex" ? "toml" : "json",
        operation: "replace",
      },
    ],
    managed_config_drift: false,
    drifted_agents: [],
    managed_collisions: [],
    requires_codex_auth_approval: false,
    ...overrides,
  };
}

const opencodeRebuildEffect = {
  agent: "opencode" as const,
  mode: "rebuild" as const,
  path: "/safe/opencode/config.json",
  role: "config",
  format: "json",
  operation: "replace",
};

function selectClaudePrimary() {
  fireEvent.change(screen.getByLabelText(/^主模型$|^Primary model$/), {
    target: { value: "model-a" },
  });
}

function generatePreview() {
  fireEvent.click(
    screen.getByRole("button", {
      name: /生成写入预览|Generate write preview/,
    }),
  );
}

function expectSingletonRequests(api: DesktopApi, agent: AgentTarget["agent"]) {
  for (const method of [
    api.previewAgents,
    api.writeAgents,
    api.importAgentModelConfig,
    api.exportAgentModelConfig,
  ]) {
    for (const call of vi.mocked(method).mock.calls) {
      const agents =
        method === api.importAgentModelConfig ||
        method === api.exportAgentModelConfig
          ? call[1]
          : call[0];
      expect(agents).toEqual([agent]);
    }
  }
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("single-Agent workflow", () => {
  it("constrains preview to one immutable target and omits selection, credentials, and the seven-step meter", async () => {
    const api = createMockApi({
      previewAgents: vi
        .fn()
        .mockResolvedValue(previewFor("claude", claudeConfig)),
    });
    renderWorkflow({ api });

    expect(
      screen.queryByText(/使用已保存的密钥|saved key/),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByLabelText(/当前步骤|Current stage/),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByLabelText(/选择 Claude Code|Select Claude Code/),
    ).not.toBeInTheDocument();
    selectClaudePrimary();
    generatePreview();

    await waitFor(() =>
      expect(api.previewAgents).toHaveBeenCalledWith(
        ["claude"],
        baseDiscovery.flow_id,
        baseDiscovery.catalog_token,
        expect.objectContaining({ claude: expect.any(Object) }),
        { claude: "merge" },
      ),
    );
    expectSingletonRequests(api, "claude");
  });

  it.each([
    ["existing", { existing: claudeConfig, preset: { version: 1 } }, "model-a"],
    ["preset", { existing: { version: 1 }, preset: claudeConfig }, "model-a"],
    ["empty", { existing: { version: 1 }, preset: { version: 1 } }, ""],
  ] as const)("initializes only Claude from %s", (_, source, expected) => {
    renderWorkflow({
      discovery: {
        ...baseDiscovery,
        existing: {
          ...baseDiscovery.existing,
          model_config: source.existing,
        },
        preset: {
          ...baseDiscovery.preset,
          model_config: source.preset,
        },
      },
    });
    expect(screen.getByLabelText(/^主模型$|^Primary model$/)).toHaveValue(
      expected,
    );
    expect(
      screen.queryByRole("heading", { name: "OpenCode" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Codex" }),
    ).not.toBeInTheDocument();
  });

  it("enables, edits, previews, and removes optional Fable", async () => {
    const api = createMockApi({
      previewAgents: vi
        .fn()
        .mockResolvedValue(previewFor("claude", claudeConfig)),
    });
    renderWorkflow({
      api,
      discovery: {
        ...baseDiscovery,
        existing: { ...baseDiscovery.existing, model_config: claudeConfig },
      },
    });

    const enable = screen.getByLabelText(/启用 Fable|Enable Fable/);
    fireEvent.click(enable);
    fireEvent.click(
      screen.getByLabelText(/fable 继承主模型|fable inherits primary/),
    );
    fireEvent.change(screen.getByLabelText(/fable 模型|fable model/), {
      target: { value: "model-b" },
    });
    fireEvent.change(
      screen.getByLabelText(/claude-fable (?:显示名称|Display name)/),
      {
        target: { value: "Fable display" },
      },
    );
    generatePreview();
    await waitFor(() => expect(api.previewAgents).toHaveBeenCalledTimes(1));
    expect(vi.mocked(api.previewAgents).mock.calls[0][3].claude?.fable).toEqual(
      {
        model: "model-b",
        name: "Fable display",
      },
    );

    fireEvent.click(
      screen.getByRole("button", { name: /返回配置|Back to configuration/ }),
    );
    fireEvent.click(enable);
    generatePreview();
    await waitFor(() => expect(api.previewAgents).toHaveBeenCalledTimes(2));
    expect(
      vi.mocked(api.previewAgents).mock.calls[1][3].claude?.fable,
    ).toBeUndefined();
    expectSingletonRequests(api, "claude");
  });

  it("round-trips constrained OpenCode nested JSON and import/export for only OpenCode", async () => {
    const createObjectURL = vi.fn().mockReturnValue("blob:safe");
    const revokeObjectURL = vi.fn();
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: createObjectURL,
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: revokeObjectURL,
    });
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(
      () => undefined,
    );
    const imported: ModelConfig = {
      version: 1,
      opencode: {
        default_model: "model-a",
        models: {
          "model-a": { variants: { imported: { reasoningEffort: "high" } } },
        },
      },
    };
    const api = createMockApi({
      importAgentModelConfig: vi.fn().mockResolvedValue(imported),
      exportAgentModelConfig: vi
        .fn()
        .mockResolvedValue(JSON.stringify(imported)),
      previewAgents: vi
        .fn()
        .mockResolvedValue(previewFor("opencode", imported)),
    });
    renderWorkflow({
      api,
      target: targets.opencode,
      discovery: {
        ...baseDiscovery,
        existing: { ...baseDiscovery.existing, model_config: opencodeConfig },
      },
    });

    const variants = screen.getByLabelText(/Variants JSON/);
    fireEvent.change(variants, {
      target: { value: '{"unsafe":{"connection":"secret"}}' },
    });
    expect(variants).toHaveAttribute("aria-invalid", "true");
    fireEvent.change(variants, {
      target: { value: '{"fast":{"reasoningEffort":"low"}}' },
    });
    expect(variants).toHaveAttribute("aria-invalid", "false");

    const file = new File([JSON.stringify(imported)], "opencode.json", {
      type: "application/json",
    });
    fireEvent.change(
      document.querySelector<HTMLInputElement>('input[type="file"]')!,
      {
        target: { files: [file] },
      },
    );
    await waitFor(() =>
      expect((variants as HTMLTextAreaElement).value).toContain("imported"),
    );
    fireEvent.click(
      screen.getByRole("button", { name: /导出.*配置|Export configuration/ }),
    );
    await waitFor(() => expect(api.exportAgentModelConfig).toHaveBeenCalled());
    generatePreview();
    await waitFor(() => expect(api.previewAgents).toHaveBeenCalled());
    expect(vi.mocked(api.previewAgents).mock.calls[0][3]).toEqual(imported);
    expect(createObjectURL).toHaveBeenCalled();
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:safe");
    expectSingletonRequests(api, "opencode");
  });

  it("preserves Codex typed omission and round-trips typed fields", async () => {
    const api = createMockApi({
      previewAgents: vi
        .fn()
        .mockImplementation(async (_agents, _flow, _catalog, config) =>
          previewFor("codex", config),
        ),
    });
    renderWorkflow({
      api,
      target: targets.codex,
      discovery: {
        ...baseDiscovery,
        preset: { ...baseDiscovery.preset, model_config: codexConfig },
      },
    });

    expect(screen.getByLabelText(/推理强度|Reasoning effort/)).toHaveValue("");
    fireEvent.change(screen.getByLabelText(/推理强度|Reasoning effort/), {
      target: { value: "high" },
    });
    fireEvent.change(screen.getByLabelText(/上下文窗口|Context window/), {
      target: { value: "400000" },
    });
    generatePreview();
    await waitFor(() => expect(api.previewAgents).toHaveBeenCalled());
    expect(vi.mocked(api.previewAgents).mock.calls[0][3]).toEqual({
      version: 1,
      codex: {
        model: "model-b",
        reasoning_effort: "high",
        context_window: 400000,
      },
    });
    expectSingletonRequests(api, "codex");
  });

  it("writes preview-normalized config only after managed drift and auth approvals", async () => {
    const normalized: ModelConfig = {
      ...claudeConfig,
      claude: {
        ...claudeConfig.claude!,
        primary: { model: "model-a", name: "Normalized" },
      },
    };
    const api = createMockApi({
      previewAgents: vi.fn().mockResolvedValue(
        previewFor("claude", normalized, {
          managed_config_drift: true,
          managed_collisions: [
            {
              agent: "claude",
              path: "/env/ANTHROPIC_MODEL",
              type: "fixed",
              action: "replace",
            },
          ],
          requires_codex_auth_approval: true,
        }),
      ),
      writeAgents: vi.fn().mockResolvedValue({
        transaction_id: "tx",
        agents: [
          { agent: "claude", success: true, changed: ["/safe/claude/config"] },
        ],
      }),
    });
    const callbacks = renderWorkflow({
      api,
      discovery: {
        ...baseDiscovery,
        existing: { ...baseDiscovery.existing, model_config: claudeConfig },
      },
    });
    callbacks.onFlowConsumed.mockImplementation(() => {
      expect(
        screen.queryByText(/Agent 配置结果|Agent configuration result/),
      ).not.toBeInTheDocument();
    });
    generatePreview();
    const write = await screen.findByRole("button", {
      name: /写入所选 Agent|Write selected Agents/,
    });
    expect(write).toBeDisabled();
    fireEvent.click(screen.getByLabelText(/漂移|drifted/));
    expect(write).toBeDisabled();
    fireEvent.click(screen.getByLabelText(/Codex/));
    fireEvent.click(write);

    await screen.findByText(/Agent 配置结果|Agent configuration result/);
    expect(api.writeAgents).toHaveBeenCalledWith(
      ["claude"],
      baseDiscovery.flow_id,
      baseDiscovery.catalog_token,
      normalized,
      "revision",
      true,
      true,
      [],
    );
    expect(callbacks.onFlowConsumed).toHaveBeenCalledTimes(1);
    expectSingletonRequests(api, "claude");
  });

  it("requires an exact rebuild preview and traps focus in explicit confirmation", async () => {
    const user = userEvent.setup();
    const rebuildPreview = previewFor("opencode", opencodeConfig, {
      files: [
        {
          ...opencodeRebuildEffect,
          backup_required: true,
          backup_pattern: "/safe/opencode/config.json.bak-<timestamp>-<random>",
          backup_sensitive: true,
          warning: "Existing unrelated settings will be discarded.",
        },
      ],
      state_change: {
        path: "/safe/manager/state.json",
        role: "state",
        format: "json",
        operation: "replace",
      },
    });
    const api = createMockApi({
      previewAgents: vi.fn().mockResolvedValue(rebuildPreview),
      writeAgents: vi
        .fn()
        .mockResolvedValue({ transaction_id: "tx", agents: [] }),
    });
    renderWorkflow({
      api,
      target: targets.rebuild,
      discovery: {
        ...baseDiscovery,
        existing: { ...baseDiscovery.existing, model_config: opencodeConfig },
      },
    });
    generatePreview();

    const write = await screen.findByRole("button", {
      name: /写入所选 Agent|Write selected Agents/,
    });
    expect(
      screen.getByText("/safe/opencode/config.json.bak-<timestamp>-<random>"),
    ).toBeVisible();
    fireEvent.click(write);
    const dialog = screen.getByRole("dialog", {
      name: /确认备份并重建|Confirm backup and rebuild/,
    });
    const cancel = screen.getByRole("button", { name: /^取消$|^Cancel$/ });
    const confirm = screen.getByRole("button", {
      name: /^备份并重建$|^Back up and rebuild$/,
    });
    expect(dialog).toHaveTextContent("OpenCode");
    expect(cancel).toHaveFocus();
    fireEvent.keyDown(cancel, { key: "Tab", shiftKey: true });
    expect(confirm).toHaveFocus();
    fireEvent.keyDown(confirm, { key: "Tab" });
    expect(cancel).toHaveFocus();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    fireEvent.click(write);
    await user.click(
      screen.getByRole("button", {
        name: /^备份并重建$|^Back up and rebuild$/,
      }),
    );
    await waitFor(() => expect(api.writeAgents).toHaveBeenCalled());
    expect(api.writeAgents).toHaveBeenCalledWith(
      ["opencode"],
      baseDiscovery.flow_id,
      baseDiscovery.catalog_token,
      opencodeConfig,
      "revision",
      false,
      false,
      ["opencode"],
    );
    expectSingletonRequests(api, "opencode");
  });

  const invalidRebuildPreviewCases: Array<
    [string, AgentTarget, Partial<AgentPreview>]
  > = [
    [
      "a merge target with a rebuild effect",
      targets.opencode,
      { files: [opencodeRebuildEffect] },
    ],
    [
      "a rebuild effect without an Agent",
      targets.rebuild,
      { files: [{ ...opencodeRebuildEffect, agent: undefined }] },
    ],
    [
      "a rebuild effect for another Agent",
      targets.rebuild,
      { files: [{ ...opencodeRebuildEffect, agent: "codex" as const }] },
    ],
    [
      "mixed merge and rebuild Agent effects",
      targets.rebuild,
      {
        files: [
          opencodeRebuildEffect,
          {
            ...opencodeRebuildEffect,
            mode: "merge" as const,
            path: "/safe/opencode/models.json",
          },
        ],
      },
    ],
    [
      "cross-Agent drift metadata",
      targets.rebuild,
      {
        files: [opencodeRebuildEffect],
        drifted_agents: ["codex" as const],
      },
    ],
    [
      "cross-Agent collision metadata",
      targets.rebuild,
      {
        files: [opencodeRebuildEffect],
        managed_collisions: [
          {
            agent: "codex" as const,
            path: "/safe/codex/config.toml",
            type: "fixed_managed_path",
            action: "replace",
          },
        ],
      },
    ],
  ];

  it.each(invalidRebuildPreviewCases)(
    "fails closed for %s",
    async (_, target, previewOverrides) => {
      const api = createMockApi({
        previewAgents: vi.fn().mockResolvedValue(
          previewFor("opencode", opencodeConfig, {
            ...previewOverrides,
          }),
        ),
      });
      renderWorkflow({
        api,
        target,
        discovery: {
          ...baseDiscovery,
          existing: { ...baseDiscovery.existing, model_config: opencodeConfig },
        },
      });
      generatePreview();
      const write = await screen.findByRole("button", {
        name: /写入所选 Agent|Write selected Agents/,
      });
      expect(write).toBeDisabled();
      fireEvent.click(write);
      expect(api.writeAgents).not.toHaveBeenCalled();
    },
  );

  it("keeps configure state after stale preview only while the immutable target remains eligible", async () => {
    const refreshDetection = vi.fn().mockResolvedValue(detection);
    const api = createMockApi({
      previewAgents: vi
        .fn()
        .mockResolvedValue(previewFor("claude", claudeConfig)),
      writeAgents: vi.fn().mockRejectedValue({
        code: "PREVIEW_STALE",
        message: "sk-stale-canary-secret",
      }),
    });
    const callbacks = renderWorkflow({
      api,
      refreshDetection,
      discovery: {
        ...baseDiscovery,
        existing: { ...baseDiscovery.existing, model_config: claudeConfig },
      },
    });
    generatePreview();
    fireEvent.click(
      await screen.findByRole("button", {
        name: /写入所选 Agent|Write selected Agents/,
      }),
    );

    expect(
      await screen.findByText(/预览后文件发生变化|Files changed after preview/),
    ).toBeVisible();
    expect(screen.getByLabelText(/^主模型$|^Primary model$/)).toBeVisible();
    expect(refreshDetection).toHaveBeenCalledTimes(1);
    expect(callbacks.onFlowConsumed).not.toHaveBeenCalled();
    expect(callbacks.onReturnToOverview).not.toHaveBeenCalled();
    expect(document.body.textContent).not.toContain("sk-stale-canary-secret");
  });

  it("rejects a blank preview revision before write and preserves parent cleanup ownership", async () => {
    const api = createMockApi({
      previewAgents: vi.fn().mockResolvedValue({
        ...previewFor("claude", claudeConfig),
        revision_token: "   ",
      }),
    });
    const callbacks = renderWorkflow({
      api,
      discovery: {
        ...baseDiscovery,
        existing: { ...baseDiscovery.existing, model_config: claudeConfig },
      },
    });
    generatePreview();

    await waitFor(() =>
      expect(callbacks.onReturnToOverview).toHaveBeenCalledWith({
        kind: "retry",
        code: "MODEL_RESPONSE_INVALID",
        target: targets.claude,
      }),
    );
    expect(callbacks.onFlowConsumed).not.toHaveBeenCalled();
    expect(api.writeAgents).not.toHaveBeenCalled();
  });

  it("returns stale and expired previews with stable retry issues", async () => {
    const codes = ["MODEL_CATALOG_STALE", "MODEL_FLOW_EXPIRED"] as const;
    for (const code of codes) {
      const api = createMockApi({
        previewAgents: vi.fn().mockRejectedValue({
          code,
          message: `sk-${code}-canary-secret`,
        }),
      });
      const callbacks = renderWorkflow({
        api,
        discovery: {
          ...baseDiscovery,
          existing: { ...baseDiscovery.existing, model_config: claudeConfig },
        },
      });
      generatePreview();
      await waitFor(() =>
        expect(callbacks.onReturnToOverview).toHaveBeenCalledWith({
          kind: "retry",
          code,
          target: targets.claude,
        }),
      );
      expect(document.body.textContent).not.toContain("canary-secret");
      expectSingletonRequests(api, "claude");
      document.body.innerHTML = "";
    }
  });

  it.each(["BACKUP_FAILED", "WRITE_FAILED"] as const)(
    "returns safe %s handling without consuming the flow",
    async (code) => {
      const api = createMockApi({
        previewAgents: vi
          .fn()
          .mockResolvedValue(previewFor("claude", claudeConfig)),
        writeAgents: vi.fn().mockRejectedValue({
          code,
          message: `sk-${code}-canary-secret`,
        }),
      });
      const callbacks = renderWorkflow({
        api,
        discovery: {
          ...baseDiscovery,
          existing: { ...baseDiscovery.existing, model_config: claudeConfig },
        },
      });
      generatePreview();
      fireEvent.click(
        await screen.findByRole("button", {
          name: /写入所选 Agent|Write selected Agents/,
        }),
      );

      await waitFor(() =>
        expect(callbacks.onReturnToOverview).toHaveBeenCalledWith({
          kind: "retry",
          code,
          target: targets.claude,
        }),
      );
      expect(callbacks.onFlowConsumed).not.toHaveBeenCalled();
      expect(document.body.textContent).not.toContain("canary-secret");
    },
  );

  it("refreshes detection after ROLLBACK_FAILED before allowing target retry", async () => {
    const refreshDetection = vi.fn().mockResolvedValue(detection);
    const api = createMockApi({
      previewAgents: vi
        .fn()
        .mockResolvedValue(previewFor("claude", claudeConfig)),
      writeAgents: vi.fn().mockRejectedValue({ code: "ROLLBACK_FAILED" }),
    });
    const callbacks = renderWorkflow({
      api,
      refreshDetection,
      discovery: {
        ...baseDiscovery,
        existing: { ...baseDiscovery.existing, model_config: claudeConfig },
      },
    });
    generatePreview();
    fireEvent.click(
      await screen.findByRole("button", {
        name: /写入所选 Agent|Write selected Agents/,
      }),
    );

    await waitFor(() =>
      expect(callbacks.onReturnToOverview).toHaveBeenCalledWith({
        kind: "retry",
        code: "ROLLBACK_FAILED",
        target: targets.claude,
      }),
    );
    expect(callbacks.onFlowConsumed).not.toHaveBeenCalled();
    expect(refreshDetection).toHaveBeenCalledTimes(1);
    expect(refreshDetection.mock.invocationCallOrder[0]).toBeLessThan(
      callbacks.onReturnToOverview.mock.invocationCallOrder[0],
    );
  });

  it("does not offer rollback retry when refreshed detection makes the target ineligible", async () => {
    const refreshDetection = vi.fn().mockResolvedValue({
      agents: detection.agents.map((agent) =>
        agent.agent === "claude" ? { ...agent, writable: false } : agent,
      ),
    });
    const api = createMockApi({
      previewAgents: vi
        .fn()
        .mockResolvedValue(previewFor("claude", claudeConfig)),
      writeAgents: vi.fn().mockRejectedValue({ code: "ROLLBACK_FAILED" }),
    });
    const callbacks = renderWorkflow({
      api,
      refreshDetection,
      discovery: {
        ...baseDiscovery,
        existing: { ...baseDiscovery.existing, model_config: claudeConfig },
      },
    });
    generatePreview();
    fireEvent.click(
      await screen.findByRole("button", {
        name: /写入所选 Agent|Write selected Agents/,
      }),
    );

    await waitFor(() =>
      expect(callbacks.onReturnToOverview).toHaveBeenCalled(),
    );
    expect(callbacks.onReturnToOverview).toHaveBeenCalledWith();
    expect(callbacks.onReturnToOverview).not.toHaveBeenCalledWith(
      expect.objectContaining({ kind: "retry" }),
    );
    expect(refreshDetection).toHaveBeenCalledTimes(1);
    expect(callbacks.onFlowConsumed).not.toHaveBeenCalled();
  });

  it("requires detection recovery instead of stale retry when rollback refresh fails", async () => {
    const refreshDetection = vi
      .fn()
      .mockRejectedValue(new Error("sk-rollback-detect-canary-secret"));
    const api = createMockApi({
      previewAgents: vi
        .fn()
        .mockResolvedValue(previewFor("claude", claudeConfig)),
      writeAgents: vi.fn().mockRejectedValue({ code: "ROLLBACK_FAILED" }),
    });
    const callbacks = renderWorkflow({
      api,
      refreshDetection,
      discovery: {
        ...baseDiscovery,
        existing: { ...baseDiscovery.existing, model_config: claudeConfig },
      },
    });
    generatePreview();
    fireEvent.click(
      await screen.findByRole("button", {
        name: /写入所选 Agent|Write selected Agents/,
      }),
    );

    await waitFor(() =>
      expect(callbacks.onReturnToOverview).toHaveBeenCalledWith({
        kind: "detect",
        code: "ROLLBACK_FAILED",
      }),
    );
    expect(callbacks.onReturnToOverview).not.toHaveBeenCalledWith(
      expect.objectContaining({ kind: "retry" }),
    );
    expect(callbacks.onFlowConsumed).not.toHaveBeenCalled();
    expect(document.body.textContent).not.toContain("canary-secret");
  });

  it.each([
    ["CREDENTIAL_NOT_FOUND", "credential"],
    ["CREDENTIAL_INVALID", "credential"],
    ["CREDENTIAL_IO_ERROR", "credential"],
    ["CREDENTIAL_LOCK_TIMEOUT", "credential"],
    ["MODEL_AUTH_FAILED", "auth"],
  ] as const)(
    "classifies write-time %s without consuming or exposing messages",
    async (code, kind) => {
      const secret = `sk-${code}-write-secret`;
      const api = createMockApi({
        previewAgents: vi
          .fn()
          .mockResolvedValue(previewFor("claude", claudeConfig)),
        writeAgents: vi.fn().mockRejectedValue({ code, message: secret }),
      });
      const callbacks = renderWorkflow({
        api,
        discovery: {
          ...baseDiscovery,
          existing: { ...baseDiscovery.existing, model_config: claudeConfig },
        },
      });
      generatePreview();
      fireEvent.click(
        await screen.findByRole("button", {
          name: /写入所选 Agent|Write selected Agents/,
        }),
      );

      await waitFor(() =>
        expect(callbacks.onReturnToOverview).toHaveBeenCalledWith({
          kind,
          code,
        }),
      );
      expect(callbacks.onFlowConsumed).not.toHaveBeenCalled();
      expect(document.body.textContent).not.toContain(secret);
    },
  );

  it("returns to overview when stale detection makes the target ineligible or fails", async () => {
    for (const refreshDetection of [
      vi.fn().mockResolvedValue({
        agents: detection.agents.map((agent) =>
          agent.agent === "claude" ? { ...agent, writable: false } : agent,
        ),
      }),
      vi.fn().mockRejectedValue(new Error("sk-detect-canary-secret")),
    ]) {
      const api = createMockApi({
        previewAgents: vi
          .fn()
          .mockResolvedValue(previewFor("claude", claudeConfig)),
        writeAgents: vi.fn().mockRejectedValue({ code: "PREVIEW_STALE" }),
      });
      const callbacks = renderWorkflow({
        api,
        refreshDetection,
        discovery: {
          ...baseDiscovery,
          existing: { ...baseDiscovery.existing, model_config: claudeConfig },
        },
      });
      generatePreview();
      fireEvent.click(
        await screen.findByRole("button", {
          name: /写入所选 Agent|Write selected Agents/,
        }),
      );
      await waitFor(() =>
        expect(callbacks.onReturnToOverview).toHaveBeenCalledWith({
          kind: "retry",
          code: "PREVIEW_STALE",
          target: targets.claude,
        }),
      );
      expect(document.body.textContent).not.toContain(
        "sk-detect-canary-secret",
      );
      document.body.innerHTML = "";
    }
  });

  it.each([
    ["missing", []],
    ["foreign", [{ agent: "codex" as const, success: true }]],
    [
      "duplicate",
      [
        { agent: "claude" as const, success: true },
        { agent: "claude" as const, success: true },
      ],
    ],
  ])(
    "rejects a %s write status without rendering transaction complete",
    async (_, agents) => {
      const api = createMockApi({
        previewAgents: vi
          .fn()
          .mockResolvedValue(previewFor("claude", claudeConfig)),
        writeAgents: vi.fn().mockResolvedValue({
          transaction_id: "tx-malformed",
          agents,
        }),
      });
      const callbacks = renderWorkflow({
        api,
        discovery: {
          ...baseDiscovery,
          existing: { ...baseDiscovery.existing, model_config: claudeConfig },
        },
      });
      generatePreview();
      fireEvent.click(
        await screen.findByRole("button", {
          name: /写入所选 Agent|Write selected Agents/,
        }),
      );

      await waitFor(() =>
        expect(callbacks.onReturnToOverview).toHaveBeenCalledWith({
          kind: "retry",
          code: "INVALID_RESPONSE",
          target: targets.claude,
        }),
      );
      expect(callbacks.onFlowConsumed).not.toHaveBeenCalled();
      expect(
        screen.queryByText(/Agent 配置结果|Agent configuration result/),
      ).not.toBeInTheDocument();
    },
  );

  it("renders the target's explicit failed write status as failure", async () => {
    const api = createMockApi({
      previewAgents: vi
        .fn()
        .mockResolvedValue(previewFor("claude", claudeConfig)),
      writeAgents: vi.fn().mockResolvedValue({
        transaction_id: "tx-failed-status",
        agents: [
          {
            agent: "claude",
            success: false,
            error_code: "WRITE_FAILED",
          },
        ],
      }),
    });
    const callbacks = renderWorkflow({
      api,
      discovery: {
        ...baseDiscovery,
        existing: { ...baseDiscovery.existing, model_config: claudeConfig },
      },
    });
    generatePreview();
    fireEvent.click(
      await screen.findByRole("button", {
        name: /写入所选 Agent|Write selected Agents/,
      }),
    );

    expect(await screen.findByText(/^失败$|^Failure$/)).toBeVisible();
    expect(callbacks.onFlowConsumed).not.toHaveBeenCalled();
    expect(callbacks.onReturnToOverview).not.toHaveBeenCalled();
  });

  it("shows sanitized result paths, consumes the flow, refreshes, and returns", async () => {
    const refreshDetection = vi.fn().mockResolvedValue(detection);
    const api = createMockApi({
      previewAgents: vi
        .fn()
        .mockResolvedValue(previewFor("claude", claudeConfig)),
      writeAgents: vi.fn().mockResolvedValue({
        transaction_id: "tx",
        agents: [
          {
            agent: "claude",
            success: true,
            changed: ["/safe/claude/config"],
            backups: [
              "/safe/claude/config.bak-safe",
              "/safe/sk-result-canary-secret.bak",
            ],
          },
        ],
      }),
    });
    const callbacks = renderWorkflow({
      api,
      refreshDetection,
      discovery: {
        ...baseDiscovery,
        existing: { ...baseDiscovery.existing, model_config: claudeConfig },
      },
    });
    generatePreview();
    fireEvent.click(
      await screen.findByRole("button", {
        name: /写入所选 Agent|Write selected Agents/,
      }),
    );
    expect(await screen.findByText("/safe/claude/config")).toBeVisible();
    expect(screen.getByText("/safe/claude/config.bak-safe")).toBeVisible();
    expect(document.body.textContent).not.toContain("sk-result-canary-secret");
    expect(callbacks.onFlowConsumed).toHaveBeenCalledTimes(1);
    fireEvent.click(
      screen.getByRole("button", {
        name: /完成并刷新检测|Finish and refresh detection/,
      }),
    );
    await waitFor(() => expect(refreshDetection).toHaveBeenCalledTimes(1));
    expect(callbacks.onReturnToOverview).toHaveBeenCalledWith();
  });

  it("sanitizes preview fragments, effects, stable codes, and never renders backend error messages", async () => {
    const secret = "sk-preview-canary-secret";
    const api = createMockApi({
      previewAgents: vi.fn().mockResolvedValue(
        previewFor("claude", claudeConfig, {
          fragments: [
            {
              agent: "claude",
              role: `config-${secret}`,
              path: `/safe/${secret}`,
              format: "json",
              content: `Authorization: Bearer ${secret}`,
            },
          ],
          files: [
            {
              agent: "claude",
              mode: "merge",
              path: `/safe/${secret}`,
              role: "config",
              format: "json",
              operation: "replace",
              warning: secret,
            },
          ],
        }),
      ),
    });
    renderWorkflow({
      api,
      discovery: {
        ...baseDiscovery,
        existing: { ...baseDiscovery.existing, model_config: claudeConfig },
      },
    });
    generatePreview();
    expect((await screen.findAllByText(/REDACTED/)).length).toBeGreaterThan(0);
    expect(document.documentElement.outerHTML).not.toContain(secret);

    fireEvent.click(
      screen.getByRole("button", { name: /返回配置|Back to configuration/ }),
    );
    vi.mocked(api.previewAgents).mockRejectedValueOnce({
      code: "MODEL_CONFIG_INVALID",
      message: secret,
      details: {
        path: "/claude/max_output_tokens",
        rule: "integer_relationship",
      },
    });
    generatePreview();
    expect(
      await screen.findByText(/最大输出 Token|maximum output tokens/),
    ).toBeVisible();
    expect(document.body.textContent).not.toContain(secret);
  });

  it("rejects oversized, non-JSON, and cross-Agent imports without changing the target", async () => {
    const api = createMockApi({
      importAgentModelConfig: vi.fn().mockResolvedValue({
        version: 1,
        claude: claudeConfig.claude,
        codex: codexConfig.codex,
      }),
    });
    renderWorkflow({ api });
    const input =
      document.querySelector<HTMLInputElement>('input[type="file"]')!;
    fireEvent.change(input, {
      target: {
        files: [new File(["{}"], "wrong.txt", { type: "text/plain" })],
      },
    });
    expect(await screen.findByText(/无法导入|Import/)).toBeVisible();
    expect(api.importAgentModelConfig).not.toHaveBeenCalled();

    const oversized = new File(["{}"], "oversized.json", {
      type: "application/json",
    });
    Object.defineProperty(oversized, "size", {
      value: 2 * 1024 * 1024 + 1,
    });
    fireEvent.change(input, { target: { files: [oversized] } });
    expect(api.importAgentModelConfig).not.toHaveBeenCalled();

    fireEvent.change(input, {
      target: {
        files: [new File(["{}"], "cross.json", { type: "application/json" })],
      },
    });
    await waitFor(() => expect(api.importAgentModelConfig).toHaveBeenCalled());
    expect(screen.getByText(/无法导入|Import/)).toBeVisible();
    expect(
      screen.queryByRole("heading", { name: "Codex" }),
    ).not.toBeInTheDocument();
    expectSingletonRequests(api, "claude");
  });

  it("uses Back for both configure and preview cancellation without consuming the flow", async () => {
    const api = createMockApi({
      previewAgents: vi
        .fn()
        .mockResolvedValue(previewFor("claude", claudeConfig)),
    });
    const callbacks = renderWorkflow({ api });
    fireEvent.click(
      screen.getByRole("button", { name: /取消并返回检测|Cancel and return/ }),
    );
    expect(callbacks.onBack).toHaveBeenCalledTimes(1);
    expect(callbacks.onFlowConsumed).not.toHaveBeenCalled();
  });
});
