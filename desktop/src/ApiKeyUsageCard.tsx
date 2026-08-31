import type { APIKeyUsage, APIKeyUsagePeriod, APIKeyUsageQuota } from "./ipc";
import { useI18n } from "./i18n";
import type { TranslationKey } from "./locales/zh-CN";

const PERIODS: APIKeyUsagePeriod[] = ["today", "7d", "30d"];

// eslint-disable-next-line react-refresh/only-export-components
export function usageErrorTranslation(error: unknown): TranslationKey {
  const code =
    error && typeof error === "object" && "code" in error
      ? String(error.code)
      : "";
  if (code === "USAGE_UNAVAILABLE") return "apikey.usage.error.unavailable";
  if (code === "USAGE_AUTH_FAILED" || code === "MODEL_AUTH_FAILED") {
    return "apikey.usage.error.auth";
  }
  if (code === "USAGE_RESPONSE_INVALID") return "apikey.usage.error.invalid";
  if (code === "USAGE_REQUEST_FAILED") return "apikey.usage.error.request";
  if (
    code === "ROUTER_NOT_FOUND" ||
    code === "ROUTER_NOT_READY" ||
    code === "ROUTER_STATE_STALE" ||
    code === "MODEL_DISCOVERY_FAILED" ||
    code === "MODEL_CATALOG_STALE"
  ) {
    return "apikey.usage.error.router";
  }
  if (code === "CREDENTIAL_INVALID") return "apikey.error.invalid";
  if (code === "CREDENTIAL_IO_ERROR") return "apikey.error.io";
  if (code === "CREDENTIAL_LOCK_TIMEOUT") return "apikey.error.lock";
  return "apikey.usage.error.load";
}

function formatCount(language: string, value: number) {
  return new Intl.NumberFormat(language).format(value);
}

function formatCost(language: string, value: number) {
  return new Intl.NumberFormat(language, {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 4,
  }).format(value);
}

function formatQuotaAmount(language: string, value: number, unit: string) {
  return unit === "usd"
    ? formatCost(language, value)
    : formatCount(language, value);
}

function quotaLabel(
  language: string,
  quota: APIKeyUsageQuota,
  t: (key: TranslationKey) => string,
) {
  const used = formatQuotaAmount(language, quota.used, quota.unit);
  if (quota.limit == null) {
    return `${used} · ${t("apikey.usage.quota.unlimited")}`;
  }
  const remaining = Math.max(0, quota.limit - quota.used);
  return `${used} / ${formatQuotaAmount(language, quota.limit, quota.unit)} · ${t("apikey.usage.quota.remaining")} ${formatQuotaAmount(language, remaining, quota.unit)}`;
}

function quotaPercent(quota: APIKeyUsageQuota) {
  if (quota.limit == null || quota.limit <= 0) return null;
  return Math.min(100, Math.max(0, (quota.used / quota.limit) * 100));
}

