import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { AgentPage } from "./AgentPage";
import {
  initializeAgentConfig,
  type AgentDetection,
  type AgentModelsResult,
  type AgentPreview,
  type ModelConfig,
} from "./ipc";
import { createMockApi } from "./test/api";

const detection: AgentDetection = {
  agents: ["claude", "opencode", "codex"].map((agent) => ({
    agent: agent as "claude" | "opencode" | "codex",
    name: agent,
    detected: true,
    path: `/safe/${agent}`,
    format: agent === "codex" ? "toml" : "json",
    exists: true,
    writable: true,
    configured: false,
    invalid: false,
    recovery: { eligible: false, files: [] },
  })),
};
const discovery: AgentModelsResult = {
  flow_id: "6e9cee5f-3e9d-42d9-a88e-9e0677edb806",
  models: ["model-a", "model-b"],
  catalog_token: "catalog-token",
  router_base_url: "http://127.0.0.1:19099",
  api_base_url: "http://127.0.0.1:19099/v1",
  existing: {
    model_config: {},
    unavailable_models: { claude: ["gone-model"] },
    drifted_agents: [],
  },
  preset: { model_config: {}, unavailable_agents: {} },
};
const configured: ModelConfig = {
  version: 1,
  claude: {
    primary: { model: "model-a" },
    haiku: { inherit_primary: true },
    sonnet: { inherit_primary: true },
    opus: { inherit_primary: true },
  },
  opencode: { default_model: "model-a", models: { "model-a": {} } },
  codex: { model: "model-b" },
};
const mergeModes = {
  claude: "merge",
  opencode: "merge",
  codex: "merge",
} as const;
const preview: AgentPreview = {
  revision_token: "revision",
  model_config: configured,
  fragments: [
    {
      agent: "claude",
      role: "config",
      path: "/safe/settings.json",
      format: "json",
      content: '{"token":"<redacted-api-key>"}',
    },
  ],
  files: [
    {
      path: "/safe/settings.json",
      role: "config",
      format: "json",
      operation: "replace",
      backup_path: "/safe/settings.json.bak-safe",
    },
  ],
  managed_config_drift: true,
  drifted_agents: ["claude"],
  managed_collisions: [
    {
      agent: "claude",
      path: "/env/ANTHROPIC_MODEL",
      type: "fixed_managed_path",
      action: "replace",
    },
  ],
  requires_codex_auth_approval: true,
};
const rebuildDetection: AgentDetection = {
  agents: [
    detection.agents[0],
    {
      ...detection.agents[1],
      invalid: true,
      configured: false,
      recovery: {
        eligible: true,
        reasons: ["syntax_invalid"],
        files: [
          {
            role: "config",
            path: "/safe/opencode/config.json",
            format: "json",
            exists: true,
            reasons: ["syntax_invalid"],
          },
        ],
      },
    },
    {
      ...detection.agents[2],
      detected: false,
      exists: false,
      writable: false,
    },
  ],
};
const rebuildDiscovery: AgentModelsResult = {
  ...discovery,
  existing: {
    model_config: {
      version: 1,
      claude: configured.claude,
      opencode: configured.opencode,
    },
    unavailable_models: {},
    drifted_agents: ["claude"],
  },
};
const rebuildPreview: AgentPreview = {
  ...preview,
  model_config: rebuildDiscovery.existing.model_config as ModelConfig,
  fragments: [
    {
      agent: "opencode",
      role: "config",
      path: "/safe/opencode/config.json",
      format: "json",
      content: '{"api_key":"sk-preview-canary-secret"}',
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
      preserves: ["unrelated root fields"],
    },
    {
      agent: "opencode",
      mode: "rebuild",
      path: "/safe/opencode/config.json",
      role: "config",
      format: "json",
      operation: "replace",
      backup_required: true,
      backup_pattern: "/safe/opencode/config.json.bak-<timestamp>-<random>",
      backup_sensitive: true,
      preserves: ["managed model selection only"],
      warning: "Existing unrelated settings will be discarded.",
    },
    {
      agent: "opencode",
      mode: "rebuild",
      path: "/safe/opencode/models.json",
      role: "models",
      format: "json",
      operation: "create",
      preserves: [],
    },
  ],
};

async function reachCredential(
  api = createMockApi({ detectAgents: vi.fn().mockResolvedValue(detection) }),
) {
  render(<AgentPage api={api} />);
  await screen.findByText("/safe/claude");
  fireEvent.click(
    screen.getByRole("button", { name: /继续输入凭据|Continue to credential/ }),
  );
  return api;
}

async function reachConfigure() {
  const api = createMockApi({
    detectAgents: vi.fn().mockResolvedValue(detection),
    discoverModels: vi.fn().mockResolvedValue(discovery),
  });
  await reachCredential(api);
  const secret = "fixture-secret-never-in-dom-after-submit";
  fireEvent.change(screen.getByLabelText(/API (?:key|密钥)/), {
    target: { value: secret },
  });
  fireEvent.click(
    screen.getByRole("button", { name: /发现模型|Discover models/ }),
  );
  await screen.findByText(/共同模型目录|Common model catalog/);
  return { api, secret };
}

function completeRequiredConfig() {
  fireEvent.change(screen.getByLabelText(/^主模型$|^Primary model$/), {
    target: { value: "model-a" },
  });
  fireEvent.click(screen.getByRole("checkbox", { name: "model-a" }));
  fireEvent.change(screen.getByLabelText(/默认模型|Default model/), {
    target: { value: "model-a" },
  });
  fireEvent.change(screen.getByLabelText(/活动模型|Active model/), {
    target: { value: "model-b" },
  });
}

async function reachRebuildPreview(
  overrides: Parameters<typeof createMockApi>[0] = {},
  value: AgentPreview = rebuildPreview,
) {
  const api = createMockApi({
    detectAgents: vi.fn().mockResolvedValue(rebuildDetection),
    discoverModels: vi.fn().mockResolvedValue(rebuildDiscovery),
    previewAgents: vi.fn().mockResolvedValue(value),
    ...overrides,
  });
  render(<AgentPage api={api} />);
  fireEvent.click(
    await screen.findByRole("button", {
      name: /备份并重建 OpenCode|Back up and rebuild OpenCode/,
    }),
  );
  fireEvent.click(
    screen.getByRole("button", { name: /继续输入凭据|Continue to credential/ }),
  );
  fireEvent.change(screen.getByLabelText(/API (?:key|密钥)/), {
    target: { value: "sk-ui-canary-secret" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: /发现模型|Discover models/ }),
  );
  await screen.findByText(/共同模型目录|Common model catalog/);
  fireEvent.click(
    screen.getByRole("button", {
      name: /生成写入预览|Generate write preview/,
    }),
  );
  await screen.findByText(
    "/safe/opencode/config.json.bak-<timestamp>-<random>",
  );
  return api;
}

