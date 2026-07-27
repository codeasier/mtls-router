import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "./App";
import { createMockApi } from "./test/api";

describe("App navigation", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });
  it("opens the production Router page and navigates from its Agent action", async () => {
    render(<App api={createMockApi()} />);

    expect(screen.getByText("CR")).toBeInTheDocument();
    expect(await screen.findByText("路由未启动")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "前往 Agent 配置" }));

    expect(
      screen.getByRole("heading", { name: "Agent 配置" }),
    ).toBeInTheDocument();
    expect(screen.getByText("模型配置工作台")).toBeInTheDocument();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Agent 检测失败",
    );
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

  it("opens API key management from the main navigation", async () => {
    render(
      <App
        api={createMockApi({
          getCredential: vi.fn().mockResolvedValue({
            present: false,
            fingerprint: "",
            saved_at: null,
          }),
        })}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "API 密钥" }));

    expect(
      screen.getByRole("heading", { name: "API 密钥" }),
    ).toBeInTheDocument();
    expect(await screen.findByText("尚未配置")).toBeInTheDocument();
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

  it("collapses the sidebar to icons and persists the preference", async () => {
    const { container, unmount } = render(<App api={createMockApi()} />);
    const frame = container.querySelector(".app-frame");
    expect(frame).toHaveAttribute("data-sidebar", "expanded");

    fireEvent.click(screen.getByRole("button", { name: "收起侧栏" }));

    expect(frame).toHaveAttribute("data-sidebar", "collapsed");
    expect(window.localStorage.getItem("mtls-router.sidebar.collapsed")).toBe(
      "1",
    );
    unmount();

    const remounted = render(<App api={createMockApi()} />);
    expect(remounted.container.querySelector(".app-frame")).toHaveAttribute(
      "data-sidebar",
      "collapsed",
    );

    fireEvent.click(screen.getByRole("button", { name: "展开侧栏" }));
    expect(remounted.container.querySelector(".app-frame")).toHaveAttribute(
      "data-sidebar",
      "expanded",
    );
    expect(window.localStorage.getItem("mtls-router.sidebar.collapsed")).toBe(
      "0",
    );
  });
});
