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

  it("describes OS protection without claiming reliable PPL detection", () => {
    expect(en["router.occupant.reason.insufficientPrivilege"]).toMatch(/PPL/);
    expect(en["router.occupant.reason.insufficientPrivilege"]).toMatch(
      /cannot reliably distinguish/,
    );
    expect(zhCN["router.occupant.reason.insufficientPrivilege"]).toMatch(/PPL/);
    expect(zhCN["router.occupant.reason.insufficientPrivilege"]).toMatch(
      /无法可靠区分/,
    );
    expect(en["router.occupant.reason.protectedProcess"]).toMatch(
      /known application lifecycle/,
    );
    expect(zhCN["router.occupant.reason.protectedProcess"]).toMatch(
      /应用生命周期/,
    );
  });

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

  it.each([en, zhCN])(
    "states cleanup authentication and retention boundaries explicitly",
    (catalog) => {
      expect(catalog["agents.cleanup.authRemoved"]).toMatch(/auth|认证/i);
      expect(catalog["agents.cleanup.globalKeyRetained"]).toMatch(
        /global|全局/i,
      );
      expect(catalog["agents.cleanup.globalKeyRetained"]).toMatch(/not|不会/i);
      expect(catalog["agents.cleanup.backupsRetained"]).toMatch(
        /historical|历史/i,
      );
      expect(catalog["agents.cleanup.backupsRetained"]).toMatch(/remain|保留/i);
    },
  );

  it("describes cleanup completion without claiming detection already refreshed", () => {
    expect(en["agents.cleanup.completeNote"]).toMatch(/cleanup.*complete/i);
    expect(en["agents.cleanup.completeNote"]).toMatch(/backup/i);
    expect(en["agents.cleanup.completeNote"]).not.toMatch(/detect|refresh/i);
    expect(zhCN["agents.cleanup.completeNote"]).toMatch(/清理.*完成/);
    expect(zhCN["agents.cleanup.completeNote"]).toMatch(/备份/);
    expect(zhCN["agents.cleanup.completeNote"]).not.toMatch(/检测|刷新/);
  });

  it("keeps stale router copy distinct from healthy and degraded", () => {
    for (const catalog of [en, zhCN]) {
      expect(catalog["router.state.stale.title"]).toBeTruthy();
      expect(catalog["router.state.stale.detail"]).toBeTruthy();
      expect(catalog["router.state.stale.signal"]).toBeTruthy();
      expect(catalog["router.state.stale.title"]).not.toEqual(
        catalog["router.state.healthy.title"],
      );
      expect(catalog["router.state.stale.title"]).not.toEqual(
        catalog["router.state.degraded.title"],
      );
      expect(catalog["router.state.stale.signal"]).not.toEqual(
        catalog["router.state.degraded.signal"],
      );
      expect(catalog["router.state.stale.detail"]).toMatch(
        /not a current healthy|不能当作健康/,
      );
    }
  });

  it("keeps pending router copy distinct from degraded and stale", () => {
    for (const catalog of [en, zhCN]) {
      expect(catalog["router.state.pending.title"]).toBeTruthy();
      expect(catalog["router.state.pending.detail"]).toBeTruthy();
      expect(catalog["router.state.pending.signal"]).toBeTruthy();
      expect(catalog["router.state.pending.title"]).not.toEqual(
        catalog["router.state.degraded.title"],
      );
      expect(catalog["router.state.pending.title"]).not.toEqual(
        catalog["router.state.stale.title"],
      );
      expect(catalog["router.state.pending.detail"]).not.toMatch(/failed|失败/);
    }
  });

  it("keeps Settings and Logs status copy bilingual and distinct", () => {
    for (const catalog of [en, zhCN]) {
      expect(catalog["logs.loading"]).toBeTruthy();
      expect(catalog["logs.empty"]).toBeTruthy();
      expect(catalog["logs.error.load"]).toBeTruthy();
      expect(catalog["settings.general"]).toBeTruthy();
      expect(catalog["settings.components"]).toBeTruthy();
      expect(catalog["settings.locations"]).toBeTruthy();
      expect(catalog["settings.prepareTitle"]).toBeTruthy();
      expect(catalog["settings.error.load"]).toBeTruthy();
      expect(catalog["logs.loading"]).not.toEqual(catalog["logs.empty"]);
    }
  });

  it("keeps ambiguous cleanup recovery and explicit finish actions bilingual", () => {
    expect(en["agents.cleanup.ambiguous"]).toMatch(/confirm|unknown/i);
    expect(zhCN["agents.cleanup.ambiguous"]).toMatch(/无法确认|未知/);
    expect(en["agents.cleanup.repreview"]).toMatch(/^Re-preview/);
    expect(zhCN["agents.cleanup.repreview"]).toMatch(/^重新预览/);
    expect(en["agents.cleanup.finish"]).toMatch(/return.*Agent overview/i);
    expect(zhCN["agents.cleanup.finish"]).toMatch(/返回 Agent 概览/);
  });
});
