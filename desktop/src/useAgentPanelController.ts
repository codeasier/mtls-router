import {
  useEffect,
  useEffectEvent,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

import type { AgentConfigDraftState } from "./AgentConfigFields";
import {
  createPanelBaselines,
  isConfigDirty,
  targetMode,
  type PanelPhase,
} from "./agentPanelState";
import { completeAgentDetection, type AgentTarget } from "./agentPresentation";
import {
  initializeAgentConfig,
  type AgentDetection,
  type AgentId,
  type AgentModelsResult,
  type CredentialSummary,
  type DesktopApi,
  type InitializationSource,
  type ModelConfig,
} from "./ipc";

interface OwnedFlow {
  api: DesktopApi;
  id: string;
  destroying: boolean;
  attempts: number;
  retryTimer: ReturnType<typeof setTimeout> | null;
}

interface FlowRef {
  current: OwnedFlow | null;
}

interface PendingFlowRef {
  current: OwnedFlow[];
}

interface ControllerSnapshot {
  ownerApi: DesktopApi;
  ownerTarget: AgentId;
  phase: PanelPhase;
  detection: AgentDetection | null;
  credential: CredentialSummary | null;
  discovery: AgentModelsResult | null;
  target: AgentTarget | null;
  config: ModelConfig | null;
  source: InitializationSource | null;
  formBaseline: ModelConfig | null;
  draftState: AgentConfigDraftState;
}

function emptySnapshot(
  ownerApi: DesktopApi,
  ownerTarget: AgentId,
): ControllerSnapshot {
  return {
    ownerApi,
    ownerTarget,
    phase: { kind: "loading" },
    detection: null,
    credential: null,
    discovery: null,
    target: null,
    config: null,
    source: null,
    formBaseline: null,
    draftState: { error: "", hasLocalDraft: false },
  };
}

const MAX_DESTROY_ATTEMPTS = 4;
const DESTROY_RETRY_BASE_MS = 25;

function destroyOwnedFlow(owner: OwnedFlow, pendingRef: PendingFlowRef) {
  let pending = pendingRef.current.find(
    (flow) => flow.api === owner.api && flow.id === owner.id,
  );
  if (!pending) {
    pending = owner;
    pendingRef.current.push(pending);
  }
  if (pending.destroying || pending.retryTimer) return;
  pending.destroying = true;
  pending.attempts += 1;
  void pending.api
    .destroyAgentModelFlow(pending.id)
    .then(() => {
      const index = pendingRef.current.indexOf(pending);
      if (index >= 0) pendingRef.current.splice(index, 1);
    })
    .catch(() => {
      pending.destroying = false;
      if (pending.attempts >= MAX_DESTROY_ATTEMPTS) return;
      pending.retryTimer = setTimeout(
        () => {
          pending.retryTimer = null;
          destroyOwnedFlow(pending, pendingRef);
        },
        DESTROY_RETRY_BASE_MS * 2 ** (pending.attempts - 1),
      );
    });
}

function cleanOwnedFlows(
  activeRef: FlowRef,
  candidateRef: FlowRef,
  pendingRef: PendingFlowRef,
) {
  const active = activeRef.current;
  const candidate = candidateRef.current;
  activeRef.current = null;
  candidateRef.current = null;
  if (active) destroyOwnedFlow(active, pendingRef);
  if (candidate) destroyOwnedFlow(candidate, pendingRef);
  for (const pending of [...pendingRef.current])
    destroyOwnedFlow(pending, pendingRef);
}

export interface AgentPanelController {
  phase: PanelPhase;
  detection: AgentDetection | null;
  credential: CredentialSummary | null;
  discovery: AgentModelsResult | null;
  target: AgentTarget | null;
  config: ModelConfig | null;
  source: InitializationSource | null;
  dirty: boolean;
  draftState: AgentConfigDraftState;
  setConfig(config: ModelConfig): void;
  setDraftState(state: AgentConfigDraftState): void;
}

export interface AgentPanelControllerOptions {
  api: DesktopApi;
  target: AgentId;
  onDirtyChange(dirty: boolean): void;
}

function errorCode(error: unknown, fallback: string) {
  return typeof error === "object" &&
    error !== null &&
    "code" in error &&
    typeof (error as { code?: unknown }).code === "string"
    ? (error as { code: string }).code
    : fallback;
}

export function useAgentPanelController({
  api,
  target: targetId,
  onDirtyChange,
}: AgentPanelControllerOptions): AgentPanelController {
  const [snapshot, setSnapshot] = useState<ControllerSnapshot>(() =>
    emptySnapshot(api, targetId),
  );
  const generationRef = useRef(0);
  const activeFlowRef = useRef<OwnedFlow | null>(null);
  const candidateFlowRef = useRef<OwnedFlow | null>(null);
  const pendingDestroyRef = useRef<OwnedFlow[]>([]);
  const reportDirty = useEffectEvent(onDirtyChange);

  useLayoutEffect(() => {
    const generation = ++generationRef.current;
    let mounted = true;
    const current = () => mounted && generation === generationRef.current;
    const publish = (next: ControllerSnapshot) => {
      if (!current()) return;
      setSnapshot(next);
    };

    void (async () => {
      let detected: AgentDetection;
      try {
        const complete = completeAgentDetection(await api.detectAgents());
        if (!complete) throw { code: "AGENT_DETECT_FAILED" };
        detected = complete;
      } catch (error) {
        publish({
          ...emptySnapshot(api, targetId),
          phase: {
            kind: "readonly",
            reason: {
              kind: "catalog",
              code: errorCode(error, "AGENT_DETECT_FAILED"),
            },
          },
        });
        return;
      }
      if (!current()) return;

      let summary: CredentialSummary;
      try {
        summary = await api.getCredential();
      } catch (error) {
        publish({
          ...emptySnapshot(api, targetId),
          detection: detected,
          phase: {
            kind: "readonly",
            reason: {
              kind: "credential",
              code: errorCode(error, "CREDENTIAL_IO_ERROR"),
            },
          },
        });
        return;
      }
      if (!current()) return;
      if (!summary.present) {
        publish({
          ...emptySnapshot(api, targetId),
          detection: detected,
          credential: summary,
          phase: {
            kind: "readonly",
            reason: { kind: "credential", code: "CREDENTIAL_NOT_FOUND" },
          },
        });
        return;
      }

      const state = detected.agents.find((agent) => agent.agent === targetId)!;
      const mode = targetMode(state);
      if (!mode) {
        publish({
          ...emptySnapshot(api, targetId),
          detection: detected,
          credential: summary,
          phase: {
            kind: "readonly",
            reason: {
              kind: state.invalid ? "not-recoverable" : "not-writable",
            },
          },
        });
        return;
      }

      let discovered: AgentModelsResult;
      try {
        discovered = await api.discoverModels([targetId]);
      } catch (error) {
        publish({
          ...emptySnapshot(api, targetId),
          detection: detected,
          credential: summary,
          phase: {
            kind: "readonly",
            reason: {
              kind: "catalog",
              code: errorCode(error, "MODEL_DISCOVERY_FAILED"),
            },
          },
        });
        return;
      }

      const flowId = discovered.flow_id.trim();
      if (!flowId) {
        publish({
          ...emptySnapshot(api, targetId),
          detection: detected,
          credential: summary,
          phase: {
            kind: "readonly",
            reason: { kind: "catalog", code: "MODEL_RESPONSE_INVALID" },
          },
        });
        return;
      }
      const candidate: OwnedFlow = {
        api,
        id: flowId,
        destroying: false,
        attempts: 0,
        retryTimer: null,
      };
      candidateFlowRef.current = candidate;
      if (!current()) {
        if (candidateFlowRef.current === candidate)
          candidateFlowRef.current = null;
        destroyOwnedFlow(candidate, pendingDestroyRef);
        return;
      }

      const initialized = initializeAgentConfig(
        [targetId],
        discovered.existing.model_config,
        discovered.preset.model_config,
      );
      const baselines = createPanelBaselines(targetId, detected, discovered);
      activeFlowRef.current = candidate;
      candidateFlowRef.current = null;
      publish({
        ownerApi: api,
        ownerTarget: targetId,
        phase: { kind: "editing", refresh: { kind: "idle" } },
        detection: detected,
        credential: summary,
        discovery: discovered,
        target: {
          agent: targetId,
          mode,
          installedAtEntry: Boolean(state.command?.trim()),
        },
        config: baselines.form,
        source: initialized.sources[targetId],
        formBaseline: baselines.form,
        draftState: { error: "", hasLocalDraft: false },
      });
    })();

    return () => {
      mounted = false;
      generationRef.current += 1;
      cleanOwnedFlows(activeFlowRef, candidateFlowRef, pendingDestroyRef);
    };
  }, [api, targetId]);

  const ownerIsCurrent =
    snapshot.ownerApi === api && snapshot.ownerTarget === targetId;
  const visible = ownerIsCurrent ? snapshot : emptySnapshot(api, targetId);
  const dirty = Boolean(
    visible.draftState.hasLocalDraft ||
    (visible.config &&
      visible.formBaseline &&
      isConfigDirty(visible.config, visible.formBaseline)),
  );

  useEffect(() => {
    reportDirty(dirty);
  }, [dirty]);

  useEffect(() => () => reportDirty(false), []);

  return {
    phase: visible.phase,
    detection: visible.detection,
    credential: visible.credential,
    discovery: visible.discovery,
    target: visible.target,
    config: visible.config,
    source: visible.source,
    dirty,
    draftState: visible.draftState,
    setConfig: (config) => setSnapshot((current) => ({ ...current, config })),
    setDraftState: (draftState) =>
      setSnapshot((current) => ({ ...current, draftState })),
  };
}
