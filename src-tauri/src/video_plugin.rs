//! Paid video generation is persisted before submission and never resubmitted automatically.
use super::image_plugin::{
    BuiltinKind, Credentials, credentials, http_client, image_mime, redact, redact_value,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) const MANIFEST: &str =
    include_str!("../plugins/jianguo-video/.codex-plugin/plugin.json");
pub(super) const SKILL: &str =
    include_str!("../plugins/jianguo-video/skills/jianguo-video/SKILL.md");
pub(super) const AGENT: &str =
    include_str!("../plugins/jianguo-video/skills/jianguo-video/agents/openai.yaml");
const BASE_URL: &str = "https://api.jianguo.ltd";
const MAX_API_RESPONSE: u64 = 4 * 1024 * 1024;
const MAX_MEDIA_TOTAL: usize = 32 * 1024 * 1024;
const MAX_VIDEO_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_PROMPT_CHARS: usize = 20_000;

#[tauri::command]
pub async fn video_plugin_status() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        super::image_plugin::plugin_status_kind_at(&super::codex_home_dir()?, BuiltinKind::Video)
    })
    .await
    .map_err(|_| "读取文生视频插件状态失败")?
}

#[tauri::command]
pub async fn install_video_plugin(api_key: String, base_url: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = super::PROVIDER_CONFIG_LOCK
            .lock()
            .map_err(|_| "配置正在更新，请稍后重试")?;
        #[cfg(target_os = "windows")]
        let installed = super::detect_codex_windows().installed;
        #[cfg(not(target_os = "windows"))]
        let installed = super::detect_codex_macos().installed;
        if !installed {
            return Err("请先安装 Codex，再安装文生视频插件".into());
        }
        super::image_plugin::install_plugin_kind_at(
            &super::codex_home_dir()?,
            &std::env::current_exe().map_err(|_| "无法定位插件运行程序")?,
            &base_url,
            &api_key,
            BuiltinKind::Video,
        )
    })
    .await
    .map_err(|_| "安装文生视频插件失败")?
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    id: String,
    model: String,
    output_file: PathBuf,
    state: String,
    task_id: Option<String>,
    url: Option<String>,
    error: Option<String>,
    cost: Option<f64>,
    submitted_at: i64,
}

fn jobs_root(home: &Path) -> PathBuf {
    home.join(".usage-assistant/video-plugin/jobs")
}

fn job_path(home: &Path, id: &str) -> Result<PathBuf, String> {
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("视频任务编号无效".into());
    }
    Ok(jobs_root(home).join(format!("{id}.json")))
}

fn save(home: &Path, job: &Job) -> Result<(), String> {
    super::provider_config::commit(vec![(
        job_path(home, &job.id)?,
        serde_json::to_vec(job).map_err(|_| "视频任务记录编码失败")?,
    )])
}

fn reply(job: &Job) -> Value {
    let pending = matches!(job.state.as_str(), "submitting" | "queued" | "in_progress");
    json!({
        "jobId":job.id,
        "taskId":job.task_id,
        "status":job.state,
        "model":job.model,
        "cost":job.cost,
        "filePath":if job.state == "completed" { Some(&job.output_file) } else { None },
        "message":job.error,
        "retryAfterSeconds":if pending { Some(4) } else { None }
    })
}

fn extension(value: &Value, name: &str) -> Result<Option<String>, String> {
    let Some(value) = value.get(name) else {
        return Ok(None);
    };
    let text = value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("{name} 需为非空字符串"))?;
    if text.len() > 64 {
        return Err(format!("{name} 过长"));
    }
    Ok(Some(text.to_owned()))
}

#[derive(Clone, Copy)]
enum MediaKind {
    Image,
    Audio,
}

fn audio_mime(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"ID3")
        || data.starts_with(&[0xff, 0xfb])
        || data.starts_with(&[0xff, 0xf3])
    {
        Some("audio/mpeg")
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WAVE") {
        Some("audio/wav")
    } else if data.starts_with(b"OggS") {
        Some("audio/ogg")
    } else if data.get(4..8) == Some(b"ftyp") {
        Some("audio/mp4")
    } else {
        None
    }
}

fn media_mime(data: &[u8], kind: MediaKind) -> Option<&'static str> {
    match kind {
        MediaKind::Image => image_mime(data),
        MediaKind::Audio => audio_mime(data),
    }
}

