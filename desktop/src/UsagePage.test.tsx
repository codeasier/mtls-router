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

  it("shows an error when reading the credential fails", async () => {
    const api = createMockApi({
      getCredential: vi.fn().mockRejectedValue(new Error("IPC unavailable")),
    });
    renderPage(api);

    expect(
      await screen.findByText("无法读取用量，请稍后重试。"),
    ).toBeInTheDocument();
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

    fireEvent.click(screen.getByRole("tab", { name: /今天/ }));

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

    fireEvent.click(screen.getByRole("tab", { name: /今天/ }));

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

  it("groups period tabs and does not expose colliding week or month tokens", async () => {
    renderPage();
    await waitFor(() =>
      expect(screen.getByRole("tab", { name: /今天/ })).toBeInTheDocument(),
    );

    expect(screen.getByText("滚动窗口")).toBeInTheDocument();
    expect(screen.getByText("日历窗口")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "近 1 小时" })).toBeInTheDocument();
    expect(
      screen.getByRole("tab", { name: "本周（周一 08:00 起）" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "week" })).not.toBeInTheDocument();
    expect(
      screen.queryByRole("tab", { name: "month" }),
    ).not.toBeInTheDocument();
  });

  it("hides quota blocks when quota and quotas are absent or empty", async () => {
    renderPage(
      createMockApi({
        getAPIKeyUsage: vi.fn().mockResolvedValue({
          period: "7d",
          summary: multiModelUsage.summary,
          quotas: [],
          by_model: multiModelUsage.by_model,
        }),
      }),
    );
    await screen.findAllByText("claude-sonnet");

    expect(screen.queryByText("配额")).not.toBeInTheDocument();
    expect(screen.queryByText("供应商限额")).not.toBeInTheDocument();
  });

  it("shows an overview quota at the top when there are no provider limits", async () => {
    renderPage();
    const quota = await screen.findByRole("region", { name: "配额" });
    expect(document.querySelector(".usage-panel")?.firstElementChild).toBe(
      quota,
    );
    expect(screen.getAllByRole("progressbar")).toHaveLength(1);
    expect(screen.queryByText("供应商限额")).not.toBeInTheDocument();
  });

  it("renders provider limits without an overview bar when only quotas are present", async () => {
    renderPage(
      createMockApi({
        getAPIKeyUsage: vi.fn().mockResolvedValue({
          period: "1h",
          summary: multiModelUsage.summary,
          quotas: [
            {
              provider: "*",
              period: "week",
              used: 1.25,
              limit: 10,
              unit: "usd",
              resets_at: "2026-09-14T00:00:00Z",
            },
            {
              provider: "codex",
              period: "day",
              used: 0.4,
              limit: 2,
              unit: "usd",
              resets_at: "2026-09-08T00:00:00Z",
            },
          ],
          by_model: multiModelUsage.by_model,
        }),
      }),
    );
    await screen.findAllByText("claude-sonnet");

    expect(screen.queryByText("配额")).not.toBeInTheDocument();
    expect(screen.getByText("供应商限额")).toBeInTheDocument();
    expect(screen.getByText("全部供应商 · 每周")).toBeInTheDocument();
    expect(screen.getByText("codex · 每日")).toBeInTheDocument();
    expect(
      screen.getByText("US$1.25 / US$10.00 · 剩余 US$8.75"),
    ).toBeInTheDocument();
    expect(screen.getByText("12.5%")).toBeInTheDocument();
    expect(
      screen.getByText("US$0.40 / US$2.00 · 剩余 US$1.60"),
    ).toBeInTheDocument();
    expect(screen.getByText("20%")).toBeInTheDocument();
    expect(screen.getAllByText(/重置时间（北京时间）/).length).toBeGreaterThan(
      0,
    );
    expect(screen.getAllByText(/08:00/).length).toBeGreaterThan(0);
  });

  it("rounds quota amounts to two decimals and shows usage percent", async () => {
    renderPage(
      createMockApi({
        getAPIKeyUsage: vi.fn().mockResolvedValue({
          period: "7d",
          summary: multiModelUsage.summary,
          quotas: [
            {
              provider: "codexex",
              period: "week",
              used: 200.26667,
              limit: 350,
              unit: "usd",
              resets_at: "2026-09-14T00:00:00Z",
            },
          ],
          by_model: multiModelUsage.by_model,
        }),
      }),
    );
    await screen.findAllByText("claude-sonnet");

    expect(screen.getByText("codexex · 每周")).toBeInTheDocument();
    expect(
      screen.getByText("US$200.27 / US$350.00 · 剩余 US$149.73"),
    ).toBeInTheDocument();
    expect(screen.queryByText(/200\.26667/)).not.toBeInTheDocument();
    expect(screen.getByText("57.2%")).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute(
      "aria-valuetext",
      "57.2%",
    );
  });

  it("shows provider limits only when both quota formats are returned", async () => {
    const api = createMockApi({
      getAPIKeyUsage: vi.fn().mockImplementation((period: string) => {
        if (period === "1h") {
          return Promise.resolve({
            period: "1h",
            summary: multiModelUsage.summary,
            quotas: [
              {
                provider: "*",
                period: "week",
                used: 1.25,
                limit: 10,
                unit: "usd",
                resets_at: "2026-09-14T00:00:00Z",
              },
            ],
            by_model: multiModelUsage.by_model,
          });
        }
        return Promise.resolve({
          period: "7d",
          summary: multiModelUsage.summary,
          quota: {
            used: 1.25,
            limit: 10,
            unit: "usd",
            resets_at: "2026-09-14T00:00:00Z",
          },
          quotas: [
            {
              provider: "*",
              period: "week",
              used: 1.25,
              limit: 10,
              unit: "usd",
              resets_at: "2026-09-14T00:00:00Z",
            },
          ],
          by_model: multiModelUsage.by_model,
        });
      }),
    });
    renderPage(api);
    const limits = await screen.findByRole("region", {
      name: "供应商限额",
    });
    expect(document.querySelector(".usage-panel")?.firstElementChild).toBe(
      limits,
    );
    expect(screen.queryByText("配额")).not.toBeInTheDocument();
    expect(screen.getAllByRole("progressbar")).toHaveLength(1);

    fireEvent.click(screen.getByRole("tab", { name: "近 1 小时" }));

    await waitFor(() => expect(api.getAPIKeyUsage).toHaveBeenCalledWith("1h"));
    expect(await screen.findByText("供应商限额")).toBeInTheDocument();
    expect(screen.queryByText("配额")).not.toBeInTheDocument();
    expect(screen.getByText("全部供应商 · 每周")).toBeInTheDocument();
    expect(screen.getAllByRole("progressbar")).toHaveLength(1);
  });

  it("paginates the model table at ten rows and keeps the table frame fixed", async () => {
    const models = Array.from({ length: 12 }, (_, index) => ({
      model: `model-${String(index + 1).padStart(2, "0")}`,
      requests: index + 1,
      prompt_tokens: (index + 1) * 100,
      completion_tokens: 0,
      cost: (index + 1) * 0.1,
    }));
    const api = createMockApi({
      getAPIKeyUsage: vi.fn().mockResolvedValue({
        period: "7d",
        summary: multiModelUsage.summary,
        quotas: [],
        by_model: models,
      }),
    });
    renderPage(api);
    const table = await screen.findByRole("table");
    expect(
      within(table).getByRole("rowheader", { name: "model-12" }),
    ).toBeInTheDocument();
    expect(within(table).getAllByRole("rowheader")).toHaveLength(10);
    expect(table.querySelectorAll("tbody tr")).toHaveLength(10);
    expect(screen.getByText("第 1 / 2 页")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "上一页" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "下一页" })).not.toBeDisabled();
    expect(
      within(table).queryByRole("rowheader", { name: "model-01" }),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "下一页" }));

    expect(screen.getByText("第 2 / 2 页")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "下一页" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "上一页" })).not.toBeDisabled();
    expect(
      within(table).getByRole("rowheader", { name: "model-02" }),
    ).toBeInTheDocument();
    expect(
      within(table).getByRole("rowheader", { name: "model-01" }),
    ).toBeInTheDocument();
    expect(table.querySelectorAll("tbody tr")).toHaveLength(10);

    fireEvent.click(screen.getByRole("button", { name: "上一页" }));
    expect(screen.getByText("第 1 / 2 页")).toBeInTheDocument();
    expect(
      within(table).getByRole("rowheader", { name: "model-12" }),
    ).toBeInTheDocument();
  });

  it("keeps the table frame fixed when the filter narrows to a few rows", async () => {
    const models = Array.from({ length: 12 }, (_, index) => ({
      model: `model-${String(index + 1).padStart(2, "0")}`,
      requests: index + 1,
      prompt_tokens: (index + 1) * 100,
      completion_tokens: 0,
      cost: (index + 1) * 0.1,
    }));
    renderPage(
      createMockApi({
        getAPIKeyUsage: vi.fn().mockResolvedValue({
          period: "7d",
          summary: multiModelUsage.summary,
          quotas: [],
          by_model: models,
        }),
      }),
    );
    await screen.findByRole("table");

    for (const name of [
      "model-01",
      "model-02",
      "model-03",
      "model-04",
      "model-05",
      "model-06",
      "model-07",
      "model-08",
      "model-09",
      "model-10",
      "model-11",
    ]) {
      fireEvent.click(screen.getByRole("checkbox", { name }));
    }

    const table = screen.getByRole("table");
    expect(within(table).getAllByRole("rowheader")).toHaveLength(1);
    expect(table.querySelectorAll("tbody tr")).toHaveLength(10);
    expect(screen.getByText("第 1 / 1 页")).toBeInTheDocument();
  });

  it("restarts on page one when the filter changes the row set", async () => {
    const models = Array.from({ length: 12 }, (_, index) => ({
      model: `model-${String(index + 1).padStart(2, "0")}`,
      requests: index + 1,
      prompt_tokens: (index + 1) * 100,
      completion_tokens: 0,
      cost: (index + 1) * 0.1,
    }));
    renderPage(
      createMockApi({
        getAPIKeyUsage: vi.fn().mockResolvedValue({
          period: "7d",
          summary: multiModelUsage.summary,
          quotas: [],
          by_model: models,
        }),
      }),
    );
    await screen.findByRole("table");

    fireEvent.click(screen.getByRole("button", { name: "下一页" }));
    expect(screen.getByText("第 2 / 2 页")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("checkbox", { name: "model-12" }));
    const table = screen.getByRole("table");
    expect(screen.getByText("第 1 / 2 页")).toBeInTheDocument();
    expect(
      within(table).getByRole("rowheader", { name: "model-11" }),
    ).toBeInTheDocument();
  });

  it("shows token counts with a one-decimal million suffix", async () => {
    renderPage(
      createMockApi({
        getAPIKeyUsage: vi.fn().mockResolvedValue({
          period: "7d",
          summary: {
            requests: 1,
            prompt_tokens: 380_084_323,
            completion_tokens: 2_491_384,
            cost: 1,
          },
          quotas: [],
          by_model: [
            {
              model: "gpt-5",
              requests: 1,
              prompt_tokens: 100,
              completion_tokens: 50,
              cost: 1,
            },
          ],
        }),
      }),
    );

    expect(await screen.findByText("382,575,707(382.6M)")).toBeInTheDocument();
    expect(screen.getByText(/380,084,323\(380\.1M\)/)).toBeInTheDocument();
    expect(screen.getByText(/2,491,384\(2\.5M\)/)).toBeInTheDocument();
    expect(screen.getByText("150")).toBeInTheDocument();
  });

  it("renders provider limits as an independent card that survives period reloads", async () => {
    let resolveNext: (value: APIKeyUsage) => void = () => {};
    const quotas = [
      {
        provider: "codex",
        period: "week" as const,
        used: 217.4,
        limit: 500,
        unit: "usd" as const,
        resets_at: "2026-09-28T00:00:00Z",
      },
    ];
    const api = createMockApi({
      getAPIKeyUsage: vi.fn().mockImplementation((period: string) => {
        if (period === "1h") {
          return new Promise<APIKeyUsage>((resolve) => {
            resolveNext = resolve;
          });
        }
        return Promise.resolve({
          period: "7d",
          summary: multiModelUsage.summary,
          quotas,
          by_model: multiModelUsage.by_model,
        });
      }),
    });
    renderPage(api);
    await screen.findByText("codex · 每周");

    const limitsCard = document.querySelector(".apikey-usage--limits");
    expect(limitsCard).not.toBeNull();
    const panel = document.querySelector(".usage-panel");
    expect(panel?.firstElementChild).toBe(limitsCard);
    expect(panel?.lastElementChild).toHaveClass("apikey-usage");
    expect(panel?.lastElementChild).not.toHaveClass("apikey-usage--limits");
    expect(limitsCard).toHaveTextContent("供应商限额");
    expect(limitsCard).toHaveTextContent(
      "供应商限额按各自窗口重置，不随用量周期切换。",
    );

    fireEvent.click(screen.getByRole("tab", { name: "近 1 小时" }));

    await waitFor(() => expect(api.getAPIKeyUsage).toHaveBeenCalledWith("1h"));
    expect(screen.getByText("正在读取用量")).toBeInTheDocument();
    expect(screen.getByText("供应商限额")).toBeInTheDocument();
    expect(screen.getByText("codex · 每周")).toBeInTheDocument();

    resolveNext({
      period: "1h",
      summary: multiModelUsage.summary,
      quotas,
      by_model: multiModelUsage.by_model,
    });
    expect(await screen.findByText("codex · 每周")).toBeInTheDocument();
  });

  it("colors quota bars by healthy, warning, and danger thresholds", async () => {
    renderPage(
      createMockApi({
        getAPIKeyUsage: vi.fn().mockResolvedValue({
          period: "7d",
          summary: multiModelUsage.summary,
          quotas: [
            {
              provider: "low",
              period: "day",
              used: 1.25,
              limit: 10,
              unit: "usd",
              resets_at: "2026-09-14T00:00:00Z",
            },
            {
              provider: "mid",
              period: "week",
              used: 4.35,
              limit: 10,
              unit: "usd",
              resets_at: "2026-09-14T00:00:00Z",
            },
            {
              provider: "high",
              period: "month",
              used: 8.5,
              limit: 10,
              unit: "usd",
              resets_at: "2026-09-14T00:00:00Z",
            },
          ],
          by_model: multiModelUsage.by_model,
        }),
      }),
    );
    await screen.findByText("供应商限额");

    const bars = screen.getAllByRole("progressbar");
    expect(bars).toHaveLength(3);
    expect(bars[0]).toHaveAttribute("data-tone", "healthy");
    expect(bars[1]).toHaveAttribute("data-tone", "warning");
    expect(bars[2]).toHaveAttribute("data-tone", "danger");
  });
});
