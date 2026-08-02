import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { spawn, spawnSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import http from "node:http";
import https from "node:https";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import test from "node:test";
import { clearTimeout, setTimeout } from "node:timers";
import { fileURLToPath } from "node:url";

import {
  bridgePaths,
  createBridge,
  filterHeaders,
  mapTargetURL,
  validateUpstreamBase,
} from "./dev-image-bridge.mjs";
import {
  generateTLSMaterial,
  parseDevImageArgs,
  restoreFiles,
  snapshotFiles,
} from "./dev-image.mjs";

const scriptsDir = path.dirname(fileURLToPath(import.meta.url));

function listen(server) {
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => resolve(server.address().port));
  });
}

function close(server) {
  server.closeAllConnections?.();
  return new Promise((resolve) => server.close(resolve));
}

async function availablePort() {
  const server = net.createServer();
  const port = await listen(server);
  await close(server);
  return port;
}

function requestBridge({
  port,
  tls,
  path: requestPath,
  method = "GET",
  headers = {},
  body,
}) {
  return new Promise((resolve, reject) => {
    const request = https.request(
      {
        hostname: "127.0.0.1",
        port,
        path: requestPath,
        method,
        ca: readFileSync(tls.ca),
        cert: readFileSync(tls.client),
        key: readFileSync(tls.clientKey),
        headers,
      },
      (response) => {
        const chunks = [];
        response.on("data", (chunk) => chunks.push(chunk));
        response.on("end", () => {
          resolve({
            status: response.statusCode,
            headers: response.headers,
            body: Buffer.concat(chunks).toString("utf8"),
          });
        });
      },
    );
    request.once("error", reject);
    if (body) request.write(body);
    request.end();
  });
}

test("upstream validation rejects unsafe plaintext and malformed bases", () => {
  assert.equal(
    validateUpstreamBase("http://127.0.0.1:8080/v1").pathname,
    "/v1",
  );
  assert.equal(validateUpstreamBase("http://[::1]:8080/v1").hostname, "[::1]");
  assert.equal(
    validateUpstreamBase("https://api.example.test/gateway/v1/").pathname,
    "/gateway/v1",
  );
  assert.throws(
    () => validateUpstreamBase("http://api.example.test/v1"),
    /private or loopback/,
  );
  assert.throws(
    () => validateUpstreamBase("http://8.8.8.8/v1"),
    /private or loopback/,
  );
  assert.throws(
    () => validateUpstreamBase("https://user:secret@example.test/v1"),
    /credentials/,
  );
  assert.throws(
    () => validateUpstreamBase("https://example.test/api"),
    /ending in \/v1/,
  );
  assert.throws(() => validateUpstreamBase("file:///v1"), /HTTP or HTTPS/);
});

test("argument parsing and path mapping preserve the API base", () => {
  const { upstreamBase, bridgePort } = parseDevImageArgs([
    "--upstream",
    "https://api.example.test/gateway/v1",
    "--bridge-port",
    "20443",
  ]);
  assert.equal(bridgePort, 20443);
  assert.equal(
    mapTargetURL(upstreamBase, "/v1/images/generations?trace=1").href,
    "https://api.example.test/gateway/v1/images/generations?trace=1",
  );
  assert.throws(() => parseDevImageArgs([]), /--upstream is required/);
  assert.throws(
    () => parseDevImageArgs(["--upstream", "https://example.test/v1", "--bad"]),
    /unknown/,
  );
  assert.throws(
    () =>
      parseDevImageArgs([
        "--upstream",
        "https://example.test/v1",
        "--bridge-port",
        "19099",
      ]),
    /fixed router port/,
  );
});

