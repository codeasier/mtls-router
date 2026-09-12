import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "./App";
import { LANGUAGE_STORAGE_KEY } from "./i18n";
import { THEME_STORAGE_KEY } from "./theme";
import { createMockApi } from "./test/api";

async function openSettings(api = createMockApi()) {
  render(<App api={api} />);
  fireEvent.click(await screen.findByRole("button", { name: /系统设置/ }));
  await screen.findByRole("heading", { name: "桌面控制面板" });
  return api;
}

beforeEach(() => localStorage.clear());

describe("SettingsPage", () => {
  it("shows one application version and application locations without sensitive controls", async () => {
    await openSettings();

    expect(screen.getByRole("switch", { name: /开机时启动/ })).toBeChecked();
    const versionList = screen.getByRole("list", { name: "版本" });
    const rows = within(versionList).getAllByRole("listitem");
    expect(rows).toHaveLength(1);
    expect(rows[0]).toHaveTextContent("应用版本");
    expect(rows[0]).toHaveTextContent("desktop-v1");
    expect(versionList).not.toHaveTextContent(/管理器|路由/);
    expect(screen.getByText("/safe/app-data")).toBeInTheDocument();
    expect(
      screen.getByText("/safe/app-data/mtls-router-logs"),
    ).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "常规" })).toBeVisible();
    expect(screen.getByRole("radiogroup", { name: "外观主题" })).toBeVisible();
    expect(screen.getByRole("radio", { name: "暖沙" })).toBeChecked();
    expect(screen.getByRole("heading", { name: "版本" })).toBeVisible();
    expect(screen.getByRole("heading", { name: "存储位置" })).toBeVisible();
    expect(screen.getByRole("heading", { name: "准备卸载" })).toBeVisible();
    expect(
      screen.queryByText(/上游 URL|证书导入|自动更新|PATH/),
    ).not.toBeInTheDocument();
  });

  it("adds a router row only while a foreign router is being reused", async () => {
    await openSettings(
      createMockApi({
        getComponentVersions: vi.fn().mockResolvedValue({
          version: "desktop-v1",
          management_protocol: "4",
          external_router: { owner: "cli", version: "0.4.1" },
        }),
      }),
    );

    const rows = within(
      screen.getByRole("list", { name: "版本" }),
    ).getAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("应用版本");
    expect(rows[0]).toHaveTextContent("desktop-v1");
    expect(rows[1]).toHaveTextContent("复用的外部路由（历史 CLI 安装）");
    expect(rows[1]).toHaveTextContent("0.4.1");
  });

  it("surfaces a safe load error when settings reads fail", async () => {
    await openSettings(
      createMockApi({
        getAutostart: vi
          .fn()
          .mockRejectedValue(new Error("secret path /tmp/settings")),
      }),
    );

    expect(
      await screen.findByText("部分设置无法读取，请稍后重试。"),
    ).toBeVisible();
    expect(document.body).not.toHaveTextContent("secret path");
    expect(document.body).not.toHaveTextContent("/tmp/settings");
  });

  it("changes current-user autostart through the typed API", async () => {
    const api = await openSettings();
    fireEvent.click(screen.getByRole("switch", { name: /开机时启动/ }));

    await waitFor(() => expect(api.setAutostart).toHaveBeenCalledWith(false));
    expect(
      screen.getByRole("switch", { name: /开机时启动/ }),
    ).not.toBeChecked();
    expect(screen.getByText("开机启动设置已更新。")).toBeInTheDocument();
  });

  it("shows current update state and supports a manual recheck", async () => {
    const api = await openSettings();

    expect(await screen.findByText("当前已是最新版本")).toBeVisible();
    expect(screen.getAllByText("desktop-v1")[0]).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "检查更新" }));

    await waitFor(() => expect(api.checkForUpdate).toHaveBeenCalledTimes(2));
    expect(
      screen.queryByRole("button", { name: "安装并重启" }),
    ).not.toBeInTheDocument();
  });

  it("surfaces an accessible error when the update check rejects", async () => {
    const api = await openSettings(
      createMockApi({
        checkForUpdate: vi.fn().mockRejectedValue(new Error("network")),
      }),
    );

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("无法检查更新");
    expect(screen.getAllByText("无法检查更新，请稍后重试。")).toHaveLength(1);
    expect(api.checkForUpdate).toHaveBeenCalledOnce();
    expect(
      screen.queryByRole("button", { name: "安装并重启" }),
    ).not.toBeInTheDocument();
  });

  it("requires confirmation before install and reports download progress", async () => {
    const progressListener = {
      current: null as
        ((progress: { downloaded: number; total?: number }) => void) | null,
    };
    let finishInstall!: () => void;
    const installPromise = new Promise<void>((resolve) => {
      finishInstall = resolve;
    });
    const api = await openSettings(
      createMockApi({
        checkForUpdate: vi.fn().mockResolvedValue({
          available: true,
          current_version: "1.0.0",
          update: {
            version: "1.1.0",
            notes: "Security and reliability fixes.",
            published_at: "2026-08-01T00:00:00Z",
          },
        }),
        subscribeUpdateProgress: vi.fn(async (listener) => {
          progressListener.current = listener;
          return () => undefined;
        }),
        installUpdate: vi.fn(() => installPromise),
      }),
    );
    await screen.findByText("Security and reliability fixes.");

    fireEvent.click(screen.getByRole("button", { name: "安装并重启" }));
    const dialog = await screen.findByRole("dialog", {
      name: "安装更新",
    });
    expect(api.installUpdate).not.toHaveBeenCalled();

    fireEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(api.installUpdate).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "安装并重启" }));
    fireEvent.click(
      within(await screen.findByRole("dialog", { name: "安装更新" })).getByRole(
        "button",
        { name: "继续安装" },
      ),
    );
    await waitFor(() =>
      expect(api.installUpdate).toHaveBeenCalledWith("1.1.0"),
    );
    expect(api.subscribeUpdateProgress).toHaveBeenCalledOnce();

    act(() => progressListener.current?.({ downloaded: 50, total: 100 }));
    const progress = await screen.findByRole("progressbar", {
      name: "更新下载进度",
    });
    expect(progress).toHaveAttribute("aria-valuenow", "50");
    expect(progress).toHaveAttribute("aria-valuemax", "100");
    expect(screen.getByText("已下载 50 / 100 字节")).toBeVisible();

    await act(async () => finishInstall());
    expect(
      await screen.findByText("更新已安装，正在重启...", {
        selector: ".settings-block__update-state",
      }),
    ).toBeVisible();
  });

  it.each([
    "UPDATE_BLOCKED_ROUTER_STATE",
    "UPDATE_DOWNLOAD_FAILED",
    "UPDATE_BUSY",
    "UPDATE_INSTALL_FAILED",
    "UPDATE_NOT_AVAILABLE",
    "UPDATE_CHANGED",
  ])("surfaces the stable install error code %s", async (code) => {
    const api = await openSettings(
      createMockApi({
        checkForUpdate: vi.fn().mockResolvedValue({
          available: true,
          current_version: "1.0.0",
          update: { version: "1.1.0" },
        }),
        installUpdate: vi.fn().mockRejectedValue({
          code,
          message: "secret path /tmp/update-cache",
        }),
      }),
    );

    fireEvent.click(await screen.findByRole("button", { name: "安装并重启" }));
    fireEvent.click(
      within(await screen.findByRole("dialog", { name: "安装更新" })).getByRole(
        "button",
        { name: "继续安装" },
      ),
    );

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(`无法下载或安装更新（${code}）`);
    expect(alert).toHaveTextContent("当前版本未更改，请重试。");
    expect(document.body).not.toHaveTextContent("secret path");
    expect(document.body).not.toHaveTextContent("/tmp/update-cache");
    expect(api.installUpdate).toHaveBeenCalledWith("1.1.0");
    expect(screen.getByRole("button", { name: "安装并重启" })).toBeEnabled();
  });

  it("uses UNKNOWN when install fails without a command code", async () => {
    await openSettings(
      createMockApi({
        checkForUpdate: vi.fn().mockResolvedValue({
          available: true,
          current_version: "1.0.0",
          update: { version: "1.1.0" },
        }),
        installUpdate: vi.fn().mockRejectedValue(new Error("network")),
      }),
    );

    fireEvent.click(await screen.findByRole("button", { name: "安装并重启" }));
    fireEvent.click(
      within(await screen.findByRole("dialog", { name: "安装更新" })).getByRole(
        "button",
        { name: "继续安装" },
      ),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "无法下载或安装更新（UNKNOWN）",
    );
  });

  it("defaults to Chinese, switches to English, and stores only language", async () => {
    const api = await openSettings();
    const setItem = vi.spyOn(localStorage, "setItem");
    const languageSelect = screen.getByRole("combobox", { name: /界面语言/ });

    expect(languageSelect.parentElement).toHaveClass("language-select");

    fireEvent.change(languageSelect, {
      target: { value: "en" },
    });

    expect(
      screen.getByRole("heading", { name: "Settings" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Router control" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Agent configuration" }),
    ).toBeInTheDocument();
    expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe("en");
    expect(setItem).toHaveBeenCalledTimes(1);
    expect(setItem).toHaveBeenCalledWith(LANGUAGE_STORAGE_KEY, "en");
    await waitFor(() =>
      expect(api.setNativeLanguage).toHaveBeenLastCalledWith("en"),
    );
  });

  it("moves theme selection with arrows, Home, and End", async () => {
    await openSettings();
    const group = screen.getByRole("radiogroup", { name: "外观主题" });
    const warm = screen.getByRole("radio", { name: "暖沙" });
    const light = screen.getByRole("radio", { name: "浅色" });
    const dark = screen.getByRole("radio", { name: "深色" });

    expect(warm).toHaveAttribute("tabIndex", "0");
    expect(light).toHaveAttribute("tabIndex", "-1");
    warm.focus();
    fireEvent.keyDown(group, { key: "ArrowRight" });
    expect(light).toBeChecked();
    expect(light).toHaveFocus();
    fireEvent.keyDown(group, { key: "ArrowLeft" });
    expect(warm).toBeChecked();
    expect(warm).toHaveFocus();
    fireEvent.keyDown(group, { key: "ArrowLeft" });
    expect(dark).toBeChecked();
    expect(dark).toHaveFocus();
    fireEvent.keyDown(group, { key: "Home" });
    expect(warm).toBeChecked();
    expect(warm).toHaveFocus();
    fireEvent.keyDown(group, { key: "End" });
    expect(dark).toBeChecked();
    expect(dark).toHaveFocus();
  });

  it("switches appearance themes and persists only the theme key", async () => {
    await openSettings();
    const light = screen.getByRole("radio", { name: "浅色" });

    expect(screen.getByRole("radio", { name: "暖沙" })).toBeChecked();
    expect(document.documentElement.dataset.theme).toBe("warm");

    fireEvent.click(light);

    expect(light).toBeChecked();
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(document.documentElement.style.colorScheme).toBe("light");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("light");
    expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBeNull();
  });

  it("loads a stored dark theme and ignores unsupported stored themes", async () => {
    localStorage.setItem(THEME_STORAGE_KEY, "dark");
    const first = render(<App api={createMockApi()} />);
    fireEvent.click(await screen.findByRole("button", { name: /系统设置/ }));
    expect(await screen.findByRole("radio", { name: "深色" })).toBeChecked();
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(document.documentElement.style.colorScheme).toBe("dark");
    first.unmount();

    localStorage.setItem(THEME_STORAGE_KEY, "neon");
    render(<App api={createMockApi()} />);
    fireEvent.click(await screen.findByRole("button", { name: /系统设置/ }));
    expect(await screen.findByRole("radio", { name: "暖沙" })).toBeChecked();
    expect(document.documentElement.dataset.theme).toBe("warm");
  });

  it("loads valid English and ignores unsupported stored languages", async () => {
    localStorage.setItem(LANGUAGE_STORAGE_KEY, "en");
    const englishApi = createMockApi();
    const first = render(<App api={englishApi} />);
    expect(
      await screen.findByRole("heading", { name: "Router control" }),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(englishApi.setNativeLanguage).toHaveBeenCalledWith("en"),
    );
    first.unmount();

    localStorage.setItem(LANGUAGE_STORAGE_KEY, "fr");
    const fallbackApi = createMockApi();
    render(<App api={fallbackApi} />);
    expect(
      await screen.findByRole("heading", { name: "路由控制" }),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(fallbackApi.setNativeLanguage).toHaveBeenCalledWith("zh-CN"),
    );
  });

  it("prepares uninstall only after confirmation on supported platforms", async () => {
    const api = await openSettings();
    fireEvent.click(screen.getByRole("button", { name: "准备卸载并退出" }));
    const dialog = await screen.findByRole("dialog", {
      name: "准备卸载",
    });

    fireEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(api.prepareForUninstall).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "准备卸载并退出" }));
    fireEvent.click(
      within(await screen.findByRole("dialog", { name: "准备卸载" })).getByRole(
        "button",
        { name: "继续卸载" },
      ),
    );
    await waitFor(() => expect(api.prepareForUninstall).toHaveBeenCalledOnce());
  });

  it("omits uninstall preparation when the native platform does not support it", async () => {
    await openSettings(
      createMockApi({
        getDesktopPaths: vi.fn().mockResolvedValue({
          data_dir: "C:\\Users\\test\\AppData\\Roaming\\mtls-router",
          log_directory:
            "C:\\Users\\test\\AppData\\Roaming\\mtls-router\\mtls-router-logs",
          can_prepare_for_uninstall: false,
        }),
      }),
    );
    expect(
      screen.queryByRole("button", { name: "准备卸载并退出" }),
    ).not.toBeInTheDocument();
  });
});
