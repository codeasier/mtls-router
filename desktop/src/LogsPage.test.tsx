import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  COMMANDS,
  createDesktopApi,
  MAX_LOG_LINES,
  type InvokeFn,
} from "./ipc";
import { LogsPage } from "./LogsPage";
import { createMockApi } from "./test/api";
import { renderWithI18n } from "./test/render";

const writeText = vi.fn().mockResolvedValue(undefined);

beforeEach(() => {
  writeText.mockClear();
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText },
  });
});

describe("LogsPage", () => {
  it("requests and renders only the bounded recent log range", async () => {
    const api = createMockApi({
      getRouterLogs: vi.fn().mockResolvedValue({
        lines: ["first safe line", "second safe line"],
      }),
    });
    renderWithI18n(<LogsPage api={api} />);

    expect(await screen.findByText("first safe line")).toBeInTheDocument();
    expect(screen.getByText("second safe line")).toBeInTheDocument();
    expect(api.getRouterLogs).toHaveBeenCalledWith(MAX_LOG_LINES);
    expect(screen.getByText(/不读取完整文件/)).toBeInTheDocument();
  });

  it("opens the trusted log location through the typed API", async () => {
    const api = createMockApi();
    renderWithI18n(<LogsPage api={api} />);
    await screen.findByText("暂无路由日志");

    fireEvent.click(screen.getByRole("button", { name: "打开日志位置" }));

    await waitFor(() => expect(api.openLogLocation).toHaveBeenCalledOnce());
    expect(screen.getByText("已打开日志位置。")).toBeInTheDocument();
  });

  it("copies only the sanitized diagnostic summary", async () => {
    const api = createMockApi({
      collectDiagnostics: vi.fn().mockResolvedValue({
        summary: "router=healthy credentials=[REDACTED]",
      }),
    });
    renderWithI18n(<LogsPage api={api} />);
    await screen.findByText("暂无路由日志");

    fireEvent.click(screen.getByRole("button", { name: "复制诊断摘要" }));

    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith(
        "router=healthy credentials=[REDACTED]",
      ),
    );
    expect(screen.getByText(/已复制安全过滤后的诊断摘要/)).toBeInTheDocument();
  });

  it("shows an export log bundle button next to copy", async () => {
    const api = createMockApi();
    renderWithI18n(<LogsPage api={api} />);
    await screen.findByText("暂无路由日志");

    expect(
      screen.getByRole("button", { name: "导出日志包" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "复制诊断摘要" }),
    ).toBeInTheDocument();
  });

  it("falls back to the local diagnostic snapshot when collect fails", async () => {
    const api = createMockApi({
      collectDiagnostics: vi.fn().mockRejectedValue(new Error("collect failed")),
      getDiagnosticSnapshot: vi.fn().mockResolvedValue({
        schema_version: 1,
        captured_at: "2026-09-03T10:00:00Z",
        classification: "not_started",
        desktop: "0.1.0",
        manager: "0.1.0",
        management_protocol: "4",
        deployment_id: "dev",
        target: "aarch64-apple-darwin",
        health_stale: true,
        summary: "classification=not_started",
      }),
    });
    renderWithI18n(<LogsPage api={api} />);
    await screen.findByText("暂无路由日志");

    fireEvent.click(screen.getByRole("button", { name: "复制诊断摘要" }));

    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith("classification=not_started"),
    );
    expect(api.getDiagnosticSnapshot).toHaveBeenCalledOnce();
    expect(screen.getByText("已复制本地诊断快照。")).toBeInTheDocument();
  });

  it("does not render or copy a rejected secret canary", async () => {
    const rejectedCanary = "ui-test-canary-value";
    const api = createMockApi({
      getRouterLogs: vi.fn().mockRejectedValue(new Error(rejectedCanary)),
      collectDiagnostics: vi.fn().mockRejectedValue(new Error(rejectedCanary)),
      getDiagnosticSnapshot: vi
        .fn()
        .mockRejectedValue(new Error(rejectedCanary)),
    });
    renderWithI18n(<LogsPage api={api} />);
    expect(await screen.findByText("无法读取最近日志。")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "复制诊断摘要" }));

    expect(await screen.findByText("无法复制诊断摘要。")).toBeInTheDocument();
    expect(document.body).not.toHaveTextContent(rejectedCanary);
    expect(writeText).not.toHaveBeenCalled();
  });

  it("sanitizes invoke payloads before rendering or copying", async () => {
    const secretCanary = "sk-uiRenderCanary123456";
    const invoke = vi.fn(async (command: string) => {
      if (command === COMMANDS.routerLogs) {
        return { lines: [`Authorization: Bearer ${secretCanary}`] };
      }
      if (command === COMMANDS.diagnosticsCollect) {
        return { summary: `api_key=${secretCanary}` };
      }
      return undefined;
    });
    renderWithI18n(<LogsPage api={createDesktopApi(invoke as InvokeFn)} />);

    expect(await screen.findByText(/REDACTED/)).toBeInTheDocument();
    expect(document.body).not.toHaveTextContent(secretCanary);
    fireEvent.click(screen.getByRole("button", { name: "复制诊断摘要" }));

    await waitFor(() => expect(writeText).toHaveBeenCalledOnce());
    expect(writeText.mock.calls[0]?.[0]).not.toContain(secretCanary);
    expect(writeText.mock.calls[0]?.[0]).toContain("[REDACTED]");
  });
});
