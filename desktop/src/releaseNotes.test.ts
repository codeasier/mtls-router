import { describe, expect, it } from "vitest";

import {
  bundledReleaseNotes,
  changelogHistory,
  changelogSection,
  GITHUB_RELEASES_URL,
  localizedReleaseNotes,
  publishedDay,
  releaseHistory,
} from "./releaseNotes";

const sample = `# Changelog

## Unreleased

Not this one.

## v1.2.3 - 2026-01-02

First paragraph.

### Added

- A visible item.

---

## v1.2.2 - 2026-01-01

Older release.
`;

describe("changelogSection", () => {
  it("returns only the requested stable section", () => {
    expect(changelogSection(sample, "v1.2.3")).toBe(
      "First paragraph.\n\n### Added\n\n- A visible item.",
    );
    expect(changelogSection(sample, "1.2.2")).toBe("Older release.");
    expect(changelogSection(sample, "1.2.3-rc.1")).toBeNull();
    expect(changelogSection(sample, "9.9.9")).toBeNull();
  });
});

describe("localizedReleaseNotes", () => {
  const feed = "简体中文\n\n中文说明。\n\nEnglish\n\nEnglish note.\n";

  it("selects the active language and keeps unmarked notes intact", () => {
    expect(localizedReleaseNotes(feed, "zh-CN")).toBe("中文说明。");
    expect(localizedReleaseNotes(feed, "en")).toBe("English note.");
    expect(
      localizedReleaseNotes("Security and reliability fixes.", "zh-CN"),
    ).toBe("Security and reliability fixes.");
  });
});

describe("bundledReleaseNotes", () => {
  it("reads the shipped 0.5.3 section in the active language", () => {
    const chinese = bundledReleaseNotes("0.5.3", "zh-CN");
    const english = bundledReleaseNotes("0.5.3", "en");
    expect(chinese).toContain("回环端口");
    expect(chinese).not.toContain("This patch release");
    expect(english).toContain("loopback");
    expect(english).not.toContain("Chat images workbench");
    expect(bundledReleaseNotes("desktop-v1", "zh-CN")).toBeNull();
  });
});

describe("releaseHistory", () => {
  it("lists stable sections newest first and skips unreleased text", () => {
    const history = changelogHistory(sample);
    expect(history.map((entry) => entry.version)).toEqual(["1.2.3", "1.2.2"]);
    expect(history[0]?.date).toBe("2026-01-02");
    expect(history[0]?.body).not.toContain("Not this one.");
  });

  it("prepends a pending version that the build does not contain", () => {
    const history = releaseHistory("zh-CN", {
      version: "v9.9.9",
      notes: "简体中文\n\n待安装说明。\n\nEnglish\n\nPending note.\n",
      published_at: "2026-10-25T00:00:00Z",
    });
    expect(history[0]).toMatchObject({
      version: "9.9.9",
      date: "2026-10-25",
      body: "待安装说明。",
    });
    expect(history.some((entry) => entry.version === "0.6.0")).toBe(true);
    expect(GITHUB_RELEASES_URL).toBe(
      "https://github.com/codeasier/mtls-router/releases",
    );
  });
});

describe("publishedDay", () => {
  it("keeps the calendar day from an updater timestamp", () => {
    expect(publishedDay("2026-01-02T00:00:00Z")).toBe("2026-01-02");
    expect(publishedDay("2026-09-10 0:00:00.0 +00:00:00")).toBe("2026-09-10");
    expect(publishedDay("not a date")).toBe("not a date");
  });
});
