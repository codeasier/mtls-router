import { fireEvent, render, screen } from "@testing-library/react";
import type { ComponentProps } from "react";
import { describe, expect, it, vi } from "vitest";

import { ConfirmDialog } from "./ConfirmDialog";

function renderDialog(
  props: Partial<ComponentProps<typeof ConfirmDialog>> = {},
) {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  render(
    <ConfirmDialog
      title="安装更新"
      description="将下载并安装 CodeasierRouter 0.3.8，安装完成后应用会重启。是否继续？"
      confirmLabel="继续安装"
      cancelLabel="取消"
      onConfirm={onConfirm}
      onCancel={onCancel}
      {...props}
    />,
  );
  return { onConfirm, onCancel };
}

describe("ConfirmDialog", () => {
  it("focuses cancel on open and resolves through the dialog buttons", () => {
    const { onConfirm, onCancel } = renderDialog();

    const dialog = screen.getByRole("dialog", { name: "安装更新" });
    expect(dialog).toHaveAccessibleDescription(
      "将下载并安装 CodeasierRouter 0.3.8，安装完成后应用会重启。是否继续？",
    );
    expect(screen.getByRole("button", { name: "取消" })).toHaveFocus();

    fireEvent.click(screen.getByRole("button", { name: "继续安装" }));
    expect(onConfirm).toHaveBeenCalledOnce();
    expect(onCancel).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("cancels on Escape and on backdrop dismissal but not on clicks inside", () => {
    const { onCancel } = renderDialog();
    const backdrop = document.querySelector(".dialog-backdrop");
    expect(backdrop).not.toBeNull();

    fireEvent.mouseDown(screen.getByRole("dialog"));
    expect(onCancel).not.toHaveBeenCalled();

    fireEvent.mouseDown(backdrop as Element);
    expect(onCancel).toHaveBeenCalledOnce();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(onCancel).toHaveBeenCalledTimes(2);
  });

  it("cycles focus between the action buttons with Tab", () => {
    renderDialog();
    const cancelButton = screen.getByRole("button", { name: "取消" });
    const confirmButton = screen.getByRole("button", { name: "继续安装" });

    expect(cancelButton).toHaveFocus();
    fireEvent.keyDown(screen.getByRole("dialog"), {
      key: "Tab",
      shiftKey: true,
    });
    expect(confirmButton).toHaveFocus();
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Tab" });
    expect(cancelButton).toHaveFocus();
  });

  it("keeps destructive confirms on the danger styling and regular confirms neutral", () => {
    const neutral = render(
      <ConfirmDialog
        title="安装更新"
        description="描述"
        confirmLabel="继续安装"
        cancelLabel="取消"
        onConfirm={() => undefined}
        onCancel={() => undefined}
      />,
    );
    const neutralDialog = neutral.container.querySelector(".danger-dialog");
    expect(neutralDialog).toHaveClass("confirm-dialog");
    expect(neutral.getByRole("button", { name: "继续安装" })).not.toHaveClass(
      "control-button--danger",
    );
    neutral.unmount();

    renderDialog({
      danger: true,
      title: "准备卸载",
      confirmLabel: "继续卸载",
    });
    const dangerDialog = document.querySelector(".danger-dialog");
    expect(dangerDialog).not.toHaveClass("confirm-dialog");
    expect(screen.getByRole("button", { name: "继续卸载" })).toHaveClass(
      "control-button--danger",
    );
  });
});
