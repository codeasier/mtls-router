import type { UnlistenFn } from "@tauri-apps/api/event";

import type {
  WorkbenchAsset,
  WorkbenchChatDelta,
  WorkbenchConversation,
  WorkbenchImageJob,
  WorkbenchImageStatusEvent,
  WorkbenchMessage,
  WorkbenchOperationDone,
  WorkbenchPhaseEvent,
  WorkbenchReadiness,
  WorkbenchSnapshot,
} from "../ipc";
import {
  registerLiveWorkbenchAssetUrl,
  revokeAllLiveWorkbenchAssets,
  revokeLiveWorkbenchAssetUrl,
} from "../workbench";
import {
  EMPTY_REF_EN,
  EMPTY_REF_ZH,
  decideJobs,
  extractImagePayload,
  groupedImageChoices,
  parseChatModels,
  parseImageModels,
  rebuildChatMessages,
  selectChatModel,
  selectImageModel,
  sseContent,
  titleFrom,
} from "./workbenchOrchestrate";

export const LIVE_WORKBENCH_MARKER = "__MTLS_LIVE_WORKBENCH__";
export const LIVE_ROUTER_PREFIX = "/__router";
export const LIVE_STATUS_PATH = "/__workbench-live";
export const LIVE_KEY_STORAGE = "mtls-workbench-live-key";
export const LIVE_UPSTREAM = "127.0.0.1:19099";

const MAX_IMAGE_BYTES = 20 * 1024 * 1024;
const CATALOG_TIMEOUT_MS = 45_000;
const CATALOG_TTL_MS = 60_000;

export interface LiveWorkbenchOptions {
  fetch?: typeof fetch;
  english?: boolean;
  now?: () => string;
}

interface StoredAsset extends WorkbenchAsset {
  bytes: Uint8Array;
  mime: string;
}

function liveError(code: string): Error & { code: string } {
  const error = new Error(code) as Error & { code: string };
  error.code = code;
  return error;
}

export function readLiveWorkbenchSessionKey(): string {
  try {
    return sessionStorage.getItem(LIVE_KEY_STORAGE)?.trim() ?? "";
  } catch {
    return "";
  }
}

export function setLiveWorkbenchSessionKey(value: string): void {
  try {
    const trimmed = value.trim();
    if (trimmed) sessionStorage.setItem(LIVE_KEY_STORAGE, trimmed);
    else sessionStorage.removeItem(LIVE_KEY_STORAGE);
  } catch {
    // sessionStorage can be unavailable.
  }
}

