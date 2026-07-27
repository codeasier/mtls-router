import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { AgentPage } from "./AgentPage";
import type { OverviewIssue } from "./AgentOverview";
import type { AgentTarget } from "./agentPresentation";
import type { AgentDetection, AgentModelsResult } from "./ipc";
import { createMockApi } from "./test/api";

vi.mock("./AgentWorkflow", () => ({
  AgentWorkflow: ({
    target,
    onBack,
    onFlowConsumed,
    onReturnToOverview,
    refreshDetection,
  }: {
    target: AgentTarget;
    onBack(): void;
    onFlowConsumed(): void;
    onReturnToOverview(issue?: OverviewIssue): void;
    refreshDetection(): Promise<AgentDetection>;
  }) => (
    <div aria-label={`workflow-${target.agent}`}>
      <button onClick={onBack}>workflow back</button>
      <button
        onClick={() =>
          onReturnToOverview({
            kind: "retry",
            code: "MODEL_FLOW_EXPIRED",
            target,
          })
        }
      >
        workflow error
      </button>
      <button onClick={onFlowConsumed}>workflow consume</button>
      <button
        onClick={() => {
          onFlowConsumed();
          void refreshDetection().then(() => onReturnToOverview());
        }}
      >
        workflow finish
      </button>
    </div>
  ),
}));

const detection: AgentDetection = {
  agents: [
    {
      agent: "claude",
      name: "Claude Code",
      detected: true,
      command: "/safe/bin/claude",
      path: "/safe/claude/settings.json",
      format: "json",
      exists: true,
      writable: true,
      configured: true,
      invalid: false,
      recovery: { eligible: false, files: [] },
    },
    {
      agent: "opencode",
      name: "OpenCode",
      detected: true,
      command: "/safe/bin/opencode",
      path: "/safe/opencode/config.json",
      format: "json",
      exists: true,
      writable: true,
      configured: false,
      invalid: false,
      recovery: { eligible: false, files: [] },
    },
    {
      agent: "codex",
      name: "Codex",
      detected: true,
      command: "/safe/bin/codex",
      path: "/safe/codex/config.toml",
      format: "toml",
      exists: true,
      writable: true,
      configured: false,
      invalid: false,
      recovery: { eligible: false, files: [] },
    },
  ],
};

