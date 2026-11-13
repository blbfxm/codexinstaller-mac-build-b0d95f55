//! Paid vectorization submission is single-shot; status/result recovery never resubmits.
use super::image_plugin::{
    Credentials, bytes, credentials, download, http_client, image_mime, json_response, redact,
    redact_value,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) const MANIFEST: &str =
    include_str!("../plugins/jianguo-vectorize/.codex-plugin/plugin.json");
pub(super) const SKILL: &str =
    include_str!("../plugins/jianguo-vectorize/skills/jianguo-vectorize/SKILL.md");
pub(super) const AGENT: &str =
    include_str!("../plugins/jianguo-vectorize/skills/jianguo-vectorize/agents/openai.yaml");
const MAX_DATA: u64 = 32 * 1024 * 1024;

#[tauri::command]
pub async fn vector_plugin_status() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        super::image_plugin::plugin_status_at(&super::codex_home_dir()?, true)
    })
    .await
    .map_err(|_| "读取转矢量插件状态失败")?
}
#[tauri::command]
pub async fn install_vector_plugin(api_key: String, base_url: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = super::PROVIDER_CONFIG_LOCK
            .lock()
            .map_err(|_| "配置正在更新，请稍后重试")?;
        #[cfg(target_os = "windows")]
        let installed = super::detect_codex_windows().installed;
        #[cfg(not(target_os = "windows"))]
        let installed = super::detect_codex_macos().installed;
        if !installed {
            return Err("请先安装 Codex，再安装转矢量插件".into());
        }
        super::image_plugin::install_plugin_at(
            &super::codex_home_dir()?,
            &std::env::current_exe().map_err(|_| "无法定位插件运行程序")?,
            &base_url,
            &api_key,
            true,
        )
    })
    .await
    .map_err(|_| "安装转矢量插件失败")?
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    id: String,
    output_file: PathBuf,
    state: String,
    task_id: Option<String>,
    url: Option<String>,
    error: Option<String>,
    submitted_at: i64,
}
fn job_path(home: &Path, id: &str) -> Result<PathBuf, String> {
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("转矢量任务编号无效".into());
    }
    Ok(home
        .join(".usage-assistant/vector-plugin/jobs")
        .join(format!("{id}.json")))
}
fn save(home: &Path, job: &Job) -> Result<(), String> {
    super::provider_config::commit(vec![(
        job_path(home, &job.id)?,
        serde_json::to_vec(job).map_err(|_| "任务记录编码失败")?,
    )])
}
fn reply(job: &Job) -> Value {
    json!({"jobId":job.id,"status":job.state,"taskId":job.task_id,"filePath":if job.state == "completed" { Some(&job.output_file) } else { None },"message":job.error,"retryAfterSeconds":3})
}
fn request_body(args: &Value, key: &str) -> Result<Value, String> {
    let path = Path::new(
        args["imageFile"]
            .as_str()
            .ok_or("请指定本机位图 imageFile（绝对路径）")?,
    );
    if !path.is_absolute() {
        return Err("输入图片需为本机绝对路径，不支持远程 URL".into());
    }
    let mut data = Vec::new();
    fs::File::open(path)
        .map_err(|_| "无法读取输入位图")?
        .take(MAX_DATA + 1)
        .read_to_end(&mut data)
        .map_err(|_| "读取位图失败")?;
    if data.len() as u64 > MAX_DATA || image_mime(&data).is_none() {
        return Err("输入需为 PNG、JPEG 或 WebP 图片，最大 32 MB".into());
    }
    let mut body = json!({"kami":key,"imageBase64":base64::engine::general_purpose::STANDARD.encode(data),"format":"svg","engine":"vectorizer-ai","fileName":path.file_name().and_then(|s|s.to_str()).unwrap_or("image.png")});
    for name in ["sourceWidth", "sourceHeight"] {
        if let Some(v) = args.get(name) {
            if !v.as_u64().is_some_and(|n| (1..=65536).contains(&n)) {
                return Err("原图尺寸需为 1 至 65536 的整数".into());
            }
            body[name] = v.clone();
        }
    }
    Ok(body)
}
fn svg(data: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(data) else {
        return false;
    };
    let text = text.trim_start_matches('\u{feff}').trim_start();
    // Never interpret HTML/JSON error pages or a PNG preview as the vector result.
    (text.starts_with("<svg") || text.starts_with("<?xml") || text.starts_with("<!--"))
        && text.contains("<svg")
        && (text.contains("</svg>") || text.trim_end().ends_with("/>"))
        && !text.to_ascii_lowercase().contains("<html")
}
fn cache(home: &Path, job: &Job, data: &[u8]) -> Result<(), String> {
    if !svg(data) {
        return Err("服务未返回有效 SVG，已保留原任务，请继续查询而不要重复提交".into());
    }
    super::provider_config::commit(vec![(
        job_path(home, &job.id)?.with_extension("svg"),
        data.to_vec(),
    )])
}
fn completed(home: &Path, job: &mut Job, data: &[u8]) -> Result<Value, String> {
    if !svg(data) {
        return Err("缓存不是有效 SVG".into());
    }
    if job.output_file.exists() {
        if fs::read(&job.output_file).ok().as_deref() != Some(data) {
            return Err("输出文件已存在，原结果已缓存；请移开冲突文件后继续查询".into());
        }
    } else {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&job.output_file)
            .map_err(|_| "无法保存 SVG，原结果已缓存，请检查输出目录后继续查询")?;
        if file.write_all(data).and_then(|_| file.sync_all()).is_err() {
            drop(file);
            let _ = fs::remove_file(&job.output_file);
            return Err("保存 SVG 失败，原结果已缓存，请检查磁盘空间后继续查询".into());
        }
    }
    job.state = "completed".into();
    job.error = None;
    save(home, job)?;
    Ok(reply(job))
}
fn apply_status(job: &mut Job, value: &Value) -> Result<(), String> {
    if value["ok"] == false {
        return Err(value["message"]
            .as_str()
            .unwrap_or("转矢量接口请求失败")
            .into());
    }
    if let Some(task) = value["taskId"].as_str().filter(|s| !s.is_empty()) {
        job.task_id = Some(task.into());
    }
    let state = value["status"].as_str().or_else(|| value["type"].as_str());
    match state {
        Some("failed" | "error" | "cancelled") => {
            job.state = "failed".into();
            job.error = Some(
                value["error"]["message"]
                    .as_str()
                    .or_else(|| value["error"].as_str())
                    .or_else(|| value["message"].as_str())
                    .unwrap_or("转矢量失败")
                    .into(),
            );
        }
        Some("pending" | "running") => job.state = state.unwrap().into(),
        _ => job.state = "pending".into(),
    }
    let result = if value["data"].is_object() {
        &value["data"]
    } else {
        value
    };
    if let Some(url) = result["url"].as_str().filter(|s| !s.is_empty()) {
        job.url = Some(url.into());
    }
    if state == Some("completed") && job.url.is_none() {
        return Err("任务已完成但未返回 SVG 地址，预览图不能代替矢量结果，请继续查询原任务".into());
    }
    if job.task_id.is_none() && job.url.is_none() && job.state != "failed" {
        return Err("服务未返回任务 ID 或 SVG 地址，请勿重复提交，先检查平台记录".into());
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
    // Exactly one paid POST, including transport errors. No retry or redirection.
    let response = client
        .post(format!("{}/api/v1/vectorize", credential.base_url))
        .json(body)
        .send()
        .map_err(|_| "提交结果未确认，请勿重复转换，以免重复扣点；请稍后检查平台记录")?;
    let status = response.status();
    let data = bytes(response, MAX_DATA)?;
    if status.is_success() && svg(&data) {
        cache(home, job, &data)?;
        job.state = "pending".into();
        save(home, job)?;
        // Result queries perform the final save, so a filesystem error can be recovered.
        return Ok(());
    }
    let value: Value = serde_json::from_slice(&data)
        .map_err(|_| format!("转矢量返回了无法识别的响应（HTTP {status}），请勿重复提交"))?;
    if !status.is_success() || value["ok"] == false {
        job.state = "failed".into();
        job.error = Some(redact(
            value["message"]
                .as_str()
                .or_else(|| value["error"]["message"].as_str())
                .or_else(|| value["error"].as_str())
                .unwrap_or("转矢量请求失败"),
            &credential.api_key,
        ));
        return save(home, job);
    }
    apply_status(job, &value)?;
    save(home, job)
}
fn start(home: &Path, args: &Value) -> Result<Value, String> {
    let credential = credentials(home)?;
    let body = request_body(args, &credential.api_key)?;
    let output = PathBuf::from(
        args["outputFile"]
            .as_str()
            .ok_or("请指定 SVG 保存位置 outputFile（绝对路径）")?,
    );
    if !output.is_absolute()
        || output
            .extension()
            .and_then(|s| s.to_str())
            .is_none_or(|s| !s.eq_ignore_ascii_case("svg"))
        || output.exists()
    {
        return Err("输出需为尚不存在的 .svg 绝对路径，不会覆盖已有文件".into());
    }
    fs::create_dir_all(output.parent().ok_or("SVG 输出目录无效")?)
        .map_err(|_| "无法创建 SVG 输出目录")?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| "无法创建任务编号")?;
    let mut job = Job {
        id: random.iter().map(|b| format!("{b:02x}")).collect(),
        output_file: output,
        state: "submitting".into(),
        task_id: None,
        url: None,
        error: None,
        submitted_at: chrono::Utc::now().timestamp(),
    };
    let client = http_client()?;
    save(home, &job)?;
    let response = reply(&job);
    let home = home.to_owned();
    std::thread::spawn(move || {
        if let Err(e) = submit(&home, &mut job, &body, &credential, &client) {
            job.state = "submission-unconfirmed".into();
            job.error = Some(redact(&e, &credential.api_key));
            let _ = save(&home, &job);
        }
    });
    Ok(response)
}
fn task_url(base: &str, task: &str, events: bool) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base).map_err(|_| "坚果地址无效")?;
    let mut segments = url.path_segments_mut().map_err(|_| "坚果地址无效")?;
    segments.extend(["api", "v1", "task-status"]);
    if events {
        segments.push("events");
    }
    segments.push(task);
    drop(segments);
    Ok(url)
}
fn poll(job: &mut Job, client: &reqwest::blocking::Client, base: &str) -> Result<(), String> {
    let task = job.task_id.as_deref().ok_or("缺少原任务 ID")?;
    let response = client
        .get(task_url(base, task, false)?)
        .timeout(Duration::from_secs(15))
        .send()
        .map_err(|_| "查询任务暂时失败，请稍后继续查询原任务，不要重新转换")?;
    apply_status(job, &json_response(response)?)
}
fn events(job: &mut Job, client: &reqwest::blocking::Client, base: &str) -> Result<(), String> {
    let response = client
        .get(task_url(
            base,
            job.task_id.as_deref().ok_or("缺少任务 ID")?,
            true,
        )?)
        .header("Accept", "text/event-stream")
        .timeout(Duration::from_secs(4))
        .send()
        .map_err(|_| "进度流连接中断")?;
    if !response.status().is_success()
        || !response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| s.starts_with("text/event-stream"))
    {
        return Err("进度流暂不可用".into());
    }
    let mut reader = std::io::BufReader::new(response.take(1024 * 1024));
    let mut event = String::new();
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).map_err(|_| "进度流读取中断")?;
        if n == 0 || line.trim().is_empty() {
            if !event.is_empty() {
                let value: Value =
                    serde_json::from_str(event.trim()).map_err(|_| "进度事件格式无效")?;
                apply_status(job, &value)?;
                event.clear();
                if job.state == "failed" || job.url.is_some() {
                    return Ok(());
                }
            }
            if n == 0 {
                return Err("进度流已断开，继续查询原任务".into());
            };
        } else if let Some(data) = line.strip_prefix("data:") {
            event.push_str(data.trim_start());
            event.push('\n');
        }
    }
}
fn result(
    home: &Path,
    id: &str,
    client: &reqwest::blocking::Client,
    base: &str,
) -> Result<Value, String> {
    let mut job: Job = serde_json::from_slice(
        &fs::read(job_path(home, id)?).map_err(|_| "未找到转矢量任务，请核对 jobId")?,
    )
    .map_err(|_| "任务记录格式无效")?;
    if job.state == "completed" && job.output_file.is_file() {
        return Ok(reply(&job));
    }
    let cached = job_path(home, id)?.with_extension("svg");
    if cached.is_file() {
        return completed(
            home,
            &mut job,
            &fs::read(cached).map_err(|_| "无法读取已缓存 SVG")?,
        );
    }
    if job.state == "failed" || job.state == "submission-unconfirmed" {
        return Ok(reply(&job));
    }
    if job.state == "submitting" {
        if chrono::Utc::now().timestamp() - job.submitted_at > 240 {
            job.state = "submission-unconfirmed".into();
            job.error = Some("提交超时或连接中断，请检查平台记录，不要自动重复提交".into());
        }
        return Ok(reply(&job));
    }
    if job.url.is_none() && job.task_id.is_some() {
        // SSE disconnects do not lose progress; fall back to the same task's GET endpoint.
        if events(&mut job, client, base).is_err() && job.url.is_none() && job.state != "failed" {
            poll(&mut job, client, base)?;
        }
        save(home, &job)?;
    }
    if job.state == "failed" || job.url.is_none() {
        return Ok(reply(&job));
    }
    let url = reqwest::Url::parse(job.url.as_deref().unwrap()).map_err(|_| "SVG 下载地址无效")?;
    let response = download(client, url)?;
    if !response.status().is_success() {
        return Err("SVG 下载暂时失败，请继续查询原任务".into());
    }
    let data = bytes(response, MAX_DATA)?;
    cache(home, &job, &data)?;
    completed(home, &mut job, &data)
}
pub(super) fn tools() -> Value {
    json!([
        {"name":"get_vectorize_pricing","description":"查询坚果转矢量价格，键为 tool.vectorize.vectorizer-ai。","inputSchema":{"type":"object","properties":{},"additionalProperties":false},"annotations":{"readOnlyHint":true}},
        {"name":"vectorize_image","description":"将本机位图提交到坚果 AI 专业转矢量服务，返回 jobId。付费提交只执行一次；之后查询原任务，禁止自动重提。","inputSchema":{"type":"object","required":["imageFile","outputFile"],"additionalProperties":false,"properties":{"imageFile":{"type":"string","description":"本机 PNG/JPEG/WebP 绝对路径，会上传到坚果平台"},"outputFile":{"type":"string","description":"尚不存在的 SVG 绝对保存路径"},"sourceWidth":{"type":"integer","minimum":1,"maximum":65536},"sourceHeight":{"type":"integer","minimum":1,"maximum":65536}}},"annotations":{"readOnlyHint":false,"idempotentHint":false}},
        {"name":"get_vectorize_result","description":"查询原转矢量 jobId，优先 SSE、断线后查询状态；完成后保存 SVG。等待约3秒再查，不要重新提交。","inputSchema":{"type":"object","required":["jobId"],"properties":{"jobId":{"type":"string"}},"additionalProperties":false},"annotations":{"readOnlyHint":false,"idempotentHint":true}}
    ])
}
pub(super) fn call_tool(home: &Path, name: &str, args: &Value) -> Result<Value, String> {
    if name == "vectorize_image" {
        return start(home, args);
    }
    let credential = credentials(home)?;
    let client = http_client()?;
    let response = match name {
        "get_vectorize_pricing" => json_response(
            client
                .get(format!("{}/api/v1/pricing", credential.base_url))
                .send()
                .map_err(|_| "暂时无法查询价格，请检查网络")?,
        ),
        "get_vectorize_result" => result(
            home,
            args["jobId"].as_str().ok_or("缺少 jobId")?,
            &client,
            &credential.base_url,
        ),
        _ => Err("不支持的转矢量工具".into()),
    };
    response
        .map(|mut value| {
            redact_value(&mut value, &credential.api_key);
            value
        })
        .map_err(|e| redact(&e, &credential.api_key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_plugin::tests::{client, fixture_home, png, server};
    const SVG: &[u8] = br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><path d="M0 0h1v1z"/></svg>"#;
    fn job(home: &Path) -> Job {
        Job {
            id: "0123456789abcdef0123456789abcdef".into(),
            output_file: home.join("logo.svg"),
            state: "submitting".into(),
            task_id: None,
            url: None,
            error: None,
            submitted_at: chrono::Utc::now().timestamp(),
        }
    }
    #[test]
    fn request_uses_bound_key_fixed_engine_and_local_image() {
        let home = fixture_home();
        let input = home.join("logo.png");
        fs::write(&input, png()).unwrap();
        let args = json!({"imageFile":input,"sourceWidth":1024,"sourceHeight":512,"kami":"wrong-key","engine":"other","format":"png"});
        let body = request_body(&args, "current-user-key").unwrap();
        assert_eq!(body["kami"], "current-user-key");
        assert_eq!(body["engine"], "vectorizer-ai");
        assert_eq!(body["format"], "svg");
        assert_eq!(body["fileName"], "logo.png");
        assert_eq!(body["sourceWidth"], 1024);
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(body["imageBase64"].as_str().unwrap())
                .unwrap(),
            png()
        );
        assert!(request_body(&json!({"imageFile":"https://example.com/logo.png"}), "key").is_err());
        assert!(request_body(&json!({"imageFile":input,"sourceWidth":0}), "key").is_err());
        fs::write(&input, b"not an image").unwrap();
        assert!(request_body(&args, "key").is_err());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn synchronous_svg_is_cached_saved_and_recovered_without_a_second_post() {
        let home = fixture_home();
        let mut job = job(&home);
        let (base, handle) = server(vec![(200, "image/svg+xml", SVG.to_vec())]);
        let client = client();
        let credential = Credentials {
            base_url: base.clone(),
            api_key: "current-user-key".into(),
        };
        submit(
            &home,
            &mut job,
            &json!({"kami":credential.api_key,"engine":"vectorizer-ai"}),
            &credential,
            &client,
        )
        .unwrap();
        let value = result(&home, &job.id, &client, &base).unwrap();
        assert_eq!(value["status"], "completed");
        assert_eq!(fs::read(&job.output_file).unwrap(), SVG);
        assert_eq!(result(&home, &job.id, &client, &base).unwrap(), value);
        fs::remove_file(&job.output_file).unwrap();
        assert_eq!(result(&home, &job.id, &client, &base).unwrap(), value);
        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].0.starts_with("POST /api/v1/vectorize "));
        assert!(!requests[0].0.to_lowercase().contains("authorization:"));
        assert!(
            !fs::read_to_string(job_path(&home, &job.id).unwrap())
                .unwrap()
                .contains("current-user-key")
        );
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn sse_handles_pending_running_and_nested_completed_result() {
        let home = fixture_home();
        let mut job = job(&home);
        job.task_id = Some("original-task".into());
        let event = b": heartbeat\n\ndata: {\"status\":\"pending\"}\n\ndata: {\"status\":\"running\",\"detail\":\"converting\"}\n\ndata: {\"status\":\"completed\",\"data\":{\"url\":\"https://results.example.com/logo.svg\",\"previewBase64\":\"not-the-vector\"}}\n\n";
        let (base, handle) = server(vec![(
            200,
            "text/event-stream;charset=UTF-8",
            event.to_vec(),
        )]);
        events(&mut job, &client(), &base).unwrap();
        assert_eq!(
            job.url.as_deref(),
            Some("https://results.example.com/logo.svg")
        );
        assert_ne!(job.state, "failed");
        let requests = handle.join().unwrap();
        assert!(
            requests[0]
                .0
                .starts_with("GET /api/v1/task-status/events/original-task ")
        );
        assert!(!job.output_file.exists());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn disconnected_sse_polls_same_task_and_terminal_failure_does_not_resubmit() {
        let home = fixture_home();
        let mut job = job(&home);
        job.state = "pending".into();
        job.task_id = Some("same-task".into());
        save(&home, &job).unwrap();
        let (base, handle) = server(vec![
            (
                200,
                "text/event-stream",
                b"data: {\"status\":\"running\"}\n\n".to_vec(),
            ),
            (200, "application/json", br#"{"status":"running"}"#.to_vec()),
            (404, "text/plain", b"not available".to_vec()),
            (
                200,
                "application/json",
                br#"{"status":"failed","error":"conversion rejected"}"#.to_vec(),
            ),
        ]);
        let client = client();
        assert_eq!(
            result(&home, &job.id, &client, &base).unwrap()["status"],
            "running"
        );
        let failure = result(&home, &job.id, &client, &base).unwrap();
        assert_eq!(failure["status"], "failed");
        assert_eq!(failure["message"], "conversion rejected");
        assert_eq!(result(&home, &job.id, &client, &base).unwrap(), failure);
        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests.iter().all(|(h, _)| h.starts_with("GET ")));
        assert!(
            requests[1]
                .0
                .starts_with("GET /api/v1/task-status/same-task ")
        );
        assert!(
            requests[3]
                .0
                .starts_with("GET /api/v1/task-status/same-task ")
        );
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn sse_failed_event_and_url_submission_are_supported() {
        let home = fixture_home();
        let mut job = job(&home);
        job.task_id = Some("fail-task".into());
        let (base, handle) = server(vec![(
            200,
            "text/event-stream",
            b"data: {\"status\":\"failed\",\"error\":\"bad source\"}\n\n".to_vec(),
        )]);
        events(&mut job, &client(), &base).unwrap();
        assert_eq!(job.state, "failed");
        assert_eq!(job.error.as_deref(), Some("bad source"));
        handle.join().unwrap();
        let (base, handle) = server(vec![(
            200,
            "application/json",
            br#"{"type":"url","url":"https://results.example.com/logo.svg"}"#.to_vec(),
        )]);
        let credential = Credentials {
            base_url: base,
            api_key: "key".into(),
        };
        submit(
            &home,
            &mut job,
            &json!({"kami":"key"}),
            &credential,
            &client(),
        )
        .unwrap();
        assert_eq!(
            job.url.as_deref(),
            Some("https://results.example.com/logo.svg")
        );
        assert_eq!(job.state, "pending");
        handle.join().unwrap();
        // Previews are not vector files; HTML errors are not accepted as SVG.
        assert!(!svg(&png()));
        assert!(!svg(b"<html><svg></svg></html>"));
        assert!(
            apply_status(
                &mut self::job(&home),
                &json!({"status":"completed","data":{"previewBase64":"preview"}})
            )
            .is_err()
        );
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn cached_result_preserves_conflicts_and_uncertain_submit_is_not_repeated() {
        let home = fixture_home();
        let mut job = job(&home);
        job.state = "pending".into();
        save(&home, &job).unwrap();
        cache(&home, &job, SVG).unwrap();
        fs::write(&job.output_file, b"user file").unwrap();
        assert!(result(&home, &job.id, &client(), "http://127.0.0.1:1").is_err());
        assert_eq!(fs::read(&job.output_file).unwrap(), b"user file");
        fs::remove_file(&job.output_file).unwrap();
        assert_eq!(
            result(&home, &job.id, &client(), "http://127.0.0.1:1").unwrap()["status"],
            "completed"
        );
        fs::remove_file(job_path(&home, &job.id).unwrap().with_extension("svg")).unwrap();
        job.state = "submitting".into();
        job.submitted_at -= 300;
        save(&home, &job).unwrap();
        assert_eq!(
            result(&home, &job.id, &client(), "http://127.0.0.1:1").unwrap()["status"],
            "submission-unconfirmed"
        );
        fs::remove_dir_all(home).unwrap();
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn both_plugins_use_current_account_key_preserve_each_other_and_clear_old_key() {
        let home = fixture_home();
        let host = home.join("fixture.exe");
        fs::write(&host, b"fixture host").unwrap();
        fs::write(
            home.join("config.toml"),
            "model='existing-model'\n[plugins.'other@market']\nenabled=true\n",
        )
        .unwrap();
        super::super::image_plugin::install_plugin_at(
            &home,
            &host,
            "https://api.jianguo.ltd",
            "user-a-key",
            false,
        )
        .unwrap();
        let status = super::super::image_plugin::install_plugin_at(
            &home,
            &host,
            "https://api.jianguo.ltd",
            "user-b-key",
            true,
        )
        .unwrap();
        assert_eq!(status["enabled"], true);
        assert_eq!(credentials(&home).unwrap().api_key, "user-b-key");
        for vector in [false, true] {
            let s = super::super::image_plugin::plugin_status_at(&home, vector).unwrap();
            assert_eq!(s["enabled"], true);
            assert_eq!(s["credentialReady"], true);
        }
        let config = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains("existing-model"));
        assert!(config.contains("other@market"));
        assert!(!config.contains("user-b-key"));
        super::super::image_plugin::sync_credentials_at(&home, "user-c-key").unwrap();
        assert_eq!(credentials(&home).unwrap().api_key, "user-c-key");
        super::super::image_plugin::sync_credentials_at(&home, "").unwrap();
        assert!(credentials(&home).is_err());
        for vector in [false, true] {
            assert_eq!(
                super::super::image_plugin::plugin_status_at(&home, vector).unwrap()["credentialReady"],
                false
            );
        }
        fs::remove_dir_all(home).unwrap();
    }
}
