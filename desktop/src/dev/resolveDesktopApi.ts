export interface DesktopApiEnv {
  DEV: boolean;
  PROD: boolean;
  VITE_MOCK?: string;
}

/**
 * Mock DesktopApi is available only when both conditions hold:
 * - Vite development mode (import.meta.env.DEV)
 * - explicit VITE_MOCK=true
 *
 * Production builds always receive the real Tauri-backed API, even if
 * VITE_MOCK is somehow present in the environment.
 */
export function shouldUseMockDesktopApi(env: DesktopApiEnv): boolean {
  return env.DEV === true && env.PROD !== true && env.VITE_MOCK === "true";
}

export function desktopApiEnvFromImportMeta(
  meta: ImportMetaEnv = import.meta.env,
): DesktopApiEnv {
  return {
    DEV: meta.DEV,
    PROD: meta.PROD,
    VITE_MOCK: meta.VITE_MOCK,
  };
}
