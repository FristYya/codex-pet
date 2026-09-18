# Codex Pet：窗口状态、显示器恢复与托盘体验设计

日期：2026-09-18
状态：已批准，待实现

## 目标与边界

本阶段使 Codex Pet 成为可日常使用的 Windows 桌宠：记住收起状态下的位置，适应显示器和 DPI 变化，通过托盘控制显示、锁定、始终置顶和开机启动，并保证锁定后仍可从托盘解锁。

本阶段不扩展 Codex quota 协议，不增加 macOS 支持、安装包签名、自动更新、主题、皮肤、额度历史、多 Provider、像素级透明穿透、设置主窗口、云端或遥测。除非发现真实回归，不修改 quota 模块。

## 架构选择

采用 Rust 统一管理方案。

- Rust 的 `settings` 模块是 UI 设置、版本迁移、原子持久化、位置恢复、窗口几何和托盘状态的唯一事实来源。
- React 仅处理用户可见的展开状态和 CSS 展开方向，不持久化窗口几何，也不管理托盘状态。
- 位置、锁定、始终置顶、可见性和开机启动偏好写入 Tauri 应用用户级配置目录下的 `ui-settings.json`；不写项目目录、安装目录、Git 仓库或自定义全局目录。
- 开机启动使用 Tauri 官方 `tauri-plugin-autostart`，不直接修改 Windows Registry。

## 数据模型与兼容性

当前配置版本为 `1`：

```json
{
  "version": 1,
  "window": {
    "x": 100.0,
    "y": 200.0,
    "width": 164.0,
    "height": 154.0,
    "monitorKey": "name:DISPLAY2",
    "monitorName": "DISPLAY2",
    "scaleFactor": 1.0
  },
  "locked": false,
  "alwaysOnTop": true,
  "visible": true,
  "autostart": false
}
```

`window.x/y` 是相对于目标显示器当前 `work_area` 左上角的逻辑坐标，不是跨显示器的全局逻辑坐标。`width/height` 用于兼容、校验和未来迁移；V0.1 不允许用户 resize，当前程序定义的合法 collapsed size 是运行时权威值，读取到异常或过期尺寸时必须使用该权威值，旧配置不得永久覆盖新版预期尺寸。`monitorKey` 优先使用显示器名称；名称不可用时使用物理位置与物理尺寸组成的弱指纹，并以 `monitorName` 作为第二匹配条件。

保存的数据不包含 Codex 原始响应、Account ID、邮箱、Token、Cookie、Prompt、Session 或源码。

版本策略：

- `version == 1`：直接读取。
- `version < 1`：按显式迁移函数逐版本迁移后使用。
- `version > 1`：不迁移、不覆盖、不备份原文件；记录不含配置内容的兼容性提示，并以默认内存设置启动，防止旧程序破坏新版本配置。
- 无版本、字段缺失、无效字段、损坏 JSON 或读取失败：使用默认设置继续启动。对可安全处理的损坏/旧文件，保留备份后生成有效 v1 配置；日志仅记录解析或兼容性失败类别，不记录配置内容。

默认设置为：`alwaysOnTop=true`、`locked=false`、`visible=true`、`autostart=false`。首次启动时，桌宠出现在主显示器 work area 右下角附近，留出任务栏与屏幕边缘的安全边距。

## 原子持久化与写入节流

设置写入在 Windows 使用同目录原子写流程：创建同目录临时文件、写入、`flush`/`sync_all`，再用支持 replace-existing 的安全替换操作替换目标文件。实现不得假设普通 rename 可以覆盖已存在目标，也不得采用“先删除旧 `ui-settings.json`，再 rename”的流程；替换失败时必须保留上一份有效配置。临时文件不会成为有效配置来源。

窗口移动只更新内存中的最后 `collapsedRect`。连续移动使用约 500–800ms debounce，静止后才持久化，避免每个 mousemove 写磁盘。退出时取消待执行 debounce 并同步 flush 最后内存状态；因此“刚完成拖动但 debounce 尚未触发即退出”仍保存最后位置。

