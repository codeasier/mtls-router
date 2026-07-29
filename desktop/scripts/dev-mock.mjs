import { spawn } from "node:child_process";
import console from "node:console";
import process from "node:process";

const command = process.platform === "win32" ? "vite.cmd" : "vite";
const child = spawn(command, process.argv.slice(2), {
  env: { ...process.env, VITE_MOCK: "true" },
  shell: process.platform === "win32",
  stdio: "inherit",
});

child.on("error", (error) => {
  console.error(`dev:mock: failed to start Vite: ${error.message}`);
  process.exitCode = 1;
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exitCode = code ?? 1;
});
