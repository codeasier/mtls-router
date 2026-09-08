import { useCallback, useEffect, useRef, useState } from "react";

import { ConfirmDialog } from "./ConfirmDialog";
import { useI18n, type Translator } from "./i18n";
import type {
  DesktopApi,
  WorkbenchAsset,
  WorkbenchConversation,
  WorkbenchImageJob,
  WorkbenchMessage,
  WorkbenchReadiness,
  WorkbenchSnapshot,
} from "./ipc";
import type { TranslationKey } from "./locales/zh-CN";
import { shouldUseLiveWorkbench } from "./dev/resolveDesktopApi";
import starterArchImage from "./assets/starters/iso-microservices.jpg";
import starterCicdImage from "./assets/starters/cicd-pipeline.jpg";
import starterAiPairImage from "./assets/starters/ai-pair-programming.jpg";
import {
  commandErrorCode,
  looksLikeImplicitEdit,
  MAX_WORKBENCH_IMPORT_BYTES,
  workbenchAssetUri,
} from "./workbench";

const SIZES = ["1024x1024", "1792x1024", "1024x1792"] as const;
const STARTERS = [
  {
    prompt: "images.chip.arch",
    label: "images.chip.arch.label",
    desc: "images.chip.arch.desc",
    image: starterArchImage,
  },
  {
    prompt: "images.chip.cicd",
    label: "images.chip.cicd.label",
    desc: "images.chip.cicd.desc",
    image: starterCicdImage,
  },
  {
    prompt: "images.chip.aiPair",
    label: "images.chip.aiPair.label",
    desc: "images.chip.aiPair.desc",
    image: starterAiPairImage,
  },
] as const;
const BLOCKING_REASONS = new Set([
  "WORKBENCH_NO_CREDENTIAL",
  "WORKBENCH_ROUTER",
  "WORKBENCH_NOT_READY",
  "WORKBENCH_CATALOG",
  "WORKBENCH_MODEL_MISSING",
  "WORKBENCH_STORE_CORRUPT",
  "WORKBENCH_STORE_INCOMPATIBLE",
]);

type Draft = { text: string; reference: WorkbenchAsset | null };

const emptyDraft = (): Draft => ({ text: "", reference: null });

function WorkbenchIcon({
  type,
}: {
  type:
    | "chat"
    | "image"
    | "ratio"
    | "upload"
    | "spark"
    | "send"
    | "stop"
    | "history"
    | "quote"
    | "download"
    | "refresh";
}) {
  const paths = {
    chat: (
      <>
        <path d="M4 5.5h16v10H8l-4 3v-13Z" />
        <path d="M8 9h8M8 12h5" />
      </>
    ),
    image: (
      <>
        <rect x="3.5" y="4" width="17" height="16" rx="2" />
        <circle cx="8.5" cy="9" r="1.2" />
        <path d="m5 17 4.5-4 3 2.5 2.5-2 4 3.5" />
      </>
    ),
    ratio: (
      <>
        <rect x="4" y="4" width="16" height="16" rx="2" />
        <path d="M8 4v4H4M16 20v-4h4" />
      </>
    ),
    upload: (
      <>
        <path d="M12 16V4M7.5 8.5 12 4l4.5 4.5" />
        <path d="M5 14v5h14v-5" />
      </>
    ),
    spark: (
      <>
        <path d="m12 3 1.5 6.5L20 11l-6.5 1.5L12 19l-1.5-6.5L4 11l6.5-1.5L12 3Z" />
        <path d="m19 3 .5 2.5L22 6l-2.5.5L19 9l-.5-2.5L16 6l2.5-.5L19 3Z" />
      </>
    ),
    send: <path d="M12 20V4m-7 7 7-7 7 7" />,
    stop: <rect x="6" y="6" width="12" height="12" rx="1" />,
    history: (
      <>
        <path d="M3 11a9 9 0 1 1 2.6 7M3 4v7h7" />
        <path d="M12 7v5l3 2" />
      </>
    ),
    quote: (
      <>
        <path d="M9 15 4 10l5-5" />
        <path d="M4 10h11a4 4 0 0 1 4 4v4" />
      </>
    ),
    download: (
      <>
        <path d="M12 3v11M7.5 9.5 12 14l4.5-4.5" />
        <path d="M4 17v3h16v-3" />
      </>
    ),
    refresh: (
      <>
        <path d="M3 12a9 9 0 0 1 15.6-6.4L21 8" />
        <path d="M21 3v5h-5" />
        <path d="M21 12a9 9 0 0 1-15.6 6.4L3 16" />
        <path d="M3 21v-5h5" />
      </>
    ),
  };
  return (
    <svg className="images-icon" viewBox="0 0 24 24" aria-hidden="true">
      {paths[type]}
    </svg>
  );
}

function emptySnapshot(): WorkbenchSnapshot {
  return {
    selected_conversation_id: null,
    conversations: [],
    messages: [],
    jobs: [],
    assets: [],
  };
}

