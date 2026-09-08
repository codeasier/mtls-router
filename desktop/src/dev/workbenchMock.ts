import type { UnlistenFn } from "@tauri-apps/api/event";

import type {
  WorkbenchAsset,
  WorkbenchChatDelta,
  WorkbenchConversation,
  WorkbenchImageChoice,
  WorkbenchImageStatusEvent,
  WorkbenchOperationDone,
  WorkbenchPhaseEvent,
  WorkbenchReadiness,
  WorkbenchSnapshot,
} from "../ipc";

export const MOCK_WORKBENCH_ASSET_ID =
  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

const IMAGE_HINT =
  /画|绘|生成|出一张|做一张|来一张|重画|改成|改一下|换成|imagine|draw|generate|render|paint|illustration/i;

export function defaultWorkbenchChoices(): WorkbenchImageChoice[] {
  return [
    {
      id: "ag/gemini-3.1-flash-image",
      display_name: "ag/gemini-3.1-flash-image",
      verified_edit: false,
      allowed: true,
    },
    {
      id: "cx/gpt-5.5-image",
      display_name: "cx/gpt-5.5-image",
      verified_edit: true,
      allowed: true,
    },
  ];
}

export function defaultWorkbenchReadiness(
  overrides: Partial<WorkbenchReadiness> = {},
): WorkbenchReadiness {
  return {
    ready: true,
    has_credential: true,
    router_trusted: true,
    health_ok: true,
    store_corrupt: false,
    store_incompatible: false,
    chat_models: ["gemini-3.8-flash", "gemini-3-flash"],
    image_models: defaultWorkbenchChoices(),
    can_submit: true,
    reason: null,
    busy: false,
    ...overrides,
  };
}

export function emptyWorkbenchSnapshot(): WorkbenchSnapshot {
  return {
    selected_conversation_id: null,
    conversations: [],
    messages: [],
    jobs: [],
    assets: [],
  };
}

function mockError(code: string) {
  const error = new Error(code) as Error & { code: string };
  error.code = code;
  return error;
}

function now() {
  return "2026-09-08T10:00:00Z";
}

function nextId(prefix: string, counter: { value: number }) {
  counter.value += 1;
  return `${prefix}-${counter.value}`;
}

