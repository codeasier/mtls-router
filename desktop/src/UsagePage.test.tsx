import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "./i18n";
import type { APIKeyUsage, DesktopApi } from "./ipc";
import { createMockApi } from "./test/api";
import { UsagePage } from "./UsagePage";

const multiModelUsage: APIKeyUsage = {
  period: "7d",
  as_of: "2026-08-28T00:00:00Z",
  summary: {
    requests: 20,
    prompt_tokens: 2000,
    completion_tokens: 400,
    cost: 2.5,
  },
  by_model: [
    {
      model: "gpt-5-sol",
      requests: 5,
      prompt_tokens: 500,
      completion_tokens: 100,
      cost: 0.5,
    },
    {
      model: "claude-sonnet",
      requests: 12,
      prompt_tokens: 1000,
      completion_tokens: 200,
      cost: 1.25,
    },
    {
      model: "gemini-flash",
      requests: 3,
      prompt_tokens: 500,
      completion_tokens: 100,
      cost: 0.75,
    },
  ],
};

function renderPage(api: DesktopApi = createMockApi()) {
  const onNavigateToApiKeys = vi.fn();
  render(
    <I18nProvider>
      <UsagePage api={api} onNavigateToApiKeys={onNavigateToApiKeys} />
    </I18nProvider>,
  );
  return { api, onNavigateToApiKeys };
}

function dataRows() {
  const table = screen.getByRole("table");
  return within(table).getAllByRole("row").slice(1);
}

function rowModel(row: HTMLElement) {
  return within(row).getByRole("rowheader").textContent;
}

describe("UsagePage", () => {
  it("loads usage after a saved key is present", async () => {
    const { api } = renderPage();

    await waitFor(() => expect(api.getAPIKeyUsage).toHaveBeenCalledWith("7d"));
    expect(await screen.findAllByText("claude-sonnet")).toHaveLength(2);
    expect(screen.getAllByText("12")).toHaveLength(2);
    expect(screen.getAllByText("US$1.25").length).toBeGreaterThan(0);
  });

  it("does not query usage when no key is saved", async () => {
    const api = createMockApi({
      getCredential: vi.fn().mockResolvedValue({
        present: false,
        fingerprint: "",
        saved_at: null,
      }),
    });
    renderPage(api);

    expect(
      await screen.findByText("保存 API key 后即可查看请求、token 与费用。"),
    ).toBeInTheDocument();
    expect(api.getAPIKeyUsage).not.toHaveBeenCalled();
  });

  it("navigates to the API key page from the empty state", async () => {
    const api = createMockApi({
      getCredential: vi.fn().mockResolvedValue({
        present: false,
        fingerprint: "",
        saved_at: null,
      }),
    });
    const { onNavigateToApiKeys } = renderPage(api);

    fireEvent.click(
      await screen.findByRole("button", { name: "前往 API 密钥" }),
    );

    expect(onNavigateToApiKeys).toHaveBeenCalledOnce();
  });

  it("refetches usage when the period changes", async () => {
    const api = createMockApi();
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
    expect(await screen.findAllByText("claude-sonnet")).toHaveLength(2);

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

    expect(await screen.findAllByText("claude-haiku")).toHaveLength(2);
    expect(screen.queryByText("claude-sonnet")).not.toBeInTheDocument();
  });

  it("shows a safe unavailable message when usage is missing", async () => {
    const api = createMockApi({
      getAPIKeyUsage: vi.fn().mockRejectedValue({ code: "USAGE_UNAVAILABLE" }),
    });
    renderPage(api);

    expect(
      await screen.findByText("当前服务尚未提供用量数据。"),
    ).toBeInTheDocument();
  });

  it("sorts model rows by requests, tokens, and cost", async () => {
    renderPage(
      createMockApi({
        getAPIKeyUsage: vi.fn().mockResolvedValue(multiModelUsage),
      }),
    );
    await screen.findAllByText("claude-sonnet");

    expect(dataRows().map(rowModel)).toEqual([
      "claude-sonnet",
      "gpt-5-sol",
      "gemini-flash",
    ]);

    fireEvent.click(screen.getByRole("button", { name: /请求/ }));
    expect(dataRows().map(rowModel)).toEqual([
      "gemini-flash",
      "gpt-5-sol",
      "claude-sonnet",
    ]);

    fireEvent.click(screen.getByRole("button", { name: /Token/ }));
    expect(dataRows().map(rowModel)).toEqual([
      "claude-sonnet",
      "gpt-5-sol",
      "gemini-flash",
    ]);

    fireEvent.click(screen.getByRole("button", { name: /费用/ }));
    expect(dataRows().map(rowModel)).toEqual([
      "claude-sonnet",
      "gemini-flash",
      "gpt-5-sol",
    ]);
  });

  it("filters model rows by multi-select and restores all models", async () => {
    renderPage(
      createMockApi({
        getAPIKeyUsage: vi.fn().mockResolvedValue(multiModelUsage),
      }),
    );
    await screen.findAllByText("claude-sonnet");

    const filterState = document.querySelector(".apikey-usage__filter-state");
    expect(filterState).toHaveTextContent("全部模型");
    expect(screen.getByRole("checkbox", { name: "全部模型" })).toBeChecked();

    fireEvent.click(screen.getByRole("checkbox", { name: "gpt-5-sol" }));
    expect(dataRows().map(rowModel)).toEqual(["claude-sonnet", "gemini-flash"]);
    expect(filterState).toHaveTextContent("已选 2 / 3");
    expect(screen.getByText("20")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("checkbox", { name: "claude-sonnet" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "gemini-flash" }));
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    expect(screen.getByText("所选模型没有用量记录。")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("checkbox", { name: "全部模型" }));
    expect(dataRows()).toHaveLength(3);
    expect(filterState).toHaveTextContent("全部模型");
  });

  it("exposes the active sort through aria-sort", async () => {
    renderPage(
      createMockApi({
        getAPIKeyUsage: vi.fn().mockResolvedValue(multiModelUsage),
      }),
    );
    await screen.findAllByText("claude-sonnet");

    const requestsHeader = screen.getByRole("columnheader", { name: /请求/ });
    expect(requestsHeader).toHaveAttribute("aria-sort", "descending");
    expect(screen.getByRole("columnheader", { name: /费用/ })).toHaveAttribute(
      "aria-sort",
      "none",
    );

    fireEvent.click(screen.getByRole("button", { name: /请求/ }));
    expect(requestsHeader).toHaveAttribute("aria-sort", "ascending");
  });
});
