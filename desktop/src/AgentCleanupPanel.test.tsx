import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { AgentCleanupPanel } from "./AgentCleanupPanel";
import type { AgentCleanupPreview, AgentWriteResult } from "./ipc";
import { createMockApi } from "./test/api";
import { renderWithI18n } from "./test/render";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

const preview: AgentCleanupPreview = {
  revision_token: "cleanup-revision",
  agent: "opencode",
  files: [
    {
      path: "/safe/opencode/config.json",
      role: "config",
      format: "json",
      operation: "replace",
      backup_required: true,
      backup_sensitive: true,
      backup_pattern: "/safe/opencode/config.json.bak.*",
    },
    {
      path: "/safe/opencode/auth.json",
      role: "auth",
      format: "json",
      operation: "delete",
      backup_required: true,
      backup_sensitive: true,
      backup_pattern: "/safe/opencode/auth.json.bak.*",
    },
  ],
  removed_paths: ["auth.apiKey", "provider.mtls-router"],
  managed_config_drift: false,
  state_change: {
    path: "/safe/app-data/agent-model-state.json",
    role: "manager_state",
    format: "json",
    operation: "delete",
  },
  state_backup: {
    path: "/safe/app-data/agent-model-state.json.bak.*",
    role: "manager_state_backup",
    format: "json",
    operation: "backup",
    backup_sensitive: false,
  },
};

const result: AgentWriteResult = {
  transaction_id: "cleanup-tx",
  agents: [
    {
      agent: "opencode",
      success: true,
      changed: ["/safe/opencode/config.json", "/safe/opencode/auth.json"],
      backups: [
        "/safe/opencode/config.json.bak.20260731",
        "/safe/opencode/auth.json.bak.20260731",
      ],
    },
  ],
  state_change: {
    path: "/safe/app-data/agent-model-state.json",
    role: "manager_state",
    format: "json",
    operation: "delete",
  },
  state_backup: {
    path: "/safe/app-data/agent-model-state.json.bak.20260731",
    role: "manager_state_backup",
    format: "json",
    operation: "backup",
  },
};

function renderPanel(overrides = {}) {
  const api = createMockApi({
    previewAgentCleanup: vi.fn().mockResolvedValue(preview),
    writeAgentCleanup: vi.fn().mockResolvedValue(result),
    ...overrides,
  });
  const callbacks = {
    onBack: vi.fn(),
    onBusyChange: vi.fn(),
    onComplete: vi.fn(),
  };
  renderWithI18n(
    <AgentCleanupPanel api={api} agent="opencode" {...callbacks} />,
  );
  return { api, ...callbacks };
}