export function createWorkbenchMockHandlers(
  options: {
    readiness?: Partial<WorkbenchReadiness>;
  } = {},
) {
  let readiness = defaultWorkbenchReadiness(options.readiness);
  const snapshot = emptyWorkbenchSnapshot();
  const ids = { value: 0 };
  let busy = false;
  const chatListeners = new Set<(event: WorkbenchChatDelta) => void>();
  const phaseListeners = new Set<(event: WorkbenchPhaseEvent) => void>();
  const imageListeners = new Set<(event: WorkbenchImageStatusEvent) => void>();
  const doneListeners = new Set<(event: WorkbenchOperationDone) => void>();

  function currentReadiness(): WorkbenchReadiness {
    const selected = snapshot.conversations.find(
      (item) => item.id === snapshot.selected_conversation_id,
    );
    const chatOk = Boolean(
      selected?.selected_chat_model &&
      readiness.chat_models.includes(selected.selected_chat_model),
    );
    const imageOk = Boolean(
      selected?.selected_image_model &&
      readiness.image_models.some(
        (item) => item.id === selected.selected_image_model,
      ),
    );
    const ready =
      readiness.has_credential &&
      readiness.router_trusted &&
      readiness.health_ok &&
      !readiness.store_corrupt &&
      !readiness.store_incompatible &&
      chatOk &&
      imageOk;
    return {
      ...readiness,
      ready,
      can_submit: ready && !busy,
      busy,
      reason: readiness.store_incompatible
        ? "WORKBENCH_STORE_INCOMPATIBLE"
        : readiness.store_corrupt
          ? "WORKBENCH_STORE_CORRUPT"
          : !readiness.has_credential
            ? "WORKBENCH_NO_CREDENTIAL"
            : !readiness.router_trusted
              ? "WORKBENCH_ROUTER"
              : !readiness.health_ok
                ? "WORKBENCH_CATALOG"
                : !chatOk || !imageOk
                  ? "WORKBENCH_MODEL_MISSING"
                  : busy
                    ? "WORKBENCH_BUSY"
                    : null,
    };
  }

  function cloneSnapshot(): WorkbenchSnapshot {
    return structuredClone(snapshot);
  }

  return {
    getReadiness: async () => currentReadiness(),
    refreshCatalogs: async () => currentReadiness(),
    list: async () => cloneSnapshot(),
    create: async () => {
      const conversation: WorkbenchConversation = {
        id: nextId("c", ids),
        title: "新对话",
        created_at: now(),
        updated_at: now(),
        selected_chat_model: readiness.chat_models.includes("gemini-3.8-flash")
          ? "gemini-3.8-flash"
          : null,
        selected_image_model: readiness.image_models.some(
          (item) => item.id === "ag/gemini-3.1-flash-image",
        )
          ? "ag/gemini-3.1-flash-image"
          : null,
        selected_size: "1024x1024",
        message_ids: [],
      };
      snapshot.conversations.unshift(conversation);
      snapshot.selected_conversation_id = conversation.id;
      return structuredClone(conversation);
    },
    select: async (conversationId: string) => {
      if (!snapshot.conversations.some((item) => item.id === conversationId)) {
        throw mockError("WORKBENCH_FAILED");
      }
      snapshot.selected_conversation_id = conversationId;
      return conversationId;
    },
    remove: async (conversationId: string) => {
      snapshot.conversations = snapshot.conversations.filter(
        (item) => item.id !== conversationId,
      );
      snapshot.messages = snapshot.messages.filter(
        (item) => item.conversation_id !== conversationId,
      );
      snapshot.jobs = snapshot.jobs.filter((job) =>
        snapshot.messages.some((item) => item.id === job.message_id),
      );
      if (snapshot.selected_conversation_id === conversationId) {
        snapshot.selected_conversation_id =
          snapshot.conversations[0]?.id ?? null;
      }
      return {
        selected_conversation_id: snapshot.selected_conversation_id,
        conversations: structuredClone(snapshot.conversations),
      };
    },
    setOptions: async (
      conversationId: string,
      options: { chatModel?: string; imageModel?: string; size?: string },
    ) => {
      const conversation = snapshot.conversations.find(
        (item) => item.id === conversationId,
      );
      if (!conversation) throw mockError("WORKBENCH_FAILED");
      if (options.chatModel !== undefined) {
        conversation.selected_chat_model = options.chatModel;
      }
      if (options.imageModel !== undefined) {
        conversation.selected_image_model = options.imageModel;
      }
      if (options.size !== undefined) conversation.selected_size = options.size;
    },
    pickReference: async () => mockAsset("upload"),
    importBytes: async (bytes: Uint8Array) => {
      if (bytes.byteLength > 20 * 1024 * 1024) {
        throw mockError("WORKBENCH_IMAGE_TOO_LARGE");
      }
      if (bytes.byteLength < 8) throw mockError("WORKBENCH_IMAGE_INVALID");
      return mockAsset("import");
    },
    quoteAsset: async (assetId: string) => {
      const asset = snapshot.assets.find((item) => item.id === assetId);
      if (!asset) throw mockError("WORKBENCH_IMAGE_INVALID");
      return structuredClone(asset);
    },
    send: async (
      conversationId: string,
      text: string,
      forceImage: boolean,
      referenceAssetId?: string | null,
    ) => {
      if (busy) throw mockError("WORKBENCH_BUSY");
      const conversation = snapshot.conversations.find(
        (item) => item.id === conversationId,
      );
      if (!conversation) throw mockError("WORKBENCH_FAILED");
      const trimmed = text.trim();
      if (!trimmed && !referenceAssetId)
        throw mockError("WORKBENCH_INPUT_EMPTY");
      busy = true;
      const userId = nextId("m", ids);
      const assistantId = nextId("m", ids);
      const visible = trimmed || "根据这张参考图继续改。";
      if (!conversation.message_ids.length) {
        conversation.title = visible.slice(0, 18);
      }
      conversation.message_ids.push(userId, assistantId);
      snapshot.messages.push({
        id: userId,
        conversation_id: conversationId,
        role: "user",
        visible_text: visible,
        created_at: now(),
        reference_asset_id: referenceAssetId ?? null,
        phase: null,
        error_kind: null,
        job_ids: [],
      });
      snapshot.messages.push({
        id: assistantId,
        conversation_id: conversationId,
        role: "assistant",
        visible_text:
          forceImage || IMAGE_HINT.test(visible)
            ? "好，这就出图。"
            : "好的，我们再聊聊构图。",
        created_at: now(),
        reference_asset_id: null,
        phase: null,
        error_kind: null,
        job_ids: [],
      });
      const assistant = snapshot.messages[snapshot.messages.length - 1];
      if (forceImage || Boolean(referenceAssetId) || IMAGE_HINT.test(visible)) {
        const jobId = nextId("j", ids);
        const asset = mockAsset("generate");
        snapshot.assets.push(asset);
        snapshot.jobs.push({
          id: jobId,
          message_id: assistantId,
          action: referenceAssetId ? "edit" : "generate",
          prompt: "mock-prompt",
          size: conversation.selected_size,
          status: "succeeded",
          output_asset_id: asset.id,
          error_kind: null,
        });
        assistant.job_ids.push(jobId);
      }
      for (const listener of phaseListeners) {
        listener({
          operation_id: "op-mock",
          conversation_id: conversationId,
          message_id: assistantId,
          phase: "thinking",
        });
      }
      for (const listener of chatListeners) {
        listener({
          operation_id: "op-mock",
          conversation_id: conversationId,
          message_id: assistantId,
          visible_text: assistant.visible_text,
        });
      }
      for (const listener of doneListeners) {
        listener({
          operation_id: "op-mock",
          conversation_id: conversationId,
        });
      }
      busy = false;
      return cloneSnapshot();
    },
    regenerate: async (conversationId: string, jobId: string) => {
      if (busy) throw mockError("WORKBENCH_BUSY");
      const job = snapshot.jobs.find((item) => item.id === jobId);
      if (!job) throw mockError("WORKBENCH_FAILED");
      const asset = mockAsset("generate");
      snapshot.assets.push(asset);
      job.status = "succeeded";
      job.output_asset_id = asset.id;
      void conversationId;
      return {
        messages: structuredClone(snapshot.messages),
        jobs: structuredClone(snapshot.jobs),
        assets: structuredClone(snapshot.assets),
      };
    },
    cancel: async () => {
      busy = false;
    },
    saveAsset: async () => undefined,
    rebuild: async () => {
      snapshot.conversations = [];
      snapshot.messages = [];
      snapshot.jobs = [];
      snapshot.assets = [];
      snapshot.selected_conversation_id = null;
      readiness = {
        ...readiness,
        store_corrupt: false,
        store_incompatible: false,
      };
      return { version: 1 };
    },
    subscribeChatDelta: async (
      listener: (event: WorkbenchChatDelta) => void,
    ): Promise<UnlistenFn> => {
      chatListeners.add(listener);
      return () => {
        chatListeners.delete(listener);
      };
    },
    subscribePhase: async (
      listener: (event: WorkbenchPhaseEvent) => void,
    ): Promise<UnlistenFn> => {
      phaseListeners.add(listener);
      return () => {
        phaseListeners.delete(listener);
      };
    },
    subscribeImageStatus: async (
      listener: (event: WorkbenchImageStatusEvent) => void,
    ): Promise<UnlistenFn> => {
      imageListeners.add(listener);
      return () => {
        imageListeners.delete(listener);
      };
    },
    subscribeDone: async (
      listener: (event: WorkbenchOperationDone) => void,
    ): Promise<UnlistenFn> => {
      doneListeners.add(listener);
      return () => {
        doneListeners.delete(listener);
      };
    },
    setReadiness(next: Partial<WorkbenchReadiness>) {
      readiness = defaultWorkbenchReadiness({ ...readiness, ...next });
    },
    snapshot,
  };
}

function mockAsset(source: string): WorkbenchAsset {
  return {
    id: MOCK_WORKBENCH_ASSET_ID,
    format: "png",
    byte_len: 67,
    width: 1,
    height: 1,
    source,
  };
}