export function ApiKeyUsageCard({
  present,
  period,
  usage,
  loading,
  error,
  onPeriodChange,
  onRefresh,
}: {
  present: boolean;
  period: APIKeyUsagePeriod;
  usage: APIKeyUsage | null;
  loading: boolean;
  error: TranslationKey | "";
  onPeriodChange(period: APIKeyUsagePeriod): void;
  onRefresh(): void;
}) {
  const { language, t } = useI18n();
  const state = !present
    ? "need-key"
    : loading
      ? "loading"
      : error
        ? "error"
        : usage
          ? "ready"
          : "idle";
  const empty =
    usage != null &&
    usage.summary.requests === 0 &&
    usage.summary.prompt_tokens === 0 &&
    usage.summary.completion_tokens === 0 &&
    usage.by_model.length === 0;
  const asOf = usage?.as_of
    ? new Intl.DateTimeFormat(language, {
        dateStyle: "medium",
        timeStyle: "short",
      }).format(new Date(usage.as_of))
    : "";
  const resetsAt = usage?.quota?.resets_at
    ? new Intl.DateTimeFormat(language, {
        dateStyle: "medium",
        timeStyle: "short",
      }).format(new Date(usage.quota.resets_at))
    : "";

  return (
    <section
      className="apikey-usage"
      data-state={state}
      aria-labelledby="apikey-usage-heading"
    >
      <header className="apikey-usage__header">
        <div>
          <p className="overline">{t("apikey.usage.overline")}</p>
          <h3 id="apikey-usage-heading">{t("apikey.usage.heading")}</h3>
        </div>
        <div className="apikey-usage__toolbar">
          <div className="apikey-usage__periods" role="tablist">
            {PERIODS.map((value) => (
              <button
                key={value}
                type="button"
                role="tab"
                aria-selected={period === value}
                className="apikey-usage__period"
                disabled={loading}
                onClick={() => onPeriodChange(value)}
              >
                {t(`apikey.usage.period.${value}`)}
              </button>
            ))}
          </div>
          <button
            type="button"
            className="text-button"
            disabled={!present || loading}
            onClick={() => onRefresh()}
          >
            {t(loading ? "apikey.usage.refreshing" : "apikey.usage.refresh")}
          </button>
        </div>
      </header>
      <p className="apikey-usage__note">{t("apikey.usage.note")}</p>

      {!present && (
        <p className="apikey-usage__placeholder">{t("apikey.usage.needKey")}</p>
      )}
      {present && loading && !usage && (
        <p className="apikey-usage__placeholder">{t("apikey.usage.loading")}</p>
      )}
      {present && error && (
        <p className="apikey-usage__alert" role="alert">
          {t(error)}
        </p>
      )}
      {present && usage && !error && (
        <>
          <dl className="apikey-usage__metrics">
            <div>
              <dt>{t("apikey.usage.metric.requests")}</dt>
              <dd>{formatCount(language, usage.summary.requests)}</dd>
            </div>
            <div>
              <dt>{t("apikey.usage.metric.tokens")}</dt>
              <dd>
                {formatCount(
                  language,
                  usage.summary.prompt_tokens + usage.summary.completion_tokens,
                )}
              </dd>
              <small>
                {t("apikey.usage.metric.prompt")}{" "}
                {formatCount(language, usage.summary.prompt_tokens)} ·{" "}
                {t("apikey.usage.metric.completion")}{" "}
                {formatCount(language, usage.summary.completion_tokens)}
              </small>
            </div>
            <div>
              <dt>{t("apikey.usage.metric.cost")}</dt>
              <dd>{formatCost(language, usage.summary.cost)}</dd>
            </div>
          </dl>
          {usage.quota && (
            <section className="apikey-usage__quota">
              <header>
                <strong>{t("apikey.usage.quota.heading")}</strong>
                <span>{quotaLabel(language, usage.quota, t)}</span>
              </header>
              {quotaPercent(usage.quota) != null && (
                <div
                  className="apikey-usage__bar"
                  role="progressbar"
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={Math.round(quotaPercent(usage.quota) ?? 0)}
                >
                  <span
                    style={{ width: `${quotaPercent(usage.quota) ?? 0}%` }}
                  />
                </div>
              )}
              {resetsAt && (
                <p>
                  {t("apikey.usage.quota.resets")}: {resetsAt}
                </p>
              )}
            </section>
          )}
          {empty ? (
            <p className="apikey-usage__placeholder">
              {t("apikey.usage.empty")}
            </p>
          ) : usage.by_model.length === 0 ? null : (
            <table className="apikey-usage__models">
              <caption>{t("apikey.usage.models.heading")}</caption>
              <thead>
                <tr>
                  <th scope="col">{t("apikey.usage.models.model")}</th>
                  <th scope="col">{t("apikey.usage.metric.requests")}</th>
                  <th scope="col">{t("apikey.usage.metric.tokens")}</th>
                  <th scope="col">{t("apikey.usage.metric.cost")}</th>
                </tr>
              </thead>
              <tbody>
                {usage.by_model.map((row) => (
                  <tr key={row.model}>
                    <th scope="row">{row.model}</th>
                    <td>{formatCount(language, row.requests)}</td>
                    <td>
                      {formatCount(
                        language,
                        row.prompt_tokens + row.completion_tokens,
                      )}
                    </td>
                    <td>{formatCost(language, row.cost)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {asOf && (
            <p className="apikey-usage__asof">
              {t("apikey.usage.asOf")}: {asOf}
            </p>
          )}
        </>
      )}
    </section>
  );
}
