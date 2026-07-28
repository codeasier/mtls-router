import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

const css = readFileSync(resolve(process.cwd(), "src/styles.css"), "utf8");

function compact(value: string) {
  return value.replace(/\s+/g, " ");
}

function extractBlock(value: string, header: RegExp, description: string) {
  const match = header.exec(value);
  if (!match) throw new Error(`Missing ${description} block`);

  const openingBrace = match.index + match[0].lastIndexOf("{");
  let depth = 1;
  for (let index = openingBrace + 1; index < value.length; index += 1) {
    if (value[index] === "{") depth += 1;
    if (value[index] === "}") depth -= 1;
    if (depth === 0) return value.slice(openingBrace + 1, index);
  }

  throw new Error(`Unclosed ${description} block`);
}

function extractContainerBlock(
  value: string,
  threshold: number,
  name?: string,
) {
  const escapedName = name?.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return extractBlock(
    value,
    new RegExp(
      `@container\\s+${escapedName ? `${escapedName}\\s+` : ""}\\(\\s*max-width\\s*:\\s*${threshold}px\\s*\\)\\s*\\{`,
    ),
    `@container max-width: ${threshold}px`,
  );
}

function extractUnsupportedContainerFallback(value: string) {
  return extractBlock(
    value,
    /@supports\s+not\s*\(\s*container-type\s*:\s*inline-size\s*\)\s*\{/,
    "unsupported container fallback",
  );
}

function extractMediaBlock(value: string, threshold: number) {
  return extractBlock(
    value,
    new RegExp(
      `@media\\s*\\(\\s*max-width\\s*:\\s*${threshold}px\\s*\\)\\s*\\{`,
    ),
    `@media max-width: ${threshold}px`,
  );
}

function findRuleDeclarations(value: string, selector: string) {
  const normalizeSelector = (item: string) =>
    compact(item)
      .trim()
      .replace(/\s*([>+~])\s*/g, "$1");
  const normalizedSelector = normalizeSelector(selector);
  const rules = value.matchAll(/([^{}]+)\{([^{}]*)\}/g);

  for (const rule of rules) {
    const selectors = rule[1].split(",").map(normalizeSelector);
    if (selectors.includes(normalizedSelector)) return rule[2];
  }

  throw new Error(`Missing CSS rule for selector: ${selector}`);
}

describe("application scroll boundary", () => {
  it("keeps the header outside the vertical scroll container", () => {
    const main = compact(findRuleDeclarations(css, "main"));
    const content = compact(findRuleDeclarations(css, ".main-scroll"));

    expect(main).toMatch(/display:\s*grid;/);
    expect(main).toMatch(/grid-template-rows:\s*auto\s+minmax\(0,\s*1fr\);/);
    expect(main).toMatch(/overflow:\s*hidden;/);
    expect(main).not.toMatch(/overflow-y:\s*auto;/);
    expect(content).toMatch(/overflow-x:\s*hidden;/);
    expect(content).toMatch(/overflow-y:\s*auto;/);
    expect(content).toMatch(/overscroll-behavior:\s*contain;/);
  });
});

describe("router viewport layout", () => {
  it("keeps one full-height content column with the next action at the bottom", () => {
    const panel = compact(
      extractBlock(css, /\.panel-grid\s*\{/, "router panel layout"),
    );
    const content = compact(findRuleDeclarations(css, ".primary-panel"));
    const next = compact(findRuleDeclarations(css, ".router-next"));

    expect(panel).toMatch(/grid-template-columns:\s*minmax\(0,\s*1fr\);/);
    expect(panel).toMatch(/min-height:\s*100%;/);
    expect(content).toMatch(/display:\s*flex;/);
    expect(content).toMatch(/flex-direction:\s*column;/);
    expect(next).toMatch(/margin-top:\s*auto;/);
  });
});

describe("responsive Claude role layout", () => {
  it("uses the model panel as an inline-size query container", () => {
    expect(compact(css)).toMatch(
      /\.model-agent-panel \{[^}]*container-type: inline-size;/,
    );
  });

  it("stacks role controls at medium card widths", () => {
    const medium = compact(extractContainerBlock(css, 760));
    expect(medium).toMatch(/\.role-row\s*\{[^}]*align-items:\s*stretch;/);
    expect(medium).toMatch(/\.role-row\s*\{[^}]*flex-direction:\s*column;/);
    expect(medium).toMatch(
      /\.role-row\s*>\s*label\s*\{[^}]*flex-basis:\s*auto;/,
    );
  });

  it("stacks selection fields only at narrow card widths", () => {
    const narrow = compact(extractContainerBlock(css, 560));
    expect(narrow).toMatch(
      /\.claude-selection-fields\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\);/,
    );
  });

  it("allows selection children to shrink inside the card", () => {
    for (const selector of [
      ".claude-selection-fields > *",
      ".optional-role-editor",
      ".context-selector",
    ]) {
      expect(compact(findRuleDeclarations(css, selector))).toMatch(
        /(?:^|;)\s*min-width:\s*0\s*;/,
      );
    }
  });

  it("falls back to medium role stacking without container queries", () => {
    const fallback = extractUnsupportedContainerFallback(css);
    const medium = compact(extractMediaBlock(fallback, 1400));
    expect(medium).toMatch(/\.role-row\s*\{[^}]*align-items:\s*stretch;/);
    expect(medium).toMatch(/\.role-row\s*\{[^}]*flex-direction:\s*column;/);
    expect(medium).toMatch(
      /\.role-row\s*>\s*label\s*\{[^}]*flex-basis:\s*auto;/,
    );
    expect(medium).toMatch(
      /\.claude-selection-fields\s*,\s*\.optional-role-editor\s*\{[^}]*width:\s*100%;/,
    );
  });

  it("falls back to one-column fields at narrow legacy viewport widths", () => {
    const fallback = extractUnsupportedContainerFallback(css);
    const narrow = compact(extractMediaBlock(fallback, 1200));
    expect(narrow).toMatch(
      /\.claude-selection-fields\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\);/,
    );
  });
});

