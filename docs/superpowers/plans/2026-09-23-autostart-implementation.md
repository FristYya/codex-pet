# Codex Pet 开机自动启动 Implementation Plan

> **For agentic workers:** Implementation is being executed in the current worktree. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 通过 Tray 菜单使用官方 Tauri 插件管理 Codex Pet 的 Windows 登录启动状态。

**Architecture:** `UiSettings.autostart` 保存启动状态镜像；原生状态通过 `tauri-plugin-autostart` 的 Rust manager 查询和切换。`WindowStateController` 复用已有事务模式：系统和 Tray 操作成功后提交内存设置与持久化快照，失败时补偿回滚。

**Tech Stack:** Rust 2024、Tauri 2、`tauri-plugin-autostart` 2.x、Vitest、pnpm。

## Global Constraints

- 仅使用官方 `tauri-plugin-autostart`；不直接访问或修改 Windows Registry。
- 启动状态由原生插件查询，`UiSettings` 与 Tray 只镜像原生状态；即使 Tray 不可用，设置仍跟随查询到的原生状态。
- 用户切换时，原生启停操作和 Tray 同步成功后才提交内存状态并持久化；失败补偿恢复原状态。
- 旧设置缺少 `autostart` 时默认为关闭；不升级设置 schema 版本。
- 不修改稳定 quota 模块。

---

### Task 1: 添加状态迁移 RED 测试

**Files:**
- Modify: `src-tauri/src/window_state.rs`
- Modify: `src-tauri/src/settings.rs`

- [x] 测试 autostart 启用/关闭时先应用原生状态，再同步 Tray，最后更改设置。
- [x] 测试原生查询/设置或 Tray 同步失败时状态回滚，补偿失败标记 shutdown。
- [x] 测试启动时用插件实际状态同步设置和 Tray，旧设置缺字段时默认 false。
- [x] 运行 `cargo test` 确认新增行为测试因缺实现而失败。

### Task 2: 实现 Rust 状态控制器

**Files:**
- Modify: `src-tauri/src/window_state.rs`
- Modify: `src-tauri/src/settings.rs`

- [x] 新增 `AUTOSTART_MENU_ID`、原生 Autostart trait、`RuntimeWindowState::set_autostart` 和事务控制方法。
- [x] 先运行插件操作与菜单同步，二者成功后才修改 `UiSettings`。
- [x] 失败时回滚插件和菜单；启动同步只读取原生状态，不擅自启用/禁用系统启动项。
- [x] 运行 `cargo test` 确认新增测试通过。

### Task 3: 接入官方 Tauri 插件与 Tray

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `src-tauri/src/lib.rs`

- [x] 添加 `tauri-plugin-autostart = "2"` 并在 Tauri builder 注册官方插件。
- [x] 为插件 manager 实现原生状态 trait，并把“开机自动启动”勾选项加入 Tray。
- [x] 启动时读取真实注册状态、校正 Tray 与配置镜像；Tray 点击走统一事务控制器。
- [x] 运行 `cargo test`、Clippy 和 Tauri 开发版编译。

### Task 4: 回归验证与交付

**Files:**
- Modify: `CHANGELOG.md`
- Modify: `DEVLOG.md`
- Modify: 仓库同级的本地接力状态文件 `Codex额度监控器.codex-local/PROJECT_STATE.md`（不纳入 Git）

- [x] 运行完整前端测试、Rust 测试、Clippy、生产构建和 `git diff --check`。
- [x] Windows 人工验收开启、退出/重开后状态恢复、关闭及最终关闭状态；Release 正常，Debug 注册当前 Debug exe（Windows 启动 UI 呈灰色，已按其为开发构建行为记录）。
- [x] 更新 CHANGELOG 和 DEVLOG；本地 PROJECT_STATE 已记下当前实现、验证结果及真实机验收断点。
- [ ] 提交 `feat: 支持开机自动启动` 并推送 `origin/codex/window-state`。
