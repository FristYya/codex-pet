# 更新记录

## 0.1.0 - 2026-09-24

- 无需预装 Codex CLI：安装包附带固定版本且经完整性校验的官方 Windows Runtime；已有兼容登录时复用，否则通过系统浏览器登录并保存在 Codex Pet 专用数据目录。
- 区分 Runtime 暂时不可用与 ChatGPT 登录失败；连接恢复后可重试帐户状态读取，不会误提示重新登录。
- 安装包随 bundled Codex Runtime 提供 Apache-2.0 许可证文本，并保留厂商 notices 与第三方许可文件。
- 登录状态恢复后立即读取真实额度；手动和自动刷新共用同一额度协调器，App Server 重连后会重新订阅额度通知。
- 隔离 bundled App Server 的 Windows 控制台事件，应用正常退出时回收子进程，避免控制台 Ctrl+C 造成额度服务意外断连。
- 修复由 Windows Explorer 启动时无法继承 Codex Desktop 临时 PATH，导致无法读取额度的问题；在 PATH 未找到 CLI 时，仅回退到受限的 Codex Desktop 本机安装目录。
- 完善原生 Tray 控制：支持显示、隐藏、锁定位置、始终置顶、刷新额度与退出，显示桌宠时不抢占焦点。
- 锁定后整窗鼠标穿透并禁用拖动区；原生窗口或 Tray 操作失败时回滚到旧状态，Tray 不可用时保持窗口可操作。
- 关闭按钮在 Tray 可用时改为隐藏桌宠并保存可见状态；Tray 初始化失败时仍允许正常关闭。
- 桌宠主体和顶部拖动柄共用 Pointer Events 手势：短按切换详情，按住 250ms 或移动超过 6px 后开始拖动；刷新按钮不触发拖动。
- Tray 菜单新增“开机自动启动”，通过官方 Tauri Autostart 插件管理登录启动，并在启动时同步系统实际状态。
- 开机启动切换先更新系统和 Tray，成功后持久化；失败时回滚原生状态和菜单状态。
- 修复不同 DPI 间的桌宠位置恢复：使用目标显示器当前缩放重新计算位置与收起尺寸。
- 支持按保存的显示器身份恢复桌宠位置；原显示器不可用时回退主显示器或第一个可用显示器，并保持完整可见。
- 保存并恢复桌宠收起位置：配置使用版本化 `ui-settings.json`，Windows 上同目录临时文件同步后以 `ReplaceFileW` 原子替换。
- 启动时按主显示器 work area 恢复或在右下角以 16 logical px 边距首次定位；位置以显示器 work area 相对逻辑坐标保存，并按当前 DPI 恢复、完整可见 clamp。
- 窗口移动使用单一 650ms token debounce 调度器；退出时停止接收移动并同步写入最后待保存位置。
- 接入 `account/rateLimits/updated` 事件即时刷新：Rust 端 800ms 防抖后重新读取完整额度，React 通过 `quota://updated` 接收完整快照。
- 启动读取、60 秒对账和手动刷新统一使用 single-flight 协调器，刷新失败保留最后成功额度并标记 stale。
- 接入真实 Codex CLI App Server 额度读取。
- 桌宠启动后通过 Tauri command 获取统一 `QuotaSnapshot`。
- 收起状态显示最紧张 Codex 窗口的剩余百分比，展开状态显示窗口和重置倒计时。
- 增加 60 秒自动刷新和 stale/不可用提示。
