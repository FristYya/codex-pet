# 开机自动启动设计

## 目标

用户可通过系统 Tray 菜单启用或关闭 Codex Pet 的 Windows 登录启动；菜单和用户设置反映系统当前状态。

## 设计

- 只使用官方 Tauri `tauri-plugin-autostart` Rust 插件；不直接访问或修改 Windows Registry，也不增加 JavaScript 插件依赖。
- 在 Tray 菜单增加“开机自动启动”勾选项，并通过官方 `AutoLaunchManager` 查询、启用和关闭启动项。
- 用户切换时先读取原生真实状态，再执行插件操作、同步 Tray 勾选；所有原生操作成功后才更新 `UiSettings.autostart` 并安排原子持久化。
- 操作失败时将插件和 Tray 恢复到原始状态；补偿失败沿用现有安全策略，显式终止状态不确定的应用。
- 启动时以插件查询到的实际启用状态校正 Tray 与 `ui-settings.json` 镜像，避免手工关闭系统启动项后应用又擅自重新注册。
- 若 Tray 可用，先成功同步菜单再提交设置镜像；若 Tray 初始化失败，则仍以原生查询结果更新设置。菜单同步失败时不提交新的设置镜像。
- 旧设置若没有 `autostart` 字段，按关闭处理；不升级现有设置版本。
- 不修改 quota / Codex adapter 模块。

## 验收

- Rust 测试覆盖启用、关闭、查询失败、插件操作失败、Tray 更新失败后的回滚，以及启动状态同步。
- Windows 开发版验证 Tray 勾选能真实增删启动注册、重启应用后状态与菜单一致；关闭后确认插件查询为 false。
- 完成 Rust / 前端测试、Clippy、生产构建、`git diff --check`，更新 CHANGELOG、DEVLOG 和本地 PROJECT_STATE，再提交并推送 `codex/window-state`。
