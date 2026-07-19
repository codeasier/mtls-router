import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { ModelConfig } from "./ipc";

interface Vector {
  name: string;
  agents: string[];
  catalog: string[];
  input: string;
  canonical: string;
}

function canonical(value: unknown): string {
  if (value === null || typeof value === "boolean")
    return JSON.stringify(value);
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new Error("non-finite number");
    return Object.is(value, -0) ? "0" : JSON.stringify(value);
  }
  if (typeof value === "string") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>).sort(
      ([left], [right]) => {
        const a = Array.from({ length: left.length }, (_, index) =>
          left.charCodeAt(index),
        );
        const b = Array.from({ length: right.length }, (_, index) =>
          right.charCodeAt(index),
        );
        for (let index = 0; index < Math.min(a.length, b.length); index += 1) {
          if (a[index] !== b[index]) return a[index] - b[index];
        }
        return a.length - b.length;
      },
    );
    return `{${entries.map(([key, item]) => `${JSON.stringify(key)}:${canonical(item)}`).join(",")}}`;
  }
  throw new Error("unsupported JSON value");
}

const root = resolve(import.meta.dirname, "../..");
const vectors = JSON.parse(
  readFileSync(
    resolve(
      root,
      "internal/manager/agent/modelconfig/testdata/jcs-vectors.json",
    ),
    "utf8",
  ),
) as Vector[];

describe("canonical model config interchange", () => {
  it.each(vectors)("matches shared JCS vector: $name", (vector) => {
    const config = JSON.parse(vector.input) as ModelConfig;
    expect(config.version).toBe(1);
    expect(canonical(config)).toBe(vector.canonical);
  });

  it("keeps generated schema aligned with the TypeScript interchange surface", () => {
    const schema = JSON.parse(
      readFileSync(
        resolve(
          root,
          "internal/manager/agent/modelconfig/schema/model-config-v1.schema.json",
        ),
        "utf8",
      ),
    ) as {
      properties: Record<string, { properties?: Record<string, unknown> }>;
      $defs: Record<string, { properties?: Record<string, unknown> }>;
    };
    expect(Object.keys(schema.properties).sort()).toEqual([
      "claude",
      "codex",
      "opencode",
      "version",
    ]);
    expect(
      Object.keys(schema.$defs.openCodeModel.properties ?? {}).sort(),
    ).toEqual([
      "attachment",
      "extra",
      "interleaved",
      "limit",
      "modalities",
      "name",
      "options",
      "reasoning",
      "temperature",
      "tool_call",
      "variants",
    ]);
    expect(
      Object.keys(schema.properties.claude.properties ?? {}).sort(),
    ).toEqual([
      "context_window",
      "extra",
      "haiku",
      "max_output_tokens",
      "opus",
      "primary",
      "sonnet",
    ]);
    expect(
      Object.keys(schema.properties.codex.properties ?? {}).sort(),
    ).toEqual([
      "auto_compact_token_limit",
      "context_window",
      "extra",
      "model",
      "reasoning_effort",
      "reasoning_summary",
      "verbosity",
    ]);
  });
});
