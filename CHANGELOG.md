# 更新记录

## 未发布

- 接入 `account/rateLimits/updated` 事件即时刷新：Rust 端 800ms 防抖后重新读取完整额度，React 通过 `quota://updated` 接收完整快照。
- 启动读取、60 秒对账和手动刷新统一使用 single-flight 协调器，刷新失败保留最后成功额度并标记 stale。
- 接入真实 Codex CLI App Server 额度读取。
- 桌宠启动后通过 Tauri command 获取统一 `QuotaSnapshot`。
- 收起状态显示最紧张 Codex 窗口的剩余百分比，展开状态显示窗口和重置倒计时。
- 增加 60 秒自动刷新和 stale/不可用提示。
