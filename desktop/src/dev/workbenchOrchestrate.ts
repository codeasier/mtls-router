export const SYSTEM_PROMPT =
  '你是对话生图助手。既能闲聊，也能出图。\n\n规则：\n1. 用一两句简洁中文回复。可见文字里不要写英文画面描述，不要复述 prompt，不要贴 JSON。\n2. 需要出图或改图时，只在回复最末尾输出：\n```image\n{"action":"generate","prompt":"完整画面描述","size":"1024x1024"}\n```\naction 只能是 generate 或 edit。改图用 edit，并写出完整新 prompt。\n3. prompt 只放在上面的 JSON 里，要具体：主体、构图、光线、材质、色彩、风格。\n4. 只聊天、不需要图时，禁止输出 image 块。\n5. 用户附带了参考图时，必须用 action=edit，并基于参考图写完整新 prompt。\n6. 不要使用 markdown 图片或外链。不要解释这个协议。';

export const EMPTY_REF_ZH = "根据这张参考图继续改。";
export const EMPTY_REF_EN = "Continue editing from this reference image.";
export const DEFAULT_CHAT_MODEL = "gemini-3.8-flash";
export const DEFAULT_IMAGE_MODEL = "ag/gemini-3.1-flash-image";
export const GPT_IMAGE_2 = "cx/gpt-5.5-image";
export const MAX_TEXT_BYTES = 20 * 1024;
export const MAX_CHAT_JSON_BYTES = 256 * 1024;
export const TITLE_GRAPHEMES = 18;

const IMAGE_HINT =
  /画|绘|生成|出一张|做一张|来一张|重画|改成|改一下|换成|imagine|draw|generate|render|paint|illustration/i;
const EDIT_HINT_ZH = /改成|改一下|换成|重画成|变成/;
const EDIT_HINT_EN = /change it to|make it|turn it into|replace with|redo as/i;

export interface ImageSpec {
  action: string;
  prompt: string;
  size: string;
}

export interface JobDecision {
  action: string;
  prompt: string;
  size: string;
  reference_asset_id: string | null;
  from_model: boolean;
}

export interface HistoryTurn {
  role: string;
  visible_text: string;
  generated_prompts: string[];
  reference_hint: string | null;
}

export interface ImageChoice {
  id: string;
  display_name: string;
  verified_edit: boolean;
  allowed: boolean;
}

export function titleFrom(text: string): string {
  if (typeof Intl !== "undefined" && "Segmenter" in Intl) {
    const segments = new Intl.Segmenter(undefined, { granularity: "grapheme" });
    return Array.from(segments.segment(text), (item) => item.segment)
      .slice(0, TITLE_GRAPHEMES)
      .join("");
  }
  return Array.from(text).slice(0, TITLE_GRAPHEMES).join("");
}

export function looksLikeImageRequest(text: string): boolean {
  return IMAGE_HINT.test(text);
}

export function looksLikeImplicitEdit(text: string, english: boolean): boolean {
  if (EDIT_HINT_ZH.test(text)) return true;
  return english ? EDIT_HINT_EN.test(text) : false;
}

export function extractImagePayload(
  raw: string,
  defaultSize: string,
): { display: string; specs: ImageSpec[] } {
  const specs: ImageSpec[] = [];
  let display = replaceFences(raw, defaultSize, specs);
  display = stripOpenFence(display, defaultSize, specs);
  let cursor = 0;
  let cleaned = "";
  let stripped = false;
  for (const object of findJsonObjects(display)) {
    cleaned += display.slice(cursor, object.start);
    cursor = object.end;
    const spec = coerceSpec(object.text, defaultSize);
    if (spec && spec.prompt) {
      specs.push(spec);
      stripped = true;
      continue;
    }
    cleaned += object.text;
  }
  cleaned += display.slice(cursor);
  if (stripped) display = cleaned;
  display = stripPromptEcho(display, specs);
  return { display, specs };
}

export function rebuildChatMessages(input: {
  history: HistoryTurn[];
  lastImagePrompt?: string | null;
  forceImage: boolean;
  explicitReference: boolean;
  size: string;
}): Array<{ role: string; content: string }> {
  const messages: Array<{ role: string; content: string }> = [
    { role: "system", content: SYSTEM_PROMPT },
  ];
  if (input.lastImagePrompt) {
    messages.push({
      role: "system",
      content: `上一张已生成的画面描述：${input.lastImagePrompt}。用户若改图，action 用 edit，并写出完整新 prompt。`,
    });
  }
  for (const turn of input.history) {
    if (
      !turn.visible_text &&
      turn.generated_prompts.length === 0 &&
      !turn.reference_hint
    ) {
      continue;
    }
    let content = turn.visible_text;
    if (turn.generated_prompts.length > 0) {
      content += `\n\n[已生成图片] ${turn.generated_prompts.join(" / ")}`;
    }
    if (turn.reference_hint !== null) {
      content += turn.reference_hint
        ? `\n\n[用户附带了参考图：${turn.reference_hint}。改图必须 action=edit]`
        : "\n\n[用户附带了参考图。改图必须 action=edit]";
    }
    messages.push({ role: turn.role, content });
  }
  if (input.forceImage && !input.explicitReference) {
    messages.push({
      role: "system",
      content:
        "用户要求直接出图。必须输出 image 块，action=generate。先用一句话确认。",
    });
  }
  if (input.explicitReference) {
    messages.push({
      role: "system",
      content:
        "用户附带了参考图。必须输出 image 块，action=edit。先用一句话确认。",
    });
  }
  return fitMessages(messages);
}

