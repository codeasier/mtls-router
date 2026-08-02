import { spawn, spawnSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import net from "node:net";
import path from "node:path";
import process from "node:process";
import { setTimeout } from "node:timers";
import console from "node:console";
import { fileURLToPath } from "node:url";

import { createAgentEnvironment } from "./dev-agent.mjs";
import { resolveTarget } from "./dev-tauri-reuse.mjs";
import {
  bridgePaths,
  createBridge,
  validateUpstreamBase,
} from "./dev-image-bridge.mjs";

const desktopDir = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const reuseScript = path.join(desktopDir, "scripts", "dev-tauri-reuse.mjs");

export function parseDevImageArgs(args) {
  let upstream = "";
  let bridgePort = 19443;
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--upstream" && index + 1 < args.length) {
      upstream = args[++index];
    } else if (arg === "--bridge-port" && index + 1 < args.length) {
      bridgePort = Number(args[++index]);
    } else {
      throw new Error(`unknown or incomplete argument: ${arg}`);
    }
  }
  if (!upstream) throw new Error("--upstream is required");
  if (!Number.isInteger(bridgePort) || bridgePort < 1 || bridgePort > 65535) {
    throw new Error("--bridge-port must be between 1 and 65535");
  }
  if (bridgePort === 19099) {
    throw new Error("--bridge-port must not use the fixed router port 19099");
  }
  return { upstreamBase: validateUpstreamBase(upstream), bridgePort };
}

function runOpenSSL(args) {
  const result = spawnSync("openssl", args, { encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error("failed to generate local development TLS material");
  }
}

export function generateTLSMaterial(directory) {
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const files = {
    ca: path.join(directory, "upstream-ca.pem"),
    caKey: path.join(directory, "ca.key"),
    server: path.join(directory, "server.pem"),
    serverKey: path.join(directory, "server.key"),
    serverCSR: path.join(directory, "server.csr"),
    serverExt: path.join(directory, "server.ext"),
    client: path.join(directory, "client.pem"),
    clientKey: path.join(directory, "client.key"),
    clientCSR: path.join(directory, "client.csr"),
    clientExt: path.join(directory, "client.ext"),
  };
  runOpenSSL([
    "req",
    "-x509",
    "-newkey",
    "rsa:2048",
    "-nodes",
    "-days",
    "2",
    "-keyout",
    files.caKey,
    "-out",
    files.ca,
    "-subj",
    "/CN=mtls-router-dev-image-ca",
    "-addext",
    "basicConstraints=critical,CA:TRUE",
    "-addext",
    "keyUsage=critical,keyCertSign,cRLSign",
  ]);
  writeFileSync(
    files.serverExt,
    "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=IP:127.0.0.1\n",
    { mode: 0o600 },
  );
  runOpenSSL([
    "req",
    "-newkey",
    "rsa:2048",
    "-nodes",
    "-keyout",
    files.serverKey,
    "-out",
    files.serverCSR,
    "-subj",
    "/CN=127.0.0.1",
  ]);
  runOpenSSL([
    "x509",
    "-req",
    "-in",
    files.serverCSR,
    "-CA",
    files.ca,
    "-CAkey",
    files.caKey,
    "-CAcreateserial",
    "-days",
    "2",
    "-out",
    files.server,
    "-extfile",
    files.serverExt,
  ]);
  writeFileSync(
    files.clientExt,
    "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=clientAuth\n",
    { mode: 0o600 },
  );
  runOpenSSL([
    "req",
    "-newkey",
    "rsa:2048",
    "-nodes",
    "-keyout",
    files.clientKey,
    "-out",
    files.clientCSR,
    "-subj",
    "/CN=mtls-router-dev-image-client",
  ]);
  runOpenSSL([
    "x509",
    "-req",
    "-in",
    files.clientCSR,
    "-CA",
    files.ca,
    "-CAkey",
    files.caKey,
    "-CAcreateserial",
    "-days",
    "2",
    "-out",
    files.client,
    "-extfile",
    files.clientExt,
  ]);
  for (const keyFile of [files.caKey, files.serverKey, files.clientKey]) {
    chmodSync(keyFile, 0o600);
  }
  return files;
}

async function assertPortAvailable(port, label) {
  await new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", () =>
      reject(new Error(`${label} port ${port} is already in use`)),
    );
    server.listen(port, "127.0.0.1", () => server.close(resolve));
  });
}

