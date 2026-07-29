import { existsSync, statSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { spawn, spawnSync } from "node:child_process";
import console from "node:console";
import { fileURLToPath } from "node:url";

const desktopDir = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

export function resolveTarget(env = process.env) {
  if (env.TAURI_ENV_TARGET_TRIPLE || env.TARGET) {
    return env.TAURI_ENV_TARGET_TRIPLE || env.TARGET;
  }
  const result = spawnSync("rustc", ["--print", "host-tuple"], {
    encoding: "utf8",
  });
  return result.status === 0 ? result.stdout.trim() : "";
}

export function missingSidecars(target) {
  const suffix = target.includes("windows") ? ".exe" : "";
  return ["mtls-router-manager", "mtls-router"]
    .map((name) =>
      path.join(
        desktopDir,
        "src-tauri",
        "binaries",
        `${name}-${target}${suffix}`,
      ),
    )
    .filter((file) => !existsSync(file) || statSync(file).size === 0);
}

export function reuseEnvironment(env = process.env) {
  return {
    ...env,
    VITE_MOCK: "false",
    DEPLOYMENT_ID: env.DEPLOYMENT_ID || "dev",
    VERSION: env.VERSION || "dev",
    MANAGEMENT_PROTOCOL_VERSION: env.MANAGEMENT_PROTOCOL_VERSION || "4",
  };
}

export function run() {
  const target = resolveTarget();
  if (!target) {
    console.error(
      "dev:tauri:reuse: cannot resolve host target triple; install rustc or set TARGET",
    );
    return 1;
  }

  const missing = missingSidecars(target);
  for (const file of missing) {
    console.error(`dev:tauri:reuse: missing or empty sidecar ${file}`);
  }
  if (missing.length > 0) {
    console.error(
      "dev:tauri:reuse: run `npm run sidecars:build` (or `npm run tauri -- dev`) once before reuse",
    );
    return 1;
  }

  console.log(
    `dev:tauri:reuse: reusing sidecars for ${target} (no sidecars:build)`,
  );
  const npm = process.platform === "win32" ? "npm.cmd" : "npm";
  const child = spawn(
    npm,
    ["exec", "--", "tauri", "dev", ...process.argv.slice(2)],
    {
      cwd: desktopDir,
      env: reuseEnvironment(),
      shell: process.platform === "win32",
      stdio: "inherit",
    },
  );
  child.on("error", (error) => {
    console.error(`dev:tauri:reuse: failed to start Tauri: ${error.message}`);
    process.exitCode = 1;
  });
  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exitCode = code ?? 1;
  });
  return undefined;
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const code = run();
  if (code !== undefined) process.exitCode = code;
}
