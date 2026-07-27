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
    },
  ],
};

const callbacks = () => ({
  onRefresh: vi.fn(),
  onConfigure: vi.fn(),
  onRetry: vi.fn(),
  onNavigateToApiKeys: vi.fn(),
});

describe("AgentOverview", () => {
  it("renders fixed-order cards with independent installation and configuration states", () => {
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
    expect(screen.getByText("CLI 未安装")).toBeVisible();
    expect(screen.getByText("等待创建")).toBeVisible();
    expect(screen.getAllByText("CLI 已安装")).toHaveLength(2);
    expect(screen.getByText("配置无效")).toBeVisible();
    expect(screen.getByTitle("/safe/claude/settings.json")).toBeVisible();
    expect(screen.getByText("配置语法无效。")).toBeVisible();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();

    const installationStates = screen.getAllByLabelText(/^CLI:/);
    const configurationStates = screen.getAllByLabelText(/^配置:/);
    expect(installationStates).toHaveLength(3);
    expect(configurationStates).toHaveLength(3);
    expect(installationStates[0]).toHaveClass(
      "agent-state--installation",
      "agent-state--not-installed",
    );
    expect(configurationStates[0]).toHaveClass(
      "agent-state--configuration",
      "agent-state--create",
    );
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
      installedAtEntry: false,
    });
    fireEvent.click(
      screen.getByRole("button", { name: "备份并重建 OpenCode" }),
    );
    expect(props.onConfigure).toHaveBeenLastCalledWith({
      agent: "opencode",
      mode: "rebuild",
      installedAtEntry: true,
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

  it.each([
    ["CREDENTIAL_NOT_FOUND", "credential", "前往 API 密钥"],
    ["CREDENTIAL_INVALID", "credential", "前往 API 密钥"],
    ["CREDENTIAL_IO_ERROR", "credential", "前往 API 密钥"],
    ["CREDENTIAL_LOCK_TIMEOUT", "credential", "前往 API 密钥"],
    ["MODEL_AUTH_FAILED", "auth", "更换 API 密钥"],
  ] as const)("routes %s to API key management", (code, kind, action) => {
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
    expect(alert).toHaveTextContent(
      kind === "auth"
        ? "已保存的 API key 未通过模型服务认证"
        : "尚未保存可用的 API key",
    );
    fireEvent.click(within(alert).getByRole("button", { name: action }));
    expect(props.onNavigateToApiKeys).toHaveBeenCalledOnce();
  });

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
      installedAtEntry: true,
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
