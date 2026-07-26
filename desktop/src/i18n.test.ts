import { describe, expect, it } from "vitest";

import { resolveTranslation } from "./i18n";
import { en } from "./locales/en";
import { zhCN } from "./locales/zh-CN";

describe("localization resources", () => {
  it("keeps Chinese and English catalogs complete", () => {
    expect(Object.keys(en).sort()).toEqual(Object.keys(zhCN).sort());
  });

  it.each([en, zhCN])(
    "keeps reoccupation guidance platform-neutral in each catalog",
    (catalog) => {
      const guidance =
        catalog["router.occupant.observation.supervisorGuidance"];
      expect(guidance).toMatch(/SCM/);
      expect(guidance).toMatch(/systemd/);
      expect(guidance).toMatch(/launchd/);
    },
  );

  it("falls back to Chinese when the active catalog is missing a key", () => {
    expect(
      resolveTranslation("en", "sample.key", {
        "zh-CN": { "sample.key": "中文回退" },
        en: {},
      }),
    ).toBe("中文回退");
  });

  it("returns the key when all catalogs are missing it", () => {
    expect(
      resolveTranslation("en", "missing.key", { "zh-CN": {}, en: {} }),
    ).toBe("missing.key");
  });
});
