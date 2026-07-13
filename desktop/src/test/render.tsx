import { render, type RenderOptions } from "@testing-library/react";
import type { ReactElement } from "react";

import { I18nProvider } from "../i18n";

export function renderWithI18n(
  element: ReactElement,
  options?: Omit<RenderOptions, "wrapper">,
) {
  return render(element, { wrapper: I18nProvider, ...options });
}
