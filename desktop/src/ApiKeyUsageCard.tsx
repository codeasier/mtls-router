import { useState } from "react";

import type {
  APIKeyUsage,
  APIKeyUsageBudgetPeriod,
  APIKeyUsageModel,
  APIKeyUsagePeriod,
  APIKeyUsageProviderQuota,
  APIKeyUsageQuota,
} from "./ipc";
import { useI18n } from "./i18n";
import type { TranslationKey } from "./locales/zh-CN";

const BEIJING_TIME_ZONE = "Asia/Shanghai";

const PERIOD_GROUPS: {
  id: "rolling" | "calendar";
  periods: APIKeyUsagePeriod[];
}[] = [
  { id: "rolling", periods: ["1h", "12h", "24h", "7d", "30d"] },
  { id: "calendar", periods: ["today", "this_week", "this_month"] },
];

type UsageSortColumn = "requests" | "tokens" | "cost";
type UsageSortDirection = "asc" | "desc";

const SORT_COLUMNS: { id: UsageSortColumn; label: TranslationKey }[] = [
  { id: "requests", label: "apikey.usage.metric.requests" },
  { id: "tokens", label: "apikey.usage.metric.tokens" },
  { id: "cost", label: "apikey.usage.metric.cost" },
];

const SORT_ARIA: Record<UsageSortDirection, "ascending" | "descending"> = {
  asc: "ascending",
  desc: "descending",
};

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