export function snapshotFiles(files) {
  return files.map((file) =>
    existsSync(file)
      ? { file, data: readFileSync(file), mode: statSync(file).mode }
      : { file, data: undefined, mode: undefined },
  );
}

export function restoreFiles(snapshots) {
  for (const snapshot of snapshots) {
    if (snapshot.data === undefined) {
      rmSync(snapshot.file, { force: true });
    } else {
      writeFileSync(snapshot.file, snapshot.data, { mode: snapshot.mode });
    }
  }
}

function terminateChild(child, signal) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  if (process.platform === "win32") {
    spawnSync("taskkill", ["/pid", String(child.pid), "/t", "/f"], {
      stdio: "ignore",
    });
    return;
  }
  try {
    process.kill(-child.pid, signal);
  } catch {
    child.kill(signal);
  }
}

function terminateProcessGroup(pid, signal) {
  if (process.platform === "win32") {
    spawnSync("taskkill", ["/pid", String(pid), "/t", "/f"], {
      stdio: "ignore",
    });
    return;
  }
  try {
    process.kill(-pid, signal);
  } catch {
    // The process group has already exited.
  }
}

async function stopProcessGroups(processGroups, signal) {
  for (const pid of processGroups) terminateProcessGroup(pid, signal);
  if (process.platform === "win32") return;
  await new Promise((resolve) => setTimeout(resolve, 500));
  for (const pid of processGroups) {
    try {
      process.kill(-pid, "SIGKILL");
    } catch {
      // The process group honored the first signal.
    }
  }
}

function spawnAndWait(command, args, options, onChild) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      ...options,
      detached: process.platform !== "win32",
      shell: process.platform === "win32",
      stdio: "inherit",
    });
    onChild(child);
    child.once("error", (error) => {
      onChild(undefined);
      reject(error);
    });
    child.once("exit", (code, signal) => {
      onChild(undefined);
      if (signal) {
        const error = new Error(`${command} exited on signal ${signal}`);
        error.exitCode = signal === "SIGINT" ? 130 : 143;
        reject(error);
      } else if (code === 0) resolve();
      else reject(new Error(`${command} exited with status ${code}`));
    });
  });
}