fn decode_media_data_url(text: &str) -> Option<(Vec<u8>, usize)> {
    let (header, body) = text.split_once(',')?;
    if !header.starts_with("data:") || !header.ends_with(";base64") {
        return None;
    }
    if body.len() > ((MAX_MEDIA_TOTAL * 4 / 3) + 8) {
        return None;
    }
    let data = base64::engine::general_purpose::STANDARD
        .decode(body)
        .ok()?;
    let size = data.len();
    Some((data, size))
}

fn media_value(value: &Value, kind: MediaKind, total: &mut usize) -> Result<String, String> {
    let text = value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("参考素材需为文本路径、data URL 或公网 URL")?;
    if text.starts_with("http://") || text.starts_with("https://") {
        let url = reqwest::Url::parse(text).map_err(|_| "参考素材 URL 无效")?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err("参考素材 URL 不得包含账号信息".into());
        }
        return Ok(url.to_string());
    }

    let (data, raw_size) = if text.starts_with("data:") {
        decode_media_data_url(text).ok_or("参考素材 data URL 无效或过大")?
    } else {
        let path = Path::new(text);
        if path.is_absolute() {
            let mut data = Vec::new();
            fs::File::open(path)
                .map_err(|_| "无法读取参考素材")?
                .take((MAX_MEDIA_TOTAL + 1) as u64)
                .read_to_end(&mut data)
                .map_err(|_| "读取参考素材失败")?;
            let size = data.len();
            (data, size)
        } else {
            let data = base64::engine::general_purpose::STANDARD
                .decode(text)
                .map_err(|_| "本机参考素材请使用绝对路径或合法 data URL")?;
            let size = data.len();
            (data, size)
        }
    };
    *total += raw_size;
    if raw_size > MAX_MEDIA_TOTAL || *total > MAX_MEDIA_TOTAL {
        return Err("参考素材合计最多 32 MB".into());
    }
    let mime = media_mime(&data, kind).ok_or(match kind {
        MediaKind::Image => "参考图需为有效的 PNG、JPEG 或 WebP 图片",
        MediaKind::Audio => "参考音频需为 MP3、WAV、M4A 或 OGG",
    })?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(data)
    ))
}

fn media_array(
    args: &Value,
    name: &str,
    max: usize,
    kind: MediaKind,
    total: &mut usize,
) -> Result<Option<Vec<String>>, String> {
    let Some(values) = args.get(name) else {
        return Ok(None);
    };
    let values = values
        .as_array()
        .filter(|values| values.len() <= max)
        .ok_or_else(|| format!("{name} 最多 {max} 项"))?;
    let mut result = Vec::new();
    for value in values {
        result.push(media_value(value, kind, total)?);
    }
    Ok(Some(result))
}

fn request_body(args: &Value) -> Result<Value, String> {
    let model = args["model"]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("请先明确指定视频模型，不会自动选择默认模型")?;
    if !matches!(model, "minimax-h3" | "agnes-video-2.5" | "wan-3.0") {
        return Err("视频模型仅支持 minimax-h3、agnes-video-2.5 或 wan-3.0".into());
    }
    let prompt = args["prompt"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or("请填写视频提示词")?;
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(format!("视频提示词最多 {MAX_PROMPT_CHARS} 个字符"));
    }

    let mut body = json!({"model":model,"prompt":prompt});
    if let Some(value) = extension(args, "resolution")? {
        body["resolution"] = json!(value);
    }
    if let Some(value) = extension(args, "aspectRatio")? {
        body["aspect_ratio"] = json!(value);
    }
    if let Some(value) = extension(args, "size")? {
        body["size"] = json!(value);
    }
    for name in ["seconds", "duration"] {
        if let Some(value) = args.get(name) {
            let seconds = value
                .as_i64()
                .filter(|value| (1..=60).contains(value))
                .ok_or("视频时长需为 1 至 60 的整数秒")?;
            body["seconds"] = json!(seconds);
            break;
        }
    }
    if let Some(seed) = args.get("seed") {
        let seed = seed
            .as_i64()
            .filter(|value| *value >= 0)
            .ok_or("seed 需为非负整数")?;
        body["seed"] = json!(seed);
    }
    if let Some(value) = args.get("promptSkill") {
        body["prompt_skill"] = json!(value.as_bool().ok_or("promptSkill 需为布尔值")?);
    }

    let mut total = 0;
    if let Some(images) = media_array(args, "images", 10, MediaKind::Image, &mut total)? {
        if !images.is_empty() {
            body["images"] = json!(images);
        }
    }
    if let Some(audios) = media_array(args, "audios", 5, MediaKind::Audio, &mut total)? {
        if !audios.is_empty() {
            body["audios"] = json!(audios);
        }
    }
    for (field, target) in [("firstFrame", "first_frame"), ("lastFrame", "last_frame")] {
        if let Some(value) = args.get(field) {
            body[target] = json!(media_value(value, MediaKind::Image, &mut total)?);
        }
    }
    if body.get("first_frame").is_some() != body.get("last_frame").is_some() {
        return Err("首帧和尾帧必须同时提供".into());
    }
    if body.get("first_frame").is_some()
        && (body.get("images").is_some() || body.get("audios").is_some())
    {
        return Err("首尾帧不能与参考图或参考音频同时使用".into());
    }
    Ok(body)
}