function formatQuotaCost(language: string, value: number) {
  return new Intl.NumberFormat(language, {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(value);
}

function formatQuotaAmount(language: string, value: number, unit: string) {
  return unit === "usd"
    ? formatQuotaCost(language, value)
    : formatCount(language, value);
}

function formatQuotaPercent(language: string, percent: number) {
  return new Intl.NumberFormat(language, {
    style: "percent",
    minimumFractionDigits: 0,
    maximumFractionDigits: 1,
  }).format(percent / 100);
}

function formatBeijingDateTime(language: string, value: string) {
  return new Intl.DateTimeFormat(language, {
    dateStyle: "medium",
    timeStyle: "short",
    timeZone: BEIJING_TIME_ZONE,
  }).format(new Date(value));
}

function providerLabel(provider: string, t: (key: TranslationKey) => string) {
  return provider === "*" ? t("apikey.usage.quotas.provider.all") : provider;
}

function budgetPeriodLabel(
  period: APIKeyUsageBudgetPeriod,
  t: (key: TranslationKey) => string,
) {
  return t(`apikey.usage.quotas.period.${period}`);
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

function quotaPercent(quota: { used: number; limit: number | null }) {
  if (quota.limit == null || quota.limit <= 0) return null;
  return Math.min(100, Math.max(0, (quota.used / quota.limit) * 100));
}

function providerQuotaLabel(
  language: string,
  quota: APIKeyUsageProviderQuota,
  t: (key: TranslationKey) => string,
) {
  const remaining = Math.max(0, quota.limit - quota.used);
  return `${formatQuotaCost(language, quota.used)} / ${formatQuotaCost(language, quota.limit)} · ${t("apikey.usage.quota.remaining")} ${formatQuotaCost(language, remaining)}`;
}

function QuotaMeter({
  title,
  amounts,
  percent,
  resetsAt,
}: {
  title: string;
  amounts: string;
  percent: number | null;
  resetsAt: string;
}) {
  const { language, t } = useI18n();
  const percentLabel =
    percent == null ? "" : formatQuotaPercent(language, percent);
  return (
    <article className="apikey-usage__quota">
      <header>
        <div className="apikey-usage__quota-meta">
          <strong>{title}</strong>
          <span>{amounts}</span>
        </div>
        {percentLabel ? (
          <span className="apikey-usage__quota-percent">{percentLabel}</span>
        ) : null}
      </header>
      {percent != null && (
        <div
          className="apikey-usage__bar"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(percent)}
          aria-valuetext={percentLabel}
        >
          <span style={{ width: `${percent}%` }} />
        </div>
      )}
      {resetsAt ? (
        <p>
          {t("apikey.usage.quota.resets")}: {resetsAt}
        </p>
      ) : null}
    </article>
  );
}

function modelTokens(row: APIKeyUsageModel) {
  return row.prompt_tokens + row.completion_tokens;
}

function sortValue(row: APIKeyUsageModel, column: UsageSortColumn) {
  if (column === "requests") return row.requests;
  if (column === "tokens") return modelTokens(row);
  return row.cost;
}

export function ApiKeyUsageCard({
  present,
  period,
  usage,
  loading,
  error,
  onPeriodChange,
  onRefresh,
  onNavigateToApiKeys,
}: {
  present: boolean;
  period: APIKeyUsagePeriod;
  usage: APIKeyUsage | null;
  loading: boolean;
  error: TranslationKey | "";
  onPeriodChange(period: APIKeyUsagePeriod): void;
  onRefresh(): void;
  onNavigateToApiKeys(): void;
}) {
  const { language, t } = useI18n();
  const [sortColumn, setSortColumn] = useState<UsageSortColumn>("requests");
  const [sortDirection, setSortDirection] =
    useState<UsageSortDirection>("desc");
  const state = !present
    ? "need-key"
    : loading
      ? "loading"
      : error
        ? "error"
        : usage
          ? "ready"
          : "idle";
  const modelRows = usage?.by_model ?? [];
  const modelNames = modelRows.map((row) => row.model);
  const modelSignature = modelNames.join("\n");
  const [filter, setFilter] = useState<{
    signature: string;
    selected: Set<string> | null;
  }>({ signature: "", selected: null });
  // A fresh snapshot may carry a different model set; a stale filter resets
  // to "all models" so it never hides rows the caller did not choose.
  const selectedModels =
    filter.signature === modelSignature ? filter.selected : null;
  const selectedSet = selectedModels ?? new Set(modelNames);
  const visibleRows = modelRows
    .filter((row) => selectedSet.has(row.model))
    .sort((a, b) => {
      const delta = sortValue(a, sortColumn) - sortValue(b, sortColumn);
      return sortDirection === "asc" ? delta : -delta;
    });
  const allSelected = selectedSet.size === modelNames.length;
  const filterSummary = allSelected
    ? t("apikey.usage.filter.all")
    : t("apikey.usage.filter.summary", {
        count: selectedSet.size,
        total: modelNames.length,
      });

  const empty =
    usage != null &&
    usage.summary.requests === 0 &&
    usage.summary.prompt_tokens === 0 &&
    usage.summary.completion_tokens === 0 &&
    usage.by_model.length === 0;
  const asOf = usage?.as_of ? formatBeijingDateTime(language, usage.as_of) : "";
  const resetsAt = usage?.quota?.resets_at
    ? formatBeijingDateTime(language, usage.quota.resets_at)
    : "";
  const providerQuotas = usage?.quotas ?? [];

  function toggleSort(column: UsageSortColumn) {
    if (column === sortColumn) {
      setSortDirection((current) => (current === "desc" ? "asc" : "desc"));
      return;
    }
    setSortColumn(column);
    setSortDirection("desc");
  }

  function toggleModel(model: string, checked: boolean) {
    const next = new Set(selectedModels ?? modelNames);
    if (checked) next.add(model);
    else next.delete(model);
    setFilter({ signature: modelSignature, selected: next });
  }

  return (
    <section className="apikey-usage" data-state={state}>
      <header className="apikey-usage__header">
        <div>
          <p className="overline">{t("apikey.usage.overline")}</p>
        </div>
        <div className="apikey-usage__toolbar">
          <div className="apikey-usage__period-groups">
            {PERIOD_GROUPS.map((group) => (
              <div key={group.id} className="apikey-usage__period-group">
                <p className="apikey-usage__period-group-label">
                  {t(`apikey.usage.periodGroup.${group.id}`)}
                </p>
                <div
                  className="apikey-usage__periods"
                  role="tablist"
                  aria-label={t(`apikey.usage.periodGroup.${group.id}`)}
                >
                  {group.periods.map((value) => (
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
              </div>
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
        <div className="apikey-usage__placeholder">
          <p>{t("apikey.usage.needKey")}</p>
          <button
            type="button"
            className="text-button"
            onClick={onNavigateToApiKeys}
          >
            {t("agents.issue.toApiKeys")}
          </button>
        </div>
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
            <QuotaMeter
              title={t("apikey.usage.quota.heading")}
              amounts={quotaLabel(language, usage.quota, t)}
              percent={quotaPercent(usage.quota)}
              resetsAt={resetsAt}
            />
          )}
          {providerQuotas.length > 0 && (
            <section className="apikey-usage__quotas">
              <strong className="apikey-usage__quotas-heading">
                {t("apikey.usage.quotas.heading")}
              </strong>
              {providerQuotas.map((quota, index) => (
                <QuotaMeter
                  key={`${quota.provider}:${quota.period}:${quota.resets_at}:${index}`}
                  title={`${providerLabel(quota.provider, t)} · ${budgetPeriodLabel(quota.period, t)}`}
                  amounts={providerQuotaLabel(language, quota, t)}
                  percent={quotaPercent(quota)}
                  resetsAt={formatBeijingDateTime(language, quota.resets_at)}
                />
              ))}
            </section>
          )}
          {empty ? (
            <p className="apikey-usage__placeholder">
              {t("apikey.usage.empty")}
            </p>
          ) : modelRows.length === 0 ? null : (
            <>
              <div className="apikey-usage__tablebar">
                <strong>{t("apikey.usage.models.heading")}</strong>
                <details className="apikey-usage__filter">
                  <summary>
                    <span>{t("apikey.usage.filter.label")}</span>
                    <span className="apikey-usage__filter-state">
                      {filterSummary}
                    </span>
                  </summary>
                  <div className="apikey-usage__filter-menu">
                    <label className="apikey-usage__filter-option">
                      <input
                        type="checkbox"
                        checked={allSelected}
                        onChange={(event) =>
                          setFilter({
                            signature: modelSignature,
                            selected: event.target.checked ? null : new Set(),
                          })
                        }
                      />
                      <span>{t("apikey.usage.filter.all")}</span>
                    </label>
                    {modelNames.map((model) => (
                      <label
                        key={model}
                        className="apikey-usage__filter-option"
                      >
                        <input
                          type="checkbox"
                          checked={selectedSet.has(model)}
                          onChange={(event) =>
                            toggleModel(model, event.target.checked)
                          }
                        />
                        <span>{model}</span>
                      </label>
                    ))}
                  </div>
                </details>
              </div>
              {visibleRows.length === 0 ? (
                <p className="apikey-usage__placeholder">
                  {t("apikey.usage.filter.empty")}
                </p>
              ) : (
                <table className="apikey-usage__models">
                  <thead>
                    <tr>
                      <th scope="col">{t("apikey.usage.models.model")}</th>
                      {SORT_COLUMNS.map((column) => (
                        <th
                          key={column.id}
                          scope="col"
                          aria-sort={
                            sortColumn === column.id
                              ? SORT_ARIA[sortDirection]
                              : "none"
                          }
                        >
                          <button
                            type="button"
                            className="apikey-usage__sort"
                            onClick={() => toggleSort(column.id)}
                          >
                            {t(column.label)}
                            <span aria-hidden="true">
                              {sortColumn === column.id
                                ? sortDirection === "desc"
                                  ? "↓"
                                  : "↑"
                                : "↕"}
                            </span>
                          </button>
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {visibleRows.map((row) => (
                      <tr key={row.model}>
                        <th scope="row">{row.model}</th>
                        <td>{formatCount(language, row.requests)}</td>
                        <td>{formatCount(language, modelTokens(row))}</td>
                        <td>{formatCost(language, row.cost)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </>
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
