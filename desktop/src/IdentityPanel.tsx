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

const errorMessages: Record<string, TranslationKey> = {
  CERT_SERVICE_UNAVAILABLE: "identity.error.certServiceUnavailable",
  CERT_REAUTH_REQUIRED: "identity.error.certReauthRequired",
  CERT_OPERATION_CONFLICT: "identity.error.operationConflict",
  IDENTITY_STORAGE_UNAVAILABLE: "identity.error.storageUnavailable",
  IDENTITY_BUSY: "identity.error.busy",
  IDENTITY_STATE_INVALID: "identity.error.invalidState",
};

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
  if (active && !failed && !summary?.error_code) return null;
  const errorText =
    (summary?.error_code && errorMessages[summary.error_code]) ||
    "identity.failed";
  return (
    <div className="dialog-backdrop identity-backdrop">
      <section
        className="identity-panel"
        role="dialog"
        aria-modal="true"
        aria-label={t("identity.title")}
        aria-busy={busy}
      >
        <header className="identity-panel__header">
          <div className="identity-panel__brand">
            <span className="brand-mark" aria-hidden="true">
              CR
            </span>
            <div>
              <p className="overline">CodeasierRouter</p>
              <h2>{t("identity.title")}</h2>
            </div>
          </div>
          <span className="signal">
            {t(`identity.phase.${phase}` as TranslationKey)}
          </span>
        </header>
        <div className="identity-panel__copy">
          <p>{t("identity.keyIndependent")}</p>
          {summary?.revocation_pending && (
            <small role="status">{t("identity.remotePending")}</small>
          )}
          {authorization && (
            <div className="identity-panel__code" role="status">
              <p>{t("identity.confirm", { code: authorization.short_code })}</p>
              <strong className="identity-panel__short-code">
                {authorization.short_code}
              </strong>
              <code>{authorization.verification_uri}</code>
            </div>
          )}
          {(failed || summary?.error_code) && (
            <p className="identity-panel__error" role="alert">
              {t(errorText)}
            </p>
          )}
        </div>
        <div className="action-row identity-panel__actions">
          {!active && (
            <button
              type="button"
              className="control-button"
              disabled={busy}
              onClick={() => void act("connect")}
            >
              {t("identity.connect")}
            </button>
          )}
          {(phase === "authorizing" || phase === "expired") && (
            <button
              type="button"
              className="text-button"
              disabled={busy}
              onClick={() => void act("disconnect")}
            >
              {t("identity.disconnect")}
            </button>
          )}
        </div>
      </section>
    </div>
  );
}