## 多显示器、工作区与 DPI 恢复

保存时：

1. 取得收起窗口的全局物理 outer position。
2. 取得所在显示器及其物理 `work_area`。
3. 用该显示器保存时的 `scaleFactor`，将“窗口物理位置减 work-area 物理 origin”转换为相对 work area 的逻辑 `x/y`。
4. 保存显示器 key/name、保存时 scale factor 与收起尺寸。

恢复时：

1. 依次按 `monitorKey`、`monitorName` 查找保存时显示器。
2. 找不到时回退主显示器；主显示器不可得时选择系统报告的第一个显示器。
3. 使用目标显示器当前 `scaleFactor`，将相对逻辑 `x/y` 转为物理偏移，再加上目标 `work_area` 的物理 origin。
4. 对收起窗口完整矩形执行 work-area clamp，保证整个桌宠可见；副屏被拔除、屏幕排列改变和坐标越界均不能将窗口恢复到可见范围外。

所有物理/逻辑转换只在上述边界发生。不同显示器可拥有不同 DPI，绝不假设全局 scale factor 相同。

## 展开布局

持久化永远只保存 `collapsedRect`。展开状态使用临时窗口矩形，收起时恢复收起矩形，不让展开导致位置漂移。四个候选 expandedRect 都以 collapsedRect 为固定视觉 anchor：无论选择哪个方向，宠物本体在屏幕上的物理位置都不得因展开而跳动。

展开时 Rust 在当前显示器 work area 内评估右下、左下、右上、左上四个候选 Rect，计算每个候选 Rect 与 work area 的可见面积，选择面积最大者；完全并列时按右下、左下、右上、左上的确定顺序决策。选定后对临时 Rect clamp，并把 `horizontal: left|right` 与 `vertical: up|down` 传给 React。React 只据此切换 CSS class，令详情卡片尽可能完整可见。

## 托盘、锁定与原生状态

Tray 菜单包含：

- 显示宠物 / 隐藏宠物
- 锁定位置 / 解除锁定
- 始终置顶
- 开机启动
- 刷新额度
- 退出

锁定、始终置顶和开机启动使用可勾选菜单项。Window、Tray 和 Autostart 原生操作经由薄抽象层或等价的可测试边界调用。每次托盘操作先执行原生窗口或 autostart 动作；仅在成功后更新内存设置、菜单勾选状态和待持久化状态。失败时恢复原有状态和菜单显示，避免状态分叉。Rust 单测必须覆盖成功后更新、失败保持旧状态和 Tray checkbox 回滚。

“显示宠物”只调用 `show`，不主动抢焦点；“隐藏宠物”调用 `hide`。`CloseRequested` 严格执行 `prevent_close → hide → visible=false`；进程的真正退出只通过 Tray “退出”与 `RunEvent::Exit` 生命周期处理。

锁定只允许在 Tray 成功构建后恢复：

1. 锁定时禁用拖动区域，调用整窗 `set_ignore_cursor_events(true)`，鼠标事件穿透到后方应用。
2. 解锁时先关闭鼠标穿透，再恢复拖动区域。
3. Tray 初始化失败时强制 `locked=false`，确保桌宠不会变成无法点击、无法拖动、无法解锁的窗口。
4. 任意原生锁定操作失败都回滚为可操作状态。

## Autostart

应用启动时按 `OS is_enabled() → 内存 settings → Tray checkbox` 同步，OS 插件 `is_enabled` 的实际返回值是事实源。启动时不会因为 JSON 中保存了 `autostart=true` 而主动重新 enable。只有用户从 Codex Pet 主动修改时，才调用官方插件的 `enable` 或 `disable`；成功后才更新 `autostart` 设置和菜单状态，调用失败则保留原状态。自动化测试只验证该抽象层与状态同步逻辑，真实 Windows 登录后是否自动启动只归入真机验证。

