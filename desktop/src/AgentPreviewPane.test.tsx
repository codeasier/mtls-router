import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { AgentPreviewPane, validateRebuildPreview } from "./AgentPreviewPane";
import type { AgentPreview, ModelConfig } from "./ipc";
import { renderWithI18n } from "./test/render";

const config: ModelConfig = {
  version: 1,
  claude: {
    primary: { model: "model-a" },
    haiku: { inherit_primary: true },
    sonnet: { inherit_primary: true },
    opus: { inherit_primary: true },
  },
};

function preview(revision: string): AgentPreview {
  return {
    revision_token: revision,
    model_config: config,
    fragments: [],
    files: [],
    managed_config_drift: true,
    drifted_agents: ["claude"],
    managed_collisions: [],
    requires_codex_auth_approval: true,
  };
}

const props = {
  target: { agent: "claude", mode: "merge" } as const,
  result: null,
  busy: false,
  onGenerate: vi.fn(),
  onBackToEdit: vi.fn(),
  onWrite: vi.fn(),
  onCancel: vi.fn(),
  onFinish: vi.fn(),
};

describe("AgentPreviewPane", () => {
  it.each([
    [
      "a merge target with a rebuild effect",
      { agent: "opencode", mode: "merge" } as const,
      { agent: "opencode", mode: "rebuild" } as const,
    ],
    [
      "a rebuild target with a cross-Agent effect",
      { agent: "opencode", mode: "rebuild" } as const,
      { agent: "codex", mode: "rebuild" } as const,
    ],
    [
      "a rebuild target with a mixed merge effect",
      { agent: "opencode", mode: "rebuild" } as const,
      { agent: "opencode", mode: "merge" } as const,
    ],
  ])("fails closed for %s", (_name, target, effect) => {
    const candidate: AgentPreview = {
      ...preview("invalid-rebuild"),
      model_config: {
        version: 1,
        opencode: { default_model: "model-a", models: { "model-a": {} } },
      },
      fragments: [],
      drifted_agents: [],
      managed_config_drift: false,
      requires_codex_auth_approval: false,
      files: [
        {
          ...effect,
          path: "/safe/opencode/config.json",
          role: "config",
          format: "json",
          operation: "replace",
        },
      ],
    };

    expect(validateRebuildPreview(candidate, target)).toBe(false);
  });

  it("fails closed for cross-Agent metadata and Agent-tagged state effects", () => {
    const target = {
      agent: "opencode",
      mode: "rebuild",
    } as const;
    const base: AgentPreview = {
      ...preview("invalid-metadata"),
      model_config: {
        version: 1,
        opencode: { default_model: "model-a", models: { "model-a": {} } },
      },
      fragments: [],
      files: [
        {
          agent: "opencode",
          mode: "rebuild",
          path: "/safe/opencode/config.json",
          role: "config",
          format: "json",
          operation: "replace",
        },
      ],
      drifted_agents: [],
      managed_config_drift: false,
      requires_codex_auth_approval: false,
    };
    const invalid: AgentPreview[] = [
      { ...base, drifted_agents: ["codex" as const] },
      {
        ...base,
        managed_collisions: [
          {
            agent: "codex" as const,
            path: "/safe/codex/config.toml",
            type: "fixed",
            action: "replace",
          },
        ],
      },
      {
        ...base,
        state_change: {
          agent: "codex" as const,
          path: "/safe/manager/state.json",
          role: "state",
          format: "json",
          operation: "replace",
        },
      },
    ];

    invalid.forEach((candidate) =>
      expect(validateRebuildPreview(candidate, target)).toBe(false),
    );
  });

  it("keeps write actions inside the approval rail", () => {
    const view = renderWithI18n(
      <AgentPreviewPane {...props} preview={preview("one")} />,
    );

    const write = screen.getByRole("button", { name: /写入所选 Agent/ });
    expect(
      view.container.querySelector(".approval-rail")?.contains(write),
    ).toBe(true);
  });

  it("collects actual drift and auth approvals for a controller write", () => {
    const onWrite = vi.fn();
    renderWithI18n(
      <AgentPreviewPane
        {...props}
        onWrite={onWrite}
        preview={preview("one")}
      />,
    );
    const approvals = screen.getAllByRole("checkbox");
    approvals.forEach((approval) => fireEvent.click(approval));
    fireEvent.click(screen.getByRole("button", { name: /写入所选 Agent/ }));

    expect(onWrite).toHaveBeenCalledWith({
      managedOverwrite: true,
      codexAuthChange: true,
      rebuild: [],
    });
  });

  it("resets approvals when preview revision identity changes", () => {
    const view = renderWithI18n(
      <AgentPreviewPane {...props} preview={preview("one")} />,
    );
    screen
      .getAllByRole("checkbox")
      .forEach((approval) => fireEvent.click(approval));

    view.rerender(<AgentPreviewPane {...props} preview={preview("two")} />);
    screen
      .getAllByRole("checkbox")
      .forEach((approval) => expect(approval).not.toBeChecked());
  });

  it("renders complete file effects while sanitizing preview content", () => {
    const detailed = preview("effects");
    detailed.fragments = [
      {
        agent: "claude",
        role: "config",
        path: "/safe/claude/config.json",
        format: "json",
        content: '{"api_key":"sk-supersecret123"}',
      },
    ];
    detailed.files = [
      {
        agent: "claude",
        mode: "merge",
        path: "/safe/claude/config.json",
        role: "config",
        format: "json",
        operation: "replace",
        backup_required: true,
        backup_sensitive: true,
        backup_pattern: "/safe/claude/config.json.bak-*",
        backup_path: "/safe/claude/config.json.bak-1",
        preserves: ["theme", "hooks"],
        warning: "review replacement",
      },
    ];
    detailed.state_change = {
      path: "/safe/manager/state.json",
      role: "state",
      format: "json",
      operation: "create",
    };
    detailed.state_backup = {
      path: "/safe/manager/state.json.bak",
      role: "state-backup",
      format: "json",
      operation: "backup",
      backup_sensitive: true,
    };
    renderWithI18n(<AgentPreviewPane {...props} preview={detailed} />);

    expect(screen.getAllByText("/safe/claude/config.json")).toHaveLength(2);
    for (const text of [
      "/safe/claude/config.json.bak-*",
      "/safe/claude/config.json.bak-1",
      "/safe/manager/state.json",
      "/safe/manager/state.json.bak",
      "theme, hooks",
      "review replacement",
    ]) {
      expect(screen.getByText(text)).toBeVisible();
    }
    expect(document.body).not.toHaveTextContent("sk-supersecret123");
    expect(document.body).toHaveTextContent("[REDACTED KEY]");
  });

  it("traps rebuild confirmation focus and submits the exact target", () => {
    const onWrite = vi.fn();
    const rebuildPreview: AgentPreview = {
      revision_token: "rebuild",
      model_config: {
        version: 1,
        opencode: { default_model: "model-a", models: { "model-a": {} } },
      },
      fragments: [],
      files: [
        {
          agent: "opencode",
          mode: "rebuild",
          path: "/safe/opencode/config.json",
          role: "config",
          format: "json",
          operation: "replace",
        },
      ],
      managed_config_drift: false,
      drifted_agents: [],
      managed_collisions: [],
      requires_codex_auth_approval: false,
    };
    renderWithI18n(
      <AgentPreviewPane
        {...props}
        target={{
          agent: "opencode",
          mode: "rebuild",
        }}
        preview={rebuildPreview}
        onWrite={onWrite}
      />,
    );
    const trigger = screen.getByRole("button", { name: /写入所选 Agent/ });
    fireEvent.click(trigger);
    const dialog = screen.getByRole("dialog");
    const cancel = screen.getByRole("button", { name: /^取消$/ });
    const confirm = screen.getByRole("button", { name: /备份并重建/ });
    expect(cancel).toHaveFocus();
    fireEvent.keyDown(dialog, { key: "Tab", shiftKey: true });
    expect(confirm).toHaveFocus();
    fireEvent.keyDown(dialog, { key: "Tab" });
    expect(cancel).toHaveFocus();
    fireEvent.click(confirm);

    expect(onWrite).toHaveBeenCalledWith({
      managedOverwrite: false,
      codexAuthChange: false,
      rebuild: ["opencode"],
    });
  });

  it("shows failed result details and dismisses without navigating", () => {
    const onFinish = vi.fn();
    renderWithI18n(
      <AgentPreviewPane
        {...props}
        preview={null}
        result={{
          transaction_id: "failed",
          agents: [
            {
              agent: "claude",
              success: false,
              error_code: "WRITE_FAILED",
              backups: [
                "/safe/claude/config.bak",
                "/safe/sk-result-secret.bak",
              ],
            },
          ],
        }}
        onFinish={onFinish}
      />,
    );

    expect(screen.getByText("失败")).toBeVisible();
    expect(screen.getByText(/WRITE_FAILED/)).toBeVisible();
    expect(screen.getByText("/safe/claude/config.bak")).toBeVisible();
    expect(document.body).not.toHaveTextContent("sk-result-secret");
    fireEvent.click(screen.getByRole("button", { name: /关闭结果并继续编辑/ }));
    expect(onFinish).toHaveBeenCalledOnce();
    expect(props.onCancel).not.toHaveBeenCalled();
  });
});
