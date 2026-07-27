import { describe, expect, it } from "vitest";

import {
  createExternalSnapshot,
  createPanelBaselines,
  isConfigDirty,
  panelOperationAvailability,
  sameExternalSnapshot,
  targetMode,
} from "./agentPanelState";
import type {
  AgentDetection,
  AgentModelsResult,
  AgentState,
  ClaudeModelConfig,
  ModelConfig,
} from "./ipc";

const claudeConfig: ClaudeModelConfig = {
  primary: { model: "claude-sonnet" },
  haiku: { inherit_primary: true },
  sonnet: { inherit_primary: true },
  opus: { model: "claude-opus", name: "Opus" },
};

function agentStateFor(overrides: Partial<AgentState> = {}): AgentState {
  return {
    agent: "claude",
    name: "Claude Code",
    detected: true,
    command: "/safe/bin/claude",
    path: "/safe/claude/settings.json",
    auth_path: "/safe/claude/auth.json",
    format: "json",
    exists: true,
    writable: true,
    configured: true,
    invalid: false,
    migratable: false,
    recovery: { eligible: false, files: [] },
    ...overrides,
  };
}

function detectionFor(overrides: Partial<AgentState> = {}): AgentDetection {
  return { agents: [agentStateFor(overrides)] };
}

function discoveryFor(
  overrides: {
    existing?: Partial<ModelConfig>;
    preset?: Partial<ModelConfig>;
    unavailable?: string[];
    drifted?: AgentModelsResult["existing"]["drifted_agents"];
  } = {},
): AgentModelsResult {
  return {
    flow_id: "flow-secret-independent",
    models: ["claude-sonnet", "claude-opus"],
    catalog_token: "catalog-secret",
    router_base_url: "http://router.invalid",
    api_base_url: "http://api.invalid/v1",
    existing: {
      model_config: overrides.existing ?? {},
      unavailable_models: { claude: overrides.unavailable ?? [] },
      drifted_agents: overrides.drifted ?? [],
    },
    preset: {
      model_config: overrides.preset ?? {},
      unavailable_agents: {},
    },
  };
}

function configFor(claude: ClaudeModelConfig = claudeConfig): ModelConfig {
  return { version: 1, claude };
}