## 启动顺序

1. Tauri 创建原生窗口对象，但窗口初始不可见。
2. 创建 Tray。
3. 读取、校验和迁移设置。
4. 恢复收起位置、收起尺寸与 always-on-top。
5. 根据 `visible` 显示窗口或保持隐藏；显示不抢焦点。
6. 确认 Tray 可用后才恢复 lock 状态。
7. 查询 autostart 的 OS 实际状态并同步 Tray。
8. 最后异步启动 Codex Quota Service，额度读取不阻塞桌宠出现。

## 测试与验收

Rust 单元测试覆盖不依赖真实显示器的纯逻辑：

| 范围 | 验收用例 |
| --- | --- |
| 默认与版本 | 默认配置；v1 读写；字段缺失；损坏配置 fallback；旧版本迁移；未来版本只读保护且不覆写 |
| 原子写 | 同目录临时文件写入/flush/`sync_all`/replace-existing 成功；写入或 replace 失败时上一份有效配置不受破坏，且不采用删除旧文件的降级流程 |
| 节流与退出 | 连续快速移动只持久化最后状态；debounce 未落盘时退出 flush 最后位置 |
| 位置恢复 | 首次右下；保存/恢复 work-area 相对坐标；主屏回退；缺失显示器回退；坐标 clamp；完整窗口可见；异常/过期配置尺寸回退当前合法 collapsed size |
| DPI | 100%、125%、150%；物理到相对逻辑保存；目标显示器当前 scale factor 恢复；不同 DPI 双屏 |
| 展开 | 四个候选 Rect 的可见面积决策；并列方向顺序；固定 collapsedRect 视觉 anchor；持久化只使用 collapsedRect |
| 状态 | Tray 成功后才恢复 locked；always-on-top；visible；CloseRequested hide 语义；原生操作失败保持旧内存状态并回滚 Tray checkbox |
| Autostart | `is_enabled` 初始化；enable/disable 成功后的同步；失败后的状态回滚 |
| 托盘 | 菜单勾选与内存/原生结果一致；刷新额度继续复用既有 `read_quota` 协调器 |

前端测试仅覆盖无需真实 Windows Runtime 的用户可见行为：展开方向 CSS class、锁定时拖动区域不可用、详情展开不改变持久化位置意图，以及既有 quota 刷新/失败显示语义不回归。

Windows 真机验证单独执行并独立报告：

- 单屏：拖动、关闭、打开后位置恢复。
- 双屏：拖到第二显示器后关闭/打开，恢复到第二显示器。
- 拔掉副屏：副屏保存位置后拔除，启动时回退主屏可见区域。
- DPI：100%、125%、150%，有条件时验证不同 DPI 双显示器。
- 锁定：锁定后鼠标穿透，Tray 可解除锁定。
- 开机启动：启用后重启/登录验证自动启动；关闭后验证不会启动。

自动化测试、Clippy 和 Build 不能替代真机验证。硬件条件不足的条目必须在最终报告中明确标记为“未实际验证”。

## 实现阶段与提交

设计文档先在当前 `main` 提交并 push；仅以该确定设计提交为基线创建 `codex/` 前缀 feature 分支与隔离 worktree。不得带未提交设计改动创建 worktree。

实现按以下边界分批，每批均遵循失败测试、最小实现、验证、更新 `CHANGELOG.md` 与 `DEVLOG.md`、提交、push 的顺序：

1. `feat: 保存并恢复桌宠位置`
2. `feat: 支持多显示器位置恢复`
3. `fix: 修复不同 DPI 下桌宠位置恢复`
4. `feat: 完善桌宠锁定与托盘状态`
5. `feat: 支持开机自动启动`

最终交付必须报告位置持久化、多显示器和 DPI 的实际验证范围、托盘能力、autostart 真机结果、Rust/Frontend 测试数量、Clippy/Build 状态、全部 commit hash、push 状态与遗留问题；未实际验证的能力不得表述为已验证。
