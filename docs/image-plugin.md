# 文生图与位图转矢量图插件

助手侧栏统一为“技能库”，通过“技能”“插件市场”“我的分享”切换，原有技能目录保留。文生图、位图转矢量图标注“非分享”；社区作者投稿的插件标注“用户分享”。内置插件卡片默认只显示名称、简介、安装状态和操作，详细使用说明与费用默认折叠。免费安装，生图请求按坚果平台所选模型价格计费。须在对话中明确指定生图模型，插件不选择默认模型。内部插件 ID 保持不变，兼容既有安装和任务记录。

用户安装后重新启动 Codex，在聊天框输入 `@`，选择对应的插件名，再输入具体的生图或转矢量要求并发送。插件页分别提供操作说明与示例；账户、费用、等待与图片保存说明使用面向普通用户的表述，不展示接口地址。

点击“安装到 Codex”会使用用户此电脑上助手当前登录账户的坚果 API Key；账号密码登录但没有 Key 时复用现有的账户 Key 创建逻辑。界面不显示凭证。两个插件共用系统加密存储，每次调用读取当前绑定 Key；重新登录、添加 Key、刷新账户资料时同步已安装插件。切换到没有 Key 的账户会清除旧凭证，防止误用前一个账户余额。安装后按钮变为“更新插件”，更新所选插件的运行程序、说明与清单，同时同步当前账户凭证；不会替换另一插件已安装的程序。安装或更新后重新启动 Codex。

已安装插件提供“卸载”。确认后移除该插件的 Codex 配置条目和独立插件缓存；失败时恢复配置/文件，支持重试和重新安装。保留当前账户凭证、另一插件、原任务记录及已保存的图片或 SVG。共享运行程序和 marketplace 安装来源保留，以便其他插件继续使用或以后重新安装。Codex 重启后移除会话内已加载的插件。

插件由标准 `.codex-plugin/plugin.json`、中文技能与 stdio MCP 工具组成，在独立 `jianguo-assistant` 本地 marketplace 注册并写入 Codex 插件缓存；不依赖 Codex++、Python 或 Node。运行程序复制到 Codex 用户目录，移动或关闭助手不影响插件。只在用户点击安装时修改 Codex 配置，保留供应商、模型和其他插件设置。

Windows 凭证使用 DPAPI 加密，macOS 使用系统钥匙串。密钥不放在插件清单、MCP 参数或对话中；POST 仅访问 `https://api.jianguo.ltd/api/v1/generate`，密钥在 JSON `kami` 字段，不使用 Bearer。提交请求禁止自动重试及重定向，避免重复计费或转发密钥。

工具包括 `list_image_models`、`get_image_pricing`、`generate_image` 与 `get_image_result`。生图异步提交后返回本地 `jobId`；查询处理 HTTP 202 等待、410 失败、PNG 字节以及任务或提交直接返回的图片 URL。等待期间仅查询原任务。参考图支持 PNG/JPEG/WebP 本机绝对路径或 data URL，不直接转发远程参考图地址。参考图会上传到平台处理。

图片下载不携带凭证，支持公开 HTTPS 地址，以及平台当前使用的 `http://download.jianguo.ltd` 结果存储地址；该 HTTP 例外仅用于下载文件，API 与凭证仍只使用 HTTPS。逐跳检查重定向，拒绝其他 HTTP 主机、带用户名密码的地址及非标准 HTTP 端口。下载先缓存，再保存到项目，保留实际图片格式，不覆盖不同内容的文件；重复查询直接复用本机结果。一旦取得结果 URL，下载重试直接复用该地址，避免再次访问可能一次性消费结果的任务接口。任务元数据不保存提示词或凭证，插件自身不会启动付费生成来验证安装。

验证：

- `npm run build`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib image_plugin::tests`
- `node scripts/check-account-wechat-usage.cjs`：模拟账户接口验证安装、失败恢复、导航及深浅主题。
- `node scripts/check-image-plugin.cjs`：使用实际 Codex CLI 和打包后的 EXE，在隔离用户目录验证插件识别及 stdio MCP。可通过 `CODEX_TEST_PLUGIN_CLI` 与 `CODEX_TEST_PLUGIN_HOST` 指定测试程序。

转矢量工具为 `get_vectorize_pricing`、`vectorize_image`、`get_vectorize_result`。提交 `POST /api/v1/vectorize`，JSON 包含当前绑定的 `kami`、本机图片的 `imageBase64`、`format=svg`、`engine=vectorizer-ai`、`fileName` 与可选 `sourceWidth/sourceHeight`。支持当前服务器直接返回 SVG 字节、JSON URL 以及任务编号三种响应。

任务模式优先 `GET /api/v1/task-status/events/{taskId}`；SSE 断线/不支持时使用 `GET /api/v1/task-status/{taskId}` 查询同一任务。识别 pending/running/completed/failed，以及完成结果嵌套在 data 内的 url。previewBase64 仅为预览，不充当矢量结果。SVG 缓存后保存到新文件；下载/保存失败可查询原 jobId 恢复，不自动重新提交付费转换。不执行或自动打开 SVG。

插件测试：`cargo test --manifest-path src-tauri/Cargo.toml --lib plugin::`；两个插件共 15 项原生测试，覆盖接口字段、当前 Key、同步 SVG、SSE 状态、断线回退、失败/未确认提交、本机缓存恢复、文件冲突、共存注册、账户凭证切换、保存结果地址后的恢复及独立更新/卸载/重装。界面测试覆盖两个插件的名称、默认折叠、独立安装、更新、卸载确认/失败恢复和当前账户 Key 传递。集成脚本使用实际 Codex CLI 验证两插件的本地 marketplace/配置/缓存和发布 EXE 的两种 MCP 启动模式。

真实链路检查使用 `node scripts/check-image-plugin-live.cjs --job <已有jobId>` 恢复结果，不重新付费。只有显式提供 `--generate --model <模型ID> --output <新文件>` 才会提交一次付费测试生图。工具从已安装的加密存储读取凭证，不输出 Key，检查完成文件的图片格式、非空内容与重复查询缓存。

自动验证使用模拟服务，不消耗真实账户点数；真实出图和转矢量计费需另行验收。
