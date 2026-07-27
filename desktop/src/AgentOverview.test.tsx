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

    expect(
      screen.getAllByRole("article").map((card) => card.textContent),
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
  });

  it("routes credential and retry issues to their explicit actions", () => {
    const props = callbacks();
    const target = {
      agent: "opencode" as const,
      mode: "rebuild" as const,
      installedAtEntry: true,
    };
    const view = render(
      <AgentOverview
        detection={detection}
        refreshing={false}
        stale={false}
        issue={{ kind: "credential", code: "CREDENTIAL_NOT_FOUND" }}
        {...props}
      />,
    );

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("尚未保存可用的 API key");
    fireEvent.click(
      within(alert).getByRole("button", { name: "前往 API 密钥" }),
    );
    expect(props.onNavigateToApiKeys).toHaveBeenCalledOnce();

    view.rerender(
      <AgentOverview
        detection={detection}
        refreshing={false}
        stale={false}
        issue={{ kind: "retry", code: "MODEL_DISCOVERY_FAILED", target }}
        {...props}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "重试 OpenCode" }));
    expect(props.onRetry).toHaveBeenCalledWith(target);
  });
});
