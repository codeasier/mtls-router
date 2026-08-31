import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ApiKeysPage } from "./ApiKeysPage";
import { I18nProvider } from "./i18n";
import type { APIKeyUsage } from "./ipc";
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

  it("loads usage after a saved key is present", async () => {
    const api = createMockApi({
      getCredential: vi.fn().mockResolvedValue({
        present: true,
        fingerprint: "WXYZ",
        saved_at: "2026-07-26T00:00:00Z",
      }),
    });
    renderPage(api);

    expect(
      await screen.findByRole("heading", { name: "用量" }),
    ).toBeInTheDocument();
    await waitFor(() => expect(api.getAPIKeyUsage).toHaveBeenCalledWith("7d"));
    expect(screen.getByText("claude-sonnet")).toBeInTheDocument();
    expect(screen.getAllByText("12")).toHaveLength(2);
    expect(screen.getAllByText("US$1.25").length).toBeGreaterThan(0);
  });

  it("does not query usage when no key is saved", async () => {
    const api = renderPage();
    expect(await screen.findByText("尚未配置")).toBeInTheDocument();
    expect(
      screen.getByText("保存 API key 后即可查看请求、token 与费用。"),
    ).toBeInTheDocument();
    expect(api.getAPIKeyUsage).not.toHaveBeenCalled();
  });

  it("refetches usage when the period changes", async () => {
    const api = createMockApi({
      getCredential: vi.fn().mockResolvedValue({
        present: true,
        fingerprint: "WXYZ",
        saved_at: "2026-07-26T00:00:00Z",
      }),
    });
    renderPage(api);
    await waitFor(() => expect(api.getAPIKeyUsage).toHaveBeenCalledWith("7d"));

    fireEvent.click(screen.getByRole("tab", { name: "今天" }));

    await waitFor(() =>
      expect(api.getAPIKeyUsage).toHaveBeenCalledWith("today"),
    );
  });

  it("clears the previous snapshot while a new period loads", async () => {
    let resolveToday: (value: APIKeyUsage) => void = () => {};
    const api = createMockApi({
      getCredential: vi.fn().mockResolvedValue({
        present: true,
        fingerprint: "WXYZ",
        saved_at: "2026-07-26T00:00:00Z",
      }),
      getAPIKeyUsage: vi.fn().mockImplementation((period: string) => {
        if (period === "today") {
          return new Promise((resolve) => {
            resolveToday = resolve;
          });
        }
        return Promise.resolve({
          period: "7d",
          summary: {
            requests: 12,
            prompt_tokens: 1000,
            completion_tokens: 200,
            cost: 1.25,
          },
          by_model: [
            {
              model: "claude-sonnet",
              requests: 12,
              prompt_tokens: 1000,
              completion_tokens: 200,
              cost: 1.25,
            },
          ],
        });
      }),
    });
    renderPage(api);
    expect(await screen.findByText("claude-sonnet")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: "今天" }));

    await waitFor(() => {
      expect(screen.queryByText("claude-sonnet")).not.toBeInTheDocument();
      expect(screen.getByText("正在读取用量")).toBeInTheDocument();
    });

    resolveToday({
      period: "today",
      summary: {
        requests: 3,
        prompt_tokens: 10,
        completion_tokens: 4,
        cost: 0.2,
      },
      by_model: [
        {
          model: "claude-haiku",
          requests: 3,
          prompt_tokens: 10,
          completion_tokens: 4,
          cost: 0.2,
        },
      ],
    });

    expect(await screen.findByText("claude-haiku")).toBeInTheDocument();
    expect(screen.queryByText("claude-sonnet")).not.toBeInTheDocument();
  });

  it("shows a safe unavailable message when usage is missing", async () => {
    const api = createMockApi({
      getCredential: vi.fn().mockResolvedValue({
        present: true,
        fingerprint: "WXYZ",
        saved_at: "2026-07-26T00:00:00Z",
      }),
      getAPIKeyUsage: vi.fn().mockRejectedValue({ code: "USAGE_UNAVAILABLE" }),
    });
    renderPage(api);

    expect(
      await screen.findByText("当前服务尚未提供用量数据。"),
    ).toBeInTheDocument();
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