describe("model configuration typography", () => {
  it("keeps configuration labels and supporting text readable", () => {
    for (const selector of [
      ".config-workbench label",
      ".config-workbench legend",
      ".config-workbench small",
      ".config-workbench .option-field > span",
      ".config-workbench .catalog-model code",
      ".config-workbench .validation-path",
    ]) {
      const declarations = compact(findRuleDeclarations(css, selector));
      expect(declarations).toMatch(/font-size:\s*12px;/);
      expect(declarations).toMatch(/line-height:\s*1\.5;/);
    }
  });

  it("uses readable text and height for configuration controls", () => {
    for (const selector of [
      ".config-workbench .control-button",
      ".config-workbench .text-button",
      '.config-workbench input:not([type="checkbox"]):not([type="radio"]):not([type="file"])',
      ".config-workbench select",
      ".config-workbench textarea",
    ]) {
      const declarations = compact(findRuleDeclarations(css, selector));
      expect(declarations).toMatch(/font-size:\s*14px;/);
      expect(declarations).toMatch(/line-height:\s*1\.4;/);
    }
  });

  it("gives single-line configuration controls enough height", () => {
    for (const selector of [
      ".config-workbench .control-button",
      ".config-workbench .text-button",
      '.config-workbench input:not([type="checkbox"]):not([type="radio"]):not([type="file"])',
      ".config-workbench select",
    ]) {
      expect(compact(findRuleDeclarations(css, selector))).toMatch(
        /min-height:\s*42px;/,
      );
    }
  });
});

describe("responsive rebuild flow", () => {
  it("stacks preview and result content before the narrow layout", () => {
    const medium = compact(extractMediaBlock(css, 800));
    expect(medium).toMatch(
      /\.preview-layout\s*,\s*\.result-grid\s*\{[^}]*grid-template-columns:\s*1fr;/,
    );
    expect(medium).toMatch(/\.approval-rail\s*\{[^}]*width:\s*100%;/);
  });

  it("keeps destructive confirmation usable at phone widths", () => {
    const narrow = compact(extractMediaBlock(css, 540));
    expect(narrow).toMatch(
      /\.danger-dialog__actions\s*\{[^}]*align-items:\s*stretch;[^}]*flex-direction:\s*column;/,
    );
    expect(narrow).toMatch(/\.dialog-backdrop\s*\{[^}]*padding:\s*10px;/);
    expect(narrow).toMatch(
      /\.danger-dialog\s*\{[^}]*max-height:\s*calc\(100dvh - 20px\);/,
    );
  });

  it("allows rebuild paths and warnings to wrap without widening cards", () => {
    expect(compact(findRuleDeclarations(css, ".effect-card"))).toMatch(
      /(?:^|;)\s*min-width:\s*0\s*;/,
    );
    expect(compact(findRuleDeclarations(css, ".effect-card code"))).toMatch(
      /(?:^|;)\s*overflow-wrap:\s*anywhere\s*;/,
    );
    expect(compact(findRuleDeclarations(css, ".result-path code"))).toMatch(
      /(?:^|;)\s*overflow-wrap:\s*anywhere\s*;/,
    );
  });
});
