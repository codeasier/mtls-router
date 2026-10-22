import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { IdentityPanel } from "./IdentityPanel";
import { I18nProvider } from "./i18n";
import type { IdentitySummary } from "./ipc";
import { createMockApi } from "./test/api";

const disconnected: IdentitySummary = {
  generation: 0,
  phase: "disconnected",
  user_id: null,
  device_id: null,
  expires_at: null,
  revocation_pending: false,
  error_code: null,
};
const connected: IdentitySummary = {
  ...disconnected,
  generation: 1,
  phase: "connected",
  user_id: "member",
  device_id: "device",
  expires_at: 1900000000,
};
const authorization = {
  verification_uri:
    "https://enroll.codeasier.top/enroll/native/verify?transaction=public",
  short_code: "ABCD-EFGH",
  browser_opened: true,
};

describe("IdentityPanel", () => {
  it("connects with public approval information and keeps local disablement distinct from remote revocation", async () => {
    let state = disconnected;
    const api = createMockApi({
      getIdentityStatus: vi.fn(async () => state),
      connectIdentity: vi.fn(async () => {
        state = { ...disconnected, generation: 1, phase: "authorizing" };
        return authorization;
      }),
      disconnectIdentity: vi.fn(async () => {
        state = {
          ...disconnected,
          generation: 2,
          phase: "revocation_pending",
          revocation_pending: true,
        };
        return state;
      }),
    });
    render(
      <I18nProvider>
        <IdentityPanel api={api} />
      </I18nProvider>,
    );
    await waitFor(() => expect(api.getIdentityStatus).toHaveBeenCalled());
    fireEvent.click(
      await screen.findByRole("button", { name: "连接 GitCode 账号" }),
    );
    expect(await screen.findByText(/ABCD-EFGH/)).toBeInTheDocument();
    expect(
      screen.getByText(authorization.verification_uri),
    ).toBeInTheDocument();
    fireEvent.click(await screen.findByRole("button", { name: "停用此设备" }));
    expect(await screen.findByText(/远端撤销待确认/)).toBeInTheDocument();
    expect(screen.queryByText("已撤销")).not.toBeInTheDocument();
    expect(api.getCredential).not.toHaveBeenCalled();
    expect(api.deleteCredential).not.toHaveBeenCalled();
  });
  it("does not replace a completed connection with a late old status response", async () => {
    let resolveOld!: (value: IdentitySummary) => void;
    const old = new Promise<IdentitySummary>((resolve) => {
      resolveOld = resolve;
    });
    const api = createMockApi({
      getIdentityStatus: vi
        .fn()
        .mockImplementationOnce(() => old)
        .mockResolvedValue(connected),
      connectIdentity: vi.fn().mockResolvedValue(authorization),
    });
    render(
      <I18nProvider>
        <IdentityPanel api={api} />
      </I18nProvider>,
    );
    fireEvent.click(
      await screen.findByRole("button", { name: "连接 GitCode 账号" }),
    );
    await waitFor(() =>
      expect(screen.getByText(/设备证书 · 已连接/)).toBeInTheDocument(),
    );
    await act(async () => resolveOld(disconnected));
    expect(screen.getByText(/设备证书 · 已连接/)).toBeInTheDocument();
  });
});
