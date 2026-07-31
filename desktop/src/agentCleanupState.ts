import type { AgentCleanupPreview, AgentWriteResult } from "./ipc";

export type AgentCleanupPhase =
  | { kind: "loading-preview" }
  | {
      kind: "previewing";
      preview: AgentCleanupPreview;
      driftApproved: boolean;
    }
  | { kind: "writing"; preview: AgentCleanupPreview }
  | { kind: "result"; result: AgentWriteResult }
  | {
      kind: "repreview-required";
      preview: AgentCleanupPreview;
      code: string;
    }
  | {
      kind: "failed";
      preview: AgentCleanupPreview | null;
      code: string;
    };

export type AgentCleanupEvent =
  | { type: "LOAD_PREVIEW" }
  | { type: "PREVIEW_LOADED"; preview: AgentCleanupPreview }
  | { type: "PREVIEW_FAILED"; code: string }
  | { type: "SET_DRIFT_APPROVED"; approved: boolean }
  | { type: "WRITE_STARTED" }
  | { type: "RETRY_WRITE" }
  | { type: "WRITE_SUCCEEDED"; result: AgentWriteResult }
  | { type: "WRITE_FAILED"; code: string };

export function cleanupCanWrite(
  phase: AgentCleanupPhase,
): phase is Extract<AgentCleanupPhase, { kind: "previewing" }> {
  return (
    phase.kind === "previewing" &&
    (!phase.preview.managed_config_drift || phase.driftApproved)
  );
}

const repreviewRequiredWriteCodes = new Set([
  "PREVIEW_STALE",
  "INVALID_RESPONSE",
  "MANAGER_FAILED",
  "OPERATION_TIMEOUT",
  "CLEANUP_WRITE_FAILED",
]);

export function agentCleanupReducer(
  phase: AgentCleanupPhase,
  event: AgentCleanupEvent,
): AgentCleanupPhase {
  switch (event.type) {
    case "LOAD_PREVIEW":
      return { kind: "loading-preview" };
    case "PREVIEW_LOADED":
      return phase.kind === "loading-preview"
        ? {
            kind: "previewing",
            preview: event.preview,
            driftApproved: false,
          }
        : phase;
    case "PREVIEW_FAILED":
      return phase.kind === "loading-preview"
        ? { kind: "failed", preview: null, code: event.code }
        : phase;
    case "SET_DRIFT_APPROVED":
      return phase.kind === "previewing"
        ? { ...phase, driftApproved: event.approved }
        : phase;
    case "WRITE_STARTED":
      return cleanupCanWrite(phase)
        ? { kind: "writing", preview: phase.preview }
        : phase;
    case "RETRY_WRITE":
      return phase.kind === "failed" && phase.preview
        ? { kind: "writing", preview: phase.preview }
        : phase;
    case "WRITE_SUCCEEDED":
      return phase.kind === "writing"
        ? { kind: "result", result: event.result }
        : phase;
    case "WRITE_FAILED":
      if (phase.kind !== "writing") return phase;
      return repreviewRequiredWriteCodes.has(event.code)
        ? {
            kind: "repreview-required",
            preview: phase.preview,
            code: event.code,
          }
        : { kind: "failed", preview: phase.preview, code: event.code };
  }
}
