import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { useRef, useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  AgentId,
  AgentModelsResult,
  AgentPreview,
  DesktopApi,
  ModelConfig,
} from "./ipc";
import { AgentPanel } from "./AgentPanel";
import { createMockApi } from "./test/api";
import { renderWithI18n } from "./test/render";
import { useAgentPanelController } from "./useAgentPanelController";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

const detection = {
  agents: (["claude", "opencode", "codex"] as const).map((agent) => ({
    agent,
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

const configs: Record<AgentId, ModelConfig> = {
  claude: {
    version: 1,
    claude: {
      primary: { model: "existing-claude" },
      haiku: { inherit_primary: true },
      sonnet: { inherit_primary: true },
      opus: { inherit_primary: true },
    },
  },
  opencode: {
    version: 1,
    opencode: { default_model: "existing-opencode", models: {} },
  },
  codex: { version: 1, codex: { model: "existing-codex" } },
};

function discoveryFor(
  flowId: string,
  existing: Partial<ModelConfig> = {},
  preset: Partial<ModelConfig> = {},
): AgentModelsResult {
  return {
    flow_id: flowId,
    models: ["existing-claude", "preset-claude"],
    catalog_token: `catalog-${flowId}`,
    router_base_url: "http://127.0.0.1:19099",
    api_base_url: "http://127.0.0.1:19099/v1",
    existing: {
      model_config: existing,
      unavailable_models: {},
      drifted_agents: [],
    },
    preset: { model_config: preset, unavailable_agents: {} },
  };
}

function previewFor(
  config: ModelConfig,
  overrides: Partial<AgentPreview> = {},
): AgentPreview {
  return {
    revision_token: "revision",
    model_config: config,
    fragments: [],
    files: [],
    managed_config_drift: false,
    drifted_agents: [],
    managed_collisions: [],
    requires_codex_auth_approval: false,
    ...overrides,
  };
}

function oversizedJsonFile() {
  const file = new File(["{}"], "config.json");
  Object.defineProperty(file, "size", { value: 2 * 1024 * 1024 + 1 });
  return file;
}

function readyApi(overrides: Partial<DesktopApi> = {}) {
  return createMockApi({
    detectAgents: vi.fn().mockResolvedValue(detection),
    discoverModels: vi
      .fn()
      .mockResolvedValue(discoveryFor("flow-claude", configs.claude)),
    ...overrides,
  });
}

function Harness({
  api,
  target,
  onDirtyChange = () => undefined,
}: {
  api: DesktopApi;
  target: AgentId;
  onDirtyChange?: (dirty: boolean) => void;
}) {
  const controller = useAgentPanelController({ api, target, onDirtyChange });
  const firstRefresh = useRef<Promise<void> | null>(null);
  const [sameRefreshPromise, setSameRefreshPromise] = useState<boolean | null>(
    null,
  );
  const [settledRefreshes, setSettledRefreshes] = useState(0);
  return (
    <>
      <output data-testid="phase">{JSON.stringify(controller.phase)}</output>
      <output data-testid="mode">{controller.target?.mode ?? ""}</output>
      <output data-testid="source">{controller.source ?? ""}</output>
      <output data-testid="flow">{controller.discovery?.flow_id ?? ""}</output>
      <output data-testid="config">{JSON.stringify(controller.config)}</output>
      <output data-testid="dirty">{String(controller.dirty)}</output>
      <output data-testid="operations">
        {JSON.stringify(controller.operations)}
      </output>
      <output data-testid="draft-error">{controller.draftState.error}</output>
      <output data-testid="preview">
        {controller.preview?.revision_token ?? ""}
      </output>
      <output data-testid="result">
        {controller.result?.transaction_id ?? ""}
      </output>
      <output data-testid="issue">{JSON.stringify(controller.issue)}</output>
      <output data-testid="same-refresh-promise">
        {String(sameRefreshPromise)}
      </output>
      <output data-testid="settled-refreshes">{settledRefreshes}</output>
      <button
        onClick={() =>
          controller.setConfig({ version: 1, codex: { model: "changed" } })
        }
      >
        change
      </button>
      <button
        onClick={() =>
          controller.setConfig({
            version: 1,
            claude: {
              ...configs.claude.claude!,
              primary: { model: "candidate-c" },
            },
          })
        }
      >
        change-to-candidate-c
      </button>
      <button
        onClick={() =>
          controller.setConfig({
            version: 1,
            claude: {
              ...configs.claude.claude!,
              primary: { model: "draft-b" },
            },
          })
        }
      >
        change-to-draft-b
      </button>
      <button onClick={() => void controller.refresh()}>refresh</button>
      <button
        onClick={() => {
          const promise = controller.refresh();
          firstRefresh.current = promise;
          void promise.then(() => setSettledRefreshes((count) => count + 1));
        }}
      >
        refresh-first
      </button>
      <button
        onClick={() => {
          const promise = controller.refresh();
          setSameRefreshPromise(promise === firstRefresh.current);
          void promise.then(() => setSettledRefreshes((count) => count + 1));
        }}
      >
        refresh-second
      </button>
      <button onClick={() => controller.resolveConflict("preserve")}>
        preserve
      </button>
      <button onClick={() => controller.resolveConflict("discard")}>
        discard
      </button>
      <button onClick={() => void controller.discardBlockedDraft()}>
        discard-blocked
      </button>
      <button onClick={() => void controller.retryBlockedDraft()}>
        retry-blocked
      </button>
      <button onClick={() => void controller.generatePreview()}>preview</button>
      <button onClick={controller.returnToEditing}>return-edit</button>
      <button onClick={controller.dismissResult}>dismiss-result</button>
      <button
        onClick={() =>
          void controller.write({
            managedOverwrite: Boolean(
              controller.preview?.drifted_agents.length,
            ),
            codexAuthChange: Boolean(
              controller.preview?.requires_codex_auth_approval,
            ),
            rebuild: controller.target?.mode === "rebuild" ? [target] : [],
          })
        }
      >
        write
      </button>
      <button
        onClick={() =>
          void controller.write({
            managedOverwrite: !controller.preview?.drifted_agents.length,
            codexAuthChange: !controller.preview?.requires_codex_auth_approval,
            rebuild: [target],
          })
        }
      >
        write-invalid-approvals
      </button>
      <button
        onClick={() =>
          void controller.importConfig(
            new File(
              [
                JSON.stringify({
                  version: 1,
                  claude: {
                    ...configs.claude.claude,
                    primary: { model: "imported" },
                  },
                }),
              ],
              "config.json",
              { type: "application/json" },
            ),
          )
        }
      >
        import
      </button>
      <button onClick={() => void controller.exportConfig()}>export</button>
      <button onClick={() => controller.setConfig(configs.claude)}>
        restore
      </button>
      <button
        onClick={() =>
          controller.setDraftState({
            error: "required model",
            hasLocalDraft: false,
          })
        }
      >
        required-error
      </button>
      <button
        onClick={() =>
          controller.setDraftState({
            error: "invalid JSON",
            hasLocalDraft: true,
          })
        }
      >
        invalid-local-draft
      </button>
      <button
        onClick={() =>
          controller.setDraftState({ error: "", hasLocalDraft: false })
        }
      >
        clear-draft
      </button>
    </>
  );
}

function phase() {
  return JSON.parse(screen.getByTestId("phase").textContent ?? "null") as {
    kind: string;
    reason?: { kind: string; code?: string };
    refresh?: { kind: string; code?: string };
    canExport?: boolean;
    errorCode?: string | null;
  };
}

describe("useAgentPanelController", () => {
  afterEach(() => {
    vi.useRealTimers();
  });
  it("loads strictly as detect -> credential summary -> discover", async () => {
    const detect = deferred<typeof detection>();
    const credential =
      deferred<Awaited<ReturnType<DesktopApi["getCredential"]>>>();
    const discover = deferred<AgentModelsResult>();
    const api = readyApi({
      detectAgents: vi.fn(() => detect.promise),
      getCredential: vi.fn(() => credential.promise),
      discoverModels: vi.fn(() => discover.promise),
    });
    render(<Harness api={api} target="claude" />);

    expect(phase()).toEqual({ kind: "loading" });
    expect(api.getCredential).not.toHaveBeenCalled();
    await act(async () => detect.resolve(detection));
    expect(api.getCredential).toHaveBeenCalledTimes(1);
    expect(api.discoverModels).not.toHaveBeenCalled();
    await act(async () =>
      credential.resolve({
        present: true,
        fingerprint: "ABCD",
        saved_at: "2026-07-27T00:00:00Z",
      }),
    );
    expect(api.discoverModels).toHaveBeenCalledWith(["claude"]);
    await act(async () =>
      discover.resolve(discoveryFor("flow-ordered", configs.claude)),
    );

    expect(phase()).toEqual({ kind: "editing", refresh: { kind: "idle" } });
    expect(screen.getByTestId("flow")).toHaveTextContent("flow-ordered");
  });

  it("requires complete detection before reading credentials", async () => {
    const api = readyApi({
      detectAgents: vi
        .fn()
        .mockResolvedValue({ agents: detection.agents.slice(0, 2) }),
    });
    render(<Harness api={api} target="claude" />);

    await waitFor(() => expect(phase().kind).toBe("readonly"));
    expect(phase().reason).toEqual({
      kind: "catalog",
      code: "AGENT_DETECT_FAILED",
    });
    expect(api.getCredential).not.toHaveBeenCalled();
    expect(api.discoverModels).not.toHaveBeenCalled();
  });

  it("stops without discovery when the credential is absent", async () => {
    const api = readyApi({
      getCredential: vi.fn().mockResolvedValue({
        present: false,
        fingerprint: "",
        saved_at: null,
      }),
    });
    render(<Harness api={api} target="claude" />);

    await waitFor(() => expect(phase().kind).toBe("readonly"));
    expect(phase().reason).toEqual({
      kind: "credential",
      code: "CREDENTIAL_NOT_FOUND",
    });
    expect(api.discoverModels).not.toHaveBeenCalled();
  });

  it.each(["MODEL_AUTH_FAILED", "MODEL_DISCOVERY_FAILED"])(
    "preserves discover rejection code %s without exposing its message",
    async (code) => {
      const secret = "sk-super-secret-upstream-message";
      const api = readyApi({
        discoverModels: vi.fn().mockRejectedValue({ code, message: secret }),
      });
      render(<Harness api={api} target="claude" />);

      await waitFor(() => expect(phase().kind).toBe("readonly"));
      expect(phase().reason).toEqual({ kind: "catalog", code });
      expect(document.body).not.toHaveTextContent(secret);
    },
  );

  it.each([
    ["read-only directory", { writable: false }, { kind: "not-writable" }],
    [
      "unrecoverable invalid directory",
      { invalid: true, recovery: { eligible: false, files: [] } },
      { kind: "not-recoverable" },
    ],
  ])("makes a %s readonly", async (_name, overrides, reason) => {
    const api = readyApi({
      detectAgents: vi.fn().mockResolvedValue({
        agents: detection.agents.map((state) =>
          state.agent === "claude" ? { ...state, ...overrides } : state,
        ),
      }),
    });
    render(<Harness api={api} target="claude" />);

    await waitFor(() => expect(phase().kind).toBe("readonly"));
    expect(phase().reason).toEqual(reason);
    expect(api.getCredential).toHaveBeenCalledTimes(1);
    expect(api.discoverModels).not.toHaveBeenCalled();
  });

  it.each([
    [
      "existing",
      configs.claude.claude,
      { ...configs.claude.claude!, primary: { model: "preset-claude" } },
    ],
    ["preset", undefined, configs.claude.claude],
    ["empty", undefined, undefined],
  ] as const)("initializes from %s", async (source, existing, preset) => {
    const api = readyApi({
      discoverModels: vi
        .fn()
        .mockResolvedValue(
          discoveryFor(
            `flow-${source}`,
            existing ? { claude: existing } : {},
            preset ? { claude: preset } : {},
          ),
        ),
    });
    render(<Harness api={api} target="claude" />);

    await waitFor(() =>
      expect(screen.getByTestId("source")).toHaveTextContent(source),
    );
    const config = JSON.parse(
      screen.getByTestId("config").textContent ?? "null",
    ) as ModelConfig;
    expect(config.claude?.primary.model).toBe(
      source === "existing"
        ? "existing-claude"
        : source === "preset"
          ? "existing-claude"
          : "",
    );
  });

  it.each([
    ["merge", {}],
    ["rebuild", { invalid: true, recovery: { eligible: true, files: [] } }],
  ] as const)("selects %s from detection", async (mode, overrides) => {
    const api = readyApi({
      detectAgents: vi.fn().mockResolvedValue({
        agents: detection.agents.map((state) =>
          state.agent === "claude" ? { ...state, ...overrides } : state,
        ),
      }),
    });
    render(<Harness api={api} target="claude" />);

    await waitFor(() =>
      expect(screen.getByTestId("mode")).toHaveTextContent(mode),
    );
  });

  it("reports typed config and local draft dirtiness, recovery, and unmount cleanup", async () => {
    const onDirtyChange = vi.fn();
    const api = readyApi();
    const view = render(
      <Harness api={api} target="claude" onDirtyChange={onDirtyChange} />,
    );
    await waitFor(() => expect(phase().kind).toBe("editing"));
    expect(screen.getByTestId("dirty")).toHaveTextContent("false");

    fireEvent.click(screen.getByRole("button", { name: "required-error" }));
    expect(screen.getByTestId("draft-error")).toHaveTextContent(
      "required model",
    );
    expect(screen.getByTestId("dirty")).toHaveTextContent("false");

    fireEvent.click(
      screen.getByRole("button", { name: "invalid-local-draft" }),
    );
    expect(screen.getByTestId("dirty")).toHaveTextContent("true");
    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(true));
    fireEvent.click(screen.getByRole("button", { name: "clear-draft" }));
    expect(screen.getByTestId("dirty")).toHaveTextContent("false");
    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(false));

    fireEvent.click(screen.getByRole("button", { name: "change" }));
    expect(screen.getByTestId("dirty")).toHaveTextContent("true");
    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(true));
    fireEvent.click(screen.getByRole("button", { name: "restore" }));
    expect(screen.getByTestId("dirty")).toHaveTextContent("false");
    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(false));

    fireEvent.click(screen.getByRole("button", { name: "change" }));
    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(true));
    view.unmount();
    expect(onDirtyChange).toHaveBeenLastCalledWith(false);
  });

  it("destroys the active flow on unmount", async () => {
    const api = readyApi();
    const view = render(<Harness api={api} target="claude" />);
    await waitFor(() =>
      expect(screen.getByTestId("flow")).toHaveTextContent("flow-claude"),
    );

    view.unmount();
    await waitFor(() =>
      expect(api.destroyAgentModelFlow).toHaveBeenCalledWith("flow-claude"),
    );
  });

  it("cleans the old owner when target or API identity changes", async () => {
    const firstApi = readyApi();
    const secondApi = readyApi({
      discoverModels: vi
        .fn()
        .mockResolvedValue(discoveryFor("flow-opencode", configs.opencode)),
    });
    const view = render(<Harness api={firstApi} target="claude" />);
    await waitFor(() =>
      expect(screen.getByTestId("flow")).toHaveTextContent("flow-claude"),
    );

    view.rerender(<Harness api={firstApi} target="opencode" />);
    expect(phase()).toEqual({ kind: "loading" });
    expect(screen.getByTestId("flow")).toHaveTextContent("");
    expect(screen.getByTestId("config")).toHaveTextContent("null");
    await waitFor(() =>
      expect(firstApi.destroyAgentModelFlow).toHaveBeenCalledWith(
        "flow-claude",
      ),
    );
    await waitFor(() =>
      expect(screen.getByTestId("flow")).toHaveTextContent("flow-claude"),
    );
    view.rerender(<Harness api={secondApi} target="opencode" />);
    expect(phase()).toEqual({ kind: "loading" });
    expect(screen.getByTestId("flow")).toHaveTextContent("");
    expect(screen.getByTestId("config")).toHaveTextContent("null");
    await waitFor(() =>
      expect(firstApi.destroyAgentModelFlow).toHaveBeenCalledTimes(2),
    );
    await waitFor(() =>
      expect(screen.getByTestId("flow")).toHaveTextContent("flow-opencode"),
    );
  });

  it("self-destroys a successful discovery that arrives after supersession", async () => {
    const late = deferred<AgentModelsResult>();
    const replacement = deferred<AgentModelsResult>();
    const api = readyApi({
      discoverModels: vi
        .fn()
        .mockImplementationOnce(() => late.promise)
        .mockImplementationOnce(() => replacement.promise),
    });
    const view = render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(api.discoverModels).toHaveBeenCalledTimes(1));

    view.rerender(<Harness api={api} target="codex" />);
    await act(async () =>
      late.resolve(discoveryFor("flow-late", configs.claude)),
    );
    await waitFor(() =>
      expect(api.destroyAgentModelFlow).toHaveBeenCalledWith("flow-late"),
    );
    expect(screen.getByTestId("flow")).not.toHaveTextContent("flow-late");
  });

  it("retains an in-flight destroy through final unmount and retries after rejection", async () => {
    const firstDestroy = deferred<void>();
    const destroy = vi
      .fn()
      .mockImplementationOnce(() => firstDestroy.promise)
      .mockResolvedValueOnce(undefined);
    const api = readyApi({ destroyAgentModelFlow: destroy });
    const view = render(<Harness api={api} target="claude" />);
    await waitFor(() =>
      expect(screen.getByTestId("flow")).toHaveTextContent("flow-claude"),
    );

    view.unmount();
    await waitFor(() => expect(destroy).toHaveBeenCalledTimes(1));
    expect(destroy).toHaveBeenLastCalledWith("flow-claude");

    await act(async () => firstDestroy.reject(new Error("busy")));
    expect(destroy).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(destroy).toHaveBeenCalledTimes(2));
    expect(destroy).toHaveBeenNthCalledWith(1, "flow-claude");
    expect(destroy).toHaveBeenNthCalledWith(2, "flow-claude");
  });

  it("subscribes to native focus and throttles candidate starts for 15 seconds", async () => {
    vi.useFakeTimers();
    let focus = () => undefined;
    const api = readyApi({
      subscribeMainWindowFocused: vi.fn(async (listener) => {
        focus = listener;
        return () => undefined;
      }),
    });
    render(<Harness api={api} target="claude" />);
    await act(async () => Promise.resolve());
    await act(async () => Promise.resolve());
    expect(screen.getByTestId("flow")).toHaveTextContent("flow-claude");
    vi.mocked(api.detectAgents).mockClear();
    vi.mocked(api.discoverModels).mockClear();

    act(() => {
      focus();
      focus();
    });
    await act(async () => Promise.resolve());
    expect(api.detectAgents).toHaveBeenCalledTimes(1);
    expect(api.discoverModels).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(14_999);
    act(() => focus());
    expect(api.detectAgents).toHaveBeenCalledTimes(1);
    expect(api.discoverModels).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    act(() => focus());
    await act(async () => Promise.resolve());
    expect(api.detectAgents).toHaveBeenCalledTimes(2);
    expect(api.discoverModels).toHaveBeenCalledTimes(2);
  });

  it("lets manual refresh bypass an established interval and returns one in-flight promise", async () => {
    const candidate = deferred<AgentModelsResult>();
    let focus = () => undefined;
    const api = readyApi({
      subscribeMainWindowFocused: vi.fn(async (listener) => {
        focus = listener;
        return () => undefined;
      }),
    });
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    act(() => focus());
    await waitFor(() => expect(api.discoverModels).toHaveBeenCalledTimes(2));
    vi.mocked(api.discoverModels).mockImplementationOnce(
      () => candidate.promise,
    );

    fireEvent.click(screen.getByRole("button", { name: "refresh-first" }));
    await waitFor(() => expect(phase().refresh).toEqual({ kind: "checking" }));
    expect(
      JSON.parse(screen.getByTestId("operations").textContent ?? "{}"),
    ).toEqual({
      edit: true,
      export: true,
      preview: false,
      import: false,
    });
    fireEvent.click(screen.getByRole("button", { name: "change" }));
    fireEvent.click(screen.getByRole("button", { name: "refresh-second" }));
    expect(screen.getByTestId("same-refresh-promise")).toHaveTextContent(
      "true",
    );
    expect(screen.getByTestId("settled-refreshes")).toHaveTextContent("0");
    expect(api.discoverModels).toHaveBeenCalledTimes(3);

    await act(async () =>
      candidate.resolve(discoveryFor("flow-candidate", configs.claude)),
    );
    expect(screen.getByTestId("settled-refreshes")).toHaveTextContent("2");
    expect(screen.getByTestId("flow")).toHaveTextContent("flow-candidate");
    expect(screen.getByTestId("dirty")).toHaveTextContent("true");
  });

  it("adopts a clean candidate and explicitly destroys the old active flow", async () => {
    const api = readyApi();
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    vi.mocked(api.discoverModels).mockResolvedValueOnce(
      discoveryFor("flow-new", {
        claude: { ...configs.claude.claude!, primary: { model: "external" } },
      }),
    );

    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    await waitFor(() =>
      expect(screen.getByTestId("flow")).toHaveTextContent("flow-new"),
    );
    expect(screen.getByTestId("config")).toHaveTextContent("external");
    expect(screen.getByTestId("dirty")).toHaveTextContent("false");
    expect(api.destroyAgentModelFlow).toHaveBeenCalledWith("flow-claude");
  });

  it("moves a clean panel to readonly without requesting models when detection becomes unwritable", async () => {
    const api = readyApi();
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    vi.mocked(api.detectAgents).mockResolvedValueOnce({
      agents: detection.agents.map((state) =>
        state.agent === "claude" ? { ...state, writable: false } : state,
      ),
    });

    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    await waitFor(() => expect(phase().kind).toBe("readonly"));
    expect(phase().reason).toEqual({ kind: "not-writable" });
    expect(api.discoverModels).toHaveBeenCalledTimes(1);
    expect(api.destroyAgentModelFlow).toHaveBeenCalledWith("flow-claude");
    expect(screen.getByTestId("config")).toHaveTextContent("null");
  });

  it("preserves a dirty draft for the same snapshot and adopts the new flow", async () => {
    const api = readyApi();
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "change" }));
    vi.mocked(api.discoverModels).mockResolvedValueOnce(
      discoveryFor("flow-same", configs.claude),
    );

    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    await waitFor(() =>
      expect(screen.getByTestId("flow")).toHaveTextContent("flow-same"),
    );
    expect(screen.getByTestId("config")).toHaveTextContent("changed");
    expect(phase()).toEqual({ kind: "editing", refresh: { kind: "idle" } });
  });

  it("keeps the original form baseline when preserving a dirty conflict", async () => {
    const api = readyApi();
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "change-to-draft-b" }));
    vi.mocked(api.discoverModels).mockResolvedValueOnce(
      discoveryFor("flow-conflict", {
        claude: {
          ...configs.claude.claude!,
          primary: { model: "candidate-c" },
        },
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    await waitFor(() => expect(phase().refresh?.kind).toBe("conflict"));
    expect(screen.getByTestId("config")).toHaveTextContent("draft-b");

    fireEvent.click(screen.getByRole("button", { name: "preserve" }));
    expect(phase()).toEqual({ kind: "editing", refresh: { kind: "idle" } });
    expect(screen.getByTestId("config")).toHaveTextContent("draft-b");
    expect(screen.getByTestId("dirty")).toHaveTextContent("true");

    fireEvent.click(
      screen.getByRole("button", { name: "change-to-candidate-c" }),
    );
    expect(screen.getByTestId("dirty")).toHaveTextContent("true");
  });

  it("discards a dirty conflict into the candidate config", async () => {
    const api = readyApi();
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "change" }));

    vi.mocked(api.discoverModels).mockResolvedValueOnce(
      discoveryFor("flow-conflict-2", {
        claude: { ...configs.claude.claude!, primary: { model: "newer" } },
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    await waitFor(() => expect(phase().refresh?.kind).toBe("conflict"));
    fireEvent.click(screen.getByRole("button", { name: "discard" }));
    expect(screen.getByTestId("config")).toHaveTextContent("newer");
    expect(screen.getByTestId("dirty")).toHaveTextContent("false");
  });

  it("blocks a dirty draft on incompatible detection and discards into readonly", async () => {
    const api = readyApi();
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "change" }));
    vi.mocked(api.detectAgents).mockResolvedValueOnce({
      agents: detection.agents.map((state) =>
        state.agent === "claude" ? { ...state, writable: false } : state,
      ),
    });

    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    await waitFor(() =>
      expect(phase()).toEqual({
        kind: "blocked-dirty",
        canExport: true,
        errorCode: null,
      }),
    );
    expect(api.discoverModels).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId("config")).toHaveTextContent("changed");
    fireEvent.click(screen.getByRole("button", { name: "discard-blocked" }));
    await waitFor(() => expect(phase().kind).toBe("readonly"));
    expect(phase().reason).toEqual({ kind: "not-writable" });
    expect(api.destroyAgentModelFlow).toHaveBeenCalledWith("flow-claude");
  });

  it("only restores a blocked dirty draft after detection is compatible again", async () => {
    const blockedDetection = {
      agents: detection.agents.map((state) =>
        state.agent === "claude" ? { ...state, writable: false } : state,
      ),
    };
    const api = readyApi();
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "change" }));
    vi.mocked(api.detectAgents).mockResolvedValueOnce(blockedDetection);
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    await waitFor(() => expect(phase().kind).toBe("blocked-dirty"));

    vi.mocked(api.detectAgents).mockResolvedValueOnce(blockedDetection);
    fireEvent.click(screen.getByRole("button", { name: "retry-blocked" }));
    await waitFor(() => expect(api.detectAgents).toHaveBeenCalledTimes(3));
    expect(phase().kind).toBe("blocked-dirty");
    expect(api.discoverModels).toHaveBeenCalledTimes(1);

    vi.mocked(api.discoverModels).mockResolvedValueOnce(
      discoveryFor("flow-restored", configs.claude),
    );
    fireEvent.click(screen.getByRole("button", { name: "retry-blocked" }));
    await waitFor(() =>
      expect(screen.getByTestId("flow")).toHaveTextContent("flow-restored"),
    );
    expect(phase()).toEqual({ kind: "editing", refresh: { kind: "idle" } });
    expect(screen.getByTestId("config")).toHaveTextContent("changed");
  });

  it.each([
    ["detection", "AGENT_DETECT_IO"],
    ["model discovery", "MODEL_AUTH_FAILED"],
    ["empty flow", "MODEL_RESPONSE_INVALID"],
  ] as const)(
    "keeps a blocked dirty draft and exposes stable %s retry failure",
    async (failure, expectedCode) => {
      const api = readyApi();
      render(<Harness api={api} target="claude" />);
      await waitFor(() => expect(phase().kind).toBe("editing"));
      fireEvent.click(screen.getByRole("button", { name: "change" }));
      vi.mocked(api.detectAgents).mockResolvedValueOnce({
        agents: detection.agents.map((state) =>
          state.agent === "claude" ? { ...state, writable: false } : state,
        ),
      });
      fireEvent.click(screen.getByRole("button", { name: "refresh" }));
      await waitFor(() => expect(phase().kind).toBe("blocked-dirty"));

      if (failure === "detection") {
        vi.mocked(api.detectAgents).mockRejectedValueOnce({
          code: expectedCode,
        });
      } else if (failure === "model discovery") {
        vi.mocked(api.discoverModels).mockRejectedValueOnce({
          code: expectedCode,
        });
      } else {
        vi.mocked(api.discoverModels).mockResolvedValueOnce(
          discoveryFor("", configs.claude),
        );
      }
      fireEvent.click(screen.getByRole("button", { name: "retry-blocked" }));

      await waitFor(() => expect(phase().errorCode).toBe(expectedCode));
      expect(phase().kind).toBe("blocked-dirty");
      expect(phase().canExport).toBe(true);
      expect(screen.getByTestId("config")).toHaveTextContent("changed");
      expect(screen.getByTestId("dirty")).toHaveTextContent("true");
      expect(screen.getByTestId("flow")).toHaveTextContent("flow-claude");
    },
  );

  it("destroys a clean merge flow before entering fresh recovery discovery", async () => {
    const recoveryDiscovery = deferred<AgentModelsResult>();
    const api = readyApi();
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    vi.mocked(api.detectAgents).mockResolvedValueOnce({
      agents: detection.agents.map((state) =>
        state.agent === "claude"
          ? {
              ...state,
              invalid: true,
              recovery: { eligible: true, files: [] },
            }
          : state,
      ),
    });
    vi.mocked(api.discoverModels).mockImplementationOnce(
      () => recoveryDiscovery.promise,
    );

    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    await waitFor(() => expect(phase().kind).toBe("loading"));
    expect(api.destroyAgentModelFlow).toHaveBeenCalledWith("flow-claude");
    expect(screen.getByTestId("config")).toHaveTextContent("null");
    await act(async () =>
      recoveryDiscovery.resolve(
        discoveryFor("flow-recovery", {}, configs.claude),
      ),
    );
    expect(screen.getByTestId("mode")).toHaveTextContent("rebuild");
    expect(screen.getByTestId("flow")).toHaveTextContent("flow-recovery");
  });

  it("retries a failed old-flow destroy before the next flow operation", async () => {
    const firstDestroy = deferred<void>();
    const destroy = vi
      .fn()
      .mockImplementationOnce(() => firstDestroy.promise)
      .mockResolvedValue(undefined);
    const api = readyApi({ destroyAgentModelFlow: destroy });
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    vi.mocked(api.discoverModels)
      .mockResolvedValueOnce(discoveryFor("flow-new", configs.claude))
      .mockResolvedValueOnce(discoveryFor("flow-newer", configs.claude));
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    await waitFor(() => expect(destroy).toHaveBeenCalledWith("flow-claude"));
    vi.useFakeTimers();
    await act(async () => firstDestroy.reject(new Error("busy")));
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    await act(async () => Promise.resolve());
    expect(
      destroy.mock.calls.filter(([flowId]) => flowId === "flow-claude"),
    ).toHaveLength(2);
    expect(destroy.mock.invocationCallOrder[1]).toBeLessThan(
      vi.mocked(api.discoverModels).mock.invocationCallOrder[2],
    );
  });

  it("keeps the old active state and reports failure when refresh cannot verify changes", async () => {
    const api = readyApi();
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    vi.mocked(api.discoverModels).mockRejectedValueOnce({
      code: "MODEL_AUTH_FAILED",
    });
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));

    await waitFor(() =>
      expect(phase()).toEqual({
        kind: "editing",
        refresh: { kind: "failed", code: "MODEL_AUTH_FAILED" },
      }),
    );
    expect(screen.getByTestId("flow")).toHaveTextContent("flow-claude");
  });

  it.each([
    ["detection", "AGENT_DETECT_IO"],
    ["empty flow", "MODEL_RESPONSE_INVALID"],
  ] as const)(
    "keeps the active state when refresh fails during %s",
    async (failure, expectedCode) => {
      const api = readyApi();
      render(<Harness api={api} target="claude" />);
      await waitFor(() => expect(phase().kind).toBe("editing"));
      if (failure === "detection") {
        vi.mocked(api.detectAgents).mockRejectedValueOnce({
          code: expectedCode,
        });
      } else {
        vi.mocked(api.discoverModels).mockResolvedValueOnce(
          discoveryFor("", configs.claude),
        );
      }
      fireEvent.click(screen.getByRole("button", { name: "refresh" }));

      await waitFor(() =>
        expect(phase()).toEqual({
          kind: "editing",
          refresh: { kind: "failed", code: expectedCode },
        }),
      );
      expect(screen.getByTestId("flow")).toHaveTextContent("flow-claude");
      expect(screen.getByTestId("config")).toHaveTextContent("existing-claude");
    },
  );

  it("binds preview to the active flow, keeps normalization dirty, and drains one pending refresh after return", async () => {
    const normalized = {
      ...configs.claude,
      claude: {
        ...configs.claude.claude!,
        primary: { model: "existing-claude", name: "Normalized" },
      },
    };
    const pendingPreview = deferred<AgentPreview>();
    const api = readyApi({
      previewAgents: vi.fn(() => pendingPreview.promise),
    });
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));

    fireEvent.click(screen.getByRole("button", { name: "preview" }));
    expect(phase().kind).toBe("preview-loading");
    fireEvent.click(screen.getByRole("button", { name: "preview" }));
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    expect(api.previewAgents).toHaveBeenCalledTimes(1);
    expect(api.previewAgents).toHaveBeenCalledWith(
      ["claude"],
      "flow-claude",
      "catalog-flow-claude",
      configs.claude,
      { claude: "merge" },
    );
    expect(api.discoverModels).toHaveBeenCalledTimes(1);

    await act(async () => pendingPreview.resolve(previewFor(normalized)));
    expect(phase().kind).toBe("previewing");
    expect(screen.getByTestId("preview")).toHaveTextContent("revision");
    expect(screen.getByTestId("config")).toHaveTextContent("Normalized");
    expect(screen.getByTestId("dirty")).toHaveTextContent("true");
    expect(api.discoverModels).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "return-edit" }));
    expect(screen.getByTestId("preview")).toHaveTextContent("");
    expect(screen.getByTestId("config")).toHaveTextContent("Normalized");
    await waitFor(() => expect(api.discoverModels).toHaveBeenCalledTimes(2));
  });

  it("validates typed drafts synchronously and invalidates preview on field changes", async () => {
    const api = readyApi({
      previewAgents: vi.fn().mockResolvedValue(previewFor(configs.claude)),
    });
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));

    fireEvent.click(screen.getByRole("button", { name: "change" }));
    expect(screen.getByTestId("draft-error")).not.toHaveTextContent("");
    fireEvent.click(screen.getByRole("button", { name: "preview" }));
    expect(api.previewAgents).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "restore" }));
    expect(screen.getByTestId("draft-error")).toHaveTextContent("");
    fireEvent.click(screen.getByRole("button", { name: "preview" }));
    await waitFor(() => expect(phase().kind).toBe("previewing"));
    fireEvent.click(screen.getByRole("button", { name: "return-edit" }));
    fireEvent.click(screen.getByRole("button", { name: "change-to-draft-b" }));
    expect(screen.getByTestId("preview")).toHaveTextContent("");
  });

  it("imports only the current Agent without rebasing and exports without changing panel state", async () => {
    const createObjectURL = vi.fn().mockReturnValue("blob:safe");
    const revokeObjectURL = vi.fn();
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: createObjectURL,
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: revokeObjectURL,
    });
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(
      () => undefined,
    );
    const api = readyApi({
      importAgentModelConfig: vi.fn().mockResolvedValue({
        ...configs.claude,
        claude: {
          ...configs.claude.claude!,
          primary: { model: "imported" },
        },
      }),
      exportAgentModelConfig: vi.fn().mockResolvedValue('{"safe":true}'),
    });
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));

    fireEvent.click(screen.getByRole("button", { name: "import" }));
    await waitFor(() =>
      expect(screen.getByTestId("source")).toHaveTextContent("imported"),
    );
    expect(api.importAgentModelConfig).toHaveBeenCalledWith(
      expect.any(String),
      ["claude"],
      "flow-claude",
    );
    expect(screen.getByTestId("dirty")).toHaveTextContent("true");
    const stateBeforeExport = document.body.textContent;

    fireEvent.click(screen.getByRole("button", { name: "export" }));
    await waitFor(() => expect(api.exportAgentModelConfig).toHaveBeenCalled());
    expect(api.exportAgentModelConfig).toHaveBeenCalledWith(
      expect.objectContaining({ claude: expect.any(Object) }),
      ["claude"],
      "flow-claude",
    );
    expect(document.body.textContent).toBe(stateBeforeExport);
    expect(createObjectURL).toHaveBeenCalledTimes(1);
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:safe");
  });

  it("marks an equal import as imported but clean and rejects cross-Agent output", async () => {
    const api = readyApi({
      importAgentModelConfig: vi
        .fn()
        .mockResolvedValueOnce(configs.claude)
        .mockResolvedValueOnce({
          ...configs.claude,
          codex: configs.codex.codex,
        }),
    });
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));

    fireEvent.click(screen.getByRole("button", { name: "import" }));
    await waitFor(() =>
      expect(screen.getByTestId("source")).toHaveTextContent("imported"),
    );
    expect(screen.getByTestId("dirty")).toHaveTextContent("false");

    fireEvent.click(screen.getByRole("button", { name: "import" }));
    await waitFor(() =>
      expect(screen.getByTestId("issue")).toHaveTextContent("IMPORT_INVALID"),
    );
    expect(screen.getByTestId("config")).not.toHaveTextContent("codex");
  });

  it.each([
    "PREVIEW_STALE",
    "MODEL_FLOW_EXPIRED",
    "MODEL_CATALOG_STALE",
  ] as const)(
    "recovers %s inside the panel while preserving the draft",
    async (code) => {
      const rediscovery = deferred<AgentModelsResult>();
      const api = readyApi({
        discoverModels: vi
          .fn()
          .mockResolvedValueOnce(discoveryFor("flow-claude", configs.claude))
          .mockImplementationOnce(() => rediscovery.promise),
        previewAgents: vi.fn().mockRejectedValue({
          code,
          message: "sk-canary-secret",
        }),
      });
      render(<Harness api={api} target="claude" />);
      await waitFor(() => expect(phase().kind).toBe("editing"));
      fireEvent.click(
        screen.getByRole("button", { name: "change-to-draft-b" }),
      );
      fireEvent.click(screen.getByRole("button", { name: "preview" }));

      await waitFor(() => expect(api.discoverModels).toHaveBeenCalledTimes(2));
      expect(screen.getByTestId("config")).toHaveTextContent("draft-b");
      expect(screen.getByTestId("preview")).toHaveTextContent("");
      expect(document.body).not.toHaveTextContent("canary-secret");
      if (code !== "PREVIEW_STALE") {
        expect(
          JSON.parse(screen.getByTestId("operations").textContent ?? "{}")
            .export,
        ).toBe(false);
        fireEvent.click(screen.getByRole("button", { name: "export" }));
        expect(api.exportAgentModelConfig).not.toHaveBeenCalled();
        expect(api.destroyAgentModelFlow).toHaveBeenCalledWith("flow-claude");
      }

      await act(async () =>
        rediscovery.resolve(discoveryFor(`flow-${code}`, configs.claude)),
      );
      expect(phase()).toEqual({ kind: "editing", refresh: { kind: "idle" } });
      expect(screen.getByTestId("config")).toHaveTextContent("draft-b");
    },
  );

  it("shows write success before a complete clean reload and does not destroy the consumed flow", async () => {
    const reloadDetection = deferred<typeof detection>();
    const api = readyApi({
      detectAgents: vi
        .fn()
        .mockResolvedValueOnce(detection)
        .mockImplementationOnce(() => reloadDetection.promise),
      discoverModels: vi
        .fn()
        .mockResolvedValueOnce(discoveryFor("flow-claude", configs.claude))
        .mockResolvedValueOnce(
          discoveryFor("flow-after-write", configs.claude),
        ),
      previewAgents: vi.fn().mockResolvedValue(previewFor(configs.claude)),
      writeAgents: vi.fn().mockResolvedValue({
        transaction_id: "tx-success",
        agents: [{ agent: "claude", success: true }],
      }),
    });
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "preview" }));
    await waitFor(() => expect(phase().kind).toBe("previewing"));
    fireEvent.click(screen.getByRole("button", { name: "write" }));

    await waitFor(() => expect(phase().kind).toBe("reloading"));
    expect(api.writeAgents).toHaveBeenCalledWith(
      ["claude"],
      "flow-claude",
      "catalog-flow-claude",
      configs.claude,
      "revision",
      false,
      false,
      [],
    );
    expect(screen.getByTestId("result")).toHaveTextContent("tx-success");
    expect(screen.getByTestId("issue")).toHaveTextContent("success");
    expect(api.destroyAgentModelFlow).not.toHaveBeenCalledWith("flow-claude");
    expect(api.getCredential).toHaveBeenCalledTimes(1);

    await act(async () => reloadDetection.resolve(detection));
    await waitFor(() => expect(phase().kind).toBe("editing"));
    expect(api.getCredential).toHaveBeenCalledTimes(2);
    expect(api.discoverModels).toHaveBeenCalledTimes(2);
    expect(screen.getByTestId("flow")).toHaveTextContent("flow-after-write");
    expect(screen.getByTestId("dirty")).toHaveTextContent("false");
    expect(screen.getByTestId("issue")).toHaveTextContent("success");
  });

  it("coalesces refreshes during writing and reloading into the complete post-write reload", async () => {
    const pendingWrite =
      deferred<Awaited<ReturnType<DesktopApi["writeAgents"]>>>();
    const reloadDetection = deferred<typeof detection>();
    const api = readyApi({
      detectAgents: vi
        .fn()
        .mockResolvedValueOnce(detection)
        .mockImplementationOnce(() => reloadDetection.promise),
      discoverModels: vi
        .fn()
        .mockResolvedValueOnce(discoveryFor("flow-claude", configs.claude))
        .mockResolvedValueOnce(discoveryFor("flow-reloaded", configs.claude)),
      previewAgents: vi.fn().mockResolvedValue(previewFor(configs.claude)),
      writeAgents: vi.fn(() => pendingWrite.promise),
    });
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "preview" }));
    await waitFor(() => expect(phase().kind).toBe("previewing"));
    fireEvent.click(screen.getByRole("button", { name: "write" }));
    expect(phase().kind).toBe("writing");
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    expect(api.detectAgents).toHaveBeenCalledTimes(1);

    await act(async () =>
      pendingWrite.resolve({
        transaction_id: "tx-pending-refresh",
        agents: [{ agent: "claude", success: true }],
      }),
    );
    expect(phase().kind).toBe("reloading");
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    expect(api.detectAgents).toHaveBeenCalledTimes(2);

    await act(async () => reloadDetection.resolve(detection));
    await waitFor(() => expect(phase().kind).toBe("editing"));
    expect(api.detectAgents).toHaveBeenCalledTimes(2);
    expect(api.getCredential).toHaveBeenCalledTimes(2);
    expect(api.discoverModels).toHaveBeenCalledTimes(2);
    expect(screen.getByTestId("flow")).toHaveTextContent("flow-reloaded");
  });

  it("retains write success when reload fails and retries the full entry sequence", async () => {
    const api = readyApi({
      detectAgents: vi
        .fn()
        .mockResolvedValueOnce(detection)
        .mockRejectedValueOnce({ code: "AGENT_DETECT_IO", message: "secret" })
        .mockResolvedValueOnce(detection),
      discoverModels: vi
        .fn()
        .mockResolvedValueOnce(discoveryFor("flow-claude", configs.claude))
        .mockResolvedValueOnce(discoveryFor("flow-retry", configs.claude)),
      previewAgents: vi.fn().mockResolvedValue(previewFor(configs.claude)),
      writeAgents: vi.fn().mockResolvedValue({
        transaction_id: "tx-success",
        agents: [{ agent: "claude", success: true }],
      }),
    });
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "preview" }));
    await waitFor(() => expect(phase().kind).toBe("previewing"));
    fireEvent.click(screen.getByRole("button", { name: "write" }));

    await waitFor(() =>
      expect(phase()).toEqual({
        kind: "reload-failed",
        code: "AGENT_DETECT_IO",
      }),
    );
    expect(screen.getByTestId("result")).toHaveTextContent("tx-success");
    expect(screen.getByTestId("issue")).toHaveTextContent("success");

    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    await waitFor(() => expect(phase().kind).toBe("editing"));
    expect(api.detectAgents).toHaveBeenCalledTimes(3);
    expect(api.getCredential).toHaveBeenCalledTimes(2);
    expect(api.discoverModels).toHaveBeenCalledTimes(2);
    expect(screen.getByTestId("flow")).toHaveTextContent("flow-retry");
    expect(screen.getByTestId("issue")).toHaveTextContent("success");
  });

  it("rejects cross-Agent preview metadata before entering preview", async () => {
    const api = readyApi({
      previewAgents: vi.fn().mockResolvedValue(
        previewFor(configs.claude, {
          files: [
            {
              agent: "codex",
              mode: "merge",
              path: "/safe/codex/config.toml",
              role: "config",
              format: "toml",
              operation: "replace",
            },
          ],
        }),
      ),
    });
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "preview" }));

    await waitFor(() =>
      expect(screen.getByTestId("issue")).toHaveTextContent(
        "MODEL_RESPONSE_INVALID",
      ),
    );
    expect(phase().kind).toBe("editing");
    expect(screen.getByTestId("preview")).toHaveTextContent("");
    expect(api.writeAgents).not.toHaveBeenCalled();
  });

  it("rejects a blank preview revision before entering preview", async () => {
    const api = readyApi({
      previewAgents: vi.fn().mockResolvedValue(
        previewFor(configs.claude, {
          revision_token: "   ",
        }),
      ),
    });
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "preview" }));

    await waitFor(() =>
      expect(screen.getByTestId("issue")).toHaveTextContent(
        "MODEL_RESPONSE_INVALID",
      ),
    );
    expect(phase().kind).toBe("editing");
    expect(api.writeAgents).not.toHaveBeenCalled();
  });

  it("requires approvals to exactly match the accepted preview", async () => {
    const approvedPreview = previewFor(configs.claude, {
      managed_config_drift: true,
      drifted_agents: ["claude"],
      requires_codex_auth_approval: true,
    });
    const api = readyApi({
      previewAgents: vi.fn().mockResolvedValue(approvedPreview),
      writeAgents: vi.fn().mockResolvedValue({
        transaction_id: "tx-approved",
        agents: [{ agent: "claude", success: true }],
      }),
    });
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "preview" }));
    await waitFor(() => expect(phase().kind).toBe("previewing"));

    fireEvent.click(
      screen.getByRole("button", { name: "write-invalid-approvals" }),
    );
    expect(api.writeAgents).not.toHaveBeenCalled();
    expect(screen.getByTestId("issue")).toHaveTextContent("APPROVAL_MISMATCH");
    expect(phase().kind).toBe("previewing");

    fireEvent.click(screen.getByRole("button", { name: "write" }));
    await waitFor(() => expect(api.writeAgents).toHaveBeenCalledTimes(1));
    expect(api.writeAgents).toHaveBeenCalledWith(
      ["claude"],
      "flow-claude",
      "catalog-flow-claude",
      configs.claude,
      "revision",
      true,
      true,
      [],
    );
  });

  it("requires the exact target-only rebuild approval emitted by the pane", async () => {
    const rebuildDetection = {
      agents: detection.agents.map((state) =>
        state.agent === "claude"
          ? {
              ...state,
              invalid: true,
              recovery: { eligible: true, files: [] },
            }
          : state,
      ),
    };
    const api = readyApi({
      detectAgents: vi.fn().mockResolvedValue(rebuildDetection),
      previewAgents: vi.fn().mockResolvedValue(
        previewFor(configs.claude, {
          files: [
            {
              agent: "claude",
              mode: "rebuild",
              path: "/safe/claude/config",
              role: "config",
              format: "json",
              operation: "replace",
            },
          ],
        }),
      ),
      writeAgents: vi.fn().mockResolvedValue({
        transaction_id: "tx-rebuild",
        agents: [{ agent: "claude", success: true }],
      }),
    });
    render(<Harness api={api} target="claude" />);
    await waitFor(() =>
      expect(screen.getByTestId("mode")).toHaveTextContent("rebuild"),
    );
    fireEvent.click(screen.getByRole("button", { name: "preview" }));
    await waitFor(() => expect(phase().kind).toBe("previewing"));
    fireEvent.click(screen.getByRole("button", { name: "write" }));

    await waitFor(() => expect(api.writeAgents).toHaveBeenCalledTimes(1));
    expect(api.writeAgents).toHaveBeenCalledWith(
      ["claude"],
      "flow-claude",
      "catalog-flow-claude",
      configs.claude,
      "revision",
      false,
      false,
      ["claude"],
    );
  });

  it.each([
    [
      "malformed",
      { transaction_id: "tx-malformed", agents: [] },
      "INVALID_RESPONSE",
    ],
    [
      "explicit failure",
      {
        transaction_id: "tx-failed",
        agents: [
          {
            agent: "claude" as const,
            success: false,
            error_code: "WRITE_FAILED",
          },
        ],
      },
      "WRITE_FAILED",
    ],
    [
      "foreign status",
      {
        transaction_id: "tx-foreign",
        agents: [{ agent: "codex" as const, success: true }],
      },
      "INVALID_RESPONSE",
    ],
    [
      "duplicate status",
      {
        transaction_id: "tx-duplicate",
        agents: [
          { agent: "claude" as const, success: true },
          { agent: "claude" as const, success: true },
        ],
      },
      "INVALID_RESPONSE",
    ],
  ] as const)(
    "treats resolved %s writes as consumed and reloads instead of reusing preview",
    async (_name, writeResult, expectedIssue) => {
      const reloadDetection = deferred<typeof detection>();
      const api = readyApi({
        detectAgents: vi
          .fn()
          .mockResolvedValueOnce(detection)
          .mockImplementationOnce(() => reloadDetection.promise),
        discoverModels: vi
          .fn()
          .mockResolvedValueOnce(discoveryFor("flow-claude", configs.claude))
          .mockResolvedValueOnce(discoveryFor("flow-resolved", configs.claude)),
        previewAgents: vi.fn().mockResolvedValue(previewFor(configs.claude)),
        writeAgents: vi.fn().mockResolvedValue(writeResult),
      });
      render(<Harness api={api} target="claude" />);
      await waitFor(() => expect(phase().kind).toBe("editing"));
      fireEvent.click(screen.getByRole("button", { name: "preview" }));
      await waitFor(() => expect(phase().kind).toBe("previewing"));
      fireEvent.click(screen.getByRole("button", { name: "write" }));

      await waitFor(() => expect(phase().kind).toBe("reloading"));
      expect(screen.getByTestId("preview")).toHaveTextContent("");
      expect(screen.getByTestId("issue")).toHaveTextContent(expectedIssue);
      expect(
        JSON.parse(screen.getByTestId("operations").textContent ?? "{}").export,
      ).toBe(false);
      expect(api.destroyAgentModelFlow).not.toHaveBeenCalledWith("flow-claude");
      if (_name === "explicit failure") {
        expect(screen.getByTestId("result")).toHaveTextContent("tx-failed");
      } else {
        expect(screen.getByTestId("result")).toHaveTextContent("");
      }

      await act(async () => reloadDetection.resolve(detection));
      await waitFor(() => expect(phase().kind).toBe("editing"));
      expect(screen.getByTestId("flow")).toHaveTextContent("flow-resolved");
      expect(screen.getByTestId("issue")).toHaveTextContent(expectedIssue);
    },
  );

  it.each([
    "PREVIEW_STALE",
    "MODEL_FLOW_EXPIRED",
    "MODEL_CATALOG_STALE",
    "ROLLBACK_FAILED",
    "UNKNOWN_WRITE_FAILURE",
  ] as const)(
    "recovers rejected write %s with the correct old-flow ownership",
    async (code) => {
      const rediscovery = deferred<AgentModelsResult>();
      const api = readyApi({
        discoverModels: vi
          .fn()
          .mockResolvedValueOnce(discoveryFor("flow-claude", configs.claude))
          .mockImplementationOnce(() => rediscovery.promise),
        previewAgents: vi
          .fn()
          .mockImplementation(async (_agents, _flow, _catalog, config) =>
            previewFor(config),
          ),
        writeAgents: vi.fn().mockRejectedValue({
          code,
          message: "sk-write-canary-secret",
        }),
      });
      render(<Harness api={api} target="claude" />);
      await waitFor(() => expect(phase().kind).toBe("editing"));
      fireEvent.click(
        screen.getByRole("button", { name: "change-to-draft-b" }),
      );
      fireEvent.click(screen.getByRole("button", { name: "preview" }));
      await waitFor(() => expect(phase().kind).toBe("previewing"));
      fireEvent.click(screen.getByRole("button", { name: "write" }));

      await waitFor(() => expect(api.discoverModels).toHaveBeenCalledTimes(2));
      expect(screen.getByTestId("config")).toHaveTextContent("draft-b");
      expect(screen.getByTestId("preview")).toHaveTextContent("");
      expect(screen.getByTestId("issue")).toHaveTextContent(code);
      expect(document.body).not.toHaveTextContent("canary-secret");
      expect(
        JSON.parse(screen.getByTestId("operations").textContent ?? "{}").export,
      ).toBe(code === "PREVIEW_STALE");
      if (code === "PREVIEW_STALE") {
        expect(api.destroyAgentModelFlow).not.toHaveBeenCalledWith(
          "flow-claude",
        );
      } else {
        expect(api.destroyAgentModelFlow).toHaveBeenCalledWith("flow-claude");
      }

      await act(async () =>
        rediscovery.resolve(discoveryFor(`flow-${code}`, configs.claude)),
      );
      await waitFor(() => expect(phase().kind).toBe("editing"));
      expect(screen.getByTestId("config")).toHaveTextContent("draft-b");
      if (code === "PREVIEW_STALE") {
        expect(api.destroyAgentModelFlow).toHaveBeenCalledWith("flow-claude");
      }
    },
  );

  it("commits a dirty successful write as clean before a failed reload", async () => {
    const api = readyApi({
      detectAgents: vi
        .fn()
        .mockResolvedValueOnce(detection)
        .mockRejectedValueOnce({ code: "AGENT_DETECT_IO" }),
      previewAgents: vi
        .fn()
        .mockImplementation(async (_agents, _flow, _catalog, config) =>
          previewFor(config),
        ),
      writeAgents: vi.fn().mockResolvedValue({
        transaction_id: "tx-dirty-success",
        agents: [{ agent: "claude", success: true }],
      }),
    });
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "change-to-draft-b" }));
    expect(screen.getByTestId("dirty")).toHaveTextContent("true");
    fireEvent.click(screen.getByRole("button", { name: "preview" }));
    await waitFor(() => expect(phase().kind).toBe("previewing"));
    fireEvent.click(screen.getByRole("button", { name: "write" }));

    await waitFor(() => expect(phase().kind).toBe("reload-failed"));
    expect(screen.getByTestId("config")).toHaveTextContent("draft-b");
    expect(screen.getByTestId("dirty")).toHaveTextContent("false");
    expect(screen.getByTestId("result")).toHaveTextContent("tx-dirty-success");
    expect(screen.getByTestId("issue")).toHaveTextContent("success");
  });

  it.each(["import", "export"] as const)(
    "drains exactly one pending refresh after %s finishes",
    async (operation) => {
      const pendingImport = deferred<ModelConfig>();
      const pendingExport = deferred<string>();
      Object.defineProperty(URL, "createObjectURL", {
        configurable: true,
        value: vi.fn().mockReturnValue("blob:pending"),
      });
      Object.defineProperty(URL, "revokeObjectURL", {
        configurable: true,
        value: vi.fn(),
      });
      vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(
        () => undefined,
      );
      const api = readyApi({
        importAgentModelConfig: vi.fn(() => pendingImport.promise),
        exportAgentModelConfig: vi.fn(() => pendingExport.promise),
      });
      render(<Harness api={api} target="claude" />);
      await waitFor(() => expect(phase().kind).toBe("editing"));
      fireEvent.click(screen.getByRole("button", { name: operation }));
      fireEvent.click(screen.getByRole("button", { name: "refresh" }));
      fireEvent.click(screen.getByRole("button", { name: "refresh" }));
      expect(api.discoverModels).toHaveBeenCalledTimes(1);

      await act(async () => {
        if (operation === "import") pendingImport.resolve(configs.claude);
        else pendingExport.resolve('{"safe":true}');
      });
      await waitFor(() => expect(api.discoverModels).toHaveBeenCalledTimes(2));
      expect(api.discoverModels).toHaveBeenCalledTimes(2);
    },
  );

  it("ignores old auxiliary results after a target switch without clearing the new owner", async () => {
    const oldExport = deferred<string>();
    const newExport = deferred<string>();
    const createObjectURL = vi.fn().mockReturnValue("blob:new-target");
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: createObjectURL,
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: vi.fn(),
    });
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(
      () => undefined,
    );
    const api = readyApi({
      discoverModels: vi.fn(async (agents: AgentId[]) =>
        discoveryFor(`flow-${agents[0]}`, configs[agents[0]]),
      ),
      exportAgentModelConfig: vi
        .fn()
        .mockImplementationOnce(() => oldExport.promise)
        .mockImplementationOnce(() => newExport.promise),
    });
    const view = render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "export" }));

    view.rerender(<Harness api={api} target="codex" />);
    await waitFor(() =>
      expect(screen.getByTestId("flow")).toHaveTextContent("flow-codex"),
    );
    fireEvent.click(screen.getByRole("button", { name: "export" }));
    await act(async () => oldExport.resolve("old-target"));
    expect(createObjectURL).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    expect(api.discoverModels).toHaveBeenCalledTimes(2);

    await act(async () => newExport.resolve("new-target"));
    await waitFor(() => expect(api.discoverModels).toHaveBeenCalledTimes(3));
    expect(createObjectURL).toHaveBeenCalledTimes(1);
  });

  it("ignores a late import after switching targets", async () => {
    const oldImport = deferred<ModelConfig>();
    const api = readyApi({
      discoverModels: vi.fn(async (agents: AgentId[]) =>
        discoveryFor(`flow-${agents[0]}`, configs[agents[0]]),
      ),
      importAgentModelConfig: vi.fn(() => oldImport.promise),
    });
    const view = render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "import" }));
    view.rerender(<Harness api={api} target="codex" />);
    await waitFor(() =>
      expect(screen.getByTestId("flow")).toHaveTextContent("flow-codex"),
    );

    await act(async () => oldImport.resolve(configs.claude));
    expect(screen.getByTestId("config")).toHaveTextContent("existing-codex");
    expect(screen.getByTestId("source")).not.toHaveTextContent("imported");
  });

  it("reports a stable export error without changing the draft", async () => {
    const api = readyApi({
      exportAgentModelConfig: vi.fn().mockRejectedValue({
        code: "EXPORT_IO_ERROR",
        message: "sk-export-canary-secret",
      }),
    });
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "change-to-draft-b" }));
    fireEvent.click(screen.getByRole("button", { name: "export" }));

    await waitFor(() =>
      expect(screen.getByTestId("issue")).toHaveTextContent("EXPORT_IO_ERROR"),
    );
    expect(screen.getByTestId("config")).toHaveTextContent("draft-b");
    expect(screen.getByTestId("dirty")).toHaveTextContent("true");
    expect(screen.getByTestId("preview")).toHaveTextContent("");
    expect(document.body).not.toHaveTextContent("canary-secret");
  });

  it("foreground-recovers an expired flow reported by export", async () => {
    const rediscovery = deferred<AgentModelsResult>();
    const expiredExport = deferred<string>();
    const api = readyApi({
      discoverModels: vi
        .fn()
        .mockResolvedValueOnce(discoveryFor("flow-claude", configs.claude))
        .mockImplementationOnce(() => rediscovery.promise),
      exportAgentModelConfig: vi.fn(() => expiredExport.promise),
    });
    render(<Harness api={api} target="claude" />);
    await waitFor(() => expect(phase().kind).toBe("editing"));
    fireEvent.click(screen.getByRole("button", { name: "change-to-draft-b" }));
    fireEvent.click(screen.getByRole("button", { name: "export" }));
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    expect(api.discoverModels).toHaveBeenCalledTimes(1);
    await act(async () => expiredExport.reject({ code: "MODEL_FLOW_EXPIRED" }));

    await waitFor(() => expect(api.discoverModels).toHaveBeenCalledTimes(2));
    expect(api.discoverModels).toHaveBeenCalledTimes(2);
    expect(api.destroyAgentModelFlow).toHaveBeenCalledWith("flow-claude");
    expect(screen.getByTestId("config")).toHaveTextContent("draft-b");
    expect(screen.getByTestId("issue")).toHaveTextContent("MODEL_FLOW_EXPIRED");
    expect(
      JSON.parse(screen.getByTestId("operations").textContent ?? "{}").export,
    ).toBe(false);

    await act(async () =>
      rediscovery.resolve(
        discoveryFor("flow-export-recovered", configs.claude),
      ),
    );
    await waitFor(() => expect(phase().kind).toBe("editing"));
    expect(screen.getByTestId("flow")).toHaveTextContent(
      "flow-export-recovered",
    );
  });
});

