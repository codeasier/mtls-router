import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { AgentOverview } from "./AgentOverview";
import type { AgentDetection } from "./ipc";

const detection: AgentDetection = {
  agents: [
    {
      agent: "codex",
      name: "Codex",
      detected: true,
      command: "/safe/bin/codex",
      path: "/safe/codex/config.toml",
      format: "toml",
      exists: true,
      writable: false,
      configured: true,
      invalid: false,
      recovery: { eligible: false, files: [] },
      cleanup: { managed: false, available: false, reason: "not_managed" },
    },
    {
      agent: "claude",
      name: "Claude Code",
      detected: true,
      command: "",
      path: "/safe/claude/settings.json",
      format: "json",
      exists: false,
      writable: true,
      configured: false,
      invalid: false,
      recovery: { eligible: false, files: [] },
      cleanup: { managed: false, available: false, reason: "not_managed" },
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
      invalid: true,
      recovery: {
        eligible: true,
        reasons: ["syntax_invalid"],
        files: [],
      },
      cleanup: { managed: false, available: false, reason: "not_managed" },
    },
  ],
};

const callbacks = () => ({
  onRefresh: vi.fn(),
  onConfigure: vi.fn(),
  onCleanup: vi.fn(),
  onRetry: vi.fn(),
  onNavigateToApiKeys: vi.fn(),
});

