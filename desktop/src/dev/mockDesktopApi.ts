import type { UnlistenFn } from "@tauri-apps/api/event";

import type {
  AgentId,
  DesktopApi,
  ImageConversation,
  ImageCurrentOperation,
  ImageOperationEvent,
  ModelConfig,
  PollSnapshot,
  RouterStatus,
} from "../ipc";
import {
  discoveryFor,
  cleanupPreviewFor,
  fixtureAbsentStatus,
  fixtureConfigs,
  fixtureCredentialAbsent,
  fixtureCredentialPresent,
  fixtureDetection,
  fixtureDesktopPaths,
  fixtureHealthOk,
  fixtureOccupant,
  fixtureOwnedStatus,
  mockCommandError,
  parseMockScenario,
  pollSnapshotFor,
  previewFor,
  resolveMockScenarioFromLocation,
  type MockScenario,
  writeResultFor,
} from "./fixtures";

export interface MockDesktopApiOptions {
  scenario?: MockScenario;
  initialStatus?: RouterStatus;
  credentialPresent?: boolean;
}

type PollListener = (snapshot: PollSnapshot) => void;

function noopUnlisten(): UnlistenFn {
  return () => undefined;
}

function existingConfigFor(agents: AgentId[]): Partial<ModelConfig> {
  const existing: Partial<ModelConfig> = { version: 1 };
  for (const agent of agents) {
    if (agent === "claude" && fixtureConfigs.claude.claude) {
      existing.claude = structuredClone(fixtureConfigs.claude.claude);
    }
    if (agent === "opencode" && fixtureConfigs.opencode.opencode) {
      existing.opencode = structuredClone(fixtureConfigs.opencode.opencode);
    }
    if (agent === "codex" && fixtureConfigs.codex.codex) {
      existing.codex = structuredClone(fixtureConfigs.codex.codex);
    }
  }
  return existing;
}

