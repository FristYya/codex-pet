# Codex Pet

Codex Pet 是一个 Windows 桌面悬浮额度监控桌宠，用轻量透明小窗显示本机 Codex 使用额度。

它只通过本机已登录的 Codex CLI App Server 读取额度，不读取或保存密码、Cookie、Token、会话、提示词或项目源码。

## 当前状态

V0.1 面向 Windows 10/11 开发。透明桌宠壳、系统托盘、额度响应领域模型和本地 JSON-RPC 客户端已经建立，实时额度接入与持久化仍在开发中。

## V0.1 目标

- 无边框透明悬浮桌宠
- 显示 Codex 额度百分比和重置时间
- 点击展开额度详情
- Windows 系统托盘控制
- 位置保存与多显示器恢复
- 全程本地运行，无服务器、无遥测

## 截图

原生窗口验收截图将在 Windows 真机验证完成后补充。

## 使用要求

- Windows 10 或 Windows 11
- 已安装并使用 ChatGPT 账号登录的 Codex CLI
- Node.js 24+、pnpm 11+、Rust stable，以及 Tauri 2 的 Windows 构建环境

## 开发

```powershell
pnpm install
pnpm tauri dev
```

运行前端构建：

```powershell
pnpm build
```

运行测试：

```powershell
pnpm test --run
```

## 架构

- **React UI**：只渲染桌宠和标准化后的额度状态。
- **额度领域层**：把变化中的 Codex 协议响应转换为稳定的窗口、百分比和可用性模型。
- **Rust Codex 适配器**：管理本机 App Server 子进程和 JSON-RPC 标准输入输出通道。
- **Windows 平台层**：负责透明窗口、托盘、穿透模式和位置保存。

界面不会直接读取或解析原始协议数据。

## 额度读取方式

程序会直接启动 `codex app-server --stdio`，完成初始化握手后调用 `account/rateLimits/read`。它不会启动模型回合，也不会发送提示词。

收到 `account/rateLimits/updated` 或 `account/updated` 通知后，应用会重新读取完整额度快照，而不是用稀疏通知覆盖旧数据。

## 隐私边界

Codex Pet 采用本地优先设计：

- 不连接 Codex Pet 后端，不上传额度数据
- 不提供 OpenAI 密码、Cookie 或访问 Token 输入框
- 不读取 `auth.json`、会话、提示词或项目源码
- 不保存账号标识、原始协议响应或凭据

## 路线图

### V0.1

- Codex 额度监控
- Windows 桌宠界面
- 系统托盘
- 位置持久化
- Windows 10/11 真机验收

### 后续

- 更丰富的动画与主题
- 自动更新
- 可选的本地使用历史

## 许可证

[MIT](LICENSE)
