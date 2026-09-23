# 桌宠统一拖动手势 Implementation Plan

> **For agentic workers:** This plan was implemented and verified in the current feature worktree.

**Goal:** 在主体和顶部拖动柄上使用同一 Pointer Events 手势状态机，实现短按展开、长按或位移拖动。

**Architecture:** `PetShell` 保持该手势的一次性状态（按下坐标、250ms timer、是否已发起拖动），只在拖动触发前把主体 pointer-up 转为原有详情切换。所有窗口拖动仍由上层注入的 `onStartDragging` 连接到 Tauri `startDragging()`；刷新等原生交互控件不进入状态机。

**Tech Stack:** React 19、TypeScript、Vitest、Testing Library、Tauri 2。

## Global Constraints

- 主鼠标按下 250ms 后只调用一次 `startDragging()`。
- 相对按下点移动距离严格大于 6px 时立即且只调用一次 `startDragging()`。
- 不使用 `data-tauri-drag-region`；实际移动只走 `getCurrentWindow().startDragging()`。
- `pointerup`、`pointercancel`、组件卸载与锁定状态变更都必须清除 timer 和本次手势状态。
- locked 时不处理前端点击或拖动；不修改 quota 模块。

---

### Task 1: 为统一手势添加 RED 回归测试

**Files:**
- Modify: `src/pet/PetShell.test.tsx`
- Test: `src/pet/PetShell.test.tsx`

**Interfaces:**
- Consumes: `PetShellProps.onStartDragging?: () => void` 与主体按钮 `aria-label`。
- Produces: 对短按、250ms 长按、超过 6px 位移、拖动后释放、刷新按钮、锁定和取消/卸载清理的行为约束。

- [x] **Step 1: Write the failing tests**

```tsx
fireEvent.pointerDown(shell, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
fireEvent.pointerUp(shell, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
expect(onStartDragging).not.toHaveBeenCalled();
expect(onExpandedChange).toHaveBeenCalledWith(true);
```

```tsx
vi.useFakeTimers();
fireEvent.pointerDown(shell, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
act(() => vi.advanceTimersByTime(250));
fireEvent.pointerUp(shell, { button: 0, pointerId: 1 });
expect(onStartDragging).toHaveBeenCalledTimes(1);
expect(onExpandedChange).not.toHaveBeenCalled();
```

- [x] **Step 2: Run the focused test file to verify RED**

Run: `pnpm test --run src/pet/PetShell.test.tsx`

Expected: FAIL because the existing drag handle triggers immediately and the body has no shared pointer state.

### Task 2: 实现并验证最小统一状态机

**Files:**
- Modify: `src/pet/PetShell.tsx`
- Test: `src/pet/PetShell.test.tsx`

**Interfaces:**
- Consumes: `onStartDragging`, `locked`, `onExpandedChange`。
- Produces: 同一组 `onPointerDown`、`onPointerMove`、`onPointerUp`、`onPointerCancel` handlers，应用于主体与拖动柄。

- [x] **Step 1: Write minimal implementation**

```tsx
const gesture = useRef<{ pointerId: number; x: number; y: number; dragged: boolean } | null>(null);
const beginGesture = (event: React.PointerEvent<HTMLElement>) => {
  if (locked || event.button !== 0) return;
  gesture.current = { pointerId: event.pointerId, x: event.clientX, y: event.clientY, dragged: false };
  dragTimer.current = setTimeout(triggerDrag, 250);
};
```

`triggerDrag` 必须先将 `dragged` 置为 true、清除 timer，再调用一次 `onStartDragging`。位移处理使用 `Math.hypot(dx, dy) > 6`；释放仅在主体目标、尚未拖动且未锁定时切换详情。取消、卸载和 `locked` 改变调用同一个 `clearGesture`。

- [x] **Step 2: Run focused tests to verify GREEN**

Run: `pnpm test --run src/pet/PetShell.test.tsx`

Expected: PASS，所有 PetShell 测试通过。

- [x] **Step 3: Run affected frontend suite**

Run: `pnpm test --run src/App.test.tsx src/pet/PetShell.test.tsx`

Expected: PASS，App 的 locked 状态桥接与 PetShell 的手势回归均通过。
