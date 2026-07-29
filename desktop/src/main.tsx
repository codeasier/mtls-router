import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { desktopApi, type DesktopApi } from "./ipc";
import "./styles.css";

async function resolveStartupApi(): Promise<DesktopApi> {
  if (
    !import.meta.env.DEV ||
    import.meta.env.PROD ||
    import.meta.env.VITE_MOCK !== "true"
  ) {
    return desktopApi;
  }
  // Dynamic import keeps mock fixtures out of production bundles.
  const { createMockDesktopApi } = await import("./dev/mockDesktopApi");
  return createMockDesktopApi();
}

void resolveStartupApi().then((api) => {
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App api={api} />
    </StrictMode>,
  );
});
