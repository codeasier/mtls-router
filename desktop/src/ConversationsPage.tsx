import { useCallback, useEffect, useRef, useState } from "react";
import { useI18n } from "./i18n";
import {
  type DesktopApi,
  type ImageConversation,
  type ImageMessage,
  type ImageReadiness,
  type ImageImportResult,
  type ImageOperationEvent,
  IMAGE_PRESET_MODELS,
  imageAssetUri,
} from "./ipc";
import type { TranslationKey } from "./locales/zh-CN";

function errorTranslation(error: {
  code?: string;
  message?: string;
}): TranslationKey {
  const code = error?.code ?? "";
  if (code.startsWith("IMAGE_BUSY")) return "conversations.error.busy";
  if (code.startsWith("IMAGE_INVALID_PROMPT"))
    return "conversations.error.invalidPrompt";
  if (code.startsWith("IMAGE_INVALID_MODEL"))
    return "conversations.error.invalidModel";
  if (code.startsWith("CREDENTIAL")) return "conversations.error.notReady";
  if (code.startsWith("IMAGE_STORE")) return "conversations.error.store";
  if (code.includes("CHANNEL") || code.includes("NOT_READY"))
    return "conversations.error.channel";
  return "conversations.error.channel";
}

interface DraftState {
  prompt: string;
  referenceAssetId: string;
}

