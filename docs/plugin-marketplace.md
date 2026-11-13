# 技能库与插件市场

侧栏“技能与插件”支持展开/收起，二级菜单依次为：技能库、插件库。插件库右上角“我要分享”进入投稿与收益页面，“返回插件库”返回。分享表单进入即展开。切换目录不会清空分享草稿；展开/收起不切换当前页面。原有技能目录、中文介绍、分类、搜索和安装均保留。市场搜索按回车或点击搜索触发。

## 展示与使用

- 文生图、位图转矢量图：不展示来源标签。状态显示在插件名称同一行，不显示卡片账户绑定提示；实际凭证绑定功能保留。与用户分享卡片处于同一网格，常规桌面宽度下首屏同时展示。内置插件安装免费，服务使用仍按原规则收费。支持更新、卸载，点击“使用说明与费用”打开浮动窗口，正文在窗口内滚动，支持关闭按钮、Esc 和点击遮罩关闭。
- 投稿插件：标签“用户分享”。展示作者简介、操作说明、最多三张介绍图、下载次数和定价。详情确认后安装，支持再次下载、更新、卸载。社区插件不会自动获得助手绑定的坚果 API Key。
- 分享：填写名称、作者、简介、操作说明，上传 ZIP 和可选介绍图，设定 1～100 整数点下载价。必须声明是否需要服务授权；如需要，应介绍所用 API / 服务、申请和配置方法。所有投稿必须说明服务使用如何计费（免费和离线也需明确说明）。授权说明不是实际凭证，不要填写真实 API Key。下载费用和服务费用分别计算。支持编辑原记录并重新审核、结束分享；编辑和结束分享均保留作品编号、累计下载、收入和已购授权。编辑时可不上传 ZIP/图片以保留原内容，提交空图片数组则移除原图。
- 卡片底行左侧只显示“X 点”，右侧显示“↓ X 次”，卡片顶部为名称和用户分享标签，中间为简介，作者和版本在同一行，服务授权与查看/已安装操作排在下一行。第一张介绍图作为 16:9 封面，详情保留全部图片。鼠标移入卡片上移 4px，图片放大至 1.045；减少动画设置下关闭动画。服务端缓存压缩封面，兼容已有介绍图，已保存的图片无需重新上传；如果原记录没有图片数据，须编辑并重新选择介绍图。
- 技能页面分为“在线公共技能”和“用户分享技能”。原公共目录、分类与安装保留；用户分享独立展示，有自己的我要分享入口、搜索、审核和收入。技能 ZIP 根目录（或单层包装目录）需包含 SKILL.md，YAML 头部包含英文 name 与 description；支持 scripts/references/assets 等资源。服务端复用分享、审核、交易逻辑，以 kind=skill 区分，下载后安装至 Codex skills 目录，不混入公共目录或插件市场。

## 服务端

源码在 `F:/development/taobao/nut-server`。

| 请求 | 用途 |
| --- | --- |
| GET `/api/v1/codex-plugins?q=&sort=downloads&page=0&kind=plugin` | 公开已审核列表，每页 20 条；kind=plugin/skill，默认 plugin；返回压缩 coverImage |
| GET `/api/v1/codex-plugins/{id}` | 公开已审核详情，包括操作说明和介绍图 |
| POST `/api/v1/codex-plugins/mine` | JSON `kami,kind`；当前账户指定类型投稿与收益 |
| POST `/api/v1/codex-plugins/upload` | JSON `kami,name,author,description,instructions,price,requiresAuthorization,authorizationDetails,billingDetails,archiveBase64,screenshots` |
| POST `/api/v1/codex-plugins/{id}/download` | JSON `kami,expectedPrice`；购买/已购下载，返回 ZIP Base64 和 SHA-256 |
| POST `/api/v1/codex-plugins/{id}/edit` | JSON `kami`；作者获取完整编辑信息；upload 带原 id 和 kind 保存并重新审核 |
| POST `/api/v1/codex-plugins/{id}/end` | JSON `kami`；仅作者可结束分享，不删除交易和历史统计 |
| GET `/api/v1/admin/codex-plugins?status=pending&kind=plugin` | 管理列表，可筛 pending/published/rejected/withdrawn |
| GET `/api/v1/admin/codex-plugins/{id}/content` | 只读文件预览、介绍图、简介和说明 |
| POST `/api/v1/admin/codex-plugins/{id}/review` | JSON `action:approve/reject/withdraw,note` |

后台 `jg_panel_9527.html` 的“插件分享审核”仅列出 kind=plugin，“Codex 技能审核”仅列出 kind=skill；详情、自动审核和批准后刷新均使用对应分区。历史技能投稿仍可在折叠区域查看。审核接口沿用管理员 Session 和 `X-CSRF-Token`，匿名和缺少 CSRF 均被拒绝。

