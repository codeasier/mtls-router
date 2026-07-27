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
  panelOperationAvailability,
  isConfigDirty,
  sameExternalSnapshot,
  targetMode,
  type PanelOperationAvailability,
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
  externalBaseline: string | null;
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
    externalBaseline: null,
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
  retryPendingFlows(pendingRef);
}

function retryPendingFlows(pendingRef: PendingFlowRef) {
  for (const pending of [...pendingRef.current]) {
    if (pending.retryTimer) {
      clearTimeout(pending.retryTimer);
      pending.retryTimer = null;
    }
    destroyOwnedFlow(pending, pendingRef);
  }
}

function snapshotDirty(snapshot: ControllerSnapshot) {
  return Boolean(
    snapshot.draftState.hasLocalDraft ||
    (snapshot.config &&
      snapshot.formBaseline &&
      isConfigDirty(snapshot.config, snapshot.formBaseline)),
  );
}

type AgentPanelOperationPhase =
  "preview-loading" | "previewing" | "writing" | "reloading";

export interface AgentPanelController {
  phase: PanelPhase;
  detection: AgentDetection | null;
  credential: CredentialSummary | null;
  discovery: AgentModelsResult | null;
  target: AgentTarget | null;
  config: ModelConfig | null;
  source: InitializationSource | null;
  dirty: boolean;
  operations: PanelOperationAvailability;
  draftState: AgentConfigDraftState;
  setConfig(config: ModelConfig): void;
  setDraftState(state: AgentConfigDraftState): void;
  refresh(): Promise<void>;
  retryBlockedDraft(): Promise<void>;
  discardBlockedDraft(): Promise<void>;
  resolveConflict(choice: "preserve" | "discard"): void;
  /** @internal Task 7 must replace this with real operation methods and delete it. */
  __occupyOperationForTask7(phase: AgentPanelOperationPhase | null): void;
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
  const snapshotRef = useRef(snapshot);
  const generationRef = useRef(0);
  const activeFlowRef = useRef<OwnedFlow | null>(null);
  const candidateFlowRef = useRef<OwnedFlow | null>(null);
  const pendingDestroyRef = useRef<OwnedFlow[]>([]);
  const refreshInFlightRef = useRef<Promise<void> | null>(null);
  const refreshPendingRef = useRef(false);
  const lastCandidateStartedAtRef = useRef<number | null>(null);
  const committedStartRefreshRef = useRef<
    (manual: boolean, blockedRetry?: boolean) => Promise<void>
  >(() => Promise.resolve());
  const reportDirty = useEffectEvent(onDirtyChange);

  const commitSnapshot = (
    update: (current: ControllerSnapshot) => ControllerSnapshot,
  ) => {
    const next = update(snapshotRef.current);
    snapshotRef.current = next;
    setSnapshot(next);
  };

