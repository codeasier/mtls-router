import { useCallback, useEffect, useRef, useState } from "react";
import { useI18n } from "./i18n";
import type { DesktopApi, IdentityAuthorization, IdentitySummary } from "./ipc";
import type { TranslationKey } from "./locales/zh-CN";

const phases = new Set([
  "disconnected",
  "authorizing",
  "connected",
  "renewing",
  "expired",
  "reauth_required",
  "revocation_pending",
  "rejected",
]);

export function IdentityPanel({ api }: { api: DesktopApi }) {
  const { t } = useI18n();
  const [summary, setSummary] = useState<IdentitySummary | null>(null);
  const [authorization, setAuthorization] =
    useState<IdentityAuthorization | null>(null);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const epoch = useRef({ value: 0 });
  const working = useRef(false);
  const refresh = useCallback(async () => {
    if (!api.getIdentityStatus || working.current) return;
    const current = ++epoch.current.value;
    try {
      const next = await api.getIdentityStatus();
      if (current !== epoch.current.value) return;
      setSummary(next);
      setFailed(Boolean(next.error_code));
      if (next.phase !== "authorizing") setAuthorization(null);
    } catch {
      if (current === epoch.current.value) setFailed(true);
    }
  }, [api]);
  useEffect(() => {
    const guard = epoch.current;
    void Promise.resolve().then(refresh);
    const timer = window.setInterval(() => void refresh(), 5000);
    return () => {
      window.clearInterval(timer);
      guard.value++;
    };
  }, [refresh]);
  async function act(operation: "connect" | "renew" | "disconnect") {
    if (working.current) return;
    working.current = true;
    const current = ++epoch.current.value;
    setBusy(true);
    setFailed(false);
    try {
      if (operation === "connect") {
        const next = await api.connectIdentity?.();
        if (current === epoch.current.value && next) setAuthorization(next);
      } else if (operation === "renew") {
        await api.renewIdentity?.();
      } else {
        await api.disconnectIdentity?.();
        if (current === epoch.current.value) setAuthorization(null);
      }
    } catch {
      if (current === epoch.current.value) setFailed(true);
    } finally {
      working.current = false;
      if (current === epoch.current.value) {
        setBusy(false);
        void refresh();
      }
    }
  }
  if (!api.getIdentityStatus) return null;
  const phase =
    summary && phases.has(summary.phase) ? summary.phase : "disconnected";
  const active = phase === "connected" || phase === "renewing";
  return (
    <section
      className="identity-panel"
      aria-label={t("identity.title")}
      aria-busy={busy}
    >
      <div className="identity-panel__copy">
        <strong>
          {t("identity.title")} ·{" "}
          {t(`identity.phase.${phase}` as TranslationKey)}
        </strong>
        <p>{t("identity.keyIndependent")}</p>
        {summary?.user_id && (
          <small>{t("identity.account", { user: summary.user_id })}</small>
        )}
        {summary?.expires_at && (
          <small>
            {t("identity.expires", {
              date: new Date(summary.expires_at * 1000).toLocaleString(),
            })}
          </small>
        )}
        {summary?.revocation_pending && (
          <small role="status">{t("identity.remotePending")}</small>
        )}
        {authorization && (
          <p role="status">
            {t("identity.confirm", { code: authorization.short_code })}
            <br />
            <code>{authorization.verification_uri}</code>
          </p>
        )}
        {failed && <p role="alert">{t("identity.failed")}</p>}
      </div>
      <div className="action-row">
        {!active && (
          <button
            className="control-button"
            disabled={busy}
            onClick={() => void act("connect")}
          >
            {t("identity.connect")}
          </button>
        )}
        {active && (
          <button
            className="text-button"
            disabled={busy}
            onClick={() => void act("renew")}
          >
            {t("identity.renew")}
          </button>
        )}
        {(active || phase === "authorizing" || phase === "expired") && (
          <button
            className="text-button"
            disabled={busy}
            onClick={() => void act("disconnect")}
          >
            {t("identity.disconnect")}
          </button>
        )}
      </div>
    </section>
  );
}