export function ConversationsPage({ api }: { api: DesktopApi }) {
  const { t } = useI18n();
  const [readiness, setReadiness] = useState<ImageReadiness | null>(null);
  const [conversations, setConversations] = useState<ImageConversation[]>([]);
  const [selectedId, setSelectedId] = useState<string>("");
  const [messages, setMessages] = useState<ImageMessage[]>([]);
  const [drafts, setDrafts] = useState<Record<string, DraftState>>({});
  const [busy, setBusy] = useState(false);
  const [modelSaving, setModelSaving] = useState(false);
  const [error, setError] = useState<TranslationKey | "">("");
  const [storeFailed, setStoreFailed] = useState(false);
  const unlistenRef = useRef<(() => void) | null>(null);
  const activeOperationRef = useRef<string>("");
  const startingOperationRef = useRef(false);
  const pendingOperationEventRef = useRef<ImageOperationEvent | null>(null);
  const completedOperationRef = useRef<string>("");
  const operationEpochRef = useRef(0);
  const operationQueryRef = useRef(0);

  const takePendingOperationEvent = () => {
    const event = pendingOperationEventRef.current;
    pendingOperationEventRef.current = null;
    return event;
  };

  const loadConversations = useCallback(async () => {
    try {
      const [r, convs] = await Promise.all([
        api.imageReadiness(),
        api.imageConversations(),
      ]);
      setReadiness(r);
      setStoreFailed(false);
      setConversations(convs);
      setSelectedId((current) =>
        convs.some((conversation) => conversation.id === current)
          ? current
          : (convs.find((conversation) => conversation.selected)?.id ??
            convs[0]?.id ??
            ""),
      );
    } catch (e) {
      setStoreFailed(
        ((e as { code?: string })?.code ?? "").startsWith("IMAGE_STORE"),
      );
      setError(errorTranslation(e as { code?: string }));
    }
  }, [api]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    loadConversations();
  }, [loadConversations]);

  useEffect(() => {
    let active = true;
    const load = async () => {
      if (selectedId) {
        try {
          const msgs = await api.imageMessages(selectedId);
          if (active) setMessages(msgs);
        } catch {
          // ignore load errors
        }
      } else if (active) {
        setMessages([]);
      }
    };
    load();
    return () => {
      active = false;
    };
  }, [api, selectedId]);

  const applyOperationEvent = useCallback(
    (event: ImageOperationEvent) => {
      operationEpochRef.current += 1;
      if (event.conversation_id === selectedId) {
        api
          .imageMessages(selectedId)
          .then(setMessages)
          .catch(() => {});
      }
      activeOperationRef.current = "";
      completedOperationRef.current = event.operation_id;
      setBusy(false);
      if (event.status === "failed") {
        setError("conversations.error.channel");
      } else if (event.status === "cancelled") {
        setError("conversations.error.cancelled");
      }
    },
    [api, selectedId],
  );

  useEffect(() => {
    let disposed = false;
    let fallbackTimer: number | undefined;
    let observedOperation = "";

    const reconcileOperation = async () => {
      const epoch = operationEpochRef.current;
      const query = ++operationQueryRef.current;
      const operation = await api.imageCurrentOperation();
      if (
        disposed ||
        epoch !== operationEpochRef.current ||
        query !== operationQueryRef.current
      )
        return;
      if (operation) {
        if (completedOperationRef.current === operation.operation_id) {
          activeOperationRef.current = "";
          setBusy(false);
          return;
        }
        observedOperation = operation.operation_id;
        activeOperationRef.current = operation.operation_id;
        setBusy(true);
        return;
      }
      activeOperationRef.current = "";
      setBusy(false);
      if (observedOperation && selectedId) {
        observedOperation = "";
        api
          .imageMessages(selectedId)
          .then(setMessages)
          .catch(() => {});
      }
    };

    const setup = async () => {
      try {
        const unlisten = await api.subscribeImageOperations(
          (event: ImageOperationEvent) => {
            if (startingOperationRef.current) {
              pendingOperationEventRef.current = event;
            } else if (
              !activeOperationRef.current ||
              event.operation_id === activeOperationRef.current
            ) {
              observedOperation = "";
              applyOperationEvent(event);
            }
          },
        );
        if (disposed) unlisten();
        else unlistenRef.current = unlisten;
      } catch {
        fallbackTimer = window.setInterval(() => {
          reconcileOperation().catch(() => {});
        }, 500);
      }
      await reconcileOperation().catch(() => {});
    };
    setup();
    return () => {
      disposed = true;
      if (fallbackTimer !== undefined) window.clearInterval(fallbackTimer);
      unlistenRef.current?.();
      unlistenRef.current = null;
    };
  }, [api, applyOperationEvent, selectedId]);

  const draft = drafts[selectedId] ?? { prompt: "", referenceAssetId: "" };

  const updateDraft = (update: Partial<DraftState>) => {
    if (!selectedId) return;
    setDrafts((prev) => ({
      ...prev,
      [selectedId]: {
        ...(prev[selectedId] ?? { prompt: "", referenceAssetId: "" }),
        ...update,
      },
    }));
  };

  const handleCreate = async () => {
    try {
      const model = readiness?.available_models.find((m) => m.available)?.id;
      if (!model) {
        setError("conversations.error.invalidModel");
        return;
      }
      const conv = await api.imageCreateConversation(model);
      setConversations((prev) => [
        conv,
        ...prev.map((conversation) => ({ ...conversation, selected: false })),
      ]);
      setSelectedId(conv.id);
      setError("");
    } catch (e) {
      setError(errorTranslation(e as { code?: string }));
    }
  };

  const handleDelete = async (id: string) => {
    if (!confirm(t("conversations.deleteConfirm"))) return;
    try {
      await api.imageDeleteConversation(id);
      setConversations((prev) => prev.filter((c) => c.id !== id));
      if (selectedId === id) {
        setSelectedId("");
      }
    } catch (e) {
      setError(errorTranslation(e as { code?: string }));
    }
  };

  const handleResetStore = async () => {
    if (!confirm(t("conversations.resetConfirm"))) return;
    try {
      await api.imageResetStore();
      setDrafts({});
      setMessages([]);
      setSelectedId("");
      setStoreFailed(false);
      setError("");
      await loadConversations();
    } catch (e) {
      setError(errorTranslation(e as { code?: string }));
    }
  };

  const handleSelect = async (id: string) => {
    setSelectedId(id);
    setConversations((current) =>
      current.map((conversation) => ({
        ...conversation,
        selected: conversation.id === id,
      })),
    );
    try {
      await api.imageSelectConversation(id);
    } catch {
      // selection is local; backend failure is non-critical
    }
  };

  const handleModelSelect = async (model: string) => {
    const conversation = conversations.find((item) => item.id === selectedId);
    const available = readiness?.available_models.some(
      (item) => item.id === model && item.available,
    );
    if (
      !conversation ||
      !available ||
      modelSaving ||
      busy ||
      conversation.selected_model === model
    ) {
      return;
    }

    const previousModel = conversation.selected_model;
    setError("");
    setModelSaving(true);
    setConversations((current) =>
      current.map((item) =>
        item.id === conversation.id ? { ...item, selected_model: model } : item,
      ),
    );
    try {
      await api.imageSetConversationModel(conversation.id, model);
    } catch (e) {
      setConversations((current) =>
        current.map((item) =>
          item.id === conversation.id && item.selected_model === model
            ? { ...item, selected_model: previousModel }
            : item,
        ),
      );
      setError(errorTranslation(e as { code?: string }));
    } finally {
      setModelSaving(false);
    }
  };

  const handleUpload = async () => {
    try {
      const result: ImageImportResult = await api.imageSelectReference();
      updateDraft({ referenceAssetId: result.asset_id });
    } catch (e) {
      const code = (e as { code?: string })?.code ?? "";
      if (code !== "IMAGE_FILE_CANCELLED") {
        setError(errorTranslation(e as { code?: string }));
      }
    }
  };

  const handleSubmit = async () => {
    if (busy) {
      setError("conversations.error.busy");
      return;
    }
    if (!draft.prompt.trim()) {
      setError("conversations.error.invalidPrompt");
      return;
    }
    const model =
      conversations.find((c) => c.id === selectedId)?.selected_model ?? "";
    if (!model) {
      setError("conversations.error.invalidModel");
      return;
    }
    try {
      setError("");
      setBusy(true);
      operationEpochRef.current += 1;
      startingOperationRef.current = true;
      pendingOperationEventRef.current = null;
      const started = await api.imageStartGeneration({
        conversation_id: selectedId,
        model,
        prompt: draft.prompt,
        reference_asset_id: draft.referenceAssetId,
      });
      operationEpochRef.current += 1;
      activeOperationRef.current = started.operation_id;
      startingOperationRef.current = false;
      const pendingEvent = takePendingOperationEvent();
      if (pendingEvent?.operation_id === started.operation_id) {
        applyOperationEvent(pendingEvent);
      }
      setDrafts((prev) => ({
        ...prev,
        [selectedId]: { prompt: "", referenceAssetId: "" },
      }));
      api
        .imageMessages(selectedId)
        .then(setMessages)
        .catch(() => {});
    } catch (e) {
      operationEpochRef.current += 1;
      startingOperationRef.current = false;
      activeOperationRef.current = "";
      pendingOperationEventRef.current = null;
      setBusy(false);
      setError(errorTranslation(e as { code?: string }));
    }
  };

  const handleCancel = async () => {
    try {
      await api.imageCancelGeneration();
    } catch {
      // cancel is best-effort
    }
  };

  const handleContinueEdit = (assetId: string) => {
    updateDraft({ referenceAssetId: assetId });
  };

  const availableModels = readiness?.available_models ?? [];
  const selectedConv = conversations.find((c) => c.id === selectedId);
  const selectedModelAvailable =
    availableModels.find((m) => m.id === selectedConv?.selected_model)
      ?.available ?? false;
  const modelOptions: Array<{ id: string; display_name: string }> =
    IMAGE_PRESET_MODELS.map((model) => ({ ...model }));
  if (
    selectedConv?.selected_model &&
    !modelOptions.some((model) => model.id === selectedConv.selected_model)
  ) {
    const persistedModel = availableModels.find(
      (model) => model.id === selectedConv.selected_model,
    );
    modelOptions.push({
      id: selectedConv.selected_model,
      display_name: persistedModel?.display_name ?? selectedConv.selected_model,
    });
  }

  return (
    <section
      className="conversations-panel"
      aria-labelledby="conversations-heading"
    >
      <p className="overline">{t("conversations.overline")}</p>
      <h2 id="conversations-heading">{t("conversations.heading")}</h2>

      {error && (
        <div className="conversations-alert" role="alert">
          <p>{t(error)}</p>
          {storeFailed && (
            <button
              type="button"
              className="control-button--danger"
              onClick={handleResetStore}
            >
              {t("conversations.reset")}
            </button>
          )}
        </div>
      )}

      <div className="conversations-layout">
        <aside className="conversations-sidebar">
          <button
            type="button"
            className="control-button"
            onClick={handleCreate}
          >
            {t("conversations.new")}
          </button>
          <ul className="conversations-list">
            {conversations.length === 0 && (
              <li className="conversations-empty">
                {t("conversations.empty")}
              </li>
            )}
            {conversations.map((conv) => (
              <li
                key={conv.id}
                className="conversations-item"
                data-selected={conv.id === selectedId}
              >
                <button
                  type="button"
                  className="conversations-item-button"
                  onClick={() => handleSelect(conv.id)}
                >
                  <span className="conversations-item-title">
                    {conv.title || t("conversations.new")}
                  </span>
                  <span className="conversations-item-meta">
                    {conv.message_count} {t("conversations.status.succeeded")}
                  </span>
                </button>
                <button
                  type="button"
                  className="text-button conversations-delete"
                  onClick={() => handleDelete(conv.id)}
                  aria-label={t("conversations.delete")}
                >
                  {t("conversations.delete")}
                </button>
              </li>
            ))}
          </ul>
        </aside>

        <div className="conversations-main">
          {!readiness?.ready && (
            <div className="conversations-not-ready" role="status">
              <p>{t("conversations.error.notReady")}</p>
            </div>
          )}

          {selectedId && (
            <>
              <div
                className="conversations-models"
                role="radiogroup"
                aria-label={t("conversations.model")}
                aria-busy={modelSaving}
              >
                <span className="conversations-model-label">
                  {t("conversations.model")}
                </span>
                {modelOptions.map((preset) => {
                  const available =
                    availableModels.find((m) => m.id === preset.id)
                      ?.available ?? false;
                  const isSelected = selectedConv?.selected_model === preset.id;
                  return (
                    <button
                      type="button"
                      role="radio"
                      aria-checked={isSelected}
                      key={preset.id}
                      className="conversations-model-badge"
                      data-selected={isSelected}
                      data-available={available}
                      disabled={!available || modelSaving || busy}
                      onClick={() => handleModelSelect(preset.id)}
                    >
                      {preset.display_name}
                      {!available && (
                        <small> — {t("conversations.modelUnavailable")}</small>
                      )}
                    </button>
                  );
                })}
                <small className="conversations-model-risk">
                  {t("conversations.modelRisk")}
                </small>
              </div>

              <div className="conversations-messages" aria-live="polite">
                {messages.map((msg) => (
                  <div
                    key={msg.id}
                    className="conversations-message"
                    data-role={msg.role}
                  >
                    {msg.role === "user" && (
                      <div className="conversations-user-message">
                        {msg.reference_asset_id && (
                          <img
                            src={imageAssetUri(msg.reference_asset_id)}
                            alt=""
                            className="conversations-reference-thumb"
                          />
                        )}
                        <p>{msg.prompt}</p>
                      </div>
                    )}
                    {msg.role === "assistant" && (
                      <div
                        className="conversations-assistant-message"
                        data-status={msg.status}
                      >
                        {msg.status === "running" && (
                          <p>{t("conversations.status.running")}</p>
                        )}
                        {msg.status === "succeeded" && msg.output_asset_id && (
                          <div className="conversations-generated">
                            <img
                              src={imageAssetUri(msg.output_asset_id)}
                              alt=""
                              className="conversations-generated-image"
                            />
                            <button
                              type="button"
                              className="text-button"
                              onClick={() =>
                                handleContinueEdit(msg.output_asset_id)
                              }
                            >
                              {t("conversations.continueEdit")}
                            </button>
                          </div>
                        )}
                        {msg.status === "failed" && (
                          <p>
                            {t("conversations.status.failed")}:{" "}
                            {msg.error_category}
                          </p>
                        )}
                        {msg.status === "cancelled" && (
                          <p>{t("conversations.status.cancelled")}</p>
                        )}
                        {msg.status === "interrupted" && (
                          <p>{t("conversations.status.interrupted")}</p>
                        )}
                      </div>
                    )}
                  </div>
                ))}
              </div>

              <div className="conversations-composer">
                {draft.referenceAssetId && (
                  <div className="conversations-reference-preview">
                    <img
                      src={imageAssetUri(draft.referenceAssetId)}
                      alt=""
                      className="conversations-reference-thumb"
                    />
                    <button
                      type="button"
                      className="text-button"
                      onClick={() => updateDraft({ referenceAssetId: "" })}
                    >
                      {t("conversations.removeReference")}
                    </button>
                  </div>
                )}
                <textarea
                  className="conversations-prompt-input"
                  placeholder={t("conversations.promptPlaceholder")}
                  value={draft.prompt}
                  onChange={(e) => updateDraft({ prompt: e.target.value })}
                  aria-label={t("conversations.prompt")}
                  aria-multiline="true"
                />
                <div className="conversations-actions">
                  <button
                    type="button"
                    className="text-button"
                    onClick={handleUpload}
                  >
                    {t("conversations.upload")}
                  </button>
                  {busy ? (
                    <button
                      type="button"
                      className="control-button--danger"
                      onClick={handleCancel}
                    >
                      {t("conversations.cancel")}
                    </button>
                  ) : (
                    <button
                      type="button"
                      className="control-button"
                      onClick={handleSubmit}
                      disabled={
                        !selectedModelAvailable ||
                        !readiness?.ready ||
                        modelSaving
                      }
                    >
                      {t("conversations.submit")}
                    </button>
                  )}
                </div>
                {busy && (
                  <p className="conversations-busy" aria-live="polite">
                    {t("conversations.busy")}
                  </p>
                )}
              </div>
            </>
          )}
        </div>
      </div>

      <div className="conversations-notice">
        <p>{t("conversations.dataNotice")}</p>
        <p>
          <small>{t("conversations.limitPrompt")}</small> ·{" "}
          <small>{t("conversations.limitImage")}</small>
        </p>
      </div>
    </section>
  );
}
