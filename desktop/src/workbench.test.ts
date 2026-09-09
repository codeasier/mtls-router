import { describe, expect, it } from "vitest";

import {
  looksLikeImplicitEdit,
  registerLiveWorkbenchAssetUrl,
  revokeLiveWorkbenchAssetUrl,
  workbenchAssetUri,
} from "./workbench";

describe("workbench helpers", () => {
  it("builds image-asset URIs only for sha256 ids", () => {
    const id = "a".repeat(64);
    expect(workbenchAssetUri(id)).toBe(`image-asset://localhost/${id}`);
    expect(workbenchAssetUri("../secrets")).toBe("");
    expect(workbenchAssetUri("/tmp/abs")).toBe("");
  });

  it("prefers a registered live blob URL", () => {
    const id = "b".repeat(64);
    registerLiveWorkbenchAssetUrl(id, "blob:http://localhost/live");
    expect(workbenchAssetUri(id)).toBe("blob:http://localhost/live");
    revokeLiveWorkbenchAssetUrl(id);
    expect(workbenchAssetUri(id)).toBe(`image-asset://localhost/${id}`);
  });

  it("matches the implicit edit closed set", () => {
    expect(looksLikeImplicitEdit("改成蓝色", "zh-CN")).toBe(true);
    expect(looksLikeImplicitEdit("turn it into gold", "en")).toBe(true);
    expect(looksLikeImplicitEdit("随便聊聊", "zh-CN")).toBe(false);
  });
});
