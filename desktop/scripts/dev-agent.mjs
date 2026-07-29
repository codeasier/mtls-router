import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { spawn } from "node:child_process";
import console from "node:console";
import { fileURLToPath } from "node:url";

const desktopDir = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const reuseScript = path.join(desktopDir, "scripts", "dev-tauri-reuse.mjs");

export function createAgentEnvironment(env = process.env) {
  const persistentRoot = env.MTLS_ROUTER_DEV_AGENT_ROOT;
  const stateRoot = persistentRoot
    ? path.resolve(persistentRoot)
    : mkdtempSync(path.join(os.tmpdir(), "mtls-router-dev-agent-"));
  const desktopData = path.join(stateRoot, "desktop");
  const claudeDir = path.join(stateRoot, "claude");
  const opencodePath = path.join(stateRoot, "opencode.json");
  const codexHome = path.join(stateRoot, "codex");
  const routerState = path.join(stateRoot, "cli");
  mkdirSync(desktopData, { recursive: true });
  mkdirSync(claudeDir, { recursive: true });
  mkdirSync(codexHome, { recursive: true });
  mkdirSync(routerState, { recursive: true });

  return {
    stateRoot,
    temporary: !persistentRoot,
    env: {
      ...env,
      VITE_MOCK: "false",
      MTLS_ROUTER_DESKTOP_DATA_DIR: desktopData,
      CLAUDE_CONFIG_DIR: claudeDir,
      OPENCODE_CONFIG: opencodePath,
      CODEX_HOME: codexHome,
      MTLS_ROUTER_STATE_DIR: routerState,
      MTLS_ROUTER_LOG_PATH: path.join(routerState, "router.log"),
      DEPLOYMENT_ID: env.DEPLOYMENT_ID || "dev",
      VERSION: env.VERSION || "dev",
      MANAGEMENT_PROTOCOL_VERSION: env.MANAGEMENT_PROTOCOL_VERSION || "4",
    },
  };
}

export function run() {
  const isolated = createAgentEnvironment();
  const cleanup = () => {
    if (isolated.temporary) {
      rmSync(isolated.stateRoot, { recursive: true, force: true });
    }
  };
  process.once("exit", cleanup);

  console.log("dev:agent: isolated paths");
  for (const name of [
    "MTLS_ROUTER_DESKTOP_DATA_DIR",
    "CLAUDE_CONFIG_DIR",
    "OPENCODE_CONFIG",
    "CODEX_HOME",
  ]) {
    console.log(`  ${name}=${isolated.env[name]}`);
  }
  console.log(
    isolated.temporary
      ? `  (temporary root ${isolated.stateRoot} will be removed on exit)`
      : "  (persistent root from MTLS_ROUTER_DEV_AGENT_ROOT; not auto-deleted)",
  );
  console.log("dev:agent: fixed router port 127.0.0.1:19099 is NOT isolated");

  const child = spawn(
    process.execPath,
    [reuseScript, ...process.argv.slice(2)],
    {
      cwd: desktopDir,
      env: isolated.env,
      stdio: "inherit",
    },
  );
  child.on("error", (error) => {
    cleanup();
    console.error(`dev:agent: failed to start Tauri: ${error.message}`);
    process.exitCode = 1;
  });
  child.on("exit", (code, signal) => {
    cleanup();
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exitCode = code ?? 1;
  });
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  run();
}
