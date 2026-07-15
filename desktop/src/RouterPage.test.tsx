import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { PollSnapshot, RouterHealth, RouterStatus } from "./ipc";
import { MAX_FAILURE_LOG_LINES, RouterPage } from "./RouterPage";
import { createMockApi } from "./test/api";
import { renderWithI18n } from "./test/render";

const freshHealthy: RouterHealth = {
  status: "ok",
  checked_at: new Date().toISOString(),
};

describe("RouterPage states", () => {
  it.each<{
    name: string;
    status: RouterStatus;
    health?: RouterHealth;
    heading: string;
    process: string;
    upstream: string;
  }>([
    {
      name: "not started",
      status: { state: "absent" },
      heading: "路由未启动",
      process: "未启动",
      upstream: "不可用",
    },
    {
      name: "starting",
      status: { state: "starting", owner: "desktop" },
      heading: "正在启动路由",
      process: "启动中",
      upstream: "不可用",
    },
    {
      name: "running and healthy",
      status: { state: "desktop_owned", owner: "desktop" },
      health: freshHealthy,
      heading: "路由运行正常",
      process: "运行中",
      upstream: "健康",
    },
    {
      name: "running with unavailable upstream",
      status: { state: "desktop_owned", owner: "desktop" },
      health: { status: "degraded", checked_at: new Date().toISOString() },
      heading: "上游连接不可用",
      process: "降级运行",
      upstream: "上游不可用",
    },
    {
      name: "external router running",
      status: { state: "external_compatible", owner: "cli" },
      health: freshHealthy,
      heading: "外部路由正在运行",
      process: "外部托管",
      upstream: "健康",
    },
    {
      name: "port occupied",
      status: { state: "unknown_occupant" },
      heading: "端口已被占用",
      process: "端口冲突",
      upstream: "不可用",
    },
    {
      name: "start failed",
      status: { state: "start_failed" },
      heading: "路由启动失败",
      process: "需要处理",
      upstream: "不可用",
    },
    {
      name: "stopping",
      status: { state: "stopping", owner: "desktop" },
      heading: "正在停止路由",
      process: "停止中",
      upstream: "不可用",
    },
  ])(
    "renders $name",
    async ({ status, health, heading, process, upstream }) => {
      const api = createMockApi({
        getRouterStatus: vi.fn().mockResolvedValue(status),
        retryRouterHealth: vi.fn().mockResolvedValue(health ?? freshHealthy),
      });

      renderWithI18n(<RouterPage api={api} onNavigateToAgents={vi.fn()} />);

      expect(
        await screen.findByRole("heading", { name: heading }),
      ).toBeInTheDocument();
      const readouts = screen
        .getAllByRole("definition")
        .map((element) => element.textContent);
      expect(readouts).toContain(process);
      expect(readouts).toContain(upstream);
    },
  );

  it("marks health older than thirty seconds stale while keeping the process available", async () => {
    const api = createMockApi({
      getRouterStatus: vi.fn().mockResolvedValue({
        state: "desktop_owned",
        owner: "desktop",
      }),
      retryRouterHealth: vi.fn().mockResolvedValue({
        status: "ok",
        checked_at: new Date(Date.now() - 31_000).toISOString(),
      }),
    });

    renderWithI18n(<RouterPage api={api} onNavigateToAgents={vi.fn()} />);

    expect(await screen.findByText("结果已过期")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "上游连接不可用" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "停止路由" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "重试健康检查" })).toBeEnabled();
  });

  it("rerenders when fresh health crosses the thirty second stale boundary", async () => {
    vi.useFakeTimers();
    const api = createMockApi({
      getPollSnapshot: vi.fn().mockResolvedValue({
        revision: 1,
        status: { state: "desktop_owned", owner: "desktop" },
        health: { status: "ok", checked_at: new Date().toISOString() },
      }),
    });

    renderWithI18n(<RouterPage api={api} onNavigateToAgents={vi.fn()} />);
    await act(async () => undefined);
    expect(screen.getByText("健康")).toBeInTheDocument();

    await act(async () => vi.advanceTimersByTimeAsync(30_001));

    expect(screen.getByText("结果已过期")).toBeInTheDocument();
  });

  it("applies newer scheduler events and discards stale generations", async () => {
    let observer: ((snapshot: PollSnapshot) => void) | undefined;
    const api = createMockApi({
      getPollSnapshot: vi.fn().mockResolvedValue({
        revision: 2,
        status: { state: "absent" },
      }),
      subscribePollSnapshots: vi.fn(async (listener) => {
        observer = listener;
        return () => undefined;
      }),
    });
    renderWithI18n(<RouterPage api={api} onNavigateToAgents={vi.fn()} />);
    expect(await screen.findByText("路由未启动")).toBeInTheDocument();

    act(() =>
      observer?.({
        revision: 4,
        status: { state: "desktop_owned", owner: "desktop" },
        health: { status: "ok", checked_at: new Date().toISOString() },
      }),
    );
    expect(screen.getByText("路由运行正常")).toBeInTheDocument();

    act(() => observer?.({ revision: 3, status: { state: "absent" } }));
    expect(screen.getByText("路由运行正常")).toBeInTheDocument();
  });

  it("clears a router status alert after a newer successful snapshot", async () => {
    let observer: ((snapshot: PollSnapshot) => void) | undefined;
    const api = createMockApi({
      getPollSnapshot: vi.fn().mockResolvedValue({
        revision: 1,
        status: { state: "desktop_owned", owner: "desktop" },
        health: freshHealthy,
      }),
      subscribePollSnapshots: vi.fn(async (listener) => {
        observer = listener;
        return () => undefined;
      }),
    });
    renderWithI18n(<RouterPage api={api} onNavigateToAgents={vi.fn()} />);
    expect(
      await screen.findByRole("heading", { name: "路由运行正常" }),
    ).toBeInTheDocument();

    act(() =>
      observer?.({ revision: 2, status_error: { code: "MANAGER_FAILED" } }),
    );
    expect(
      await screen.findByText(
        "无法读取路由状态。请重新启动桌面应用或查看日志。",
      ),
    ).toBeInTheDocument();

    act(() =>
      observer?.({
        revision: 3,
        status: { state: "desktop_owned", owner: "desktop" },
        health: freshHealthy,
      }),
    );

    await waitFor(() =>
      expect(
        screen.queryByText("无法读取路由状态。请重新启动桌面应用或查看日志。"),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.getByRole("heading", { name: "路由运行正常" }),
    ).toBeInTheDocument();
  });

  it("shows only bounded sanitized unexpected-exit diagnostics", async () => {
    const secret = "sk-routerFailureCanary123456";
    const recentLogs = Array.from(
      { length: MAX_FAILURE_LOG_LINES + 2 },
      (_, index) => `safe diagnostic line ${index}`,
    );
    recentLogs[MAX_FAILURE_LOG_LINES + 1] =
      `api_key=${secret} final safe marker`;
    const api = createMockApi({
      getRouterStatus: vi.fn().mockResolvedValue({
        state: "start_failed",
        last_error: `router exited unexpectedly Bearer ${secret}`,
        recent_logs: recentLogs,
      }),
    });

    renderWithI18n(<RouterPage api={api} onNavigateToAgents={vi.fn()} />);

    expect(await screen.findByLabelText("意外退出诊断")).toBeInTheDocument();
    expect(screen.getByText(/router exited unexpectedly/)).toBeInTheDocument();
    expect(screen.getByText("safe diagnostic line 2")).toBeInTheDocument();
    expect(
      screen.queryByText("safe diagnostic line 0"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("safe diagnostic line 1"),
    ).not.toBeInTheDocument();
    expect(document.body.textContent).not.toContain(secret);
    expect(document.body.textContent).not.toContain("routerFailureCanary");
    expect(document.body.textContent).toContain("[REDACTED]");
  });

  it.each(["SIDECAR_MISSING", "SIDECAR_INVALID"])(
    "shows localized reinstall guidance for %s without offering a download",
    async (code) => {
      const api = createMockApi({
        getPollSnapshot: vi.fn().mockResolvedValue({
          revision: 1,
          status_error: { code },
        }),
      });
      renderWithI18n(<RouterPage api={api} onNavigateToAgents={vi.fn()} />);

      expect(await screen.findByText("桌面组件无效")).toBeInTheDocument();
      expect(screen.getByText(/应用不会自动下载任何组件/)).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "启动路由" })).toBeDisabled();
      expect(
        screen.queryByRole("button", { name: /下载/ }),
      ).not.toBeInTheDocument();
    },
  );

  it("shows the local address without owner or component version details", async () => {
    const api = createMockApi({
      getRouterStatus: vi.fn().mockResolvedValue({
        state: "desktop_owned",
        owner: "desktop",
        listen_addr: "127.0.0.1:19999",
      }),
    });

    renderWithI18n(<RouterPage api={api} onNavigateToAgents={vi.fn()} />);

    expect(await screen.findByText("127.0.0.1:19999")).toBeInTheDocument();
    expect(screen.queryByText("所有者")).not.toBeInTheDocument();
    expect(screen.queryByText("desktop-v1")).not.toBeInTheDocument();
    expect(screen.queryByText("manager-v1")).not.toBeInTheDocument();
    expect(screen.queryByText("router-v1")).not.toBeInTheDocument();
    expect(api.getComponentVersions).not.toHaveBeenCalled();
  });
});

