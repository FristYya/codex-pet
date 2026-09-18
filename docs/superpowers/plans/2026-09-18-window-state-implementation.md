# Codex Pet 窗口状态实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在不改动 quota 协议的前提下，为 Windows Codex Pet 实现可靠的位置持久化、多显示器/DPI 恢复、托盘控制、锁定穿透与官方开机启动。

**Architecture:** Rust 新增 `settings`（版本化文件、原子写、纯几何逻辑）和 `window_state`（原生能力薄抽象、状态变更协调）两个模块。`lib.rs` 只负责 Tauri 装配、窗口事件和 Tray 事件；React 仅消费展开方向并调整 CSS。所有落盘位置均为相对目标显示器 work area 的逻辑坐标，且只保存 collapsedRect。

**Tech Stack:** Tauri 2、Rust 2024、serde/serde_json、Tauri 官方 `tauri-plugin-autostart`、React 19、Vitest。

## Global Constraints

- Windows 是本阶段唯一实机目标；不做 macOS、安装、更新、主题、云端或遥测。
- 不修改 `src-tauri/src/quota.rs` 或 `src-tauri/src/codex_adapter/`，除非测试证明有真实回归。
- 配置路径必须由 Tauri `app_config_dir` 取得，文件名固定为 `ui-settings.json`。
- 原子替换必须是同目录临时文件 → write → flush/`sync_all` → replace-existing；不得删除旧文件再 rename。
- `version > CURRENT_VERSION` 只读保护原文件、使用默认内存设置；启动 autostart 以 OS `is_enabled()` 为事实源，不主动 enable。
- V0.1 collapsed 尺寸为代码常量；配置尺寸只供校验和迁移，不得覆盖当前运行时合法尺寸。
- 所有实现任务遵循 Red → Green → Refactor；每批完成后更新 CHANGELOG/DEVLOG、完整验证、commit、push。

---

## 文件结构

- `src-tauri/src/settings.rs`：设置模型、迁移、原子读取/写入、debounce/退出 flush、显示器无关的 Rect/DPI 恢复逻辑及单元测试。
- `src-tauri/src/window_state.rs`：`NativeWindow`、`TrayState`、`Autostart` 薄 trait，操作成功后提交内存状态的协调器及单元测试。
- `src-tauri/src/lib.rs`：Tauri 实现 trait、初始隐藏窗口、Tray 菜单构建、启动恢复、窗口移动/关闭和 command 装配。
- `src-tauri/Cargo.toml`、`src-tauri/capabilities/default.json`：official autostart plugin 与最小权限。
- `src/App.tsx`：用 Rust command 请求展开/收起临时几何并接收方向。
- `src/pet/PetShell.tsx`、`src/App.css`：方向 class、锁定时禁用拖动区。
- `src/pet/PetShell.test.tsx`、`src/App.test.tsx`：无需 Tauri Runtime 的用户可见回归测试。
- `CHANGELOG.md`、`DEVLOG.md`：每批实现记录与验证状态。

## Task 1: 设置模型、原子写与收起位置（提交 1）

**Files:**
- Create: `src-tauri/src/settings.rs`
- Modify: `src-tauri/src/lib.rs`, `src-tauri/Cargo.toml`, `CHANGELOG.md`, `DEVLOG.md`
- Test: `src-tauri/src/settings.rs` 内 `#[cfg(test)]` 模块

**Interfaces:**
- Produces `UiSettings::default()`, `SettingsStore::load(path)`, `SettingsStore::save_atomic(path, settings)`, `SettingsStore::schedule(settings)`, `SettingsStore::flush()`。
- Produces `CollapsedRect { x, y, width, height }` 和 `CURRENT_COLLAPSED_SIZE`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn malformed_file_falls_back_without_panicking() { /* load invalid JSON; assert default */ }
#[test]
fn save_failure_keeps_previous_valid_file() { /* injected ReplaceFile failure; assert old JSON */ }
#[test]
fn exit_flush_persists_pending_last_move() { /* schedule A then B; flush; assert B */ }
#[test]
fn rapid_moves_persist_only_the_final_rect() { /* manual scheduler; assert final write */ }
```

- [ ] **Step 2: 验证 RED**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings::tests`

