# 无 CLI 首次使用与 ChatGPT 登录设计

## 目标与边界

Codex Pet 必须能在未安装 Codex CLI 的 Windows 电脑上启动官方 Codex App Server，打开系统浏览器完成一次 ChatGPT 登录，并自动读取额度。它不读取 `auth.json`、Cookie、Token 或密码，也不将认证数据发送给前端。既有额度协调器、事件防抖、Tray 和窗口状态保持不变。

## 已核验的官方事实

- 官方 `openai/codex` 与 npm 包 `@openai/codex` 均为 Apache-2.0。该许可第 4 节允许以对象形式再分发，要求附带许可证和适用 NOTICE。
- 打包输入固定为官方 npm registry 的 `@openai/codex@0.156.1-win32-x64`，SHA-512 SRI 为 `sha512-MJyLxbBs2zzp5kbaR/99Zwe7SmbrwUkveTcT+ayYlO48V0nYh0eU+h2lalBwvC7VJ/ya/bXnUtISJfJKhGCD/g==`。构建脚本下载并校验归档完整性，再将 Windows x64 Runtime 和 Apache-2.0 license 放入 Tauri bundle resources；生成的二进制不提交 Git。
- App Server 帐户流程使用 `account/read`、`account/login/start`（`chatgpt`）、`account/login/completed`、`account/login/failed`、`account/login/cancelled`、`account/updated` 和 `account/login/cancel`。浏览器流程的 `authUrl` 由系统浏览器打开，不需要内嵌 WebView。

## 架构

启动时先探测受信任系统 Codex（PATH 与 Codex Desktop 安装目录），仅当版本兼容且 `account/read` 说明已有 ChatGPT 登录时复用系统会话。无可复用会话、版本不兼容或系统运行时不可用时，改用安装包 resources 中的 bundled runtime；该进程以 Tauri `app_data_dir/codex-runtime-home` 作为专用 `CODEX_HOME`。

`AccountManager` 在 Rust 进程内持有 App Server RPC 会话，向前端只暴露经过最小化的状态：`checking`、`unavailable`、`loggedOut`、`loggingIn`、`loggedIn`、`loginFailed`、`cancelled`。运行时连接或 `account/read` 失败显示为 `unavailable`，重试会重新查询帐户状态；登录失败通知和浏览器打开失败显示为 `loginFailed`。它调用系统浏览器打开 `authUrl`，按匹配的 `loginId` 处理完成通知；成功后触发现有 `QuotaRefreshCoordinator` 的完整刷新。当前实现提供 ChatGPT 浏览器登录，不提供设备码登录界面。

## 取舍

1. 只复用系统 `CODEX_HOME`：实现短，但未登录时会修改用户原有 Codex 环境，拒绝。
2. 始终使用应用私有目录：隔离最好，但无法自动复用已登录系统 Codex，拒绝。
3. 已登录系统会话复用；否则 bundled sidecar + 应用私有目录：同时满足零命令首启与隔离，采用。

## 安全、兼容与失败语义

运行时解析只接受受信任的系统 Codex 或安装目录中的 bundled runtime；`--version` 成功且版本不低于固定兼容底线才可用。系统运行时过旧或不可启动时自动回退 bundled runtime。启用诊断环境变量时，日志还会记录运行时路径、`CODEX_HOME`、PID 和状态；这些本地信息可能包含 Windows 用户目录名，不包括认证内容。卸载默认不删除应用数据，因此私有登录可保留；不会触及系统 Codex 或项目目录。

前端将“未登录”“运行时不可用”和“额度暂时无法读取”作为不同状态显示，但不泄露底层命令、路径、RPC 或凭据。所有认证判断经 App Server 帐户 API 完成。

## 自检

- 没有使用 `chatgptAuthTokens`、Token/Cookie/密码读取或传递。
- 不替换、不删除系统 Codex，也不写入系统 `CODEX_HOME`。
- bundled runtime 获取可复现、版本和完整性固定，且 Apache-2.0 license 与 vendor notices 随包保留。
- quota 仍只有一个协调器和事件刷新路径；登录成功只触发该路径。
