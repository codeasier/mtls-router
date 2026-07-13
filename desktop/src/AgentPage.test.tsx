import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { AgentPage } from "./AgentPage";
import type { AgentDetection, AgentPreview, AgentWriteResult } from "./ipc";
import { createMockApi } from "./test/api";

const detection: AgentDetection = {
  agents: [
    {
      agent: "claude",
      name: "Claude Code",
      detected: true,
      path: "/home/operator/.claude/settings.json",
      format: "json",
      exists: true,
      writable: true,
      configured: true,
      invalid: false,
    },
    {
      agent: "opencode",
      name: "opencode",
      detected: false,
      path: "/home/operator/.config/opencode/opencode.jsonc",
      format: "jsonc",
      exists: false,
      writable: true,
      configured: false,
      invalid: false,
    },
    {
      agent: "codex",
      name: "Codex",
      detected: true,
      path: "/home/operator/.codex/config.toml",
      auth_path: "/home/operator/.codex/auth.json",
      format: "toml",
      exists: true,
      writable: true,
      configured: false,
      invalid: true,
    },
  ],
};

const apiKeyLabel = /API (?:key|密钥)/;

const structuredPreview: AgentPreview = {
  revision_token: "revision-structured",
  agents: [
    {
      agent: "opencode",
      name: "opencode",
      files: [
        {
          path: "/home/operator/.config/opencode/opencode.json",
          source_path: "/home/operator/.config/opencode/opencode.jsonc",
          format: "json",
          operation: "replace",
          operations: ["replace", "preserve"],
          contains_api_key: false,
          preserves: ["其他 providers", "根级设置"],
          backup: {
            required: true,
            pattern:
              "/home/operator/.config/opencode/opencode.jsonc.bak-<timestamp>-<random>",
            sensitive: true,
            warning: "backup may contain a previous key",
          },
          warning: "comments and formatting are not preserved",
        },
      ],
    },
    {
      agent: "codex",
      name: "Codex",
      files: [
        {
          path: "/home/operator/.codex/config.toml",
          format: "toml",
          operation: "replace",
          operations: ["replace", "preserve"],
          contains_api_key: false,
          preserves: ["unrelated root keys", "unmanaged sections"],
          backup: {
            required: true,
            pattern:
              "/home/operator/.codex/config.toml.bak-<timestamp>-<random>",
            sensitive: true,
          },
        },
        {
          path: "/home/operator/.codex/auth.json",
          format: "json",
          operation: "create",
          operations: ["create"],
          contains_api_key: true,
          backup: { required: false, sensitive: false },
        },
      ],
    },
  ],
};

const writeResult: AgentWriteResult = {
  transaction_id: "transaction-1",
  sensitive_files: true,
  warning: "backups are sensitive",
  agents: [
    {
      agent: "opencode",
      success: true,
      files: [],
      changed: ["/home/operator/.config/opencode/opencode.json"],
      backups: [
        "/home/operator/.config/opencode/opencode.jsonc.bak-20260712-safe",
      ],
    },
    {
      agent: "codex",
      success: true,
      files: [],
      changed: [
        "/home/operator/.codex/config.toml",
        "/home/operator/.codex/auth.json",
      ],
      backups: ["/home/operator/.codex/config.toml.bak-20260712-safe"],
    },
  ],
};

function selectableDetection(): AgentDetection {
  return {
    agents: detection.agents.map((agent) => ({
      ...agent,
      detected: true,
      writable: true,
      invalid: false,
    })),
  };
}

async function reachKeyInput(
  preview = structuredPreview,
  selectedDetection = selectableDetection(),
) {
  const api = createMockApi({
    detectAgents: vi.fn().mockResolvedValue(selectedDetection),
    previewAgents: vi.fn().mockResolvedValue(preview),
    writeAgents: vi.fn().mockResolvedValue(writeResult),
  });
  const view = render(<AgentPage api={api} />);
  await screen.findByRole("button", { name: "生成写入预览" });
  fireEvent.click(screen.getByRole("button", { name: "生成写入预览" }));
  await screen.findByText("确认变更范围");
  fireEvent.click(screen.getByRole("button", { name: "我已审阅并批准" }));
  return { api, unmount: view.unmount };
}

