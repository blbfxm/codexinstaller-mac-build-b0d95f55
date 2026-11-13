# 恢复 Codex 初始状态（保留记录）

入口：安装与开始 → Codex 后面的三个点 → 恢复初始状态（保留记录）。

此操作用于撤销助手写入的第三方供应商连接，恢复 OpenAI 官方登录。卸载重装应用不会自动撤销用户目录中的这些配置。

- 操作前退出 Codex++。确认后助手停止自己的微信工作进程并关闭 Codex。
- 修改 `CODEX_HOME/config.toml` 的供应商、默认模型、模型目录、选中 profile 和强制登录设置；恢复 `openai`、`chatgpt` 登录和文件凭证存储。保留与连接无关的设置，包括 MCP、技能、语言及权限设置。
- 备份后移除 `auth.json` 登录凭证文件，恢复为未登录状态。不能写入空对象 `{}`：Codex 0.167.0-alpha.3.2 的 `account/read` 会将其视为不完整的认证记录，报 `plan type is required for chatgpt authentication`，导致桌面端显示 `ChatGPT hit a snag / We couldn’t load your account`。系统凭证存储不会被删除，恢复后的配置使用文件凭证。
- 清除助手/Codex++ 供应商列表中的当前启用标记，保留所有供应商条目。清除助手内存中的网关凭证，避免重启助手恢复旧网关。
- 所有原文件在替换前备份到各自父目录的 `backup/provider-<时间戳>`。完成窗口显示备份目录。备份包含敏感凭证，不能公开分享。
- 不读取、移动、删除或改写 sessions、archived_sessions、会话 SQLite 数据库及其 WAL/SHM、历史索引、全局桌面状态、附件、技能和插件目录。不重写旧会话的供应商归属。
- 不操作 WindowsApps 安装目录、应用包注册或助手登录账户，不影响账户余额。

恢复不会自动启动或重新配置第三方供应商。请启动 Codex 并登录原来的 OpenAI 账号，再继续原有的官方会话。如果之后切回第三方供应商，官方会话仍可能提示 provider mismatch；恢复功能不把官方会话迁移到其他供应商。

验证使用隔离临时目录和模拟原生命令的浏览器；不对开发机的真实 Codex 配置或会话执行恢复。

实际组件回归：`node scripts/check-codex-reset-auth.cjs <codex.exe 路径>`。脚本将独立引擎复制到 `target` 下的测试目录，用隔离的 `CODEX_HOME` 启动 app-server，仅调用本地账号读取接口。对照 `{}` 凭证触发同类错误，验证没有凭证文件时返回 `account: null / requiresOpenaiAuth: true`，不接触真实账号或现有 Codex 进程。
