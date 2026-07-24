import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { App } from "./App";
import { createMockApi } from "./test/api";

describe("App navigation", () => {
  it("opens the production Router page and navigates from its Agent action", async () => {
    render(<App api={createMockApi()} />);

    expect(screen.getByText("CR")).toBeInTheDocument();
    expect(await screen.findByText("路由未启动")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "前往 Agent 配置" }));

    expect(
      screen.getByRole("heading", { name: "Agent 配置" }),
    ).toBeInTheDocument();
    expect(screen.getByText("模型配置工作台")).toBeInTheDocument();
    expect(await screen.findAllByText("未返回检测结果")).toHaveLength(3);
  });

  it("opens runtime logs from startup failure diagnostics", async () => {
    const api = createMockApi({
      getRouterStatus: vi.fn().mockResolvedValue({
        state: "start_failed",
        last_error: "router startup failed",
        recent_logs: ["startup diagnostic"],
      }),
      getRouterLogs: vi.fn().mockResolvedValue({
        lines: ["full runtime log line"],
      }),
    });
    render(<App api={api} />);

    fireEvent.click(
      await screen.findByRole("button", { name: "查看运行日志" }),
    );

    expect(
      screen.getByRole("heading", { name: "运行日志" }),
    ).toBeInTheDocument();
    expect(
      await screen.findByText("full runtime log line"),
    ).toBeInTheDocument();
  });

  it("keeps Settings available after Agent navigation is integrated", async () => {
    const api = createMockApi();
    render(<App api={api} />);
    await screen.findByText("路由未启动");

    fireEvent.click(screen.getByRole("button", { name: /系统设置/ }));

    expect(
      screen.getByRole("heading", { name: "系统设置" }),
    ).toBeInTheDocument();
    expect(screen.getByText("桌面控制面板")).toBeInTheDocument();
  });

  it("uses document visibility only to coordinate native polling", () => {
    const api = createMockApi();
    render(<App api={api} />);

    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "hidden",
    });
    fireEvent(document, new Event("visibilitychange"));

    expect(vi.mocked(api.setWindowVisibility)).toHaveBeenLastCalledWith(false);
    expect(api.destroyAgentModelFlow).not.toHaveBeenCalled();
  });
});
