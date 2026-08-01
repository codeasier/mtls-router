import { afterEach, describe, expect, it } from "vitest";

import { createMockDesktopApi, setMockScenario } from "./mockDesktopApi";
import {
  fixtureAbsentStatus,
  fixtureOwnedStatus,
  type MockScenario,
} from "./fixtures";

afterEach(() => {
  delete (window as Window & { __MTLS_MOCK_SCENARIO__?: string })
    .__MTLS_MOCK_SCENARIO__;
});

describe("createMockDesktopApi", () => {
  it("starts router and updates poll snapshots without real sidecars", async () => {
    const api = createMockDesktopApi({ initialStatus: fixtureAbsentStatus });
    const snapshots: unknown[] = [];
    const stop = await api.subscribePollSnapshots((snapshot) => {
      snapshots.push(snapshot);
    });

    const started = await api.startRouter();
    expect(started.state).toBe("desktop_owned");
    expect(started.listen_addr).toBe("127.0.0.1:19099");
    expect(snapshots).toHaveLength(1);

    const stopped = await api.stopRouter();
    expect(stopped).toEqual(fixtureAbsentStatus);
    expect(snapshots).toHaveLength(2);
    stop();
  });

  it("keeps credential and agent write state in memory only", async () => {
    const api = createMockDesktopApi({ credentialPresent: true });
    expect((await api.getCredential()).present).toBe(true);

    await api.deleteCredential();
    expect((await api.getCredential()).present).toBe(false);

    await api.saveCredential("mock-only-key");
    expect((await api.getCredential()).fingerprint).toBe("MOCK");

    const detection = await api.detectAgents();
    expect(detection.agents).toHaveLength(3);

    const discovery = await api.discoverModels(["claude"]);
    const preview = await api.previewAgents(
      ["claude"],
      discovery.flow_id,
      discovery.catalog_token,
      discovery.existing.model_config as never,
      { claude: "merge" },
    );
    expect(preview.revision_token).toBe("revision-mock");

    const written = await api.writeAgents(
      ["claude"],
      discovery.flow_id,
      discovery.catalog_token,
      preview.model_config,
      preview.revision_token,
      false,
      false,
      [],
    );
    expect(written.agents[0]?.success).toBe(true);
    expect(JSON.stringify(written)).not.toMatch(/sk-|BEGIN |api[_-]?key/i);
  });

  it("keeps image conversation model changes in memory", async () => {
    const api = createMockDesktopApi();
    const conversation = await api.imageCreateConversation("cx/gpt-5.5-image");

    await api.imageSetConversationModel(
      conversation.id,
      "ag/gemini-3.1-flash-image",
    );

    expect(await api.imageConversations()).toEqual([
      expect.objectContaining({
        id: conversation.id,
        selected_model: "ag/gemini-3.1-flash-image",
      }),
    ]);
  });

  it.each([
    ["protocol-error", "AGENT_DETECT_IO"],
    ["preview-stale", "AGENT_PREVIEW_REVISION_MISMATCH"],
    ["write-fail", "AGENT_WRITE_FAILED"],
  ] as const satisfies ReadonlyArray<readonly [MockScenario, string]>)(
    "supports the %s failure scenario",
    async (scenario, expectedCode) => {
      setMockScenario(scenario);
      const api = createMockDesktopApi({ scenario: "success" });

      if (scenario === "protocol-error") {
        await expect(api.detectAgents()).rejects.toMatchObject({
          code: expectedCode,
        });
        return;
      }

      const discovery = await api.discoverModels(["claude"]);
      if (scenario === "preview-stale") {
        await expect(
          api.previewAgents(
            ["claude"],
            discovery.flow_id,
            discovery.catalog_token,
            { version: 1 },
            {},
          ),
        ).rejects.toMatchObject({ code: expectedCode });
        return;
      }

      const preview = await api.previewAgents(
        ["claude"],
        discovery.flow_id,
        discovery.catalog_token,
        { version: 1 },
        {},
      );
      await expect(
        api.writeAgents(
          ["claude"],
          discovery.flow_id,
          discovery.catalog_token,
          preview.model_config,
          preview.revision_token,
          false,
          false,
          [],
        ),
      ).rejects.toMatchObject({ code: expectedCode });
    },
  );

  it("rejects unknown flow ids and stale revision tokens", async () => {
    const api = createMockDesktopApi();
    await expect(
      api.previewAgents(
        ["claude"],
        "missing-flow",
        "catalog",
        { version: 1 },
        {},
      ),
    ).rejects.toMatchObject({ code: "AGENT_FLOW_UNKNOWN" });

    const discovery = await api.discoverModels(["claude"]);
    const preview = await api.previewAgents(
      ["claude"],
      discovery.flow_id,
      discovery.catalog_token,
      { version: 1 },
      {},
    );
    await expect(
      api.writeAgents(
        ["claude"],
        discovery.flow_id,
        discovery.catalog_token,
        preview.model_config,
        "stale-token",
        false,
        false,
        [],
      ),
    ).rejects.toMatchObject({ code: "AGENT_WRITE_REVISION_MISMATCH" });
  });

  it("exposes a desktop-owned fixture status when requested", async () => {
    const api = createMockDesktopApi({ initialStatus: fixtureOwnedStatus });
    expect(await api.getRouterStatus()).toEqual(fixtureOwnedStatus);
    const snapshot = await api.getPollSnapshot();
    expect(snapshot.status?.state).toBe("desktop_owned");
    expect(snapshot.health?.status).toBe("ok");
  });

  it("provides an in-memory update fixture without installing anything", async () => {
    const api = createMockDesktopApi();

    expect(await api.checkForUpdate()).toMatchObject({
      available: true,
      update: { version: "0.2.0-mock" },
    });
    await expect(api.installUpdate("0.2.0-mock")).resolves.toBeUndefined();
    const stop = await api.subscribeUpdateProgress(() => undefined);
    expect(stop()).toBeUndefined();
  });
});
