import { existsSync, readFileSync } from "node:fs";
import type { IncomingMessage, ServerResponse } from "node:http";
import { homedir } from "node:os";
import path from "node:path";

import react from "@vitejs/plugin-react";
import type { Plugin } from "vite";
import { defineConfig } from "vitest/config";

const host = process.env.TAURI_DEV_HOST;
const liveWorkbench = process.env.VITE_WORKBENCH_LIVE === "true";

function desktopDataDir(): string {
  if (process.env.MTLS_ROUTER_DESKTOP_DATA_DIR) {
    return process.env.MTLS_ROUTER_DESKTOP_DATA_DIR;
  }
  const home = homedir();
  if (process.platform === "darwin") {
    return path.join(
      home,
      "Library",
      "Application Support",
      "com.codeasier.mtls-router",
    );
  }
  if (process.platform === "win32") {
    return path.join(
      process.env.APPDATA || path.join(home, "AppData", "Roaming"),
      "com.codeasier.mtls-router",
    );
  }
  return path.join(
    process.env.XDG_DATA_HOME || path.join(home, ".local", "share"),
    "com.codeasier.mtls-router",
  );
}

function resolveLiveWorkbenchKey(): string {
  const fromEnv = process.env.WORKBENCH_API_KEY?.trim();
  if (fromEnv) return fromEnv;
  const file = path.join(desktopDataDir(), "credentials.json");
  if (!existsSync(file)) return "";
  try {
    const parsed = JSON.parse(readFileSync(file, "utf8")) as {
      version?: number;
      key?: unknown;
    };
    if (parsed.version === 1 && typeof parsed.key === "string") {
      return parsed.key.trim();
    }
  } catch {
    return "";
  }
  return "";
}

function liveWorkbenchStatusPlugin(): Plugin {
  return {
    name: "live-workbench-status",
    configureServer(server) {
      server.middlewares.use(
        "/__workbench-live",
        (_request: IncomingMessage, response: ServerResponse) => {
          response.setHeader("Content-Type", "application/json");
          response.end(
            JSON.stringify({
              upstream: "127.0.0.1:19099",
              has_server_key: Boolean(resolveLiveWorkbenchKey()),
            }),
          );
        },
      );
    },
  };
}

export default defineConfig({
  plugins: [react(), ...(liveWorkbench ? [liveWorkbenchStatusPlugin()] : [])],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
    proxy: liveWorkbench
      ? {
          "/__router": {
            target: "http://127.0.0.1:19099",
            changeOrigin: true,
            timeout: 180_000,
            proxyTimeout: 180_000,
            rewrite: (path) => path.replace(/^\/__router/, ""),
            configure(proxy) {
              proxy.on("proxyReq", (proxyReq, request) => {
                if (request.headers.authorization) return;
                const key = resolveLiveWorkbenchKey();
                if (key) {
                  proxyReq.setHeader("Authorization", `Bearer ${key}`);
                }
              });
              proxy.on("proxyRes", (proxyRes, request, response) => {
                if (!(request.url ?? "").includes("/v1/chat/completions")) {
                  return;
                }
                proxyRes.headers["x-accel-buffering"] = "no";
                proxyRes.headers["cache-control"] = "no-cache";
                response.setHeader("X-Accel-Buffering", "no");
                response.setHeader("Cache-Control", "no-cache");
              });
            },
          },
        }
      : undefined,
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target:
      process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: process.env.TAURI_ENV_DEBUG ? false : "oxc",
    sourcemap: Boolean(process.env.TAURI_ENV_DEBUG),
  },
  test: {
    environment: "jsdom",
    setupFiles: "./src/test/setup.ts",
  },
});