export async function runDevImage(
  args = process.argv.slice(2),
  inheritedEnv = process.env,
) {
  const { upstreamBase, bridgePort } = parseDevImageArgs(args);
  await assertPortAvailable(19099, "router");
  await assertPortAvailable(bridgePort, "bridge");

  const target = resolveTarget({});
  if (!target) throw new Error("cannot resolve host target triple");
  const suffix = target.includes("windows") ? ".exe" : "";
  const binariesDir = path.join(desktopDir, "src-tauri", "binaries");
  const sidecarSnapshots = snapshotFiles(
    ["mtls-router-manager", "mtls-router"].map((name) =>
      path.join(binariesDir, `${name}-${target}${suffix}`),
    ),
  );
  const persistentRoot = inheritedEnv.MTLS_ROUTER_DEV_IMAGE_ROOT;
  const isolated = createAgentEnvironment({
    ...inheritedEnv,
    MTLS_ROUTER_DEV_AGENT_ROOT: persistentRoot,
  });
  let tlsDir;
  let bridge;
  let activeChild;
  let currentProcessGroup;
  const processGroups = new Set();
  let requestedSignal;
  const handleSignal = (signal) => {
    requestedSignal ||= signal;
    terminateChild(activeChild, signal);
  };
  const interruptIfRequested = () => {
    if (!requestedSignal) return;
    const error = new Error(`interrupted by ${requestedSignal}`);
    error.exitCode = requestedSignal === "SIGINT" ? 130 : 143;
    throw error;
  };
  const trackChild = (child) => {
    if (!child) {
      if (process.platform === "win32" && activeChild?.pid) {
        processGroups.delete(activeChild.pid);
      }
      activeChild = undefined;
      return;
    }
    activeChild = child;
    currentProcessGroup = child.pid;
    if (child.pid) processGroups.add(child.pid);
    if (requestedSignal) terminateChild(child, requestedSignal);
  };
  const releaseCompletedProcessGroup = async () => {
    if (!currentProcessGroup) return;
    if (process.platform !== "win32") {
      await stopProcessGroups(new Set([currentProcessGroup]), "SIGTERM");
    }
    processGroups.delete(currentProcessGroup);
    currentProcessGroup = undefined;
  };
  const handledSignals =
    process.platform === "win32"
      ? ["SIGINT", "SIGTERM"]
      : ["SIGHUP", "SIGINT", "SIGTERM"];
  for (const signal of handledSignals) process.on(signal, handleSignal);
  let workError;
  let cleanupError;
  try {
    tlsDir = mkdtempSync(path.join(isolated.stateRoot, ".image-bridge-tls-"));
    const tls = generateTLSMaterial(tlsDir);
    bridge = createBridge({
      upstreamBase,
      key: readFileSync(tls.serverKey),
      cert: readFileSync(tls.server),
      ca: readFileSync(tls.ca),
    });
    await new Promise((resolve, reject) => {
      bridge.once("error", reject);
      bridge.listen(bridgePort, "127.0.0.1", resolve);
    });

    const npm = process.platform === "win32" ? "npm.cmd" : "npm";
    const env = {
      ...isolated.env,
      TAURI_ENV_TARGET_TRIPLE: target,
      MTLS_ROUTER_DEV_CERT_DIR: tlsDir,
      UPSTREAM_URL: `https://127.0.0.1:${bridgePort}${bridgePaths.ready}`,
    };
    console.log(
      `dev:image: upstream ${upstreamBase.origin}${upstreamBase.pathname}`,
    );
    console.log(`dev:image: bridge https://127.0.0.1:${bridgePort}`);
    console.log(`dev:image: isolated data ${isolated.stateRoot}`);
    console.log("dev:image: enter the API key in the app's API Keys page");

    interruptIfRequested();
    await spawnAndWait(
      npm,
      ["run", "sidecars:build"],
      { cwd: desktopDir, env },
      trackChild,
    );
    await releaseCompletedProcessGroup();
    interruptIfRequested();
    await spawnAndWait(
      process.execPath,
      [reuseScript],
      { cwd: desktopDir, env },
      trackChild,
    );
    await releaseCompletedProcessGroup();
    interruptIfRequested();
  } catch (error) {
    workError = error;
  } finally {
    for (const signal of handledSignals) process.off(signal, handleSignal);
    try {
      await stopProcessGroups(processGroups, requestedSignal || "SIGTERM");
      if (bridge?.listening) {
        bridge.closeAllConnections();
        await new Promise((resolve) => bridge.close(resolve));
      }
      restoreFiles(sidecarSnapshots);
    } catch (error) {
      cleanupError = error;
    }
    try {
      if (tlsDir) rmSync(tlsDir, { recursive: true, force: true });
      if (isolated.temporary)
        rmSync(isolated.stateRoot, { recursive: true, force: true });
    } catch (error) {
      cleanupError ||= error;
    }
  }
  if (cleanupError) throw cleanupError;
  if (workError) throw workError;
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  runDevImage().catch((error) => {
    console.error(`dev:image: ${error.message}`);
    process.exitCode = error.exitCode || 1;
  });
}
