import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import {
  agentCleanupReducer,
  cleanupCanWrite,
  type AgentCleanupPhase,
} from "./agentCleanupState";
import type { AgentCleanupPreview, AgentId } from "./ipc";
import { createMockApi } from "./test/api";
import { useAgentCleanupController } from "./useAgentCleanupController";

const preview = {
  revision_token: "cleanup-revision",
  agent: "opencode",
  files: [
    {
      path: "/mock/opencode/config",
      role: "config",
      format: "json",
      operation: "replace",
      backup_required: true,
    },
  ],
  removed_paths: ["provider.mtls-router"],
  managed_config_drift: true,
} satisfies AgentCleanupPreview;

const result = {
  transaction_id: "cleanup-transaction",
  agents: [{ agent: "opencode" as const, success: true }],
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("agent cleanup state", () => {
  it("gates writes on preview state and explicit drift approval", () => {
    let state: AgentCleanupPhase = { kind: "loading-preview" };
    state = agentCleanupReducer(state, { type: "PREVIEW_LOADED", preview });
    expect(cleanupCanWrite(state)).toBe(false);

    state = agentCleanupReducer(state, {
      type: "SET_DRIFT_APPROVED",
      approved: true,
    });
    expect(cleanupCanWrite(state)).toBe(true);
    expect(agentCleanupReducer(state, { type: "WRITE_STARTED" })).toEqual({
      kind: "writing",
      preview,
    });
  });

  it("keeps explicit failures retryable and makes stale or ambiguous writes repreview-only", () => {
    const writing: AgentCleanupPhase = { kind: "writing", preview };
    expect(
      agentCleanupReducer(writing, {
        type: "WRITE_FAILED",
        code: "BACKUP_FAILED",
      }),
    ).toEqual({ kind: "failed", preview, code: "BACKUP_FAILED" });
    for (const code of [
      "PREVIEW_STALE",
      "INVALID_RESPONSE",
      "MANAGER_FAILED",
      "OPERATION_TIMEOUT",
      "CLEANUP_WRITE_FAILED",
    ]) {
      expect(
        agentCleanupReducer(writing, { type: "WRITE_FAILED", code }),
      ).toEqual({ kind: "repreview-required", preview, code });
    }
  });

  it("ignores duplicate write events", () => {
    const writing: AgentCleanupPhase = { kind: "writing", preview };
    expect(agentCleanupReducer(writing, { type: "WRITE_STARTED" })).toBe(
      writing,
    );
  });
});

describe("useAgentCleanupController", () => {
  it("loads, applies the drift gate, and writes without credential or model calls", async () => {
    const api = createMockApi({
      previewAgentCleanup: vi.fn().mockResolvedValue(preview),
      writeAgentCleanup: vi.fn().mockResolvedValue(result),
    });
    const { result: hook } = renderHook(() =>
      useAgentCleanupController({ api, agent: "opencode" }),
    );

    await waitFor(() => expect(hook.current.phase.kind).toBe("previewing"));
    expect(hook.current.canWrite).toBe(false);
    act(() => hook.current.setDriftApproved(true));
    expect(hook.current.canWrite).toBe(true);
    await act(() => hook.current.write());

    expect(api.writeAgentCleanup).toHaveBeenCalledWith(
      "opencode",
      "cleanup-revision",
      true,
    );
    expect(hook.current.phase).toEqual({ kind: "result", result });
    for (const forbidden of [
      api.getCredential,
      api.discoverModels,
      api.renderAgentConfig,
      api.previewAgents,
      api.writeAgents,
      api.destroyAgentModelFlow,
    ]) {
      expect(forbidden).not.toHaveBeenCalled();
    }
  });

  it("repreviews ambiguous writes without replay and retries explicit failures", async () => {
    const writeAgentCleanup = vi
      .fn()
      .mockRejectedValueOnce({ code: "INVALID_RESPONSE" })
      .mockRejectedValueOnce({ code: "BACKUP_FAILED" })
      .mockResolvedValueOnce(result);
    const previewAgentCleanup = vi.fn().mockResolvedValue(preview);
    const api = createMockApi({ previewAgentCleanup, writeAgentCleanup });
    const { result: hook } = renderHook(() =>
      useAgentCleanupController({ api, agent: "opencode" }),
    );
    await waitFor(() => expect(hook.current.phase.kind).toBe("previewing"));
    act(() => hook.current.setDriftApproved(true));
    await act(() => hook.current.write());
    expect(hook.current.phase).toMatchObject({
      kind: "repreview-required",
      code: "INVALID_RESPONSE",
    });

    await act(() => hook.current.retry());
    expect(writeAgentCleanup).toHaveBeenCalledTimes(1);

    await act(() => hook.current.repreview());
    expect(hook.current.phase.kind).toBe("previewing");
    expect(writeAgentCleanup).toHaveBeenCalledTimes(1);
    act(() => hook.current.setDriftApproved(true));
    await act(() => hook.current.write());
    expect(hook.current.phase).toMatchObject({
      kind: "failed",
      preview,
      code: "BACKUP_FAILED",
    });
    await act(() => hook.current.retry());
    expect(writeAgentCleanup).toHaveBeenCalledTimes(3);
    expect(hook.current.phase.kind).toBe("result");
  });

  it("ignores late preview completion after the owned Agent changes", async () => {
    const oldPreview = deferred<AgentCleanupPreview>();
    const nextPreview = { ...preview, agent: "claude" as const };
    const api = createMockApi({
      previewAgentCleanup: vi
        .fn()
        .mockImplementationOnce(() => oldPreview.promise)
        .mockResolvedValueOnce(nextPreview),
    });
    const { result: hook, rerender } = renderHook(
      ({ agent }: { agent: AgentId }) =>
        useAgentCleanupController({ api, agent }),
      { initialProps: { agent: "opencode" } },
    );

    rerender({ agent: "claude" });
    await waitFor(() =>
      expect(hook.current.phase).toMatchObject({
        kind: "previewing",
        preview: nextPreview,
      }),
    );
    await act(async () => oldPreview.resolve(preview));
    expect(hook.current.phase).toMatchObject({
      kind: "previewing",
      preview: nextPreview,
    });
  });
});
