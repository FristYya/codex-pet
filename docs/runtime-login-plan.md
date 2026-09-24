# Runtime 与首次登录实施计划

**目标：** 将 Codex Pet 从“要求用户预装并登录 CLI”升级为“bundled runtime + 应用内浏览器登录”。

1. 新增可单测的运行时选择模型：系统运行时探测、版本门槛、bundled Runtime 资源路径和私有 `CODEX_HOME` 环境；先写缺失/过旧/回退的失败测试，再接入进程启动。
2. 增强 App Server 客户端：先写 `account/read`、开始/取消登录和匹配通知路由的失败测试；实现宽松 JSON 解析及敏感字段不外泄。
3. 在应用状态层新增 AccountManager：先写已登录、已登出、浏览器失败、完成、失败和取消测试；登录成功调用原有额度协调器，不复制额度服务。
4. 前端先写“未连接”“等待浏览器”“登录成功切回额度”的组件测试；实现桌宠轻量卡和最小 Tauri commands/events。首版采用系统浏览器 ChatGPT 登录，不提供设备码登录界面。
5. 新增固定版本的官方 Runtime 获取脚本和 Tauri bundle resources 配置；脚本校验 npm SRI、平台二进制及许可文件，并将生成目录忽略。
6. 更新 notices、README、开发构建说明和卸载数据保留策略；执行前端、Rust、Clippy、release bundle 与 Windows PATH 缺失场景验收。

每一步均按 Red → Green → Refactor 进行，分别提交后推送；正式 v0.1.0 tag 与 Release 仍保持禁止状态。