fn api_json(response: reqwest::blocking::Response) -> Result<Value, String> {
    let status = response.status();
    let mut data = Vec::new();
    response
        .take(MAX_API_RESPONSE + 1)
        .read_to_end(&mut data)
        .map_err(|_| "读取视频接口响应失败，请查询原任务，不要重复提交")?;
    if data.len() as u64 > MAX_API_RESPONSE {
        return Err("视频接口响应超出大小限制".into());
    }
    let value: Value = serde_json::from_slice(&data)
        .map_err(|_| format!("视频服务返回了无法识别的响应（HTTP {status}）"))?;
    if !status.is_success() || value.get("error").is_some() {
        let message = value["error"]["message"]
            .as_str()
            .or_else(|| value["error"].as_str())
            .or_else(|| value["message"].as_str())
            .unwrap_or("视频接口请求失败");
        return Err(format!("{message}（HTTP {status}）"));
    }
    Ok(value)
}

fn apply_status(job: &mut Job, value: &Value) -> Result<(), String> {
    if let Some(id) = value["id"].as_str().filter(|id| !id.is_empty()) {
        if let Some(expected) = job.task_id.as_deref() {
            if id != expected {
                return Err("接口返回了不同的任务 ID，已停止处理原任务".into());
            }
        } else {
            job.task_id = Some(id.to_owned());
        }
    }
    if let Some(cost) = value["cost"].as_f64() {
        job.cost = Some(cost);
    }
    if let Some(url) = value["url"].as_str().filter(|url| !url.is_empty()) {
        job.url = Some(url.to_owned());
    } else if let Some(url) = value["results"]
        .as_array()
        .and_then(|items| items.iter().find_map(|item| item["url"].as_str()))
        .filter(|url| !url.is_empty())
    {
        job.url = Some(url.to_owned());
    }
    match value["status"].as_str() {
        Some("completed") => {
            job.state = "completed".into();
            job.error = None;
        }
        Some("failed") => {
            job.state = "failed".into();
            job.error = Some(
                value["error"]["message"]
                    .as_str()
                    .or_else(|| value["error"].as_str())
                    .unwrap_or("视频生成失败")
                    .into(),
            );
        }
        Some("in_progress") => job.state = "in_progress".into(),
        Some("queued") => job.state = "queued".into(),
        Some(status) if !status.is_empty() => job.state = status.into(),
        _ => {}
    }
    Ok(())
}

