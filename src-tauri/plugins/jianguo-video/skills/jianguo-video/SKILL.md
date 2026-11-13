---
name: jianguo-video
description: 通过坚果平台，使用用户明确指定的视频模型生成 MP4 视频。支持 MiniMax H3、Agnes Video 2.5、Wan 3.0，可选择横屏或竖屏、分辨率、时长、参考图与首尾帧。用户未指定视频模型时先询问，不能用列表首项代替。
---

# 文生视频

使用本插件的 MCP 工具提交和查询坚果视频任务，不调用其他供应商的视频接口。

1. 用户必须明确指定视频模型 ID：`minimax-h3`、`agnes-video-2.5` 或 `wan-3.0`。未指定时先询问；需要比较能力或价格时调用 `list_video_models`，保留接口返回顺序，不把第一项当作默认模型。
2. 提交前确认提示词、画面比例和大致时长。用户说“横屏”时可用 `landscape` 或 `16:9`，说“竖屏”时可用 `portrait` 或 `9:16`；具体可选值以 `list_video_models` 为准。视频按秒计费，提交即可能预留额度，生成成功后扣费，失败或违规不扣费。余额或价格问题以平台返回为准。
3. 组织用户要求后调用 `generate_video`，传入 `model`、`prompt`、新的 `.mp4` 本机绝对路径 `outputFile`，再按需传入 `resolution`、`aspectRatio`、`seconds`、参考图、参考音频、首尾帧、`seed` 或 `promptSkill`。本机参考素材会上传到坚果平台处理。
4. 保存返回的 `jobId` 和 `taskId`，用 `get_video_result` 查询同一个任务。`queued`、`in_progress` 时按返回的间隔等待后继续查询，生成可能需要数分钟。网络超时、断线或状态暂时不可用时，不再次调用 `generate_video`；重复提交会产生新的预留和费用。
5. 完成后使用返回的本机绝对 `filePath` 告知用户。不要声称视频已经生成，直到工具返回 `completed` 和真实文件路径。下载中断时继续查询原任务，插件会复用已缓存结果。
6. 凭证由插件读取本机的安全存储，不询问、打印或写入 API Key。提示凭证缺失时，让用户到 Codex 使用助手的“技能与插件 → 插件库”中更新该插件。
