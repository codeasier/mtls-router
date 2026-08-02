import { Buffer } from "node:buffer";
import http from "node:http";
import https from "node:https";
import net from "node:net";
import { Transform } from "node:stream";
import { URL } from "node:url";

const READY_PATH = "/_mtls-router-dev/ready";
const CATALOG_PATH = "/v1/models/image";
const GENERATION_PATH = "/v1/images/generations";
const MAX_CATALOG_BYTES = 1024 * 1024;
const MAX_GENERATION_BYTES = 32 * 1024 * 1024;
const HOP_BY_HOP = new Set([
  "connection",
  "keep-alive",
  "proxy-authenticate",
  "proxy-authorization",
  "te",
  "trailer",
  "transfer-encoding",
  "upgrade",
]);

function isPrivateHttpHost(hostname) {
  const host = hostname.replace(/^\[|\]$/g, "");
  const kind = net.isIP(host);
  if (kind === 4) {
    const octets = host.split(".").map(Number);
    return (
      octets[0] === 10 ||
      octets[0] === 127 ||
      (octets[0] === 172 && octets[1] >= 16 && octets[1] <= 31) ||
      (octets[0] === 192 && octets[1] === 168)
    );
  }
  if (kind === 6) {
    const normalized = host.toLowerCase();
    return (
      normalized === "::1" ||
      normalized.startsWith("fc") ||
      normalized.startsWith("fd")
    );
  }
  return false;
}

export function validateUpstreamBase(value) {
  let target;
  try {
    target = new URL(value);
  } catch {
    throw new Error("upstream must be an absolute HTTP or HTTPS URL");
  }
  if (!["http:", "https:"].includes(target.protocol)) {
    throw new Error("upstream must use HTTP or HTTPS");
  }
  if (target.username || target.password || target.search || target.hash) {
    throw new Error(
      "upstream must not contain credentials, query, or fragment",
    );
  }
  const pathname = target.pathname.replace(/\/+$/, "") || "/v1";
  if (!pathname.endsWith("/v1")) {
    throw new Error("upstream must be an API base URL ending in /v1");
  }
  if (target.protocol === "http:" && !isPrivateHttpHost(target.hostname)) {
    throw new Error(
      "plain HTTP upstream must use a private or loopback IP address",
    );
  }
  target.pathname = pathname;
  return target;
}

export function filterHeaders(headers, targetHost) {
  const connectionTokens = String(headers.connection ?? "")
    .split(",")
    .map((value) => value.trim().toLowerCase())
    .filter(Boolean);
  const blocked = new Set([...HOP_BY_HOP, ...connectionTokens]);
  const result = {};
  for (const [name, value] of Object.entries(headers)) {
    if (!blocked.has(name.toLowerCase()) && value !== undefined) {
      result[name] = value;
    }
  }
  if (targetHost) result.host = targetHost;
  return result;
}

export function mapTargetURL(upstreamBase, incomingURL) {
  const incoming = new URL(incomingURL, "https://127.0.0.1");
  if (![CATALOG_PATH, GENERATION_PATH].includes(incoming.pathname)) {
    throw new Error("unsupported bridge path");
  }
  const target = new URL(upstreamBase.origin);
  target.pathname = `${upstreamBase.pathname}${incoming.pathname.slice(3)}`;
  target.search = incoming.search;
  return target;
}

function sendJSON(response, status, message, extraHeaders = {}) {
  const body = Buffer.from(JSON.stringify({ error: message }));
  response.writeHead(status, {
    "Content-Type": "application/json",
    "Content-Length": body.length,
    ...extraHeaders,
  });
  response.end(body);
}

function endpointLimit(pathname) {
  return pathname === CATALOG_PATH ? MAX_CATALOG_BYTES : MAX_GENERATION_BYTES;
}

class ByteLimit extends Transform {
  constructor(limit, onLimit) {
    super();
    this.limit = limit;
    this.onLimit = onLimit;
    this.total = 0;
  }

  _transform(chunk, encoding, callback) {
    this.total += chunk.length;
    if (this.total > this.limit) {
      this.onLimit();
      callback(new Error("body limit exceeded"));
      return;
    }
    callback(null, chunk);
  }
}

