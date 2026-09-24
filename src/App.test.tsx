import { StrictMode } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { QuotaSnapshot } from "./quota/types";
// @ts-expect-error Node filesystem is used only by Vitest; this test is not shipped in the browser build.
import { readFileSync } from "node:fs";

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
    Promise.resolve(command === "read_window_locked" ? false : command === "read_account_status" ? "loggedIn" : snapshot));
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
  it("未登录时显示 ChatGPT 登录卡", async () => {
    tauri.invoke.mockImplementation((command) => Promise.resolve(
      command === "read_window_locked" ? false : command === "read_account_status" ? "loggedOut" : snapshot,
    ));

    render(<App />);

    expect(await screen.findByText("尚未连接 ChatGPT")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "登录 ChatGPT" })).toBeInTheDocument();
  });

  it("首次登录入口位于收起小窗可见区域内", async () => {
    tauri.invoke.mockImplementation((command) => Promise.resolve(
      command === "read_window_locked" ? false : command === "read_account_status" ? "loggedOut" : snapshot,
    ));

    render(<App />);

    await screen.findByRole("region", { name: "ChatGPT 登录状态" });
    const styles = readFileSync("src/App.css", "utf8");
    const accountCard = styles.match(/\.account-card\s*\{([^}]+)\}/)?.[1];
    const top = Number.parseFloat(accountCard?.match(/\btop:\s*(-?\d+(?:\.\d+)?)px/)?.[1] ?? "NaN");
    expect(Number.isFinite(top)).toBe(true);
    expect(top).toBeLessThanOrEqual(20);
  });

  it("运行时不可用时提供连接重试，不提示重新登录", async () => {
    let accountReads = 0;
    tauri.invoke.mockImplementation((command) => {
      if (command === "read_window_locked") return Promise.resolve(false);
      if (command === "read_account_status") {
        accountReads += 1;
        return Promise.resolve(accountReads === 1 ? "unavailable" : "loggedIn");
      }
      return Promise.resolve(snapshot);
    });

    render(<App />);

    expect(await screen.findByText("无法连接 Codex Runtime")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "重试连接" }));

    expect(await screen.findByText("36%")).toBeInTheDocument();
    expect(tauri.invoke).toHaveBeenCalledWith("read_account_status");
    expect(screen.queryByRole("button", { name: "重新登录" })).not.toBeInTheDocument();
  });

  it("登录中只等待后端账号事件，不轮询 Rust 状态", async () => {
    vi.useFakeTimers();
    const handlers = new Map<string, (event: { payload: unknown }) => void>();
    tauri.listen.mockImplementation(async (event, handler) => {
      handlers.set(event, handler);
      return vi.fn();
    });
    tauri.invoke.mockImplementation((command) => Promise.resolve(
      command === "read_window_locked" ? false : command === "read_account_status" ? "loggingIn" : snapshot,
    ));

    render(<App />);
    await act(async () => undefined);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });

    expect(tauri.invoke.mock.calls.filter(([command]) => command === "read_account_status")).toHaveLength(1);
    act(() => handlers.get("account://updated")?.({ payload: "loggedIn" }));
    await act(async () => undefined);
    expect(screen.getByText("36%")).toBeInTheDocument();
  });

  it("完成事件先于登录命令返回时仍保留已登录状态", async () => {
    const handlers = new Map<string, (event: { payload: unknown }) => void>();
    const login = deferred<string>();
    tauri.listen.mockImplementation(async (event, handler) => {
      handlers.set(event, handler);
      return vi.fn();
    });
    tauri.invoke.mockImplementation((command) => {
      if (command === "read_window_locked") return Promise.resolve(false);
      if (command === "read_account_status") return Promise.resolve("loggedOut");
      if (command === "start_chatgpt_login") return login.promise;
      return Promise.resolve(snapshot);
    });
    render(<App />);
    expect(await screen.findByRole("button", { name: "登录 ChatGPT" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "登录 ChatGPT" }));
    act(() => handlers.get("account://updated")?.({ payload: "loggedIn" }));
    login.resolve("loggingIn");
    await act(async () => undefined);
    expect(screen.queryByText("正在等待浏览器登录…")).not.toBeInTheDocument();
  });

  it("直接使用 quota://updated 的完整 payload 更新界面，不再次 invoke", async () => {
    const handlers = new Map<string, (event: { payload: unknown }) => void>();
    tauri.listen.mockImplementation(async (event, handler) => {
      handlers.set(event, handler);
      return vi.fn();
    });

    render(<App />);
    expect(await screen.findByText("36%")).toBeInTheDocument();
    expect(tauri.invoke).toHaveBeenCalledTimes(3);

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

    expect(unlisten).toHaveBeenCalledTimes(3);
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

    await waitFor(() => expect(unlisten).toHaveBeenCalledTimes(6));
  });

  it("保留每 60 秒一次的 read_quota 轮询", async () => {
    vi.useFakeTimers();
    render(<App />);
    await act(async () => undefined);
    expect(tauri.invoke).toHaveBeenCalledTimes(3);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });

    expect(tauri.invoke).toHaveBeenCalledTimes(4);
    const quotaCalls = tauri.invoke.mock.calls.filter(([command]) => command === "read_quota");
    expect(quotaCalls[quotaCalls.length - 1]).toEqual([
      "read_quota",
      { source: "automatic" },
    ]);
  });

  it("点击展开卡片的刷新额度按钮会调用 read_quota", async () => {
    const user = userEvent.setup();
    render(<App />);
    expect(await screen.findByText("36%")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "展开额度详情" }));
    await user.click(screen.getByRole("button", { name: "刷新额度" }));

    expect(tauri.invoke).toHaveBeenCalledTimes(4);
    expect(tauri.invoke).toHaveBeenLastCalledWith("read_quota", { source: "manual" });
  });

  it("手动刷新立即显示 loading，成功后恢复按钮并短暂提示成功", async () => {
    const pending = deferred<QuotaSnapshot>();
    let quotaReads = 0;
    tauri.invoke.mockImplementation((command) => {
      if (command === "read_window_locked") return Promise.resolve(false);
      if (command === "read_account_status") return Promise.resolve("loggedIn");
      quotaReads += 1;
      return quotaReads === 1 ? Promise.resolve(snapshot) : pending.promise;
    });
    const user = userEvent.setup();
    render(<App />);
    expect(await screen.findByText("36%")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "展开额度详情" }));

    await user.click(screen.getByRole("button", { name: "刷新额度" }));

    const loadingButton = screen.getByRole("button", { name: "刷新中…" });
    expect(loadingButton).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent("刷新中…");

    await act(async () => pending.resolve(updatedSnapshot));

    expect(await screen.findByRole("button", { name: "刷新额度" })).toBeEnabled();
    expect(await screen.findByText("额度已更新")).toBeInTheDocument();
    expect(screen.getByText("59%")).toBeInTheDocument();
  });

  it("成功反馈在短暂显示后自动消失", async () => {
    vi.useFakeTimers();
    const pending = deferred<QuotaSnapshot>();
    let quotaReads = 0;
    tauri.invoke.mockImplementation((command) => {
      if (command === "read_window_locked") return Promise.resolve(false);
      if (command === "read_account_status") return Promise.resolve("loggedIn");
      quotaReads += 1;
      return quotaReads === 1 ? Promise.resolve(snapshot) : pending.promise;
    });
    render(<App />);
    await act(async () => undefined);
    expect(screen.getByText("36%")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "展开额度详情" }));
    fireEvent.click(screen.getByRole("button", { name: "刷新额度" }));
    await act(async () => pending.resolve(updatedSnapshot));

    expect(screen.getByText("额度已更新")).toBeInTheDocument();
    await act(async () => vi.advanceTimersByTimeAsync(2_500));
    expect(screen.queryByText("额度已更新")).not.toBeInTheDocument();
  });

  it("刷新进行中重复点击不会再触发 read_quota", async () => {
    const pending = deferred<QuotaSnapshot>();
    let quotaReads = 0;
    tauri.invoke.mockImplementation((command) => {
      if (command === "read_window_locked") return Promise.resolve(false);
      if (command === "read_account_status") return Promise.resolve("loggedIn");
      quotaReads += 1;
      return quotaReads === 1 ? Promise.resolve(snapshot) : pending.promise;
    });
    const user = userEvent.setup();
    render(<App />);
    expect(await screen.findByText("36%")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "展开额度详情" }));
    const refreshButton = screen.getByRole("button", { name: "刷新额度" });

    await user.click(refreshButton);
    fireEvent.click(refreshButton);

    expect(tauri.invoke.mock.calls.filter(([command]) => command === "read_quota")).toHaveLength(2);
    await act(async () => pending.resolve(snapshot));
  });

  it("手动刷新复用进行中的自动刷新 Promise", async () => {
    const pending = deferred<QuotaSnapshot>();
    tauri.invoke.mockImplementation((command) => {
      if (command === "read_window_locked") return Promise.resolve(false);
      if (command === "read_account_status") return Promise.resolve("loggedIn");
      return pending.promise;
    });
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => {
      expect(tauri.invoke.mock.calls.filter(([command]) => command === "read_quota")).toHaveLength(1);
    });
    await user.click(screen.getByRole("button", { name: "展开额度详情" }));
    await user.click(screen.getByRole("button", { name: "刷新额度" }));

    expect(screen.getByRole("button", { name: "刷新中…" })).toBeDisabled();
    expect(tauri.invoke.mock.calls.filter(([command]) => command === "read_quota")).toHaveLength(1);
    await act(async () => pending.resolve(snapshot));
    expect(await screen.findByText("额度已更新")).toBeInTheDocument();
  });

  it("刷新失败时保留最后成功百分比并显示更新失败", async () => {
    let quotaReads = 0;
    tauri.invoke.mockImplementation((command) => {
      if (command === "read_window_locked") return Promise.resolve(false);
      if (command === "read_account_status") return Promise.resolve("loggedIn");
      quotaReads += 1;
      return quotaReads === 1 ? Promise.resolve(snapshot) : Promise.reject(new Error("offline"));
    });
    render(<App />);
    expect(await screen.findByText("36%")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "展开额度详情" }));
    fireEvent.click(screen.getByRole("button", { name: "刷新额度" }));

    expect(await screen.findByText("更新失败")).toBeInTheDocument();
    expect(await screen.findByText("刷新失败")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "刷新额度" })).toBeEnabled();
    expect(screen.getAllByText("36%").length).toBeGreaterThan(0);
  });

  it("刷新进行中卸载会清理轮询计时器且不会遗留加载 UI", async () => {
    vi.useFakeTimers();
    const pending = deferred<QuotaSnapshot>();
    let quotaReads = 0;
    tauri.invoke.mockImplementation((command) => {
      if (command === "read_window_locked") return Promise.resolve(false);
      if (command === "read_account_status") return Promise.resolve("loggedIn");
      quotaReads += 1;
      return quotaReads === 1 ? Promise.resolve(snapshot) : pending.promise;
    });
    const view = render(<App />);
    await act(async () => undefined);
    fireEvent.click(screen.getByRole("button", { name: "展开额度详情" }));
    fireEvent.click(screen.getByRole("button", { name: "刷新额度" }));
    expect(screen.getByRole("button", { name: "刷新中…" })).toBeDisabled();

    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
    await act(async () => pending.resolve(updatedSnapshot));
    expect(vi.getTimerCount()).toBe(0);
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
