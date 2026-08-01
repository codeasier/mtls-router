import { useEffectEvent, useLayoutEffect, useRef, useState } from "react";

import {
  agentCleanupReducer,
  cleanupCanWrite,
  type AgentCleanupEvent,
  type AgentCleanupPhase,
} from "./agentCleanupState";
import type { AgentCleanupPreview, AgentId, DesktopApi } from "./ipc";

export interface AgentCleanupControllerOptions {
  api: DesktopApi;
  agent: AgentId;
}

export interface AgentCleanupController {
  phase: AgentCleanupPhase;
  busy: boolean;
  canWrite: boolean;
  setDriftApproved(approved: boolean): void;
  write(): Promise<void>;
  repreview(): Promise<void>;
  retry(): Promise<void>;
}

function errorCode(error: unknown, fallback: string): string {
  return typeof error === "object" &&
    error !== null &&
    "code" in error &&
    typeof (error as { code?: unknown }).code === "string"
    ? (error as { code: string }).code
    : fallback;
}

export function useAgentCleanupController({
  api,
  agent,
}: AgentCleanupControllerOptions): AgentCleanupController {
  const [phase, setPhase] = useState<AgentCleanupPhase>({
    kind: "loading-preview",
  });
  const phaseRef = useRef(phase);
  const generationRef = useRef(0);

  const commit = (event: AgentCleanupEvent) => {
    const next = agentCleanupReducer(phaseRef.current, event);
    phaseRef.current = next;
    setPhase(next);
  };

  const loadPreview = async () => {
    const generation = ++generationRef.current;
    commit({ type: "LOAD_PREVIEW" });
    try {
      const preview = await api.previewAgentCleanup(agent);
      if (generation !== generationRef.current) return;
      if (preview.agent !== agent || !preview.revision_token.trim()) {
        commit({ type: "PREVIEW_FAILED", code: "INVALID_RESPONSE" });
        return;
      }
      commit({ type: "PREVIEW_LOADED", preview });
    } catch (error) {
      if (generation !== generationRef.current) return;
      commit({
        type: "PREVIEW_FAILED",
        code: errorCode(error, "CLEANUP_PREVIEW_FAILED"),
      });
    }
  };

  const writePreview = async (
    preview: AgentCleanupPreview,
    approveManagedOverwrite: boolean,
    retrying: boolean,
  ) => {
    const generation = ++generationRef.current;
    commit({ type: retrying ? "RETRY_WRITE" : "WRITE_STARTED" });
    if (phaseRef.current.kind !== "writing") return;
    try {
      const result = await api.writeAgentCleanup(
        agent,
        preview.revision_token,
        approveManagedOverwrite,
      );
      if (generation !== generationRef.current) return;
      commit({ type: "WRITE_SUCCEEDED", result });
    } catch (error) {
      if (generation !== generationRef.current) return;
      commit({
        type: "WRITE_FAILED",
        code: errorCode(error, "CLEANUP_WRITE_FAILED"),
      });
    }
  };

  const loadInitialPreview = useEffectEvent(loadPreview);

  useLayoutEffect(() => {
    void loadInitialPreview();
    return () => {
      generationRef.current += 1;
    };
  }, [api, agent]);

  return {
    phase,
    busy: phase.kind === "loading-preview" || phase.kind === "writing",
    canWrite: cleanupCanWrite(phase),
    setDriftApproved: (approved) =>
      commit({ type: "SET_DRIFT_APPROVED", approved }),
    write: async () => {
      const current = phaseRef.current;
      if (!cleanupCanWrite(current)) return;
      await writePreview(
        current.preview,
        current.preview.managed_config_drift,
        false,
      );
    },
    repreview: async () => {
      if (phaseRef.current.kind !== "repreview-required") return;
      await loadPreview();
    },
    retry: async () => {
      const current = phaseRef.current;
      if (current.kind !== "failed") return;
      if (!current.preview) {
        await loadPreview();
        return;
      }
      await writePreview(
        current.preview,
        current.preview.managed_config_drift,
        true,
      );
    },
  };
}