describe("AgentOverview", () => {
  it("renders fixed-order cards with configuration states only", () => {
    render(
      <AgentOverview
        detection={detection}
        refreshing={false}
        stale={false}
        issue={null}
        {...callbacks()}
      />,
    );

    const cardList = screen.getByRole("list", {
      name: /模型配置工作台|Model configuration workbench/,
    });
    expect(
      Array.from(cardList.children).map((card) => card.textContent),
    ).toEqual([
      expect.stringContaining("Claude Code"),
      expect.stringContaining("OpenCode"),
      expect.stringContaining("Codex"),
    ]);
    expect(screen.getByText("等待创建")).toBeVisible();
    expect(screen.getByText("配置无效")).toBeVisible();
    expect(screen.getByTitle("/safe/claude/settings.json")).toBeVisible();
    expect(screen.getByText("配置语法无效。")).toBeVisible();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();

    const configurationStates = screen.getAllByLabelText(/^配置:/);
    expect(configurationStates).toHaveLength(3);
    expect(screen.queryByText(/CLI (?:已安装|未安装)/)).not.toBeInTheDocument();
    expect(configurationStates[0]).toHaveClass(
      "agent-state--configuration",
      "agent-state--create",
    );
  });

  it("promotes configured edit as the primary action and keeps the path in details", () => {
    const configured: AgentDetection = {
      agents: detection.agents.map((agent) =>
        agent.agent === "claude"
          ? {
              ...agent,
              command: "/safe/bin/claude",
              exists: true,
              writable: true,
              configured: true,
              invalid: false,
            }
          : agent,
      ),
    };
    render(
      <AgentOverview
        detection={configured}
        refreshing={false}
        stale={false}
        issue={null}
        {...callbacks()}
      />,
    );

    const card = screen
      .getByRole("heading", { name: "Claude Code" })
      .closest("li");
    expect(card).not.toBeNull();
    expect(within(card!).getByText("已配置")).toBeVisible();
    expect(
      within(card!).getByRole("button", { name: "编辑 Claude Code 配置" }),
    ).toHaveClass("control-button");
    expect(within(card!).getByText("路径与诊断详情")).toBeVisible();
    expect(
      within(card!).getByTitle("/safe/claude/settings.json"),
    ).toBeVisible();
  });

  it("keeps readonly cards disabled with path details still available", () => {
    render(
      <AgentOverview
        detection={detection}
        refreshing={false}
        stale={false}
        issue={null}
        {...callbacks()}
      />,
    );

    const card = screen.getByRole("heading", { name: "Codex" }).closest("li");
    expect(card).not.toBeNull();
    expect(within(card!).getByText("不可写")).toBeVisible();
    expect(within(card!).getByRole("button", { name: /Codex/ })).toBeDisabled();
    expect(within(card!).getByTitle("/safe/codex/config.toml")).toBeVisible();
  });

  it("emits one immutable target per card action and disables blocked cards", () => {
    const props = callbacks();
    render(
      <AgentOverview
        detection={detection}
        refreshing={false}
        stale={false}
        issue={null}
        {...props}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", { name: "生成 Claude Code 配置" }),
    );
    expect(props.onConfigure).toHaveBeenCalledWith({
      agent: "claude",
      mode: "merge",
    });
    fireEvent.click(
      screen.getByRole("button", { name: "备份并重建 OpenCode" }),
    );
    expect(props.onConfigure).toHaveBeenLastCalledWith({
      agent: "opencode",
      mode: "rebuild",
    });
    expect(screen.getByRole("button", { name: /Codex/ })).toBeDisabled();
  });

  it("marks retained data stale and exposes refresh state and action", () => {
    const props = callbacks();
    const view = render(
      <AgentOverview
        detection={detection}
        refreshing={false}
        stale
        issue={null}
        {...props}
      />,
    );

    expect(screen.getByRole("note")).toHaveTextContent("状态可能已过期");
    fireEvent.click(screen.getByRole("button", { name: "重新检测" }));
    expect(props.onRefresh).toHaveBeenCalledOnce();

    view.rerender(
      <AgentOverview
        detection={detection}
        refreshing
        stale={false}
        issue={null}
        {...props}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("正在重新检测");
    expect(screen.getByRole("button", { name: "正在重新检测" })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Claude Code/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /OpenCode/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Codex/ })).toBeDisabled();
    expect(
      screen.getByRole("list", {
        name: /模型配置工作台|Model configuration workbench/,
      }),
    ).toHaveAttribute("aria-busy", "true");
  });

  it("offers cleanup only for managed available Agents", () => {
    const props = callbacks();
    const managed: AgentDetection = {
      agents: detection.agents.map((agent) =>
        agent.agent === "opencode"
          ? {
              ...agent,
              cleanup: { managed: true, available: true, reason: null },
            }
          : agent,
      ),
    };
    const view = render(
      <AgentOverview
        detection={managed}
        refreshing={false}
        stale={false}
        issue={null}
        {...props}
      />,
    );

    const cleanup = screen.getByRole("button", {
      name: "清理 OpenCode 托管配置",
    });
    expect(cleanup).toHaveAttribute("id", "agent-opencode-cleanup");
    fireEvent.click(cleanup);
    expect(props.onCleanup).toHaveBeenCalledWith("opencode");
    expect(screen.queryByText(/Claude Code 托管配置/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Codex 托管配置/)).not.toBeInTheDocument();

    view.rerender(
      <AgentOverview
        detection={managed}
        refreshing
        stale={false}
        issue={null}
        {...props}
      />,
    );
    expect(document.getElementById("agent-opencode-action")).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "清理 OpenCode 托管配置" }),
    ).toBeDisabled();
  });

  it.each([
    ["model_state_invalid", "无法验证桌面端保存的 Agent 托管状态"],
    ["writes_disabled", "Agent 配置写入当前不可用"],
  ])("shows an actionable cleanup diagnostic for %s", (reason, message) => {
    const unavailable: AgentDetection = {
      agents: detection.agents.map((agent) =>
        agent.agent === "opencode"
          ? {
              ...agent,
              cleanup: { managed: true, available: false, reason },
            }
          : agent,
      ),
    };
    render(
      <AgentOverview
        detection={unavailable}
        refreshing={false}
        stale={false}
        issue={null}
        {...callbacks()}
      />,
    );

    expect(screen.getByText(new RegExp(message))).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "清理 OpenCode 托管配置" }),
    ).not.toBeInTheDocument();
  });

  it.each([
    ["CREDENTIAL_NOT_FOUND", "credential", "尚未保存 API key", "前往 API 密钥"],
    [
      "CREDENTIAL_INVALID",
      "credential",
      "已保存的 API key 格式无效，请重新保存",
      "前往 API 密钥",
    ],
    [
      "CREDENTIAL_IO_ERROR",
      "credential",
      "凭据存储暂时不可用",
      "前往 API 密钥",
    ],
    [
      "CREDENTIAL_LOCK_TIMEOUT",
      "credential",
      "凭据存储正在执行其他操作",
      "前往 API 密钥",
    ],
    [
      "MODEL_AUTH_FAILED",
      "auth",
      "已保存的 API key 未通过模型服务认证",
      "更换 API 密钥",
    ],
  ] as const)(
    "routes %s to API key management",
    (code, kind, message, action) => {
      const props = callbacks();
      render(
        <AgentOverview
          detection={detection}
          refreshing={false}
          stale={false}
          issue={{ kind, code }}
          {...props}
        />,
      );

      const alert = screen.getByRole("alert");
      expect(alert).toHaveClass("agent-overview__error");
      expect(alert).toHaveTextContent(message);
      fireEvent.click(within(alert).getByRole("button", { name: action }));
      expect(props.onNavigateToApiKeys).toHaveBeenCalledOnce();
    },
  );

  it.each([
    ["MODEL_DISCOVERY_FAILED", "暂时无法取得模型目录"],
    ["MODEL_RESPONSE_INVALID", "暂时无法取得模型目录"],
    ["MODEL_CATALOG_EMPTY", "暂时无法取得模型目录"],
    ["OPERATION_TIMEOUT", "暂时无法取得模型目录"],
    ["AGENT_OPERATION_BUSY", "暂时无法取得模型目录"],
    ["MANAGER_FAILED", "本地 manager 暂时不可用"],
    ["SIDECAR_MISSING", "本地 manager 暂时不可用"],
    ["UNKNOWN_BACKEND_CODE", "无法开始配置（UNKNOWN_BACKEND_CODE）"],
  ])("renders a stable action for %s", (code, message) => {
    const props = callbacks();
    const target = {
      agent: "opencode" as const,
      mode: "rebuild" as const,
    };
    render(
      <AgentOverview
        detection={detection}
        refreshing={false}
        stale={false}
        issue={{ kind: "retry", code, target }}
        {...props}
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent(message);
    fireEvent.click(screen.getByRole("button", { name: "重试 OpenCode" }));
    expect(props.onRetry).toHaveBeenCalledWith(target);
  });
});