  useLayoutEffect(() => {
    const generation = ++generationRef.current;
    let mounted = true;
    const current = () => mounted && generation === generationRef.current;
    const publish = (next: ControllerSnapshot) => {
      if (!current()) return;
      snapshotRef.current = next;
      setSnapshot(next);
    };

    const initial = emptySnapshot(api, targetId);
    snapshotRef.current = initial;
    setSnapshot(initial);

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
        retryPendingFlows(pendingDestroyRef);
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
        externalBaseline: baselines.external,
        draftState: { error: "", hasLocalDraft: false },
      });
    })();

    return () => {
      mounted = false;
      generationRef.current += 1;
      refreshInFlightRef.current = null;
      refreshPendingRef.current = false;
      lastCandidateStartedAtRef.current = null;
      cleanOwnedFlows(activeFlowRef, candidateFlowRef, pendingDestroyRef);
    };
  }, [api, targetId]);

  const startRefresh = (
    manual: boolean,
    blockedRetry = false,
  ): Promise<void> => {
    const before = snapshotRef.current;
    const normalEditing = before.phase.kind === "editing";
    if (!normalEditing) {
      if (!(blockedRetry && before.phase.kind === "blocked-dirty")) {
        if (
          before.phase.kind === "preview-loading" ||
          before.phase.kind === "previewing" ||
          before.phase.kind === "writing" ||
          before.phase.kind === "reloading"
        ) {
          refreshPendingRef.current = true;
        }
        return Promise.resolve();
      }
    }
    if (refreshInFlightRef.current) return refreshInFlightRef.current;
    if (
      before.phase.kind === "editing" &&
      (before.phase.refresh.kind === "checking" ||
        before.phase.refresh.kind === "conflict")
    ) {
      return Promise.resolve();
    }

    const now = Date.now();
    if (
      !manual &&
      lastCandidateStartedAtRef.current !== null &&
      now - lastCandidateStartedAtRef.current < 15_000
    ) {
      return Promise.resolve();
    }
    lastCandidateStartedAtRef.current = now;
    const generation = generationRef.current;
    if (normalEditing) {
      commitSnapshot((current) =>
        current.phase.kind === "editing"
          ? {
              ...current,
              phase: { kind: "editing", refresh: { kind: "checking" } },
            }
          : current,
      );
    }

    const request = (async () => {
      let detected: AgentDetection;
      try {
        retryPendingFlows(pendingDestroyRef);
        const complete = completeAgentDetection(await api.detectAgents());
        if (!complete) throw { code: "AGENT_DETECT_FAILED" };
        detected = complete;
      } catch (error) {
        if (generation !== generationRef.current) return;
        commitSnapshot((current) => {
          const code = errorCode(error, "AGENT_DETECT_FAILED");
          if (blockedRetry && current.phase.kind === "blocked-dirty") {
            return { ...current, phase: { ...current.phase, errorCode: code } };
          }
          return current.phase.kind === "editing"
            ? {
                ...current,
                phase: {
                  kind: "editing",
                  refresh: { kind: "failed", code },
                },
              }
            : current;
        });
        return;
      }
      if (generation !== generationRef.current) return;

      const targetState = detected.agents.find(
        (agent) => agent.agent === targetId,
      )!;
      const candidateMode = targetMode(targetState);
      const currentSnapshot = snapshotRef.current;
      const currentMode = currentSnapshot.target?.mode ?? null;
      const dirty = snapshotDirty(currentSnapshot);
      const changingMode = candidateMode !== currentMode;
      if (changingMode) {
        if (dirty) {
          commitSnapshot((current) => ({
            ...current,
            detection: detected,
            phase: {
              kind: "blocked-dirty",
              canExport: activeFlowRef.current !== null,
              errorCode: null,
            },
          }));
          return;
        }
        if (candidateMode === null) {
          const active = activeFlowRef.current;
          activeFlowRef.current = null;
          if (active) destroyOwnedFlow(active, pendingDestroyRef);
          commitSnapshot((current) => ({
            ...current,
            detection: detected,
            discovery: null,
            target: null,
            config: null,
            source: null,
            formBaseline: null,
            externalBaseline: null,
            draftState: { error: "", hasLocalDraft: false },
            phase: {
              kind: "readonly",
              reason: {
                kind: targetState.invalid ? "not-recoverable" : "not-writable",
              },
            },
          }));
          return;
        }
        const active = activeFlowRef.current;
        activeFlowRef.current = null;
        if (active) destroyOwnedFlow(active, pendingDestroyRef);
        commitSnapshot((current) => ({
          ...current,
          phase: { kind: "loading" },
          detection: detected,
          discovery: null,
          target: null,
          config: null,
          source: null,
          formBaseline: null,
          externalBaseline: null,
          draftState: { error: "", hasLocalDraft: false },
        }));
      }

      let discovered: AgentModelsResult;
      try {
        retryPendingFlows(pendingDestroyRef);
        discovered = await api.discoverModels([targetId]);
      } catch (error) {
        if (generation !== generationRef.current) return;
        commitSnapshot((current) => {
          const code = errorCode(error, "MODEL_DISCOVERY_FAILED");
          if (blockedRetry && current.phase.kind === "blocked-dirty") {
            return { ...current, phase: { ...current.phase, errorCode: code } };
          }
          if (changingMode) {
            return {
              ...current,
              phase: { kind: "readonly", reason: { kind: "catalog", code } },
            };
          }
          return current.phase.kind === "editing"
            ? {
                ...current,
                phase: {
                  kind: "editing",
                  refresh: { kind: "failed", code },
                },
              }
            : current;
        });
        return;
      }

      const flowId = discovered.flow_id.trim();
      if (!flowId) {
        if (generation !== generationRef.current) return;
        commitSnapshot((current) => {
          if (blockedRetry && current.phase.kind === "blocked-dirty") {
            return {
              ...current,
              phase: { ...current.phase, errorCode: "MODEL_RESPONSE_INVALID" },
            };
          }
          if (changingMode) {
            return {
              ...current,
              phase: {
                kind: "readonly",
                reason: { kind: "catalog", code: "MODEL_RESPONSE_INVALID" },
              },
            };
          }
          return current.phase.kind === "editing"
            ? {
                ...current,
                phase: {
                  kind: "editing",
                  refresh: { kind: "failed", code: "MODEL_RESPONSE_INVALID" },
                },
              }
            : current;
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
      if (generation !== generationRef.current) {
        if (candidateFlowRef.current === candidate)
          candidateFlowRef.current = null;
        destroyOwnedFlow(candidate, pendingDestroyRef);
        return;
      }

      const baselines = createPanelBaselines(targetId, detected, discovered);
      const initialized = initializeAgentConfig(
        [targetId],
        discovered.existing.model_config,
        discovered.preset.model_config,
      );
      const latest = snapshotRef.current;
      const latestDirty = snapshotDirty(latest);
      const externalChanged = !sameExternalSnapshot(
        latest.externalBaseline ?? "",
        baselines.external,
      );
      const oldActive = activeFlowRef.current;
      activeFlowRef.current = candidate;
      candidateFlowRef.current = null;

      commitSnapshot((current) => {
        const common = {
          ...current,
          detection: detected,
          discovery: discovered,
          target: {
            agent: targetId,
            mode: candidateMode!,
            installedAtEntry: Boolean(targetState.command?.trim()),
          },
          externalBaseline: baselines.external,
        };
        if (!latestDirty) {
          return {
            ...common,
            phase: { kind: "editing", refresh: { kind: "idle" } },
            config: baselines.form,
            source: initialized.sources[targetId],
            formBaseline: baselines.form,
            draftState: { error: "", hasLocalDraft: false },
          };
        }
        if (!externalChanged) {
          return {
            ...common,
            phase: { kind: "editing", refresh: { kind: "idle" } },
          };
        }
        return {
          ...common,
          phase: {
            kind: "editing",
            refresh: {
              kind: "conflict",
              candidate: {
                detection: detected,
                discovery: discovered,
                externalBaseline: baselines.external,
              },
            },
          },
        };
      });
      if (oldActive) destroyOwnedFlow(oldActive, pendingDestroyRef);
    })().finally(() => {
      if (refreshInFlightRef.current === request)
        refreshInFlightRef.current = null;
    });
    refreshInFlightRef.current = request;
    return request;
  };

  useLayoutEffect(() => {
    committedStartRefreshRef.current = startRefresh;
  });

  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | null = null;
    const subscribedRefresh = committedStartRefreshRef.current;
    void api
      .subscribeMainWindowFocused(() => void subscribedRefresh(false))
      .then((stop) => {
        if (disposed) stop();
        else unsubscribe = stop;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [api, targetId]);

  useEffect(() => {
    if (
      snapshot.phase.kind === "editing" &&
      refreshPendingRef.current &&
      !refreshInFlightRef.current
    ) {
      refreshPendingRef.current = false;
      void committedStartRefreshRef.current(true);
    }
  }, [snapshot.phase]);

  const ownerIsCurrent =
    snapshot.ownerApi === api && snapshot.ownerTarget === targetId;
  const visible = ownerIsCurrent ? snapshot : emptySnapshot(api, targetId);
  const dirty = snapshotDirty(visible);
  const operations = panelOperationAvailability(
    visible.phase,
    ownerIsCurrent && activeFlowRef.current !== null,
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
    operations,
    draftState: visible.draftState,
    setConfig: (config) => {
      if (
        !panelOperationAvailability(
          snapshotRef.current.phase,
          activeFlowRef.current !== null,
        ).edit
      )
        return;
      commitSnapshot((current) => ({ ...current, config }));
    },
    setDraftState: (draftState) => {
      if (
        !panelOperationAvailability(
          snapshotRef.current.phase,
          activeFlowRef.current !== null,
        ).edit
      )
        return;
      commitSnapshot((current) => ({ ...current, draftState }));
    },
    refresh: () => committedStartRefreshRef.current(true),
    retryBlockedDraft: () => committedStartRefreshRef.current(true, true),
    discardBlockedDraft: async () => {
      const current = snapshotRef.current;
      if (current.phase.kind !== "blocked-dirty" || !current.detection) return;
      const state = current.detection.agents.find(
        (agent) => agent.agent === targetId,
      )!;
      const mode = targetMode(state);
      const active = activeFlowRef.current;
      activeFlowRef.current = null;
      if (active) destroyOwnedFlow(active, pendingDestroyRef);
      if (!mode) {
        commitSnapshot((latest) => ({
          ...latest,
          discovery: null,
          target: null,
          config: null,
          source: null,
          formBaseline: null,
          externalBaseline: null,
          draftState: { error: "", hasLocalDraft: false },
          phase: {
            kind: "readonly",
            reason: {
              kind: state.invalid ? "not-recoverable" : "not-writable",
            },
          },
        }));
        return;
      }
      commitSnapshot((latest) => ({
        ...latest,
        config: latest.formBaseline,
        draftState: { error: "", hasLocalDraft: false },
        phase: { kind: "editing", refresh: { kind: "idle" } },
      }));
      await committedStartRefreshRef.current(true);
    },
    resolveConflict: (choice) => {
      const current = snapshotRef.current;
      if (
        current.phase.kind !== "editing" ||
        current.phase.refresh.kind !== "conflict"
      )
        return;
      if (choice === "preserve") {
        const candidate = current.phase.refresh.candidate;
        const baselines = createPanelBaselines(
          targetId,
          candidate.detection,
          candidate.discovery,
        );
        commitSnapshot((latest) => ({
          ...latest,
          formBaseline: baselines.form,
          externalBaseline: candidate.externalBaseline,
          phase: { kind: "editing", refresh: { kind: "idle" } },
        }));
        return;
      }
      const candidate = current.phase.refresh.candidate;
      const baselines = createPanelBaselines(
        targetId,
        candidate.detection,
        candidate.discovery,
      );
      const initialized = initializeAgentConfig(
        [targetId],
        candidate.discovery.existing.model_config,
        candidate.discovery.preset.model_config,
      );
      commitSnapshot((latest) => ({
        ...latest,
        config: baselines.form,
        formBaseline: baselines.form,
        externalBaseline: baselines.external,
        source: initialized.sources[targetId],
        draftState: { error: "", hasLocalDraft: false },
        phase: { kind: "editing", refresh: { kind: "idle" } },
      }));
    },
    __occupyOperationForTask7: (phase) => {
      const current = snapshotRef.current;
      if (phase === null) {
        if (
          current.phase.kind === "preview-loading" ||
          current.phase.kind === "previewing" ||
          current.phase.kind === "writing" ||
          current.phase.kind === "reloading"
        ) {
          commitSnapshot((latest) => ({
            ...latest,
            phase: { kind: "editing", refresh: { kind: "idle" } },
          }));
        }
        return;
      }
      if (
        current.phase.kind === "editing" &&
        (current.phase.refresh.kind === "idle" ||
          current.phase.refresh.kind === "failed")
      ) {
        commitSnapshot((latest) => ({ ...latest, phase: { kind: phase } }));
      }
    },
  };
}
