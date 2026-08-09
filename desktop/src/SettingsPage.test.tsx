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
import { createMockApi } from "./test/api";

async function openSettings(api = createMockApi()) {
  render(<App api={api} />);
  fireEvent.click(await screen.findByRole("button", { name: /系统设置/ }));
  await screen.findByRole("heading", { name: "桌面控制面板" });
  return api;
}

beforeEach(() => localStorage.clear());

describe("SettingsPage", () => {
  it("shows all component versions and application locations without sensitive controls", async () => {
    await openSettings();

    expect(screen.getByRole("switch", { name: /开机时启动/ })).toBeChecked();
    expect(screen.getAllByText("desktop-v1")).not.toHaveLength(0);
    expect(screen.getByText("manager-v1")).toBeInTheDocument();
    expect(screen.getByText("router-v1")).toBeInTheDocument();
    expect(
      within(screen.getByRole("list", { name: "组件版本" })).getAllByRole(
        "listitem",
      ),
    ).toHaveLength(3);
    expect(screen.getByText("/safe/app-data")).toBeInTheDocument();
    expect(
      screen.getByText("/safe/app-data/mtls-router-logs"),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(/上游 URL|证书导入|自动更新|PATH/),
    ).not.toBeInTheDocument();
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

    vi.spyOn(window, "confirm").mockReturnValueOnce(false);
    fireEvent.click(screen.getByRole("button", { name: "安装并重启" }));
    expect(api.installUpdate).not.toHaveBeenCalled();

    vi.spyOn(window, "confirm").mockReturnValueOnce(true);
    fireEvent.click(screen.getByRole("button", { name: "安装并重启" }));
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
    vi.spyOn(window, "confirm").mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: "准备卸载并退出" }));
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
