import { fireEvent, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ImagesPage } from "./ImagesPage";
import {
  defaultWorkbenchReadiness,
  MOCK_WORKBENCH_ASSET_ID,
} from "./dev/workbenchMock";
import type {
  WorkbenchChatDelta,
  WorkbenchPhaseEvent,
  WorkbenchSnapshot,
} from "./ipc";
import { renderWithI18n } from "./test/render";
import { createMockApi } from "./test/api";
import { MAX_WORKBENCH_IMPORT_BYTES } from "./workbench";

async function openFreshConversation(api = createMockApi()) {
  renderWithI18n(<ImagesPage api={api} />);
  fireEvent.click(await screen.findByRole("button", { name: "新建会话" }));
  await screen.findByLabelText("对话模型");
  return api;
}

describe("ImagesPage", () => {
  it("disables submit when the workbench is not ready", async () => {
    const api = createMockApi({
      getWorkbenchReadiness: vi.fn().mockResolvedValue(
        defaultWorkbenchReadiness({
          ready: false,
          has_credential: false,
          can_submit: false,
          reason: "WORKBENCH_NO_CREDENTIAL",
          chat_models: [],
          image_models: [],
        }),
      ),
    });
    renderWithI18n(<ImagesPage api={api} />);
    await screen.findByText("还没有保存 API 密钥，请先到密钥页保存后再发送。");
    fireEvent.click(screen.getByRole("button", { name: "新建会话" }));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "发送" })).toBeDisabled();
      expect(screen.getByRole("button", { name: "出图" })).toBeDisabled();
    });
  });

  it("says the catalog is unavailable when models did not load", async () => {
    const api = createMockApi({
      getWorkbenchReadiness: vi.fn().mockResolvedValue(
        defaultWorkbenchReadiness({
          ready: false,
          can_submit: false,
          chat_models: [],
          image_models: [],
          reason: "WORKBENCH_CATALOG",
        }),
      ),
    });
    renderWithI18n(<ImagesPage api={api} />);
    await screen.findByText("模型目录不可用，发送已禁用。");
    expect(screen.queryByText(/本地路由尚未就绪/)).not.toBeInTheDocument();
  });

  it("disables submit when the default chat model is missing", async () => {
    const api = createMockApi({
      getWorkbenchReadiness: vi.fn().mockResolvedValue(
        defaultWorkbenchReadiness({
          chat_models: ["other-flash"],
          ready: false,
          can_submit: false,
          reason: "WORKBENCH_MODEL_MISSING",
        }),
      ),
    });
    renderWithI18n(<ImagesPage api={api} />);
    fireEvent.click(await screen.findByRole("button", { name: "新建会话" }));
    await screen.findByText(/不在本次目录里/);
    expect(screen.getByRole("button", { name: "发送" })).toBeDisabled();
  });

  it("only lists slash-free chat models from readiness", async () => {
    await openFreshConversation(
      createMockApi({
        getWorkbenchReadiness: vi.fn().mockResolvedValue(
          defaultWorkbenchReadiness({
            chat_models: ["gemini-3.8-flash", "gemini-3-flash"],
          }),
        ),
      }),
    );
    const chat = screen.getByLabelText("对话模型");
    expect(chat).toHaveTextContent("gemini-3.8-flash");
    expect(chat).not.toHaveTextContent("ag/");
    expect(chat).not.toHaveTextContent("cx/");
  });

  it("keeps model dials on the composer and centers a new conversation", async () => {
    const api = await openFreshConversation();
    const stage = document.querySelector(".images-stage");
    const composer = document.querySelector(".images-composer");
    const mast = document.querySelector(".images-mast");
    expect(stage).toHaveClass("is-landing");
    expect(composer?.querySelector('[aria-label="对话模型"]')).toBeTruthy();
    expect(composer?.querySelector('[aria-label="生图模型"]')).toBeTruthy();
    expect(mast?.querySelector("select")).toBeNull();
    expect(screen.getByText("建议")).toBeInTheDocument();
    expect(screen.getByLabelText("描述你想聊或想画的画面")).toHaveAttribute(
      "placeholder",
      "有什么想画的？",
    );

    fireEvent.change(screen.getByLabelText("描述你想聊或想画的画面"), {
      target: { value: "画一只猫" },
    });
    fireEvent.click(screen.getByRole("button", { name: "出图" }));
    await screen.findByAltText("生成的图片");
    expect(document.querySelector(".images-stage")).not.toHaveClass(
      "is-landing",
    );
    expect(screen.queryByText("建议")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "新建会话" }));
    await waitFor(() => {
      expect(document.querySelector(".images-stage")).toHaveClass("is-landing");
    });
    fireEvent.click(screen.getByRole("button", { name: "画一只猫" }));
    await waitFor(() => {
      expect(document.querySelector(".images-stage")).not.toHaveClass(
        "is-landing",
      );
    });
    expect(api.sendWorkbench).toHaveBeenCalled();
  });

  it("fills starter chips into the draft without sending", async () => {
    const api = await openFreshConversation();
    fireEvent.click(screen.getByRole("button", { name: "微服务云架构" }));
    const composer = screen.getByLabelText("描述你想聊或想画的画面");
    expect((composer as HTMLTextAreaElement).value).toContain(
      "现代软件微服务架构",
    );
    expect(composer).toHaveFocus();
    expect(api.sendWorkbench).not.toHaveBeenCalled();
  });

  it("shows the user turn while send is still running", async () => {
    let emitPhase: ((event: WorkbenchPhaseEvent) => void) | undefined;
    const api = createMockApi();
    const originalPhase = api.subscribeWorkbenchPhase;
    api.subscribeWorkbenchPhase = vi.fn(async (listener) => {
      emitPhase = listener;
      return originalPhase(listener);
    });
    api.sendWorkbench = vi.fn(
      async () => await new Promise<WorkbenchSnapshot>(() => undefined),
    );
    await openFreshConversation(api);
    const listed = await api.listWorkbench();
    const conversationId = listed.selected_conversation_id ?? "";
    api.listWorkbench = vi.fn().mockResolvedValue({
      ...listed,
      messages: [
        {
          id: "m-user",
          conversation_id: conversationId,
          role: "user",
          visible_text: "画一只猫",
          created_at: "2026-09-08T15:00:00Z",
          reference_asset_id: null,
          phase: null,
          error_kind: null,
          job_ids: [],
        },
        {
          id: "m-asst",
          conversation_id: conversationId,
          role: "assistant",
          visible_text: "",
          created_at: "2026-09-08T15:00:00Z",
          reference_asset_id: null,
          phase: "requesting",
          error_kind: null,
          job_ids: [],
        },
      ],
    });
    fireEvent.change(screen.getByLabelText("描述你想聊或想画的画面"), {
      target: { value: "画一只猫" },
    });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await screen.findByText("画一只猫");
    expect(screen.queryByText("有什么想画的？")).not.toBeInTheDocument();
    emitPhase?.({
      operation_id: "op-live",
      conversation_id: conversationId,
      message_id: "m-asst",
      phase: "thinking",
    });
    await screen.findByText("正在连接");
    expect(api.sendWorkbench).toHaveBeenCalled();
  });

  it("sends imagine as forceImage and explicit reference as edit without switching models", async () => {
    const api = await openFreshConversation();
    expect(screen.getByLabelText("生图模型")).toHaveValue(
      "ag/gemini-3.1-flash-image",
    );

    fireEvent.change(screen.getByLabelText("描述你想聊或想画的画面"), {
      target: { value: "画一只猫" },
    });
    fireEvent.click(screen.getByRole("button", { name: "出图" }));
    await waitFor(() =>
      expect(api.sendWorkbench).toHaveBeenLastCalledWith(
        expect.any(String),
        "画一只猫",
        true,
        null,
      ),
    );

    fireEvent.click(screen.getByRole("button", { name: "上传参考图" }));
    await screen.findByAltText("参考图");
    fireEvent.change(screen.getByLabelText("描述你想聊或想画的画面"), {
      target: { value: "改成蓝色" },
    });
    await screen.findByText(/不会改切到 gpt-image-2/);
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() =>
      expect(api.sendWorkbench).toHaveBeenLastCalledWith(
        expect.any(String),
        "改成蓝色",
        false,
        MOCK_WORKBENCH_ASSET_ID,
      ),
    );
    expect(screen.getByLabelText("生图模型")).toHaveValue(
      "ag/gemini-3.1-flash-image",
    );
  });

  it("sends implicit edit text without a reference and without forceImage", async () => {
    const api = await openFreshConversation();
    fireEvent.change(screen.getByLabelText("描述你想聊或想画的画面"), {
      target: { value: "画一只猫" },
    });
    fireEvent.click(screen.getByRole("button", { name: "出图" }));
    await screen.findByAltText("生成的图片");
    fireEvent.change(screen.getByLabelText("描述你想聊或想画的画面"), {
      target: { value: "改成蓝色的天空" },
    });
    await screen.findByText(/不会改切到 gpt-image-2/);
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() =>
      expect(api.sendWorkbench).toHaveBeenLastCalledWith(
        expect.any(String),
        "改成蓝色的天空",
        false,
        null,
      ),
    );
  });

  it("keeps drafts isolated across conversations", async () => {
    const api = await openFreshConversation();
    fireEvent.change(screen.getByLabelText("描述你想聊或想画的画面"), {
      target: { value: "会话 A 草稿" },
    });
    fireEvent.click(screen.getByRole("button", { name: "新建会话" }));
    await waitFor(() =>
      expect(screen.getByLabelText("描述你想聊或想画的画面")).toHaveValue(""),
    );
    fireEvent.change(screen.getByLabelText("描述你想聊或想画的画面"), {
      target: { value: "会话 B 草稿" },
    });
    fireEvent.click(screen.getAllByRole("button", { name: "未命名会话" })[1]);
    await waitFor(() =>
      expect(screen.getByLabelText("描述你想聊或想画的画面")).toHaveValue(
        "会话 A 草稿",
      ),
    );
    expect(api.sendWorkbench).not.toHaveBeenCalled();
  });

  it("offers rebuild only when local storage is corrupt", async () => {
    const api = createMockApi({
      getWorkbenchReadiness: vi.fn().mockResolvedValue(
        defaultWorkbenchReadiness({
          ready: false,
          store_corrupt: true,
          can_submit: false,
          reason: "WORKBENCH_STORE_CORRUPT",
        }),
      ),
    });
    renderWithI18n(<ImagesPage api={api} />);
    await screen.findByText(/本地会话存储无法读取/);
    expect(
      screen.getByRole("button", { name: "重建空工作台" }),
    ).toBeInTheDocument();
  });

  it("does not offer rebuild when storage needs a newer app", async () => {
    const api = createMockApi({
      getWorkbenchReadiness: vi.fn().mockResolvedValue(
        defaultWorkbenchReadiness({
          ready: false,
          store_incompatible: true,
          can_submit: false,
          reason: "WORKBENCH_STORE_INCOMPATIBLE",
        }),
      ),
    });
    renderWithI18n(<ImagesPage api={api} />);
    await screen.findByText(/请先更新应用/);
    expect(
      screen.queryByRole("button", { name: "重建空工作台" }),
    ).not.toBeInTheDocument();
    expect(api.createWorkbenchConversation).not.toHaveBeenCalled();
  });

  it("rejects a second submit while busy and keeps the draft", async () => {
    const api = createMockApi({
      getWorkbenchReadiness: vi.fn().mockResolvedValue(
        defaultWorkbenchReadiness({
          busy: true,
          can_submit: false,
          ready: true,
        }),
      ),
    });
    renderWithI18n(<ImagesPage api={api} />);
    fireEvent.click(await screen.findByRole("button", { name: "新建会话" }));
    fireEvent.change(await screen.findByLabelText("描述你想聊或想画的画面"), {
      target: { value: "还在输入" },
    });
    expect(screen.getByRole("button", { name: "停止" })).toBeInTheDocument();
    expect(screen.getByLabelText("描述你想聊或想画的画面")).toHaveValue(
      "还在输入",
    );
    expect(
      screen.queryByRole("button", { name: "发送" }),
    ).not.toBeInTheDocument();
    expect(api.sendWorkbench).not.toHaveBeenCalled();
  });

  it("cancels an in-flight operation", async () => {
    const api = createMockApi({
      getWorkbenchReadiness: vi
        .fn()
        .mockResolvedValue(
          defaultWorkbenchReadiness({ busy: true, can_submit: false }),
        ),
    });
    renderWithI18n(<ImagesPage api={api} />);
    fireEvent.click(await screen.findByRole("button", { name: "停止" }));
    await waitFor(() => expect(api.cancelWorkbench).toHaveBeenCalled());
  });

  it("does not render keys, paths, or base64", async () => {
    const api = await openFreshConversation();
    fireEvent.change(screen.getByLabelText("描述你想聊或想画的画面"), {
      target: { value: "画一只猫" },
    });
    fireEvent.click(screen.getByRole("button", { name: "出图" }));
    await screen.findByAltText("生成的图片");
    expect(document.body.textContent).not.toMatch(/sk-/);
    expect(document.body.textContent).not.toMatch(/\/Users\//);
    expect(document.body.textContent).not.toMatch(/base64/);
    expect(document.body.innerHTML).not.toContain("data:image");
    expect(screen.getByAltText("生成的图片").getAttribute("src")).toBe(
      `image-asset://localhost/${MOCK_WORKBENCH_ASSET_ID}`,
    );
    expect(api.sendWorkbench).toHaveBeenCalled();
  });

  it("shows the reference edit warning without changing the selected model", async () => {
    const api = await openFreshConversation();
    fireEvent.click(screen.getByRole("button", { name: "上传参考图" }));
    await screen.findByAltText("参考图");
    fireEvent.change(screen.getByLabelText("描述你想聊或想画的画面"), {
      target: { value: "改成水彩" },
    });
    expect(screen.getByText(/不会改切到 gpt-image-2/)).toBeInTheDocument();
    expect(screen.getByLabelText("生图模型")).toHaveValue(
      "ag/gemini-3.1-flash-image",
    );
    expect(api.setWorkbenchOptions).not.toHaveBeenCalled();
  });

  it("rejects an oversized pasted image without calling import", async () => {
    const api = await openFreshConversation();
    const file = new File([new Uint8Array([1, 2, 3])], "huge.png", {
      type: "image/png",
    });
    Object.defineProperty(file, "size", {
      value: MAX_WORKBENCH_IMPORT_BYTES + 1,
    });
    fireEvent.paste(screen.getByLabelText("描述你想聊或想画的画面"), {
      clipboardData: { files: [file] },
    });
    await screen.findByText("图片超过大小或像素上限，未上传。");
    expect(api.importWorkbenchBytes).not.toHaveBeenCalled();
  });

  it("ignores late chat deltas from another operation", async () => {
    let listener: ((event: WorkbenchChatDelta) => void) | undefined;
    const api = createMockApi();
    const original = api.subscribeWorkbenchChatDelta;
    api.subscribeWorkbenchChatDelta = vi.fn(async (next) => {
      listener = next;
      return original(next);
    });
    await openFreshConversation(api);
    fireEvent.change(screen.getByLabelText("描述你想聊或想画的画面"), {
      target: { value: "画一只猫" },
    });
    fireEvent.click(screen.getByRole("button", { name: "出图" }));
    await screen.findByAltText("生成的图片");
    const snapshot = await api.listWorkbench();
    const assistant = snapshot.messages.find(
      (item) => item.role === "assistant",
    );
    expect(assistant).toBeDefined();
    listener?.({
      operation_id: "op-stale",
      conversation_id: snapshot.selected_conversation_id ?? "",
      message_id: assistant?.id ?? "",
      visible_text: '迟到泄漏 ```image {"prompt":"secret"}',
    });
    expect(screen.queryByText(/迟到泄漏/)).not.toBeInTheDocument();
    expect(screen.queryByText(/```image/)).not.toBeInTheDocument();
    expect(screen.getByText("好，这就出图。")).toBeInTheDocument();
  });
});