fn submit(
    home: &Path,
    job: &mut Job,
    body: &Value,
    credential: &Credentials,
    client: &reqwest::blocking::Client,
) -> Result<(), String> {
    let response = client
        .post(format!("{}/v1/videos", credential.base_url))
        .bearer_auth(&credential.api_key)
        .json(body)
        .send()
        .map_err(|_| "提交结果未确认，请勿重新生成，以免重复扣点；请检查平台任务记录")?;
    let status = response.status();
    let mut data = Vec::new();
    response
        .take(MAX_API_RESPONSE + 1)
        .read_to_end(&mut data)
        .map_err(|_| "提交响应中断，无法确认任务；请勿重新生成，请检查平台记录")?;
    if data.len() as u64 > MAX_API_RESPONSE {
        return Err("视频提交响应超出大小限制，请检查平台记录".into());
    }
    let value: Value = serde_json::from_slice(&data).map_err(|_| {
        format!("视频提交返回未知响应（HTTP {status}），请检查平台记录，不要自动重新生成")
    })?;
    if !status.is_success() || value.get("error").is_some() {
        job.state = "failed".into();
        job.error = Some(
            value["error"]["message"]
                .as_str()
                .or_else(|| value["error"].as_str())
                .or_else(|| value["message"].as_str())
                .unwrap_or("视频任务创建失败")
                .into(),
        );
        return save(home, job);
    }
    let id = value["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("服务未返回视频任务 ID，请检查平台记录，不要重复提交")?;
    job.task_id = Some(id.to_owned());
    apply_status(job, &value)?;
    save(home, job)
}

fn start(home: &Path, args: &Value) -> Result<Value, String> {
    let credential = credentials(home)?;
    let body = request_body(args)?;
    let output = PathBuf::from(
        args["outputFile"]
            .as_str()
            .ok_or("请指定 MP4 保存位置 outputFile（绝对路径）")?,
    );
    if !output.is_absolute()
        || output.file_name().is_none()
        || output
            .extension()
            .and_then(|value| value.to_str())
            .is_none_or(|value| !value.eq_ignore_ascii_case("mp4"))
        || output.exists()
    {
        return Err("视频保存路径需为尚不存在的 .mp4 绝对路径，插件不会覆盖已有文件".into());
    }
    fs::create_dir_all(output.parent().ok_or("视频保存目录无效")?)
        .map_err(|_| "无法创建视频保存目录")?;

    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| "无法创建视频任务编号")?;
    let id = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut job = Job {
        id,
        model: body["model"].as_str().unwrap_or_default().to_owned(),
        output_file: output,
        state: "submitting".into(),
        task_id: None,
        url: None,
        error: None,
        cost: None,
        submitted_at: chrono::Utc::now().timestamp(),
    };
    // This record must exist before the paid request is sent.
    save(home, &job)?;
    let client = http_client()?;
    if let Err(error) = submit(home, &mut job, &body, &credential, &client) {
        job.state = "submission-unconfirmed".into();
        job.error = Some(redact(&error, &credential.api_key));
        let _ = save(home, &job);
    }
    let mut value = reply(&job);
    redact_value(&mut value, &credential.api_key);
    Ok(value)
}

fn task_url(base: &str, task: &str, content: bool) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base).map_err(|_| "坚果地址无效")?;
    let mut segments = url.path_segments_mut().map_err(|_| "坚果地址无效")?;
    segments.extend(["v1", "videos", task]);
    if content {
        segments.push("content");
    }
    drop(segments);
    Ok(url)
}

fn query_status(
    home: &Path,
    job: &mut Job,
    client: &reqwest::blocking::Client,
    credential: &Credentials,
) -> Result<(), String> {
    let task = job.task_id.as_deref().ok_or("原任务尚未返回任务 ID")?;
    let response = client
        .get(task_url(&credential.base_url, task, false)?)
        .bearer_auth(&credential.api_key)
        .send()
        .map_err(|_| "查询视频任务暂时失败，请继续查询原任务，不要重新生成")?;
    apply_status(job, &api_json(response)?)?;
    save(home, job)
}

fn mp4(data: &[u8]) -> bool {
    data.len() >= 12
        && u32::from_be_bytes([data[0], data[1], data[2], data[3]]) >= 8
        && data.get(4..8) == Some(b"ftyp")
}

fn valid_mp4_file(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut prefix = [0u8; 64];
    let Ok(size) = file.read(&mut prefix) else {
        return false;
    };
    mp4(&prefix[..size])
}

fn save_response_to_cache(
    home: &Path,
    job: &Job,
    mut response: reqwest::blocking::Response,
) -> Result<PathBuf, String> {
    let cache = job_path(home, &job.id)?.with_extension("mp4");
    if valid_mp4_file(&cache) {
        return Ok(cache);
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_VIDEO_BYTES)
    {
        return Err("视频文件超过 2 GB，未保存".into());
    }
    let part = job_path(home, &job.id)?.with_extension("mp4.part");
    let _ = fs::remove_file(&part);
    fs::create_dir_all(part.parent().ok_or("视频缓存目录无效")?)
        .map_err(|_| "无法创建视频缓存目录")?;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&part)
        .map_err(|_| "无法写入视频缓存")?;
    let mut prefix = Vec::with_capacity(64);
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = response
            .read(&mut buffer)
            .map_err(|_| "视频下载中断，请继续查询原任务")?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_VIDEO_BYTES {
            drop(file);
            let _ = fs::remove_file(&part);
            return Err("视频文件超过 2 GB，未保存".into());
        }
        if prefix.len() < 64 {
            let take = (64 - prefix.len()).min(count);
            prefix.extend_from_slice(&buffer[..take]);
        }
        if file.write_all(&buffer[..count]).is_err() {
            drop(file);
            let _ = fs::remove_file(&part);
            return Err("保存视频失败，请检查磁盘空间后继续查询原任务".into());
        }
    }
    if file.sync_all().is_err() {
        drop(file);
        let _ = fs::remove_file(&part);
        return Err("保存视频失败，请检查磁盘空间".into());
    }
    if !mp4(&prefix) {
        drop(file);
        let _ = fs::remove_file(&part);
        return Err("下载结果不是有效的 MP4 视频，请继续查询原任务".into());
    }
    fs::rename(&part, &cache).map_err(|_| "无法完成视频缓存，请继续查询原任务")?;
    Ok(cache)
}

