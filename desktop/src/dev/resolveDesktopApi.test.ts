import { describe, expect, it } from "vitest";

import {
  desktopApiEnvFromImportMeta,
  shouldUseMockDesktopApi,
} from "./resolveDesktopApi";

describe("shouldUseMockDesktopApi", () => {
  it("requires both DEV and explicit VITE_MOCK=true", () => {
    expect(
      shouldUseMockDesktopApi({ DEV: true, PROD: false, VITE_MOCK: "true" }),
    ).toBe(true);
    expect(
      shouldUseMockDesktopApi({ DEV: true, PROD: false, VITE_MOCK: "1" }),
    ).toBe(false);
    expect(
      shouldUseMockDesktopApi({ DEV: true, PROD: false, VITE_MOCK: undefined }),
    ).toBe(false);
    expect(
      shouldUseMockDesktopApi({ DEV: false, PROD: false, VITE_MOCK: "true" }),
    ).toBe(false);
  });

  it("never enables mock under production builds", () => {
    expect(
      shouldUseMockDesktopApi({ DEV: false, PROD: true, VITE_MOCK: "true" }),
    ).toBe(false);
    expect(
      shouldUseMockDesktopApi({ DEV: true, PROD: true, VITE_MOCK: "true" }),
    ).toBe(false);
  });
});

describe("desktopApiEnvFromImportMeta", () => {
  it("reads DEV/PROD/VITE_MOCK from the Vite env object", () => {
    expect(
      desktopApiEnvFromImportMeta({
        DEV: true,
        PROD: false,
        VITE_MOCK: "true",
        BASE_URL: "/",
        MODE: "development",
        SSR: false,
      }),
    ).toEqual({ DEV: true, PROD: false, VITE_MOCK: "true" });
  });
});