export function createWorkbenchLiveHandlers(
  options: LiveWorkbenchOptions = {},
) {
  void LIVE_WORKBENCH_MARKER;
  const fetchImpl = options.fetch ?? fetch;
  const english = Boolean(options.english);
  const now = options.now ?? (() => new Date().toISOString());
  const snapshot: WorkbenchSnapshot = {
    selected_conversation_id: null,
    conversations: [],
    messages: [],
    jobs: [],
    assets: [],
  };
  const blobs = new Map<string, StoredAsset>();
  const chatListeners = new Set<(event: WorkbenchChatDelta) => void>();
  const phaseListeners = new Set<(event: WorkbenchPhaseEvent) => void>();
  const imageListeners = new Set<(event: WorkbenchImageStatusEvent) => void>();
  const doneListeners = new Set<(event: WorkbenchOperationDone) => void>();
  let ids = 0;
  let busy = false;
  let abort: AbortController | null = null;
  let chatModels: string[] = [];
  let imageModels: string[] = [];
  let routerReachable = false;
  let healthOk = false;
  let catalogOk = false;
  let catalogFetchedAt = 0;
  let serverKey = false;

  function nextId(prefix: string): string {
    ids += 1;
    return `${prefix}-${ids}`;
  }

  function cloneSnapshot(): WorkbenchSnapshot {
    return structuredClone(snapshot);
  }

  function authorizationHeaders(): Headers {
    const headers = new Headers();
    const key = readLiveWorkbenchSessionKey();
    if (key) headers.set("Authorization", `Bearer ${key}`);
    return headers;
  }

  async function routerFetch(
    path: string,
    init: RequestInit = {},
  ): Promise<Response> {
    const headers = authorizationHeaders();
    new Headers(init.headers).forEach((value, key) => {
      headers.set(key, value);
    });
    try {
      return await fetchImpl(`${LIVE_ROUTER_PREFIX}${path}`, {
        ...init,
        headers,
      });
    } catch (cause) {
      if (abort?.signal.aborted) {
        throw liveError(
          abort.signal.reason === "timeout"
            ? "WORKBENCH_TIMEOUT"
            : "WORKBENCH_CANCELLED",
        );
      }
      throw cause instanceof Error && "code" in cause
        ? cause
        : liveError("WORKBENCH_NOT_READY");
    }
  }

  async function fetchProbe(url: string, ms = 8_000): Promise<Response> {
    const controller = new AbortController();
    const timer = window.setTimeout(() => controller.abort("timeout"), ms);
    try {
      return await fetchImpl(url, { signal: controller.signal });
    } finally {
      window.clearTimeout(timer);
    }
  }

  async function refreshLiveStatus(): Promise<void> {
    try {
      const status = await fetchProbe(LIVE_STATUS_PATH, 3_000);
      if (status.ok) {
        const body = (await status.json()) as { has_server_key?: boolean };
        serverKey = Boolean(body.has_server_key);
      }
    } catch {
      serverKey = false;
    }
    try {
      const health = await fetchProbe(`${LIVE_ROUTER_PREFIX}/health`, 5_000);
      routerReachable = health.ok;
      if (!health.ok) {
        healthOk = false;
        return;
      }
      const body = (await health.json()) as { status?: string };
      healthOk = body.status === "ok";
    } catch {
      routerReachable = false;
      healthOk = false;
    }
  }

  async function refreshCatalogs(force = true): Promise<WorkbenchReadiness> {
    try {
      return await refreshCatalogsInner(force);
    } catch {
      catalogOk = false;
      return currentReadiness();
    }
  }

  async function refreshCatalogsInner(
    force: boolean,
  ): Promise<WorkbenchReadiness> {
    await refreshLiveStatus();
    if (!routerReachable || !healthOk) {
      chatModels = [];
      imageModels = [];
      catalogOk = false;
      catalogFetchedAt = 0;
      return currentReadiness();
    }
    if (
      !force &&
      catalogOk &&
      catalogFetchedAt > 0 &&
      Date.now() - catalogFetchedAt < CATALOG_TTL_MS
    ) {
      return currentReadiness();
    }
    const controller = new AbortController();
    const timer = window.setTimeout(
      () => controller.abort("timeout"),
      CATALOG_TIMEOUT_MS,
    );
    try {
      const [chat, image] = await Promise.all([
        routerFetch("/v1/models", { signal: controller.signal }),
        routerFetch("/v1/models/image", { signal: controller.signal }),
      ]);
      if (chat.status === 401 || image.status === 401) {
        catalogOk = false;
        catalogFetchedAt = 0;
        chatModels = [];
        imageModels = [];
        return currentReadiness();
      }
      if (!chat.ok || !image.ok) {
        catalogOk = false;
        catalogFetchedAt = 0;
        return currentReadiness();
      }
      chatModels = parseChatModels(await chat.json());
      imageModels = parseImageModels(await image.json());
      catalogOk = chatModels.length > 0 && imageModels.length > 0;
      catalogFetchedAt = catalogOk ? Date.now() : 0;
    } catch {
      catalogOk = false;
      catalogFetchedAt = 0;
    } finally {
      window.clearTimeout(timer);
    }
    return currentReadiness();
  }

  function hasCredential(): boolean {
    return serverKey || Boolean(readLiveWorkbenchSessionKey());
  }

  function currentReadiness(): WorkbenchReadiness {
    const selected = snapshot.conversations.find(
      (item) => item.id === snapshot.selected_conversation_id,
    );
    const chatOk = Boolean(
      selected?.selected_chat_model &&
      chatModels.includes(selected.selected_chat_model),
    );
    const imageChoices = groupedImageChoices(
      imageModels,
      selected?.selected_image_model,
    );
    const imageOk = Boolean(
      selected?.selected_image_model &&
      imageChoices.some(
        (item) => item.id === selected.selected_image_model && item.allowed,
      ),
    );
    const ready =
      hasCredential() &&
      routerReachable &&
      healthOk &&
      catalogOk &&
      chatOk &&
      imageOk;
    return {
      ready,
      has_credential: hasCredential(),
      router_trusted: routerReachable,
      health_ok: healthOk,
      store_corrupt: false,
      store_incompatible: false,
      chat_models: [...chatModels],
      image_models: imageChoices,
      can_submit: ready && !busy,
      busy,
      reason: !routerReachable
        ? "WORKBENCH_ROUTER"
        : !hasCredential()
          ? "WORKBENCH_NO_CREDENTIAL"
          : !healthOk || !catalogOk
            ? "WORKBENCH_CATALOG"
            : selected && (!chatOk || !imageOk)
              ? "WORKBENCH_MODEL_MISSING"
              : busy
                ? "WORKBENCH_BUSY"
                : null,
    };
  }

  function emitPhase(
    operationId: string,
    conversationId: string,
    messageId: string,
    phase: string,
  ) {
    const event = {
      operation_id: operationId,
      conversation_id: conversationId,
      message_id: messageId,
      phase,
    };
    for (const listener of phaseListeners) listener(event);
  }

  async function importImage(
    bytes: Uint8Array,
    source: string,
  ): Promise<WorkbenchAsset> {
    if (bytes.byteLength > MAX_IMAGE_BYTES) {
      throw liveError("WORKBENCH_IMAGE_TOO_LARGE");
    }
    const inspected = inspectImage(bytes);
    const id = await sha256Hex(bytes);
    const existing = blobs.get(id);
    if (existing) return publicAsset(existing);
    const mime =
      inspected.format === "jpg"
        ? "image/jpeg"
        : inspected.format === "webp"
          ? "image/webp"
          : "image/png";
    const stored: StoredAsset = {
      id,
      format: inspected.format,
      byte_len: bytes.byteLength,
      width: inspected.width,
      height: inspected.height,
      source,
      bytes,
      mime,
    };
    blobs.set(id, stored);
    snapshot.assets.push(publicAsset(stored));
    registerLiveWorkbenchAssetUrl(
      id,
      URL.createObjectURL(new Blob([ownedBytes(bytes)], { type: mime })),
    );
    return publicAsset(stored);
  }

  function publicAsset(asset: StoredAsset): WorkbenchAsset {
    return {
      id: asset.id,
      format: asset.format,
      byte_len: asset.byte_len,
      width: asset.width,
      height: asset.height,
      source: asset.source,
    };
  }

  function dataUriFor(id: string): string {
    const asset = blobs.get(id);
    if (!asset) throw liveError("WORKBENCH_NO_REFERENCE");
    return `data:${asset.mime};base64,${bytesToBase64(asset.bytes)}`;
  }

  function lastSuccess(conversationId: string): {
    assetId: string | null;
    prompt: string | null;
  } {
    for (let index = snapshot.messages.length - 1; index >= 0; index -= 1) {
      const message = snapshot.messages[index];
      if (message.conversation_id !== conversationId) continue;
      for (
        let jobIndex = message.job_ids.length - 1;
        jobIndex >= 0;
        jobIndex -= 1
      ) {
        const job = snapshot.jobs.find(
          (item) => item.id === message.job_ids[jobIndex],
        );
        if (job?.status === "succeeded") {
          return {
            assetId: job.output_asset_id,
            prompt: job.prompt,
          };
        }
      }
    }
    return { assetId: null, prompt: null };
  }

  async function streamChat(
    model: string,
    messages: Array<{ role: string; content: string }>,
    signal: AbortSignal,
    onDelta: (visible: string) => void,
    defaultSize: string,
  ): Promise<string> {
    const response = await routerFetch("/v1/chat/completions", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ model, messages, stream: true }),
      signal,
    });
    if (!response.ok || !response.body)
      throw liveError("WORKBENCH_CHAT_FAILED");
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let raw = "";
    let buffer = "";
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split(/\r?\n/);
      buffer = lines.pop() ?? "";
      for (const line of lines) {
        const piece = sseContent(line);
        if (!piece) continue;
        raw += piece;
        onDelta(extractImagePayload(raw, defaultSize).display);
      }
    }
    const tail = sseContent(buffer);
    if (tail) {
      raw += tail;
      onDelta(extractImagePayload(raw, defaultSize).display);
    }
    return raw;
  }

  async function generateImage(
    model: string,
    prompt: string,
    size: string,
    referenceId: string | null,
    signal: AbortSignal,
  ): Promise<WorkbenchAsset> {
    const body: Record<string, unknown> = {
      model,
      prompt,
      n: 1,
      size,
    };
    if (referenceId) {
      const uri = dataUriFor(referenceId);
      body.image = uri;
      body.images = [uri];
    }
    const response = await routerFetch(
      "/v1/images/generations?response_format=binary",
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
        signal,
      },
    );
    if (!response.ok) throw liveError("WORKBENCH_IMAGE_FAILED");
    const bytes = decodeGeneration(
      new Uint8Array(await response.arrayBuffer()),
      response.headers.get("content-type") ?? "",
    );
    return importImage(bytes, "generate");
  }

  async function send(
    conversationId: string,
    text: string,
    forceImage: boolean,
    referenceAssetId?: string | null,
  ): Promise<WorkbenchSnapshot> {
    if (busy) throw liveError("WORKBENCH_BUSY");
    const conversation = snapshot.conversations.find(
      (item) => item.id === conversationId,
    );
    if (!conversation) throw liveError("WORKBENCH_FAILED");
    let trimmed = text.trim();
    if (!trimmed) {
      if (!referenceAssetId) throw liveError("WORKBENCH_INPUT_EMPTY");
      trimmed = english ? EMPTY_REF_EN : EMPTY_REF_ZH;
    }
    const controller = new AbortController();
    abort = controller;
    const timeout = window.setTimeout(
      () => controller.abort("timeout"),
      180_000,
    );
    busy = true;
    const operationId = nextId("op");
    const userId = nextId("m");
    const assistantId = nextId("m");
    try {
      if (!conversation.message_ids.length) {
        conversation.title = titleFrom(trimmed);
      }
      conversation.message_ids.push(userId, assistantId);
      conversation.updated_at = now();
      snapshot.messages.push({
        id: userId,
        conversation_id: conversationId,
        role: "user",
        visible_text: trimmed,
        created_at: now(),
        reference_asset_id: referenceAssetId ?? null,
        phase: null,
        error_kind: null,
        job_ids: [],
      });
      const assistant: WorkbenchMessage = {
        id: assistantId,
        conversation_id: conversationId,
        role: "assistant",
        visible_text: "",
        created_at: now(),
        reference_asset_id: null,
        phase: "requesting",
        error_kind: null,
        job_ids: [],
      };
      snapshot.messages.push(assistant);
      emitPhase(operationId, conversationId, assistantId, "requesting");
      if (!catalogOk) {
        await refreshCatalogs(false);
      }
      const chat = selectChatModel(
        conversation.selected_chat_model,
        chatModels,
      );
      const image = selectImageModel(
        conversation.selected_image_model,
        imageModels,
      );
      if (!chat.ok || !image.ok || !chat.id || !image.id) {
        throw liveError("WORKBENCH_NOT_READY");
      }
      const history = historyTurns(conversationId);
      const last = lastSuccess(conversationId);
      const messages = rebuildChatMessages({
        history,
        lastImagePrompt: last.prompt,
        forceImage,
        explicitReference: Boolean(referenceAssetId),
        size: conversation.selected_size,
      });
      emitPhase(operationId, conversationId, assistantId, "thinking");
      const raw = await streamChat(
        chat.id,
        messages,
        controller.signal,
        (visible) => {
          assistant.visible_text = visible;
          const event = {
            operation_id: operationId,
            conversation_id: conversationId,
            message_id: assistantId,
            visible_text: visible,
          };
          for (const listener of chatListeners) listener(event);
        },
        conversation.selected_size,
      );
      const extracted = extractImagePayload(raw, conversation.selected_size);
      assistant.visible_text = extracted.display;
      const jobs = decideJobs(
        extracted.specs,
        trimmed,
        forceImage,
        referenceAssetId ?? null,
        last.assetId,
        conversation.selected_size,
        english,
      );
      assistant.phase = jobs.length ? "imaging" : null;
      if (jobs.length) {
        emitPhase(operationId, conversationId, assistantId, "imaging");
      }
      for (const decision of jobs) {
        const job: WorkbenchImageJob = {
          id: nextId("j"),
          message_id: assistantId,
          action: decision.action,
          prompt: decision.prompt,
          size: decision.size,
          status: "running",
          output_asset_id: null,
          error_kind: null,
        };
        assistant.job_ids.push(job.id);
        snapshot.jobs.push(job);
        try {
          if (decision.action === "edit" && !decision.reference_asset_id) {
            throw liveError("WORKBENCH_NO_REFERENCE");
          }
          const asset = await generateImage(
            image.id,
            decision.prompt,
            decision.size,
            decision.reference_asset_id,
            controller.signal,
          );
          job.status = "succeeded";
          job.output_asset_id = asset.id;
        } catch (cause) {
          const code =
            cause instanceof Error && "code" in cause
              ? String((cause as { code: string }).code)
              : controller.signal.aborted
                ? controller.signal.reason === "timeout"
                  ? "WORKBENCH_TIMEOUT"
                  : "WORKBENCH_CANCELLED"
                : "WORKBENCH_IMAGE_FAILED";
          job.status = code === "WORKBENCH_CANCELLED" ? "cancelled" : "failed";
          job.error_kind = code;
        }
        const statusEvent = {
          operation_id: operationId,
          conversation_id: conversationId,
          message_id: assistantId,
          job_id: job.id,
          status: job.status,
          output_asset_id: job.output_asset_id,
          error_kind: job.error_kind,
        };
        for (const listener of imageListeners) listener(statusEvent);
      }
      assistant.phase = null;
      const done: WorkbenchOperationDone = {
        operation_id: operationId,
        conversation_id: conversationId,
      };
      for (const listener of doneListeners) listener(done);
      return cloneSnapshot();
    } catch (cause) {
      const assistant = snapshot.messages.find(
        (item) => item.id === assistantId,
      );
      if (assistant) {
        assistant.phase = null;
        assistant.error_kind =
          cause instanceof Error && "code" in cause
            ? String((cause as { code: string }).code)
            : "WORKBENCH_CHAT_FAILED";
      }
      throw cause instanceof Error && "code" in cause
        ? cause
        : liveError("WORKBENCH_CHAT_FAILED");
    } finally {
      window.clearTimeout(timeout);
      busy = false;
      abort = null;
    }
  }

  return {
    getReadiness: async () => refreshCatalogs(false),
    refreshCatalogs: async () => refreshCatalogs(true),
    list: async () => cloneSnapshot(),
    create: async () => {
      await refreshCatalogs(false);
      const chat = selectChatModel(null, chatModels);
      const image = selectImageModel(null, imageModels);
      const conversation: WorkbenchConversation = {
        id: nextId("c"),
        title: "新对话",
        created_at: now(),
        updated_at: now(),
        selected_chat_model: chat.ok ? chat.id : null,
        selected_image_model: image.ok ? image.id : null,
        selected_size: "1024x1024",
        message_ids: [],
      };
      snapshot.conversations.unshift(conversation);
      snapshot.selected_conversation_id = conversation.id;
      return structuredClone(conversation);
    },
    select: async (conversationId: string) => {
      if (!snapshot.conversations.some((item) => item.id === conversationId)) {
        throw liveError("WORKBENCH_FAILED");
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
      const keep = new Set(
        snapshot.jobs
          .map((job) => job.output_asset_id)
          .concat(
            snapshot.messages.map((item) => item.reference_asset_id ?? ""),
          )
          .filter(Boolean),
      );
      for (const [id] of blobs) {
        if (keep.has(id)) continue;
        blobs.delete(id);
        revokeLiveWorkbenchAssetUrl(id);
        snapshot.assets = snapshot.assets.filter((item) => item.id !== id);
      }
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
      next: { chatModel?: string; imageModel?: string; size?: string },
    ) => {
      const conversation = snapshot.conversations.find(
        (item) => item.id === conversationId,
      );
      if (!conversation) throw liveError("WORKBENCH_FAILED");
      if (next.chatModel !== undefined) {
        conversation.selected_chat_model = next.chatModel;
      }
      if (next.imageModel !== undefined) {
        conversation.selected_image_model = next.imageModel;
      }
      if (next.size !== undefined) conversation.selected_size = next.size;
    },
    pickReference: async () => {
      const file = await pickImageFile();
      if (!file) throw liveError("WORKBENCH_CANCELLED");
      return importImage(new Uint8Array(await file.arrayBuffer()), "upload");
    },
    importBytes: async (bytes: Uint8Array) => importImage(bytes, "import"),
    quoteAsset: async (assetId: string) => {
      const asset = snapshot.assets.find((item) => item.id === assetId);
      if (!asset) throw liveError("WORKBENCH_IMAGE_INVALID");
      return structuredClone(asset);
    },
    send,
    regenerate: async (
      conversationId: string,
      jobId: string,
      referenceAssetId?: string | null,
    ) => {
      if (busy) throw liveError("WORKBENCH_BUSY");
      await refreshCatalogs(false);
      const job = snapshot.jobs.find((item) => item.id === jobId);
      const conversation = snapshot.conversations.find(
        (item) => item.id === conversationId,
      );
      if (!job || !conversation) throw liveError("WORKBENCH_FAILED");
      const image = selectImageModel(
        conversation.selected_image_model,
        imageModels,
      );
      if (!image.ok || !image.id) throw liveError("WORKBENCH_NOT_READY");
      const controller = new AbortController();
      abort = controller;
      const timeout = window.setTimeout(
        () => controller.abort("timeout"),
        180_000,
      );
      busy = true;
      try {
        job.status = "running";
        const asset = await generateImage(
          image.id,
          job.prompt,
          job.size,
          referenceAssetId ?? lastSuccess(conversationId).assetId,
          controller.signal,
        );
        job.status = "succeeded";
        job.output_asset_id = asset.id;
        job.error_kind = null;
        return {
          messages: structuredClone(snapshot.messages),
          jobs: structuredClone(snapshot.jobs),
          assets: structuredClone(snapshot.assets),
        };
      } catch (cause) {
        job.status = "failed";
        job.error_kind =
          cause instanceof Error && "code" in cause
            ? String((cause as { code: string }).code)
            : "WORKBENCH_IMAGE_FAILED";
        throw cause instanceof Error && "code" in cause
          ? cause
          : liveError("WORKBENCH_IMAGE_FAILED");
      } finally {
        window.clearTimeout(timeout);
        busy = false;
        abort = null;
      }
    },
    cancel: async () => {
      abort?.abort("cancel");
    },
    saveAsset: async (assetId: string) => {
      const asset = blobs.get(assetId);
      if (!asset) throw liveError("WORKBENCH_IMAGE_INVALID");
      const url = URL.createObjectURL(
        new Blob([ownedBytes(asset.bytes)], { type: asset.mime }),
      );
      const link = document.createElement("a");
      link.href = url;
      link.download = `${assetId}.${asset.format}`;
      link.click();
      URL.revokeObjectURL(url);
    },
    rebuild: async () => {
      snapshot.conversations = [];
      snapshot.messages = [];
      snapshot.jobs = [];
      snapshot.assets = [];
      snapshot.selected_conversation_id = null;
      blobs.clear();
      revokeAllLiveWorkbenchAssets();
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
    setReadiness() {
      return currentReadiness();
    },
    snapshot,
  };

  function historyTurns(conversationId: string) {
    return snapshot.messages
      .filter((item) => item.conversation_id === conversationId)
      .filter((item) => item.role === "user" || item.role === "assistant")
      .slice(0, -1)
      .map((item) => ({
        role: item.role,
        visible_text: item.visible_text,
        generated_prompts: snapshot.jobs
          .filter(
            (job) =>
              item.job_ids.includes(job.id) && job.status === "succeeded",
          )
          .map((job) => job.prompt),
        reference_hint: item.reference_asset_id ? "" : null,
      }));
  }
}

function inspectImage(bytes: Uint8Array): {
  format: "png" | "jpg" | "webp";
  width: number;
  height: number;
} {
  if (
    bytes.length >= 24 &&
    bytes[0] === 0x89 &&
    bytes[1] === 0x50 &&
    bytes[2] === 0x4e &&
    bytes[3] === 0x47
  ) {
    const width = view32(bytes, 16);
    const height = view32(bytes, 20);
    return { format: "png", width, height };
  }
  if (bytes.length >= 3 && bytes[0] === 0xff && bytes[1] === 0xd8) {
    return { format: "jpg", width: 1, height: 1 };
  }
  if (
    bytes.length >= 12 &&
    bytes[0] === 0x52 &&
    bytes[1] === 0x49 &&
    bytes[2] === 0x46 &&
    bytes[3] === 0x46
  ) {
    return { format: "webp", width: 1, height: 1 };
  }
  throw liveError("WORKBENCH_IMAGE_INVALID");
}

function view32(bytes: Uint8Array, offset: number): number {
  return (
    (bytes[offset] << 24) |
    (bytes[offset + 1] << 16) |
    (bytes[offset + 2] << 8) |
    bytes[offset + 3]
  );
}

function decodeGeneration(bytes: Uint8Array, contentType: string): Uint8Array {
  if (looksLikeImage(bytes)) return bytes;
  if (!contentType.includes("json") && bytes[0] !== 0x7b) {
    throw liveError("WORKBENCH_IMAGE_FAILED");
  }
  const value = JSON.parse(new TextDecoder().decode(bytes)) as {
    data?: Array<{ b64_json?: string; url?: string }>;
  };
  const items = value.data;
  if (!items || items.length !== 1) throw liveError("WORKBENCH_IMAGE_FAILED");
  if (items[0].b64_json) {
    const binary = atob(items[0].b64_json);
    const out = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
      out[index] = binary.charCodeAt(index);
    }
    return out;
  }
  throw liveError("WORKBENCH_IMAGE_FAILED");
}

function looksLikeImage(bytes: Uint8Array): boolean {
  return (
    (bytes.length >= 8 &&
      bytes[0] === 0x89 &&
      bytes[1] === 0x50 &&
      bytes[2] === 0x4e &&
      bytes[3] === 0x47) ||
    (bytes.length >= 3 && bytes[0] === 0xff && bytes[1] === 0xd8) ||
    (bytes.length >= 12 &&
      bytes[0] === 0x52 &&
      bytes[1] === 0x49 &&
      bytes[2] === 0x46 &&
      bytes[3] === 0x46)
  );
}

function ownedBytes(bytes: Uint8Array): Uint8Array<ArrayBuffer> {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy;
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", ownedBytes(bytes));
  return [...new Uint8Array(digest)]
    .map((item) => item.toString(16).padStart(2, "0"))
    .join("");
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let index = 0; index < bytes.length; index += chunk) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunk));
  }
  return btoa(binary);
}

function pickImageFile(): Promise<File | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/png,image/jpeg,image/webp";
    input.onchange = () => resolve(input.files?.[0] ?? null);
    input.click();
  });
}
