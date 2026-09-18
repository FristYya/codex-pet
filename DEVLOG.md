# 开发日志

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
