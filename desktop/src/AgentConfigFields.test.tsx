import { createRef, useState, type RefObject } from "react";
import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import {
  AgentConfigFields,
  type AgentConfigFieldsHandle,
} from "./AgentConfigFields";
import type { AgentTarget } from "./agentPresentation";
import type { AgentModelsResult, ModelConfig } from "./ipc";
import { renderWithI18n } from "./test/render";

const discovery: AgentModelsResult = {
  flow_id: "flow-fields",
  models: ["model-a", "model-b"],
  catalog_token: "catalog-fields",
  router_base_url: "http://127.0.0.1:19099",
  api_base_url: "http://127.0.0.1:19099/v1",
  existing: {
    model_config: {},
    unavailable_models: {},
    drifted_agents: [],
  },
  preset: { model_config: {}, unavailable_agents: {} },
};

const config: ModelConfig = {
  version: 1,
  opencode: { default_model: "model-a", models: { "model-a": {} } },
};

function ControlledFields({
  initial,
  target,
  fieldsRef,
}: {
  initial: ModelConfig;
  target: AgentTarget;
  fieldsRef: RefObject<AgentConfigFieldsHandle | null>;
}) {
  const [value, setValue] = useState(initial);
  return (
    <AgentConfigFields
      ref={fieldsRef}
      target={target}
      discovery={discovery}
      config={value}
      disabled={false}
      resetToken={0}
      onChange={setValue}
      onDraftStateChange={vi.fn()}
    />
  );
}

