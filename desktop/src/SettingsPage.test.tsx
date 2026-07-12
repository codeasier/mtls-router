import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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
  it("shows autostart, versions, and application locations without sensitive controls", async () => {
    await openSettings();

    expect(screen.getByRole("switch", { name: /开机时启动/ })).toBeChecked();
    expect(screen.getByText("desktop-v1")).toBeInTheDocument();
    expect(screen.getByText("manager-v1")).toBeInTheDocument();
    expect(screen.getByText("router-v1")).toBeInTheDocument();
    expect(screen.getByText("/safe/app-data")).toBeInTheDocument();
    expect(
      screen.getByText("/safe/app-data/mtls-router.log"),
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

  it("defaults to Chinese, switches to English, and stores only language", async () => {
    const api = await openSettings();
    const setItem = vi.spyOn(localStorage, "setItem");

    fireEvent.change(screen.getByRole("combobox", { name: /界面语言/ }), {
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
          log_file:
            "C:\\Users\\test\\AppData\\Roaming\\mtls-router\\mtls-router.log",
          can_prepare_for_uninstall: false,
        }),
      }),
    );
    expect(
      screen.queryByRole("button", { name: "准备卸载并退出" }),
    ).not.toBeInTheDocument();
  });
});