export function decideJobs(
  specs: ImageSpec[],
  userText: string,
  forceImage: boolean,
  explicitRef: string | null,
  lastImage: string | null,
  defaultSize: string,
  english: boolean,
): JobDecision[] {
  const jobs: JobDecision[] = specs
    .filter((spec) => spec.prompt)
    .map((spec) => ({
      action: spec.action,
      prompt: spec.prompt,
      size: spec.size || defaultSize,
      reference_asset_id: null,
      from_model: true,
    }));
  if (
    jobs.length === 0 &&
    (forceImage || explicitRef || looksLikeImageRequest(userText))
  ) {
    const implicitEdit =
      !forceImage &&
      !explicitRef &&
      Boolean(lastImage) &&
      looksLikeImplicitEdit(userText, english);
    jobs.push({
      action: implicitEdit ? "edit" : "generate",
      prompt: userText,
      size: defaultSize,
      reference_asset_id: null,
      from_model: false,
    });
  }
  if (explicitRef) {
    for (const job of jobs) {
      job.action = "edit";
      job.reference_asset_id = explicitRef;
    }
  } else {
    for (const job of jobs) {
      if (job.action !== "edit") continue;
      if (!lastImage) {
        const error = new Error("WORKBENCH_NO_REFERENCE") as Error & {
          code: string;
        };
        error.code = "WORKBENCH_NO_REFERENCE";
        throw error;
      }
      job.reference_asset_id = lastImage;
    }
  }
  for (const job of jobs) {
    if (new TextEncoder().encode(job.prompt).length > MAX_TEXT_BYTES) {
      const error = new Error("WORKBENCH_INPUT_TOO_LARGE") as Error & {
        code: string;
      };
      error.code = "WORKBENCH_INPUT_TOO_LARGE";
      throw error;
    }
  }
  return jobs;
}

export function parseChatModels(body: unknown): string[] {
  return parseIds(body).filter((id) => !id.includes("/"));
}

export function parseImageModels(body: unknown): string[] {
  return parseIds(body);
}

export function selectChatModel(
  current: string | null | undefined,
  catalog: string[],
): { id: string | null; ok: boolean } {
  if (current) return { id: current, ok: catalog.includes(current) };
  if (catalog.includes(DEFAULT_CHAT_MODEL)) {
    return { id: DEFAULT_CHAT_MODEL, ok: true };
  }
  return { id: null, ok: false };
}

export function selectImageModel(
  current: string | null | undefined,
  catalog: string[],
): { id: string | null; ok: boolean } {
  if (current) return { id: current, ok: catalog.includes(current) };
  if (catalog.includes(DEFAULT_IMAGE_MODEL)) {
    return { id: DEFAULT_IMAGE_MODEL, ok: true };
  }
  if (catalog.length > 0) {
    return { id: catalog[0], ok: true };
  }
  return { id: null, ok: false };
}

export function groupedImageChoices(
  catalog: string[],
  current?: string | null,
): ImageChoice[] {
  const choices: ImageChoice[] = catalog.map((id) => ({
    id,
    display_name: id,
    verified_edit: id === GPT_IMAGE_2,
    allowed: true,
  }));
  if (current && !choices.some((choice) => choice.id === current)) {
    choices.unshift({
      id: current,
      display_name: current,
      verified_edit: current === GPT_IMAGE_2,
      allowed: catalog.includes(current),
    });
  }
  return choices;
}

export function sseContent(line: string): string | null {
  if (!line.startsWith("data:")) return null;
  const payload = line.slice(5).trim();
  if (!payload || payload === "[DONE]") return null;
  try {
    const value = JSON.parse(payload) as {
      choices?: Array<{ delta?: { content?: string } }>;
    };
    return value.choices?.[0]?.delta?.content ?? null;
  } catch {
    return null;
  }
}