fn public_download_url(url: &reqwest::Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    if url.scheme() == "http" {
        return url.host_str() == Some("download.jianguo.ltd") && url.port().is_none();
    }
    if url.scheme() != "https" {
        return false;
    }
    match url.host_str() {
        Some(host) => match host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V4(ip)) => {
                !ip.is_private() && !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified()
            }
            Ok(std::net::IpAddr::V6(ip)) => {
                !ip.is_loopback()
                    && !ip.is_unspecified()
                    && (ip.segments()[0] & 0xfe00) != 0xfc00
                    && (ip.segments()[0] & 0xffc0) != 0xfe80
            }
            Err(_) => {
                host.contains('.') && !host.ends_with(".localhost") && !host.ends_with(".local")
            }
        },
        None => false,
    }
}

fn download(
    home: &Path,
    job: &Job,
    credential: &Credentials,
    client: &reqwest::blocking::Client,
) -> Result<PathBuf, String> {
    let task = job.task_id.as_deref().ok_or("任务没有可下载的视频 ID")?;
    let mut url = task_url(&credential.base_url, task, true)?;
    let storage_client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法初始化视频下载连接")?;
    for attempt in 0..=5 {
        let request_client = if url.scheme() == "http" {
            &storage_client
        } else {
            client
        };
        let mut request = request_client
            .get(url.clone())
            .header("Accept", "video/mp4");
        if attempt == 0 {
            request = request.bearer_auth(&credential.api_key);
        }
        let response = request
            .send()
            .map_err(|_| "视频下载暂时失败，请继续查询原任务")?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or("视频重定向地址缺失")?;
            url = url.join(location).map_err(|_| "视频重定向地址无效")?;
            if !public_download_url(&url) {
                return Err("视频下载地址无效或不受支持".into());
            }
            continue;
        }
        if !response.status().is_success() {
            return Err(format!(
                "视频下载失败（HTTP {}），请继续查询原任务",
                response.status()
            ));
        }
        return save_response_to_cache(home, job, response);
    }
    Err("视频下载重定向次数过多".into())
}

fn finalize(home: &Path, job: &mut Job, cache: &Path) -> Result<Value, String> {
    if !valid_mp4_file(cache) {
        return Err("视频缓存不是有效的 MP4，请继续查询原任务".into());
    }
    if job.output_file.exists() {
        if !valid_mp4_file(&job.output_file) {
            return Err("目标文件已存在且不是视频，原结果已缓存；请移开冲突文件后继续查询".into());
        }
    } else {
        let mut source = fs::File::open(cache).map_err(|_| "无法读取已缓存的视频")?;
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&job.output_file)
            .map_err(|_| "无法保存视频，原结果已缓存；请检查输出目录后继续查询")?;
        if std::io::copy(&mut source, &mut output).is_err() || output.sync_all().is_err() {
            drop(output);
            let _ = fs::remove_file(&job.output_file);
            return Err("保存视频失败，原结果已缓存，请检查磁盘空间后继续查询".into());
        }
    }
    job.state = "completed".into();
    job.error = None;
    save(home, job)?;
    Ok(reply(job))
}