test("sidecar snapshots restore existing files and remove generated files", () => {
  const tempDir = mkdtempSync(
    path.join(os.tmpdir(), "mtls-router-sidecar-snapshot-test-"),
  );
  const existing = path.join(tempDir, "existing");
  const generated = path.join(tempDir, "generated");
  try {
    writeFileSync(existing, "before", { mode: 0o700 });
    const snapshots = snapshotFiles([existing, generated]);
    writeFileSync(existing, "debug");
    writeFileSync(generated, "debug");
    restoreFiles(snapshots);
    assert.equal(readFileSync(existing, "utf8"), "before");
    assert.equal(existsSync(generated), false);
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
});

test(
  "launcher terminates its child and removes short-lived TLS on SIGTERM",
  { skip: process.platform === "win32" },
  async () => {
    const tempDir = mkdtempSync(
      path.join(os.tmpdir(), "mtls-router-dev-image-signal-test-"),
    );
    const binDir = path.join(tempDir, "bin");
    const stateRoot = path.join(tempDir, "state");
    const fakeTarget = "dev-image-test-target";
    const router = path.join(
      scriptsDir,
      "..",
      "src-tauri",
      "binaries",
      `mtls-router-${fakeTarget}`,
    );
    const manager = path.join(
      scriptsDir,
      "..",
      "src-tauri",
      "binaries",
      `mtls-router-manager-${fakeTarget}`,
    );
    try {
      mkdirSync(binDir);
      const rustc = path.join(binDir, "rustc");
      const npm = path.join(binDir, "npm");
      writeFileSync(rustc, `#!/bin/sh\nprintf '%s\\n' '${fakeTarget}'\n`);
      writeFileSync(npm, "#!/bin/sh\nsleep 30\n");
      chmodSync(rustc, 0o700);
      chmodSync(npm, 0o700);
      const bridgePort = await availablePort();
      const child = spawn(
        process.execPath,
        [
          path.join(scriptsDir, "dev-image.mjs"),
          "--upstream",
          "http://127.0.0.1:9/v1",
          "--bridge-port",
          String(bridgePort),
        ],
        {
          env: {
            ...process.env,
            PATH: `${binDir}${path.delimiter}${process.env.PATH}`,
            MTLS_ROUTER_DEV_IMAGE_ROOT: stateRoot,
          },
          stdio: ["ignore", "pipe", "pipe"],
        },
      );
      let output = "";
      child.stdout.setEncoding("utf8");
      child.stdout.on("data", (chunk) => {
        output += chunk;
        if (output.includes("enter the API key")) child.kill("SIGTERM");
      });
      const result = await new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
          child.kill("SIGKILL");
          reject(new Error("signal cleanup test timed out"));
        }, 15000);
        child.once("error", reject);
        child.once("exit", (code, signal) => {
          clearTimeout(timeout);
          resolve({ code, signal });
        });
      });
      assert.deepEqual(result, { code: 143, signal: null });
      assert.equal(existsSync(router), false);
      assert.equal(existsSync(manager), false);
      assert.equal(
        readdirSync(stateRoot).some((name) =>
          name.startsWith(".image-bridge-tls-"),
        ),
        false,
      );
    } finally {
      rmSync(router, { force: true });
      rmSync(manager, { force: true });
      rmSync(tempDir, { recursive: true, force: true });
    }
  },
);

test("header filtering removes hop-by-hop headers and rewrites request host", () => {
  assert.deepEqual(
    filterHeaders(
      {
        authorization: "Bearer opaque",
        connection: "keep-alive, x-private",
        "x-private": "remove-me",
        "x-public": "keep-me",
      },
      "upstream.test:8443",
    ),
    {
      authorization: "Bearer opaque",
      "x-public": "keep-me",
      host: "upstream.test:8443",
    },
  );
});

