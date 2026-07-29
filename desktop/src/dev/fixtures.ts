import type {
  AgentDetection,
  AgentId,
  AgentModelsResult,
  AgentPreview,
  AgentWriteResult,
  CredentialSummary,
  DesktopPaths,
  ModelConfig,
  OccupantInspection,
  PollSnapshot,
  RouterHealth,
  RouterStatus,
} from "../ipc";

export type MockScenario =
  "success" | "protocol-error" | "preview-stale" | "write-fail";

export const MOCK_SCENARIOS: readonly MockScenario[] = [
  "success",
  "protocol-error",
  "preview-stale",
  "write-fail",
] as const;

export const fixtureDetection: AgentDetection = {
  agents: (["claude", "opencode", "codex"] as const).map((agent) => ({
    agent,
    name: agent,
    detected: true,
    command: `/mock/bin/${agent}`,
    path: `/mock/${agent}/config`,
    format: agent === "codex" ? "toml" : "json",
    exists: true,
    writable: true,
    configured: true,
    invalid: false,
    recovery: { eligible: false, files: [] },
  })),
};

export const fixtureConfigs: Record<AgentId, ModelConfig> = {
  claude: {
    version: 1,
    claude: {
      primary: { model: "mock-claude" },
      haiku: { inherit_primary: true },
      sonnet: { inherit_primary: true },
      opus: { inherit_primary: true },
    },
  },
  opencode: {
    version: 1,
    opencode: { default_model: "mock-opencode", models: {} },
  },
  codex: { version: 1, codex: { model: "mock-codex" } },
};

export const fixtureModels = [
  "mock-claude",
  "mock-opencode",
  "mock-codex",
  "preset-claude",
] as const;

export const fixtureCredentialPresent: CredentialSummary = {
  present: true,
  fingerprint: "MOCK",
  saved_at: "2026-07-29T00:00:00Z",
};

export const fixtureCredentialAbsent: CredentialSummary = {
  present: false,
  fingerprint: "",
  saved_at: null,
};

export const fixtureDesktopPaths: DesktopPaths = {
  data_dir: "/mock/app-data",
  log_file: "/mock/app-data/mtls-router.log",
  credentials_path: "/mock/app-data/credentials.json",
  can_prepare_for_uninstall: true,
};

export const fixtureAbsentStatus: RouterStatus = { state: "absent" };

export const fixtureOwnedStatus: RouterStatus = {
  state: "desktop_owned",
  owner: "desktop",
  listen_addr: "127.0.0.1:19099",
  pid: 19099,
};

export const fixtureHealthOk: RouterHealth = {
  status: "ok",
  checked_at: "2026-07-29T00:00:00Z",
};

export const fixtureOccupant: OccupantInspection = {
  pid: 4242,
  verification_mode: "verified_identity",
  process_name: "mock-server",
  executable: "/mock/bin/mock-server",
  listen_addr: "127.0.0.1:19099",
  recovery: { action: "force_terminate" },
  confirmation_token: "mock-token",
  expires_at: "2026-07-29T12:00:30Z",
};

export function discoveryFor(
  flowId: string,
  existing: Partial<ModelConfig> = {},
  preset: Partial<ModelConfig> = {},
): AgentModelsResult {
  return {
    flow_id: flowId,
    models: [...fixtureModels],
    catalog_token: `catalog-${flowId}`,
    router_base_url: "http://127.0.0.1:19099",
    api_base_url: "http://127.0.0.1:19099/v1",
    existing: {
      model_config: { version: 1, ...existing },
      unavailable_models: {},
      drifted_agents: [],
    },
    preset: {
      model_config: { version: 1, ...preset },
      unavailable_agents: {},
    },
  };
}

export function previewFor(
  config: ModelConfig,
  overrides: Partial<AgentPreview> = {},
): AgentPreview {
  return {
    revision_token: "revision-mock",
    model_config: config,
    fragments: [
      {
        agent: "claude",
        role: "settings",
        path: "/mock/claude/config",
        format: "json",
        content: '{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:19099"}}',
      },
    ],
    files: [
      {
        agent: "claude",
        mode: "merge",
        path: "/mock/claude/config",
        role: "settings",
        format: "json",
        operation: "replace",
        backup_required: true,
        backup_pattern: "*.bak",
      },
    ],
    managed_config_drift: false,
    drifted_agents: [],
    managed_collisions: [],
    requires_codex_auth_approval: false,
    ...overrides,
  };
}

export function writeResultFor(
  agents: AgentId[],
  success = true,
): AgentWriteResult {
  return {
    transaction_id: "tx-mock",
    agents: agents.map((agent) =>
      success
        ? {
            agent,
            success: true,
            changed: [`/mock/${agent}/config`],
            backups: [`/mock/${agent}/config.bak`],
          }
        : {
            agent,
            success: false,
            error_code: "AGENT_WRITE_FAILED",
          },
    ),
  };
}

export function pollSnapshotFor(
  status: RouterStatus,
  revision: number,
): PollSnapshot {
  const healthy =
    status.state === "desktop_owned" ||
    status.state === "external_compatible" ||
    status.state === "degraded";
  return {
    revision,
    status,
    health: healthy
      ? {
          status: status.state === "degraded" ? "degraded" : "ok",
          checked_at: new Date().toISOString(),
        }
      : undefined,
  };
}

export function parseMockScenario(
  raw: string | null | undefined,
): MockScenario {
  if (raw && (MOCK_SCENARIOS as readonly string[]).includes(raw)) {
    return raw as MockScenario;
  }
  return "success";
}

export function resolveMockScenarioFromLocation(
  search = typeof window !== "undefined" ? window.location.search : "",
): MockScenario {
  try {
    const params = new URLSearchParams(search);
    return parseMockScenario(params.get("mockScenario"));
  } catch {
    return "success";
  }
}

export function mockCommandError(
  code: string,
  message = "mock diagnostic only",
): { code: string; message: string } {
  return { code, message };
}