`requiresAuthorization` 必须为 Boolean，`authorizationDetails` 在需要授权时必填，`billingDetails` 始终必填；两项说明最多 3000 字。后台审核和公开详情均展示这些说明。历史投稿缺少声明时显示“授权信息待补充”，不默认宣称无需授权。

首次下载的扣款、作者全额入账、双向账本、购买授权和下载计数在同一个数据库事务完成。调用平台现有余额扣除逻辑，考虑生图预留金额。文件存在性和哈希校验在扣费前完成。按余额归属账户去重，同账户不同 API Key 不会重复支付；作者自己下载不计费、不增加下载计数。下载计数表示成功获得下载授权的独立购买账户数，重复下载不增加。

SQLite/MySQL 启动时自动新建 `codex_plugin_shares`、`codex_plugin_purchases`，为旧插件表自动补充授权与计费说明列，保留投稿与购买记录；不修改旧技能市场的表或价格。ZIP 存储于配置的 `app.data-dir/codex-plugin-market`。服务器迁移或备份需同时保存数据库和此目录。

2026-11-11：启动迁移同时检查已有 MySQL 表的 `screenshots` 和 `cover_image` 字段类型，将旧 TEXT 字段扩容为 LONGTEXT，避免大图编辑保存超出 64 KiB；重复启动不再修改已升级字段。新增真实大图编辑与购买记录保留测试，以及旧表迁移检查。保存失败的服务器日志仅记录投稿编号、图片数量、异常类型、SQLState 和数据库错误码，不记录凭证、SQL 参数或图片内容。客户端无须更改；部署更新后的后端 JAR 才会执行此迁移。线上报错是否由字段容量引起，仍需对应服务器日志确认。

ZIP 最大 10 MB，解压最大 20 MB，最多 400 项，每个文件最大 2 MB；包根或单个外层目录内须有 `.codex-plugin/plugin.json`。重新打包为普通文件，拒绝路径穿越、Windows 保留名、大小写重名和文件/目录冲突。助手接受最多三张 PNG/JPEG 原图，每张最多 20 MB，上传前自动压缩至 1 MB 以内，并展示图片预览和错误。图片处理失败时阻止提交，避免投稿成功却丢失图片。服务端仅接受真实 PNG/JPEG，每张最多 1 MB、边长不超过 6000 像素。请求体最多 20 MB；部署时确认反向代理允许该上传路径至少 20 MB 的 JSON 请求。

原生安装前检查 Codex 和可写目录，安装时复核 SHA-256 与 ZIP，使用每份作品独立的 `jg-community-{id}` marketplace/cache。配置写入采用现有备份与回滚方式，不执行投稿代码。下载成功但本机安装失败，可重试且不会再次扣点。用户卸载保留服务器购买记录。

## 构建、验证与上线

- `npm run build`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib plugin --no-fail-fast`
- `node scripts/check-account-wechat-usage.cjs`
- `node scripts/check-image-plugin.cjs`：隔离用户目录，通过实际 Codex CLI 和发布 EXE 检查插件识别与 MCP。
- 后台：`mvn -Dtest=CodexPluginMarketplaceServiceTest,CodexPluginMarketplaceHttpTest,CodexSkillMarketplaceServiceTest,SecurityConfigTest package`
- Windows：`npm run tauri -- build`，然后 `powershell -ExecutionPolicy Bypass -File scripts/package-portable-win.ps1`，沿用原 EXE/MSI 路径，不生成 ZIP。

完成本地构建不等于线上 API 已部署。服务端程序需要更新后，用户投稿、审核和付费下载才能在线使用。服务未开放时，助手给出友好提示，原技能和两个内置插件仍可使用。

## 线上未开放提示的排查

2026-11-10 实测 `GET https://api.jianguo.ltd/api/v1/codex-plugins` 返回 HTTP 404，JSON `code: ERR_NOT_FOUND`。助手 Rust 请求层在 404 时显示“插件市场服务尚未开放…”。空列表应返回 HTTP 200，`data.items: []`，不会显示该错误。

本地构建产物为 `F:/development/taobao/nut-server/target/nut-server-1.0.1.jar`。应按现有生产发布流程备份并更新实际承接 `api.jianguo.ltd` 请求的后端服务，保留数据库和 `app.data-dir/codex-plugin-market`，检查反向代理路由和 20 MB 上传大小。仅更新助手 EXE/MSI 不会发布服务器接口。

部署后验收：公开列表返回 200；登录管理员可进入 现有管理页的“插件分享审核”和“Codex 技能审核”；测试投稿在批准前不公开，批准后可见；测试下载的扣款与作者全额入账正确且重复下载不扣费。只有线上验收完成才能将插件市场标为已上线。本次未执行线上部署。