function readinessKey(reason: string | null): TranslationKey {
  switch (reason) {
    case "WORKBENCH_NO_CREDENTIAL":
      return "images.ready.noCredential";
    case "WORKBENCH_CHECKING":
      return "images.ready.checking";
    case "WORKBENCH_ROUTER":
    case "WORKBENCH_NOT_READY":
      return "images.ready.router";
    case "WORKBENCH_CATALOG":
      return "images.ready.catalog";
    case "WORKBENCH_MODEL_MISSING":
      return "images.ready.modelMissing";
    case "WORKBENCH_STORE_CORRUPT":
      return "images.ready.storeCorrupt";
    case "WORKBENCH_STORE_INCOMPATIBLE":
      return "images.ready.storeIncompatible";
    case "WORKBENCH_BUSY":
      return "images.ready.busy";
    default:
      return reason ? "images.ready.unknown" : "images.ready.ok";
  }
}

function errorKey(code: string): TranslationKey {
  switch (code) {
    case "WORKBENCH_BUSY":
      return "images.error.busy";
    case "WORKBENCH_INPUT_EMPTY":
      return "images.error.empty";
    case "WORKBENCH_INPUT_TOO_LARGE":
      return "images.error.tooLarge";
    case "WORKBENCH_IMAGE_INVALID":
      return "images.error.imageInvalid";
    case "WORKBENCH_IMAGE_TOO_LARGE":
      return "images.error.imageTooLarge";
    case "WORKBENCH_NO_REFERENCE":
      return "images.error.noReference";
    case "WORKBENCH_CANCELLED":
      return "images.error.cancelled";
    case "WORKBENCH_INTERRUPTED":
      return "images.error.interrupted";
    case "WORKBENCH_CHAT_FAILED":
      return "images.error.chatFailed";
    case "WORKBENCH_IMAGE_FAILED":
      return "images.error.imageFailed";
    case "WORKBENCH_TIMEOUT":
      return "images.error.timeout";
    case "WORKBENCH_IDENTITY":
      return "images.error.identity";
    case "WORKBENCH_REDIAL":
      return "images.error.redial";
    case "WORKBENCH_NOT_READY":
      return "images.error.notReady";
    case "WORKBENCH_STORE_CORRUPT":
      return "images.error.storeCorrupt";
    case "WORKBENCH_STORE_INCOMPATIBLE":
      return "images.error.storeIncompatible";
    default:
      return "images.error.failed";
  }
}

function conversationTitle(
  conversation: WorkbenchConversation,
  untitled: string,
): string {
  if (!conversation.message_ids.length || conversation.title === "新对话") {
    return untitled;
  }
  return conversation.title;
}

