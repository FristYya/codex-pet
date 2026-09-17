# Codex 额度事件即时刷新 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 App Server 的 `account/rateLimits/updated` 通知接成一次防抖后的完整额度读取，并让事件、60 秒轮询和手动刷新共享无并发读取的协调器。

**Architecture:** Rust 新增额度刷新协调器，统一持有 Codex 客户端、最后成功快照和 single-flight 状态。App Server 通知只进入 800ms 防抖器；防抖到期后调用协调器执行 `account/rateLimits/read`，再通过 `quota://updated` 发完整 `QuotaSnapshot`。React 保留启动读取和 60 秒轮询，同时监听同一事件并在卸载时释放 listener。

**Tech Stack:** Rust 2024、标准库线程/同步原语、Tauri 2 Event API、React 19、TypeScript、Vitest、Testing Library。

## Global Constraints

- `account/rateLimits/updated` 只能作为刷新信号，不能把通知参数直接推给前端。
- 事件、60 秒轮询和手动刷新必须共享同一个 single-flight 协调器，任意时刻最多一个 `account/rateLimits/read`。
- 通知防抖固定为 800ms；连续通知合并为一次刷新。
- 刷新失败必须保留最后一次成功快照并标记 `stale`，不能清空已有窗口。
- 前端事件名固定为 `quota://updated`，载荷固定为完整 `QuotaSnapshot`。
- 60 秒轮询必须保留。
- 日志和联调记录不得包含 Prompt、邮箱、Account ID、Token 或 Session 内容。

---

### Task 1: Rust 刷新协调器与通知防抖

**Files:**
- Create: `src-tauri/src/quota.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/quota.rs`

**Interfaces:**
- Consumes: `codex_adapter::CodexClient::read_rate_limits()` 与 `take_notification_receiver()`。
- Produces: `QuotaRefreshCoordinator::refresh() -> QuotaSnapshot`、`spawn_notification_bridge(...)`、`QUOTA_UPDATED_EVENT`。

- [ ] **Step 1: 写 single-flight、失败保留、防抖和事件发送的失败测试**

  在 `quota.rs` 使用可注入的读取闭包和事件发送闭包，覆盖：并发调用只有一个读取；刷新中到达的请求复用/合并；三个短间隔通知只触发一次读取；成功后发送完整快照；失败后发送带 `stale=true` 的最后快照。

- [ ] **Step 2: 运行 Rust 定向测试并确认按预期失败**

  Run: `cargo test quota::tests --manifest-path src-tauri/Cargo.toml`
  Expected: FAIL，原因是协调器、防抖器或事件桥尚未实现。

- [ ] **Step 3: 实现最小协调器**

  用 `Mutex + Condvar`（或等价状态机）保护 `refreshing`、`pending_event` 和最后快照；读取逻辑在互斥状态锁外执行。等待中的轮询/手动请求复用当前读取结果，事件在读取期间到达时最多追加一次后续读取。

- [ ] **Step 4: 实现 800ms 通知防抖与 Tauri emit 适配**

  只筛选 `account/rateLimits/updated`；安静窗口到期后调用完整读取，随后 emit `quota://updated`。通知通道断开时线程自然退出，不影响轮询。

- [ ] **Step 5: 运行 Rust 定向测试并确认通过**

  Run: `cargo test quota::tests --manifest-path src-tauri/Cargo.toml`
  Expected: PASS，且计数断言证明没有并发读取和通知风暴。

### Task 2: 应用层统一三个刷新入口

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/quota.rs`

**Interfaces:**
- Consumes: Task 1 的 `QuotaRefreshCoordinator`。
- Produces: Tauri command `read_quota`，托盘 command/event 也调用同一协调器。

- [ ] **Step 1: 写应用装配行为测试**

  测试首次连接后只接管一次通知 receiver；手动/轮询入口与事件入口共享读取计数；事件读取失败不覆盖最后成功数据。

- [ ] **Step 2: 运行定向测试确认失败**

  Run: `cargo test quota::tests --manifest-path src-tauri/Cargo.toml`
  Expected: FAIL，原因是 `AppState` 仍直接持有 client/last。

- [ ] **Step 3: 将 `read_quota` 攺为调用协调器**

  `AppState` 只持有 `Arc<QuotaRefreshCoordinator>`；setup 启动通知桥，command 继续返回 `QuotaSnapshot`，不改变前端现有调用协议。

- [ ] **Step 4: 运行全部 Rust 测试**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml`
  Expected: 全部 PASS。