describe("RouterPage actions", () => {
  it("starts, refreshes status, and retries health", async () => {
    const getRouterStatus = vi
      .fn()
      .mockResolvedValueOnce({ state: "absent" })
      .mockResolvedValue({
        state: "desktop_owned",
        owner: "desktop",
        listen_addr: "127.0.0.1:19099",
      });
    const api = createMockApi({ getRouterStatus });
    renderWithI18n(<RouterPage api={api} onNavigateToAgents={vi.fn()} />);
    const start = await screen.findByRole("button", { name: "启动路由" });

    fireEvent.click(start);

    expect(
      await screen.findByRole("heading", { name: "路由运行正常" }),
    ).toBeInTheDocument();
    expect(api.startRouter).toHaveBeenCalledOnce();
    expect(getRouterStatus).toHaveBeenCalledTimes(2);
    expect(api.retryRouterHealth).toHaveBeenCalledOnce();
  });

  it("stops only a desktop-owned router", async () => {
    const getRouterStatus = vi
      .fn()
      .mockResolvedValueOnce({ state: "desktop_owned", owner: "desktop" })
      .mockResolvedValue({ state: "absent" });
    const api = createMockApi({ getRouterStatus });
    renderWithI18n(<RouterPage api={api} onNavigateToAgents={vi.fn()} />);
    const stop = await screen.findByRole("button", { name: "停止路由" });
    await waitFor(() => expect(stop).toBeEnabled());

    fireEvent.click(stop);

    expect(
      await screen.findByRole("heading", { name: "路由未启动" }),
    ).toBeInTheDocument();
    expect(api.stopRouter).toHaveBeenCalledOnce();
  });

  it("allows stop for a desktop-owned degraded router", async () => {
    const getRouterStatus = vi
      .fn()
      .mockResolvedValueOnce({ state: "degraded", owner: "desktop" })
      .mockResolvedValue({ state: "absent" });
    const api = createMockApi({ getRouterStatus });
    renderWithI18n(<RouterPage api={api} onNavigateToAgents={vi.fn()} />);

    const stop = await screen.findByRole("button", { name: "停止路由" });
    expect(stop).toBeEnabled();
    fireEvent.click(stop);

    expect(await screen.findByText("路由未启动")).toBeInTheDocument();
    expect(api.stopRouter).toHaveBeenCalledOnce();
  });

  it("never enables stop for an external owner and invokes Agent navigation", async () => {
    const navigate = vi.fn();
    const api = createMockApi({
      getRouterStatus: vi.fn().mockResolvedValue({
        state: "external_compatible",
        owner: "cli",
      }),
    });
    renderWithI18n(<RouterPage api={api} onNavigateToAgents={navigate} />);

    expect(await screen.findByText("外部路由正在运行")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "停止路由" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "前往 Agent 配置" }));
    expect(navigate).toHaveBeenCalledOnce();
  });

  it("uses a sanitized local message when start fails", async () => {
    const api = createMockApi({
      startRouter: vi
        .fn()
        .mockRejectedValue(new Error("key-shaped-canary-secret")),
    });
    renderWithI18n(<RouterPage api={api} onNavigateToAgents={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "启动路由" }));

    expect(await screen.findByText(/启动失败。请查看/)).toBeInTheDocument();
    expect(
      screen.queryByText(/key-shaped-canary-secret/),
    ).not.toBeInTheDocument();
  });

  it("reconciles ROUTER_DEGRADED as running and keeps desktop stop enabled", async () => {
    const api = createMockApi({
      startRouter: vi.fn().mockRejectedValue({ code: "ROUTER_DEGRADED" }),
      getPollSnapshot: vi
        .fn()
        .mockResolvedValueOnce({ revision: 1, status: { state: "absent" } })
        .mockResolvedValue({
          revision: 2,
          status: { state: "degraded", owner: "desktop" },
          health: { status: "degraded", checked_at: new Date().toISOString() },
        }),
    });
    renderWithI18n(<RouterPage api={api} onNavigateToAgents={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "启动路由" }));

    expect(await screen.findByText("上游连接不可用")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "停止路由" })).toBeEnabled();
    expect(screen.queryByText(/启动失败/)).not.toBeInTheDocument();
  });
});

afterEach(() => vi.useRealTimers());