describe("panel baselines", () => {
  it("keeps form and external baselines independent", () => {
    const detection = detectionFor();
    const discovery = discoveryFor({ existing: { claude: claudeConfig } });
    const baselines = createPanelBaselines("claude", detection, discovery);
    const normalizedPreviewConfig = configFor({
      ...claudeConfig,
      context_window: 200_000,
    });

    expect(isConfigDirty(normalizedPreviewConfig, baselines.form)).toBe(true);
    expect(
      sameExternalSnapshot(
        baselines.external,
        createExternalSnapshot("claude", detection, discovery),
      ),
    ).toBe(true);
  });

  it("sorts unavailable models and selected-Agent drift deterministically", () => {
    const first = createExternalSnapshot(
      "claude",
      detectionFor(),
      discoveryFor({
        existing: { claude: claudeConfig },
        unavailable: ["z-model", "a-model"],
        drifted: ["codex", "claude"],
      }),
    );
    const second = createExternalSnapshot(
      "claude",
      detectionFor(),
      discoveryFor({
        existing: { claude: claudeConfig },
        unavailable: ["a-model", "z-model"],
        drifted: ["claude", "codex"],
      }),
    );

    expect(sameExternalSnapshot(first, second)).toBe(true);
  });

  it("detects selected-Agent existing configuration changes", () => {
    const baseline = createExternalSnapshot(
      "claude",
      detectionFor(),
      discoveryFor({ existing: { claude: claudeConfig } }),
    );
    const changed = createExternalSnapshot(
      "claude",
      detectionFor(),
      discoveryFor({
        existing: {
          claude: {
            ...claudeConfig,
            primary: { model: "different-model" },
          },
        },
      }),
    );

    expect(sameExternalSnapshot(baseline, changed)).toBe(false);
  });

  it("detects selected-Agent unavailable model membership changes", () => {
    const baseline = createExternalSnapshot(
      "claude",
      detectionFor(),
      discoveryFor({ unavailable: ["model-a"] }),
    );
    const changed = createExternalSnapshot(
      "claude",
      detectionFor(),
      discoveryFor({ unavailable: ["model-a", "model-b"] }),
    );

    expect(sameExternalSnapshot(baseline, changed)).toBe(false);
  });

  it("detects selected-Agent drift membership changes", () => {
    const baseline = createExternalSnapshot(
      "claude",
      detectionFor(),
      discoveryFor({ drifted: [] }),
    );
    const changed = createExternalSnapshot(
      "claude",
      detectionFor(),
      discoveryFor({ drifted: ["claude"] }),
    );

    expect(sameExternalSnapshot(baseline, changed)).toBe(false);
  });

  it.each([
    ["path", { path: "/different/settings.json" }],
    ["format", { format: "jsonc" }],
    ["exists", { exists: false }],
    ["writable", { writable: false }],
    ["configured", { configured: false }],
    ["invalid", { invalid: true }],
    [
      "recovery",
      {
        recovery: {
          eligible: true,
          reasons: ["syntax_invalid"],
          files: [
            {
              role: "settings",
              path: "/safe/claude/settings.json",
              format: "json",
              exists: true,
              reasons: ["syntax_invalid"],
            },
          ],
        },
      },
    ],
  ] satisfies Array<[string, Partial<AgentState>]>)(
    "detects a change to relevant detection field %s",
    (_field, change) => {
      const discovery = discoveryFor();
      const original = createExternalSnapshot(
        "claude",
        detectionFor(),
        discovery,
      );

      expect(
        sameExternalSnapshot(
          original,
          createExternalSnapshot("claude", detectionFor(change), discovery),
        ),
      ).toBe(false);
    },
  );

  it("ignores catalog and flow metadata unrelated to configuration content", () => {
    const discovery = discoveryFor({ existing: { claude: claudeConfig } });
    const baseline = createExternalSnapshot(
      "claude",
      detectionFor(),
      discovery,
    );
    const changedDiscovery = {
      ...discovery,
      flow_id: "different-flow",
      catalog_token: "different-catalog",
      router_base_url: "http://different-router.invalid",
      api_base_url: "http://different-api.invalid/v1",
      models: ["different-model"],
    };

    expect(
      sameExternalSnapshot(
        baseline,
        createExternalSnapshot("claude", detectionFor(), changedDiscovery),
      ),
    ).toBe(true);
  });

  it("ignores detected and command because installation does not affect config conflicts", () => {
    const discovery = discoveryFor({ existing: { claude: claudeConfig } });
    const baseline = createExternalSnapshot(
      "claude",
      detectionFor(),
      discovery,
    );

    expect(
      sameExternalSnapshot(
        baseline,
        createExternalSnapshot(
          "claude",
          detectionFor({ detected: false, command: "/different/command" }),
          discovery,
        ),
      ),
    ).toBe(true);
  });

  it("ignores migratable while the current panel mode remains unchanged", () => {
    const discovery = discoveryFor({ existing: { claude: claudeConfig } });
    const baselineDetection = detectionFor({ migratable: false });
    const changedDetection = detectionFor({ migratable: true });

    expect(targetMode(baselineDetection.agents[0])).toBe("merge");
    expect(targetMode(changedDetection.agents[0])).toBe("merge");
    expect(
      sameExternalSnapshot(
        createExternalSnapshot("claude", baselineDetection, discovery),
        createExternalSnapshot("claude", changedDetection, discovery),
      ),
    ).toBe(true);
  });

  it("ignores display and auth metadata unrelated to configuration content", () => {
    const discovery = discoveryFor({ existing: { claude: claudeConfig } });
    const baseline = createExternalSnapshot(
      "claude",
      detectionFor(),
      discovery,
    );

    expect(
      sameExternalSnapshot(
        baseline,
        createExternalSnapshot(
          "claude",
          detectionFor({
            name: "Different label",
            auth_path: "/different/auth.json",
          }),
          discovery,
        ),
      ),
    ).toBe(true);
  });

  it("prefers existing configuration when existing and preset differ", () => {
    const existing = {
      ...claudeConfig,
      primary: { model: "existing-model" },
    };
    const preset = {
      ...claudeConfig,
      primary: { model: "preset-model" },
    };
    const baselines = createPanelBaselines(
      "claude",
      detectionFor(),
      discoveryFor({
        existing: { claude: existing },
        preset: { claude: preset },
      }),
    );

    expect(isConfigDirty(configFor(existing), baselines.form)).toBe(false);
    expect(isConfigDirty(configFor(preset), baselines.form)).toBe(true);
  });

  it("uses preset configuration when existing configuration is absent", () => {
    const baselines = createPanelBaselines(
      "claude",
      detectionFor({ exists: false, configured: false }),
      discoveryFor({ preset: { claude: claudeConfig } }),
    );

    expect(isConfigDirty(configFor(), baselines.form)).toBe(false);
  });

  it("uses the empty single-Agent configuration when no prefill exists", () => {
    const baselines = createPanelBaselines(
      "claude",
      detectionFor({ exists: false, configured: false }),
      discoveryFor(),
    );

    expect(
      isConfigDirty(
        configFor({
          primary: { model: "" },
          haiku: { inherit_primary: true },
          sonnet: { inherit_primary: true },
          opus: { inherit_primary: true },
        }),
        baselines.form,
      ),
    ).toBe(false);
  });

  it("treats an imported draft with reordered object keys as equal", () => {
    const baseline = configFor({
      ...claudeConfig,
      extra: { BETA: "two", ALPHA: "one" },
    });
    const imported = configFor({
      extra: { ALPHA: "one", BETA: "two" },
      opus: { name: "Opus", model: "claude-opus" },
      sonnet: { inherit_primary: true },
      haiku: { inherit_primary: true },
      primary: { model: "claude-sonnet" },
    });

    expect(isConfigDirty(imported, baseline)).toBe(false);
  });
});

