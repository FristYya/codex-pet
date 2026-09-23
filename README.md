# Codex Pet

Codex Pet 是面向 Windows 的本地透明悬浮桌宠，用小窗持续显示你当前的 Codex 使用额度。它通过本机已登录的 Codex CLI App Server 获取额度；不需要另行注册 Codex Pet 账号。

Codex Pet is an independent open-source project and is not an official OpenAI product.

## 核心功能

- 显示当前最紧张额度窗口的剩余百分比、重置时间与详细额度。
- 启动时读取真实额度；额度事件触发即时刷新，并每 60 秒自动对账刷新一次。
- 点击桌宠展开或收起详情；按住或拖动桌宠可移动位置。
- 位置会保存，并在多显示器、显示器拔插和不同 DPI 缩放下尽力恢复到可见位置。
- 系统托盘提供显示、隐藏、锁定位置、始终置顶、刷新、开机自动启动与退出操作。

## 系统要求

- Windows 10 或 Windows 11。
- 已安装 Codex CLI 或 Codex Desktop，并已使用你的 ChatGPT 账号完成登录。
- 设备可正常启动 `codex app-server --stdio`。

Codex Pet 不会替你登录，也不会读取 `auth.json`、密码、Cookie 或 Token。它会优先使用 PATH 中的 Codex CLI；在 Windows 上也支持 Codex Desktop 的本机安装。若 Codex 未安装、未登录或无法读取额度，桌宠会显示不可用状态。

## 安装

1. 在 [GitHub Releases](https://github.com/FristYya/codex-pet/releases) 下载最新的 Windows 安装包。
2. 运行下载的安装程序，并按 Windows 提示完成安装。
3. 从开始菜单启动 **Codex Pet**。桌宠会出现在屏幕上，并从本机已登录的 Codex CLI 读取额度。

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

Codex Pet 全程在本机运行：没有 Codex Pet 后端、没有遥测，也不会上传额度数据。它只向本机 Codex CLI App Server 发起额度读取请求，不会启动模型对话或发送提示词。

程序不读取或保存密码、Token、Cookie、`auth.json`、会话、提示词、项目源码、账号标识或原始协议响应。

## 当前限制

- 目前仅支持 Windows 10/11；尚未提供完整的 macOS 适配。
- 额度显示依赖已登录且可用的 Codex CLI，以及其 App Server 提供的额度接口。
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
pnpm test --run
pnpm build
Set-Location src-tauri
cargo test
cargo clippy -- -D warnings
```

## 许可证

本项目采用 [MIT License](LICENSE)。第三方组件与参考项目的说明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
