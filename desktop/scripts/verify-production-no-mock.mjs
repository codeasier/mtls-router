import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import console from "node:console";
import { fileURLToPath } from "node:url";

const desktopDir = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const assetsDir = path.join(desktopDir, "dist", "assets");
const forbidden = [
  "mockDesktopApi",
  "__MTLS_BROWSER_MOCK__",
  "revision-mock",
  "mock-desktop",
];

for (const name of readdirSync(assetsDir)) {
  if (!name.endsWith(".js")) continue;
  const source = readFileSync(path.join(assetsDir, name), "utf8");
  const marker = forbidden.find(
    (candidate) => name.includes(candidate) || source.includes(candidate),
  );
  if (marker) {
    console.error(
      `production build contains browser mock marker ${marker} in ${name}`,
    );
    process.exit(1);
  }
}
