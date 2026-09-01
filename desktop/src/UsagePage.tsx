import { useCallback, useEffect, useRef, useState } from "react";

import { ApiKeyUsageCard, usageErrorTranslation } from "./ApiKeyUsageCard";
import { useI18n } from "./i18n";
import type { APIKeyUsage, APIKeyUsagePeriod, DesktopApi } from "./ipc";
import type { TranslationKey } from "./locales/zh-CN";

export function UsagePage({
  api,
  onNavigateToApiKeys,
}: {
  api: DesktopApi;
  onNavigateToApiKeys(): void;
}) {
  const { t } = useI18n();
  const [present, setPresent] = useState<boolean | null>(null);
  const [period, setPeriod] = useState<APIKeyUsagePeriod>("7d");
  const [usage, setUsage] = useState<APIKeyUsage | null>(null);
  const [usageLoading, setUsageLoading] = useState(false);
  const [usageError, setUsageError] = useState<TranslationKey | "">("");
  const usageGeneration = useRef(0);

  const loadUsage = useCallback(
    (nextPeriod: APIKeyUsagePeriod, retainSnapshot = false) => {
      const generation = ++usageGeneration.current;
      setUsageLoading(true);
      setUsageError("");
      if (!retainSnapshot) {
        setUsage(null);
      }
      void api
        .getAPIKeyUsage(nextPeriod)
        .then((snapshot) => {
          if (generation !== usageGeneration.current) return;
          setUsage(snapshot);
          setUsageLoading(false);
        })
        .catch((loadError: unknown) => {
          if (generation !== usageGeneration.current) return;
          setUsage(null);
          setUsageError(usageErrorTranslation(loadError));
          setUsageLoading(false);
        });
    },
    [api],
  );

  const clearUsage = useCallback(() => {
    usageGeneration.current += 1;
    setUsage(null);
    setUsageError("");
    setUsageLoading(false);
  }, []);

  useEffect(() => {
    let current = true;
    void api
      .getCredential()
      .then((summary) => {
        if (!current) return;
        setPresent(summary.present);
        if (summary.present) {
          loadUsage("7d");
        } else {
          clearUsage();
        }
      })
      .catch(() => {
        if (!current) return;
        setPresent(false);
        setUsageError("apikey.usage.error.load");
        clearUsage();
      });
    return () => {
      current = false;
    };
  }, [api, clearUsage, loadUsage]);

  return (
    <section className="apikey-panel" aria-label={t("section.usage.title")}>
      <ApiKeyUsageCard
        present={Boolean(present)}
        period={period}
        usage={usage}
        loading={usageLoading}
        error={usageError}
        onPeriodChange={(next) => {
          setPeriod(next);
          if (present) {
            loadUsage(next);
          }
        }}
        onRefresh={() => {
          if (present) {
            loadUsage(period, true);
          }
        }}
        onNavigateToApiKeys={onNavigateToApiKeys}
      />
    </section>
  );
}
