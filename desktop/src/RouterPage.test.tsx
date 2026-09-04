import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  OccupantInspection,
  PollSnapshot,
  RouterHealth,
  RouterStatus,
} from "./ipc";
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
    light: "off" | "green" | "yellow" | "red";
  }>([
    {
      name: "not started",
      status: { state: "absent" },
      heading: "路由未启动",
      process: "未启动",
      upstream: "不可用",
      light: "off",
    },
    {
      name: "starting",
      status: { state: "starting", owner: "desktop" },
      heading: "正在启动路由",
      process: "启动中",
      upstream: "不可用",
      light: "yellow",
    },
    {
      name: "running and healthy",
      status: { state: "desktop_owned", owner: "desktop" },
      health: freshHealthy,
      heading: "路由运行正常",
      process: "运行中",
      upstream: "健康",
      light: "green",
    },
    {
      name: "running with unavailable upstream",
      status: { state: "desktop_owned", owner: "desktop" },
      health: { status: "degraded", checked_at: new Date().toISOString() },
      heading: "上游连接不可用",
      process: "降级运行",
      upstream: "上游不可用",
      light: "yellow",
    },
    {
      name: "external router running",
      status: { state: "external_compatible", owner: "cli" },
      health: freshHealthy,
      heading: "外部路由正在运行",
      process: "外部托管",
      upstream: "健康",
      light: "green",
    },
    {
      name: "port occupied",
      status: { state: "unknown_occupant" },
      heading: "端口已被占用",
      process: "端口冲突",
      upstream: "不可用",
      light: "red",
    },
    {
      name: "legacy managed router",
      status: { state: "legacy_managed", owner: "desktop" },
      heading: "发现历史桌面路由",
      process: "需要迁移",
      upstream: "不可用",
      light: "yellow",
    },
    {
      name: "start failed",
      status: { state: "start_failed" },
      heading: "路由启动失败",
      process: "需要处理",
      upstream: "不可用",
      light: "red",
    },
    {
      name: "stopping",
      status: { state: "stopping", owner: "desktop" },
      heading: "正在停止路由",
      process: "停止中",
      upstream: "不可用",
      light: "yellow",
    },
  ])(
    "renders $name",
    async ({ status, health, heading, process, upstream, light }) => {
      const api = createMockApi({
        getRouterStatus: vi.fn().mockResolvedValue(status),
        retryRouterHealth: vi.fn().mockResolvedValue(health ?? freshHealthy),
      });

      renderWithI18n(
        <RouterPage
          api={api}
          onNavigateToAgents={vi.fn()}
          onNavigateToLogs={vi.fn()}
        />,
      );

      expect(
        await screen.findByRole("heading", { name: heading }),
      ).toBeInTheDocument();
      const readouts = screen
        .getAllByRole("definition")
        .map((element) => element.textContent);
      expect(readouts).toContain(process);
      expect(readouts).toContain(upstream);
      expect(document.querySelector(".traffic-light")).toHaveAttribute(
        "data-state",
        light,
      );
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

    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

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

    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
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
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
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
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    expect(
      await screen.findByRole("heading", { name: "路由运行正常" }),
    ).toBeInTheDocument();

    act(() =>
      observer?.({ revision: 2, status_error: { code: "MANAGER_FAILED" } }),
    );
    expect(
      await screen.findByText(
        "无法读取路由状态（MANAGER_FAILED）。请先复制诊断快照或导出日志包发给维护者，然后再考虑重新启动。",
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
        screen.queryByText(/无法读取路由状态（MANAGER_FAILED）/),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.getByRole("heading", { name: "路由运行正常" }),
    ).toBeInTheDocument();
  });

  it("keeps cached healthy state after a transient status error", async () => {
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
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    expect(
      await screen.findByRole("heading", { name: "路由运行正常" }),
    ).toBeInTheDocument();

    act(() =>
      observer?.({
        revision: 2,
        status: { state: "desktop_owned", owner: "desktop" },
        health: freshHealthy,
        status_error: { code: "OPERATION_TIMEOUT" },
      }),
    );

    expect(
      screen.getByRole("heading", { name: "路由状态暂时不可用" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/无法读取路由状态（OPERATION_TIMEOUT）/),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "停止路由" })).toBeEnabled();
    expect(screen.getByText("进程状态（上次已知）")).toBeInTheDocument();
    expect(screen.getByText("上游健康（上次已知）")).toBeInTheDocument();
    expect(screen.queryByText("路由启动失败")).not.toBeInTheDocument();
  });

  it("shows status unavailable when the first status poll fails", async () => {
    const api = createMockApi({
      getPollSnapshot: vi.fn().mockResolvedValue({
        revision: 1,
        status_error: { code: "OPERATION_TIMEOUT" },
      }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(
      await screen.findByRole("heading", { name: "路由状态暂时不可用" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("路由启动失败")).not.toBeInTheDocument();
    expect(
      screen.getByText(
        "无法读取路由状态（OPERATION_TIMEOUT）。请先复制诊断快照或导出日志包发给维护者，然后再考虑重新启动。",
      ),
    ).toBeInTheDocument();
  });

  it("shows status unavailable on first poll OPERATION_TIMEOUT with watchdog stage", async () => {
    const api = createMockApi({
      getPollSnapshot: vi.fn().mockResolvedValue({
        revision: 1,
        status_error: {
          code: "OPERATION_TIMEOUT",
          stage: "watchdog_timeout",
        },
      }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(
      await screen.findByRole("heading", { name: "路由状态暂时不可用" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("路由启动失败")).not.toBeInTheDocument();
    expect(
      screen.getByText(
        "无法读取路由状态（OPERATION_TIMEOUT / watchdog_timeout）。请先复制诊断快照或导出日志包发给维护者，然后再考虑重新启动。",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "停止路由" })).toBeDisabled();
  });

  it("keeps start_failed heading when status_error arrives", async () => {
    const api = createMockApi({
      getPollSnapshot: vi.fn().mockResolvedValue({
        revision: 1,
        status: { state: "start_failed", last_error: "stage=spawn code=FAIL" },
        status_error: { code: "OPERATION_TIMEOUT" },
      }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(
      await screen.findByRole("heading", { name: "路由启动失败" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/无法读取路由状态（OPERATION_TIMEOUT）/),
    ).toBeInTheDocument();
  });

  it("keeps occupied heading when status_error arrives", async () => {
    const api = createMockApi({
      getPollSnapshot: vi.fn().mockResolvedValue({
        revision: 1,
        status: { state: "unknown_occupant" },
        status_error: { code: "OPERATION_TIMEOUT" },
      }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(
      await screen.findByRole("heading", { name: "端口已被占用" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/无法读取路由状态（OPERATION_TIMEOUT）/),
    ).toBeInTheDocument();
  });

  it("keeps legacy heading when status_error arrives", async () => {
    const api = createMockApi({
      getPollSnapshot: vi.fn().mockResolvedValue({
        revision: 1,
        status: { state: "legacy_managed", owner: "desktop" },
        status_error: { code: "OPERATION_TIMEOUT" },
      }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(
      await screen.findByRole("heading", { name: "发现历史桌面路由" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/无法读取路由状态（OPERATION_TIMEOUT）/),
    ).toBeInTheDocument();
  });

  it("keeps fresh cached health after a transient health error", async () => {
    const api = createMockApi({
      getPollSnapshot: vi.fn().mockResolvedValue({
        revision: 1,
        status: { state: "desktop_owned", owner: "desktop" },
        health: freshHealthy,
        health_error: { code: "OPERATION_TIMEOUT" },
      }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(
      await screen.findByRole("heading", { name: "路由运行正常" }),
    ).toBeInTheDocument();
    expect(screen.getByText("健康")).toBeInTheDocument();
    expect(
      screen.getByText("健康检查失败。路由进程状态未受影响。"),
    ).toBeInTheDocument();
  });

  it("prioritizes a status warning when one snapshot has both errors", async () => {
    const api = createMockApi({
      getPollSnapshot: vi.fn().mockResolvedValue({
        revision: 1,
        status_error: { code: "OPERATION_TIMEOUT" },
        health_error: { code: "OPERATION_TIMEOUT" },
      }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(
      await screen.findByText(
        "无法读取路由状态（OPERATION_TIMEOUT）。请先复制诊断快照或导出日志包发给维护者，然后再考虑重新启动。",
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("健康检查失败。路由进程状态未受影响。"),
    ).not.toBeInTheDocument();
  });

  it("shows only bounded sanitized router failure diagnostics with a logs action", async () => {
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

    const navigateToLogs = vi.fn();
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={navigateToLogs}
      />,
    );

    const details = await screen.findByLabelText("路由失败诊断");
    expect(details).not.toHaveAttribute("open");
    expect(screen.getByText("运行中的路由意外退出")).toBeInTheDocument();
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
    fireEvent.click(screen.getByText("技术详情"));
    expect(details).toHaveAttribute("open");
    fireEvent.click(screen.getByRole("button", { name: "查看运行日志" }));
    expect(navigateToLogs).toHaveBeenCalledOnce();
  });

  it.each([
    ["upstream_probe_failed", "无法连接到上游服务"],
    ["tls_material_invalid", "路由凭据无法使用"],
    ["config_invalid", "路由启动配置无效"],
    ["log_open_failed", "无法准备运行日志"],
    ["listen_failed", "无法打开本地端口"],
    ["shutdown_failed", "路由未能正常关闭"],
    ["router_failure", "路由启动未完成"],
  ])("explains router failure reason %s as %s", async (reason, heading) => {
    const rawLog = `time=2026-08-09T10:20:17+08:00 level=ERROR msg=fatal reason=${reason}`;
    const api = createMockApi({
      getRouterStatus: vi.fn().mockResolvedValue({
        state: "start_failed",
        last_error: "stage=process_exit code=ROUTER_START_FAILED",
        recent_logs: [rawLog],
      }),
    });

    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(await screen.findByRole("heading", { name: heading })).toBeVisible();
    expect(screen.getByText("建议处理")).toBeVisible();
    const details = screen.getByLabelText("路由失败诊断");
    expect(details).not.toHaveAttribute("open");
    expect(details).toHaveTextContent(rawLog);
  });

  it.each([
    ["log_directory", "无法准备运行日志"],
    ["process_launch", "系统未能启动路由组件"],
    ["process_inspect", "无法安全确认路由进程"],
    ["readiness", "路由启动超时"],
    ["identity_validate", "路由组件身份不匹配"],
    ["state_reconcile", "无法确认现有路由状态"],
    ["state_persist", "无法保存路由运行状态"],
    ["process_exit", "路由进程在启动期间退出"],
  ])("explains manager startup stage %s as %s", async (stage, heading) => {
    const diagnostic = `stage=${stage} code=ROUTER_START_FAILED`;
    const api = createMockApi({
      getRouterStatus: vi.fn().mockResolvedValue({
        state: "start_failed",
        last_error: diagnostic,
        recent_logs: [diagnostic],
      }),
    });

    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(await screen.findByRole("heading", { name: heading })).toBeVisible();
  });

  it("explains an unexpected exit without exposing it as the primary message", async () => {
    const raw = "desktop-owned router exited unexpectedly";
    const api = createMockApi({
      getRouterStatus: vi.fn().mockResolvedValue({
        state: "start_failed",
        last_error: raw,
      }),
    });

    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(
      await screen.findByRole("heading", { name: "运行中的路由意外退出" }),
    ).toBeVisible();
    expect(screen.getByLabelText("路由失败诊断")).not.toHaveAttribute("open");
  });

  it.each([
    [
      "status failure",
      { code: "MANAGER_FAILED" },
      "无法读取路由状态（MANAGER_FAILED）。请先复制诊断快照或导出日志包发给维护者，然后再考虑重新启动。",
    ],
    [
      "sidecar failure",
      { code: "SIDECAR_INVALID" },
      "必要的打包组件缺失或无效。请重新安装桌面应用；应用不会自动下载任何组件。",
    ],
  ])(
    "shows a newer %s alongside cached startup guidance",
    async (_name, statusError, warning) => {
      let observer: ((snapshot: PollSnapshot) => void) | undefined;
      const diagnostic = "stage=process_launch code=ROUTER_START_FAILED";
      const api = createMockApi({
        getPollSnapshot: vi.fn().mockResolvedValue({
          revision: 1,
          status: {
            state: "start_failed",
            last_error: diagnostic,
          },
        }),
        subscribePollSnapshots: vi.fn(async (listener) => {
          observer = listener;
          return () => undefined;
        }),
      });

      renderWithI18n(
        <RouterPage
          api={api}
          onNavigateToAgents={vi.fn()}
          onNavigateToLogs={vi.fn()}
        />,
      );
      expect(
        await screen.findByRole("heading", {
          name: "系统未能启动路由组件",
        }),
      ).toBeVisible();

      act(() =>
        observer?.({
          revision: 2,
          status_error: statusError,
        }),
      );

      expect(await screen.findByText(warning)).toBeVisible();
      expect(
        screen.getByRole("heading", { name: "系统未能启动路由组件" }),
      ).toBeVisible();
    },
  );

  it.each(["SIDECAR_MISSING", "SIDECAR_INVALID"])(
    "shows localized reinstall guidance for %s without offering a download",
    async (code) => {
      const api = createMockApi({
        getPollSnapshot: vi.fn().mockResolvedValue({
          revision: 1,
          status_error: { code },
        }),
      });
      renderWithI18n(
        <RouterPage
          api={api}
          onNavigateToAgents={vi.fn()}
          onNavigateToLogs={vi.fn()}
        />,
      );

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

    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(await screen.findByText("127.0.0.1:19999")).toBeInTheDocument();
    expect(screen.queryByText("所有者")).not.toBeInTheDocument();
    expect(screen.queryByText("desktop-v1")).not.toBeInTheDocument();
    expect(screen.queryByText("manager-v1")).not.toBeInTheDocument();
    expect(screen.queryByText("router-v1")).not.toBeInTheDocument();
    expect(api.getComponentVersions).not.toHaveBeenCalled();
  });

  it("always shows copy and export diagnostics actions while healthy", async () => {
    const api = createMockApi({
      getRouterStatus: vi.fn().mockResolvedValue({
        state: "desktop_owned",
        owner: "desktop",
      }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(
      await screen.findByRole("heading", { name: "路由运行正常" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "复制诊断快照" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "导出日志包" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "日志包含本地运行日志原文，仅发给维护者，不要发到公开群。",
      ),
    ).toBeInTheDocument();
  });

  it("copies the diagnostic snapshot summary to the clipboard", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, {
      clipboard: { writeText },
    });
    const api = createMockApi({
      getRouterStatus: vi.fn().mockResolvedValue({
        state: "desktop_owned",
        owner: "desktop",
      }),
      getDiagnosticSnapshot: vi.fn().mockResolvedValue({
        schema_version: 1,
        captured_at: "2026-09-03T10:00:00Z",
        classification: "healthy",
        desktop: "0.1.0",
        manager: "0.1.0",
        management_protocol: "4",
        deployment_id: "dev",
        target: "aarch64-apple-darwin",
        health_stale: false,
        summary: "classification=healthy",
      }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    await screen.findByRole("heading", { name: "路由运行正常" });

    fireEvent.click(screen.getByRole("button", { name: "复制诊断快照" }));

    await waitFor(() =>
      expect(api.getDiagnosticSnapshot).toHaveBeenCalledOnce(),
    );
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith("classification=healthy"),
    );
    expect(await screen.findByText("已复制诊断快照。")).toBeInTheDocument();
  });

  it("exports a support bundle and reports cancelled exports without alert role", async () => {
    const api = createMockApi({
      getRouterStatus: vi.fn().mockResolvedValue({
        state: "desktop_owned",
        owner: "desktop",
      }),
      exportSupportBundle: vi
        .fn()
        .mockRejectedValue({ code: "DIALOG_CANCELLED" }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    await screen.findByRole("heading", { name: "路由运行正常" });

    fireEvent.click(screen.getByRole("button", { name: "导出日志包" }));

    await waitFor(() => expect(api.exportSupportBundle).toHaveBeenCalledOnce());
    const cancelled = await screen.findByText("已取消导出。");
    expect(cancelled).toBeInTheDocument();
    expect(cancelled).not.toHaveAttribute("role", "alert");
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
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
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
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
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
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

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
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={navigate}
        onNavigateToLogs={vi.fn()}
      />,
    );

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
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "启动路由" }));

    expect(await screen.findByText(/启动失败。请查看/)).toBeInTheDocument();
    expect(
      screen.queryByText(/key-shaped-canary-secret/),
    ).not.toBeInTheDocument();
  });

  it("reconciles a rejected start immediately", async () => {
    const diagnostic =
      "stage=process_launch code=ROUTER_START_FAILED os_error=5";
    const api = createMockApi({
      startRouter: vi.fn().mockRejectedValue({ code: "ROUTER_START_FAILED" }),
      getPollSnapshot: vi
        .fn()
        .mockResolvedValueOnce({ revision: 1, status: { state: "absent" } })
        .mockResolvedValue({
          revision: 2,
          status: {
            state: "start_failed",
            owner: "desktop",
            last_error: diagnostic,
            recent_logs: [diagnostic],
          },
        }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    fireEvent.click(await screen.findByRole("button", { name: "启动路由" }));

    expect(
      await screen.findByRole("heading", { name: "路由启动失败" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("启动失败。请查看安全过滤后的日志并重试。"),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "系统未能启动路由组件" }),
    ).toBeVisible();
    expect(await screen.findAllByText(diagnostic)).not.toHaveLength(0);
    expect(api.getPollSnapshot).toHaveBeenCalledTimes(2);
  });

  it("preserves a real start failure across a transient status error", async () => {
    let observer: ((snapshot: PollSnapshot) => void) | undefined;
    const api = createMockApi({
      startRouter: vi.fn().mockRejectedValue({ code: "START_FAILED" }),
      subscribePollSnapshots: vi.fn(async (listener) => {
        observer = listener;
        return () => undefined;
      }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "启动路由" }));
    expect(
      await screen.findByRole("heading", { name: "路由启动失败" }),
    ).toBeInTheDocument();

    act(() =>
      observer?.({
        revision: 2,
        status: { state: "absent" },
        status_error: { code: "OPERATION_TIMEOUT" },
      }),
    );

    expect(
      screen.getByRole("heading", { name: "路由启动失败" }),
    ).toBeInTheDocument();
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
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "启动路由" }));

    expect(await screen.findByText("上游连接不可用")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "停止路由" })).toBeEnabled();
    expect(screen.queryByText(/启动失败/)).not.toBeInTheDocument();
  });
});

describe("RouterPage occupant recovery", () => {
  const inspection: OccupantInspection = {
    pid: 4242,
    verification_mode: "verified_identity",
    process_name: "example-server",
    executable:
      "/Users/example/a-very-long-directory-name/another-directory/example-server",
    listen_addr: "127.0.0.1:19099",
    recovery: { action: "force_terminate" },
    confirmation_token: "opaque-confirmation-token",
    expires_at: "2026-07-18T12:00:30Z",
  };
  const pidOnlyInspection: OccupantInspection = {
    pid: 4343,
    verification_mode: "windows_pid_only",
    listen_addr: "127.0.0.1:19099",
    recovery: { action: "force_terminate" },
    confirmation_token: "pid-only-confirmation-token",
    expires_at: "2026-07-22T12:00:30Z",
  };
  const windowsServiceInspection: OccupantInspection = {
    pid: 4444,
    verification_mode: "windows_pid_only",
    listen_addr: "127.0.0.1:19099",
    recovery: {
      action: "manual_stop_required",
      reason: "service_managed",
    },
    supervisor: {
      kind: "windows_service",
      scope: "system",
      identifiers: ["RouterHelper", "RouterSvc"],
    },
  };

  function occupiedApi(overrides = {}) {
    return createMockApi({
      getRouterStatus: vi.fn().mockResolvedValue({
        state: "unknown_occupant",
        listen_addr: "127.0.0.1:19099",
      }),
      inspectRouterOccupant: vi.fn().mockResolvedValue(inspection),
      ...overrides,
    });
  }

  it("inspects on occupied transition, shows a target, and keeps Stop disabled", async () => {
    const api = occupiedApi();
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(await screen.findByText("example-server")).toBeInTheDocument();
    expect(screen.getByText("4242")).toBeInTheDocument();
    expect(api.inspectRouterOccupant).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: "停止路由" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "强制终止占用进程" }),
    ).toBeEnabled();
  });

  it("shows every Windows Service and fixed copyable commands without a force flow", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    const api = occupiedApi({
      inspectRouterOccupant: vi
        .fn()
        .mockResolvedValue(windowsServiceInspection),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(await screen.findByText("RouterHelper")).toBeInTheDocument();
    expect(screen.getByText("RouterSvc")).toBeInTheDocument();
    expect(screen.getByText("sc.exe stop 'RouterHelper'")).toBeInTheDocument();
    expect(screen.getByText("sc.exe stop 'RouterSvc'")).toBeInTheDocument();
    expect(screen.getByText(/services\.msc/)).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "强制终止占用进程" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    fireEvent.click(screen.getAllByRole("button", { name: "复制命令" })[1]);
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith("sc.exe stop 'RouterSvc'"),
    );
    expect(await screen.findByText("命令已复制")).toBeInTheDocument();
    expect(api.forceTerminateRouterOccupant).not.toHaveBeenCalled();
  });

  it("quotes malicious-valid Windows Service names as PowerShell literals", async () => {
    const identifier = "Svc'$`(calc)%;&| name";
    const command = "sc.exe stop 'Svc''$`(calc)%;&| name'";
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    const api = occupiedApi({
      inspectRouterOccupant: vi.fn().mockResolvedValue({
        ...windowsServiceInspection,
        supervisor: {
          ...windowsServiceInspection.supervisor,
          identifiers: [identifier],
        },
      }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(await screen.findByText(identifier)).toBeInTheDocument();
    expect(screen.getByText(command)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "复制命令" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(command));
  });

  it("keeps the newest clipboard result when older requests settle later", async () => {
    let resolveFirst: (() => void) | undefined;
    const first = new Promise<void>((resolve) => {
      resolveFirst = resolve;
    });
    const writeText = vi
      .fn()
      .mockReturnValueOnce(first)
      .mockRejectedValueOnce(new Error("clipboard denied"));
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    const api = occupiedApi({
      inspectRouterOccupant: vi
        .fn()
        .mockResolvedValue(windowsServiceInspection),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    await screen.findByText("RouterHelper");
    const copyButtons = screen.getAllByRole("button", { name: "复制命令" });

    fireEvent.click(copyButtons[0]);
    fireEvent.click(copyButtons[1]);
    expect(await screen.findByText("无法复制命令")).toBeInTheDocument();
    await act(async () => resolveFirst?.());

    expect(screen.getByText("无法复制命令")).toBeInTheDocument();
    expect(screen.queryByText("命令已复制")).not.toBeInTheDocument();
  });

  it.each([
    [
      "systemd user",
      {
        kind: "systemd_user" as const,
        scope: "user" as const,
        identifiers: [String.raw`router\x2dhelper.service`],
      },
      String.raw`systemctl --user stop -- 'router\x2dhelper.service'`,
    ],
    [
      "systemd system",
      {
        kind: "systemd_system" as const,
        scope: "system" as const,
        identifiers: ["mtls-router.service"],
      },
      "sudo systemctl stop -- 'mtls-router.service'",
    ],
  ])(
    "shows a fixed non-executed command for %s",
    async (_name, supervisor, command) => {
      const writeText = vi.fn().mockResolvedValue(undefined);
      Object.defineProperty(navigator, "clipboard", {
        configurable: true,
        value: { writeText },
      });
      const api = occupiedApi({
        inspectRouterOccupant: vi.fn().mockResolvedValue({
          pid: 4555,
          verification_mode: "verified_identity",
          process_name: "supervised-router",
          executable: "/usr/local/bin/supervised-router",
          listen_addr: "127.0.0.1:19099",
          recovery: {
            action: "manual_stop_required",
            reason: "service_managed",
          },
          supervisor,
        } satisfies OccupantInspection),
      });
      renderWithI18n(
        <RouterPage
          api={api}
          onNavigateToAgents={vi.fn()}
          onNavigateToLogs={vi.fn()}
        />,
      );

      expect(await screen.findByText(command)).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "复制命令" }));
      await waitFor(() => expect(writeText).toHaveBeenCalledWith(command));
      expect(
        screen.queryByRole("button", { name: "强制终止占用进程" }),
      ).not.toBeInTheDocument();
      expect(api.forceTerminateRouterOccupant).not.toHaveBeenCalled();
    },
  );

  it.each<[OccupantInspection, string]>([
    [
      {
        pid: 4661,
        verification_mode: "windows_pid_only",
        listen_addr: "127.0.0.1:19099",
        recovery: {
          action: "manual_stop_required",
          reason: "insufficient_privilege",
        },
      },
      "Windows PPL",
    ],
    [
      {
        pid: 4662,
        verification_mode: "verified_identity",
        listen_addr: "127.0.0.1:19099",
        recovery: {
          action: "manual_stop_required",
          reason: "different_user",
        },
      },
      "属于其他用户",
    ],
    [
      {
        pid: 4663,
        verification_mode: "verified_identity",
        process_name: "protected-router",
        executable: "/usr/local/bin/protected-router",
        listen_addr: "127.0.0.1:19099",
        recovery: { action: "unavailable", reason: "protected_process" },
      },
      "应用生命周期保护",
    ],
    [
      {
        pid: 4664,
        verification_mode: "windows_pid_only",
        listen_addr: "127.0.0.1:19099",
        recovery: { action: "unavailable", reason: "identity_unavailable" },
      },
      "无法可靠验证",
    ],
  ])("renders reason-specific blocked guidance", async (blocked, guidance) => {
    const api = occupiedApi({
      inspectRouterOccupant: vi.fn().mockResolvedValue(blocked),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(await screen.findByText(new RegExp(guidance))).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "强制终止占用进程" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(api.forceTerminateRouterOccupant).not.toHaveBeenCalled();
  });

  it.each([
    [
      "PID-only identity field",
      { ...pidOnlyInspection, process_name: "fabricated-name" },
    ],
    [
      "extra identity-like field",
      { ...inspection, process_owner: "fabricated-owner" },
    ],
    [
      "wrong endpoint",
      { ...pidOnlyInspection, listen_addr: "127.0.0.1:19100" },
    ],
    ["zero PID", { ...pidOnlyInspection, pid: 0 }],
    ["PID above u32", { ...pidOnlyInspection, pid: 0x1_0000_0000 }],
    ["blank token", { ...pidOnlyInspection, confirmation_token: " " }],
    ["blank expiry", { ...pidOnlyInspection, expires_at: " " }],
    ["malformed expiry", { ...pidOnlyInspection, expires_at: "next Tuesday" }],
    [
      "lowercase date-time separator",
      { ...pidOnlyInspection, expires_at: "2026-07-22t12:00:30Z" },
    ],
    [
      "lowercase UTC marker",
      { ...pidOnlyInspection, expires_at: "2026-07-22T12:00:30z" },
    ],
    [
      "leap second",
      { ...pidOnlyInspection, expires_at: "2026-07-22T12:00:60Z" },
    ],
  ])(
    "blocks a malformed runtime inspection with %s",
    async (_case, malformed) => {
      const api = occupiedApi({
        inspectRouterOccupant: vi.fn().mockResolvedValue(malformed),
      });
      renderWithI18n(
        <RouterPage
          api={api}
          onNavigateToAgents={vi.fn()}
          onNavigateToLogs={vi.fn()}
        />,
      );

      expect(
        await screen.findByText(/无法完整验证占用进程身份/),
      ).toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: "强制终止占用进程" }),
      ).not.toBeInTheDocument();
      expect(screen.getByRole("button", { name: "重试检查" })).toBeEnabled();
      expect(api.forceTerminateRouterOccupant).not.toHaveBeenCalled();
    },
  );

  it.each([
    ["standard UTC", "2026-07-22T12:00:30Z"],
    ["numeric offset", "2026-07-22T20:00:30+08:00"],
  ])("accepts a manager %s expiry", async (_case, expiresAt) => {
    const api = occupiedApi({
      inspectRouterOccupant: vi.fn().mockResolvedValue({
        ...pidOnlyInspection,
        expires_at: expiresAt,
      }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(
      await screen.findByRole("button", { name: "强制终止占用进程" }),
    ).toBeEnabled();
  });

  it.each([
    ["OCCUPANT_NOT_OWNED", "该进程不属于当前用户"],
    ["OCCUPANT_IDENTITY_UNAVAILABLE", "无法完整验证占用进程身份"],
    ["OCCUPANT_PROTECTED", "该进程受桌面生命周期保护"],
    ["OCCUPANT_CHANGED", "端口占用进程已变化"],
    ["MANAGER_FAILED", "暂时无法检查占用进程"],
  ])("explains blocked inspection %s and allows retry", async (code, copy) => {
    const inspect = vi
      .fn()
      .mockRejectedValueOnce({ code })
      .mockResolvedValueOnce(inspection);
    const api = occupiedApi({ inspectRouterOccupant: inspect });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    expect(await screen.findByText(new RegExp(copy))).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "强制终止占用进程" }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "重试检查" }));
    expect(await screen.findByText("example-server")).toBeInTheDocument();
    expect(inspect).toHaveBeenCalledTimes(2);
  });

  it("discards an in-flight inspection after status leaves occupied", async () => {
    let observer: ((snapshot: PollSnapshot) => void) | undefined;
    let resolveInspection: ((value: OccupantInspection) => void) | undefined;
    const api = createMockApi({
      getPollSnapshot: vi.fn().mockResolvedValue({
        revision: 1,
        status: { state: "unknown_occupant" },
      }),
      subscribePollSnapshots: vi.fn(async (listener) => {
        observer = listener;
        return () => undefined;
      }),
      inspectRouterOccupant: vi.fn(
        () =>
          new Promise<OccupantInspection>((resolve) => {
            resolveInspection = resolve;
          }),
      ),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    expect(
      await screen.findByText("正在安全检查占用进程..."),
    ).toBeInTheDocument();

    act(() => observer?.({ revision: 2, status: { state: "absent" } }));
    await act(async () => resolveInspection?.(inspection));

    expect(screen.getByText("路由未启动")).toBeInTheDocument();
    expect(screen.queryByText("example-server")).not.toBeInTheDocument();
  });

  it("traps modal focus and restores it to the force trigger on close", async () => {
    const api = occupiedApi();
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    const trigger = await screen.findByRole("button", {
      name: "强制终止占用进程",
    });
    fireEvent.click(trigger);

    const dialog = screen.getByRole("dialog", { name: "确认强制终止占用进程" });
    expect(dialog).toHaveTextContent(inspection.process_name);
    expect(dialog).toHaveTextContent(String(inspection.pid));
    expect(dialog).toHaveTextContent(inspection.executable);
    expect(dialog).toHaveTextContent("未保存的数据可能丢失");
    const cancel = screen.getByRole("button", { name: "取消" });
    const confirm = screen.getByRole("button", { name: "强制终止" });
    expect(cancel).toHaveFocus();
    fireEvent.keyDown(cancel, { key: "Tab", shiftKey: true });
    expect(confirm).toHaveFocus();
    fireEvent.keyDown(confirm, { key: "Tab" });
    expect(cancel).toHaveFocus();
    fireEvent.click(cancel);

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
    expect(api.forceTerminateRouterOccupant).not.toHaveBeenCalled();
  });

  it("shows only PID with the Windows warning in the panel and dialog", async () => {
    const api = occupiedApi({
      inspectRouterOccupant: vi.fn().mockResolvedValue(pidOnlyInspection),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    const action = await screen.findByRole("button", {
      name: "强制终止占用进程",
    });
    const panel = action.closest("section")!;
    expect(panel).toHaveTextContent("4343");
    expect(panel).not.toHaveTextContent("127.0.0.1:19099");
    expect(panel).toHaveTextContent(
      "Windows 未验证该进程的身份、所有者、启动时间或可执行文件",
    );
    expect(
      screen.queryByText("进程", { selector: "dt" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("可执行文件完整路径", { selector: "dt" }),
    ).not.toBeInTheDocument();
    expect(panel).not.toHaveTextContent("example-server");

    fireEvent.click(action);

    const dialog = screen.getByRole("dialog", { name: "确认强制终止占用进程" });
    expect(dialog).toHaveTextContent("4343");
    expect(dialog).not.toHaveTextContent("127.0.0.1:19099");
    expect(dialog).toHaveTextContent(
      "Windows 未验证该进程的身份、所有者、启动时间或可执行文件",
    );
    expect(dialog).toHaveTextContent(
      "管理器会在终止前重新检查同一端口仍由同一 PID 占用",
    );
    expect(dialog).toHaveTextContent(
      "PID 重用和无法读取托管路由状态仍会留下风险",
    );
    expect(dialog.querySelectorAll("dt")[0]).toHaveTextContent("PID");
    expect(dialog.querySelectorAll("dt")).toHaveLength(1);
    expect(dialog).not.toHaveTextContent("example-server");
    expect(api.forceTerminateRouterOccupant).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "取消" })).toHaveFocus();
  });

  it("cancels PID-only confirmation without a request", async () => {
    const api = occupiedApi({
      inspectRouterOccupant: vi.fn().mockResolvedValue(pidOnlyInspection),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    fireEvent.click(
      await screen.findByRole("button", { name: "强制终止占用进程" }),
    );

    fireEvent.click(screen.getByRole("button", { name: "取消" }));

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(api.forceTerminateRouterOccupant).not.toHaveBeenCalled();
  });

  it("requires PID-only dialog confirmation, submits exactly the token, and does not start", async () => {
    const api = occupiedApi({
      inspectRouterOccupant: vi.fn().mockResolvedValue(pidOnlyInspection),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    fireEvent.click(
      await screen.findByRole("button", { name: "强制终止占用进程" }),
    );
    expect(api.forceTerminateRouterOccupant).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "强制终止" }));

    await waitFor(() =>
      expect(api.forceTerminateRouterOccupant).toHaveBeenCalledOnce(),
    );
    expect(api.forceTerminateRouterOccupant).toHaveBeenCalledWith(
      pidOnlyInspection.confirmation_token,
    );
    expect(api.startRouter).not.toHaveBeenCalled();
  });

  it("closes on Escape before submit without a termination request", async () => {
    const api = occupiedApi();
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    const trigger = await screen.findByRole("button", {
      name: "强制终止占用进程",
    });
    fireEvent.click(trigger);
    fireEvent.keyDown(window, { key: "Escape" });

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
    expect(api.forceTerminateRouterOccupant).not.toHaveBeenCalled();
  });

  it("locks duplicate submission and dismissal until termination completes", async () => {
    let resolveTermination:
      | ((result: {
          termination: "process_terminated";
          port_state: "released";
        }) => void)
      | undefined;
    const terminate = vi.fn(
      () =>
        new Promise<{
          termination: "process_terminated";
          port_state: "released";
        }>((resolve) => {
          resolveTermination = resolve;
        }),
    );
    const api = occupiedApi({ forceTerminateRouterOccupant: terminate });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    fireEvent.click(
      await screen.findByRole("button", { name: "强制终止占用进程" }),
    );
    const confirm = screen.getByRole("button", { name: "强制终止" });
    fireEvent.click(confirm);
    fireEvent.click(confirm);
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.mouseDown(screen.getByRole("dialog").parentElement!);

    expect(terminate).toHaveBeenCalledOnce();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    const cancel = screen.getByRole("button", { name: "取消" });
    expect(cancel).not.toBeDisabled();
    expect(cancel).toHaveAttribute("aria-disabled", "true");
    expect(screen.getByRole("button", { name: "正在终止..." })).toBeDisabled();
    cancel.focus();
    fireEvent.keyDown(cancel, { key: "Tab" });
    expect(cancel).toHaveFocus();
    fireEvent.click(cancel);
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    await act(async () =>
      resolveTermination?.({
        termination: "process_terminated",
        port_state: "released",
      }),
    );
  });

  it("begins observation without claiming final release and never starts the router", async () => {
    const getRouterStatus = vi
      .fn()
      .mockResolvedValueOnce({ state: "unknown_occupant" })
      .mockResolvedValue({ state: "absent" });
    const api = occupiedApi({ getRouterStatus });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    fireEvent.click(
      await screen.findByRole("button", { name: "强制终止占用进程" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "强制终止" }));

    await waitFor(() =>
      expect(api.forceTerminateRouterOccupant).toHaveBeenCalledOnce(),
    );
    expect(screen.queryByText(/端口已释放/)).not.toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(api.forceTerminateRouterOccupant).toHaveBeenCalledWith(
      inspection.confirmation_token,
    );
    expect(api.startRouter).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "启动路由" })).toBeEnabled();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "启动路由" })).toHaveFocus(),
    );
  });

  it.each([
    ["OCCUPANT_CHANGED", "端口占用进程已变化"],
    ["CONFIRMATION_EXPIRED", "端口占用进程已变化"],
    ["OCCUPANT_PERMISSION_DENIED", "终止权限被拒绝"],
    ["OCCUPANT_TERMINATION_FAILED", "终止请求未能结束进程"],
    ["PORT_RELEASE_TIMEOUT", "未能确认端口已释放"],
    ["MANAGER_FAILED", "暂时无法检查占用进程"],
  ])(
    "consumes the target after force error %s and requires reinspection",
    async (code, expected) => {
      const inspect = vi
        .fn()
        .mockResolvedValueOnce(inspection)
        .mockResolvedValueOnce(pidOnlyInspection);
      const api = occupiedApi({
        inspectRouterOccupant: inspect,
        forceTerminateRouterOccupant: vi.fn().mockRejectedValue({ code }),
      });
      renderWithI18n(
        <RouterPage
          api={api}
          onNavigateToAgents={vi.fn()}
          onNavigateToLogs={vi.fn()}
        />,
      );
      fireEvent.click(
        await screen.findByRole("button", { name: "强制终止占用进程" }),
      );
      fireEvent.click(screen.getByRole("button", { name: "强制终止" }));

      expect(await screen.findByText(new RegExp(expected))).toBeInTheDocument();
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      expect(screen.queryByText("example-server")).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: "强制终止占用进程" }),
      ).not.toBeInTheDocument();
      const retry = screen.getByRole("button", { name: "重试检查" });
      await waitFor(() => expect(retry).toHaveFocus());
      fireEvent.click(retry);
      expect(
        await screen.findByRole("button", { name: "强制终止占用进程" }),
      ).toBeEnabled();
      expect(inspect).toHaveBeenCalledTimes(2);
    },
  );

  it("renders revisioned observation states and ignores stale terminal replacement", async () => {
    let observer: ((snapshot: PollSnapshot) => void) | undefined;
    const api = createMockApi({
      getPollSnapshot: vi.fn().mockResolvedValue({
        revision: 1,
        status: { state: "absent" },
      }),
      subscribePollSnapshots: vi.fn(async (listener) => {
        observer = listener;
        return () => undefined;
      }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    await screen.findByText("路由未启动");

    act(() =>
      observer?.({
        revision: 2,
        status: { state: "absent" },
        release_observation: { state: "observing" },
      }),
    );
    expect(screen.getByText("正在确认端口保持释放")).toBeInTheDocument();

    act(() =>
      observer?.({
        revision: 4,
        status: { state: "absent" },
        release_observation: { state: "released" },
      }),
    );
    expect(screen.getByText("端口保持释放")).toBeInTheDocument();

    act(() =>
      observer?.({
        revision: 3,
        status: { state: "absent" },
        release_observation: { state: "observing" },
      }),
    );
    expect(screen.getByText("端口保持释放")).toBeInTheDocument();
    expect(screen.queryByText("正在确认端口保持释放")).not.toBeInTheDocument();
  });

  it("reinspects only once when the newest observation is reoccupied and unknown", async () => {
    let observer: ((snapshot: PollSnapshot) => void) | undefined;
    const inspect = vi.fn().mockResolvedValue(windowsServiceInspection);
    const api = createMockApi({
      getPollSnapshot: vi.fn().mockResolvedValue({
        revision: 1,
        status: { state: "absent" },
        release_observation: { state: "observing" },
      }),
      subscribePollSnapshots: vi.fn(async (listener) => {
        observer = listener;
        return () => undefined;
      }),
      inspectRouterOccupant: inspect,
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    await screen.findByText("正在确认端口保持释放");

    act(() =>
      observer?.({
        revision: 2,
        status: { state: "unknown_occupant" },
        release_observation: { state: "reoccupied" },
      }),
    );
    expect(
      await screen.findByText("替代进程或监管器重新占用端口"),
    ).toBeInTheDocument();
    const guidance = screen.getByText(
      /服务控制管理器（SCM）服务、systemd unit 或 launchd job/,
    );
    expect(guidance).toHaveTextContent("应用不会猜测监管器标识");
    await waitFor(() => expect(inspect).toHaveBeenCalledOnce());

    act(() =>
      observer?.({
        revision: 3,
        status: { state: "unknown_occupant" },
        release_observation: { state: "reoccupied" },
      }),
    );
    await act(async () => undefined);
    expect(inspect).toHaveBeenCalledOnce();
  });

  it.each([
    [4242, "终止请求未能结束原占用进程"],
    [5252, "替代进程已重新占用端口"],
  ])(
    "distinguishes the reoccupied PID %s after force termination",
    async (reinspectedPid, expectedOutcome) => {
      let observer: ((snapshot: PollSnapshot) => void) | undefined;
      const inspect = vi
        .fn()
        .mockResolvedValueOnce(inspection)
        .mockResolvedValueOnce({ ...inspection, pid: reinspectedPid });
      const api = occupiedApi({
        inspectRouterOccupant: inspect,
        subscribePollSnapshots: vi.fn(async (listener) => {
          observer = listener;
          return () => undefined;
        }),
      });
      renderWithI18n(
        <RouterPage
          api={api}
          onNavigateToAgents={vi.fn()}
          onNavigateToLogs={vi.fn()}
        />,
      );
      fireEvent.click(
        await screen.findByRole("button", { name: "强制终止占用进程" }),
      );
      fireEvent.click(screen.getByRole("button", { name: "强制终止" }));
      await waitFor(() =>
        expect(api.forceTerminateRouterOccupant).toHaveBeenCalledOnce(),
      );

      act(() =>
        observer?.({
          revision: 10,
          status: { state: "unknown_occupant" },
          release_observation: { state: "reoccupied" },
        }),
      );

      expect(await screen.findByText(expectedOutcome)).toBeInTheDocument();
      expect(inspect).toHaveBeenCalledTimes(2);
    },
  );

  it("does not let a stale reinspection classify a newer observation revision", async () => {
    let observer: ((snapshot: PollSnapshot) => void) | undefined;
    let resolveStale: ((value: OccupantInspection) => void) | undefined;
    const staleInspection = new Promise<OccupantInspection>((resolve) => {
      resolveStale = resolve;
    });
    const inspect = vi
      .fn()
      .mockResolvedValueOnce(inspection)
      .mockReturnValueOnce(staleInspection)
      .mockResolvedValueOnce({ ...inspection, pid: 5252 });
    const api = occupiedApi({
      inspectRouterOccupant: inspect,
      subscribePollSnapshots: vi.fn(async (listener) => {
        observer = listener;
        return () => undefined;
      }),
    });
    renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );
    fireEvent.click(
      await screen.findByRole("button", { name: "强制终止占用进程" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "强制终止" }));
    await waitFor(() =>
      expect(api.forceTerminateRouterOccupant).toHaveBeenCalledOnce(),
    );

    act(() =>
      observer?.({
        revision: 10,
        status: { state: "unknown_occupant" },
        release_observation: { state: "reoccupied" },
      }),
    );
    await waitFor(() => expect(inspect).toHaveBeenCalledTimes(2));
    act(() =>
      observer?.({
        revision: 11,
        status: { state: "unknown_occupant" },
      }),
    );
    await act(async () => undefined);
    act(() =>
      observer?.({
        revision: 12,
        status: { state: "unknown_occupant" },
        release_observation: { state: "reoccupied" },
      }),
    );

    expect(
      await screen.findByText("替代进程已重新占用端口"),
    ).toBeInTheDocument();
    expect(inspect).toHaveBeenCalledTimes(3);
    await act(async () => resolveStale?.(inspection));
    expect(
      screen.queryByText("终止请求未能结束原占用进程"),
    ).not.toBeInTheDocument();
  });

  it("cancels release observation before start and on page unmount", async () => {
    const calls: string[] = [];
    const api = createMockApi({
      cancelRouterReleaseObservation: vi.fn(async () => {
        calls.push("cancel");
      }),
      startRouter: vi.fn(async () => {
        calls.push("start");
        return {
          state: "desktop_owned" as const,
          owner: "desktop" as const,
        };
      }),
    });
    const rendered = renderWithI18n(
      <RouterPage
        api={api}
        onNavigateToAgents={vi.fn()}
        onNavigateToLogs={vi.fn()}
      />,
    );

    fireEvent.click(await screen.findByRole("button", { name: "启动路由" }));
    await waitFor(() => expect(api.startRouter).toHaveBeenCalledOnce());
    expect(calls.slice(0, 2)).toEqual(["cancel", "start"]);

    rendered.unmount();
    expect(api.cancelRouterReleaseObservation).toHaveBeenCalledTimes(2);
  });
});

afterEach(() => vi.useRealTimers());
