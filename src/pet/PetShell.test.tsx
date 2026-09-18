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
});
