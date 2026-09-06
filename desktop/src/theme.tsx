/* eslint-disable react-refresh/only-export-components */
import { createContext, useContext, useLayoutEffect, useState } from "react";

export const THEME_STORAGE_KEY = "mtls-router.theme";
export const THEMES = ["warm", "light", "dark"] as const;
export type ThemeId = (typeof THEMES)[number];
export const DEFAULT_THEME: ThemeId = "warm";

export function isThemeId(value: string | null | undefined): value is ThemeId {
  return THEMES.some((theme) => theme === value);
}

export function readStoredTheme(): ThemeId {
  try {
    const stored = localStorage.getItem(THEME_STORAGE_KEY);
    return isThemeId(stored) ? stored : DEFAULT_THEME;
  } catch {
    return DEFAULT_THEME;
  }
}

export function applyTheme(theme: ThemeId) {
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme =
    theme === "dark" ? "dark" : "light";
}

applyTheme(readStoredTheme());

interface ThemeValue {
  theme: ThemeId;
  setTheme: (theme: ThemeId) => void;
}

const ThemeContext = createContext<ThemeValue>({
  theme: DEFAULT_THEME,
  setTheme: () => undefined,
});

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [theme, setThemeState] = useState<ThemeId>(readStoredTheme);

  useLayoutEffect(() => {
    applyTheme(theme);
  }, [theme]);

  function setTheme(nextTheme: ThemeId) {
    setThemeState(nextTheme);
    try {
      localStorage.setItem(THEME_STORAGE_KEY, nextTheme);
    } catch {
      // The selected theme still applies for this session if storage is unavailable.
    }
  }

  return <ThemeContext value={{ theme, setTheme }}>{children}</ThemeContext>;
}

export function useTheme(): ThemeValue {
  return useContext(ThemeContext);
}
