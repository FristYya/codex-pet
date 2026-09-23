# 开发日志

## 2026-09-23：锁定、Tray 与拖动手势验收

- 桌宠主体与顶部拖动柄共用 Pointer Events 手势状态；短按保留详情展开/收起，长按 250ms 或移动超过 6px 时只调用一次 Tauri `startDragging()`。
- 刷新按钮不参与拖动；拖动后的 click 被抑制；pointercancel、卸载与进入锁定时清理手势计时器和状态。
- 用户人工验收通过：主体短按、长按拖动、移动拖动、刷新按钮、Tray 锁定穿透/解锁点击拖动、隐藏/显示以及显示后不抢前台焦点。
- 清理用于真机排障的临时 Tray action 日志；错误路径日志仍保留。

## 2026-09-18：锁定与 Tray 状态

- `WindowStateController` 以 `NativeWindow` / `TrayState` 为可测试边界：原生操作与 checkbox 同步全部成功后才提交内存设置，任一步失败则恢复窗口和菜单旧状态。
- 补偿回滚失败会聚合原始错误与回滚错误并显式终止应用，避免原生鼠标穿透、Tray 勾选与内存设置在不确定状态下继续运行。
- 启动恢复锁定以 Tray ready 为前提；checkbox 同步失败或 Tray 初始化失败时关闭鼠标穿透、保持未锁定并显示窗口。
- Windows 的 Show 路径通过 Tauri `WebviewWindow::show()` 同步 Tao 可见状态，且不调用 `set_focus`。
- Tray 初始化失败后，若强制解锁或显示窗口也失败，setup 直接返回错误终止启动，不留下无 Tray 且不可见的后台进程。
- `CloseRequested` 在 Tray 可用时按 `prevent_close → hide → visible=false` 顺序处理；`hide` 失败不会伪造已隐藏持久化状态。
- React 通过初始状态 command 和 `window://locked` 事件同步锁定状态；实际拖动由主体与拖动柄共用的 Pointer Events 状态机调用 Tauri `startDragging()`，不依赖注入式 `data-tauri-drag-region`。
- 本批 Windows 真机人工验收已通过：锁定穿透、Tray 解锁、隐藏/显示、拖动与刷新按钮行为、显示后不抢前台焦点。

## 2026-09-18：不同 DPI 位置恢复

- 补充 125%→150% 与 150%→100% 恢复测试；物理坐标、work area clamp 与 164×154 logical 收起尺寸均由目标屏当前缩放计算。
- 跨屏后下一次用户移动以新显示器的 work area、scale factor 和身份生成持久化快照。

## 2026-09-18：多显示器位置恢复

- 启动恢复先按 `monitorKey`、`monitorName` 匹配已保存显示器，缺失或不可用时保持主显示器、首个可用显示器的回退顺序。
- 用户将桌宠移动至另一显示器时，持久化快照更新该显示器身份与 work area 相对位置；本批不改变既有 DPI 转换规则。

## 2026-09-18：桌宠位置持久化

- Rust 设置层新增版本化 UI 配置、损坏配置 fallback、未来版本只读保护、Windows 原子替换和退出同步 flush。
- 收起位置仅以目标显示器 work area 的相对逻辑坐标持久化；启动使用目标当前 DPI 生成 164×154 logical 的运行时矩形并 clamp。
- 生产链路为 `WindowEvent::Moved → RuntimeWindowState → 单一 token debounce scheduler → atomic writer`；启动产生的程序移动会被一次性消费。
- 已在本机启动 Tauri 桌宠，确认首次启动在主显示器右下角生成 `ui-settings.json`，且记录当前显示器和 150% DPI 下的相对逻辑位置。

## 2026-09-17：真实额度链路

- 已验证本机 Codex CLI 可被发现，版本为 `0.155.0-alpha.2.6`。
- 已通过 `codex app-server --stdio` 完成初始化并成功调用 `account/rateLimits/read`。
- 本次真实响应包含 5H 和 Weekly 两个 Codex 窗口；应用只保留额度字段，不记录账号标识或原始响应。
- Tauri `read_quota` command 已将 Rust Adapter 结果转换为前端 `QuotaSnapshot`。
- 已完成 Rust 24 项测试、前端 11 项测试、Clippy 和构建验证。

## 2026-09-18：额度事件即时刷新 MEP

- `account/rateLimits/updated` 只作为刷新信号，Rust 端固定 800ms 防抖后重新执行完整 `account/rateLimits/read`。
- 启动读取、60 秒轮询、手动刷新和事件刷新共用 single-flight 协调器。
- Tauri 通过 `quota://updated` 发送完整 `QuotaSnapshot`；React 正确注册/释放 listener，失败时保留最近成功额度并显示 stale。
- 未记录 Prompt、邮箱、Account ID、Token 或 Session 内容。
