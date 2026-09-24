# Codex Pet

Codex Pet 是面向 Windows 的本地透明悬浮桌宠，用小窗持续显示你当前的 Codex 使用额度。它会优先复用系统中兼容且已登录的 Codex Runtime；否则使用安装包附带的官方 Runtime，并引导你通过系统浏览器登录 ChatGPT。额度由本机 App Server 读取，无需另行注册 Codex Pet 账号。

Codex Pet is an independent open-source project and is not an official OpenAI product.

## 核心功能

- 显示当前最紧张额度窗口的剩余百分比、重置时间与详细额度。
- 启动时读取真实额度；额度事件触发即时刷新，并每 60 秒自动对账刷新一次。
- 点击桌宠展开或收起详情；按住或拖动桌宠可移动位置。
- 位置会保存，并在多显示器、显示器拔插和不同 DPI 缩放下尽力恢复到可见位置。
- 系统托盘提供显示、隐藏、锁定位置、始终置顶、刷新、开机自动启动与退出操作。

## 系统要求

- Windows 10 或 Windows 11（x64）。
- 无需预装 Codex CLI，也不需要打开终端。

Codex Pet 不提供密码、Cookie 或 Token 输入框，也不会自行解析或展示认证文件。随包的官方 Codex Runtime 会在 Codex Pet 专用的用户数据目录中管理自己的登录数据，并通过本机 App Server 使用该登录状态。若发现已有可用 Codex 登录会直接复用；否则桌宠会引导你通过系统浏览器登录 ChatGPT。

## 安装

1. 在 [GitHub Releases](https://github.com/FristYya/codex-pet/releases) 下载最新的 Windows 安装包。
2. 运行下载的安装程序，并按 Windows 提示完成安装。
3. 从开始菜单启动 **Codex Pet**。
4. 如果桌宠提示尚未连接 ChatGPT，点击“登录 ChatGPT”，并在系统浏览器中完成登录。
5. 返回桌面后额度会自动显示；以后启动无需重复操作。

同一版本提供两种安装包时：

- **NSIS `.exe`**：适合大多数个人用户，安装与卸载流程更熟悉。
- **MSI `.msi`**：适合企业部署、软件分发或偏好 Windows Installer 的用户。

需要卸载时，请在 Windows 的“已安装的应用”中选择 **Codex Pet** 并卸载；也可使用安装程序提供的卸载入口。

## 使用方式

- **拖动与展开**：短按桌宠可展开或收起详情；按住约 250ms，或移动超过约 6px，即可拖动桌宠。刷新按钮不会触发拖动。
- **Tray**：右键系统托盘中的 Codex Pet 图标，可显示或隐藏桌宠、手动刷新、调整置顶状态或退出。
- **锁定位置**：从 Tray 选择“锁定位置”后，桌宠会忽略鼠标操作以避免误触；请从 Tray 取消锁定。
- **开机自动启动**：从 Tray 勾选“开机自动启动”。正式安装版本会由 Windows 在登录后启动其已安装的 Release 程序；取消勾选即可关闭。

## Local First 与隐私

Codex Pet 全程在本机运行：没有 Codex Pet 后端、没有遥测，也不会上传额度数据。它只向本机 Codex App Server 发起额度读取请求，不会启动模型对话或发送提示词。

Codex Pet 的界面与 Rust 适配层不会直接读取、记录或展示密码、Token、Cookie、`auth.json` 或会话内容；随包的官方 Codex Runtime 会在专用 `CODEX_HOME` 中读取和保存其登录数据，以便 App Server 恢复登录。该目录位于 Codex Pet 的用户级应用数据目录，卸载后默认保留；它不会改动系统 Codex 或用户项目。额度请求和快照只在本机处理，没有 Codex Pet 后端或遥测，也不会上传额度数据。

## 当前限制

- v0.1.0 正式支持 Windows 10/11 x64；macOS 尚未作为正式发布目标支持。
- 额度显示依赖可用的本地 Codex App Server；安装包会随附已验证版本的 runtime，系统已有的兼容登录可直接复用。
- 多显示器和 DPI 恢复已具备保护逻辑，但完整的真实硬件组合仍会随 Windows、显示器和缩放设置而不同。
- 不提供额度历史、自动更新、独立设置主窗口、额外皮肤或 Claude/Gemini 支持。

## Roadmap

- 继续完成更多 Windows 真实设备组合的验收。
- 在未来版本评估：额度历史、自动更新、新动画与皮肤、设置窗口、其他模型服务，以及 macOS 适配。

这些项目不属于 v0.1.0 的功能范围。

## 开发与验证

开发环境需要 Node.js、pnpm、Rust stable 与 Tauri 2 的 Windows 构建环境。

```powershell
pnpm install
pnpm run prepare-runtime
pnpm test --run
pnpm build
Set-Location src-tauri
cargo test
cargo clippy -- -D warnings
```

## 许可证

本项目采用 [MIT License](LICENSE)。第三方组件与参考项目的说明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
