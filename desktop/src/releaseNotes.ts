import changelogEn from "../../docs/CHANGELOG.md?raw";
import changelogZh from "../../docs/zh-CN/CHANGELOG.md?raw";

import type { Language } from "./i18n";

const EN_SEPARATOR = "\n\nEnglish\n\n";
const VERSION_HEADING = /^## v(\d+\.\d+\.\d+) - (\d{4}-\d{2}-\d{2})[ \t]*$/gm;

/** Public releases page. The desktop command opens this exact URL. */
export const GITHUB_RELEASES_URL =
  "https://github.com/codeasier/mtls-router/releases";

export interface ChangelogEntry {
  version: string;
  date: string;
  body: string;
}

export interface PendingReleaseNotes {
  version: string;
  notes?: string | null;
  published_at?: string | null;
}

export function changelogHistory(markdown: string): ChangelogEntry[] {
  const matches = [...markdown.matchAll(VERSION_HEADING)];
  return matches.flatMap((match, index) => {
    const start = (match.index ?? 0) + match[0].length;
    const end = matches[index + 1]?.index ?? markdown.length;
    const body = markdown
      .slice(start, end)
      .replace(/\n---\s*$/, "")
      .trim();
    if (!body || !match[1] || !match[2]) return [];
    return [{ version: match[1], date: match[2], body }];
  });
}

export function changelogSection(
  markdown: string,
  version: string,
): string | null {
  const normalized = version.trim().replace(/^v/, "");
  if (!/^\d+\.\d+\.\d+$/.test(normalized)) return null;
  return (
    changelogHistory(markdown).find((entry) => entry.version === normalized)
      ?.body ?? null
  );
}

/** Feed notes are bilingual. Plain text without those headings is shown whole. */
export function localizedReleaseNotes(
  notes: string,
  language: Language,
): string {
  const trimmed = notes.trim();
  const head = "简体中文\n\n";
  if (!trimmed.startsWith(head)) return trimmed;
  const splitAt = trimmed.indexOf(EN_SEPARATOR);
  if (splitAt < head.length) return trimmed;
  const chinese = trimmed.slice(head.length, splitAt).trim();
  const english = trimmed.slice(splitAt + EN_SEPARATOR.length).trim();
  if (!chinese || !english) return trimmed;
  return language === "en" ? english : chinese;
}

export function bundledReleaseNotes(
  version: string,
  language: Language,
): string | null {
  const source = language === "en" ? changelogEn : changelogZh;
  return changelogSection(source, version);
}

/** Bundled stable sections, newest first. A pending version missing from the build is prepended. */
export function releaseHistory(
  language: Language,
  pending?: PendingReleaseNotes | null,
): ChangelogEntry[] {
  const source = language === "en" ? changelogEn : changelogZh;
  const bundled = changelogHistory(source);
  const version = pending?.version.trim().replace(/^v/, "") ?? "";
  if (!version || bundled.some((entry) => entry.version === version)) {
    return bundled;
  }
  const notes = pending?.notes?.trim() ?? "";
  if (!notes) return bundled;
  const body = localizedReleaseNotes(notes, language);
  if (!body) return bundled;
  const published = pending?.published_at?.trim() ?? "";
  return [
    {
      version,
      date: published ? publishedDay(published) : "",
      body,
    },
    ...bundled,
  ];
}

export function publishedDay(value: string): string {
  const match = /^(\d{4}-\d{2}-\d{2})/.exec(value.trim());
  return match?.[1] ?? value.trim();
}
