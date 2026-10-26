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
  it("blocks the app with a modal gate during OAuth and keeps local reset distinct from remote sign-out", async () => {
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
    expect(
      screen.getByRole("dialog", { name: "GitCode 身份认证" }),
    ).toHaveAttribute("aria-modal", "true");
    fireEvent.click(
      await screen.findByRole("button", { name: "连接 GitCode 账号" }),
    );
    expect(await screen.findByText("ABCD-EFGH")).toBeInTheDocument();
    expect(
      screen.getByText(authorization.verification_uri),
    ).toBeInTheDocument();
    fireEvent.click(await screen.findByRole("button", { name: "取消并重置" }));
    expect(await screen.findByText(/远端退出待确认/)).toBeInTheDocument();
    expect(screen.queryByText("已撤销")).not.toBeInTheDocument();
    expect(api.getCredential).not.toHaveBeenCalled();
    expect(api.deleteCredential).not.toHaveBeenCalled();
  });
  it("reports a concrete authentication-service failure instead of waiting for a browser", async () => {
    const api = createMockApi({
      getIdentityStatus: vi.fn(async () => ({
        ...disconnected,
        error_code: "CERT_SERVICE_UNAVAILABLE",
      })),
      connectIdentity: vi.fn(),
    });
    render(
      <I18nProvider>
        <IdentityPanel api={api} />
      </I18nProvider>,
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      /认证服务暂时不可用/,
    );
    expect(screen.queryByText(/检查网络/)).not.toBeInTheDocument();
    expect(screen.queryByText(/等待浏览器授权/)).not.toBeInTheDocument();
  });
  it("distinguishes a missing organization membership from a network failure", async () => {
    const api = createMockApi({
      getIdentityStatus: vi.fn(async () => ({
        ...disconnected,
        phase: "authorizing",
        error_code: "ORG_MEMBERSHIP_REQUIRED",
      })),
    });
    render(
      <I18nProvider>
        <IdentityPanel api={api} />
      </I18nProvider>,
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      /不是 msinsight-org 组织成员/,
    );
    expect(screen.queryByText(/检查网络/)).not.toBeInTheDocument();
  });
  it("asks for a network check only when the authentication service cannot be reached", async () => {
    const api = createMockApi({
      getIdentityStatus: vi.fn(async () => ({
        ...disconnected,
        error_code: "IDENTITY_NETWORK_UNAVAILABLE",
      })),
    });
    render(
      <I18nProvider>
        <IdentityPanel api={api} />
      </I18nProvider>,
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(/请检查网络/);
  });
  it("tells the user which GitCode scopes a new authorization needs", async () => {
    const api = createMockApi({
      getIdentityStatus: vi.fn(async () => ({
        ...disconnected,
        error_code: "CERT_REAUTH_REQUIRED",
      })),
    });
    render(
      <I18nProvider>
        <IdentityPanel api={api} />
      </I18nProvider>,
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(/all_user/);
    expect(screen.getByRole("alert")).toHaveTextContent(/read_groups/);
  });
  it("hides the modal gate once authenticated and ignores late stale status responses", async () => {
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
      expect(
        screen.queryByRole("dialog", { name: "GitCode 身份认证" }),
      ).not.toBeInTheDocument(),
    );
    await act(async () => resolveOld(disconnected));
    expect(
      screen.queryByRole("dialog", { name: "GitCode 身份认证" }),
    ).not.toBeInTheDocument();
  });
});
