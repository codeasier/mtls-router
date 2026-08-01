import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import process from "node:process";

const [path, extra] = process.argv.slice(2);
if (!path || extra) {
  throw new Error(
    "usage: updater-public-key-fingerprint.mjs <public-key-path>",
  );
}

const publicKey = readFileSync(path, "utf8").trim();
if (!publicKey) throw new Error("public key is empty");
process.stdout.write(
  `${createHash("sha256").update(publicKey).digest("hex")}\n`,
);