### Task 3: React 事件监听与释放

**Files:**
- Create: `src/App.test.tsx`
- Modify: `src/App.tsx`
- Modify: `src/pet/PetShell.tsx`
- Modify: `src/pet/PetShell.test.tsx`
- Modify: `src/App.css`
- Modify: `src/test/setup.ts`（仅在需要共享 mock 清理时）

**Interfaces:**
- Consumes: Tauri `listen<QuotaSnapshot>("quota://updated", handler)`。
- Produces: 事件到达时更新 `snapshot`，组件卸载时调用异步返回的 unlisten，展开卡片中的“刷新额度”按钮复用同一个 `refresh`。

- [ ] **Step 1: 写监听、更新、释放和失败保留测试**

  Mock `@tauri-apps/api/core` 与 `@tauri-apps/api/event`；断言 mount 后注册一次，事件 payload 更新 UI，unmount 调用 unlisten；invoke reject 时已有窗口仍显示且状态变为 stale。

- [ ] **Step 2: 运行前端定向测试确认失败**

  Run: `pnpm test --run src/App.test.tsx`
  Expected: FAIL，原因是尚未调用 `listen`。

- [ ] **Step 3: 实现事件 listener 生命周期**

  在独立 `useEffect` 中注册 `quota://updated`；用 disposed 标志处理“组件已卸载但 listen Promise 后返回”的竞态；cleanup 中可靠调用 unlisten。

- [ ] **Step 4: 保留启动读取与 60 秒轮询并补充测试**

  使用 fake timers 断言 60 秒仍调用 `read_quota`；事件与轮询共存但前端不解析稀疏通知。给 `PetShell` 增加 `onRefresh`，点击“刷新额度”时调用同一个 `refresh`，并在 `PetShell.test.tsx` 验证按钮行为。

- [ ] **Step 5: 运行全部前端测试**

  Run: `pnpm test --run`
  Expected: 全部 PASS。

### Task 4: 文档、全量验证、真实联调与提交

**Files:**
- Modify: `CHANGELOG.md`
- Modify: `DEVLOG.md`
- Modify: `README.md`

- [ ] **Step 1: 更新面向用户和开发者的说明**

  README 说明“事件即时更新 + 60 秒对账 + 手动刷新共用协调器”；DEVLOG 只记录通知是否收到、完整读取是否成功、UI 是否变化，不记录敏感内容。

- [ ] **Step 2: 运行完整验证**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml`
  Run: `pnpm test --run`
  Run: `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`
  Run: `pnpm build`
  Expected: 四条命令全部 exit 0。

- [ ] **Step 3: 启动 Codex Pet 做真实事件联调**

  Run: `pnpm tauri dev`
  Expected: 实际 Codex 使用后收到 `account/rateLimits/updated`，800ms 防抖后完整读取成功，UI 无需等待 60 秒更新；若当前额度没有产生可见百分比变化，记录“事件和读取已发生、显示值因服务端取整未变化”，不得伪造变化。

- [ ] **Step 4: 提交并推送**

  Run: `git add src-tauri/src/quota.rs src-tauri/src/lib.rs src/App.tsx src/App.test.tsx src/pet/PetShell.tsx src/pet/PetShell.test.tsx src/App.css src/test/setup.ts CHANGELOG.md DEVLOG.md README.md docs/superpowers/plans/2026-09-17-event-driven-quota-refresh.md`
  Run: `git commit -m "feat: 支持 Codex 额度事件即时刷新"`
  Run: `git push origin main`
  Expected: push 成功且 `git status --short` 为空。
