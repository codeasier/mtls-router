import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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

  it("returns to the absent state after deletion", async () => {
    const api = createMockApi({
      getCredential: vi.fn().mockResolvedValue({
        present: true,
        fingerprint: "WXYZ",
        saved_at: "2026-07-26T00:00:00Z",
      }),
    });
    renderPage(api);

    fireEvent.click(
      await screen.findByRole("button", { name: "删除已保存密钥" }),
    );

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
});
