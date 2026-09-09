export const WORKBENCH_SHA256 = /^[a-f0-9]{64}$/;
export const MAX_WORKBENCH_IMPORT_BYTES = 20 * 1024 * 1024;

const EDIT_HINT_ZH = /改成|改一下|换成|重画成|变成/;
const EDIT_HINT_EN = /change it to|make it|turn it into|replace with|redo as/i;

const liveAssetUrls = new Map<string, string>();

export function registerLiveWorkbenchAssetUrl(id: string, url: string): void {
  if (!WORKBENCH_SHA256.test(id) || !url) return;
  const previous = liveAssetUrls.get(id);
  if (previous && previous !== url && previous.startsWith("blob:")) {
    URL.revokeObjectURL(previous);
  }
  liveAssetUrls.set(id, url);
}

export function revokeLiveWorkbenchAssetUrl(id: string): void {
  const previous = liveAssetUrls.get(id);
  if (previous?.startsWith("blob:")) URL.revokeObjectURL(previous);
  liveAssetUrls.delete(id);
}

export function revokeAllLiveWorkbenchAssets(): void {
  for (const id of [...liveAssetUrls.keys()]) {
    revokeLiveWorkbenchAssetUrl(id);
  }
}

export function workbenchAssetUri(id: string): string {
  const live = liveAssetUrls.get(id);
  if (live) return live;
  if (!WORKBENCH_SHA256.test(id)) return "";
  if (
    typeof navigator !== "undefined" &&
    /windows/i.test(navigator.userAgent)
  ) {
    return `http://image-asset.localhost/${id}`;
  }
  return `image-asset://localhost/${id}`;
}

export function looksLikeImplicitEdit(text: string, language: string): boolean {
  return language === "en" ? EDIT_HINT_EN.test(text) : EDIT_HINT_ZH.test(text);
}

export function commandErrorCode(error: unknown): string {
  if (!error || typeof error !== "object" || !("code" in error)) return "";
  return typeof error.code === "string" ? error.code : "";
}
