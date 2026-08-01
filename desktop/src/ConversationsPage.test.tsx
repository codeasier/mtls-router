import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ConversationsPage } from "./ConversationsPage";
import type { DesktopApi, ImageConversation } from "./ipc";
import { createMockApi } from "./test/api";
import { renderWithI18n } from "./test/render";

const GPT_MODEL = "cx/gpt-5.5-image";
const GEMINI_MODEL = "ag/gemini-3.1-flash-image";

function conversation(model = GPT_MODEL): ImageConversation {
  return {
    id: "conversation-1",
    selected: true,
    title: "Fixture conversation",
    selected_model: model,
    message_count: 0,
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
  };
}

function renderPage(overrides: Partial<DesktopApi> = {}) {
  const api = createMockApi({
    imageConversations: vi.fn().mockResolvedValue([conversation()]),
    ...overrides,
  });
  renderWithI18n(<ConversationsPage api={api} />);
  return api;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("ConversationsPage", () => {
  it("persists an explicit model selection and submits that exact model", async () => {
    const user = userEvent.setup();
    const api = renderPage();

    const gemini = await screen.findByRole("radio", {
      name: "Gemini 3.1 Flash Image",
    });
    expect(screen.getByRole("radio", { name: "GPT 5.5 Image" })).toBeChecked();

    await user.click(gemini);

    await waitFor(() =>
      expect(api.imageSetConversationModel).toHaveBeenCalledWith(
        "conversation-1",
        GEMINI_MODEL,
      ),
    );
    expect(gemini).toBeChecked();

    await user.type(screen.getByLabelText("输入提示词"), "draw a lighthouse");
    await user.click(screen.getByRole("button", { name: "生成" }));

    expect(api.imageStartGeneration).toHaveBeenCalledWith({
      conversation_id: "conversation-1",
      model: GEMINI_MODEL,
      prompt: "draw a lighthouse",
      reference_asset_id: "",
    });
  });

  it("preserves an unavailable selection until the user chooses an available model", async () => {
    const user = userEvent.setup();
    const api = renderPage({
      imageReadiness: vi.fn().mockResolvedValue({
        ready: true,
        available_models: [
          { id: GPT_MODEL, display_name: "GPT 5.5 Image", available: false },
          {
            id: GEMINI_MODEL,
            display_name: "Gemini 3.1 Flash Image",
            available: true,
          },
        ],
        reason: "ok",
      }),
    });

    const unavailable = await screen.findByRole("radio", {
      name: /GPT 5\.5 Image/,
    });
    expect(unavailable).toBeChecked();
    expect(unavailable).toBeDisabled();
    expect(screen.getByRole("button", { name: "生成" })).toBeDisabled();
    expect(api.imageSetConversationModel).not.toHaveBeenCalled();

    await user.click(
      screen.getByRole("radio", { name: "Gemini 3.1 Flash Image" }),
    );

    await waitFor(() =>
      expect(api.imageSetConversationModel).toHaveBeenCalledWith(
        "conversation-1",
        GEMINI_MODEL,
      ),
    );
    expect(screen.getByRole("button", { name: "生成" })).toBeEnabled();
  });

  it("rolls back the model control when persistence fails", async () => {
    const user = userEvent.setup();
    const api = renderPage({
      imageSetConversationModel: vi
        .fn()
        .mockRejectedValue({ code: "IMAGE_STORE_IO" }),
    });

    await user.click(
      await screen.findByRole("radio", { name: "Gemini 3.1 Flash Image" }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent("本地存储错误");
    expect(screen.getByRole("radio", { name: "GPT 5.5 Image" })).toBeChecked();
    expect(api.imageStartGeneration).not.toHaveBeenCalled();
  });

  it("passes an imported reference asset through generation", async () => {
    const user = userEvent.setup();
    const api = renderPage();

    await screen.findByText("Fixture conversation");
    await user.click(screen.getByRole("button", { name: "上传参考图" }));
    await user.type(screen.getByLabelText("输入提示词"), "edit this image");
    await user.click(screen.getByRole("button", { name: "生成" }));

    expect(api.imageStartGeneration).toHaveBeenCalledWith(
      expect.objectContaining({
        model: GPT_MODEL,
        prompt: "edit this image",
        reference_asset_id: "a".repeat(64),
      }),
    );
  });

  it("does not create a conversation with an unavailable fallback model", async () => {
    const user = userEvent.setup();
    const api = renderPage({
      imageReadiness: vi.fn().mockResolvedValue({
        ready: false,
        available_models: [
          { id: GPT_MODEL, display_name: "GPT 5.5 Image", available: false },
        ],
        reason: "unavailable",
      }),
      imageConversations: vi.fn().mockResolvedValue([]),
    });

    await user.click(await screen.findByRole("button", { name: "新建对话" }));

    expect(api.imageCreateConversation).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent("所选模型不可用");
  });

  it("restores the persisted selected conversation", async () => {
    const first = conversation();
    first.selected = false;
    const second = { ...conversation(GEMINI_MODEL), id: "conversation-2" };
    const imageMessages = vi.fn().mockResolvedValue([]);

    renderPage({
      imageConversations: vi.fn().mockResolvedValue([first, second]),
      imageMessages,
    });

    await waitFor(() =>
      expect(imageMessages).toHaveBeenCalledWith("conversation-2"),
    );
  });

  it("offers an explicit confirmed rebuild after storage corruption", async () => {
    const user = userEvent.setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const imageConversations = vi
      .fn()
      .mockRejectedValueOnce({ code: "IMAGE_STORE_ERROR" })
      .mockResolvedValue([]);
    const api = renderPage({ imageConversations });

    await user.click(
      await screen.findByRole("button", { name: "重建图片数据" }),
    );

    expect(api.imageResetStore).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(imageConversations).toHaveBeenCalledTimes(2));
  });

  it("ignores a stale operation event and applies only the active operation", async () => {
    const user = userEvent.setup();
    const started = deferred<{ operation_id: string; message_id: string }>();
    let listener:
      | ((event: {
          operation_id: string;
          conversation_id: string;
          message_id: string;
          status: "succeeded" | "failed" | "cancelled";
        }) => void)
      | null = null;
    const api = renderPage({
      imageStartGeneration: vi.fn().mockReturnValue(started.promise),
      subscribeImageOperations: vi.fn().mockImplementation(async (next) => {
        listener = next;
        return () => undefined;
      }),
    });

    await user.type(
      await screen.findByLabelText("输入提示词"),
      "draw a harbor",
    );
    await user.click(screen.getByRole("button", { name: "生成" }));
    await waitFor(() => expect(listener).not.toBeNull());

    act(() => {
      listener?.({
        operation_id: "stale-operation",
        conversation_id: "conversation-1",
        message_id: "stale-message",
        status: "failed",
      });
    });
    expect(screen.getByRole("button", { name: "取消" })).toBeVisible();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();

    await act(async () => {
      started.resolve({
        operation_id: "active-operation",
        message_id: "msg-1",
      });
      await started.promise;
    });
    act(() => {
      listener?.({
        operation_id: "active-operation",
        conversation_id: "conversation-1",
        message_id: "msg-1",
        status: "succeeded",
      });
    });

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "生成" })).toBeVisible(),
    );
    expect(api.imageMessages).toHaveBeenCalledWith("conversation-1");
  });

  it("recovers an active backend operation after the page remounts", async () => {
    let listener:
      | ((event: {
          operation_id: string;
          conversation_id: string;
          message_id: string;
          status: "succeeded" | "failed" | "cancelled";
        }) => void)
      | null = null;
    renderPage({
      imageCurrentOperation: vi.fn().mockResolvedValue({
        operation_id: "restored-operation",
        conversation_id: "conversation-1",
        message_id: "restored-message",
      }),
      subscribeImageOperations: vi.fn().mockImplementation(async (next) => {
        listener = next;
        return () => undefined;
      }),
    });

    expect(await screen.findByRole("button", { name: "取消" })).toBeVisible();
    act(() => {
      listener?.({
        operation_id: "restored-operation",
        conversation_id: "conversation-1",
        message_id: "restored-message",
        status: "succeeded",
      });
    });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "生成" })).toBeVisible(),
    );
  });

  it("ignores an operation query older than a completion and newer start", async () => {
    const user = userEvent.setup();
    const staleQuery = deferred<{
      operation_id: string;
      conversation_id: string;
      message_id: string;
    } | null>();
    let listener:
      | ((event: {
          operation_id: string;
          conversation_id: string;
          message_id: string;
          status: "succeeded" | "failed" | "cancelled";
        }) => void)
      | null = null;
    renderPage({
      imageCurrentOperation: vi.fn().mockReturnValue(staleQuery.promise),
      imageStartGeneration: vi.fn().mockResolvedValue({
        operation_id: "new-operation",
        message_id: "new-message",
      }),
      subscribeImageOperations: vi.fn().mockImplementation(async (next) => {
        listener = next;
        return () => undefined;
      }),
    });

    await waitFor(() => expect(listener).not.toBeNull());
    act(() => {
      listener?.({
        operation_id: "old-operation",
        conversation_id: "conversation-1",
        message_id: "old-message",
        status: "succeeded",
      });
    });
    await user.type(
      await screen.findByLabelText("输入提示词"),
      "start the newer operation",
    );
    await user.click(screen.getByRole("button", { name: "生成" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "取消" })).toBeVisible(),
    );

    await act(async () => {
      staleQuery.resolve({
        operation_id: "old-operation",
        conversation_id: "conversation-1",
        message_id: "old-message",
      });
      await staleQuery.promise;
    });
    expect(screen.getByRole("button", { name: "取消" })).toBeVisible();
  });
});
