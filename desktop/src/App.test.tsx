import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "./App";
import type { AgentDetection, AgentModelsResult } from "./ipc";
import { createMockApi } from "./test/api";

const detection: AgentDetection = {
  agents: ["claude", "opencode", "codex"].map((agent) => ({
    agent: agent as "claude" | "opencode" | "codex",
    name: agent,
    detected: true,
    command: `/safe/bin/${agent}`,
    path: `/safe/${agent}/config`,
    format: agent === "codex" ? "toml" : "json",
    exists: true,
    writable: true,
    configured: true,
    invalid: false,
    recovery: { eligible: false, files: [] },
  })),
};

const agentDiscovery: AgentModelsResult = {
  flow_id: "flow-guard",
  models: ["model-a", "model-b"],
  catalog_token: "catalog-guard",
  router_base_url: "http://127.0.0.1:19099",
  api_base_url: "http://127.0.0.1:19099/v1",
  existing: {
    model_config: {
      version: 1,
      claude: {
        primary: { model: "model-a" },
        haiku: { inherit_primary: true },
        sonnet: { inherit_primary: true },
        opus: { inherit_primary: true },
      },
    },
    unavailable_models: {},
    drifted_agents: [],
  },
  preset: { model_config: {}, unavailable_agents: {} },
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

async function openDirtyAgent(api: ReturnType<typeof createMockApi>) {
  fireEvent.click(screen.getByRole("button", { name: "Agent 配置" }));
  fireEvent.click(
    await screen.findByRole("button", { name: "编辑 Claude Code 配置" }),
  );
  fireEvent.change(await screen.findByLabelText("主模型"), {
    target: { value: "model-b" },
  });
  await waitFor(() =>
    expect(api.setAgentDraftDirty).toHaveBeenLastCalledWith(true),
  );
}

describe("App navigation", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });
  it("opens the production Router page and navigates from its Agent action", async () => {
    render(<App api={createMockApi()} />);

    expect(screen.getByText("CR")).toBeInTheDocument();
    expect(await screen.findByText("路由未启动")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "前往 Agent 配置" }));

    expect(
      screen.getByRole("heading", { name: "Agent 配置" }),
    ).toBeInTheDocument();
    expect(screen.getByText("模型配置工作台")).toBeInTheDocument();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Agent 检测失败",
    );
  });

  it("opens runtime logs from startup failure diagnostics", async () => {
    const api = createMockApi({
      getRouterStatus: vi.fn().mockResolvedValue({
        state: "start_failed",
        last_error: "router startup failed",
        recent_logs: ["startup diagnostic"],
      }),
      getRouterLogs: vi.fn().mockResolvedValue({
        lines: ["full runtime log line"],
      }),
    });
    render(<App api={api} />);

    fireEvent.click(
      await screen.findByRole("button", { name: "查看运行日志" }),
    );

    expect(
      screen.getByRole("heading", { name: "运行日志" }),
    ).toBeInTheDocument();
    expect(
      await screen.findByText("full runtime log line"),
    ).toBeInTheDocument();
  });

  it("keeps Settings available after Agent navigation is integrated", async () => {
    const api = createMockApi();
    render(<App api={api} />);
    await screen.findByText("路由未启动");

    fireEvent.click(screen.getByRole("button", { name: /系统设置/ }));

    expect(
      screen.getByRole("heading", { name: "系统设置" }),
    ).toBeInTheDocument();
    expect(screen.getByText("桌面控制面板")).toBeInTheDocument();
  });

  it("opens API key management from the main navigation", async () => {
    render(
      <App
        api={createMockApi({
          getCredential: vi.fn().mockResolvedValue({
            present: false,
            fingerprint: "",
            saved_at: null,
          }),
        })}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "API 密钥" }));

    expect(
      screen.getByRole("heading", { name: "API 密钥" }),
    ).toBeInTheDocument();
    expect(await screen.findByText("尚未配置")).toBeInTheDocument();
  });

  it("opens API key management from an Agent credential error", async () => {
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
      discoverModels: vi.fn().mockRejectedValue({
        code: "CREDENTIAL_NOT_FOUND",
        message: "credential is not configured",
      }),
    });
    render(<App api={api} />);

    fireEvent.click(screen.getByRole("button", { name: "Agent 配置" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "编辑 Claude Code 配置" }),
    );
    fireEvent.click(
      await screen.findByRole("button", { name: "前往 API 密钥" }),
    );

    expect(screen.getByRole("heading", { name: "API 密钥" })).toBeVisible();
  });

  it("writes configuration for an uninstalled Agent and keeps the install-later guidance", async () => {
    const uninstalledDetection: AgentDetection = {
      agents: detection.agents.map((agent) =>
        agent.agent === "claude"
          ? {
              ...agent,
              command: "",
              exists: false,
              configured: false,
            }
          : agent,
      ),
    };
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(uninstalledDetection),
      discoverModels: vi.fn().mockResolvedValue({
        flow_id: "flow-uninstalled-claude",
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
      }),
      previewAgents: vi.fn().mockResolvedValue({
        revision_token: "revision",
        model_config: {
          version: 1,
          claude: {
            primary: { model: "model-a" },
            haiku: { inherit_primary: true },
            sonnet: { inherit_primary: true },
            opus: { inherit_primary: true },
          },
        },
        fragments: [
          {
            agent: "claude",
            role: "config",
            path: "/safe/claude/settings.json",
            format: "json",
            content: "{}",
          },
        ],
        files: [
          {
            agent: "claude",
            mode: "merge",
            path: "/safe/claude/settings.json",
            role: "config",
            format: "json",
            operation: "replace",
          },
        ],
        managed_config_drift: false,
        drifted_agents: [],
        managed_collisions: [],
        requires_codex_auth_approval: false,
      }),
      writeAgents: vi.fn().mockResolvedValue({
        transaction_id: "tx-uninstalled-claude",
        agents: [{ agent: "claude", success: true }],
      }),
    });
    render(<App api={api} />);

    fireEvent.click(screen.getByRole("button", { name: "Agent 配置" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "生成 Claude Code 配置" }),
    );
    fireEvent.change(await screen.findByLabelText("主模型"), {
      target: { value: "model-a" },
    });
    fireEvent.click(screen.getByRole("button", { name: "生成写入预览" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "写入所选 Agent" }),
    );

    expect(await screen.findByText("成功")).toBeVisible();
    expect(screen.getByRole("note")).toHaveTextContent(
      "配置已生成；安装 Claude Code 后即可使用。",
    );
    expect(api.writeAgents).toHaveBeenCalledOnce();
  });

  it("uses document visibility only to coordinate native polling", () => {
    const api = createMockApi();
    render(<App api={api} />);

    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "hidden",
    });
    fireEvent(document, new Event("visibilitychange"));

    expect(vi.mocked(api.setWindowVisibility)).toHaveBeenLastCalledWith(false);
    expect(api.destroyAgentModelFlow).not.toHaveBeenCalled();
  });

  it("collapses the sidebar to icons and persists the preference", async () => {
    const { container, unmount } = render(<App api={createMockApi()} />);
    const frame = container.querySelector(".app-frame");
    expect(frame).toHaveAttribute("data-sidebar", "expanded");

    fireEvent.click(screen.getByRole("button", { name: "收起侧栏" }));

    expect(frame).toHaveAttribute("data-sidebar", "collapsed");
    expect(window.localStorage.getItem("mtls-router.sidebar.collapsed")).toBe(
      "1",
    );
    unmount();

    const remounted = render(<App api={createMockApi()} />);
    expect(remounted.container.querySelector(".app-frame")).toHaveAttribute(
      "data-sidebar",
      "collapsed",
    );

    fireEvent.click(screen.getByRole("button", { name: "展开侧栏" }));
    expect(remounted.container.querySelector(".app-frame")).toHaveAttribute(
      "data-sidebar",
      "expanded",
    );
    expect(window.localStorage.getItem("mtls-router.sidebar.collapsed")).toBe(
      "0",
    );
  });

  it("guards sidebar navigation, cancels without leaving, and restores focus", async () => {
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
      discoverModels: vi.fn().mockResolvedValue(agentDiscovery),
    });
    render(<App api={api} />);
    await openDirtyAgent(api);

    const settings = screen.getByRole("button", { name: "系统设置" });
    fireEvent.click(settings);
    expect(screen.getByRole("dialog")).toHaveAccessibleName("放弃更改并离开？");
    fireEvent.click(screen.getByRole("button", { name: "继续编辑" }));

    expect(
      screen.getByRole("heading", { level: 2, name: "Claude Code" }),
    ).toBeVisible();
    expect(settings).toHaveFocus();

    fireEvent.click(settings);
    fireEvent.click(screen.getByRole("button", { name: "放弃并离开" }));
    const settingsHeading = screen.getByRole("heading", {
      level: 1,
      name: "系统设置",
    });
    expect(settingsHeading).toBeVisible();
    expect(settingsHeading).toHaveFocus();
    await waitFor(() =>
      expect(api.setAgentDraftDirty).toHaveBeenLastCalledWith(false),
    );
  });

  it("uses one accessible dialog to resolve native quit cancel and confirm", async () => {
    const quitRequest = { current: null as (() => void) | null };
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
      discoverModels: vi.fn().mockResolvedValue(agentDiscovery),
      subscribeAgentDraftQuitRequested: vi.fn(async (listener) => {
        quitRequest.current = listener;
        return () => undefined;
      }),
    });
    render(<App api={api} />);
    await openDirtyAgent(api);

    act(() => {
      quitRequest.current?.();
      quitRequest.current?.();
    });
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(api.resolveAppQuit).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "放弃并离开" }));
    expect(api.resolveAppQuit).toHaveBeenLastCalledWith(true);

    act(() => quitRequest.current?.());
    fireEvent.click(screen.getByRole("button", { name: "继续编辑" }));
    expect(api.resolveAppQuit).toHaveBeenLastCalledWith(false);
  });

  it("routes clean-panel native quit through the live guard without a dialog", async () => {
    const quitRequest = { current: null as (() => void) | null };
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
      discoverModels: vi.fn().mockResolvedValue(agentDiscovery),
      subscribeAgentDraftQuitRequested: vi.fn(async (listener) => {
        quitRequest.current = listener;
        return () => undefined;
      }),
    });
    render(<App api={api} />);
    fireEvent.click(screen.getByRole("button", { name: "Agent 配置" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "编辑 Claude Code 配置" }),
    );
    await screen.findByLabelText("主模型");
    await waitFor(() =>
      expect(api.setAgentDraftDirty).toHaveBeenLastCalledWith(true),
    );

    act(() => quitRequest.current?.());

    expect(api.resolveAppQuit).toHaveBeenLastCalledWith(true);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("does nothing when the active Agent section is selected again", async () => {
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
      discoverModels: vi.fn().mockResolvedValue(agentDiscovery),
    });
    render(<App api={api} />);
    await openDirtyAgent(api);

    fireEvent.click(screen.getByRole("button", { name: "Agent 配置" }));

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(
      screen.getByRole("heading", { level: 2, name: "Claude Code" }),
    ).toBeVisible();
  });

  it("blocks Back, sidebar, and native quit while preview is loading", async () => {
    const pendingPreview =
      deferred<
        Awaited<ReturnType<ReturnType<typeof createMockApi>["previewAgents"]>>
      >();
    const quitRequest = { current: null as (() => void) | null };
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
      discoverModels: vi.fn().mockResolvedValue(agentDiscovery),
      previewAgents: vi.fn(() => pendingPreview.promise),
      subscribeAgentDraftQuitRequested: vi.fn(async (listener) => {
        quitRequest.current = listener;
        return () => undefined;
      }),
    });
    render(<App api={api} />);
    fireEvent.click(screen.getByRole("button", { name: "Agent 配置" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "编辑 Claude Code 配置" }),
    );
    fireEvent.click(
      await screen.findByRole("button", { name: "生成写入预览" }),
    );
    await waitFor(() =>
      expect(api.setAgentDraftDirty).toHaveBeenLastCalledWith(true),
    );

    expect(
      screen.getByRole("button", { name: "返回 Agent 概览" }),
    ).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "系统设置" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(
      screen.getByRole("heading", { level: 2, name: "Claude Code" }),
    ).toBeVisible();

    act(() => quitRequest.current?.());
    expect(api.resolveAppQuit).toHaveBeenLastCalledWith(false);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    await act(async () =>
      pendingPreview.resolve({
        revision_token: "revision-busy",
        model_config: {
          ...agentDiscovery.existing.model_config,
          version: 1,
        },
        fragments: [],
        files: [],
        managed_config_drift: false,
        drifted_agents: [],
        managed_collisions: [],
        requires_codex_auth_approval: false,
      }),
    );
  });
});
