import {
  initializeAgentConfig,
  type AgentDetection,
  type AgentId,
  type AgentMode,
  type AgentModelsResult,
  type AgentState,
  type ModelConfig,
} from "./ipc";

export type PanelPhase =
  | { kind: "loading" }
  | { kind: "readonly"; reason: ReadonlyReason }
  | { kind: "blocked-dirty"; canExport: boolean; errorCode: string | null }
  | { kind: "editing"; refresh: RefreshState }
  | { kind: "preview-loading" }
  | { kind: "previewing" }
  | { kind: "writing" }
  | { kind: "reloading" }
  | { kind: "reload-failed"; code: string };

export type RefreshState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "conflict"; candidate: CandidateDiscovery }
  | { kind: "failed"; code: string };

export interface PanelOperationAvailability {
  edit: boolean;
  export: boolean;
  preview: boolean;
  import: boolean;
}

export type ReadonlyReason =
  | { kind: "credential"; code: string }
  | { kind: "catalog"; code: string }
  | { kind: "not-writable" }
  | { kind: "not-recoverable" };

export interface CandidateDiscovery {
  detection: AgentDetection;
  discovery: AgentModelsResult;
  externalBaseline: string;
}

export type PanelIssue =
  | { kind: "success" }
  | { kind: "refresh"; code: string }
  | { kind: "operation"; code: string };

export interface WriteApprovals {
  managedOverwrite: boolean;
  codexAuthChange: boolean;
  rebuild: AgentId[];
}

export interface PanelBaselines {
  form: ModelConfig;
  external: string;
}

type NormalizedValue =
  | null
  | boolean
  | number
  | string
  | NormalizedValue[]
  | { [key: string]: NormalizedValue };

function normalize(value: unknown): NormalizedValue {
  if (Array.isArray(value)) return value.map(normalize);
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .filter(([, child]) => child !== undefined)
        .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
        .map(([key, child]) => [key, normalize(child)]),
    );
  }
  return value as null | boolean | number | string;
}

function normalizedConfig(config: ModelConfig): ModelConfig {
  return normalize(config) as unknown as ModelConfig;
}

export function createPanelBaselines(
  agent: AgentId,
  detection: AgentDetection,
  discovery: AgentModelsResult,
): PanelBaselines {
  const initialized = initializeAgentConfig(
    [agent],
    discovery.existing.model_config,
    discovery.preset.model_config,
  );

  return {
    form: normalizedConfig(initialized.config),
    external: createExternalSnapshot(agent, detection, discovery),
  };
}

export function createExternalSnapshot(
  agent: AgentId,
  detection: AgentDetection,
  discovery: AgentModelsResult,
): string {
  const state = detection.agents.find((candidate) => candidate.agent === agent);
  const detectionSnapshot = state
    ? {
        path: state.path,
        format: state.format,
        exists: state.exists,
        writable: state.writable,
        configured: state.configured,
        invalid: state.invalid,
        recovery: state.recovery,
      }
    : null;

  return JSON.stringify(
    normalize({
      agent,
      existing: discovery.existing.model_config[agent] ?? null,
      unavailableModels: [
        ...(discovery.existing.unavailable_models[agent] ?? []),
      ].sort(),
      driftedAgents: discovery.existing.drifted_agents
        .filter((driftedAgent) => driftedAgent === agent)
        .sort(),
      detection: detectionSnapshot,
    }),
  );
}

export function sameExternalSnapshot(left: string, right: string): boolean {
  return left === right;
}

export function isConfigDirty(
  config: ModelConfig,
  baseline: ModelConfig,
): boolean {
  return (
    JSON.stringify(normalize(config)) !== JSON.stringify(normalize(baseline))
  );
}

export function targetMode(state: AgentState): AgentMode | null {
  if (state.invalid) return state.recovery.eligible ? "rebuild" : null;
  return state.writable ? "merge" : null;
}

export function panelOperationAvailability(
  phase: PanelPhase,
  hasActiveFlow: boolean,
): PanelOperationAvailability {
  if (phase.kind === "blocked-dirty") {
    return {
      edit: false,
      export: phase.canExport && hasActiveFlow,
      preview: false,
      import: false,
    };
  }
  if (phase.kind !== "editing") {
    return { edit: false, export: false, preview: false, import: false };
  }

  const checking = phase.refresh.kind === "checking";
  const conflicted = phase.refresh.kind === "conflict";
  return {
    edit: true,
    export: hasActiveFlow,
    preview: hasActiveFlow && !checking && !conflicted,
    import: hasActiveFlow && !checking && !conflicted,
  };
}