Expected: FAIL，因为 `settings` 模块和接口尚不存在。

- [ ] **Step 3: 最小实现**

```rust
pub const CURRENT_COLLAPSED_SIZE: LogicalSize = LogicalSize { width: 164.0, height: 154.0 };
pub struct UiSettings { pub version: u32, pub window: Option<SavedWindow>, pub locked: bool, pub always_on_top: bool, pub visible: bool, pub autostart: bool }
pub trait AtomicFileOps { fn write_sync(&self, temp: &Path, bytes: &[u8]) -> io::Result<()>; fn replace_existing(&self, temp: &Path, target: &Path) -> io::Result<()>; }
```

实现 Windows `replace_existing`，在 replace 失败时不删除 target；注入 `AtomicFileOps` 使失败路径可测。实现内存 pending settings 与退出同步 flush。

- [ ] **Step 4: 验证 GREEN**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings::tests`

Expected: PASS。

- [ ] **Step 5: 集成并提交**

从 `lib.rs` 注册 settings state，窗口移动 debounce 后保存收起位置，退出前 flush；更新 CHANGELOG/DEVLOG。

Run: `cargo test --manifest-path src-tauri/Cargo.toml && pnpm test --run && cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings && pnpm build`

Commit: `feat: 保存并恢复桌宠位置`

## Task 2: 显示器回退、work-area 相对坐标与 DPI（提交 2、3）

**Files:**
- Modify: `src-tauri/src/settings.rs`, `src-tauri/src/lib.rs`, `CHANGELOG.md`, `DEVLOG.md`

**Interfaces:**
- Produces `MonitorSnapshot`, `save_relative_position`, `restore_position`, `clamp_rect_to_work_area`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn restores_saved_monitor_when_name_matches() { /* DISPLAY2 retained */ }
#[test]
fn missing_monitor_falls_back_to_primary_and_clamps() { /* no off-screen rect */ }
#[test]
fn converts_using_each_monitor_current_scale_factor() { /* 1.0 -> 1.25 -> 1.5 */ }
#[test]
fn stale_or_invalid_saved_size_uses_current_collapsed_size() { /* 164x154 */ }
```

- [ ] **Step 2: 验证 RED**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings::tests::restores_saved_monitor_when_name_matches`

Expected: FAIL，因为显示器选择和转换尚不存在。

- [ ] **Step 3: 最小实现与验证**

```rust
pub fn restore_position(saved: &SavedWindow, monitors: &[MonitorSnapshot], primary: Option<&MonitorSnapshot>) -> PhysicalRect;
pub fn logical_relative_to_physical(saved: LogicalPoint, work_area: PhysicalRect, scale: f64) -> PhysicalPoint;
```

按 key、name、primary、first 顺序选择 monitor；用目标当前 DPI 转换，并对完整 collapsedRect clamp。

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings::tests`

- [ ] **Step 4: 提交多显示器恢复**

更新日志并运行全量验证。

Commit: `feat: 支持多显示器位置恢复`

- [ ] **Step 5: 提交 DPI 修复**

补齐 100/125/150 与异 DPI 双屏测试、运行全量验证、更新日志。

Commit: `fix: 修复不同 DPI 下桌宠位置恢复`

## Task 3: 固定锚点的展开临时几何与前端方向（提交 4 的前半）

**Files:**
- Modify: `src-tauri/src/settings.rs`, `src-tauri/src/lib.rs`, `src/App.tsx`, `src/pet/PetShell.tsx`, `src/App.css`, `src/pet/PetShell.test.tsx`, `src/App.test.tsx`

**Interfaces:**
- Produces `ExpansionDirection { horizontal, vertical }`、`expanded_rect(collapsed, work_area, expanded_size)`。
- Produces Tauri command `set_pet_expanded(expanded: bool) -> ExpansionDirection`。

- [ ] **Step 1: 写失败测试并验证 RED**

```rust
#[test]
fn chooses_largest_visible_candidate_without_moving_pet_anchor() { /* all four directions */ }
```