async function targetReachable(target, timeoutMs = 2000) {
  const port = Number(target.port) || (target.protocol === "https:" ? 443 : 80);
  return new Promise((resolve) => {
    const host = target.hostname.replace(/^\[|\]$/g, "");
    const socket = net.createConnection({ host, port });
    const finish = (value) => {
      socket.destroy();
      resolve(value);
    };
    socket.setTimeout(timeoutMs, () => finish(false));
    socket.once("connect", () => finish(true));
    socket.once("error", () => finish(false));
  });
}

export function createBridge({ upstreamBase, key, cert, ca }) {
  const httpAgent = new http.Agent({ keepAlive: true });
  const httpsAgent = new https.Agent({ keepAlive: true });
  const server = https.createServer(
    {
      key,
      cert,
      ca,
      requestCert: true,
      rejectUnauthorized: true,
      minVersion: "TLSv1.2",
      maxHeaderSize: 64 * 1024,
    },
    async (request, response) => {
      const incoming = new URL(request.url ?? "/", "https://127.0.0.1");
      if (incoming.pathname === READY_PATH) {
        if (request.method !== "GET") {
          sendJSON(response, 405, "Method Not Allowed", { Allow: "GET" });
          return;
        }
        if (await targetReachable(upstreamBase)) {
          response.writeHead(204, { "Content-Length": "0" });
          response.end();
        } else {
          sendJSON(response, 503, "Service Unavailable");
        }
        return;
      }

      const expectedMethod =
        incoming.pathname === CATALOG_PATH
          ? "GET"
          : incoming.pathname === GENERATION_PATH
            ? "POST"
            : "";
      if (!expectedMethod) {
        sendJSON(response, 404, "Not Found");
        return;
      }
      if (request.method !== expectedMethod) {
        sendJSON(response, 405, "Method Not Allowed", {
          Allow: expectedMethod,
        });
        return;
      }

      const limit = endpointLimit(incoming.pathname);
      const declaredLength = Number(request.headers["content-length"] ?? 0);
      if (
        !Number.isFinite(declaredLength) ||
        declaredLength < 0 ||
        declaredLength > limit
      ) {
        sendJSON(response, 413, "Payload Too Large");
        return;
      }

      const targetURL = mapTargetURL(upstreamBase, request.url ?? "/");
      const transport = targetURL.protocol === "https:" ? https : http;
      let responseStarted = false;
      const upstreamRequest = transport.request(
        targetURL,
        {
          method: request.method,
          headers: filterHeaders(request.headers, targetURL.host),
          agent: targetURL.protocol === "https:" ? httpsAgent : httpAgent,
          timeout: incoming.pathname === CATALOG_PATH ? 10000 : 180000,
        },
        (upstreamResponse) => {
          const declaredResponseLength = Number(
            upstreamResponse.headers["content-length"] ?? 0,
          );
          if (
            !Number.isFinite(declaredResponseLength) ||
            declaredResponseLength < 0 ||
            declaredResponseLength > limit
          ) {
            upstreamResponse.destroy();
            sendJSON(response, 502, "Bad Gateway");
            return;
          }
          responseStarted = true;
          response.writeHead(
            upstreamResponse.statusCode ?? 502,
            filterHeaders(upstreamResponse.headers),
          );
          const boundedResponse = new ByteLimit(limit, () => {
            upstreamResponse.destroy();
            response.destroy();
          });
          boundedResponse.on("error", () => response.destroy());
          upstreamResponse.pipe(boundedResponse).pipe(response);
        },
      );

      let failureHandled = false;
      const fail = (status = 502, message = "Bad Gateway") => {
        if (failureHandled) return;
        failureHandled = true;
        if (!responseStarted && !response.headersSent) {
          sendJSON(response, status, message);
        } else {
          response.destroy();
        }
      };
      upstreamRequest.once("timeout", () => {
        fail(504, "Gateway Timeout");
        upstreamRequest.destroy();
      });
      upstreamRequest.once("error", fail);
      request.once("aborted", () => upstreamRequest.destroy());
      const boundedRequest = new ByteLimit(limit, () => {
        failureHandled = true;
        upstreamRequest.destroy();
        if (!response.headersSent) sendJSON(response, 413, "Payload Too Large");
      });
      boundedRequest.on("error", () => undefined);
      request.pipe(boundedRequest).pipe(upstreamRequest);
    },
  );
  server.requestTimeout = 190000;
  server.headersTimeout = 10000;
  return server;
}

export const bridgePaths = {
  ready: READY_PATH,
  catalog: CATALOG_PATH,
  generation: GENERATION_PATH,
};
