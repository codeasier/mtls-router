import { describe, expect, it } from "vitest";

import type { AgentDetection, AgentState } from "./ipc";
import {
  agentOrder,
  completeAgentDetection,
  configurationPresentation,
} from "./agentPresentation";

const base: AgentState = {
  agent: "claude",
  name: "Claude Code",
  detected: true,
  command: "/safe/bin/claude",
  path: "/safe/.claude/settings.json",
  format: "json",
  exists: true,
  writable: true,
  configured: false,
  invalid: false,
  recovery: { eligible: false, files: [] },
};

describe("Agent presentation state", () => {
  it.each([
    [
      { invalid: true, writable: false, configured: true },
      "invalid",
      "disabled",
    ],
    [
      { invalid: false, writable: false, configured: true },
      "readonly",
      "disabled",
    ],
    [
      { invalid: false, writable: true, configured: true },
      "configured",
      "merge",
    ],
    [
      { invalid: false, writable: true, configured: false, exists: false },
      "create",
      "merge",
    ],
    [
      { invalid: false, writable: true, configured: false, exists: true },
      "ready",
      "merge",
    ],
  ] as const)("applies configuration priority", (override, state, action) => {
    expect(configurationPresentation({ ...base, ...override })).toMatchObject({
      state,
      action,
    });
  });

  it("allows only eligible invalid configuration to rebuild", () => {
    expect(
      configurationPresentation({
        ...base,
        invalid: true,
        recovery: { eligible: true, files: [] },
      }).action,
    ).toBe("rebuild");
  });

  it("accepts exactly one state for each supported Agent", () => {
    const agents = agentOrder.map((agent) => ({ ...base, agent }));
    const detection: AgentDetection = { agents };

    expect(completeAgentDetection(detection)).toEqual(detection);
    expect(completeAgentDetection({ agents: agents.slice(0, 2) })).toBeNull();
    expect(
      completeAgentDetection({ agents: [agents[0], agents[0], agents[2]] }),
    ).toBeNull();
  });
});