```tsx
it("按 Rust 返回的左右上下方向添加展开 class", async () => { /* assert opens-left opens-up */ });
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings::tests::chooses_largest_visible_candidate_without_moving_pet_anchor` and `pnpm test --run src/pet/PetShell.test.tsx`

Expected: FAIL。

- [ ] **Step 2: 最小实现与 GREEN**

在 Rust 评估四个候选 Rect 可见面积，固定 pet anchor，expandedRect 不写 settings；React 只应用返回 class。

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings::tests && pnpm test --run`

## Task 4: 可测试 Tray/Window 状态、锁定与 CloseRequested（提交 4）

**Files:**
- Modify: `src-tauri/src/window_state.rs`, `src-tauri/src/lib.rs`, `src/pet/PetShell.tsx`, `src/App.css`, tests, `CHANGELOG.md`, `DEVLOG.md`

**Interfaces:**
- `trait NativeWindow { fn show(&self); fn hide(&self); fn set_always_on_top(&self, bool) -> Result<()>; fn set_ignore_cursor_events(&self, bool) -> Result<()>; }`
- `trait TrayState { fn set_checked(&self, id: &str, checked: bool) -> Result<()>; }`
- `WindowStateController::{set_locked,set_always_on_top,set_visible}`。

- [ ] **Step 1: RED tests**

```rust
#[test]
fn locked_state_changes_only_after_native_success() { /* failure retains unlocked and checkbox false */ }
#[test]
fn tray_failure_rolls_checkbox_back() { /* expected old checked state */ }
#[test]
fn close_requested_hides_and_marks_invisible() { /* prevent then hide then state */ }
#[test]
fn restored_lock_requires_ready_tray() { /* failed tray => unlocked */ }
```

- [ ] **Step 2: GREEN implementation and verification**

Tauri adapter wraps real window/menu calls; Tray menu has show/hide, lock, always-on-top, refresh, quit. Show does not focus. Locked uses full-window cursor ignore; unlock clears it before drag re-enables.

Run: `cargo test --manifest-path src-tauri/Cargo.toml && pnpm test --run && cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings && pnpm build`

- [ ] **Step 3: Commit and push**

Update CHANGELOG/DEVLOG.

Commit: `feat: 完善桌宠锁定与托盘状态`

## Task 5: 官方 Autostart（提交 5）

**Files:**
- Modify: `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `package.json`, `pnpm-lock.yaml`, `src-tauri/capabilities/default.json`, `src-tauri/src/window_state.rs`, `src-tauri/src/lib.rs`, `CHANGELOG.md`, `DEVLOG.md`

**Interfaces:**
- `trait Autostart { fn is_enabled(&self) -> Result<bool>; fn enable(&self) -> Result<()>; fn disable(&self) -> Result<()>; }`
- `WindowStateController::initialize_autostart()` and `set_autostart_from_tray(enabled)`。

- [ ] **Step 1: RED tests**

```rust
#[test]
fn startup_uses_os_is_enabled_without_enabling_from_saved_preference() { /* saved=true, OS=false */ }
#[test]
fn enable_failure_keeps_saved_setting_and_checkbox_unchanged() { /* failure */ }
#[test]
fn successful_user_toggle_updates_setting_after_os_call() { /* enable then settings */ }
```

- [ ] **Step 2: GREEN implementation and verify**

Add official plugin and capability permissions. On startup only query `is_enabled`, then update memory and Tray. Tray action calls enable/disable first, persists only after success.

Run: `cargo test --manifest-path src-tauri/Cargo.toml && pnpm test --run && cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings && pnpm build`

- [ ] **Step 3: Commit and push**

Update CHANGELOG/DEVLOG.

Commit: `feat: 支持开机自动启动`

## Task 6: Windows 真机验收

- [ ] 单屏拖动、退出、重启，记录实际位置恢复结果。
- [ ] 双屏拖至副屏、退出、重启，记录显示器恢复结果。
- [ ] 副屏位置保存后拔除，确认主屏 work area 内可见。
- [ ] 分别验证 100%、125%、150% DPI；有条件时异 DPI 双屏。
- [ ] 锁定后测试点击穿透，并从 Tray 解锁。
- [ ] 手动启用/关闭 autostart 后重启或重新登录验证；未拥有条件时在最终报告标为未实际验证。
