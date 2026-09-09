import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  decideJobs,
  extractImagePayload,
  groupedImageChoices,
  parseChatModels,
  parseImageModels,
  rebuildChatMessages,
  selectChatModel,
  sseContent,
  titleFrom,
} from "./workbenchOrchestrate";

const testdata = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../internal/proxy/testdata",
);

describe("workbenchOrchestrate", () => {
  it("parses catalogs without inventing fallbacks", () => {
    const chat = parseChatModels(
      JSON.parse(readFileSync(path.join(testdata, "models.json"), "utf8")),
    );
    const image = parseImageModels(
      JSON.parse(
        readFileSync(path.join(testdata, "models_image.json"), "utf8"),
      ),
    );
    expect(chat.every((id) => !id.includes("/"))).toBe(true);
    expect(chat).toContain("gemini-3.8-flash");
    expect(image).toContain("ag/gemini-3.1-flash-image");
    expect(image).toContain("cx/gpt-5.5-image");
    expect(image).toContain("cx/gpt-5.6-image");
    expect(selectChatModel(null, ["other-flash"]).ok).toBe(false);
    const choices = groupedImageChoices(image, "unknown/custom-model");
    expect(
      choices.some((item) => item.display_name === "ag/gemini-3.1-flash-image"),
    ).toBe(true);
    expect(
      choices.find((item) => item.id === "unknown/custom-model")?.allowed,
    ).toBe(false);
  });

  it("strips image fences and prompt echo from SSE fixtures", () => {
    const raw = [
      sseContent(
        'data: {"choices":[{"delta":{"content":"好的，这就出图。\\n"}}]}',
      ),
      sseContent(
        'data: {"choices":[{"delta":{"content":"```image\\n{\\"action\\":\\"generate\\",\\"prompt\\":\\"a red cube on a wooden table, soft daylight\\",\\"size\\":\\"1024x1024\\"}\\n```"}}]}',
      ),
    ].join("");
    const extracted = extractImagePayload(raw, "1024x1024");
    expect(extracted.specs[0]?.action).toBe("generate");
    expect(extracted.display).toContain("好的");
    expect(extracted.display).not.toContain("```");
    expect(extracted.display).not.toContain("a red cube");
  });

  it("matches the edit decision matrix", () => {
    const explicit = decideJobs(
      [{ action: "generate", prompt: "a lamp", size: "1024x1024" }],
      "改成蓝色",
      false,
      "asset-a",
      "asset-b",
      "1024x1024",
      false,
    );
    expect(explicit[0]).toMatchObject({
      action: "edit",
      reference_asset_id: "asset-a",
    });
    const implicit = decideJobs(
      [],
      "改成蓝色的天空",
      false,
      null,
      "asset-b",
      "1024x1024",
      false,
    );
    expect(implicit[0]).toMatchObject({
      action: "edit",
      reference_asset_id: "asset-b",
    });
    const force = decideJobs(
      [],
      "随便画一个",
      true,
      null,
      "asset-b",
      "1024x1024",
      false,
    );
    expect(force[0]).toMatchObject({
      action: "generate",
      reference_asset_id: null,
    });
    expect(() =>
      decideJobs(
        [{ action: "edit", prompt: "make it gold", size: "1024x1024" }],
        "hi",
        false,
        null,
        null,
        "1024x1024",
        true,
      ),
    ).toThrow(/WORKBENCH_NO_REFERENCE/);
  });

  it("rebuilds chat messages with last-image and edit requirements", () => {
    const messages = rebuildChatMessages({
      history: [
        {
          role: "user",
          visible_text: "画一只猫",
          generated_prompts: [],
          reference_hint: "旧图",
        },
      ],
      lastImagePrompt: "a sitting cat",
      forceImage: true,
      explicitReference: true,
      size: "1024x1024",
    });
    const encoded = JSON.stringify(messages);
    expect(encoded).toContain("上一张已生成的画面描述：a sitting cat");
    expect(encoded).toContain("改图必须 action=edit");
    expect(encoded).not.toContain("sk-");
  });

  it("truncates titles locally", () => {
    expect(titleFrom("一二三四五六七八九十abcdefghij")).toBe(
      "一二三四五六七八九十abcdefgh",
    );
  });
});