export function createMockDesktopApi(
  options: MockDesktopApiOptions = {},
): DesktopApi {
  const browserMockMarker = "__MTLS_BROWSER_MOCK__";
  const scenario = options.scenario ?? resolveMockScenarioFromLocation();
  let status: RouterStatus = structuredClone(
    options.initialStatus ?? fixtureAbsentStatus,
  );
  let credential = structuredClone(
    options.credentialPresent === false
      ? fixtureCredentialAbsent
      : fixtureCredentialPresent,
  );
  let autostart = true;
  let revision = 0;
  let flowCounter = 0;
  const pollListeners = new Set<PollListener>();
  const activeFlows = new Set<string>();
  let imageConversations: ImageConversation[] = [];
  let imageOperation: ImageCurrentOperation | null = null;
  const imageOperationListeners = new Set<
    (event: ImageOperationEvent) => void
  >();

  function currentScenario(): MockScenario {
    if (typeof window !== "undefined") {
      const globalScenario = (
        window as Window & { __MTLS_MOCK_SCENARIO__?: string }
      ).__MTLS_MOCK_SCENARIO__;
      if (globalScenario) return parseMockScenario(globalScenario);
    }
    return scenario;
  }

  function emitPoll() {
    revision += 1;
    const snapshot = pollSnapshotFor(status, revision);
    for (const listener of pollListeners) listener(snapshot);
  }

  function nextFlowId(prefix: string): string {
    flowCounter += 1;
    return `${prefix}-${flowCounter}`;
  }

  function assertActiveFlow(flowId: string) {
    if (!activeFlows.has(flowId)) {
      throw mockCommandError("AGENT_FLOW_UNKNOWN");
    }
  }

  return {
    getPollSnapshot: async () => {
      revision += 1;
      return pollSnapshotFor(status, revision);
    },
    subscribePollSnapshots: async (listener) => {
      pollListeners.add(listener);
      return () => {
        pollListeners.delete(listener);
      };
    },
    subscribeMainWindowFocused: async () => noopUnlisten(),
    subscribeAgentDraftQuitRequested: async () => noopUnlisten(),
    setWindowVisibility: async () => undefined,
    setAgentDraftDirty: async () => undefined,
    resolveAppQuit: async () => undefined,
    getRouterStatus: async () => structuredClone(status),
    startRouter: async () => {
      if (currentScenario() === "protocol-error") {
        throw mockCommandError("MANAGER_PROTOCOL_ERROR");
      }
      status = structuredClone(fixtureOwnedStatus);
      emitPoll();
      return structuredClone(status);
    },
    stopRouter: async () => {
      status = structuredClone(fixtureAbsentStatus);
      emitPoll();
      return structuredClone(status);
    },
    inspectRouterOccupant: async () => structuredClone(fixtureOccupant),
    forceTerminateRouterOccupant: async () => ({
      termination: "process_terminated",
      port_state: "released",
    }),
    cancelRouterReleaseObservation: async () => undefined,
    retryRouterHealth: async () => ({
      ...fixtureHealthOk,
      checked_at: new Date().toISOString(),
    }),
    getComponentVersions: async () => ({
      desktop: "mock-desktop",
      manager: "mock-manager",
      router: "mock-router",
      management_protocol: "4",
    }),
    checkForUpdate: async () => ({
      available: true,
      current_version: "0.1.0-mock",
      update: {
        version: "0.2.0-mock",
        notes: "Mock update for browser-only UX testing.",
        published_at: "2026-08-01T00:00:00Z",
      },
    }),
    installUpdate: async () => undefined,
    subscribeUpdateProgress: async () => noopUnlisten(),
    getRouterLogs: async () => ({
      lines: [
        "mock: router ready on 127.0.0.1:19099",
        "mock: no real credentials or agent files are read or written",
      ],
    }),
    collectDiagnostics: async () => ({
      summary: `${browserMockMarker}: diagnostics summary (in-memory only)`,
    }),
    openLogLocation: async () => undefined,
    detectAgents: async () => {
      if (currentScenario() === "protocol-error") {
        throw mockCommandError("AGENT_DETECT_IO");
      }
      return structuredClone(fixtureDetection);
    },
    discoverModels: async (agents) => {
      if (currentScenario() === "protocol-error") {
        throw mockCommandError("AGENT_MODELS_UNAVAILABLE");
      }
      const flowId = nextFlowId(`flow-${agents[0] ?? "all"}`);
      activeFlows.add(flowId);
      return discoveryFor(flowId, existingConfigFor(agents), {});
    },
    renderAgentConfig: async (_agents, flowId, _catalog, modelConfig) => {
      assertActiveFlow(flowId);
      return {
        model_config: structuredClone(modelConfig),
        fragments: previewFor(modelConfig).fragments,
      };
    },
    previewAgents: async (agents, flowId, _catalog, modelConfig) => {
      assertActiveFlow(flowId);
      if (currentScenario() === "preview-stale") {
        throw mockCommandError("AGENT_PREVIEW_REVISION_MISMATCH");
      }
      if (currentScenario() === "protocol-error") {
        throw mockCommandError("AGENT_PREVIEW_FAILED");
      }
      return previewFor(structuredClone(modelConfig), {
        files: agents.map((agent) => ({
          agent,
          mode: "merge" as const,
          path: `/mock/${agent}/config`,
          role: "settings",
          format: agent === "codex" ? "toml" : "json",
          operation: "replace",
          backup_required: true,
        })),
      });
    },
    writeAgents: async (
      agents,
      flowId,
      _catalog,
      _modelConfig,
      revisionToken,
    ) => {
      assertActiveFlow(flowId);
      if (currentScenario() === "preview-stale") {
        throw mockCommandError("AGENT_WRITE_REVISION_MISMATCH");
      }
      if (
        currentScenario() === "write-fail" ||
        currentScenario() === "protocol-error"
      ) {
        throw mockCommandError("AGENT_WRITE_FAILED");
      }
      if (revisionToken !== "revision-mock") {
        throw mockCommandError("AGENT_WRITE_REVISION_MISMATCH");
      }
      return writeResultFor(agents, true);
    },
    previewAgentCleanup: async (agent) => {
      if (currentScenario() === "protocol-error") {
        throw mockCommandError("AGENT_CLEANUP_PREVIEW_FAILED");
      }
      return cleanupPreviewFor(agent);
    },
    writeAgentCleanup: async (agent, revisionToken) => {
      if (currentScenario() === "preview-stale") {
        throw mockCommandError("PREVIEW_STALE");
      }
      if (
        currentScenario() === "write-fail" ||
        currentScenario() === "protocol-error"
      ) {
        throw mockCommandError("AGENT_CLEANUP_WRITE_FAILED");
      }
      if (revisionToken !== `cleanup-revision-${agent}`) {
        throw mockCommandError("PREVIEW_STALE");
      }
      return writeResultFor([agent], true);
    },
    destroyAgentModelFlow: async (flowId) => {
      activeFlows.delete(flowId);
    },
    importAgentModelConfig: async (content) => {
      const parsed = JSON.parse(content) as ModelConfig;
      if (!parsed || parsed.version !== 1) {
        throw mockCommandError("AGENT_MODEL_CONFIG_INVALID");
      }
      return parsed;
    },
    exportAgentModelConfig: async (modelConfig) =>
      JSON.stringify(modelConfig, null, 2),
    getCredential: async () => structuredClone(credential),
    saveCredential: async (apiKey) => {
      if (!apiKey.trim()) {
        throw mockCommandError("CREDENTIAL_INVALID");
      }
      credential = {
        present: true,
        fingerprint: "MOCK",
        saved_at: new Date().toISOString(),
      };
      return structuredClone(credential);
    },
    deleteCredential: async () => {
      credential = structuredClone(fixtureCredentialAbsent);
      return structuredClone(credential);
    },
    getAutostart: async () => autostart,
    setAutostart: async (enabled) => {
      autostart = enabled;
      return enabled;
    },
    setNativeLanguage: async () => undefined,
    getDesktopPaths: async () => structuredClone(fixtureDesktopPaths),
    prepareForUninstall: async () => undefined,
    imageReadiness: async () => ({
      ready: true,
      available_models: [
        {
          id: "cx/gpt-5.5-image",
          display_name: "GPT 5.5 Image",
          available: true,
        },
        {
          id: "ag/gemini-3.1-flash-image",
          display_name: "Gemini 3.1 Flash Image",
          available: true,
        },
      ],
      reason: "ok",
    }),
    imageCurrentOperation: async () => structuredClone(imageOperation),
    imageConversations: async () => structuredClone(imageConversations),
    imageCreateConversation: async (model: string) => {
      const now = new Date().toISOString();
      const conversation = {
        id: crypto.randomUUID(),
        selected: true,
        title: "",
        selected_model: model,
        message_count: 0,
        created_at: now,
        updated_at: now,
      };
      imageConversations = [
        conversation,
        ...imageConversations.map((item) => ({ ...item, selected: false })),
      ];
      return structuredClone(conversation);
    },
    imageSelectConversation: async (conversationId) => {
      imageConversations = imageConversations.map((conversation) => ({
        ...conversation,
        selected: conversation.id === conversationId,
      }));
    },
    imageSetConversationModel: async (conversationId, model) => {
      imageConversations = imageConversations.map((conversation) =>
        conversation.id === conversationId
          ? {
              ...conversation,
              selected_model: model,
              updated_at: new Date().toISOString(),
            }
          : conversation,
      );
    },
    imageDeleteConversation: async (conversationId) => {
      imageConversations = imageConversations.filter(
        (conversation) => conversation.id !== conversationId,
      );
    },
    imageResetStore: async () => {
      imageConversations = [];
    },
    imageMessages: async () => [],
    imageSelectReference: async () => ({
      asset_id: "a".repeat(64),
      format: "png",
      width: 4,
      height: 4,
    }),
    imageStartGeneration: async (request) => {
      if (imageOperation) throw mockCommandError("IMAGE_BUSY");
      imageOperation = {
        operation_id: crypto.randomUUID(),
        conversation_id: request.conversation_id,
        message_id: crypto.randomUUID(),
      };
      const started = structuredClone(imageOperation);
      window.setTimeout(() => {
        if (imageOperation?.operation_id !== started.operation_id) return;
        imageOperation = null;
        const event: ImageOperationEvent = { ...started, status: "succeeded" };
        for (const listener of imageOperationListeners) listener(event);
      }, 50);
      return {
        operation_id: started.operation_id,
        message_id: started.message_id,
      };
    },
    imageCancelGeneration: async () => {
      if (!imageOperation) return;
      const event: ImageOperationEvent = {
        ...imageOperation,
        status: "cancelled",
      };
      imageOperation = null;
      for (const listener of imageOperationListeners) listener(event);
    },
    subscribeImageOperations: async (listener) => {
      imageOperationListeners.add(listener);
      return () => imageOperationListeners.delete(listener);
    },
  };
}

/** Test/helper: force the in-memory scenario used by an existing mock API. */
export function setMockScenario(next: MockScenario): void {
  if (typeof window !== "undefined") {
    (
      window as Window & { __MTLS_MOCK_SCENARIO__?: string }
    ).__MTLS_MOCK_SCENARIO__ = next;
  }
}
