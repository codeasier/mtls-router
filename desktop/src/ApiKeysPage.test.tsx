import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ApiKeysPage } from "./ApiKeysPage";
import { I18nProvider } from "./i18n";
import { createMockApi } from "./test/api";

function renderPage(
  api = createMockApi({
    getCredential: vi.fn().mockResolvedValue({
      present: false,
      fingerprint: "",
      saved_at: null,
    }),
  }),
) {
  render(
    <I18nProvider>
      <ApiKeysPage api={api} />
    </I18nProvider>,
  );
  return api;
}

describe("ApiKeysPage", () => {
  it("shows the absent state without exposing a credential readback API", async () => {
    const api = renderPage();

    expect(await screen.findByText("尚未配置")).toBeInTheDocument();
    expect(
      screen.getByText("/safe/app-data/credentials.json"),
    ).toBeInTheDocument();
    expect("useCredential" in (api as unknown as Record<string, unknown>)).toBe(
      false,
    );
  });

  it("saves from the input, clears it, and renders only the summary", async () => {
    const api = renderPage();
    const input = await screen.findByLabelText("API key");
    await userEvent.type(input, "fixture-secret");

    fireEvent.click(screen.getByRole("button", { name: "保存密钥" }));

    await waitFor(() =>
      expect(api.saveCredential).toHaveBeenCalledWith("fixture-secret"),
    );
    expect(input).toHaveValue("");
    expect(screen.getByText("...ABCD")).toBeInTheDocument();
    expect(
      screen.queryByDisplayValue("fixture-secret"),
    ).not.toBeInTheDocument();
  });

  it("confirms deletion and returns to the absent state", async () => {
    const api = createMockApi({
      getCredential: vi.fn().mockResolvedValue({
        present: true,
        fingerprint: "WXYZ",
        saved_at: "2026-07-26T00:00:00Z",
      }),
    });
    renderPage(api);

    const deleteButton = await screen.findByRole("button", {
      name: "删除已保存密钥",
    });
    fireEvent.click(deleteButton);

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(deleteButton).toHaveFocus();
    fireEvent.click(deleteButton);
    fireEvent.click(screen.getByRole("button", { name: "确认删除" }));

    await waitFor(() => expect(api.deleteCredential).toHaveBeenCalledOnce());
    expect(screen.getByText("尚未配置")).toBeInTheDocument();
  });

  it("clears the plaintext input when saving fails", async () => {
    const api = createMockApi({
      saveCredential: vi.fn().mockRejectedValue({
        code: "CREDENTIAL_IO_ERROR",
      }),
    });
    renderPage(api);
    const input = await screen.findByLabelText("API key");
    await userEvent.type(input, "failed-secret");

    fireEvent.click(screen.getByRole("button", { name: "替换密钥" }));

    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(input).toHaveValue("");
  });

  it("shows saving progress and ignores a second submit", async () => {
    let finishSave!: (summary: {
      present: boolean;
      fingerprint: string;
      saved_at: string | null;
    }) => void;
    const saveCredential = vi.fn(
      () =>
        new Promise<{
          present: boolean;
          fingerprint: string;
          saved_at: string | null;
        }>((resolve) => {
          finishSave = resolve;
        }),
    );
    renderPage(
      createMockApi({
        getCredential: vi.fn().mockResolvedValue({
          present: false,
          fingerprint: "",
          saved_at: null,
        }),
        saveCredential,
      }),
    );
    const input = await screen.findByLabelText("API key");
    await userEvent.type(input, "fixture-secret");

    fireEvent.click(screen.getByRole("button", { name: "保存密钥" }));

    const saving = await screen.findByRole("button", { name: "正在保存..." });
    expect(saving).toBeDisabled();
    fireEvent.click(saving);
    expect(saveCredential).toHaveBeenCalledOnce();

    await act(async () => {
      finishSave({
        present: true,
        fingerprint: "ABCD",
        saved_at: "2026-07-26T00:00:00Z",
      });
    });
    expect(await screen.findByText("已配置")).toBeInTheDocument();
  });

  it("shows deleting progress after confirmation", async () => {
    let finishDelete!: (summary: {
      present: boolean;
      fingerprint: string;
      saved_at: string | null;
    }) => void;
    const deleteCredential = vi.fn(
      () =>
        new Promise<{
          present: boolean;
          fingerprint: string;
          saved_at: string | null;
        }>((resolve) => {
          finishDelete = resolve;
        }),
    );
    renderPage(
      createMockApi({
        getCredential: vi.fn().mockResolvedValue({
          present: true,
          fingerprint: "WXYZ",
          saved_at: "2026-07-26T00:00:00Z",
        }),
        deleteCredential,
      }),
    );
    fireEvent.click(
      await screen.findByRole("button", { name: "删除已保存密钥" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "确认删除" }));

    const deleting = await screen.findByRole("button", { name: "正在删除..." });
    expect(deleting).toBeDisabled();
    expect(deleteCredential).toHaveBeenCalledOnce();

    await act(async () => {
      finishDelete({
        present: false,
        fingerprint: "",
        saved_at: null,
      });
    });
    expect(await screen.findByText("尚未配置")).toBeInTheDocument();
  });
});