const claudeDiscovery: AgentModelsResult = {
  flow_id: "flow-claude",
  models: ["model-a"],
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

function renderPage(
  overrides: Parameters<typeof createMockApi>[0] = {},
  onNavigateToApiKeys = vi.fn(),
) {
  const api = createMockApi({
    detectAgents: vi.fn().mockResolvedValue(detection),
    discoverModels: vi.fn().mockResolvedValue(claudeDiscovery),
    ...overrides,
  });
  const view = render(
    <AgentPage api={api} onNavigateToApiKeys={onNavigateToApiKeys} />,
  );
  return { api, view, onNavigateToApiKeys };
}

async function openClaude(overrides: Parameters<typeof createMockApi>[0] = {}) {
  const rendered = renderPage(overrides);
  fireEvent.click(
    await screen.findByRole("button", {
      name: /编辑 Claude Code 配置|Edit Claude Code configuration/,
    }),
  );
  await screen.findByLabelText("workflow-claude");
  return rendered;
}

describe("Agent page coordinator", () => {
  it("loads only complete local detection until one card is opened", async () => {
    const { api } = renderPage();

    await screen.findByText("/safe/claude/settings.json");
    expect(api.discoverModels).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("button", {
        name: /编辑 Claude Code 配置|Edit Claude Code configuration/,
      }),
    );

    await waitFor(() =>
      expect(api.discoverModels).toHaveBeenCalledWith(["claude"]),
    );
    expect(api.discoverModels).toHaveBeenCalledTimes(1);
  });

  it("retains cards and marks them stale when refresh fails", async () => {
    const detectAgents = vi
      .fn()
      .mockResolvedValueOnce(detection)
      .mockRejectedValueOnce({ code: "MANAGER_FAILED" });
    renderPage({ detectAgents });

    await screen.findByText("/safe/claude/settings.json");
    fireEvent.click(
      screen.getByRole("button", {
        name: /刷新|重新检测|Refresh|Detect again/,
      }),
    );

    expect(await screen.findByRole("note")).toHaveTextContent(
      /可能已过期|may be out of date/,
    );
    expect(screen.getByText("/safe/claude/settings.json")).toBeVisible();
  });

  it.each([
    ["missing", { agents: detection.agents.slice(0, 2) }],
    [
      "duplicate",
      {
        agents: [detection.agents[0], detection.agents[0], detection.agents[2]],
      },
    ],
  ])(
    "rejects %s initial detection without fabricating cards",
    async (_, value) => {
      const { api } = renderPage({
        detectAgents: vi.fn().mockResolvedValue(value),
      });

      expect(await screen.findByRole("alert")).toHaveTextContent(
        /Agent 检测失败|Agent detection failed/,
      );
      expect(
        screen.queryByText("/safe/claude/settings.json"),
      ).not.toBeInTheDocument();
      expect(api.discoverModels).not.toHaveBeenCalled();
    },
  );

  it("retains complete cards when refreshed detection is malformed", async () => {
    const detectAgents = vi
      .fn()
      .mockResolvedValueOnce(detection)
      .mockResolvedValueOnce({ agents: detection.agents.slice(0, 2) });
    renderPage({ detectAgents });

    await screen.findByText("/safe/claude/settings.json");
    fireEvent.click(
      screen.getByRole("button", {
        name: /刷新|重新检测|Refresh|Detect again/,
      }),
    );

    expect(await screen.findByRole("note")).toBeVisible();
    expect(screen.getByText("/safe/codex/config.toml")).toBeVisible();
  });

  it.each([
    ["CREDENTIAL_NOT_FOUND", /前往 API 密钥|Go to API Keys/],
    ["CREDENTIAL_INVALID", /前往 API 密钥|Go to API Keys/],
    ["CREDENTIAL_IO_ERROR", /前往 API 密钥|Go to API Keys/],
    ["CREDENTIAL_LOCK_TIMEOUT", /前往 API 密钥|Go to API Keys/],
    ["MODEL_AUTH_FAILED", /更换 API 密钥|Replace API key/],
    ["MODEL_DISCOVERY_FAILED", /重试 Claude Code|Retry Claude Code/],
    ["UNKNOWN_BACKEND_CODE", /重试 Claude Code|Retry Claude Code/],
  ])(
    "classifies %s without rendering backend messages",
    async (code, action) => {
      const secret = `secret-${code}`;
      const { api } = renderPage({
        discoverModels: vi.fn().mockRejectedValue({ code, message: secret }),
      });
      fireEvent.click(
        await screen.findByRole("button", {
          name: /编辑 Claude Code 配置|Edit Claude Code configuration/,
        }),
      );

      expect(await screen.findByRole("button", { name: action })).toBeVisible();
      expect(document.body.textContent).not.toContain(secret);
      expect(api.destroyAgentModelFlow).not.toHaveBeenCalled();
    },
  );

  it("retries discovery only for the issue's original Agent", async () => {
    const discoverModels = vi
      .fn()
      .mockRejectedValueOnce({ code: "MODEL_DISCOVERY_FAILED" })
      .mockResolvedValueOnce(claudeDiscovery);
    const { api } = renderPage({ discoverModels });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /编辑 Claude Code 配置|Edit Claude Code configuration/,
      }),
    );
    fireEvent.click(
      await screen.findByRole("button", {
        name: /重试 Claude Code|Retry Claude Code/,
      }),
    );

    await screen.findByLabelText("workflow-claude");
    expect(api.discoverModels).toHaveBeenNthCalledWith(1, ["claude"]);
    expect(api.discoverModels).toHaveBeenNthCalledWith(2, ["claude"]);
  });

  it("destroys the exact live flow once when the workflow goes back", async () => {
    const { api } = await openClaude();
    fireEvent.click(screen.getByRole("button", { name: "workflow back" }));

    await waitFor(() =>
      expect(api.destroyAgentModelFlow).toHaveBeenCalledWith("flow-claude"),
    );
    expect(api.destroyAgentModelFlow).toHaveBeenCalledTimes(1);
    expect(screen.getByText("/safe/claude/settings.json")).toBeVisible();
  });

  it("destroys once on a flow-invalidating workflow error and retains overview", async () => {
    const { api } = await openClaude();
    fireEvent.click(screen.getByRole("button", { name: "workflow error" }));

    expect(
      await screen.findByRole("button", {
        name: /重试 Claude Code|Retry Claude Code/,
      }),
    ).toBeVisible();
    expect(api.destroyAgentModelFlow).toHaveBeenCalledWith("flow-claude");
    expect(api.destroyAgentModelFlow).toHaveBeenCalledTimes(1);
    expect(screen.getByText("/safe/claude/settings.json")).toBeVisible();
  });

  it("consumes a successful write flow without destroying it", async () => {
    const { api, view } = await openClaude();
    fireEvent.click(screen.getByRole("button", { name: "workflow consume" }));
    view.unmount();

    expect(api.destroyAgentModelFlow).not.toHaveBeenCalled();
  });

  it("destroys the exact live flow once on unmount", async () => {
    const { api, view } = await openClaude();
    view.unmount();

    await waitFor(() =>
      expect(api.destroyAgentModelFlow).toHaveBeenCalledWith("flow-claude"),
    );
    expect(api.destroyAgentModelFlow).toHaveBeenCalledTimes(1);
  });

  it("does not destroy the live flow when rerendered with the same API", async () => {
    const { api, view, onNavigateToApiKeys } = await openClaude();
    view.rerender(
      <AgentPage api={api} onNavigateToApiKeys={onNavigateToApiKeys} />,
    );

    expect(screen.getByLabelText("workflow-claude")).toBeVisible();
    expect(api.destroyAgentModelFlow).not.toHaveBeenCalled();
  });

  it("refreshes detection after result finish without rediscovering or destroying", async () => {
    const { api } = await openClaude();
    fireEvent.click(screen.getByRole("button", { name: "workflow finish" }));

    await waitFor(() => expect(api.detectAgents).toHaveBeenCalledTimes(2));
    expect(screen.getByText("/safe/claude/settings.json")).toBeVisible();
    expect(api.discoverModels).toHaveBeenCalledTimes(1);
    expect(api.destroyAgentModelFlow).not.toHaveBeenCalled();
  });

  it("cleans up before navigating from a classified credential issue", async () => {
    const onNavigateToApiKeys = vi.fn();
    renderPage(
      {
        discoverModels: vi.fn().mockRejectedValue({
          code: "CREDENTIAL_NOT_FOUND",
        }),
      },
      onNavigateToApiKeys,
    );
    fireEvent.click(
      await screen.findByRole("button", {
        name: /编辑 Claude Code 配置|Edit Claude Code configuration/,
      }),
    );
    fireEvent.click(
      await screen.findByRole("button", {
        name: /前往 API 密钥|Go to API Keys/,
      }),
    );

    expect(onNavigateToApiKeys).toHaveBeenCalledTimes(1);
  });
});