describe("Agent detection", () => {
  it("shows paths, formats, writability and state without selecting absent or invalid Agents", async () => {
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
    });
    render(<AgentPage api={api} />);

    expect(
      await screen.findByText("/home/operator/.claude/settings.json"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("/home/operator/.codex/auth.json"),
    ).toBeInTheDocument();
    expect(screen.getAllByText("JSON").length).toBeGreaterThan(0);
    expect(screen.getByText("JSONC")).toBeInTheDocument();
    expect(screen.getByText("TOML")).toBeInTheDocument();
    expect(screen.getByText("未检测到")).toBeInTheDocument();
    expect(screen.getByText("配置无效")).toBeInTheDocument();
    expect(
      screen.getByText(
        (text) => text.includes("config.toml") && text.includes("TOML 语法"),
      ),
    ).toBeInTheDocument();

    const selectors = screen.getAllByRole("checkbox");
    expect(selectors[0]).toBeChecked();
    expect(selectors[1]).not.toBeChecked();
    expect(selectors[1]).toBeDisabled();
    expect(selectors[2]).not.toBeChecked();
    expect(selectors[2]).toBeDisabled();
  });

  it("refreshes detection and recomputes safe default selection", async () => {
    const detectAgents = vi
      .fn()
      .mockResolvedValueOnce(detection)
      .mockResolvedValueOnce(selectableDetection());
    render(<AgentPage api={createMockApi({ detectAgents })} />);
    await screen.findByText("未检测到");

    fireEvent.click(screen.getByRole("button", { name: "刷新检测" }));

    await waitFor(() => expect(detectAgents).toHaveBeenCalledTimes(2));
    await waitFor(() =>
      expect(
        screen
          .getAllByRole("checkbox")
          .every((item) => item.matches(":checked")),
      ).toBe(true),
    );
  });
});