fn result(
    home: &Path,
    id: &str,
    credential: &Credentials,
    client: &reqwest::blocking::Client,
) -> Result<Value, String> {
    let mut job: Job = serde_json::from_slice(
        &fs::read(job_path(home, id)?).map_err(|_| "未找到此视频任务，请核对 jobId")?,
    )
    .map_err(|_| "视频任务记录格式无效")?;
    if job.state == "completed" && job.output_file.is_file() {
        return Ok(reply(&job));
    }
    if matches!(job.state.as_str(), "failed" | "submission-unconfirmed") {
        return Ok(reply(&job));
    }
    let cache = job_path(home, id)?.with_extension("mp4");
    if valid_mp4_file(&cache) {
        return finalize(home, &mut job, &cache);
    }
    if job.task_id.is_none() {
        job.state = "submission-unconfirmed".into();
        job.error = Some("提交结果未确认，请检查平台任务记录，不要自动重新生成".into());
        save(home, &job)?;
        return Ok(reply(&job));
    }
    query_status(home, &mut job, client, credential)?;
    if job.state == "failed" {
        return Ok(reply(&job));
    }
    if job.state != "completed" {
        return Ok(reply(&job));
    }
    let cache = download(home, &job, credential, client)?;
    finalize(home, &mut job, &cache)
}

pub(super) fn tools() -> Value {
    json!([
        {"name":"list_video_models","description":"查询坚果文生视频模型、能力、分辨率、时长与实时按秒价格，保留接口顺序。不要自动选择列表首项。","inputSchema":{"type":"object","properties":{},"additionalProperties":false},"annotations":{"readOnlyHint":true}},
        {"name":"generate_video","description":"按用户明确指定的视频模型提交一次付费异步任务。保存原任务后用 get_video_result 查询，禁止在超时后自动重提。","inputSchema":{"type":"object","required":["model","prompt","outputFile"],"additionalProperties":false,"properties":{"model":{"type":"string","enum":["minimax-h3","agnes-video-2.5","wan-3.0"],"description":"用户明确指定的视频模型 ID，无默认值"},"prompt":{"type":"string","maxLength":20000},"outputFile":{"type":"string","description":"新 MP4 的绝对保存路径，不覆盖已有文件"},"resolution":{"type":"string","description":"例如 480p、720p、768p、1080p 或 2k，以模型列表为准"},"aspectRatio":{"type":"string","description":"例如 landscape、portrait、16:9、9:16 或 1:1，以模型列表为准"},"size":{"type":"string","description":"也可使用 1280x720 或 720p 形式"},"seconds":{"type":"integer","minimum":1,"maximum":60,"description":"视频时长（秒），以模型允许范围为准"},"images":{"type":"array","maxItems":10,"items":{"type":"string"},"description":"参考图的本机绝对路径、data URL 或公网 URL"},"audios":{"type":"array","maxItems":5,"items":{"type":"string"},"description":"参考音频的本机绝对路径、data URL 或公网 URL"},"firstFrame":{"type":"string","description":"首帧图片；必须与 lastFrame 同时提供"},"lastFrame":{"type":"string","description":"尾帧图片；必须与 firstFrame 同时提供"},"seed":{"type":"integer","minimum":0},"promptSkill":{"type":"boolean","description":"是否启用模型支持的提示词增强"}}},"annotations":{"readOnlyHint":false,"idempotentHint":false}},
        {"name":"get_video_result","description":"查询既有视频 jobId 的同一个任务。未完成时等待约 4 秒再查；完成后下载原始 MP4 并保存到指定绝对路径。","inputSchema":{"type":"object","required":["jobId"],"properties":{"jobId":{"type":"string"}},"additionalProperties":false},"annotations":{"readOnlyHint":false,"idempotentHint":true}}
    ])
}

pub(super) fn call_tool(home: &Path, name: &str, args: &Value) -> Result<Value, String> {
    if name == "list_video_models" {
        let client = http_client()?;
        return api_json(
            client
                .get(format!("{BASE_URL}/v1/videos/models"))
                .send()
                .map_err(|_| "视频模型列表暂时无法连接，请检查网络或开启网络代理")?,
        );
    }
    if name == "generate_video" {
        return start(home, args);
    }
    let credential = credentials(home)?;
    let client = http_client()?;
    let value = match name {
        "get_video_result" => result(
            home,
            args["jobId"].as_str().ok_or("缺少 jobId")?,
            &credential,
            &client,
        ),
        _ => Err("不支持的文生视频工具".into()),
    };
    value
        .map(|mut value| {
            redact_value(&mut value, &credential.api_key);
            value
        })
        .map_err(|error| redact(&error, &credential.api_key))
}

#[cfg(test)]
mod tests {
    use super::super::image_plugin::tests::{client, fixture_home, server};
    use super::*;