describe("AgentCleanupPanel", () => {
  it("reviews key-free replace/delete effects and retained data", async () => {
    const { api, onBack } = renderPanel();

    expect(await screen.findByText("provider.mtls-router")).toBeVisible();
    expect(screen.getByText("auth.apiKey")).toBeVisible();
    expect(screen.getByText("替换")).toBeVisible();
    expect(screen.getAllByText("删除").length).toBeGreaterThan(0);
    expect(screen.getByText("备份")).toBeVisible();
    expect(screen.getAllByText("SENSITIVE BACKUP")).toHaveLength(2);
    expect(screen.getByText(/Agent 文件中的认证设置会删除/)).toBeVisible();
    expect(screen.getByText(/桌面端保存的全局 API key 不会删除/)).toBeVisible();
    expect(screen.getByText(/历史和新生成的备份都会保留/)).toBeVisible();
    expect(screen.getByText(/部分 Agent 备份可能含凭据/)).toBeVisible();
    expect(screen.getByText("/safe/opencode/config.json.bak.*")).toBeVisible();
    expect(document.body).not.toHaveTextContent(
      "all configuration outside the recorded managed paths",
    );
    expect(document.body).not.toHaveTextContent(
      "Backups may contain credentials or prior managed values",
    );

    const write = screen.getByRole("button", { name: "备份并清理" });
    expect(write).toBeEnabled();
    expect(write).toHaveClass("control-button--danger");
    expect(
      document.querySelector(".cleanup-approval-rail")?.contains(write),
    ).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "返回 Agent 概览" }));
    expect(onBack).toHaveBeenCalledOnce();
    expect(api.getCredential).not.toHaveBeenCalled();
    expect(api.discoverModels).not.toHaveBeenCalled();
  });

  it("requires dedicated drift approval and prevents duplicate writes", async () => {
    const write = deferred<AgentWriteResult>();
    const drifted = { ...preview, managed_config_drift: true };
    const { api } = renderPanel({
      previewAgentCleanup: vi.fn().mockResolvedValue(drifted),
      writeAgentCleanup: vi.fn(() => write.promise),
    });

    const button = await screen.findByRole("button", { name: "备份并清理" });
    expect(button).toBeDisabled();
    fireEvent.click(
      screen.getByRole("checkbox", {
        name: /批准清理已漂移的托管命名空间/,
      }),
    );
    expect(button).toBeEnabled();
    fireEvent.click(button);
    fireEvent.click(button);
    expect(api.writeAgentCleanup).toHaveBeenCalledOnce();
    expect(api.writeAgentCleanup).toHaveBeenCalledWith(
      "opencode",
      "cleanup-revision",
      true,
    );
    expect(
      screen.getByRole("button", { name: "返回 Agent 概览" }),
    ).toBeDisabled();
    write.resolve(result);
    expect(await screen.findByText("清理完成")).toBeVisible();
  });

  it("keeps actual backup paths visible until explicit finish", async () => {
    const { onComplete } = renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: "备份并清理" }));

    const heading = await screen.findByRole("heading", { name: "清理完成" });
    expect(heading).toHaveAttribute("tabindex", "-1");
    await waitFor(() => expect(heading).toHaveFocus());
    expect(
      screen.getByText("/safe/opencode/auth.json.bak.20260731"),
    ).toBeVisible();
    const stateEffect = screen.getByText("manager_state").closest("article");
    expect(stateEffect).not.toBeNull();
    expect(within(stateEffect!).getByText("删除")).toBeVisible();
    expect(onComplete).not.toHaveBeenCalled();
    const finish = screen.getByRole("button", {
      name: "完成并返回 Agent 概览",
    });
    fireEvent.click(finish);
    fireEvent.click(finish);
    expect(onComplete).toHaveBeenCalledOnce();
  });

  it("retries an ordinary write failure with the same preview", async () => {
    const drifted = { ...preview, managed_config_drift: true };
    const api = createMockApi({
      previewAgentCleanup: vi.fn().mockResolvedValue(drifted),
      writeAgentCleanup: vi
        .fn()
        .mockRejectedValueOnce({ code: "BACKUP_FAILED" })
        .mockResolvedValueOnce(result),
    });
    renderWithI18n(
      <AgentCleanupPanel
        api={api}
        agent="opencode"
        onBack={vi.fn()}
        onBusyChange={vi.fn()}
        onComplete={vi.fn()}
      />,
    );
    fireEvent.click(
      await screen.findByRole("checkbox", {
        name: /批准清理已漂移的托管命名空间/,
      }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "备份并清理" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("BACKUP_FAILED");
    expect(screen.getByText("provider.mtls-router")).toBeVisible();
    expect(screen.getByText("/safe/opencode/auth.json")).toBeVisible();
    expect(screen.getByText(/桌面端保存的全局 API key 不会删除/)).toBeVisible();
    expect(
      screen.getByRole("checkbox", {
        name: /批准清理已漂移的托管命名空间/,
      }),
    ).toBeChecked();
    fireEvent.click(within(alert).getByRole("button", { name: "重试清理" }));
    expect(await screen.findByText("清理完成")).toBeVisible();
    expect(api.previewAgentCleanup).toHaveBeenCalledOnce();
    expect(api.writeAgentCleanup).toHaveBeenCalledTimes(2);
  });

  it("requires a new preview after PREVIEW_STALE", async () => {
    const drifted = { ...preview, managed_config_drift: true };
    const refreshed = {
      ...preview,
      revision_token: "cleanup-revision-2",
      managed_config_drift: false,
    };
    const api = createMockApi({
      previewAgentCleanup: vi
        .fn()
        .mockResolvedValueOnce(drifted)
        .mockResolvedValueOnce(refreshed),
      writeAgentCleanup: vi.fn().mockRejectedValue({ code: "PREVIEW_STALE" }),
    });
    renderWithI18n(
      <AgentCleanupPanel
        api={api}
        agent="opencode"
        onBack={vi.fn()}
        onBusyChange={vi.fn()}
        onComplete={vi.fn()}
      />,
    );
    fireEvent.click(
      await screen.findByRole("checkbox", {
        name: /批准清理已漂移的托管命名空间/,
      }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "备份并清理" }));
    expect(await screen.findByText(/预览后文件发生变化/)).toBeVisible();
    expect(screen.getByText("provider.mtls-router")).toBeVisible();
    expect(screen.getByText("/safe/opencode/auth.json")).toBeVisible();
    expect(screen.getByText(/历史和新生成的备份都会保留/)).toBeVisible();
    expect(screen.getByText(/部分 Agent 备份可能含凭据/)).toBeVisible();
    expect(
      screen.getByRole("checkbox", {
        name: /批准清理已漂移的托管命名空间/,
      }),
    ).toBeChecked();
    expect(api.previewAgentCleanup).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: "重新预览清理" }));
    await waitFor(() =>
      expect(api.previewAgentCleanup).toHaveBeenCalledTimes(2),
    );
    expect(screen.getByRole("button", { name: "备份并清理" })).toBeEnabled();
  });

  it.each(["INVALID_RESPONSE", "MANAGER_FAILED", "OPERATION_TIMEOUT"])(
    "requires repreview after ambiguous %s without offering direct retry",
    async (code) => {
      const api = createMockApi({
        previewAgentCleanup: vi.fn().mockResolvedValue(preview),
        writeAgentCleanup: vi.fn().mockRejectedValue({ code }),
      });
      renderWithI18n(
        <AgentCleanupPanel
          api={api}
          agent="opencode"
          onBack={vi.fn()}
          onBusyChange={vi.fn()}
          onComplete={vi.fn()}
        />,
      );
      fireEvent.click(
        await screen.findByRole("button", { name: "备份并清理" }),
      );

      expect(await screen.findByRole("alert")).toHaveTextContent(code);
      expect(screen.getByText("provider.mtls-router")).toBeVisible();
      expect(
        screen.getByRole("button", { name: "重新预览清理" }),
      ).toBeVisible();
      expect(
        screen.queryByRole("button", { name: "重试清理" }),
      ).not.toBeInTheDocument();
      expect(api.writeAgentCleanup).toHaveBeenCalledOnce();
    },
  );

  it("retries a failed initial preview", async () => {
    const api = createMockApi({
      previewAgentCleanup: vi
        .fn()
        .mockRejectedValueOnce({ code: "AGENT_OPERATION_BUSY" })
        .mockResolvedValueOnce(preview),
    });
    renderWithI18n(
      <AgentCleanupPanel
        api={api}
        agent="opencode"
        onBack={vi.fn()}
        onBusyChange={vi.fn()}
        onComplete={vi.fn()}
      />,
    );
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("AGENT_OPERATION_BUSY");
    fireEvent.click(within(alert).getByRole("button", { name: "重试预览" }));
    expect(await screen.findByText("provider.mtls-router")).toBeVisible();
  });
});
