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
      ".agent-panel label",
      ".agent-panel legend",
      ".agent-panel small",
      ".agent-panel .option-field > span",
      ".agent-panel .catalog-model code",
      ".agent-panel .validation-path",
      ".agent-panel .role-row > label",
      ".agent-panel .context-selector legend",
      ".agent-panel .context-selector label",
    ]) {
      const declarations = compact(findRuleDeclarations(css, selector));
      expect(declarations).toMatch(/font-size:\s*13px;/);
      expect(declarations).toMatch(/line-height:\s*1\.5;/);
    }
  });

  it("uses readable text and height for configuration controls", () => {
    for (const selector of [
      ".agent-panel .control-button",
      ".agent-panel .text-button",
      '.agent-panel input:not([type="checkbox"]):not([type="radio"]):not([type="file"])',
      ".agent-panel select",
      ".agent-panel textarea",
      ".agent-panel .advanced-editor textarea",
    ]) {
      const declarations = compact(findRuleDeclarations(css, selector));
      expect(declarations).toMatch(/font-size:\s*15px;/);
      expect(declarations).toMatch(/line-height:\s*1\.4;/);
    }
  });

  it("gives single-line configuration controls enough height", () => {
    for (const selector of [
      ".agent-panel .control-button",
      ".agent-panel .text-button",
      '.agent-panel input:not([type="checkbox"]):not([type="radio"]):not([type="file"])',
      ".agent-panel select",
    ]) {
      expect(compact(findRuleDeclarations(css, selector))).toMatch(
        /min-height:\s*42px;/,
      );
    }
  });

  it("centers discovery and execution status across the full workspace", () => {
    const workspace = compact(
      findRuleDeclarations(css, ".agent-panel__workspace--status"),
    );
    const processing = compact(
      findRuleDeclarations(css, ".agent-panel__processing"),
    );

    expect(workspace).toMatch(/grid-template-columns:\s*minmax\(0,\s*1fr\);/);
    expect(workspace).toMatch(/align-items:\s*center;/);
    expect(workspace).toMatch(/justify-items:\s*center;/);
    expect(processing).toMatch(/grid-column:\s*1\s*\/\s*-1;/);
    expect(processing).toMatch(/justify-self:\s*center;/);
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

describe("responsive Agent workspace", () => {
  it("keeps a sticky dual-column rail above 900px", () => {
    const workspace = compact(
      extractBlock(
        css,
        /\.agent-panel__workspace\s*\{/,
        "agent panel workspace",
      ),
    );
    const rail = compact(
      extractBlock(css, /\.agent-panel__rail\s*\{/, "agent panel rail"),
    );
    const preview = compact(
      findRuleDeclarations(css, ".agent-panel__rail .preview-layout"),
    );
    const previewMain = compact(
      findRuleDeclarations(css, ".agent-panel__rail .preview-main"),
    );
    const approval = compact(
      findRuleDeclarations(css, ".agent-panel__rail .approval-rail"),
    );

    expect(workspace).toMatch(
      /grid-template-columns:\s*minmax\(0,\s*1fr\)\s+minmax\(310px,\s*0\.38fr\);/,
    );
    expect(rail).toMatch(/position:\s*sticky;/);
    expect(rail).toMatch(/max-height:\s*calc\(100dvh - 190px\);/);
    expect(rail).toMatch(/overflow:\s*auto;/);
    expect(preview).toMatch(/display:\s*flex;/);
    expect(previewMain).toMatch(/overflow:\s*auto;/);
    expect(approval).toMatch(/position:\s*sticky;/);
    expect(approval).toMatch(/bottom:\s*0;/);
  });

  it("stacks the workspace and keeps submit pinned below 900px", () => {
    const medium = compact(extractMediaBlock(css, 900));
    expect(medium).toMatch(
      /\.agent-panel__workspace[\s\S]*grid-template-columns:\s*minmax\(0,\s*1fr\);/,
    );
    expect(medium).toMatch(/\.agent-panel__editor\s*\{[^}]*min-height:\s*0;/);
    expect(medium).toMatch(/\.agent-panel__rail\s*\{[^}]*position:\s*static;/);
    expect(medium).toMatch(/\.agent-panel__rail\s*\{[^}]*overflow:\s*visible;/);
    expect(medium).toMatch(
      /\.agent-panel__rail \.preview-main\s*\{[^}]*overflow:\s*visible;/,
    );
    expect(medium).toMatch(
      /\.cleanup-approval-rail\s*\{[^}]*position:\s*sticky;[^}]*bottom:\s*12px;/,
    );
  });
});

describe("logs scroll boundary", () => {
  it("keeps long log lines inside the log screen", () => {
    const panel = compact(findRuleDeclarations(css, ".logs-panel"));
    const screen = compact(findRuleDeclarations(css, ".log-screen"));
    const scroll = compact(findRuleDeclarations(css, ".log-screen--scroll"));
    const line = compact(findRuleDeclarations(css, ".log-screen code"));

    expect(panel).toMatch(/overflow:\s*hidden;/);
    expect(screen).toMatch(/overflow:\s*auto;/);
    expect(scroll).toMatch(/overflow:\s*auto;/);
    expect(scroll).toMatch(/overscroll-behavior:\s*contain;/);
    expect(line).toMatch(/overflow-wrap:\s*anywhere;/);
  });
});

describe("settings path readability", () => {
  it("lets location paths wrap instead of widening the page", () => {
    expect(
      compact(findRuleDeclarations(css, ".settings-block--locations dd")),
    ).toMatch(/overflow-wrap:\s*anywhere;/);
  });
});

describe("appearance themes", () => {
  it("defines warm, light, and dark token palettes", () => {
    expect(css).toMatch(/\[data-theme="warm"\]/);
    expect(css).toMatch(/\[data-theme="light"\]/);
    expect(css).toMatch(/\[data-theme="dark"\]/);
    expect(compact(findRuleDeclarations(css, ".nav-marker"))).not.toMatch(
      /font-family/,
    );
    expect(compact(findRuleDeclarations(css, ".theme-picker"))).toMatch(
      /role|flex/,
    );
  });

  it("keeps chrome, logos, and catalog wells on theme tokens", () => {
    expect(compact(findRuleDeclarations(css, ".topbar"))).toMatch(
      /color:\s*var\(--ink\);/,
    );
    expect(compact(findRuleDeclarations(css, ".topbar"))).toMatch(
      /background:\s*var\(--topbar\);/,
    );
    expect(compact(findRuleDeclarations(css, ".dialog-backdrop"))).toMatch(
      /background:\s*var\(--scrim\);/,
    );
    expect(compact(findRuleDeclarations(css, ".danger-dialog"))).toMatch(
      /box-shadow:\s*var\(--scrim-shadow\);/,
    );
    expect(compact(findRuleDeclarations(css, ".agent-logo--claude"))).toMatch(
      /color:\s*#d97757;/,
    );
    expect(compact(findRuleDeclarations(css, ".agent-logo--opencode"))).toMatch(
      /color:\s*#211e1e;/,
    );
    expect(compact(findRuleDeclarations(css, ".agent-logo--codex"))).toMatch(
      /color:\s*#302a25;/,
    );
    expect(css).not.toMatch(/\[data-theme="light"\]\s+\.agent-logo--claude/);
    expect(compact(findRuleDeclarations(css, ".catalog-rail"))).toMatch(
      /background:\s*var\(--catalog\);/,
    );
    expect(compact(findRuleDeclarations(css, ".catalog-rail"))).toMatch(
      /color:\s*var\(--catalog-ink\);/,
    );
    expect(compact(findRuleDeclarations(css, ".failure-guidance"))).not.toMatch(
      /255,\s*250,\s*244/,
    );

    for (const theme of ["warm", "light", "dark"]) {
      const block = compact(
        extractBlock(
          css,
          new RegExp(`\\[data-theme="${theme}"\\]\\s*\\{`),
          `${theme} theme tokens`,
        ),
      );
      expect(block).toMatch(/--topbar:/);
      expect(block).toMatch(/--scrim:/);
      expect(block).toMatch(/--catalog:/);
      expect(block).toMatch(/--log-ink:/);
    }
  });

  it("keeps the light theme to cool paper, ink, and one blue", () => {
    const light = compact(
      extractBlock(css, /\[data-theme="light"\]\s*\{/, "light theme tokens"),
    );
    expect(light).toMatch(/--paper:\s*#f5f5f5;/);
    expect(light).toMatch(/--surface:\s*#ffffff;/);
    expect(light).toMatch(/--surface-raised:\s*#ffffff;/);
    expect(light).toMatch(/--ink:\s*#3a3a3a;/);
    expect(light).toMatch(/--signal:\s*#3b82f6;/);
    expect(light).toMatch(/--good:\s*#3b82f6;/);
    expect(light).toMatch(/--danger:\s*#3a3a3a;/);
    expect(light).toMatch(/--warning:\s*#6b5e3a;/);
    expect(light).toMatch(/--muted:\s*#636366;/);
    expect(light).not.toMatch(/--warning:\s*#8e8e93;/);
    expect(light).not.toMatch(/--muted:\s*#8e8e93;/);
    expect(light).not.toMatch(
      /#c2410c|#b94722|#2c2c2a|#eeeeee|#fafafa|#fafaf8|#f3f0ea|#f6f5f2|#f7f2ea/,
    );
    expect(
      compact(findRuleDeclarations(css, ".agent-state--create")),
    ).not.toMatch(/185,\s*71,\s*34/);
    expect(compact(findRuleDeclarations(css, ".toggle span"))).not.toMatch(
      /76,\s*57,\s*43/,
    );
  });
});

describe("narrow overview and compact nav", () => {
  it("lets Agent cards shrink below 280px instead of overflowing", () => {
    expect(compact(findRuleDeclarations(css, ".agent-card-grid"))).toMatch(
      /grid-template-columns:\s*repeat\(\s*auto-fit,\s*minmax\(\s*min\(\s*280px,\s*100%\s*\),\s*1fr\s*\)\s*\);/,
    );
    expect(compact(findRuleDeclarations(css, ".agent-card-grid"))).toMatch(
      /(?:^|;)\s*min-width:\s*0\s*;/,
    );
  });

  it("keeps Router actions above the decorative instrument on narrow screens", () => {
    const narrow = compact(extractMediaBlock(css, 540));
    expect(narrow).toMatch(
      /\.primary-panel \.readout-grid\s*\{[^}]*grid-template-columns:\s*repeat\(\s*2,\s*minmax\(\s*0,\s*1fr\s*\)\s*\);/,
    );
    expect(narrow).toMatch(/\.primary-panel \.action-row\s*\{[^}]*order:\s*3;/);
    expect(narrow).toMatch(/\.primary-panel \.instrument\s*\{[^}]*order:\s*5;/);
  });

  it("keeps compact navigation icon-only for the collapse control", () => {
    const compactNav = compact(extractMediaBlock(css, 800));
    expect(compactNav).toMatch(/\.nav-label--full\s*\{[^}]*display:\s*none;/);
    expect(compactNav).toMatch(/\.nav-label--short\s*\{[^}]*display:\s*block;/);
    expect(compactNav).toMatch(/\.sidebar-collapse\s*\{[^}]*width:\s*40px;/);
    expect(compactNav).toMatch(
      /\.sidebar-collapse__label\s*\{[^}]*display:\s*none;/,
    );
  });
});

describe("responsive Agent cleanup", () => {
  it("stacks card actions below 900px", () => {
    const medium = compact(extractMediaBlock(css, 900));
    const baseRule =
      /\.agent-card__actions\s*\{[^}]*grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\);/.exec(
        css,
      );
    const responsiveRule =
      /@media\s*\(\s*max-width\s*:\s*900px\s*\)[\s\S]*?\.agent-card__actions\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\);/.exec(
        css,
      );
    expect(baseRule).not.toBeNull();
    expect(responsiveRule).not.toBeNull();
    expect(responsiveRule!.index).toBeGreaterThan(baseRule!.index);
    expect(medium).toMatch(
      /\.agent-card__actions\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\);/,
    );
  });

  it("keeps cleanup paths within their cards", () => {
    expect(
      compact(findRuleDeclarations(css, ".cleanup-removed-paths code")),
    ).toMatch(/overflow-wrap:\s*anywhere;/);
    expect(
      compact(findRuleDeclarations(css, ".cleanup-result-path code")),
    ).toMatch(/overflow-wrap:\s*anywhere;/);
  });
});
