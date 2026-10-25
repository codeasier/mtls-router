import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { ChangelogMarkdown } from "./changelogMarkdown";

describe("ChangelogMarkdown", () => {
  it("renders headings, lists, emphasis, and code", () => {
    render(
      <ChangelogMarkdown
        source={[
          "### 新增",
          "",
          "一段说明，含 `listen_failed` 与 **保留端口**。",
          "",
          "- 第一项",
          "- 第二项",
        ].join("\n")}
      />,
    );

    expect(screen.getByRole("heading", { name: "新增" }).tagName).toBe("H4");
    expect(screen.getByText("listen_failed").tagName).toBe("CODE");
    expect(screen.getByText("保留端口").tagName).toBe("STRONG");
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
    expect(screen.queryByText(/### 新增/)).not.toBeInTheDocument();
  });

  it("keeps raw html as text and only links https urls", () => {
    render(
      <ChangelogMarkdown
        source={
          "见 [发布页](https://github.com/codeasier/mtls-router/releases) 与 [本地](javascript:alert(1))\n\n<script>alert(1)</script>"
        }
      />,
    );

    const link = screen.getByRole("link", { name: "发布页" });
    expect(link).toHaveAttribute(
      "href",
      "https://github.com/codeasier/mtls-router/releases",
    );
    expect(document.querySelector("a[href^='javascript']")).toBeNull();
    expect(screen.getByText(/javascript:alert\(1\)/)).toBeVisible();
    expect(document.querySelector("script")).toBeNull();
    expect(screen.getByText("<script>alert(1)</script>")).toBeVisible();
  });
});
