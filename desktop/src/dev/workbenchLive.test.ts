import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it, vi } from "vitest";

import { workbenchAssetUri } from "../workbench";
import {
  createWorkbenchLiveHandlers,
  LIVE_ROUTER_PREFIX,
  setLiveWorkbenchSessionKey,
} from "./workbenchLive";

const testdata = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../internal/proxy/testdata",
);
const png = readFileSync(path.join(testdata, "generation_binary.png"));

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function liveFetch(): typeof fetch {
  return async (input, init) => {
    const url = String(input);
    if (url === "/__workbench-live") {
      return jsonResponse({
        upstream: "127.0.0.1:19099",
        has_server_key: true,
      });
    }
    if (url === `${LIVE_ROUTER_PREFIX}/health`) {
      return jsonResponse({ status: "ok" });
    }
    if (url === `${LIVE_ROUTER_PREFIX}/v1/models`) {
      return jsonResponse({
        data: [{ id: "gemini-3.8-flash" }, { id: "ag/hidden" }],
      });
    }
    if (url === `${LIVE_ROUTER_PREFIX}/v1/models/image`) {
      return jsonResponse({
        data: [{ id: "ag/gemini-3.1-flash-image" }, { id: "cx/gpt-5.5-image" }],
      });
    }
    if (url.endsWith("/v1/chat/completions")) {
      const payload = JSON.parse(String(init?.body ?? "{}")) as {
        messages?: Array<{ content?: string }>;
      };
      expect(JSON.stringify(payload)).not.toMatch(/sk-/);
      const stream = [
        'data: {"choices":[{"delta":{"content":"好的，这就出图。\\n"}}]}\n\n',
        'data: {"choices":[{"delta":{"content":"```image\\n{\\"action\\":\\"generate\\",\\"prompt\\":\\"a red cube on a wooden table, soft daylight\\",\\"size\\":\\"1024x1024\\"}\\n```"}}]}\n\n',
        "data: [DONE]\n\n",
      ].join("");
      return new Response(stream, {
        status: 200,
        headers: { "Content-Type": "text/event-stream" },
      });
    }
    if (url.includes("/v1/images/generations")) {
      expect(url).toContain("response_format=binary");
      const body = JSON.parse(String(init?.body ?? "{}")) as {
        model?: string;
        prompt?: string;
        n?: number;
        image?: string;
      };
      expect(body.model).toBe("ag/gemini-3.1-flash-image");
      expect(body.n).toBe(1);
      expect(body.image).toBeUndefined();
      return new Response(png, {
        status: 200,
        headers: { "Content-Type": "image/png" },
      });
    }
    return new Response("missing", { status: 404 });
  };
}

describe("createWorkbenchLiveHandlers", () => {
  it("treats a Vite-injected server key as a present credential", async () => {
    setLiveWorkbenchSessionKey("");
    const workbench = createWorkbenchLiveHandlers({ fetch: liveFetch() });
    const readiness = await workbench.getReadiness();
    expect(readiness.has_credential).toBe(true);
    expect(readiness.router_trusted).toBe(true);
    expect(readiness.health_ok).toBe(true);
    expect(readiness.chat_models).toEqual(["gemini-3.8-flash"]);
    expect(readiness.ready).toBe(false);
    expect(readiness.reason).toBeNull();
  });

  it("does not call a healthy router untrusted when catalogs return 401", async () => {
    setLiveWorkbenchSessionKey("");
    const workbench = createWorkbenchLiveHandlers({
      fetch: async (input) => {
        const url = String(input);
        if (url === "/__workbench-live") {
          return jsonResponse({ has_server_key: true });
        }
        if (url === `${LIVE_ROUTER_PREFIX}/health`) {
          return jsonResponse({ status: "ok" });
        }
        if (url.includes("/v1/models")) {
          return jsonResponse({ error: "API key required" }, 401);
        }
        return new Response("missing", { status: 404 });
      },
    });
    const readiness = await workbench.getReadiness();
    expect(readiness.router_trusted).toBe(true);
    expect(readiness.health_ok).toBe(true);
    expect(readiness.chat_models).toEqual([]);
    expect(readiness.image_models).toEqual([]);
    expect(readiness.reason).toBe("WORKBENCH_CATALOG");
  });

  it("reuses a fresh catalog instead of refetching on create", async () => {
    setLiveWorkbenchSessionKey("sk-test-live");
    let modelsCalls = 0;
    const fetchImpl: typeof fetch = async (input, init) => {
      const url = String(input);
      if (url === `${LIVE_ROUTER_PREFIX}/v1/models`) modelsCalls += 1;
      return liveFetch()(input, init);
    };
    const workbench = createWorkbenchLiveHandlers({ fetch: fetchImpl });
    await workbench.getReadiness();
    expect(modelsCalls).toBe(1);
    await workbench.create();
    expect(modelsCalls).toBe(1);
    await workbench.refreshCatalogs();
    expect(modelsCalls).toBe(2);
    setLiveWorkbenchSessionKey("");
  });

  it("records the user turn before the chat stream finishes", async () => {
    setLiveWorkbenchSessionKey("sk-test-live");
    const phases: string[] = [];
    const fetchImpl: typeof fetch = async (input, init) => {
      if (String(input).endsWith("/v1/chat/completions")) {
        return new Promise<Response>(() => undefined);
      }
      return liveFetch()(input, init);
    };
    const workbench = createWorkbenchLiveHandlers({
      fetch: fetchImpl,
      now: () => "2026-09-08T12:00:00Z",
    });
    await workbench.subscribePhase(async (event) => {
      phases.push(event.phase);
    });
    await workbench.getReadiness();
    const conversation = await workbench.create();
    void workbench.send(conversation.id, "画一只猫", true);
    await vi.waitFor(() => expect(phases).toContain("requesting"));
    const listed = await workbench.list();
    expect(listed.messages.map((item) => item.visible_text)).toContain(
      "画一只猫",
    );
    setLiveWorkbenchSessionKey("");
  });

  it("chats against the proxied router and stores a binary image", async () => {
    setLiveWorkbenchSessionKey("sk-test-live");
    const workbench = createWorkbenchLiveHandlers({
      fetch: liveFetch(),
      now: () => "2026-09-08T12:00:00Z",
    });
    const readiness = await workbench.getReadiness();
    expect(readiness.chat_models).toEqual(["gemini-3.8-flash"]);
    expect(
      readiness.image_models.some(
        (item) => item.id === "ag/gemini-3.1-flash-image",
      ),
    ).toBe(true);
    const conversation = await workbench.create();
    const snapshot = await workbench.send(conversation.id, "画一只猫", true);
    const assistant = snapshot.messages.find(
      (item) => item.role === "assistant",
    );
    expect(assistant?.visible_text).toContain("好的");
    expect(assistant?.visible_text).not.toContain("```");
    expect(snapshot.jobs[0]?.status).toBe("succeeded");
    expect(snapshot.assets[0]?.id).toMatch(/^[a-f0-9]{64}$/);
    expect(workbenchAssetUri(snapshot.assets[0]!.id)).toMatch(/^blob:/);
    expect(JSON.stringify(snapshot)).not.toMatch(/sk-/);
    expect(JSON.stringify(snapshot)).not.toContain("base64");
    setLiveWorkbenchSessionKey("");
  });
});