describe("targetMode", () => {
  it.each([
    ["writable valid target", {}, "merge"],
    [
      "recoverable invalid target",
      { invalid: true, recovery: { eligible: true, files: [] } },
      "rebuild",
    ],
    ["read-only target", { writable: false }, null],
    ["unrecoverable invalid target", { invalid: true }, null],
  ] satisfies Array<[string, Partial<AgentState>, "merge" | "rebuild" | null]>)(
    "returns the compatible mode for a %s",
    (_name, overrides, expected) => {
      expect(targetMode(agentStateFor(overrides))).toBe(expected);
    },
  );

  it("reports mode incompatibility after detection changes", () => {
    const currentMode = targetMode(agentStateFor());
    const changedMode = targetMode(
      agentStateFor({
        invalid: true,
        recovery: { eligible: true, files: [] },
      }),
    );

    expect(currentMode).toBe("merge");
    expect(changedMode).toBe("rebuild");
    expect(changedMode).not.toBe(currentMode);
  });
});

describe("panel operation availability", () => {
  it("keeps editing and export available while a candidate is checking", () => {
    expect(
      panelOperationAvailability(
        { kind: "editing", refresh: { kind: "checking" } },
        true,
      ),
    ).toEqual({ edit: true, export: true, preview: false, import: false });
  });

  it("allows only safe export for a blocked dirty draft with a live flow", () => {
    expect(
      panelOperationAvailability(
        { kind: "blocked-dirty", canExport: true, errorCode: null },
        true,
      ),
    ).toEqual({ edit: false, export: true, preview: false, import: false });
    expect(
      panelOperationAvailability(
        { kind: "blocked-dirty", canExport: true, errorCode: null },
        false,
      ),
    ).toEqual({ edit: false, export: false, preview: false, import: false });
  });
});
