---
name: jianguo-vectorize
description: 使用位图转矢量图插件，将用户指定的本机 PNG、JPEG、WebP 图片转换为 SVG 矢量图。当用户要求位图转矢量、描摹标志、图片转 SVG 时使用。
---

# 位图转矢量图

使用此插件提供的 MCP 工具完成转换，不自行编造 SVG 来代替转换结果。

1. 明确用户的输入图片与输出位置。输入使用本机绝对路径；输出使用新的 `.svg` 绝对路径，不覆盖已有文件。图片会上传到坚果平台处理，转换按平台价格扣点。没有明确转换意图时先询问，不自动启动付费任务。
2. 如用户询问价格，用 `get_vectorize_pricing` 查询 `tool.vectorize.vectorizer-ai`，不得把安装免费说成转换免费。
3. 调用 `vectorize_image`，传 `imageFile`、`outputFile`；已知原图尺寸时可传 `sourceWidth`、`sourceHeight`。插件负责读取图片并使用系统安全存储中的当前坚果 API Key，固定 `engine=vectorizer-ai`、`format=svg`。不要要求用户把 Key 发到对话里，不读取或展示凭证文件。
4. 保存返回的 `jobId`，调用 `get_vectorize_result` 继续查询。`submitting`、`pending`、`running` 时按返回的间隔等待后再查询同一任务，最长等待约十分钟。查询优先订阅 SSE，断线后使用状态查询接口恢复。支持同步 SVG 和异步任务两种服务响应。
5. `completed` 后向用户提供 `filePath`。这是矢量 SVG；`previewBase64` 只是预览，不得冒充 SVG 结果。
6. 提交超时或结果未确认时，不自动重新调用 `vectorize_image`，以免重复扣点。保留原 jobId，提示查询平台记录。查询或保存失败可继续查询原任务；终态 `failed` 要说明失败原因，再次付费转换必须由用户决定。

插件仅连接坚果平台，输出是文件而非待执行代码。不执行 SVG 中的脚本或外部资源；不自动在浏览器中打开返回的 SVG。
