import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { QuotaSnapshot } from "../quota/types";
import { PetShell } from "./PetShell";

const snapshot: QuotaSnapshot = {
  availability: "allowed",
  planType: "plus",
  fetchedAt: 1_789_585_200,
  stale: false,
  windows: [
    {
      id: "primary",
      name: "5H",
      usedPercent: 64,
      remainingPercent: 36,
      windowDurationMins: 300,
      resetsAt: 1_789_588_800,
    },
    {
      id: "secondary",
      name: "Weekly",
      usedPercent: 18,
      remainingPercent: 82,
      windowDurationMins: 10_080,
      resetsAt: 1_790_190_000,
    },
  ],
};

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("PetShell", () => {
  it("默认只显示最紧张窗口，点击后展开全部窗口", async () => {
    const onExpandedChange = vi.fn();
    const user = userEvent.setup();
    render(<PetShell snapshot={snapshot} onExpandedChange={onExpandedChange} onRefresh={() => undefined} />);

    expect(screen.getByText("36%")).toBeInTheDocument();
    expect(screen.queryByText("Weekly")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "展开额度详情" }));

    expect(screen.getByText("Weekly")).toBeInTheDocument();
    expect(onExpandedChange).toHaveBeenLastCalledWith(true);
  });

  it("鼠标离开后延迟收起详情", async () => {
    const onExpandedChange = vi.fn();
    const { container } = render(
      <PetShell snapshot={snapshot} onExpandedChange={onExpandedChange} onRefresh={() => undefined} />,
    );

    fireEvent.click(screen.getByRole("button", { name: "展开额度详情" }));
    fireEvent.pointerOver(container.firstElementChild as Element);
    fireEvent.pointerOut(container.firstElementChild as Element, { relatedTarget: null });

    await waitFor(() => expect(screen.queryByText("Weekly")).not.toBeInTheDocument(), { timeout: 1_500 });
    expect(onExpandedChange).toHaveBeenLastCalledWith(false);
  });

  it("首次无数据时显示额度暂不可用而不是 0%", () => {
    render(
      <PetShell
        snapshot={{ ...snapshot, availability: "unavailable", windows: [] }}
        onExpandedChange={() => undefined}
        onRefresh={() => undefined}
      />,
    );

    expect(screen.getByText("额度暂不可用")).toBeInTheDocument();
    expect(screen.queryByText("0%")).not.toBeInTheDocument();
  });

  it("展开后可手动刷新额度", async () => {
    const onRefresh = vi.fn();
    const user = userEvent.setup();
    render(
      <PetShell
        snapshot={snapshot}
        onExpandedChange={() => undefined}
        onRefresh={onRefresh}
      />,
    );

    await user.click(screen.getByRole("button", { name: "展开额度详情" }));
    await user.click(screen.getByRole("button", { name: "刷新额度" }));

    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it("锁定时标记不可拖动", () => {
    render(
      <PetShell
        snapshot={snapshot}
        locked
        onExpandedChange={() => undefined}
        onRefresh={() => undefined}
      />,
    );

    const dragHandle = screen.getByLabelText("拖动桌宠");
    expect(dragHandle).toHaveAttribute("aria-disabled", "true");
  });

  it("从锁定解除后恢复拖动区可用提示", () => {
    const { rerender } = render(
      <PetShell
        snapshot={snapshot}
        locked
        onExpandedChange={() => undefined}
        onRefresh={() => undefined}
      />,
    );

    const shell = document.querySelector(".pet-shell");
    const dragHandle = screen.getByLabelText("拖动桌宠");
    expect(shell).toHaveClass("is-locked");
    expect(dragHandle).toHaveAttribute("aria-disabled", "true");

    rerender(
      <PetShell
        snapshot={snapshot}
        locked={false}
        onExpandedChange={() => undefined}
        onRefresh={() => undefined}
      />,
    );

    expect(shell).not.toHaveClass("is-locked");
    expect(dragHandle).not.toHaveAttribute("aria-disabled");
  });

  it("主体短按只切换详情且不请求原生拖动", () => {
    const onExpandedChange = vi.fn();
    const onStartDragging = vi.fn();
    render(
      <PetShell
        snapshot={snapshot}
        onExpandedChange={onExpandedChange}
        onStartDragging={onStartDragging}
        onRefresh={() => undefined}
      />,
    );

    const body = screen.getByRole("button", { name: "展开额度详情" });
    fireEvent.pointerDown(body, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
    fireEvent.pointerUp(body, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
    fireEvent.click(body);

    expect(onStartDragging).not.toHaveBeenCalled();
    expect(onExpandedChange).toHaveBeenCalledTimes(1);
    expect(onExpandedChange).toHaveBeenLastCalledWith(true);
  });

  it("主体长按 250ms 仅请求一次原生拖动且不切换详情", () => {
    vi.useFakeTimers();
    const onExpandedChange = vi.fn();
    const onStartDragging = vi.fn();
    render(<PetShell snapshot={snapshot} onExpandedChange={onExpandedChange} onStartDragging={onStartDragging} onRefresh={() => undefined} />);

    const body = screen.getByRole("button", { name: "展开额度详情" });
    fireEvent.pointerDown(body, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
    vi.advanceTimersByTime(250);
    fireEvent.pointerUp(body, { button: 0, pointerId: 1 });
    fireEvent.click(body);

    expect(onStartDragging).toHaveBeenCalledTimes(1);
    expect(onExpandedChange).not.toHaveBeenCalled();
  });

  it("主体移动超过 6px 时立即且仅请求一次原生拖动", () => {
    vi.useFakeTimers();
    const onStartDragging = vi.fn();
    render(<PetShell snapshot={snapshot} onExpandedChange={() => undefined} onStartDragging={onStartDragging} onRefresh={() => undefined} />);

    const body = screen.getByRole("button", { name: "展开额度详情" });
    fireEvent.pointerDown(body, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
    fireEvent.pointerMove(body, { pointerId: 1, clientX: 17, clientY: 10 });
    vi.advanceTimersByTime(250);

    expect(onStartDragging).toHaveBeenCalledTimes(1);
  });

  it("开始拖动后 release 和 click 均不切换详情", () => {
    vi.useFakeTimers();
    const onExpandedChange = vi.fn();
    render(<PetShell snapshot={snapshot} onExpandedChange={onExpandedChange} onStartDragging={() => undefined} onRefresh={() => undefined} />);

    const body = screen.getByRole("button", { name: "展开额度详情" });
    fireEvent.pointerDown(body, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
    vi.advanceTimersByTime(250);
    fireEvent.pointerUp(body, { button: 0, pointerId: 1 });
    fireEvent.click(body);

    expect(onExpandedChange).not.toHaveBeenCalled();
  });

  it("刷新按钮不参与拖动手势", async () => {
    const onStartDragging = vi.fn();
    const user = userEvent.setup();
    render(<PetShell snapshot={snapshot} onExpandedChange={() => undefined} onStartDragging={onStartDragging} onRefresh={() => undefined} />);

    await user.click(screen.getByRole("button", { name: "展开额度详情" }));
    const refresh = screen.getByRole("button", { name: "刷新额度" });
    fireEvent.pointerDown(refresh, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
    fireEvent.pointerMove(refresh, { pointerId: 1, clientX: 30, clientY: 10 });

    expect(onStartDragging).not.toHaveBeenCalled();
  });

  it("锁定时不处理主体和拖动柄的前端手势", () => {
    const onExpandedChange = vi.fn();
    const onStartDragging = vi.fn();
    render(<PetShell snapshot={snapshot} locked onExpandedChange={onExpandedChange} onStartDragging={onStartDragging} onRefresh={() => undefined} />);

    const body = screen.getByRole("button", { name: "展开额度详情" });
    const dragHandle = screen.getByLabelText("拖动桌宠");
    fireEvent.pointerDown(body, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
    fireEvent.pointerUp(body, { button: 0, pointerId: 1 });
    fireEvent.click(body);
    fireEvent.pointerDown(dragHandle, { button: 0, pointerId: 2, clientX: 10, clientY: 10 });

    expect(onStartDragging).not.toHaveBeenCalled();
    expect(onExpandedChange).not.toHaveBeenCalled();
  });

  it("取消或卸载会清理未完成手势的 timer", () => {
    vi.useFakeTimers();
    const onStartDragging = vi.fn();
    const view = render(<PetShell snapshot={snapshot} onExpandedChange={() => undefined} onStartDragging={onStartDragging} onRefresh={() => undefined} />);
    const body = screen.getByRole("button", { name: "展开额度详情" });
    fireEvent.pointerDown(body, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
    fireEvent.pointerCancel(body, { pointerId: 1 });
    vi.advanceTimersByTime(250);
    expect(onStartDragging).not.toHaveBeenCalled();

    fireEvent.pointerDown(body, { button: 0, pointerId: 2, clientX: 10, clientY: 10 });
    view.unmount();
    vi.advanceTimersByTime(250);
    expect(onStartDragging).not.toHaveBeenCalled();
  });

  it("按下后捕获指针，确保移出元素后仍能接收释放事件", () => {
    const capture = vi.fn();
    const original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "setPointerCapture");
    Object.defineProperty(HTMLElement.prototype, "setPointerCapture", { configurable: true, value: capture });
    try {
      render(<PetShell snapshot={snapshot} onExpandedChange={() => undefined} onRefresh={() => undefined} />);
      const body = screen.getByRole("button", { name: "展开额度详情" });
      fireEvent.pointerDown(body, { button: 0, pointerId: 7, clientX: 10, clientY: 10 });

      expect(capture).toHaveBeenCalledWith(7);
    } finally {
      if (original) Object.defineProperty(HTMLElement.prototype, "setPointerCapture", original);
      else Reflect.deleteProperty(HTMLElement.prototype, "setPointerCapture");
    }
  });

  it("手势进行中变为锁定会清理长按计时器", () => {
    vi.useFakeTimers();
    const onStartDragging = vi.fn();
    const view = render(<PetShell snapshot={snapshot} locked={false} onExpandedChange={() => undefined} onStartDragging={onStartDragging} onRefresh={() => undefined} />);
    const body = screen.getByRole("button", { name: "展开额度详情" });
    fireEvent.pointerDown(body, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });

    view.rerender(<PetShell snapshot={snapshot} locked onExpandedChange={() => undefined} onStartDragging={onStartDragging} onRefresh={() => undefined} />);
    vi.advanceTimersByTime(250);

    expect(onStartDragging).not.toHaveBeenCalled();
  });
});
