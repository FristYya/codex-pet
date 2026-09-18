# 开发日志

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
