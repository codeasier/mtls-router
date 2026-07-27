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
  DesktopApi,
  ModelConfig,
} from "./ipc";
import { createMockApi } from "./test/api";
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
      {(["preview-loading", "previewing", "writing", "reloading"] as const).map(
        (operation) => (
          <button
            key={operation}
            onClick={() => controller.__occupyOperationForTask7(operation)}
          >
            occupy-{operation}
          </button>
        ),
      )}
      <button onClick={() => controller.__occupyOperationForTask7(null)}>
        finish-operation
      </button>
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

  it("rebases a preserved dirty draft onto the candidate form baseline", async () => {
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
    expect(screen.getByTestId("dirty")).toHaveTextContent("false");
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

  it.each(["preview-loading", "previewing", "writing", "reloading"] as const)(
    "coalesces one pending refresh while %s owns the panel",
    async (operation) => {
      const api = readyApi();
      render(<Harness api={api} target="claude" />);
      await waitFor(() => expect(phase().kind).toBe("editing"));
      fireEvent.click(
        screen.getByRole("button", { name: `occupy-${operation}` }),
      );
      fireEvent.click(screen.getByRole("button", { name: "refresh" }));
      fireEvent.click(screen.getByRole("button", { name: "refresh" }));
      expect(api.discoverModels).toHaveBeenCalledTimes(1);

      fireEvent.click(screen.getByRole("button", { name: "finish-operation" }));
      await waitFor(() => expect(api.discoverModels).toHaveBeenCalledTimes(2));
      expect(api.discoverModels).toHaveBeenLastCalledWith(["claude"]);
    },
  );
});