test("bridge requires mTLS and transparently proxies only image endpoints", async () => {
  const tempDir = mkdtempSync(
    path.join(os.tmpdir(), "mtls-router-dev-image-test-"),
  );
  const tls = generateTLSMaterial(path.join(tempDir, "tls"));
  const seen = [];
  const upstream = http.createServer((request, response) => {
    const chunks = [];
    request.on("data", (chunk) => chunks.push(chunk));
    request.on("end", () => {
      seen.push({
        method: request.method,
        url: request.url,
        authorization: request.headers.authorization,
        body: Buffer.concat(chunks).toString("utf8"),
      });
      response.setHeader("Connection", "x-internal");
      response.setHeader("X-Internal", "remove-me");
      response.setHeader("Content-Type", "application/json");
      response.end(
        request.method === "GET"
          ? JSON.stringify({ data: [{ id: "image-test" }] })
          : JSON.stringify({ created: 1, data: [{ b64_json: "aW1hZ2U=" }] }),
      );
    });
  });

  let bridge;
  try {
    const upstreamPort = await listen(upstream);
    bridge = createBridge({
      upstreamBase: validateUpstreamBase(
        `http://127.0.0.1:${upstreamPort}/gateway/v1`,
      ),
      key: readFileSync(tls.serverKey),
      cert: readFileSync(tls.server),
      ca: readFileSync(tls.ca),
    });
    const bridgePort = await listen(bridge);

    const ready = await requestBridge({
      port: bridgePort,
      tls,
      path: bridgePaths.ready,
    });
    assert.equal(ready.status, 204);

    const catalog = await requestBridge({
      port: bridgePort,
      tls,
      path: bridgePaths.catalog,
      headers: { Authorization: "Bearer opaque-test-key" },
    });
    assert.equal(catalog.status, 200);
    assert.deepEqual(JSON.parse(catalog.body), {
      data: [{ id: "image-test" }],
    });
    assert.equal(catalog.headers["x-internal"], undefined);

    const requestBody = JSON.stringify({ model: "image-test", prompt: "draw" });
    const generation = await requestBridge({
      port: bridgePort,
      tls,
      path: bridgePaths.generation,
      method: "POST",
      headers: {
        Authorization: "Bearer opaque-test-key",
        "Content-Type": "application/json",
        "Content-Length": Buffer.byteLength(requestBody),
      },
      body: requestBody,
    });
    assert.equal(generation.status, 200);
    assert.equal(JSON.parse(generation.body).data[0].b64_json, "aW1hZ2U=");
    assert.deepEqual(seen, [
      {
        method: "GET",
        url: "/gateway/v1/models/image",
        authorization: "Bearer opaque-test-key",
        body: "",
      },
      {
        method: "POST",
        url: "/gateway/v1/images/generations",
        authorization: "Bearer opaque-test-key",
        body: requestBody,
      },
    ]);

    const unsupported = await requestBridge({
      port: bridgePort,
      tls,
      path: "/version",
    });
    assert.equal(unsupported.status, 404);

    await assert.rejects(
      new Promise((resolve, reject) => {
        https
          .get(
            {
              hostname: "127.0.0.1",
              port: bridgePort,
              path: bridgePaths.ready,
              ca: readFileSync(tls.ca),
            },
            resolve,
          )
          .once("error", reject);
      }),
    );
  } finally {
    if (bridge?.listening) await close(bridge);
    if (upstream.listening) await close(upstream);
    rmSync(tempDir, { recursive: true, force: true });
  }
});

test("sidecar build rejects development certificates for release and partial input", () => {
  const script = path.join(scriptsDir, "build-sidecars.sh");
  const tempDir = mkdtempSync(
    path.join(os.tmpdir(), "mtls-router-dev-cert-test-"),
  );
  const baseEnv = { PATH: process.env.PATH, HOME: process.env.HOME };
  try {
    const release = spawnSync("bash", [script], {
      env: {
        ...baseEnv,
        RELEASE_BUILD: "1",
        MTLS_ROUTER_DEV_CERT_DIR: tempDir,
      },
      encoding: "utf8",
    });
    assert.notEqual(release.status, 0);
    assert.match(release.stderr, /forbidden for release builds/);

    writeFileSync(path.join(tempDir, "client.pem"), "partial");
    const partial = spawnSync("bash", [script], {
      env: { ...baseEnv, MTLS_ROUTER_DEV_CERT_DIR: tempDir },
      encoding: "utf8",
    });
    assert.notEqual(partial.status, 0);
    assert.match(
      partial.stderr,
      /must contain client.pem, client.key, and upstream-ca.pem/,
    );
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
});