function parseIds(body: unknown): string[] {
  if (!body || typeof body !== "object" || !("data" in body)) return [];
  const data = (body as { data: unknown }).data;
  if (!Array.isArray(data)) return [];
  const ids: string[] = [];
  for (const item of data) {
    if (
      !item ||
      typeof item !== "object" ||
      typeof (item as { id?: unknown }).id !== "string"
    ) {
      continue;
    }
    const id = (item as { id: string }).id.trim();
    if (!id || id.length > 256 || ids.includes(id)) continue;
    ids.push(id);
  }
  return ids;
}

function fitMessages(
  messages: Array<{ role: string; content: string }>,
): Array<{ role: string; content: string }> {
  const copy = [...messages];
  while (JSON.stringify(copy).length > MAX_CHAT_JSON_BYTES) {
    const index = copy.findIndex((item) => item.role !== "system");
    if (index < 0) break;
    copy.splice(index, 1);
  }
  return copy;
}

function replaceFences(
  display: string,
  defaultSize: string,
  specs: ImageSpec[],
): string {
  let out = "";
  const lower = display.toLowerCase();
  let cursor = 0;
  while (true) {
    const rel = lower.indexOf("```", cursor);
    if (rel < 0) break;
    out += display.slice(cursor, rel);
    const after = display.slice(rel + 3);
    const tagEnd =
      after.indexOf("\n") === -1 ? after.length : after.indexOf("\n");
    const tag = after.slice(0, tagEnd).trim().toLowerCase();
    if (tag && tag !== "image" && tag !== "json" && tag !== "jsonc") {
      out += display.slice(rel, rel + 3);
      cursor = rel + 3;
      continue;
    }
    const rest = after.slice(tagEnd);
    const closeRel = rest.indexOf("```");
    if (closeRel < 0) break;
    const spec = coerceSpec(rest.slice(0, closeRel).trim(), defaultSize);
    if (spec?.prompt) specs.push(spec);
    cursor = rel + 3 + tagEnd + closeRel + 3;
    out += "\n";
  }
  if (cursor < display.length) out += display.slice(cursor);
  return out || display;
}

function stripOpenFence(
  display: string,
  defaultSize: string,
  specs: ImageSpec[],
): string {
  const start = display.toLowerCase().lastIndexOf("```");
  if (start < 0) return display;
  const after = display
    .slice(start + 3)
    .replace(/^[A-Za-z]+/, "")
    .replace(/^[\r\n]+/, "");
  const spec = coerceSpec(after, defaultSize);
  if (spec?.prompt) specs.push(spec);
  return display.slice(0, start);
}

function coerceSpec(text: string, defaultSize: string): ImageSpec | null {
  const parsed = parseLooseJson(text);
  if (!parsed || typeof parsed !== "object") return null;
  const record = parsed as {
    prompt?: unknown;
    action?: unknown;
    size?: unknown;
  };
  if (record.prompt == null && record.action == null) return null;
  return {
    action: record.action === "edit" ? "edit" : "generate",
    prompt: typeof record.prompt === "string" ? record.prompt.trim() : "",
    size: typeof record.size === "string" ? record.size : defaultSize,
  };
}

function parseLooseJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    try {
      return JSON.parse(text.replace(/,(\s*[}\]])/g, "$1"));
    } catch {
      return null;
    }
  }
}

function findJsonObjects(
  text: string,
): Array<{ start: number; end: number; text: string }> {
  const out: Array<{ start: number; end: number; text: string }> = [];
  for (let i = 0; i < text.length; i += 1) {
    if (text[i] !== "{") continue;
    let depth = 0;
    let inStr = false;
    let esc = false;
    for (let j = i; j < text.length; j += 1) {
      const char = text[j];
      if (inStr) {
        if (esc) esc = false;
        else if (char === "\\") esc = true;
        else if (char === '"') inStr = false;
      } else if (char === '"') inStr = true;
      else if (char === "{") depth += 1;
      else if (char === "}") {
        depth -= 1;
        if (depth === 0) {
          out.push({ start: i, end: j + 1, text: text.slice(i, j + 1) });
          i = j;
          break;
        }
      }
    }
  }
  return out;
}

function stripPromptEcho(display: string, specs: ImageSpec[]): string {
  let text = display;
  for (const spec of specs) {
    if (spec.prompt.length < 12) continue;
    text = text.replaceAll(spec.prompt, "");
    const head = spec.prompt.slice(0, 36);
    text = text
      .split("\n")
      .filter((line) => {
        const trimmed = line.trim();
        if (!trimmed) return true;
        if (trimmed.includes(head) || head.includes(trimmed.slice(0, 36))) {
          return false;
        }
        const letters = (trimmed.match(/[A-Za-z]/g) ?? []).length;
        return !(trimmed.length > 36 && (letters * 100) / trimmed.length > 55);
      })
      .join("\n");
  }
  return text.replaceAll("\n\n\n", "\n\n").trim();
}