describe("Agent model workbench", () => {
  it("initializes each Agent independently without merging existing and preset sections", () => {
    const existingClaude = configured.claude!;
    const preset = {
      version: 1 as const,
      claude: {
        ...existingClaude,
        primary: { model: "preset-must-not-merge" },
        fable: { inherit_primary: true as const },
      },
      opencode: configured.opencode,
    };
    const initialized = initializeAgentConfig(
      ["claude", "opencode", "codex"],
      { version: 1, claude: existingClaude },
      preset,
    );
    expect(initialized.config.claude).toEqual(existingClaude);
    expect(initialized.config.claude?.fable).toBeUndefined();
    expect(initialized.config.opencode).toEqual(configured.opencode);
    expect(initialized.config.codex).toEqual({ model: "" });
    expect(initialized.sources).toEqual({
      claude: "existing",
      opencode: "preset",
      codex: "empty",
    });

    const empty = initializeAgentConfig(["claude"], {}, {});
    expect(empty.config.claude?.fable).toBeUndefined();
  });

  it("enables, edits, and completely disables optional Fable", async () => {
    const { api } = await reachConfigure();
    completeRequiredConfig();
    const enable = screen.getByLabelText(/启用 Fable|Enable Fable/);
    expect(enable).not.toBeChecked();
    expect(
      screen.queryByLabelText(/fable 继承主模型|fable inherits primary/),
    ).not.toBeInTheDocument();

    fireEvent.click(enable);
    const inherit = screen.getByLabelText(
      /fable 继承主模型|fable inherits primary/,
    );
    expect(inherit).toBeChecked();
    fireEvent.click(inherit);
    expect(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    ).toBeDisabled();

    fireEvent.change(screen.getByLabelText(/fable 模型|fable model/), {
      target: { value: "model-b" },
    });
    fireEvent.change(
      screen.getByLabelText(/claude-fable (?:显示名称|Display name)/),
      { target: { value: "Fable display" } },
    );
    const fields = screen
      .getByLabelText(/claude-fable (?:显示名称|Display name)/)
      .closest(".claude-selection-fields")!;
    fireEvent.click(
      fields.querySelectorAll<HTMLInputElement>('input[type="radio"]')[1],
    );
    fireEvent.click(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    );
    await waitFor(() => expect(api.previewAgents).toHaveBeenCalledTimes(1));
    expect(vi.mocked(api.previewAgents).mock.calls[0][3].claude?.fable).toEqual(
      {
        model: "model-b",
        name: "Fable display",
        context: "1m",
      },
    );

    fireEvent.click(
      screen.getByRole("button", { name: /返回配置|Back to configuration/ }),
    );
    fireEvent.click(enable);
    expect(
      screen.queryByLabelText(/fable 模型|fable model/),
    ).not.toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    );
    await waitFor(() => expect(api.previewAgents).toHaveBeenCalledTimes(2));
    expect(
      vi.mocked(api.previewAgents).mock.calls[1][3].claude?.fable,
    ).toBeUndefined();
  });

  it("keeps Claude role labels and selection fields independently stackable", async () => {
    const claude = {
      version: 1 as const,
      claude: {
        primary: { model: "model-a" },
        haiku: { model: "model-b" },
        sonnet: { inherit_primary: true as const },
        opus: { inherit_primary: true as const },
        fable: { model: "model-a" },
      },
    };
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
      discoverModels: vi.fn().mockResolvedValue({
        ...discovery,
        existing: {
          ...discovery.existing,
          model_config: claude,
          unavailable_models: {},
        },
        preset: { model_config: {}, unavailable_agents: {} },
      }),
    });
    await reachCredential(api);
    fireEvent.change(screen.getByLabelText(/API (?:key|密钥)/), {
      target: { value: "layout-secret" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /发现模型|Discover models/ }),
    );

    const haikuSelect = await screen.findByLabelText(/haiku 模型|haiku model/);
    const haikuFields = haikuSelect.closest(".claude-selection-fields")!;
    const haikuRow = haikuFields.closest(".role-row")!;
    expect(haikuRow.children[0].tagName).toBe("LABEL");
    expect(haikuRow.children[1]).toBe(haikuFields);

    const fableSelect = screen.getByLabelText(/fable 模型|fable model/);
    const fableFields = fableSelect.closest(".claude-selection-fields")!;
    const optionalEditor = fableFields.closest(".optional-role-editor")!;
    const fableRow = optionalEditor.closest(".role-row")!;
    expect(fableRow.children[0].tagName).toBe("LABEL");
    expect(fableRow.children[1]).toBe(optionalEditor);
    expect(optionalEditor.children[1]).toBe(fableFields);
    const modelPanel = haikuRow.closest(".model-agent-panel");
    expect(modelPanel).not.toBeNull();
    expect(fableRow.closest(".model-agent-panel")).toBe(modelPanel);
  });

  it("contains detected configuration paths and exposes their complete values", async () => {
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
    });
    render(<AgentPage api={api} />);

    for (const agent of detection.agents) {
      const path = await screen.findByTitle(agent.path);
      expect(path).toHaveClass("agent-card__config-path");
      expect(path).toHaveTextContent(agent.path);
    }
  });

  it("recovery selection keeps merge and eligible rebuild modes independent", async () => {
    const recoveryDetection: AgentDetection = {
      agents: [
        detection.agents[0],
        {
          ...detection.agents[1],
          invalid: true,
          configured: false,
          recovery: {
            eligible: true,
            files: [
              {
                role: "config",
                path: "/safe/opencode",
                format: "json",
                exists: true,
                reasons: ["syntax_invalid"],
              },
            ],
          },
        },
        {
          ...detection.agents[2],
          invalid: true,
          configured: false,
          recovery: {
            eligible: false,
            reasons: ["linked"],
            files: [],
          },
        },
      ],
    };
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(recoveryDetection),
      discoverModels: vi.fn().mockResolvedValue({
        ...discovery,
        existing: {
          model_config: configured,
          unavailable_models: {},
          drifted_agents: [],
        },
      }),
    });
    render(<AgentPage api={api} />);

    const claude = await screen.findByRole("checkbox", {
      name: /选择 Claude Code|Select Claude Code/,
    });
    const opencode = screen.getByRole("checkbox", {
      name: /选择 OpenCode|Select OpenCode/,
    });
    const codex = screen.getByRole("checkbox", {
      name: /选择 Codex|Select Codex/,
    });
    expect(claude).toBeChecked();
    expect(opencode).toBeDisabled();
    expect(codex).toBeDisabled();
    expect(screen.getByText(/链接文件|linked file/i)).toBeVisible();
    expect(
      screen.queryByRole("button", {
        name: /备份并重建 Codex|Back up and rebuild Codex/,
      }),
    ).not.toBeInTheDocument();

    const rebuild = screen.getByRole("button", {
      name: /备份并重建 OpenCode|Back up and rebuild OpenCode/,
    });
    fireEvent.click(rebuild);
    expect(rebuild).toHaveAttribute("aria-pressed", "true");
    expect(claude).toBeChecked();
    expect(
      screen.getByText(/已选择 2 个 Agent|2 Agents selected/),
    ).toBeVisible();

    fireEvent.click(
      screen.getByRole("button", {
        name: /继续输入凭据|Continue to credential/,
      }),
    );
    fireEvent.change(screen.getByLabelText(/API (?:key|密钥)/), {
      target: { value: "recovery-selection-secret" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /发现模型|Discover models/ }),
    );
    await screen.findByText(/共同模型目录|Common model catalog/);
    expect(screen.queryByDisplayValue("recovery-selection-secret")).toBeNull();
    fireEvent.click(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    );

    await waitFor(() => expect(api.previewAgents).toHaveBeenCalledTimes(1));
    expect(api.discoverModels).toHaveBeenCalledWith(
      ["claude", "opencode"],
      "recovery-selection-secret",
    );
    expect(api.previewAgents).toHaveBeenCalledWith(
      ["claude", "opencode"],
      discovery.flow_id,
      discovery.catalog_token,
      {
        version: 1,
        claude: configured.claude,
        opencode: configured.opencode,
      },
      { claude: "merge", opencode: "rebuild" },
    );
  });

  it("recovery selection shows actual absent and read-only guidance", async () => {
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue({
        agents: [
          {
            ...detection.agents[0],
            detected: false,
            exists: false,
            writable: false,
          },
          { ...detection.agents[1], writable: false },
          detection.agents[2],
        ],
      }),
    });
    render(<AgentPage api={api} />);

    expect(
      await screen.findByRole("checkbox", {
        name: /选择 Claude Code|Select Claude Code/,
      }),
    ).toBeDisabled();
    expect(
      screen.getByRole("checkbox", { name: /选择 OpenCode|Select OpenCode/ }),
    ).toBeDisabled();
    expect(screen.getByText(/未检测到|Not detected/)).toBeVisible();
    expect(screen.getByText(/不可写|Not writable/)).toBeVisible();
    expect(
      screen.getByText(/安装或启动该 Agent|Install or start this Agent/),
    ).toBeVisible();
    expect(
      screen.getByText(
        /恢复当前用户对配置目录的写权限|Restore current-user write access/,
      ),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: /备份并重建|Back up and rebuild/ }),
    ).not.toBeInTheDocument();
  });

  it("does not auto-select models and clears the credential immediately on discovery submit", async () => {
    const { api, secret } = await reachConfigure();
    expect(api.discoverModels).toHaveBeenCalledWith(
      ["claude", "opencode", "codex"],
      secret,
    );
    expect(document.body.textContent).not.toContain(secret);
    expect(screen.getByLabelText(/^主模型$|^Primary model$/)).toHaveValue("");
    expect(screen.getByLabelText(/活动模型|Active model/)).toHaveValue("");
    expect(
      screen
        .getAllByRole("checkbox")
        .filter(
          (item) => item.closest(".catalog-model") && item.matches(":checked"),
        ),
    ).toHaveLength(0);
    expect(document.body.textContent).toContain("gone-model");
  });

  it("keeps every configuration select native inside the Agent theme wrapper", async () => {
    await reachConfigure();

    const selects = screen.getAllByRole("combobox");
    expect(selects.length).toBeGreaterThan(0);
    for (const select of selects) {
      expect(select.tagName).toBe("SELECT");
      expect(select.parentElement).toHaveClass("agent-select-control");
      expect(
        select.parentElement?.querySelector(".agent-select-control__chevron"),
      ).toHaveAttribute("aria-hidden", "true");
    }
  });

  it("supports searchable catalog, Claude inheritance, opencode multi/default, Codex typed omission, and constrained extra", async () => {
    await reachConfigure();
    const searches = screen.getAllByRole("searchbox");
    expect(searches).toHaveLength(1);
    fireEvent.change(searches[0], { target: { value: "model-b" } });
    expect(screen.getAllByText("model-b").length).toBeGreaterThan(0);
    expect(
      screen.getByLabelText(/haiku 继承主模型|haiku inherits primary/),
    ).toBeChecked();
    fireEvent.change(searches[0], { target: { value: "model-a" } });
    fireEvent.click(screen.getByRole("checkbox", { name: "model-a" }));
    fireEvent.change(screen.getByLabelText(/默认模型|Default model/), {
      target: { value: "model-a" },
    });
    expect(screen.getByLabelText(/推理强度|Reasoning effort/)).toHaveValue("");
    fireEvent.click(
      screen.getAllByText(/高级受限 extra|Advanced constrained extra/)[1],
    );
    const editor = screen.getByLabelText(/OpenCode extra JSON/);
    fireEvent.change(editor, { target: { value: '{"api-key":"forbidden"}' } });
    expect(await screen.findByText(/protected_path/)).toBeInTheDocument();
    expect(editor).toHaveAttribute("aria-invalid", "true");
    fireEvent.change(editor, {
      target: { value: '{"safe":{"connection":"forbidden"}}' },
    });
    expect(editor).toHaveAttribute("aria-invalid", "true");
    const variants = screen.getByLabelText(/Variants JSON/);
    fireEvent.change(variants, {
      target: { value: '{"fast":{"safe":{"connection":"forbidden"}}}' },
    });
    expect(variants).toHaveAttribute("aria-invalid", "true");
    fireEvent.change(editor, { target: { value: '{"status":"active"}' } });
    fireEvent.click(
      editor.closest("details")!.querySelector<HTMLButtonElement>("button")!,
    );
    expect(editor).toHaveValue('{\n  "status": "active"\n}');
  });

  it("preserves preset Claude metadata, clears it on model changes, and resets explicit roles on inheritance", async () => {
    const presetClaude: NonNullable<ModelConfig["claude"]> = {
      primary: { model: "model-a", name: "Primary 1M", context: "1m" },
      haiku: { model: "model-a", name: "Fast 1M", context: "1m" },
      sonnet: { inherit_primary: true },
      opus: { inherit_primary: true },
    };
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
      discoverModels: vi.fn().mockResolvedValue({
        ...discovery,
        preset: {
          model_config: { version: 1, claude: presetClaude },
          unavailable_agents: {
            codex: { code: "MODEL_NOT_AVAILABLE", models: ["missing-codex"] },
          },
        },
      }),
    });
    await reachCredential(api);
    fireEvent.change(screen.getByLabelText(/API (?:key|密钥)/), {
      target: { value: "preset-secret" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /发现模型|Discover models/ }),
    );
    expect(await screen.findByDisplayValue("Primary 1M")).toBeVisible();
    expect(screen.getByDisplayValue("Fast 1M")).toBeVisible();
    expect(screen.getByText(/missing-codex/)).toBeVisible();
    expect(
      screen.getAllByText(/推荐预设|recommended preset/).length,
    ).toBeGreaterThan(0);
    expect(screen.getAllByRole("radio", { name: "1M" })[0]).toBeChecked();

    fireEvent.change(screen.getByLabelText(/^主模型$|^Primary model$/), {
      target: { value: "model-b" },
    });
    expect(screen.queryByDisplayValue("Primary 1M")).not.toBeInTheDocument();
    expect(
      screen.getAllByRole("radio", { name: /标准|Standard/ })[0],
    ).toBeChecked();

    const inheritHaiku = screen.getByLabelText(
      /haiku 继承主模型|haiku inherits primary/,
    );
    fireEvent.click(inheritHaiku);
    expect(screen.queryByDisplayValue("Fast 1M")).not.toBeInTheDocument();
    fireEvent.click(inheritHaiku);
    expect(screen.getByLabelText(/haiku 模型|haiku model/)).toHaveValue("");
    expect(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    ).toBeDisabled();
  });

  it("round-trips initialized Claude budgets and typed opencode variants through preview", async () => {
    const claude = {
      primary: { model: "model-a" },
      haiku: { inherit_primary: true } as const,
      sonnet: { inherit_primary: true } as const,
      opus: { inherit_primary: true } as const,
      context_window: 353400,
      max_output_tokens: 100000,
      extra: { preserved: "claude" },
    };
    const opencode = {
      default_model: "model-a",
      models: {
        "model-a": {
          name: "Preserved model name",
          variants: { medium: { reasoningEffort: "medium" } },
          extra: { preserved: true },
        },
      },
    };
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
      discoverModels: vi.fn().mockResolvedValue({
        ...discovery,
        existing: {
          model_config: { version: 1, claude },
          unavailable_models: {},
          drifted_agents: [],
        },
        preset: {
          model_config: {
            version: 1,
            opencode,
            codex: { model: "model-b" },
          },
          unavailable_agents: {},
        },
      }),
    });
    await reachCredential(api);
    fireEvent.change(screen.getByLabelText(/API (?:key|密钥)/), {
      target: { value: "variant-test-secret" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /发现模型|Discover models/ }),
    );

    expect(
      await screen.findByLabelText(/Claude 上下文窗口|Claude context window/),
    ).toHaveValue(353400);
    expect(
      screen.getByLabelText(/Claude 最大输出 Token|Claude max output tokens/),
    ).toHaveValue(100000);
    const variants = screen.getByLabelText(/Variants JSON/);
    expect(variants).toHaveValue(
      '{\n  "medium": {\n    "reasoningEffort": "medium"\n  }\n}',
    );

    fireEvent.change(
      screen.getByLabelText(/Claude 上下文窗口|Claude context window/),
      { target: { value: "400000" } },
    );
    fireEvent.change(
      screen.getByLabelText(/Claude 最大输出 Token|Claude max output tokens/),
      { target: { value: "120000" } },
    );
    fireEvent.change(variants, {
      target: {
        value:
          '{"medium":{"reasoningEffort":"high"},"fast":{"reasoningSummary":"auto"}}',
      },
    });
    fireEvent.click(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    );

    await waitFor(() => expect(api.previewAgents).toHaveBeenCalledTimes(1));
    expect(api.previewAgents).toHaveBeenCalledWith(
      ["claude", "opencode", "codex"],
      discovery.flow_id,
      discovery.catalog_token,
      {
        version: 1,
        claude: {
          ...claude,
          context_window: 400000,
          max_output_tokens: 120000,
        },
        opencode: {
          default_model: "model-a",
          models: {
            "model-a": {
              name: "Preserved model name",
              variants: {
                medium: { reasoningEffort: "high" },
                fast: { reasoningSummary: "auto" },
              },
              extra: { preserved: true },
            },
          },
        },
        codex: { model: "model-b" },
      },
      mergeModes,
    );
  });

  it("blocks preview while a model object field contains invalid JSON and recovers", async () => {
    const { api } = await reachConfigure();
    completeRequiredConfig();
    const variants = screen.getByLabelText(/Variants JSON/);

    fireEvent.change(variants, {
      target: { value: '{"fast":{"reasoningEffort":"high"}}' },
    });
    fireEvent.change(variants, { target: { value: '{"fast":' } });

    expect(variants).toHaveAttribute("aria-invalid", "true");
    const previewButton = screen.getByRole("button", {
      name: /生成写入预览|Generate write preview/,
    });
    expect(previewButton).toBeDisabled();
    fireEvent.click(previewButton);
    expect(api.previewAgents).not.toHaveBeenCalled();
    expect(screen.getAllByRole("alert").length).toBeGreaterThan(0);

    fireEvent.change(variants, {
      target: { value: '{"fast":{"reasoningEffort":"low"}}' },
    });
    expect(variants).toHaveAttribute("aria-invalid", "false");
    expect(previewButton).toBeEnabled();

    fireEvent.change(variants, { target: { value: "{" } });
    expect(previewButton).toBeDisabled();
    fireEvent.change(variants, { target: { value: "" } });
    expect(variants).toHaveAttribute("aria-invalid", "false");
    expect(previewButton).toBeEnabled();
    fireEvent.click(previewButton);
    await waitFor(() => expect(api.previewAgents).toHaveBeenCalledTimes(1));
  });

  it("previews edited explicit Claude roles with independent names and contexts", async () => {
    const { api } = await reachConfigure();
    fireEvent.change(screen.getByLabelText(/^主模型$|^Primary model$/), {
      target: { value: "model-a" },
    });

    const roles = [
      ["haiku", "model-a", "Haiku display", false],
      ["sonnet", "model-b", "Sonnet display", true],
      ["opus", "model-a", "Opus display", true],
    ] as const;
    for (const [role, model, name, oneMillion] of roles) {
      fireEvent.click(
        screen.getByLabelText(
          new RegExp(`${role} (?:继承主模型|inherits primary)`),
        ),
      );
      fireEvent.change(
        screen.getByLabelText(new RegExp(`${role} (?:模型|model)`)),
        { target: { value: model } },
      );
      const nameInput = screen.getByLabelText(
        new RegExp(`claude-${role} (?:显示名称|Display name)`),
      );
      fireEvent.change(nameInput, { target: { value: name } });
      if (oneMillion) {
        const fields = nameInput.closest(".claude-selection-fields")!;
        fireEvent.click(
          fields.querySelectorAll<HTMLInputElement>('input[type="radio"]')[1],
        );
      }
    }

    completeRequiredConfig();
    fireEvent.click(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    );
    await waitFor(() => expect(api.previewAgents).toHaveBeenCalledTimes(1));
    expect(api.previewAgents).toHaveBeenCalledWith(
      ["claude", "opencode", "codex"],
      discovery.flow_id,
      discovery.catalog_token,
      {
        version: 1,
        claude: {
          primary: { model: "model-a" },
          haiku: { model: "model-a", name: "Haiku display" },
          sonnet: {
            model: "model-b",
            name: "Sonnet display",
            context: "1m",
          },
          opus: {
            model: "model-a",
            name: "Opus display",
            context: "1m",
          },
        },
        opencode: { default_model: "model-a", models: { "model-a": {} } },
        codex: { model: "model-b" },
      },
      mergeModes,
    );
  });

  it("imports a canonical document as a complete replacement for generated defaults", async () => {
    const imported: ModelConfig = {
      version: 1,
      claude: {
        primary: {
          model: "model-b",
          name: "Imported primary",
          context: "1m",
        },
        haiku: { inherit_primary: true },
        sonnet: { inherit_primary: true },
        opus: { inherit_primary: true },
      },
      opencode: { default_model: "model-b", models: { "model-b": {} } },
      codex: { model: "model-a" },
    };
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
      discoverModels: vi.fn().mockResolvedValue({
        ...discovery,
        existing: {
          model_config: configured,
          unavailable_models: {},
          drifted_agents: [],
        },
        preset: {
          model_config: {
            version: 1,
            codex: { model: "preset-must-be-replaced" },
          },
          unavailable_agents: {},
        },
      }),
      importAgentModelConfig: vi.fn().mockResolvedValue(imported),
    });
    await reachCredential(api);
    fireEvent.change(screen.getByLabelText(/API (?:key|密钥)/), {
      target: { value: "import-secret" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /发现模型|Discover models/ }),
    );
    expect(
      await screen.findByLabelText(/^主模型$|^Primary model$/),
    ).toHaveValue("model-a");

    const file = new File([JSON.stringify(imported)], "canonical.json", {
      type: "application/json",
    });
    fireEvent.change(
      document.querySelector<HTMLInputElement>('input[type="file"]')!,
      { target: { files: [file] } },
    );
    expect(await screen.findByDisplayValue("Imported primary")).toBeVisible();
    expect(screen.getByLabelText(/^主模型$|^Primary model$/)).toHaveValue(
      "model-b",
    );
    expect(screen.getByLabelText(/活动模型|Active model/)).toHaveValue(
      "model-a",
    );

    fireEvent.click(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    );
    await waitFor(() => expect(api.previewAgents).toHaveBeenCalledTimes(1));
    expect(api.importAgentModelConfig).toHaveBeenCalledWith(
      JSON.stringify(imported),
      ["claude", "opencode", "codex"],
      discovery.flow_id,
    );
    expect(api.previewAgents).toHaveBeenCalledWith(
      ["claude", "opencode", "codex"],
      discovery.flow_id,
      discovery.catalog_token,
      imported,
      mergeModes,
    );
  });

  it("replaces displayed and submitted variants when importing the same model ID", async () => {
    const imported: ModelConfig = {
      version: 1,
      opencode: {
        default_model: "model-a",
        models: {
          "model-a": {
            variants: { imported: { reasoningEffort: "high" } },
          },
        },
      },
    };
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
      discoverModels: vi.fn().mockResolvedValue({
        ...discovery,
        existing: {
          model_config: {
            version: 1,
            opencode: {
              default_model: "model-a",
              models: {
                "model-a": {
                  variants: { stale: { reasoningEffort: "low" } },
                },
              },
            },
          },
          unavailable_models: {},
          drifted_agents: [],
        },
      }),
      importAgentModelConfig: vi.fn().mockResolvedValue(imported),
    });
    await reachCredential(api);
    fireEvent.change(screen.getByLabelText(/API (?:key|密钥)/), {
      target: { value: "variant-import-secret" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /发现模型|Discover models/ }),
    );

    const variants = await screen.findByLabelText(/Variants JSON/);
    expect(variants).toHaveValue(
      '{\n  "stale": {\n    "reasoningEffort": "low"\n  }\n}',
    );
    fireEvent.change(variants, {
      target: { value: '{"stale":{"reasoningEffort":"medium"}}' },
    });
    expect(variants).toHaveValue('{"stale":{"reasoningEffort":"medium"}}');

    const file = new File([JSON.stringify(imported)], "variants.json", {
      type: "application/json",
    });
    fireEvent.change(
      document.querySelector<HTMLInputElement>('input[type="file"]')!,
      { target: { files: [file] } },
    );
    await waitFor(() =>
      expect(variants).toHaveValue(
        '{\n  "imported": {\n    "reasoningEffort": "high"\n  }\n}',
      ),
    );

    fireEvent.click(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    );
    await waitFor(() => expect(api.previewAgents).toHaveBeenCalledTimes(1));
    expect(api.previewAgents).toHaveBeenCalledWith(
      ["claude", "opencode", "codex"],
      discovery.flow_id,
      discovery.catalog_token,
      imported,
      mergeModes,
    );
  });

  it("replaces same-model Fable metadata and inheritance on import", async () => {
    const explicit: ModelConfig = {
      version: 1,
      claude: {
        primary: { model: "model-a" },
        haiku: { inherit_primary: true },
        sonnet: { inherit_primary: true },
        opus: { inherit_primary: true },
        fable: { model: "model-a", name: "Imported Fable", context: "1m" },
      },
    };
    const inherited: ModelConfig = {
      ...explicit,
      claude: { ...explicit.claude!, fable: { inherit_primary: true } },
    };
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
      discoverModels: vi.fn().mockResolvedValue({
        ...discovery,
        existing: {
          model_config: {
            version: 1,
            claude: {
              ...explicit.claude!,
              fable: { model: "model-a", name: "Stale Fable" },
            },
          },
          unavailable_models: {},
          drifted_agents: [],
        },
      }),
      importAgentModelConfig: vi
        .fn()
        .mockResolvedValueOnce(explicit)
        .mockResolvedValueOnce(inherited),
    });
    await reachCredential(api);
    fireEvent.change(screen.getByLabelText(/API (?:key|密钥)/), {
      target: { value: "fable-import-secret" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /发现模型|Discover models/ }),
    );
    expect(await screen.findByDisplayValue("Stale Fable")).toBeVisible();

    const input =
      document.querySelector<HTMLInputElement>('input[type="file"]')!;
    fireEvent.change(input, {
      target: {
        files: [
          new File([JSON.stringify(explicit)], "explicit.json", {
            type: "application/json",
          }),
        ],
      },
    });
    expect(await screen.findByDisplayValue("Imported Fable")).toBeVisible();
    expect(screen.getByLabelText(/fable 模型|fable model/)).toHaveValue(
      "model-a",
    );
    const fableFields = screen
      .getByDisplayValue("Imported Fable")
      .closest(".claude-selection-fields")!;
    expect(
      fableFields.querySelectorAll<HTMLInputElement>('input[type="radio"]')[1],
    ).toBeChecked();

    fireEvent.change(input, {
      target: {
        files: [
          new File([JSON.stringify(inherited)], "inherited.json", {
            type: "application/json",
          }),
        ],
      },
    });
    expect(
      await screen.findByLabelText(/fable 继承主模型|fable inherits primary/),
    ).toBeChecked();
    fireEvent.click(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    );
    await waitFor(() => expect(api.previewAgents).toHaveBeenCalledTimes(1));
    expect(api.previewAgents).toHaveBeenCalledWith(
      ["claude", "opencode", "codex"],
      discovery.flow_id,
      discovery.catalog_token,
      inherited,
      mergeModes,
    );
  });

  it("writes the preview-normalized Fable config", async () => {
    const configuredFable: ModelConfig = {
      ...configured,
      claude: {
        ...configured.claude!,
        fable: { model: "model-b", name: "Before normalization" },
      },
    };
    const normalized: ModelConfig = {
      ...configuredFable,
      claude: {
        ...configuredFable.claude!,
        fable: { model: "model-b", name: "Normalized Fable", context: "1m" },
      },
    };
    const normalizedPreview: AgentPreview = {
      ...preview,
      model_config: normalized,
      managed_config_drift: false,
      requires_codex_auth_approval: false,
    };
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
      discoverModels: vi.fn().mockResolvedValue({
        ...discovery,
        existing: {
          model_config: configuredFable,
          unavailable_models: {},
          drifted_agents: [],
        },
      }),
      previewAgents: vi.fn().mockResolvedValue(normalizedPreview),
      writeAgents: vi
        .fn()
        .mockResolvedValue({ transaction_id: "tx", agents: [] }),
    });
    await reachCredential(api);
    fireEvent.change(screen.getByLabelText(/API (?:key|密钥)/), {
      target: { value: "normalized-write-secret" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /发现模型|Discover models/ }),
    );
    await screen.findByDisplayValue("Before normalization");
    fireEvent.click(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    );
    fireEvent.click(
      await screen.findByRole("button", {
        name: /写入所选 Agent|Write selected Agents/,
      }),
    );
    await waitFor(() => expect(api.writeAgents).toHaveBeenCalledTimes(1));
    expect(api.writeAgents).toHaveBeenCalledWith(
      ["claude", "opencode", "codex"],
      discovery.flow_id,
      discovery.catalog_token,
      normalized,
      normalizedPreview.revision_token,
      false,
      false,
      [],
    );
  });

  it("renders a sanitized rebuild preview and requires exact accessible confirmation", async () => {
    const user = userEvent.setup();
    Object.defineProperty(window, "innerWidth", {
      configurable: true,
      value: 360,
    });
    let resolveWrite: (() => void) | undefined;
    const api = await reachRebuildPreview({
      writeAgents: vi.fn().mockImplementation(
        () =>
          new Promise((resolve) => {
            resolveWrite = () =>
              resolve({ transaction_id: "tx-rebuild", agents: [] });
          }),
      ),
    });

    expect(screen.getAllByText(/重建|Rebuild/)).toHaveLength(2);
    expect(screen.getByText(/合并|Merge/)).toBeVisible();
    expect(screen.getByText("/safe/claude/settings.json")).toBeVisible();
    expect(screen.getAllByText("/safe/opencode/config.json")).toHaveLength(2);
    expect(screen.getByText("/safe/opencode/models.json")).toBeVisible();
    expect(
      screen.getByText("/safe/opencode/config.json.bak-<timestamp>-<random>"),
    ).toBeVisible();
    expect(screen.getByText(/SENSITIVE BACKUP/)).toBeVisible();
    expect(screen.getByText(/managed model selection only/)).toBeVisible();
    expect(
      screen.getByText(/Existing unrelated settings will be discarded/),
    ).toBeVisible();
    expect(document.documentElement.outerHTML).not.toContain(
      "sk-ui-canary-secret",
    );
    expect(document.body.textContent).not.toContain("sk-preview-canary-secret");

    const write = screen.getByRole("button", {
      name: /写入所选 Agent|Write selected Agents/,
    });
    expect(write).toBeDisabled();
    fireEvent.click(screen.getByLabelText(/漂移|drifted/));
    expect(write).toBeDisabled();
    fireEvent.click(screen.getByLabelText(/Codex/));
    expect(write).toBeEnabled();
    fireEvent.click(write);

    let dialog = screen.getByRole("dialog", {
      name: /确认备份并重建|Confirm backup and rebuild/,
    });
    const cancel = screen.getByRole("button", { name: /^取消$|^Cancel$/ });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(cancel).toHaveFocus();
    expect(dialog).toHaveTextContent("OpenCode");
    expect(dialog).not.toHaveTextContent("Claude Code");
    const confirm = screen.getByRole("button", {
      name: /^备份并重建$|^Back up and rebuild$/,
    });
    fireEvent.keyDown(cancel, { key: "Tab", shiftKey: true });
    expect(confirm).toHaveFocus();
    fireEvent.keyDown(confirm, { key: "Tab" });
    expect(cancel).toHaveFocus();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await waitFor(() => expect(write).toHaveFocus());

    fireEvent.click(write);
    dialog = screen.getByRole("dialog");
    fireEvent.mouseDown(dialog.parentElement!, {
      target: dialog.parentElement,
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await waitFor(() => expect(write).toHaveFocus());

    fireEvent.click(write);
    const finalConfirm = screen.getByRole("button", {
      name: /^备份并重建$|^Back up and rebuild$/,
    });
    finalConfirm.focus();
    await user.keyboard("{Enter}");
    expect(api.writeAgents).toHaveBeenCalledTimes(1);
    expect(api.writeAgents).toHaveBeenCalledWith(
      ["claude", "opencode"],
      rebuildDiscovery.flow_id,
      rebuildDiscovery.catalog_token,
      rebuildPreview.model_config,
      rebuildPreview.revision_token,
      true,
      true,
      ["opencode"],
    );
    resolveWrite?.();
  });

  it("fails closed when preview includes an extra rebuild Agent", async () => {
    const api = await reachRebuildPreview(
      {},
      {
        ...rebuildPreview,
        managed_config_drift: false,
        requires_codex_auth_approval: false,
        files: [
          ...rebuildPreview.files,
          {
            agent: "codex",
            mode: "rebuild",
            path: "/safe/codex/config.toml",
            role: "config",
            format: "toml",
            operation: "replace",
          },
        ],
      },
    );

    const write = screen.getByRole("button", {
      name: /写入所选 Agent|Write selected Agents/,
    });
    expect(write).toBeDisabled();
    await userEvent.setup().click(write);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(api.writeAgents).not.toHaveBeenCalled();
  });

  it("reuses discovery and Rust flow when stale rebuild selections remain eligible", async () => {
    const detectAgents = vi
      .fn()
      .mockResolvedValueOnce(rebuildDetection)
      .mockResolvedValueOnce({
        agents: rebuildDetection.agents.map((agent) => ({ ...agent })),
      });
    const api = await reachRebuildPreview(
      {
        detectAgents,
        writeAgents: vi.fn().mockRejectedValue({
          code: "PREVIEW_STALE",
          message: "sk-reusable-stale-canary-secret",
        }),
      },
      {
        ...rebuildPreview,
        managed_config_drift: false,
        requires_codex_auth_approval: false,
      },
    );
    fireEvent.click(
      screen.getByRole("button", {
        name: /写入所选 Agent|Write selected Agents/,
      }),
    );
    fireEvent.click(
      screen.getByRole("button", {
        name: /^备份并重建$|^Back up and rebuild$/,
      }),
    );

    expect(
      await screen.findByText(/预览后文件发生变化|Files changed after preview/),
    ).toBeVisible();
    expect(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    ).toBeVisible();
    expect(screen.queryByLabelText(/API (?:key|密钥)/)).not.toBeInTheDocument();
    expect(detectAgents).toHaveBeenCalledTimes(2);
    expect(api.discoverModels).toHaveBeenCalledTimes(1);
    expect(api.destroyAgentModelFlow).not.toHaveBeenCalled();
    expect(document.body.textContent).not.toContain(
      "sk-reusable-stale-canary-secret",
    );

    fireEvent.click(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    );
    await waitFor(() => expect(api.previewAgents).toHaveBeenCalledTimes(2));
    expect(api.previewAgents).toHaveBeenLastCalledWith(
      ["claude", "opencode"],
      rebuildDiscovery.flow_id,
      rebuildDiscovery.catalog_token,
      rebuildPreview.model_config,
      { claude: "merge", opencode: "rebuild" },
    );
  });

  it("falls back to authoritative selection when a stale rebuild source was repaired", async () => {
    const refreshed: AgentDetection = {
      agents: detection.agents.map((agent) => ({ ...agent, configured: true })),
    };
    const detectAgents = vi
      .fn()
      .mockResolvedValueOnce(rebuildDetection)
      .mockResolvedValueOnce(refreshed);
    const api = await reachRebuildPreview(
      {
        detectAgents,
        writeAgents: vi.fn().mockRejectedValue({
          code: "PREVIEW_STALE",
          message: "sk-stale-canary-secret",
        }),
      },
      {
        ...rebuildPreview,
        managed_config_drift: false,
        requires_codex_auth_approval: false,
      },
    );
    fireEvent.click(
      screen.getByRole("button", {
        name: /写入所选 Agent|Write selected Agents/,
      }),
    );
    fireEvent.click(
      screen.getByRole("button", {
        name: /^备份并重建$|^Back up and rebuild$/,
      }),
    );

    expect(
      await screen.findByText(/预览后文件发生变化|Files changed after preview/),
    ).toBeVisible();
    expect(detectAgents).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(
      screen.getByText(/已选择 3 个 Agent|3 Agents selected/),
    ).toBeVisible();
    expect(document.body.textContent).not.toContain("sk-stale-canary-secret");
    expect(api.destroyAgentModelFlow).toHaveBeenCalledWith(
      rebuildDiscovery.flow_id,
    );
  });

  it("clears stale flow and safely reports a detection failure", async () => {
    const detectAgents = vi
      .fn()
      .mockResolvedValueOnce(rebuildDetection)
      .mockRejectedValueOnce(new Error("sk-detect-canary-secret"));
    const api = await reachRebuildPreview(
      {
        detectAgents,
        writeAgents: vi.fn().mockRejectedValue({ code: "PREVIEW_STALE" }),
      },
      {
        ...rebuildPreview,
        managed_config_drift: false,
        requires_codex_auth_approval: false,
      },
    );
    fireEvent.click(
      screen.getByRole("button", {
        name: /写入所选 Agent|Write selected Agents/,
      }),
    );
    fireEvent.click(
      screen.getByRole("button", {
        name: /^备份并重建$|^Back up and rebuild$/,
      }),
    );

    expect(
      await screen.findByText(/Agent 检测失败|Agent detection failed/),
    ).toBeVisible();
    expect(api.destroyAgentModelFlow).toHaveBeenCalledWith(
      rebuildDiscovery.flow_id,
    );
    expect(screen.queryByLabelText(/API (?:key|密钥)/)).not.toBeInTheDocument();
    expect(document.body.textContent).not.toContain("sk-detect-canary-secret");
  });

  it.each([
    ["BACKUP_FAILED", /备份失败|Backup failed/, 1],
    ["WRITE_FAILED", /已从回滚备份恢复|restored from rollback backups/, 1],
    ["ROLLBACK_FAILED", /无法完成回滚|Rollback could not be completed/, 2],
  ] as const)(
    "reports stable %s rebuild outcomes and refreshes only unresolved rollback",
    async (code, message, detectionCalls) => {
      const writesDisabled: AgentDetection = {
        agents: rebuildDetection.agents.map((agent) => ({
          ...agent,
          writable: false,
          recovery: agent.recovery
            ? {
                ...agent.recovery,
                eligible: false,
                reasons: ["writes_disabled"],
              }
            : {
                eligible: false,
                reasons: ["writes_disabled"],
                files: [],
              },
        })),
      };
      const detectAgents = vi
        .fn()
        .mockResolvedValueOnce(rebuildDetection)
        .mockResolvedValue(writesDisabled);
      const api = await reachRebuildPreview(
        {
          detectAgents,
          writeAgents: vi.fn().mockRejectedValue({
            code,
            message: `sk-${code.toLowerCase()}-canary-secret`,
          }),
        },
        {
          ...rebuildPreview,
          managed_config_drift: false,
          requires_codex_auth_approval: false,
        },
      );
      fireEvent.click(
        screen.getByRole("button", {
          name: /写入所选 Agent|Write selected Agents/,
        }),
      );
      fireEvent.click(
        screen.getByRole("button", {
          name: /^备份并重建$|^Back up and rebuild$/,
        }),
      );

      expect(await screen.findByText(message)).toBeVisible();
      expect(detectAgents).toHaveBeenCalledTimes(detectionCalls);
      expect(document.body.textContent).not.toContain("canary-secret");
      expect(api.destroyAgentModelFlow).toHaveBeenCalledWith(
        rebuildDiscovery.flow_id,
      );
      if (code === "ROLLBACK_FAILED") {
        expect(screen.getByText(/已禁用|disabled/i)).toBeVisible();
      }
    },
  );

  it("shows actual backup paths on success and refreshes detection on Finish", async () => {
    const detectAgents = vi.fn().mockResolvedValue(rebuildDetection);
    const api = await reachRebuildPreview(
      {
        detectAgents,
        writeAgents: vi.fn().mockResolvedValue({
          transaction_id: "tx-success",
          agents: [
            {
              agent: "opencode",
              success: true,
              changed: ["/safe/opencode/config.json"],
              backups: ["/safe/opencode/config.json.bak-actual-42"],
            },
          ],
        }),
      },
      {
        ...rebuildPreview,
        managed_config_drift: false,
        requires_codex_auth_approval: false,
      },
    );
    fireEvent.click(
      screen.getByRole("button", {
        name: /写入所选 Agent|Write selected Agents/,
      }),
    );
    fireEvent.click(
      screen.getByRole("button", {
        name: /^备份并重建$|^Back up and rebuild$/,
      }),
    );

    expect(
      await screen.findByText("/safe/opencode/config.json.bak-actual-42"),
    ).toBeVisible();
    expect(screen.getByText(/已创建备份|Backup created/)).toBeVisible();
    expect(screen.getByText(/^重建$|^Rebuild$/)).toBeVisible();
    expect(detectAgents).toHaveBeenCalledTimes(1);
    fireEvent.click(
      screen.getByRole("button", {
        name: /完成并刷新检测|Finish and refresh detection/,
      }),
    );
    await waitFor(() => expect(detectAgents).toHaveBeenCalledTimes(2));
    expect(api.writeAgents).toHaveBeenLastCalledWith(
      ["claude", "opencode"],
      rebuildDiscovery.flow_id,
      rebuildDiscovery.catalog_token,
      rebuildPreview.model_config,
      rebuildPreview.revision_token,
      false,
      false,
      ["opencode"],
    );
  });

  it("keeps labels, stage status, alerts, and controls accessible at a narrow viewport", async () => {
    Object.defineProperty(window, "innerWidth", {
      configurable: true,
      value: 360,
    });
    await reachConfigure();
    expect(
      screen.getByRole("searchbox", { name: /搜索模型|Search models/ }),
    ).toBeVisible();
    expect(screen.getByLabelText(/当前步骤|Current stage/)).toBeInTheDocument();
    expect(screen.getByLabelText(/^主模型$|^Primary model$/)).toBeVisible();
    expect(screen.getByLabelText(/启用 Fable|Enable Fable/)).toBeVisible();
    expect(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    ).toBeVisible();
    expect(document.documentElement.outerHTML).not.toContain(
      "fixture-secret-never-in-dom-after-submit",
    );
  });

  it("prefills complete available canonical config and renders drift/auth approvals and redacted fragments", async () => {
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
      discoverModels: vi.fn().mockResolvedValue({
        ...discovery,
        existing: {
          model_config: configured,
          unavailable_models: {},
          drifted_agents: ["claude"],
        },
      }),
      previewAgents: vi.fn().mockResolvedValue(preview),
      writeAgents: vi.fn().mockResolvedValue({
        transaction_id: "tx",
        agents: [
          {
            agent: "claude",
            success: true,
            changed: ["/safe/settings.json"],
          },
        ],
      }),
    });
    await reachCredential(api);
    fireEvent.change(screen.getByLabelText(/API (?:key|密钥)/), {
      target: { value: "prefill-secret" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /发现模型|Discover models/ }),
    );
    expect(
      (await screen.findAllByDisplayValue("model-a")).length,
    ).toBeGreaterThan(0);
    fireEvent.click(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    );
    expect(await screen.findByText(/<redacted-api-key>/)).toBeInTheDocument();
    const write = screen.getByRole("button", {
      name: /写入所选 Agent|Write selected Agents/,
    });
    expect(write).toBeDisabled();
    fireEvent.click(screen.getByLabelText(/漂移|drifted/));
    fireEvent.click(screen.getByLabelText(/Codex/));
    fireEvent.click(write);
    await screen.findByText(/Agent 配置结果|Agent configuration result/);
    expect(api.writeAgents).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).not.toContain("prefill-secret");
  });

  it("destroys Rust flow state on cancel and navigation unmount", async () => {
    const { api } = await reachConfigure();
    fireEvent.click(
      screen.getByRole("button", { name: /取消并返回检测|Cancel and return/ }),
    );
    await waitFor(() =>
      expect(api.destroyAgentModelFlow).toHaveBeenCalledWith(discovery.flow_id),
    );
    const view = render(<AgentPage api={api} />);
    view.unmount();
  });

  it("does not destroy the active flow when the page rerenders", async () => {
    const api = createMockApi({
      detectAgents: vi.fn().mockResolvedValue(detection),
      discoverModels: vi.fn().mockResolvedValue(discovery),
    });
    const view = render(<AgentPage api={api} />);
    await screen.findByText("/safe/claude");
    fireEvent.click(
      screen.getByRole("button", {
        name: /继续输入凭据|Continue to credential/,
      }),
    );
    fireEvent.change(screen.getByLabelText(/API (?:key|密钥)/), {
      target: { value: "rerender-secret" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /发现模型|Discover models/ }),
    );
    await screen.findByText(/共同模型目录|Common model catalog/);

    view.rerender(<AgentPage api={api} />);

    expect(api.destroyAgentModelFlow).not.toHaveBeenCalled();
  });

  it("returns stale catalog failures to credential entry without exposing secrets", async () => {
    const { api } = await reachConfigure();
    vi.mocked(api.previewAgents).mockRejectedValueOnce({
      code: "MODEL_CATALOG_STALE",
    });
    const selects = screen.getAllByRole("combobox");
    fireEvent.change(selects[0], { target: { value: "model-a" } });
    fireEvent.change(screen.getByLabelText(/活动模型|Active model/), {
      target: { value: "model-b" },
    });
    const opencodeModel = screen
      .getAllByText("model-a")
      .find((node) =>
        node.closest("label")?.querySelector('input[type="checkbox"]'),
      );
    if (opencodeModel) fireEvent.click(opencodeModel.closest("label")!);
    fireEvent.change(screen.getByLabelText(/默认模型|Default model/), {
      target: { value: "model-a" },
    });
    fireEvent.click(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    );
    expect(await screen.findByLabelText(/API (?:key|密钥)/)).toHaveValue("");
  });

  it("shows a readable validation reason without exposing backend messages", async () => {
    const { api } = await reachConfigure();
    completeRequiredConfig();
    vi.mocked(api.previewAgents).mockRejectedValueOnce({
      code: "MODEL_CONFIG_INVALID",
      message: "unsafe-backend-validation-message",
      details: {
        path: "/claude/max_output_tokens",
        rule: "integer_relationship",
      },
    });
    fireEvent.click(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    );
    expect(
      await screen.findByText(/Claude 最大输出 Token 必须小于上下文窗口/),
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain(
      "unsafe-backend-validation-message",
    );
  });

  it("shows a safe non-empty fallback when validation details are unavailable", async () => {
    const { api } = await reachConfigure();
    completeRequiredConfig();
    vi.mocked(api.previewAgents).mockRejectedValueOnce({
      code: "MODEL_CONFIG_INVALID",
      message: "unsafe-backend-validation-message",
    });
    fireEvent.click(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    );
    expect(
      await screen.findByText(/模型配置不符合要求，请刷新模型目录后重新选择/),
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain(
      "unsafe-backend-validation-message",
    );
  });

  it.each([
    ["CONFIG_INVALID", /现有 Agent 配置无法读取或格式无效/],
    ["CONFIG_NOT_WRITABLE", /Agent 配置路径不可写/],
    ["AGENT_OPERATION_BUSY", /另一个 Agent 配置操作正在进行/],
    ["MANAGER_FAILED", /本地 manager 无法完成预览/],
  ])(
    "does not misclassify %s as a model validation error",
    async (code, reason) => {
      const { api } = await reachConfigure();
      completeRequiredConfig();
      vi.mocked(api.previewAgents).mockRejectedValueOnce({
        code,
        message: "unsafe-backend-preview-message",
      });
      fireEvent.click(
        screen.getByRole("button", {
          name: /生成写入预览|Generate write preview/,
        }),
      );
      expect(await screen.findByText(reason)).toBeInTheDocument();
      expect(document.body.textContent).not.toContain("模型配置无效");
      expect(document.body.textContent).not.toContain(
        "unsafe-backend-preview-message",
      );
    },
  );

  it("returns expired model flows to credential discovery", async () => {
    const { api } = await reachConfigure();
    completeRequiredConfig();
    vi.mocked(api.previewAgents).mockRejectedValueOnce({
      code: "MODEL_FLOW_EXPIRED",
    });
    fireEvent.click(
      screen.getByRole("button", {
        name: /生成写入预览|Generate write preview/,
      }),
    );
    expect(await screen.findByLabelText(/API (?:key|密钥)/)).toHaveValue("");
    expect(screen.getByText(/模型发现会话已失效/)).toBeInTheDocument();
  });

  it.each([
    "MODEL_AUTH_FAILED",
    "MODEL_DISCOVERY_FAILED",
    "MODEL_CATALOG_EMPTY",
  ])(
    "returns %s discovery failure to an empty credential stage",
    async (code) => {
      const secret = `transition-secret-${code}`;
      const api = createMockApi({
        detectAgents: vi.fn().mockResolvedValue(detection),
        discoverModels: vi.fn().mockRejectedValue({ code, message: secret }),
      });
      await reachCredential(api);
      fireEvent.change(screen.getByLabelText(/API (?:key|密钥)/), {
        target: { value: secret },
      });
      fireEvent.click(
        screen.getByRole("button", { name: /发现模型|Discover models/ }),
      );
      expect(await screen.findByLabelText(/API (?:key|密钥)/)).toHaveValue("");
      expect(document.body.textContent).not.toContain(secret);
      expect(localStorage.length).toBe(0);
    },
  );

  it.each([
    ["PREVIEW_STALE", "configure"],
    ["MODEL_NOT_AVAILABLE", "credential"],
    ["MODEL_CATALOG_STALE", "credential"],
    ["MODEL_AUTH_FAILED", "credential"],
  ] as const)(
    "handles %s write failure at the recoverable %s stage",
    async (code, target) => {
      const api = createMockApi({
        detectAgents: vi.fn().mockResolvedValue(detection),
        discoverModels: vi.fn().mockResolvedValue(discovery),
        previewAgents: vi.fn().mockResolvedValue({
          ...preview,
          managed_config_drift: false,
          requires_codex_auth_approval: false,
        }),
        writeAgents: vi
          .fn()
          .mockRejectedValue({ code, message: `write-secret-${code}` }),
      });
      await reachCredential(api);
      fireEvent.change(screen.getByLabelText(/API (?:key|密钥)/), {
        target: { value: "terminal-write-secret" },
      });
      fireEvent.click(
        screen.getByRole("button", { name: /发现模型|Discover models/ }),
      );
      await screen.findByText(/共同模型目录|Common model catalog/);
      completeRequiredConfig();
      fireEvent.click(
        screen.getByRole("button", {
          name: /生成写入预览|Generate write preview/,
        }),
      );
      await screen.findByText(/<redacted-api-key>/);
      fireEvent.click(
        screen.getByRole("button", {
          name: /写入所选 Agent|Write selected Agents/,
        }),
      );
      if (target === "credential") {
        expect(await screen.findByLabelText(/API (?:key|密钥)/)).toHaveValue(
          "",
        );
        expect(api.destroyAgentModelFlow).toHaveBeenCalledWith(
          discovery.flow_id,
        );
      } else {
        expect(
          await screen.findByText(/共同模型目录|Common model catalog/),
        ).toBeInTheDocument();
        expect(api.detectAgents).toHaveBeenCalledTimes(2);
        expect(api.destroyAgentModelFlow).not.toHaveBeenCalled();
      }
      expect(document.documentElement.outerHTML).not.toContain(
        `write-secret-${code}`,
      );
      expect(localStorage.length).toBe(0);
    },
  );
});