describe("AgentPanel integration", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("shows model discovery during initial loading without blocking return", async () => {
    const pendingDiscovery = deferred<AgentModelsResult>();
    const onBack = vi.fn();
    const onGuardStateChange = vi.fn();
    const api = readyApi({
      discoverModels: vi.fn(() => pendingDiscovery.promise),
    });

    const view = renderWithI18n(
      <AgentPanel
        api={api}
        target="claude"
        onBack={onBack}
        onNavigateToApiKeys={vi.fn()}
        onGuardStateChange={onGuardStateChange}
      />,
    );

    await waitFor(() =>
      expect(api.discoverModels).toHaveBeenCalledWith(["claude"]),
    );
    const status = screen.getByRole("status");
    expect(status).toHaveTextContent("正在通过可信本地路由发现模型...");
    expect(status.textContent).not.toMatch(/\b(?:GET|TX)\b/);
    expect(view.container.querySelector(".instrument__dial")).toBeNull();
    expect(view.container.querySelector(".agent-panel__rail")).toBeNull();
    expect(
      view.container.querySelector(
        ".agent-panel__workspace--status > .agent-panel__processing",
      ),
    ).toBe(status);
    expect(onGuardStateChange).toHaveBeenLastCalledWith({
      dirty: false,
      busy: false,
    });

    fireEvent.click(screen.getByRole("button", { name: /返回 Agent 概览/ }));
    expect(onBack).toHaveBeenCalledOnce();

    await act(async () =>
      pendingDiscovery.resolve(discoveryFor("flow-loading", configs.claude)),
    );
    expect(await screen.findByLabelText(/^主模型$/)).toBeVisible();
  });

  it("keeps execution status in the rail without decorative protocol badges", async () => {
    const pendingPreview = deferred<AgentPreview>();
    const api = readyApi({
      previewAgents: vi.fn(() => pendingPreview.promise),
    });
    const view = renderWithI18n(
      <AgentPanel
        api={api}
        target="claude"
        onBack={vi.fn()}
        onNavigateToApiKeys={vi.fn()}
      />,
    );

    await screen.findByLabelText(/^主模型$/);
    fireEvent.click(screen.getByRole("button", { name: /生成写入预览/ }));

    const status = screen.getByRole("status");
    const rail = view.container.querySelector(".agent-panel__rail");
    expect(status).toHaveClass("agent-panel__processing");
    expect(status.textContent).not.toMatch(/\b(?:GET|TX)\b/);
    expect(view.container.querySelector(".instrument__dial")).toBeNull();
    expect(rail).not.toBeNull();
    expect(rail?.contains(status)).toBe(true);
    expect(
      view.container.querySelector(
        ".agent-panel__workspace > .agent-panel__processing",
      ),
    ).toBeNull();
    expect(view.container.querySelector(".agent-panel__editor")).toBeVisible();

    await act(async () => pendingPreview.resolve(previewFor(configs.claude)));
    expect(
      await screen.findByRole("button", { name: /写入所选 Agent/ }),
    ).toBeVisible();
    expect(view.container.querySelector(".agent-panel__preview")).toBeVisible();
  });

  it("keeps the disabled editor mounted beside preview and focuses its heading on mobile", async () => {
    vi.stubGlobal("matchMedia", vi.fn().mockReturnValue({ matches: true }));
    const pendingPreview = deferred<AgentPreview>();
    const api = readyApi({
      previewAgents: vi.fn(() => pendingPreview.promise),
    });
    const view = renderWithI18n(
      <AgentPanel
        api={api}
        target="claude"
        onBack={vi.fn()}
        onDirtyChange={vi.fn()}
        onNavigateToApiKeys={vi.fn()}
      />,
    );
    const primary = await screen.findByLabelText(/^主模型$/);
    fireEvent.click(screen.getByRole("button", { name: /生成写入预览/ }));

    const editor = view.container.querySelector(".agent-panel__editor")!;
    expect(editor).toBeVisible();
    expect(primary).toBeDisabled();
    await act(async () => pendingPreview.resolve(previewFor(configs.claude)));

    const rail = view.container.querySelector(".agent-panel__rail")!;
    const previewHeading = view.container.querySelector(
      ".agent-panel__preview > h3",
    );
    expect(view.container.querySelector(".agent-panel__editor")).toBeVisible();
    expect(view.container.querySelector(".agent-panel__preview")).toBeVisible();
    expect(
      editor.compareDocumentPosition(rail) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(primary).toBeDisabled();
    expect(previewHeading).toHaveFocus();
  });

  it("warns before editing an eligible recovery target", async () => {
    const recoveryDetection = {
      agents: detection.agents.map((state) =>
        state.agent === "claude"
          ? {
              ...state,
              invalid: true,
              recovery: { eligible: true, files: [] },
            }
          : state,
      ),
    };
    const api = readyApi({
      detectAgents: vi.fn().mockResolvedValue(recoveryDetection),
    });
    renderWithI18n(
      <AgentPanel
        api={api}
        target="claude"
        onBack={vi.fn()}
        onNavigateToApiKeys={vi.fn()}
      />,
    );

    expect(
      await screen.findByText(/无关设置、注释、格式以及有效伴随文件/),
    ).toHaveAttribute("role", "alert");
  });

  it("sends the imperative field snapshot when preview is generated", async () => {
    const api = readyApi({
      previewAgents: vi
        .fn()
        .mockImplementation(async (_agents, _flow, _catalog, config) =>
          previewFor(config),
        ),
    });
    renderWithI18n(
      <AgentPanel
        api={api}
        target="claude"
        onBack={vi.fn()}
        onDirtyChange={vi.fn()}
        onNavigateToApiKeys={vi.fn()}
      />,
    );

    const extra = await screen.findByLabelText(/Claude Code extra JSON/);
    fireEvent.change(extra, { target: { value: '{"custom":"latest"}' } });
    fireEvent.click(screen.getByRole("button", { name: /生成写入预览/ }));

    await waitFor(() => expect(api.previewAgents).toHaveBeenCalledOnce());
    expect(vi.mocked(api.previewAgents).mock.calls[0][3].claude?.extra).toEqual(
      {
        custom: "latest",
      },
    );
  });

  it("passes preview approvals through the controller to the write request", async () => {
    const api = readyApi({
      previewAgents: vi.fn().mockResolvedValue(
        previewFor(configs.claude, {
          managed_config_drift: true,
          drifted_agents: ["claude"],
          requires_codex_auth_approval: true,
        }),
      ),
      writeAgents: vi.fn().mockResolvedValue({
        transaction_id: "tx-panel",
        agents: [{ agent: "claude", success: true }],
      }),
    });
    renderWithI18n(
      <AgentPanel
        api={api}
        target="claude"
        onBack={vi.fn()}
        onDirtyChange={vi.fn()}
        onNavigateToApiKeys={vi.fn()}
      />,
    );

    fireEvent.click(
      await screen.findByRole("button", { name: /生成写入预览/ }),
    );
    fireEvent.click(
      await screen.findByRole("checkbox", {
        name: /批准覆盖已漂移的托管命名空间/,
      }),
    );
    fireEvent.click(
      screen.getByRole("checkbox", {
        name: /批准 Codex 切换为文件型 API key 认证/,
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: /写入所选 Agent/ }));

    await waitFor(() => expect(api.writeAgents).toHaveBeenCalledOnce());
    expect(vi.mocked(api.writeAgents).mock.calls[0].slice(5)).toEqual([
      true,
      true,
      [],
    ]);
  });

  it("shows a successful result with reloaded prefill and dismisses it without leaving", async () => {
    const reloaded = {
      ...configs.claude,
      claude: {
        ...configs.claude.claude!,
        primary: { model: "preset-claude", name: "Reloaded" },
      },
    };
    const onBack = vi.fn();
    const onReloaded = vi.fn();
    const api = readyApi({
      discoverModels: vi
        .fn()
        .mockResolvedValueOnce(discoveryFor("flow-before", configs.claude))
        .mockResolvedValueOnce(discoveryFor("flow-after", reloaded)),
      previewAgents: vi.fn().mockResolvedValue(previewFor(reloaded)),
      writeAgents: vi.fn().mockResolvedValue({
        transaction_id: "tx-reloaded",
        agents: [
          {
            agent: "claude",
            success: true,
            changed: ["/safe/claude/config.json"],
          },
        ],
      }),
    });
    const view = renderWithI18n(
      <AgentPanel
        api={api}
        target="claude"
        onBack={onBack}
        onReloaded={onReloaded}
        onNavigateToApiKeys={vi.fn()}
      />,
    );
    fireEvent.click(
      await screen.findByRole("button", { name: /生成写入预览/ }),
    );
    fireEvent.click(
      await screen.findByRole("button", { name: /写入所选 Agent/ }),
    );

    expect(await screen.findByText("成功")).toBeVisible();
    expect(screen.getByLabelText(/^主模型$/)).toHaveValue("preset-claude");
    expect(screen.getByLabelText(/claude-primary 显示名称/)).toHaveValue(
      "Reloaded",
    );
    expect(view.container.querySelector(".agent-panel__editor")).toBeVisible();
    expect(onReloaded).toHaveBeenCalledWith(detection);

    fireEvent.click(screen.getByRole("button", { name: /关闭结果并继续编辑/ }));
    expect(screen.queryByText("成功")).not.toBeInTheDocument();
    expect(view.container.querySelector(".agent-panel__editor")).toBeVisible();
    expect(view.container.querySelector(".agent-panel__editor")).toHaveFocus();
    expect(onBack).not.toHaveBeenCalled();
  });

  it("refreshes overview detection after dismissing a result before reload retry", async () => {
    const onReloaded = vi.fn();
    const api = readyApi({
      detectAgents: vi
        .fn()
        .mockResolvedValueOnce(detection)
        .mockRejectedValueOnce({ code: "AGENT_DETECT_IO" })
        .mockResolvedValueOnce(detection),
      discoverModels: vi
        .fn()
        .mockResolvedValueOnce(discoveryFor("flow-before", configs.claude))
        .mockResolvedValueOnce(discoveryFor("flow-after", configs.claude)),
      previewAgents: vi.fn().mockResolvedValue(previewFor(configs.claude)),
      writeAgents: vi.fn().mockResolvedValue({
        transaction_id: "tx-retry-after-dismiss",
        agents: [{ agent: "claude", success: true }],
      }),
    });
    renderWithI18n(
      <AgentPanel
        api={api}
        target="claude"
        onBack={vi.fn()}
        onReloaded={onReloaded}
        onNavigateToApiKeys={vi.fn()}
      />,
    );
    fireEvent.click(
      await screen.findByRole("button", { name: /生成写入预览/ }),
    );
    fireEvent.click(
      await screen.findByRole("button", { name: /写入所选 Agent/ }),
    );
    expect(await screen.findByText(/写入后重新加载失败/)).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: /关闭结果并继续编辑/ }));
    fireEvent.click(screen.getByRole("button", { name: "重试" }));

    await waitFor(() => expect(onReloaded).toHaveBeenCalledWith(detection));
  });

  it("resynchronizes extra JSON after a clean external refresh", async () => {
    const initial = {
      ...configs.claude,
      claude: { ...configs.claude.claude!, extra: { canonical: "old" } },
    };
    const refreshed = {
      ...configs.claude,
      claude: { ...configs.claude.claude!, extra: { canonical: "new" } },
    };
    const api = readyApi({
      discoverModels: vi
        .fn()
        .mockResolvedValueOnce(discoveryFor("flow-before", initial))
        .mockResolvedValueOnce(discoveryFor("flow-after", refreshed)),
    });
    renderWithI18n(
      <AgentPanel
        api={api}
        target="claude"
        onBack={vi.fn()}
        onNavigateToApiKeys={vi.fn()}
      />,
    );
    const extra = await screen.findByLabelText(/Claude Code extra JSON/);
    expect(extra).toHaveValue(JSON.stringify({ canonical: "old" }, null, 2));

    fireEvent.click(screen.getByRole("button", { name: /重新检测/ }));

    await waitFor(() =>
      expect(extra).toHaveValue(JSON.stringify({ canonical: "new" }, null, 2)),
    );
    fireEvent.click(screen.getByRole("button", { name: /生成写入预览/ }));
    await waitFor(() => expect(api.previewAgents).toHaveBeenCalledOnce());
    expect(vi.mocked(api.previewAgents).mock.calls[0][3].claude?.extra).toEqual(
      {
        canonical: "new",
      },
    );
  });

  it("keeps invalid local text mounted while blocked and resets it on discard", async () => {
    const rebuildDetection = {
      agents: detection.agents.map((agent) =>
        agent.agent === "claude"
          ? {
              ...agent,
              invalid: true,
              recovery: {
                eligible: true,
                files: [
                  {
                    role: "config",
                    path: agent.path,
                    format: agent.format,
                    exists: true,
                    reasons: ["syntax_invalid"],
                  },
                ],
              },
            }
          : agent,
      ),
    };
    const api = readyApi({
      detectAgents: vi
        .fn()
        .mockResolvedValueOnce(detection)
        .mockResolvedValue(rebuildDetection),
      discoverModels: vi
        .fn()
        .mockResolvedValueOnce(discoveryFor("flow-merge", configs.claude))
        .mockResolvedValue(discoveryFor("flow-rebuild", configs.claude)),
    });
    const view = renderWithI18n(
      <AgentPanel
        api={api}
        target="claude"
        onBack={vi.fn()}
        onNavigateToApiKeys={vi.fn()}
      />,
    );
    const extra = await screen.findByLabelText(/Claude Code extra JSON/);
    fireEvent.change(extra, { target: { value: '{"broken":' } });
    fireEvent.click(screen.getByRole("button", { name: /重新检测/ }));

    await screen.findByText(/当前草稿无法继续编辑/);
    expect(view.container.querySelector(".agent-panel__editor")).toBeVisible();
    expect(extra).toHaveValue('{"broken":');
    expect(extra).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: /放弃草稿/ }));
    await waitFor(() =>
      expect(screen.getByLabelText(/Claude Code extra JSON/)).toHaveValue(""),
    );
  });

  it("shows readonly metadata and offers a real catalog session retry", async () => {
    const onRetrySession = vi.fn();
    const api = readyApi({
      discoverModels: vi.fn().mockRejectedValue({
        code: "MODEL_DISCOVERY_FAILED",
        message: "secret upstream detail",
      }),
    });
    renderWithI18n(
      <AgentPanel
        api={api}
        target="claude"
        onBack={vi.fn()}
        onRetrySession={onRetrySession}
        onNavigateToApiKeys={vi.fn()}
      />,
    );

    expect(await screen.findByText("/safe/claude/config")).toBeVisible();
    expect(screen.getByText("json")).toBeVisible();
    expect(document.body).not.toHaveTextContent("secret upstream detail");
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    expect(onRetrySession).toHaveBeenCalledOnce();
  });

  it("shows readonly recovery reasons without attempting discovery", async () => {
    const readonlyDetection = {
      agents: detection.agents.map((agent) =>
        agent.agent === "claude"
          ? {
              ...agent,
              invalid: true,
              writable: false,
              recovery: {
                eligible: false,
                reasons: ["syntax_invalid"],
                files: [],
              },
            }
          : agent,
      ),
    };
    const api = readyApi({
      detectAgents: vi.fn().mockResolvedValue(readonlyDetection),
    });
    renderWithI18n(
      <AgentPanel
        api={api}
        target="claude"
        onBack={vi.fn()}
        onNavigateToApiKeys={vi.fn()}
      />,
    );

    expect(await screen.findByText("配置语法无效。")).toBeVisible();
    expect(screen.getByText("/safe/claude/config")).toBeVisible();
    expect(screen.getByText("不可写")).toBeVisible();
    expect(screen.getByText("配置无效")).toBeVisible();
    expect(screen.getByRole("button", { name: "重试" })).toBeVisible();
    expect(api.discoverModels).not.toHaveBeenCalled();
  });

  it.each([
    ["wrong extension", new File(["{}"], "config.txt")],
    ["oversized JSON", oversizedJsonFile()],
  ])("rejects the %s import boundary before IPC", async (_, file) => {
    const api = readyApi();
    renderWithI18n(
      <AgentPanel
        api={api}
        target="claude"
        onBack={vi.fn()}
        onNavigateToApiKeys={vi.fn()}
      />,
    );
    const input = await waitFor(() => {
      const value =
        document.querySelector<HTMLInputElement>('input[type="file"]');
      expect(value).not.toBeNull();
      return value!;
    });
    fireEvent.change(input, { target: { files: [file] } });

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "IMPORT_INVALID",
    );
    expect(api.importAgentModelConfig).not.toHaveBeenCalled();
  });
});