    fn job(home: &Path) -> Job {
        Job {
            id: "0123456789abcdef0123456789abcdef".into(),
            model: "wan-3.0".into(),
            output_file: home.join("result.mp4"),
            state: "submitting".into(),
            task_id: None,
            url: None,
            error: None,
            cost: None,
            submitted_at: chrono::Utc::now().timestamp(),
        }
    }

    fn mp4_bytes() -> Vec<u8> {
        let mut value = vec![0, 0, 0, 24];
        value.extend_from_slice(b"ftypisom");
        value.extend_from_slice(&[0; 32]);
        value
    }

    #[test]
    fn explicit_model_and_reference_media_are_required() {
        assert!(request_body(&json!({"prompt":"a cat"})).is_err());
        assert!(request_body(&json!({"model":"unknown","prompt":"a cat"})).is_err());
        assert!(request_body(&json!({"model":"wan-3.0","prompt":"a cat","images":["https://example.com/a.png"],"firstFrame":"https://example.com/a.png","lastFrame":"https://example.com/b.png"})).is_err());
        let png = super::super::image_plugin::tests::png();
        let reference = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        );
        let body = request_body(&json!({"model":"wan-3.0","prompt":"a cat","seconds":8,"aspectRatio":"横屏","resolution":"720p","images":[reference],"seed":42,"promptSkill":false})).unwrap();
        assert_eq!(body["model"], "wan-3.0");
        assert_eq!(body["seconds"], 8);
        assert_eq!(body["aspect_ratio"], "横屏");
        assert_eq!(body["seed"], 42);
        assert_eq!(body["images"].as_array().unwrap().len(), 1);
        assert!(body.get("kami").is_none());
    }

    #[test]
    fn submit_query_and_mp4_download_use_one_task_and_reuse_cache() {
        let home = fixture_home();
        let mut task = job(&home);
        save(&home, &task).unwrap();
        let video = mp4_bytes();
        let (base, handle) = server(vec![
            (200, "application/json", br#"{"id":"task-123","status":"queued","cost":21.2}"#.to_vec()),
            (200, "application/json", br#"{"id":"task-123","status":"completed","url":"https://results.example.com/video.mp4","cost":21.2}"#.to_vec()),
            (200, "video/mp4", video.clone()),
        ]);
        let credential = Credentials {
            base_url: base.clone(),
            api_key: "fixture-key".into(),
        };
        let body =
            request_body(&json!({"model":"wan-3.0","prompt":"product video","seconds":8})).unwrap();
        submit(&home, &mut task, &body, &credential, &client()).unwrap();
        assert_eq!(task.task_id.as_deref(), Some("task-123"));
        assert_eq!(task.state, "queued");
        let completed = result(&home, &task.id, &credential, &client()).unwrap();
        assert_eq!(completed["status"], "completed");
        assert_eq!(fs::read(&task.output_file).unwrap(), video);
        assert_eq!(
            result(&home, &task.id, &credential, &client()).unwrap(),
            completed
        );
        fs::remove_file(&task.output_file).unwrap();
        assert_eq!(
            result(&home, &task.id, &credential, &client()).unwrap(),
            completed
        );
        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[0].0.starts_with("POST /v1/videos "));
        assert!(requests[1].0.starts_with("GET /v1/videos/task-123 "));
        assert!(
            requests[2]
                .0
                .starts_with("GET /v1/videos/task-123/content ")
        );
        assert!(requests.iter().all(|(headers, _)| {
            headers
                .to_lowercase()
                .contains("authorization: bearer fixture-key")
        }));
        assert!(
            !fs::read_to_string(job_path(&home, &task.id).unwrap())
                .unwrap()
                .contains("fixture-key")
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn mp4_validation_rejects_html_and_uncertain_jobs_are_not_resubmitted() {
        assert!(mp4(&mp4_bytes()));
        assert!(!mp4(b"<html>not a video</html>"));
        let home = fixture_home();
        let mut task = job(&home);
        task.state = "submission-unconfirmed".into();
        task.error = Some("uncertain".into());
        save(&home, &task).unwrap();
        let credential = Credentials {
            base_url: "http://127.0.0.1:1".into(),
            api_key: "key".into(),
        };
        assert_eq!(
            result(&home, &task.id, &credential, &client()).unwrap()["status"],
            "submission-unconfirmed"
        );
        fs::remove_dir_all(home).unwrap();
    }
}
