# 开发日志

## 2026-09-24：v0.1.1 刷新反馈与 5 小时额度优先展示

- 手动刷新立即进入 loading，禁用重复点击并显示轻量旋转指示；刷新 Promise 完成后显示成功或失败反馈，提示 2.5 秒后清除。automatic/manual 调用复用前端进行中的 Promise，未改 Rust quota coordinator、single-flight、事件或 60 秒轮询。
- 刷新失败继续保留最近成功额度并标记 stale；新增回归覆盖旧快照、失败反馈、并发调用复用、组件卸载与反馈 timer 清理。
- 主额度依据窗口语义排序：300 分钟 5H、其他短周期、Weekly；详情也将所选主额度置顶。只有 Weekly 或缺少可识别周期时沿用有效额度 fallback，不依赖返回数组顺序或固定 limitId。
- 自动化验证通过：Rust 115 项、前端 51 项、`cargo fmt -- --check`、`cargo clippy -- -D warnings`、`pnpm build` 与 `git diff --check`。
- 本轮未重新启动桌面窗口进行真实按钮点击和真实 5H/Weekly 并列额度人工验收；当前环境没有可用的原生窗口交互通道，不能将自动化测试称作真机人工验证。

## 2026-09-24：Bundled Runtime、登录恢复与生命周期验收

- 固定官方 `@openai/codex@0.156.1-win32-x64` 包及 SHA-512 SRI；Tauri 将 Runtime 作为安装资源打包，运行时从应用 exe 相邻的 `resources/codex-runtime/bin/codex.exe` 定位。
- 已登录兼容的系统 Runtime 优先复用；本机真机验收在系统 Runtime 不可用时使用 bundled Runtime，并为其保持稳定的应用私有 `CODEX_HOME`。登录状态通过 App Server `account/read` 判断，不读取系统或私有 `auth.json` 内容。
- 两轮 Tray 正常退出和重启均通过：退出后桌宠及两个 App Server 进程消失；重启后无需重新登录，`LoggedIn`、真实 5H/Weekly quota、automatic refresh 和 manual refresh 均成功。两个正常退出的 quota App Server 均以退出码 0 结束；新版本日志未再出现 `0xC000013A`。
- App Server 遇到终止类错误时单次重连、重建 quota generation 并重新订阅通知；初始化使用 15 秒期限，普通请求使用 5 秒期限。附着控制台回归测试修复前失败、修复后通过。
- 运行时或 `account/read` 失败会呈现独立的 `unavailable` 状态，并可原地重试读取；登录通知触发的 `account/read` 错误也不再被折算成已登出或登录失败。新增 Rust 与前端回归测试。
- bundled Runtime 的 Apache-2.0 license 现作为 `resources/codex-runtime/CODEX-RUNTIME-LICENSE.txt` 随 MSI/NSIS 资源打包；第三方说明和设计文档的 SHA-512 SRI 与构建脚本常量一致。
- 用户在 Windows 安装版验收拖动、锁定/解锁、Hide/Show、Always on Top 和 Autostart；最终设置恢复为未锁定、始终置顶、自动启动开启。注册表启动目标核实为安装版 Release exe。系统与私有 auth 文件大小、修改时间保持不变。

## 2026-09-23：开机自动启动

- 接入官方 `tauri-plugin-autostart` Rust manager（2.x）；Tray 提供“开机自动启动”勾选项，不增加前端插件，也不直接调用 Windows Registry。
- 启动时用插件查询的系统实际状态校正 Tray 与设置快照，避免用户通过 Windows 启动应用管理器关闭后又被应用擅自启用。
- Tray 初始化失败时仍查询并保存原生自动启动状态，避免设置镜像因没有菜单而保持陈旧；窗口按既有安全回退保持可操作。
- 运行时切换先查询并操作系统启动注册，再同步 Tray；两步成功后更新 `UiSettings.autostart` 并进入现有原子持久化调度。失败补偿恢复原生状态与菜单，补偿失败要求关闭应用。
- 为旧 `ui-settings.json` 缺少 `autostart` 字段的情况增加默认关闭兼容，不升级 schema 版本。
- 自动化验证已覆盖启用/关闭、查询/设置/菜单失败补偿、补偿失败、Tray 不可用或同步失败时的启动状态处理和旧配置解析；Rust 85 项、前端 32 项通过，Clippy 与 MSI/NSIS Release 构建通过。
- Windows 人工验收通过：启用后退出并重开仍勾选，关闭后退出并重开已取消勾选，最终保持关闭。用户观察到 Debug 启动目标为当前 worktree 下的 Debug exe 且 Windows UI 显示为灰色，Release 正常；插件按启用时的 `current_exe` 登记目标，因此开发版的启动目标不同于正式 Release 属预期，最终关闭避免启动调试版。

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
