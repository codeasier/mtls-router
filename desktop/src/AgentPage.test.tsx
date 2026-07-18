import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { AgentPage } from "./AgentPage";
import type {
  AgentDetection,
  AgentModelsResult,
  AgentPreview,
  ModelConfig,
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

describe("Agent model workbench", () => {
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
    const editor = screen.getByLabelText(/opencode extra JSON/);
    fireEvent.change(editor, { target: { value: '{"api-key":"forbidden"}' } });
    expect(await screen.findByText(/protected_path/)).toBeInTheDocument();
    expect(editor).toHaveAttribute("aria-invalid", "true");
    fireEvent.change(editor, { target: { value: '{"status":"active"}' } });
    fireEvent.click(
      editor.closest("details")!.querySelector<HTMLButtonElement>("button")!,
    );
    expect(editor).toHaveValue('{\n  "status": "active"\n}');
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
      }
      expect(document.documentElement.outerHTML).not.toContain(
        `write-secret-${code}`,
      );
      expect(localStorage.length).toBe(0);
    },
  );
});