describe("AgentConfigFields", () => {
  it.each([
    ["claude", /Claude Code extra JSON/],
    ["codex", /Codex extra JSON/],
  ] as const)(
    "rejects protected top-level extra fields for %s",
    (agent, label) => {
      const ref = createRef<AgentConfigFieldsHandle>();
      const initial: ModelConfig =
        agent === "claude"
          ? {
              version: 1,
              claude: {
                primary: { model: "model-a" },
                haiku: { inherit_primary: true },
                sonnet: { inherit_primary: true },
                opus: { inherit_primary: true },
              },
            }
          : { version: 1, codex: { model: "model-a" } };
      renderWithI18n(
        <AgentConfigFields
          ref={ref}
          target={{ agent, mode: "merge", installedAtEntry: true }}
          discovery={discovery}
          config={initial}
          disabled={false}
          resetToken={0}
          onChange={vi.fn()}
          onDraftStateChange={vi.fn()}
        />,
      );

      const extra = screen.getByLabelText(label);
      fireEvent.change(extra, {
        target: { value: '{"safe":{"api_key":"must-not-pass"}}' },
      });

      expect(extra).toHaveAttribute("aria-invalid", "true");
      expect(ref.current?.getSnapshot().error).toContain("protected_path");
      fireEvent.change(extra, { target: { value: '{"custom":7}' } });
      expect(ref.current?.getSnapshot().config[agent]?.extra).toEqual({
        custom: agent === "claude" ? "7" : 7,
      });
    },
  );

  it("resynchronizes canonical extra JSON when the reset token changes", () => {
    const target = {
      agent: "claude",
      mode: "merge",
      installedAtEntry: true,
    } as const;
    const initial: ModelConfig = {
      version: 1,
      claude: {
        primary: { model: "model-a" },
        haiku: { inherit_primary: true },
        sonnet: { inherit_primary: true },
        opus: { inherit_primary: true },
        extra: { canonical: "old" },
      },
    };
    const view = renderWithI18n(
      <AgentConfigFields
        target={target}
        discovery={discovery}
        config={initial}
        disabled={false}
        resetToken={0}
        onChange={vi.fn()}
        onDraftStateChange={vi.fn()}
      />,
    );
    const refreshed: ModelConfig = {
      ...initial,
      claude: {
        ...initial.claude!,
        extra: { canonical: "new" },
      },
    };

    view.rerender(
      <AgentConfigFields
        target={target}
        discovery={discovery}
        config={refreshed}
        disabled={false}
        resetToken={1}
        onChange={vi.fn()}
        onDraftStateChange={vi.fn()}
      />,
    );

    expect(screen.getByLabelText(/Claude Code extra JSON/)).toHaveValue(
      JSON.stringify({ canonical: "new" }, null, 2),
    );
  });

  it("keeps invalid local JSON in its imperative dirty snapshot", () => {
    const ref = createRef<AgentConfigFieldsHandle>();
    const onDraftStateChange = vi.fn();
    renderWithI18n(
      <AgentConfigFields
        ref={ref}
        target={{ agent: "opencode", mode: "merge", installedAtEntry: true }}
        discovery={discovery}
        config={config}
        disabled={false}
        resetToken={0}
        onChange={vi.fn()}
        onDraftStateChange={onDraftStateChange}
      />,
    );

    const variants = screen.getByLabelText(/Variants JSON/);
    fireEvent.change(variants, { target: { value: '{"broken":' } });

    expect(variants).toHaveAttribute("aria-invalid", "true");
    expect(ref.current?.getSnapshot()).toEqual(
      expect.objectContaining({
        error: expect.any(String),
        hasLocalDraft: true,
      }),
    );
    expect(onDraftStateChange).toHaveBeenLastCalledWith(
      expect.objectContaining({
        error: expect.any(String),
        hasLocalDraft: true,
      }),
    );
  });

  it("does not lose invalid ObjectField text when a sibling changes", () => {
    renderWithI18n(
      <AgentConfigFields
        target={{ agent: "opencode", mode: "merge", installedAtEntry: true }}
        discovery={discovery}
        config={config}
        disabled={false}
        resetToken={0}
        onChange={vi.fn()}
        onDraftStateChange={vi.fn()}
      />,
    );
    const variants = screen.getByLabelText(/Variants JSON/);
    fireEvent.change(variants, { target: { value: '{"broken":' } });
    fireEvent.change(screen.getByLabelText("model-a reasoning"), {
      target: { value: "true" },
    });

    expect(variants).toHaveValue('{"broken":');
    expect(variants).toHaveAttribute("aria-invalid", "true");
  });

  it("round-trips optional Claude Fable through the imperative snapshot", () => {
    const ref = createRef<AgentConfigFieldsHandle>();
    const claude: ModelConfig = {
      version: 1,
      claude: {
        primary: { model: "model-a" },
        haiku: { inherit_primary: true },
        sonnet: { inherit_primary: true },
        opus: { inherit_primary: true },
      },
    };
    renderWithI18n(
      <ControlledFields
        fieldsRef={ref}
        target={{ agent: "claude", mode: "merge", installedAtEntry: true }}
        initial={claude}
      />,
    );
    fireEvent.click(screen.getByLabelText(/启用 Fable/));
    fireEvent.click(screen.getByLabelText(/fable 继承主模型/));
    fireEvent.change(screen.getByLabelText(/fable 模型/), {
      target: { value: "model-b" },
    });
    fireEvent.change(screen.getByLabelText(/claude-fable 显示名称/), {
      target: { value: "Fable display" },
    });

    expect(ref.current?.getSnapshot().config.claude?.fable).toEqual({
      model: "model-b",
      name: "Fable display",
    });
    fireEvent.click(screen.getByLabelText(/启用 Fable/));
    expect(ref.current?.getSnapshot().config.claude?.fable).toBeUndefined();
  });

  it("round-trips constrained OpenCode nested JSON", () => {
    const ref = createRef<AgentConfigFieldsHandle>();
    renderWithI18n(
      <ControlledFields
        fieldsRef={ref}
        target={{ agent: "opencode", mode: "merge", installedAtEntry: true }}
        initial={config}
      />,
    );
    fireEvent.change(screen.getByLabelText(/Variants JSON/), {
      target: { value: '{"fast":{"reasoningEffort":"low"}}' },
    });

    expect(
      ref.current?.getSnapshot().config.opencode?.models["model-a"].variants,
    ).toEqual({ fast: { reasoningEffort: "low" } });
  });

  it("preserves Codex typed omission and round-trips typed values", () => {
    const ref = createRef<AgentConfigFieldsHandle>();
    const codex: ModelConfig = {
      version: 1,
      codex: { model: "model-a" },
    };
    renderWithI18n(
      <ControlledFields
        fieldsRef={ref}
        target={{ agent: "codex", mode: "merge", installedAtEntry: true }}
        initial={codex}
      />,
    );
    expect(screen.getByLabelText(/推理强度/)).toHaveValue("");
    fireEvent.change(screen.getByLabelText(/推理强度/), {
      target: { value: "high" },
    });
    fireEvent.change(screen.getByLabelText(/上下文窗口/), {
      target: { value: "400000" },
    });

    expect(ref.current?.getSnapshot().config.codex).toEqual({
      model: "model-a",
      reasoning_effort: "high",
      context_window: 400000,
    });
  });
});
