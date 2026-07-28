import { fireEvent, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { AgentPage, type LeaveGuard } from "./AgentPage";
import type { AgentDetection } from "./ipc";
import { createMockApi } from "./test/api";
import { renderWithI18n } from "./test/render";

vi.mock("./AgentPanel", () => ({
  AgentPanel: ({
    target,
    onBack,
    onGuardStateChange,
    onReloaded,
  }: {
    target: string;
    onBack(): void;
    onGuardStateChange(state: { dirty: boolean; busy: boolean }): void;
    onReloaded(detection: AgentDetection): void;
  }) => (
    <div aria-label={`panel-${target}`}>
      <button onClick={() => onGuardStateChange({ dirty: true, busy: false })}>
        make dirty
      </button>
      <button onClick={() => onGuardStateChange({ dirty: false, busy: true })}>
        make busy
      </button>
      <button
        onClick={() =>
          onReloaded({
            agents: detection.agents.map((agent) => ({
              ...agent,
              path: `/reloaded/${agent.agent}`,
            })),
          })
        }
      >
        report reload
      </button>
      <button onClick={onBack}>panel back</button>
    </div>
  ),
}));

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

function setup() {
  const api = createMockApi({
    detectAgents: vi.fn().mockResolvedValue(detection),
  });
  let guard: LeaveGuard | null = null;
  const onRequestLeave = vi.fn((action: () => void) => action());
  const onDirtyChange = vi.fn();
  renderWithI18n(
    <AgentPage
      api={api}
      onNavigateToApiKeys={vi.fn()}
      onRequestLeave={onRequestLeave}
      onDirtyChange={onDirtyChange}
      registerLeaveGuard={(next) => {
        guard = next;
      }}
    />,
  );
  return { api, onRequestLeave, onDirtyChange, getGuard: () => guard };
}

describe("AgentPage", () => {
  it("shows overview detection status without decorative protocol badges", async () => {
    const pending = new Promise<AgentDetection>(() => undefined);
    const api = createMockApi({
      detectAgents: vi.fn(() => pending),
    });
    const { container } = renderWithI18n(
      <AgentPage
        api={api}
        onNavigateToApiKeys={vi.fn()}
        onRequestLeave={(action) => action()}
        onDirtyChange={vi.fn()}
        registerLeaveGuard={() => undefined}
      />,
    );

    const status = screen.getByRole("status");
    expect(status).toHaveTextContent("检测中...");
    expect(status.textContent).not.toMatch(/\b(?:GET|TX)\b/);
    expect(container.querySelector(".instrument__dial")).toBeNull();
  });

  it("keeps the overview detection-only and lets AgentPanel own discovery", async () => {
    const { api } = setup();
    await screen.findByText("/safe/claude/config");
    expect(api.discoverModels).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("button", { name: /编辑 Claude Code 配置/ }),
    );
    expect(screen.getByLabelText("panel-claude")).toBeVisible();
    expect(api.discoverModels).not.toHaveBeenCalled();
  });

  it("registers panel dirtiness as the leave guard", async () => {
    const { getGuard, onDirtyChange } = setup();
    fireEvent.click(
      await screen.findByRole("button", { name: /编辑 Claude Code 配置/ }),
    );
    expect(getGuard()?.()).toBe("allow");
    fireEvent.click(screen.getByRole("button", { name: "make dirty" }));
    expect(getGuard()?.()).toBe("confirm");
    expect(onDirtyChange).toHaveBeenLastCalledWith(true);
    fireEvent.click(screen.getByRole("button", { name: "make busy" }));
    expect(getGuard()?.()).toBe("block");
  });

  it("routes Back through the guard and restores focus to the card action", async () => {
    const { onRequestLeave } = setup();
    fireEvent.click(
      await screen.findByRole("button", { name: /编辑 Claude Code 配置/ }),
    );
    fireEvent.click(screen.getByRole("button", { name: "panel back" }));

    expect(onRequestLeave).toHaveBeenCalledOnce();
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /编辑 Claude Code 配置/ }),
      ).toHaveFocus(),
    );
  });

  it("uses successful panel reload detection when returning to overview", async () => {
    setup();
    fireEvent.click(
      await screen.findByRole("button", { name: /编辑 Claude Code 配置/ }),
    );
    fireEvent.click(screen.getByRole("button", { name: "report reload" }));
    fireEvent.click(screen.getByRole("button", { name: "panel back" }));

    expect(await screen.findByText("/reloaded/claude")).toBeVisible();
  });
});