describe("Agent preview and approval", () => {
  it("renders operations, sensitive backups, JSONC migration, and Codex's two files", async () => {
    const previewAgents = vi.fn().mockResolvedValue(structuredPreview);
    render(
      <AgentPage
        api={createMockApi({
          detectAgents: vi.fn().mockResolvedValue(selectableDetection()),
          previewAgents,
        })}
      />,
    );
    await screen.findByRole("button", { name: "生成写入预览" });
    fireEvent.click(screen.getByRole("button", { name: "生成写入预览" }));

    expect(
      await screen.findByText(/JSONC\s*(?:→|->)\s*JSON 迁移/),
    ).toBeInTheDocument();
    expect(screen.getByText(/注释与原格式不会保留/)).toBeInTheDocument();
    expect(
      screen.getByText("/home/operator/.codex/config.toml"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("/home/operator/.codex/auth.json"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/此文件会保存本次提供的 API key/),
    ).toBeInTheDocument();
    expect(document.querySelectorAll(".backup-plan")).toHaveLength(2);
    expect(
      screen.queryByRole("textbox", { name: apiKeyLabel }),
    ).not.toBeInTheDocument();
    expect(previewAgents).toHaveBeenCalledWith(["claude", "opencode", "codex"]);
  });

  it("sanitizes manager-derived strings before rendering them", async () => {
    const secret = "sk-previewBoundaryCanary123456";
    const preview: AgentPreview = {
      ...structuredPreview,
      agents: [
        {
          ...structuredPreview.agents[0],
          files: [
            {
              ...structuredPreview.agents[0].files[0],
              warning: `api_key=${secret}`,
              path: `/tmp/${secret}.json`,
            },
          ],
        },
      ],
    };
    await reachKeyInput(preview);

    expect(document.body.textContent).not.toContain(secret);
    expect(document.body.textContent).toContain("[REDACTED");
  });

  it("returns invalid previews to detection without exposing backend details", async () => {
    const previewAgents = vi.fn().mockRejectedValue({
      code: "CONFIG_INVALID",
      message: "invalid config includes a sensitive backend value",
    });
    render(
      <AgentPage
        api={createMockApi({
          detectAgents: vi.fn().mockResolvedValue(selectableDetection()),
          previewAgents,
        })}
      />,
    );
    fireEvent.click(
      await screen.findByRole("button", { name: "生成写入预览" }),
    );

    expect(
      await screen.findByText(/目标配置无效，未修改任何文件/),
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("sensitive backend value");
    expect(
      screen.getByRole("button", { name: "刷新检测" }),
    ).toBeInTheDocument();
  });
});

describe("transient Agent write", () => {
  it("requires approval, sends the key once, clears it, and renders per-Agent paths", async () => {
    const { api } = await reachKeyInput();
    const secret = "fixture-write-secret-value";
    const field = screen.getByLabelText(apiKeyLabel);
    expect(field).toHaveAttribute("type", "password");
    fireEvent.change(field, { target: { value: secret } });
    expect(field).toHaveValue(secret);

    fireEvent.click(screen.getByRole("button", { name: "写入所选 Agent" }));

    expect(await screen.findByText("Agent 配置结果")).toBeInTheDocument();
    expect(api.writeAgents).toHaveBeenCalledTimes(1);
    expect(api.writeAgents).toHaveBeenCalledWith(
      ["claude", "opencode", "codex"],
      "revision-structured",
      secret,
    );
    expect(screen.queryByDisplayValue(secret)).not.toBeInTheDocument();
    expect(document.body.textContent).not.toContain(secret);
    expect(
      screen.getByText("/home/operator/.codex/auth.json"),
    ).toBeInTheDocument();
    expect(screen.getAllByText(/\.bak-20260712-safe/)).toHaveLength(2);
  });

  it("clears the password on cancellation and starts with an empty field after reapproval", async () => {
    await reachKeyInput();
    fireEvent.change(screen.getByLabelText(apiKeyLabel), {
      target: { value: "cancelled-secret-value" },
    });
    fireEvent.click(screen.getByRole("button", { name: "取消并清除密钥" }));
    fireEvent.click(screen.getByRole("button", { name: "我已审阅并批准" }));

    expect(screen.getByLabelText(apiKeyLabel)).toHaveValue("");
    expect(document.body.textContent).not.toContain("cancelled-secret-value");
  });

  it("does not retain the password after navigation unmounts and remounts the page", async () => {
    const { api, unmount } = await reachKeyInput();
    fireEvent.change(screen.getByLabelText(apiKeyLabel), {
      target: { value: "navigation-secret-value" },
    });
    // App navigation unmounts AgentPage; a new visit always starts from detection.
    unmount();
    render(<AgentPage api={api} />);

    expect(
      await screen.findByRole("button", { name: "生成写入预览" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByDisplayValue("navigation-secret-value"),
    ).not.toBeInTheDocument();
    expect(document.body.textContent).not.toContain("navigation-secret-value");
  });

  it("clears the password after an ordinary write error", async () => {
    const { api } = await reachKeyInput();
    vi.mocked(api.writeAgents).mockRejectedValueOnce({ code: "WRITE_FAILED" });
    fireEvent.change(screen.getByLabelText(apiKeyLabel), {
      target: { value: "error-secret-value" },
    });
    fireEvent.click(screen.getByRole("button", { name: "写入所选 Agent" }));

    expect(
      await screen.findByText(/写入失败，密钥输入已清除/),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText(apiKeyLabel)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "我已审阅并批准" }));
    expect(screen.getByLabelText(apiKeyLabel)).toHaveValue("");
    expect(document.body.textContent).not.toContain("error-secret-value");
  });

  it("refreshes a stale preview and requires approval and key entry again", async () => {
    const refreshed = {
      ...structuredPreview,
      revision_token: "revision-refreshed",
    };
    const { api } = await reachKeyInput();
    vi.mocked(api.writeAgents).mockRejectedValueOnce({ code: "PREVIEW_STALE" });
    vi.mocked(api.previewAgents).mockResolvedValueOnce(refreshed);
    fireEvent.change(screen.getByLabelText(apiKeyLabel), {
      target: { value: "stale-secret-value" },
    });
    fireEvent.click(screen.getByRole("button", { name: "写入所选 Agent" }));

    expect(
      await screen.findByText(/文件已变化，预览已刷新/),
    ).toBeInTheDocument();
    expect(api.previewAgents).toHaveBeenCalledTimes(2);
    expect(screen.queryByLabelText(apiKeyLabel)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "我已审阅并批准" }));
    expect(screen.getByLabelText(apiKeyLabel)).toHaveValue("");
    expect(document.body.textContent).not.toContain("stale-secret-value");
  });

  it("reports rollback and diagnostic backup paths per Agent", async () => {
    const rollbackResult: AgentWriteResult = {
      transaction_id: "transaction-rollback",
      sensitive_files: true,
      warning: "backups are sensitive",
      agents: [
        {
          agent: "claude",
          success: false,
          files: [],
          changed: ["/home/operator/.claude/settings.json"],
          backups: ["/home/operator/.claude/settings.json.bak-original"],
          rollback_backups: [
            "/home/operator/.claude/settings.json.rollback-failed-write",
          ],
          rolled_back: true,
          error_code: "WRITE_FAILED",
        },
      ],
    };
    const { api } = await reachKeyInput();
    vi.mocked(api.writeAgents).mockResolvedValueOnce(rollbackResult);
    fireEvent.change(screen.getByLabelText(apiKeyLabel), {
      target: { value: "rollback-secret-value" },
    });
    fireEvent.click(screen.getByRole("button", { name: "写入所选 Agent" }));

    expect(await screen.findByText("已回滚本次变更")).toBeInTheDocument();
    expect(screen.getByText("错误代码：WRITE_FAILED")).toBeInTheDocument();
    expect(
      screen.getByText("/home/operator/.claude/settings.json.bak-original"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "/home/operator/.claude/settings.json.rollback-failed-write",
      ),
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("rollback-secret-value");
  });
});
