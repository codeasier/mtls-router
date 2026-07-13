/* eslint-disable react-refresh/only-export-components */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
} from "react";

import { en } from "./locales/en";
import { zhCN, type TranslationKey } from "./locales/zh-CN";
import type { NativeLanguage } from "./ipc";

export type Language = NativeLanguage;
export type Translator = (
  key: TranslationKey,
  variables?: Record<string, string | number>,
) => string;

export const LANGUAGE_STORAGE_KEY = "mtls-router.language";
export const resources: Record<Language, Record<TranslationKey, string>> = {
  "zh-CN": zhCN,
  en,
};

type Catalogs = Record<string, Partial<Record<string, string>>>;

export function resolveTranslation(
  language: string,
  key: string,
  catalogs: Catalogs = resources,
): string {
  return catalogs[language]?.[key] ?? catalogs["zh-CN"]?.[key] ?? key;
}

function interpolate(
  value: string,
  variables: Record<string, string | number> = {},
): string {
  return value.replace(/\{([^}]+)\}/g, (match, name: string) =>
    Object.hasOwn(variables, name) ? String(variables[name]) : match,
  );
}

function initialLanguage(): Language {
  try {
    return localStorage.getItem(LANGUAGE_STORAGE_KEY) === "en" ? "en" : "zh-CN";
  } catch {
    return "zh-CN";
  }
}

interface I18nValue {
  language: Language;
  setLanguage: (language: Language) => void;
  t: Translator;
}

const defaultTranslator: Translator = (key, variables) =>
  interpolate(resolveTranslation("zh-CN", key), variables);
const I18nContext = createContext<I18nValue>({
  language: "zh-CN",
  setLanguage: () => undefined,
  t: defaultTranslator,
});

export function I18nProvider({
  children,
  synchronizeNativeLanguage,
}: {
  children: React.ReactNode;
  synchronizeNativeLanguage?: (language: Language) => Promise<void>;
}) {
  const [language, setLanguageState] = useState<Language>(initialLanguage);

  useEffect(() => {
    document.documentElement.lang = language;
    void synchronizeNativeLanguage?.(language).catch(() => undefined);
  }, [language, synchronizeNativeLanguage]);

  function setLanguage(nextLanguage: Language) {
    setLanguageState(nextLanguage);
    try {
      localStorage.setItem(LANGUAGE_STORAGE_KEY, nextLanguage);
    } catch {
      // The selected language still applies for this session if storage is unavailable.
    }
  }

  const t = useCallback<Translator>(
    (key, variables) =>
      interpolate(resolveTranslation(language, key), variables),
    [language],
  );

  return (
    <I18nContext value={{ language, setLanguage, t }}>{children}</I18nContext>
  );
}

export function useI18n(): I18nValue {
  return useContext(I18nContext);
}