function shortWhen(iso: string, language: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleString(language === "en" ? "en" : "zh-CN", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function autosizeComposer(node: HTMLTextAreaElement | null) {
  if (!node) return;
  node.style.height = "auto";
  node.style.height = `${Math.min(node.scrollHeight, 160)}px`;
}

function ModelDials({
  selected,
  readiness,
  onChange,
  t,
}: {
  selected: WorkbenchConversation | undefined;
  readiness: WorkbenchReadiness | null;
  onChange: (patch: {
    chatModel?: string;
    imageModel?: string;
    size?: string;
  }) => void;
  t: Translator;
}) {
  const currentChatLabel =
    selected?.selected_chat_model ??
    (readiness?.chat_models.length === 0
      ? t("images.ready.modelMissing")
      : t("images.dial.chat"));

  const currentImageChoice = readiness?.image_models.find(
    (choice) => choice.id === selected?.selected_image_model,
  );
  const currentImageLabel =
    currentImageChoice?.display_name ??
    selected?.selected_image_model ??
    (readiness?.image_models.length === 0
      ? t("images.ready.modelMissing")
      : t("images.dial.image"));

  const currentSizeLabel =
    selected?.selected_size === "1792x1024"
      ? t("images.size.landscape")
      : selected?.selected_size === "1024x1792"
        ? t("images.size.portrait")
        : t("images.size.square");

  return (
    <div className="images-dials">
      <label
        className="images-dial"
        title={`${t("images.chatModel")}: ${currentChatLabel}`}
      >
        <WorkbenchIcon type="chat" />
        <span className="images-dial__value">{currentChatLabel}</span>
        <select
          aria-label={t("images.chatModel")}
          value={selected?.selected_chat_model ?? ""}
          onChange={(event) =>
            onChange({
              chatModel: event.target.value || undefined,
            })
          }
        >
          {!selected?.selected_chat_model && (
            <option value="">
              {selected
                ? t("images.ready.modelMissing")
                : t("images.dial.chat")}
            </option>
          )}
          {selected?.selected_chat_model &&
            !readiness?.chat_models.includes(selected.selected_chat_model) && (
              <option value={selected.selected_chat_model}>
                {selected.selected_chat_model}
              </option>
            )}
          {readiness?.chat_models.map((id) => (
            <option key={id} value={id}>
              {id}
            </option>
          ))}
        </select>
      </label>
      <label
        className="images-dial"
        title={`${t("images.imageModel")}: ${currentImageLabel}`}
      >
        <WorkbenchIcon type="image" />
        <span className="images-dial__value">{currentImageLabel}</span>
        <select
          aria-label={t("images.imageModel")}
          value={selected?.selected_image_model ?? ""}
          onChange={(event) =>
            onChange({
              imageModel: event.target.value || undefined,
            })
          }
        >
          {!selected?.selected_image_model && (
            <option value="">
              {selected
                ? t("images.ready.modelMissing")
                : t("images.dial.image")}
            </option>
          )}
          {selected?.selected_image_model &&
            !readiness?.image_models.some(
              (item) => item.id === selected.selected_image_model,
            ) && (
              <option value={selected.selected_image_model}>
                {selected.selected_image_model}
              </option>
            )}
          {readiness?.image_models.map((choice) => (
            <option key={choice.id} value={choice.id}>
              {choice.display_name}
            </option>
          ))}
        </select>
      </label>
      <label
        className="images-dial"
        title={`${t("images.size")}: ${currentSizeLabel}`}
      >
        <WorkbenchIcon type="ratio" />
        <span className="images-dial__value">{currentSizeLabel}</span>
        <select
          aria-label={t("images.size")}
          value={selected?.selected_size ?? "1024x1024"}
          onChange={(event) => onChange({ size: event.target.value })}
        >
          {SIZES.map((size) => (
            <option key={size} value={size}>
              {size === "1024x1024"
                ? t("images.size.square")
                : size === "1792x1024"
                  ? t("images.size.landscape")
                  : t("images.size.portrait")}
            </option>
          ))}
        </select>
      </label>
    </div>
  );
}

export function ImagesPage({ api }: { api: DesktopApi }) {
  const { t, language } = useI18n();
  const [readiness, setReadiness] = useState<WorkbenchReadiness | null>(null);
  const [snapshot, setSnapshot] = useState<WorkbenchSnapshot>(emptySnapshot);
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [localBusy, setLocalBusy] = useState(false);
  const [error, setError] = useState<TranslationKey | "">("");
  const [notice, setNotice] = useState<TranslationKey | "">("");
  const [dragging, setDragging] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const [pendingText, setPendingText] = useState("");
  const [railCollapsed, setRailCollapsed] = useState(false);
  const initialLoadRef = useRef<{
    api: DesktopApi;
    promise: Promise<[WorkbenchReadiness, WorkbenchSnapshot]>;
  } | null>(null);
  const [initializing, setInitializing] = useState(true);
  const operationRef = useRef<string | null>(null);
  const acceptingRef = useRef(false);
  const wellRef = useRef<HTMLElement | null>(null);
  const composerRef = useRef<HTMLTextAreaElement | null>(null);
  const selectedId = snapshot.selected_conversation_id;

  const draft = selectedId
    ? (drafts[selectedId] ?? emptyDraft())
    : emptyDraft();

  const selected = snapshot.conversations.find(
    (item) => item.id === selectedId,
  );
  const messages = snapshot.messages.filter(
    (item) => item.conversation_id === selectedId,
  );
  let lastImage: WorkbenchAsset | undefined;
  for (let index = snapshot.messages.length - 1; index >= 0; index -= 1) {
    const message = snapshot.messages[index];
    if (message.conversation_id !== selectedId) continue;
    for (
      let jobIndex = message.job_ids.length - 1;
      jobIndex >= 0;
      jobIndex -= 1
    ) {
      const job = snapshot.jobs.find(
        (item) => item.id === message.job_ids[jobIndex],
      );
      if (job?.status === "succeeded" && job.output_asset_id) {
        lastImage = snapshot.assets.find(
          (item) => item.id === job.output_asset_id,
        );
        break;
      }
    }
    if (lastImage) break;
  }

  const imageChoice = readiness?.image_models.find(
    (item) => item.id === selected?.selected_image_model,
  );
  const willEdit =
    Boolean(draft.reference) ||
    Boolean(
      !draft.reference &&
      lastImage &&
      looksLikeImplicitEdit(draft.text, language),
    );
  const showEditWarning = Boolean(
    willEdit && imageChoice && !imageChoice.verified_edit,
  );
  const chatOk = Boolean(
    selected?.selected_chat_model &&
    readiness?.chat_models.includes(selected.selected_chat_model),
  );
  const imageOk = Boolean(
    selected?.selected_image_model &&
    readiness?.image_models.some(
      (item) => item.id === selected.selected_image_model && item.allowed,
    ),
  );
  const busy = Boolean(readiness?.busy || localBusy);
  const canSubmit = Boolean(
    readiness &&
    readiness.has_credential &&
    readiness.router_trusted &&
    readiness.health_ok &&
    !readiness.store_corrupt &&
    !readiness.store_incompatible &&
    chatOk &&
    imageOk &&
    selected &&
    !busy,
  );

  function statusReason(): string | null {
    if (!readiness) return "WORKBENCH_CHECKING";
    if (readiness.store_incompatible) return "WORKBENCH_STORE_INCOMPATIBLE";
    if (readiness.store_corrupt) return "WORKBENCH_STORE_CORRUPT";
    if (!readiness.has_credential) return "WORKBENCH_NO_CREDENTIAL";
    if (!readiness.router_trusted || !readiness.health_ok)
      return "WORKBENCH_ROUTER";
    if (
      !readiness.chat_models.length ||
      !readiness.image_models.some((item) => item.allowed)
    ) {
      return "WORKBENCH_CATALOG";
    }
    if (selected && (!chatOk || !imageOk)) return "WORKBENCH_MODEL_MISSING";
    if (busy) return "WORKBENCH_BUSY";
    return null;
  }

  const applySnapshot = useCallback((next: WorkbenchSnapshot) => {
    setSnapshot(next);
  }, []);

  useEffect(() => {
    let cancelled = false;
    // Share initialization across StrictMode effect replay, including creation.
    if (initialLoadRef.current?.api !== api) {
      initialLoadRef.current = {
        api,
        promise: Promise.all([
          api.getWorkbenchReadiness(),
          api.listWorkbench(),
        ]).then(async ([nextReadiness, nextSnapshot]) => {
          if (
            !nextSnapshot.selected_conversation_id &&
            !nextReadiness.store_corrupt &&
            !nextReadiness.store_incompatible
          ) {
            await api.createWorkbenchConversation();
            return Promise.all([
              api.getWorkbenchReadiness(),
              api.listWorkbench(),
            ]);
          }
          return [nextReadiness, nextSnapshot];
        }),
      };
    }
    void initialLoadRef.current.promise
      .then(([nextReadiness, nextSnapshot]) => {
        if (cancelled) return;
        setReadiness(nextReadiness);
        applySnapshot(nextSnapshot);
      })
      .catch(() => {
        if (!cancelled) setError("images.error.failed");
      })
      .finally(() => {
        if (!cancelled) setInitializing(false);
      });
    return () => {
      cancelled = true;
    };
  }, [api, applySnapshot]);

  useEffect(() => {
    let disposed = false;
    const stops: Array<() => void> = [];
    function keep(operationId: string | null) {
      if (!operationId) return false;
      if (operationRef.current == null) {
        if (!acceptingRef.current) return false;
        operationRef.current = operationId;
        return true;
      }
      return operationId === operationRef.current;
    }
    void Promise.all([
      api.subscribeWorkbenchChatDelta((event) => {
        if (disposed || !keep(event.operation_id)) return;
        setSnapshot((current) => {
          if (!current.messages.some((item) => item.id === event.message_id)) {
            void api
              .listWorkbench()
              .then(applySnapshot)
              .catch(() => undefined);
            return current;
          }
          return {
            ...current,
            messages: current.messages.map((message) =>
              message.id === event.message_id
                ? { ...message, visible_text: event.visible_text }
                : message,
            ),
          };
        });
      }),
      api.subscribeWorkbenchPhase((event) => {
        if (disposed || !keep(event.operation_id)) return;
        operationRef.current = event.operation_id;
        setSnapshot((current) => {
          if (!current.messages.some((item) => item.id === event.message_id)) {
            void api
              .listWorkbench()
              .then(applySnapshot)
              .catch(() => undefined);
            return current;
          }
          return {
            ...current,
            messages: current.messages.map((message) =>
              message.id === event.message_id
                ? { ...message, phase: event.phase }
                : message,
            ),
          };
        });
      }),
      api.subscribeWorkbenchImageStatus((event) => {
        if (disposed || !keep(event.operation_id)) return;
        setSnapshot((current) => {
          if (!current.jobs.some((item) => item.id === event.job_id)) {
            void api
              .listWorkbench()
              .then(applySnapshot)
              .catch(() => undefined);
            return current;
          }
          return {
            ...current,
            jobs: current.jobs.map((job) =>
              job.id === event.job_id
                ? {
                    ...job,
                    status: event.status,
                    output_asset_id:
                      event.output_asset_id ?? job.output_asset_id,
                    error_kind: event.error_kind ?? job.error_kind,
                  }
                : job,
            ),
          };
        });
      }),
      api.subscribeWorkbenchOperationDone((event) => {
        if (disposed || !keep(event.operation_id)) return;
        acceptingRef.current = false;
        operationRef.current = null;
        setLocalBusy(false);
        void api
          .getWorkbenchReadiness()
          .then(setReadiness)
          .catch(() => undefined);
        void api
          .listWorkbench()
          .then(applySnapshot)
          .catch(() => undefined);
      }),
    ]).then((unsubscribes) => {
      if (disposed) {
        unsubscribes.forEach((stop) => stop());
        return;
      }
      stops.push(...unsubscribes);
    });
    return () => {
      disposed = true;
      stops.forEach((stop) => stop());
    };
  }, [api, applySnapshot]);

  function updateDraft(patch: Partial<Draft>, conversationId = selectedId) {
    if (!conversationId) return;
    setDrafts((current) => ({
      ...current,
      [conversationId]: {
        ...(current[conversationId] ?? emptyDraft()),
        ...patch,
      },
    }));
  }

  useEffect(() => {
    const node = wellRef.current;
    if (!node) return;
    node.scrollTop = node.scrollHeight;
  }, [messages, snapshot.jobs, snapshot.assets]);

  useEffect(() => {
    autosizeComposer(composerRef.current);
  }, [draft.text]);

  async function selectConversation(id: string) {
    const selectedConversation = await api.selectWorkbenchConversation(id);
    applySnapshot({
      ...snapshot,
      selected_conversation_id: selectedConversation,
    });
    const next = await api.listWorkbench();
    applySnapshot(next);
  }

  async function createConversation() {
    const conversation = await api.createWorkbenchConversation();
    const next = await api.listWorkbench();
    applySnapshot(next);
    setDrafts((current) => ({ ...current, [conversation.id]: emptyDraft() }));
    setReadiness(await api.getWorkbenchReadiness());
    return conversation;
  }

  async function fillStarter(prompt: string) {
    if (selectedId) {
      updateDraft({ text: prompt });
      composerRef.current?.focus();
      window.requestAnimationFrame(() => {
        autosizeComposer(composerRef.current);
      });
      return;
    }
    const conversation = await createConversation();
    updateDraft({ text: prompt }, conversation.id);
    composerRef.current?.focus();
    window.requestAnimationFrame(() => {
      autosizeComposer(composerRef.current);
    });
  }

  async function confirmDelete() {
    if (!pendingDelete) return;
    const id = pendingDelete;
    setPendingDelete(null);
    const result = await api.deleteWorkbenchConversation(id);
    setDrafts((current) => {
      const next = { ...current };
      delete next[id];
      return next;
    });
    applySnapshot({
      ...snapshot,
      selected_conversation_id: result.selected_conversation_id,
      conversations: result.conversations,
      messages: snapshot.messages.filter((item) => item.conversation_id !== id),
      jobs: snapshot.jobs.filter((job) => {
        const message = snapshot.messages.find(
          (item) => item.id === job.message_id,
        );
        return message?.conversation_id !== id;
      }),
    });
    const next = await api.listWorkbench();
    applySnapshot(next);
  }

  async function changeOptions(patch: {
    chatModel?: string;
    imageModel?: string;
    size?: string;
  }) {
    if (!selectedId) return;
    await api.setWorkbenchOptions(selectedId, patch);
    const next = await api.listWorkbench();
    applySnapshot(next);
    const refreshed = await api.getWorkbenchReadiness();
    setReadiness(refreshed);
  }

  async function importBytes(bytes: Uint8Array) {
    if (bytes.byteLength > MAX_WORKBENCH_IMPORT_BYTES) {
      setError("images.error.imageTooLarge");
      return;
    }
    try {
      const asset = await api.importWorkbenchBytes(bytes);
      updateDraft({ reference: asset });
      setError("");
    } catch (cause) {
      setError(errorKey(commandErrorCode(cause)));
    }
  }

  async function onFiles(files: FileList | File[]) {
    const file = Array.from(files).find((item) =>
      item.type.startsWith("image/"),
    );
    if (!file) return;
    if (file.size > MAX_WORKBENCH_IMPORT_BYTES) {
      setError("images.error.imageTooLarge");
      return;
    }
    const bytes = new Uint8Array(await file.arrayBuffer());
    await importBytes(bytes);
  }

  async function submit(forceImage: boolean) {
    if (!selectedId || !canSubmit) return;
    if (!draft.text.trim() && !draft.reference) {
      setError("images.error.empty");
      return;
    }
    const saved = draft;
    setError("");
    acceptingRef.current = true;
    operationRef.current = null;
    setLocalBusy(true);
    setPendingText(saved.text.trim());
    setDrafts((current) => ({ ...current, [selectedId]: emptyDraft() }));
    try {
      const next = await api.sendWorkbench(
        selectedId,
        saved.text,
        forceImage,
        saved.reference?.id ?? null,
      );
      applySnapshot(next);
      const refreshed = await api.getWorkbenchReadiness();
      setReadiness(refreshed);
    } catch (cause) {
      const code = commandErrorCode(cause);
      setError(errorKey(code));
      if (code === "WORKBENCH_BUSY") {
        setDrafts((current) => ({ ...current, [selectedId]: saved }));
      } else {
        try {
          const listed = await api.listWorkbench();
          applySnapshot(listed);
          const appeared = listed.messages.some(
            (item) =>
              item.conversation_id === selectedId &&
              item.role === "user" &&
              item.visible_text === saved.text.trim(),
          );
          if (!appeared) {
            setDrafts((current) => ({ ...current, [selectedId]: saved }));
          }
        } catch {
          setDrafts((current) => ({ ...current, [selectedId]: saved }));
        }
      }
    } finally {
      acceptingRef.current = false;
      setLocalBusy(false);
      setPendingText("");
    }
  }

  async function regenerate(jobId: string) {
    if (!selectedId || !canSubmit) return;
    setError("");
    acceptingRef.current = true;
    operationRef.current = null;
    setLocalBusy(true);
    try {
      const next = await api.regenerateWorkbench(
        selectedId,
        jobId,
        draft.reference?.id ?? null,
      );
      applySnapshot({
        ...snapshot,
        messages: next.messages,
        jobs: next.jobs,
        assets: next.assets,
      });
    } catch (cause) {
      setError(errorKey(commandErrorCode(cause)));
    } finally {
      acceptingRef.current = false;
      setLocalBusy(false);
    }
  }

  const reason = statusReason();
  const blocking = Boolean(reason && BLOCKING_REASONS.has(reason));
  const live = shouldUseLiveWorkbench({
    DEV: import.meta.env.DEV,
    PROD: import.meta.env.PROD,
    VITE_MOCK: import.meta.env.VITE_MOCK,
    VITE_WORKBENCH_LIVE: import.meta.env.VITE_WORKBENCH_LIVE,
  });
  const landing = messages.length === 0 && !busy && !pendingText;

  return (
    <section
      className={`${dragging ? "images-panel is-dropping" : "images-panel"}${railCollapsed ? " is-rail-collapsed" : ""}`}
      aria-label={t("section.images.title")}
      onDragOver={(event) => {
        event.preventDefault();
        setDragging(true);
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={(event) => {
        event.preventDefault();
        setDragging(false);
        void onFiles(event.dataTransfer.files);
      }}
    >
      <aside className="images-rail" id="images-history" hidden={railCollapsed}>
        <div className="images-rail__head">
          <div className="images-brand">{t("images.brand")}</div>
        </div>
        <button
          type="button"
          className="images-new"
          aria-label={t("images.newConversation")}
          disabled={initializing}
          onClick={() => void createConversation()}
        >
          {t("images.newConversationShort")}
        </button>
        {snapshot.conversations.length === 0 ? (
          <p className="images-empty-rail">{t("images.noConversations")}</p>
        ) : (
          <ul className="images-conversations">
            {snapshot.conversations.map((conversation) => (
              <li
                key={conversation.id}
                className={
                  conversation.id === selectedId
                    ? "images-roll is-active"
                    : "images-roll"
                }
              >
                <button
                  type="button"
                  className="images-conversation"
                  aria-label={conversationTitle(
                    conversation,
                    t("images.untitled"),
                  )}
                  onClick={() => void selectConversation(conversation.id)}
                >
                  <span className="images-roll__name">
                    {conversationTitle(conversation, t("images.untitled"))}
                  </span>
                  <span className="images-roll__when">
                    {shortWhen(conversation.updated_at, language)}
                  </span>
                </button>
                <button
                  type="button"
                  className="images-roll__kill"
                  aria-label={t("images.deleteConversation")}
                  onClick={() => setPendingDelete(conversation.id)}
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
        )}
        <div className="images-rail-foot">
          <p
            className={
              blocking
                ? "images-conn is-bad"
                : reason === "WORKBENCH_CHECKING"
                  ? "images-conn"
                  : reason === "WORKBENCH_BUSY"
                    ? "images-conn"
                    : "images-conn is-ok"
            }
          >
            {t(
              blocking
                ? "images.conn.blocked"
                : reason === "WORKBENCH_CHECKING"
                  ? "images.conn.checking"
                  : reason === "WORKBENCH_BUSY"
                    ? "images.conn.busy"
                    : "images.conn.ok",
            )}
          </p>
          <button
            type="button"
            className="images-ghost"
            onClick={() => {
              void api
                .refreshWorkbenchCatalogs()
                .then((next) => setReadiness(next))
                .catch(() => setError("images.error.failed"));
            }}
          >
            {t("images.refreshReady")}
          </button>
          {live && (
            <p className="images-live-note" title={t("images.live.banner")}>
              {t("images.live.banner")}
            </p>
          )}
        </div>
      </aside>

      <div className={landing ? "images-stage is-landing" : "images-stage"}>
        <header className="images-mast">
          <h2>
            {selected
              ? conversationTitle(selected, t("images.untitled"))
              : t("images.untitled")}
          </h2>
          <button
            type="button"
            className="images-rail-toggle"
            aria-label={t(
              railCollapsed
                ? "images.history.expand"
                : "images.history.collapse",
            )}
            title={t(
              railCollapsed
                ? "images.history.expand"
                : "images.history.collapse",
            )}
            aria-expanded={!railCollapsed}
            aria-controls="images-history"
            onClick={() => setRailCollapsed((current) => !current)}
          >
            <WorkbenchIcon type="history" />
          </button>
        </header>

        {blocking && (
          <div className="images-alert" role="status">
            <span>{t(readinessKey(reason))}</span>
            {readiness?.store_corrupt && (
              <button
                type="button"
                className="images-send"
                onClick={() => {
                  void api.rebuildWorkbench().then(async () => {
                    setNotice("images.rebuildDone");
                    try {
                      const [nextReadiness, nextSnapshot] = await Promise.all([
                        api.getWorkbenchReadiness(),
                        api.listWorkbench(),
                      ]);
                      setReadiness(nextReadiness);
                      applySnapshot(nextSnapshot);
                    } catch {
                      setError("images.error.failed");
                    }
                  });
                }}
              >
                {t("images.rebuild")}
              </button>
            )}
          </div>
        )}

        {showEditWarning && (
          <p className="images-warning" role="status">
            {t("images.editWarning")}
          </p>
        )}

        {landing ? null : (
          <section className="images-well" ref={wellRef} aria-live="polite">
            <div className="images-thread">
              {messages.length === 0 ? (
                <>
                  {pendingText ? (
                    <article className="images-bubble images-bubble--user">
                      <p className="images-who">{t("images.who.you")}</p>
                      <div className="images-paper">{pendingText}</div>
                    </article>
                  ) : null}
                  <article className="images-bubble">
                    <p className="images-who">{t("images.who.assistant")}</p>
                    <p className="images-progress">
                      <span className="images-dots" aria-hidden="true">
                        <i />
                        <i />
                        <i />
                      </span>
                      {t("images.phase.requesting")}
                    </p>
                  </article>
                </>
              ) : (
                messages.map((message) => (
                  <TimelineMessage
                    key={message.id}
                    message={message}
                    jobs={snapshot.jobs.filter((job) =>
                      message.job_ids.includes(job.id),
                    )}
                    assets={snapshot.assets}
                    busy={busy}
                    canSubmit={canSubmit}
                    onQuote={(asset) => updateDraft({ reference: asset })}
                    onRegenerate={(jobId) => void regenerate(jobId)}
                    onSave={(assetId) => void api.saveWorkbenchAsset(assetId)}
                    t={t}
                  />
                ))
              )}
            </div>
          </section>
        )}

        <footer className="images-composer">
          <div className="images-composer-wrap">
            <ModelDials
              selected={selected}
              readiness={readiness}
              onChange={(patch) => void changeOptions(patch)}
              t={t}
            />
            {draft.reference && (
              <div
                className="images-ref-chip"
                aria-label={t("images.referenceLabel")}
              >
                <img
                  alt={t("images.referenceAlt")}
                  src={workbenchAssetUri(draft.reference.id)}
                />
                <span>{t("images.referenceChip")}</span>
                <button
                  type="button"
                  aria-label={t("images.clearReferenceShort")}
                  onClick={() => updateDraft({ reference: null })}
                >
                  ×
                </button>
              </div>
            )}
            <div className="images-composer-shell">
              <textarea
                ref={composerRef}
                rows={landing ? 3 : 1}
                aria-label={t("images.composerPlaceholder")}
                placeholder={
                  landing
                    ? t("images.emptyTitle")
                    : t("images.composerInputHint")
                }
                value={draft.text}
                onChange={(event) => {
                  updateDraft({ text: event.target.value });
                  autosizeComposer(event.currentTarget);
                }}
                onKeyDown={(event) => {
                  if (
                    event.key === "Enter" &&
                    !event.shiftKey &&
                    !event.nativeEvent.isComposing
                  ) {
                    event.preventDefault();
                    void submit(false);
                  }
                }}
                onPaste={(event) => {
                  const file = Array.from(event.clipboardData.files)[0];
                  if (file) {
                    event.preventDefault();
                    void onFiles([file]);
                  }
                }}
              />
              <div className="images-composer__toolbar">
                <div className="images-composer__tools">
                  <button
                    type="button"
                    className="images-ghost-btn"
                    aria-label={t("images.upload")}
                    title={t("images.upload")}
                    onClick={() =>
                      void api
                        .pickWorkbenchReference()
                        .then((asset) => {
                          updateDraft({ reference: asset });
                        })
                        .catch((cause: unknown) => {
                          const code = commandErrorCode(cause);
                          if (code && code !== "WORKBENCH_CANCELLED") {
                            setError(errorKey(code));
                          }
                        })
                    }
                  >
                    <WorkbenchIcon type="upload" />
                  </button>
                  {busy ? null : (
                    <button
                      type="button"
                      className="images-ghost-btn"
                      aria-label={t("images.imagine")}
                      title={t("images.imagine")}
                      disabled={!canSubmit}
                      onClick={() => void submit(true)}
                    >
                      <WorkbenchIcon type="spark" />
                    </button>
                  )}
                </div>
                {busy ? (
                  <button
                    type="button"
                    className="images-send is-stop"
                    aria-label={t("images.stop")}
                    title={t("images.stop")}
                    onClick={() => void api.cancelWorkbench()}
                  >
                    <WorkbenchIcon type="stop" />
                  </button>
                ) : (
                  <button
                    type="button"
                    className="images-send"
                    aria-label={t("images.send")}
                    title={t("images.send")}
                    disabled={!canSubmit}
                    onClick={() => void submit(false)}
                  >
                    <WorkbenchIcon type="send" />
                  </button>
                )}
              </div>
            </div>
            {landing ? (
              <div className="images-suggest">
                <p className="images-suggest__label">
                  {t("images.suggestions")}
                </p>
                <ul>
                  {STARTERS.map((item) => (
                    <li key={item.prompt}>
                      <button
                        type="button"
                        aria-label={t(item.label)}
                        title={t(item.desc)}
                        onClick={() => void fillStarter(t(item.prompt))}
                      >
                        <div className="images-suggest__thumb">
                          <img
                            src={item.image}
                            alt={t(item.label)}
                            loading="lazy"
                          />
                        </div>
                        <div className="images-suggest__body">
                          <strong>{t(item.label)}</strong>
                          <span>{t(item.desc)}</span>
                        </div>
                      </button>
                    </li>
                  ))}
                </ul>
              </div>
            ) : (
              <p className="images-hint">{t("images.composerHint")}</p>
            )}
          </div>
        </footer>

        {error && (
          <p className="images-error" role="alert">
            {t(error)}
          </p>
        )}
        {notice && (
          <p className="images-notice" role="status">
            {t(notice)}
          </p>
        )}
        {dragging && (
          <p className="images-drop" role="status">
            {t("images.dropHint")}
          </p>
        )}
      </div>

      {pendingDelete && (
        <ConfirmDialog
          title={t("images.deleteConfirmTitle")}
          description={t("images.deleteConfirm")}
          confirmLabel={t("images.deleteAction")}
          cancelLabel={t("images.deleteCancel")}
          danger
          onCancel={() => setPendingDelete(null)}
          onConfirm={() => void confirmDelete()}
        />
      )}
    </section>
  );
}

function TimelineMessage({
  message,
  jobs,
  assets,
  busy,
  canSubmit,
  onQuote,
  onRegenerate,
  onSave,
  t,
}: {
  message: WorkbenchMessage;
  jobs: WorkbenchImageJob[];
  assets: WorkbenchAsset[];
  busy: boolean;
  canSubmit: boolean;
  onQuote(asset: WorkbenchAsset): void;
  onRegenerate(jobId: string): void;
  onSave(assetId: string): void;
  t: (key: TranslationKey) => string;
}) {
  const quoted = message.reference_asset_id
    ? assets.find((item) => item.id === message.reference_asset_id)
    : undefined;
  return (
    <article
      className={
        message.role === "user"
          ? "images-bubble images-bubble--user"
          : "images-bubble"
      }
    >
      <p className="images-who">
        {t(message.role === "user" ? "images.who.you" : "images.who.assistant")}
      </p>
      {quoted && (
        <div className="images-msg-ref">
          <img
            alt={t("images.referenceAlt")}
            src={workbenchAssetUri(quoted.id)}
          />
          <span>{t("images.referenceChip")}</span>
        </div>
      )}
      {message.visible_text && (
        <div className="images-paper">{message.visible_text}</div>
      )}
      {message.phase && (
        <p className="images-progress">
          <span className="images-dots" aria-hidden="true">
            <i />
            <i />
            <i />
          </span>
          {t(
            message.phase === "thinking"
              ? "images.phase.thinking"
              : message.phase === "imaging"
                ? "images.phase.imaging"
                : "images.phase.requesting",
          )}
        </p>
      )}
      {message.error_kind && (
        <p className="images-error">{t(errorKey(message.error_kind))}</p>
      )}
      {jobs.map((job) => {
        const asset = assets.find((item) => item.id === job.output_asset_id);
        return (
          <figure key={job.id} className="images-print">
            {asset ? (
              <div className="images-print-frame">
                <img
                  alt={t("images.generatedAlt")}
                  src={workbenchAssetUri(asset.id)}
                />
                <div className="images-print-hover">
                  <button
                    type="button"
                    className="images-quote-btn"
                    title={t("images.quote")}
                    aria-label={t("images.quote")}
                    onClick={() => onQuote(asset)}
                  >
                    <WorkbenchIcon type="quote" />
                  </button>
                </div>
              </div>
            ) : (
              <div className="images-tray">
                {t(
                  job.status === "cancelled"
                    ? "images.status.cancelled"
                    : job.status === "interrupted"
                      ? "images.status.interrupted"
                      : job.status === "failed"
                        ? "images.status.failed"
                        : "images.status.running",
                )}
              </div>
            )}
            {job.error_kind && <p>{t(errorKey(job.error_kind))}</p>}
            <div className="images-print-actions">
              {asset && (
                <>
                  <button
                    type="button"
                    title={t("images.quote")}
                    aria-label={t("images.quote")}
                    onClick={() => onQuote(asset)}
                  >
                    <WorkbenchIcon type="quote" />
                  </button>
                  <button
                    type="button"
                    title={t("images.save")}
                    aria-label={t("images.save")}
                    onClick={() => onSave(asset.id)}
                  >
                    <WorkbenchIcon type="download" />
                  </button>
                </>
              )}
              <button
                type="button"
                disabled={busy || !canSubmit}
                title={t("images.regenerate")}
                aria-label={t("images.regenerate")}
                onClick={() => onRegenerate(job.id)}
              >
                <WorkbenchIcon type="refresh" />
              </button>
            </div>
          </figure>
        );
      })}
    </article>
  );
}
