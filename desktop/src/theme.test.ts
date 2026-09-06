import { afterEach, describe, expect, it } from "vitest";

import {
  applyTheme,
  DEFAULT_THEME,
  isThemeId,
  readStoredTheme,
  THEME_STORAGE_KEY,
} from "./theme";

describe("theme persistence", () => {
  afterEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute("data-theme");
    document.documentElement.style.removeProperty("color-scheme");
  });

  it("accepts only the three appearance themes", () => {
    expect(isThemeId("warm")).toBe(true);
    expect(isThemeId("light")).toBe(true);
    expect(isThemeId("dark")).toBe(true);
    expect(isThemeId("neon")).toBe(false);
    expect(isThemeId(null)).toBe(false);
  });

  it("defaults to warm sand and ignores unsupported stored values", () => {
    expect(readStoredTheme()).toBe(DEFAULT_THEME);
    localStorage.setItem(THEME_STORAGE_KEY, "neon");
    expect(readStoredTheme()).toBe("warm");
  });

  it("reads a stored light or dark theme", () => {
    localStorage.setItem(THEME_STORAGE_KEY, "dark");
    expect(readStoredTheme()).toBe("dark");
  });

  it("applies color-scheme for native controls", () => {
    applyTheme("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(document.documentElement.style.colorScheme).toBe("dark");
    applyTheme("light");
    expect(document.documentElement.style.colorScheme).toBe("light");
  });
});
