import { StrictMode } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { QuotaSnapshot } from "./quota/types";

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  setSize: vi.fn(),
  startDragging: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: tauri.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: tauri.listen }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setSize: tauri.setSize, startDragging: tauri.startDragging }),
}));

import App from "./App";

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
  ],
};

const updatedSnapshot: QuotaSnapshot = {
  ...snapshot,
  fetchedAt: snapshot.fetchedAt + 1,
  windows: [{ ...snapshot.windows[0], usedPercent: 41, remainingPercent: 59 }],
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
  tauri.invoke.mockReset().mockImplementation((command) =>
    Promise.resolve(command === "read_window_locked" ? false : snapshot));
  tauri.listen.mockReset().mockResolvedValue(vi.fn());
  tauri.setSize.mockReset().mockResolvedValue(undefined);
  tauri.startDragging.mockReset().mockResolvedValue(undefined);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
});

describe("App 额度刷新", () => {
  it("直接使用 quota://updated 的完整 payload 更新界面，不再次 invoke", async () => {
    const handlers = new Map<string, (event: { payload: unknown }) => void>();
    tauri.listen.mockImplementation(async (event, handler) => {
      handlers.set(event, handler);
      return vi.fn();
    });

    render(<App />);
    expect(await screen.findByText("36%")).toBeInTheDocument();
    expect(tauri.invoke).toHaveBeenCalledTimes(2);

    act(() => handlers.get("quota://updated")?.({ payload: updatedSnapshot }));

    expect(screen.getByText("59%")).toBeInTheDocument();
    expect(tauri.invoke.mock.calls.filter(([command]) => command === "read_quota")).toHaveLength(1);
  });

  it("卸载时释放已经注册的 listener", async () => {
    const unlisten = vi.fn();
    tauri.listen.mockResolvedValue(unlisten);
    const view = render(<App />);

    await waitFor(() => expect(tauri.listen).toHaveBeenCalledWith("quota://updated", expect.any(Function)));
    await act(async () => undefined);
    view.unmount();

    expect(unlisten).toHaveBeenCalledTimes(2);
  });

  it("listener 异步注册完成前卸载，注册完成后仍释放", async () => {
    const registration = deferred<() => void>();
    const unlisten = vi.fn();
    tauri.listen.mockReturnValue(registration.promise);
    const view = render(
      <StrictMode>
        <App />
      </StrictMode>,
    );

    view.unmount();
    registration.resolve(unlisten);

    await waitFor(() => expect(unlisten).toHaveBeenCalledTimes(4));
  });

  it("保留每 60 秒一次的 read_quota 轮询", async () => {
    vi.useFakeTimers();
    render(<App />);
    await act(async () => undefined);
    expect(tauri.invoke).toHaveBeenCalledTimes(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });

    expect(tauri.invoke).toHaveBeenCalledTimes(3);
  });

  it("点击展开卡片的刷新额度按钮会调用 read_quota", async () => {
    const user = userEvent.setup();
    render(<App />);
    expect(await screen.findByText("36%")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "展开额度详情" }));
    await user.click(screen.getByRole("button", { name: "刷新额度" }));

    expect(tauri.invoke).toHaveBeenCalledTimes(3);
    expect(tauri.invoke).toHaveBeenLastCalledWith("read_quota");
  });

  it("刷新失败时保留最后成功百分比并显示更新失败", async () => {
    let quotaReads = 0;
    tauri.invoke.mockImplementation((command) => {
      if (command === "read_window_locked") return Promise.resolve(false);
      quotaReads += 1;
      return quotaReads === 1 ? Promise.resolve(snapshot) : Promise.reject(new Error("offline"));
    });
    render(<App />);
    expect(await screen.findByText("36%")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "展开额度详情" }));
    fireEvent.click(screen.getByRole("button", { name: "刷新额度" }));

    expect(await screen.findByText("更新失败")).toBeInTheDocument();
    expect(screen.getAllByText("36%").length).toBeGreaterThan(0);
  });

  it("收到锁定状态后标记为不可拖动", async () => {
    const handlers = new Map<string, (event: { payload: unknown }) => void>();
    tauri.listen.mockImplementation(async (event, handler) => {
      handlers.set(event, handler);
      return vi.fn();
    });

    render(<App />);
    const dragHandle = screen.getByLabelText("拖动桌宠");
    act(() => handlers.get("window://locked")?.({ payload: true }));

    expect(dragHandle).toHaveAttribute("aria-disabled", "true");
  });

  it("Tray 解锁事件到达后恢复拖动区可用提示", async () => {
    const handlers = new Map<string, (event: { payload: unknown }) => void>();
    tauri.listen.mockImplementation(async (event, handler) => {
      handlers.set(event, handler);
      return vi.fn();
    });

    render(<App />);
    const dragHandle = await screen.findByLabelText("拖动桌宠");
    act(() => handlers.get("window://locked")?.({ payload: true }));
    expect(dragHandle).toHaveAttribute("aria-disabled", "true");

    act(() => handlers.get("window://locked")?.({ payload: false }));

    expect(dragHandle).not.toHaveAttribute("aria-disabled");
  });

  it("Tray 解锁后拖动柄长按会请求原生窗口拖动", async () => {
    vi.useFakeTimers();
    const handlers = new Map<string, (event: { payload: unknown }) => void>();
    tauri.listen.mockImplementation(async (event, handler) => {
      handlers.set(event, handler);
      return vi.fn();
    });

    render(<App />);
    const dragHandle = screen.getByLabelText("拖动桌宠");
    act(() => handlers.get("window://locked")?.({ payload: true }));
    act(() => handlers.get("window://locked")?.({ payload: false }));

    fireEvent.pointerDown(dragHandle, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
    act(() => vi.advanceTimersByTime(250));

    expect(tauri.startDragging).toHaveBeenCalledTimes(1);
  });
});
