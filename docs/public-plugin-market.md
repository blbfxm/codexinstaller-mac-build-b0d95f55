# 公开插件市场

在“技能与插件 → 插件库”中，“插件市场”与“内置与用户分享”分开。前者是公开精选源，后者保留内置生图、转矢量插件及社区投稿、审核、下载计费和收益功能。

公开源参考 [Codex++ 插件市场](https://github.com/BigPizzaV3/CodexPlusPlus/blob/main/docs/plugin-market.md)，使用 [CodexPlusPlusPluginCache](https://github.com/BigPizzaV3/CodexPlusPlusPluginCache) 的公开 plugin-index.json。没有复制上游市场实现。内置目录固定在 revision `1c13d7180da9717bb5b8b35fdb40261290bacd7d`，索引日期 2026-10-08，共 520 个插件，其中 104 个声明需要服务授权。

`src-tauri/catalog/public-plugins.json` 保留源说明、作者、版本、许可及内容树标识，并逐项增加中文功能摘要 `descriptionZh`。卡片、搜索和详情优先使用中文；详情可展开原文。中文摘要不修改插件代码、技能或实际执行指令。后续维护应对照新的英文说明更新中文摘要，不按标签套用通用描述。

首次浏览读取内置或上次缓存目录；“刷新市场”先请求 raw.githubusercontent.com，失败再尝试 GitHub contents API。离线或刷新失败时保留本地目录并显示提示。刷新后仅对原文未变化的条目复用中文摘要；新条目或原文变化时显示“中文介绍正在补充”，保留原文供查看，不显示过时翻译。

支持中英文名称、中文说明、作者和标签搜索，按回车或搜索按钮提交；提供全部、可安装、已安装、有更新、需服务授权筛选，按内容树检测更新，每页 48 条。详情包含作者、版本、大小、技能数量、许可、安装状态、使用步骤及授权费用提示。下载安装不扣除坚果点数，外部服务仍按其要求授权和收费。市场请求不携带坚果 API Key。

安装仅下载所选插件的固定 Git tree ZIP，无需 Git，不下载整个仓库。下载前由原生端重新查找目录条目、核对 tree 并检测 Codex。压缩包最多 64 MiB，解压沿用现有路径、符号链接和大小检查（单文件 32 MiB、合计 256 MiB、最多 20,000 项）。支持 `.codex-plugin/plugin.json`、`.claude-plugin/plugin.json` 和根目录 `plugin.json`；后两者保留原文并补充 Codex 清单，验证名称和版本后再注册。Unix 保留脚本可执行位。

每个插件使用独立的 `usage-public-<id哈希>` 市场，源码记录放在 `CODEX_HOME/.usage-assistant/public-plugins`，内容放在 `plugins/cache/<独立市场>/<name>/<version>`，市场清单使用该缓存根目录内的相对路径。配置、原生市场清单和安装记录通过既有备份回滚机制提交。更新将旧版本移出 Codex 可发现的缓存目录，避免误载旧版；卸载只移除该独立市场；原公共技能、社区付费插件和内置插件不共用安装记录。安装或卸载后可点击“重启 Codex”，然后在聊天框输入 @ 选择插件。

验证命令：

```powershell
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --lib plugin --no-fail-fast
cargo test --manifest-path src-tauri/Cargo.toml --lib skill_archive --no-fail-fast
cargo test --manifest-path src-tauri/Cargo.toml --lib public_plugin_market::tests::live_public_package_registers_native_market -- --ignored --nocapture
node scripts/check-account-wechat-usage.cjs
```

联网测试在随机临时 Codex home 安装和卸载 Agent Parley、Zotero；本机存在测试 Codex CLI 或配置 `CODEX_TEST_PLUGIN_CLI` 时，还检查真实 CLI 是否识别相应的原生安装。测试不使用真实账户密钥，不访问收费服务。界面测试覆盖 520 条中文、104 个授权条目、分页、搜索、失败重试、跨页面安装、更新卸载、分区隔离和明暗主题的 390/760/1120 像素宽度。
