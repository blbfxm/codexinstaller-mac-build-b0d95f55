//! A personal, owner-only WeChat bridge. Credentials never cross the webview boundary.
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashSet, VecDeque},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{Manager, State};

const BASE: &str = "https://ilinkai.weixin.qq.com";
const VERSION: &str = "2.1.3";
#[derive(Clone, Serialize, Deserialize)]
struct Binding {
    token: String,
    owner: String,
    bot: String,
    base: String,
    #[serde(default)]
    cursor: String,
    #[serde(default)]
    handled: VecDeque<String>,
}
struct Pending {
    code: String,
    base: String,
    started: Instant,
}
#[derive(Default)]
struct Inner {
    loaded: bool,
    binding: Option<Binding>,
    pending: Option<Pending>,
    running: bool,
    workspace: String,
    writable: bool,
    phase: String,
    error: String,
    received: u64,
    replied: u64,
    activity: String,
    thread: Option<String>,
}
#[derive(Default)]
struct Shared {
    inner: Mutex<Inner>,
    generation: AtomicU64,
    child: Mutex<Option<Child>>,
    changed: tokio::sync::Notify,
}
#[derive(Default)]
pub struct WeChatState {
    shared: Arc<Shared>,
    actions: tokio::sync::Mutex<()>,
    qr_generation: AtomicU64,
    qr_changed: tokio::sync::Notify,
}

async fn fetch_qr(
    client: &reqwest::Client,
    base: &str,
    deadline: Duration,
) -> Result<Value, String> {
    tokio::time::timeout(deadline, async {
        let response = client
            .get(format!("{base}/ilink/bot/get_bot_qrcode"))
            .query(&[("bot_type", "3")])
            .header("iLink-App-Id", "bot")
            .header("iLink-App-ClientVersion", "131331")
            .send()
            .await
            .map_err(|_| "获取微信二维码失败，请检查网络，或切换网络代理后重试")?;
        decode(response).await
    })
    .await
    .map_err(|_| "获取微信二维码超时，请检查网络，或切换网络代理后重试".to_string())?
}

fn random_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "生成连接标识失败")?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn safe_base(raw: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "微信连接地址无效")?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "https"
        || !(host == "ilinkai.weixin.qq.com" || host.ends_with(".ilinkai.weixin.qq.com"))
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("微信服务返回了不受信任的连接地址，请重新扫码".into());
    }
    Ok(format!("https://{host}"))
}
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(45))
        .user_agent("CodexUsageAssistant-WeChat/1.0")
        .build()
        .map_err(|_| "微信网络初始化失败".into())
}
async fn decode(response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    if !status.is_success() {
        return Err(format!("微信服务暂不可用（HTTP {}）", status.as_u16()));
    }
    let mut response = response;
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "微信响应读取失败")? {
        if data.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("微信响应过大".into());
        }
        data.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&data).map_err(|_| "微信响应格式异常，请稍后重试")?;
    let code = value["errcode"]
        .as_i64()
        .filter(|c| *c != 0)
        .or_else(|| value["ret"].as_i64().filter(|c| *c != 0));
    if let Some(code) = code {
        return Err(format!(
            "微信服务返回错误 {code}，请稍后重试；若持续出现请重新绑定"
        ));
    }
    Ok(value)
}
async fn post(
    client: &reqwest::Client,
    binding: &Binding,
    endpoint: &str,
    mut body: Value,
) -> Result<Value, String> {
    body["base_info"] = json!({"channel_version": VERSION});
    let id = random_id()?;
    let uin = base64::engine::general_purpose::STANDARD
        .encode(u32::from_str_radix(&id[..8], 16).unwrap_or(1).to_string());
    let response = client
        .post(format!("{}/{endpoint}", binding.base))
        .bearer_auth(&binding.token)
        .header("AuthorizationType", "ilink_bot_token")
        .header("X-WECHAT-UIN", uin)
        .header("iLink-App-Id", "bot")
        .header("iLink-App-ClientVersion", "131331")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "微信轮询等待超时"
            } else {
                "连接微信服务失败，请检查网络"
            }
        })?;
    decode(response).await
}

#[cfg(target_os = "windows")]
fn credential_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_local_data_dir()
        .map_err(|_| "无法访问本机安全存储")?
        .join("wechat-binding.bin"))
}
fn save_binding(app: &tauri::AppHandle, binding: Option<&Binding>) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let path = credential_path(app)?;
        if let Some(binding) = binding {
            let bytes = serde_json::to_vec(binding).map_err(|_| "微信授权保存失败")?;
            let encrypted = super::protect_auto_login_kami(&bytes, true)?;
            fs::create_dir_all(path.parent().ok_or("安全存储目录无效")?)
                .map_err(|_| "无法创建安全存储目录")?;
            fs::write(path, encrypted).map_err(|_| "微信授权保存失败")?;
        } else if path.exists() {
            fs::remove_file(path).map_err(|_| "微信授权移除失败")?;
        }
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("/usr/bin/security");
        if let Some(binding) = binding {
            let value = base64::engine::general_purpose::STANDARD
                .encode(serde_json::to_vec(binding).map_err(|_| "微信授权保存失败")?);
            command.args([
                "add-generic-password",
                "-U",
                "-s",
                "com.jianguo.codex-assistant.wechat",
                "-a",
                "binding",
                "-w",
                &value,
            ]);
        } else {
            command.args([
                "delete-generic-password",
                "-s",
                "com.jianguo.codex-assistant.wechat",
                "-a",
                "binding",
            ]);
        }
        let status = command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| "无法访问 macOS 钥匙串")?;
        return if status.success() || binding.is_none() {
            Ok(())
        } else {
            Err("微信授权写入钥匙串失败".into())
        };
    }
    #[allow(unreachable_code)]
    Err("此系统暂不支持微信授权安全存储".into())
}
fn load_binding(app: &tauri::AppHandle) -> Result<Option<Binding>, String> {
    #[cfg(target_os = "windows")]
    {
        let path = credential_path(app)?;
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(path).map_err(|_| "微信授权读取失败")?;
        let decrypted = super::protect_auto_login_kami(&bytes, false)
            .map_err(|_| "微信授权无法解密，请解除绑定后重新扫码")?;
        let mut binding: Binding =
            serde_json::from_slice(&decrypted).map_err(|_| "微信授权损坏，请重新绑定")?;
        binding.base = safe_base(&binding.base)?;
        return Ok(Some(binding));
    }
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("/usr/bin/security")
            .args([
                "find-generic-password",
                "-s",
                "com.jianguo.codex-assistant.wechat",
                "-a",
                "binding",
                "-w",
            ])
            .output()
            .map_err(|_| "无法访问 macOS 钥匙串")?;
        if !output.status.success() {
            return Ok(None);
        }
        let data = base64::engine::general_purpose::STANDARD
            .decode(String::from_utf8_lossy(&output.stdout).trim())
            .map_err(|_| "微信授权损坏")?;
        let mut binding: Binding = serde_json::from_slice(&data).map_err(|_| "微信授权损坏")?;
        binding.base = safe_base(&binding.base)?;
        return Ok(Some(binding));
    }
    #[allow(unreachable_code)]
    Ok(None)
}
fn status(shared: &Shared) -> Result<Value, String> {
    let inner = shared.inner.lock().map_err(|_| "微信状态繁忙")?;
    let owner = inner.binding.as_ref().map(|b| {
        let chars: Vec<char> = b.owner.chars().collect();
        if chars.len() > 8 {
            format!(
                "{}…{}",
                chars[..4].iter().collect::<String>(),
                chars[chars.len() - 4..].iter().collect::<String>()
            )
        } else {
            "已绑定个人微信".into()
        }
    });
    Ok(
        json!({"bound":inner.binding.is_some(),"owner":owner,"running":inner.running,"workspace":inner.workspace,"writable":inner.writable,"phase":inner.phase,"error":inner.error,"received":inner.received,"replied":inner.replied,"activity":inner.activity,"hasSession":inner.thread.is_some()}),
    )
}
fn terminate_owned_child(shared: &Shared) {
    if let Ok(mut guard) = shared.child.lock() {
        if let Some(mut child) = guard.take() {
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                let _ = Command::new("taskkill.exe")
                    .args(["/PID", &child.id().to_string(), "/T", "/F"])
                    .creation_flags(0x08000000)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
pub fn shutdown(state: &WeChatState) {
    state.qr_generation.fetch_add(1, Ordering::SeqCst);
    state.qr_changed.notify_waiters();
    state.shared.generation.fetch_add(1, Ordering::SeqCst);
    state.shared.changed.notify_waiters();
    terminate_owned_child(&state.shared);
    if let Ok(mut inner) = state.shared.inner.lock() {
        inner.running = false;
        inner.pending = None;
        inner.phase = "已停止".into();
    }
}
fn active(shared: &Shared, generation: u64) -> bool {
    shared.generation.load(Ordering::SeqCst) == generation
}
fn activity(shared: &Shared, phase: &str) {
    if let Ok(mut inner) = shared.inner.lock() {
        inner.phase = phase.into();
        inner.activity = chrono::Local::now().format("%H:%M:%S").to_string();
    }
}
fn checkpoint(
    app: &tauri::AppHandle,
    shared: &Shared,
    generation: u64,
    binding: &Binding,
) -> Result<bool, String> {
    let mut inner = shared.inner.lock().map_err(|_| "微信状态繁忙")?;
    if !active(shared, generation) {
        return Ok(false);
    }
    save_binding(app, Some(binding))?;
    inner.binding = Some(binding.clone());
    Ok(true)
}
fn resolve_cli(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        let installation = super::detect_codex_windows();
        if !installation.installed {
            return Err("请先在“安装与开始”安装 Codex".into());
        }
        let desktop = PathBuf::from(&installation.path);
        let source = desktop
            .parent()
            .ok_or("Codex 安装位置无效")?
            .join("resources/codex.exe");
        if !source.is_file() {
            return Err("当前 Codex 安装缺少会话组件，请更新 Codex 后重试".into());
        }
        let meta = fs::metadata(&source).map_err(|_| "无法读取 Codex 会话组件")?;
        use sha2::{Digest, Sha256};
        let key = format!(
            "{:x}",
            Sha256::digest(format!(
                "{}-{}-{:?}",
                source.display(),
                meta.len(),
                meta.modified().ok()
            ))
        );
        let directory = app
            .path()
            .app_local_data_dir()
            .map_err(|_| "无法访问本机运行目录")?
            .join("wechat-runtime")
            .join(&key[..16]);
        fs::create_dir_all(&directory).map_err(|_| "创建微信会话运行目录失败")?;
        let target = directory.join("codex.exe");
        for name in [
            "codex.exe",
            "codex-command-runner.exe",
            "codex-code-mode-host.exe",
            "codex-windows-sandbox-service.exe",
            "codex-windows-sandbox-setup.exe",
            "rg.exe",
        ] {
            let original = source.parent().unwrap().join(name);
            let dest = directory.join(name);
            if original.is_file() && !dest.is_file() {
                fs::copy(original, dest).map_err(|_| "准备 Codex 会话组件失败，请检查磁盘空间")?;
            }
        }
        verify_cli(&target)?;
        return Ok(target);
    }
    #[cfg(target_os = "macos")]
    {
        use super::macos_apps;
        let _ = app;
        let desktop = macos_apps::discover(false);
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut found_component = false;
        for candidate in macos_apps::session_cli_candidates(
            desktop.as_ref().map(|app| app.executable.as_path()), home.as_deref(), std::env::var_os("PATH").as_deref(),
        ) {
            if !macos_apps::executable_file(&candidate) { continue; }
            found_component = true;
            if verify_cli(&candidate).is_ok() { return Ok(candidate); }
        }
        return Err(if found_component {
            "已找到 Codex 会话组件，但无法运行。请更新 Codex，并检查 macOS 是否阻止了应用运行；已有微信绑定会保留，无需重新扫码。"
        } else if desktop.is_some() {
            "已检测到 Codex，但安装中缺少可用的会话组件。请更新 Codex 后点击“重新检测”；已有微信绑定会保留，无需重新扫码。"
        } else {
            "未检测到可用的 Codex 会话组件。请先在“安装与开始”安装 Codex；若已经安装，请将应用移入“应用程序”文件夹后重新检测。"
        }.into());
    }
    #[allow(unreachable_code)]
    Err("未找到可用的 Codex 会话组件，请先安装 Codex".into())
}

fn verify_cli(cli: &Path) -> Result<(), String> {
    let mut command = Command::new(cli);
    // Help is local and does not send a model request or start a desktop window.
    command.args(["exec", "--help"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|_| "Codex 会话组件无法运行，请更新 Codex 后重试".to_string())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err("Codex 会话组件检查失败，请更新 Codex 后重试".into()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(30)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("检查 Codex 会话组件超时，请稍后重新检测".into());
            }
        }
    }
}

async fn prepare_cli(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let app = app.clone();
    tokio::time::timeout(Duration::from_secs(25), tauri::async_runtime::spawn_blocking(move || resolve_cli(&app)))
        .await.map_err(|_| "检查 Codex 会话组件超时，请点击“重新检测”重试")?
        .map_err(|_| "Codex 会话组件检查未完成，请重新检测")?
}

fn workspace_directory(body: &Value) -> Result<PathBuf, String> {
    let directory = PathBuf::from(body["workspace"].as_str().unwrap_or_default());
    if !directory.is_absolute() || !directory.is_dir() {
        return Err("请先选择一个存在的本机项目文件夹，或填写文件夹的完整路径".into());
    }
    let directory = directory.canonicalize().map_err(|_| "无法访问项目文件夹，请重新选择")?;
    fs::read_dir(&directory).map_err(|_| "无法读取项目文件夹，请检查权限或重新选择")?;
    Ok(directory)
}
fn invoke_codex(
    shared: Arc<Shared>,
    generation: u64,
    cli: &Path,
    directory: &Path,
    writable: bool,
    prompt: &str,
) -> Result<String, String> {
    let thread = shared
        .inner
        .lock()
        .map_err(|_| "微信状态繁忙")?
        .thread
        .clone();
    let sandbox = if writable {
        "workspace-write"
    } else {
        "read-only"
    };
    let mut command = Command::new(cli);
    command.current_dir(directory).args([
        "-c",
        "approval_policy=\"never\"",
        "-c",
        &format!("sandbox_mode=\"{sandbox}\""),
        "exec",
    ]);
    if let Some(thread) = thread {
        command.args(["resume", "--skip-git-repo-check", "--json", &thread, "-"]);
    } else {
        command.args(["--skip-git-repo-check", "--json", "--sandbox", sandbox, "-"]);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut guard = shared.child.lock().map_err(|_| "微信会话繁忙")?;
    if !active(&shared, generation) {
        return Err("连接已停止".into());
    }
    let mut child = command
        .spawn()
        .map_err(|_| "Codex 会话启动失败，请先在助手中配置可用供应商")?;
    let stdout = child.stdout.take().ok_or("无法读取 Codex 会话输出")?;
    let mut stdin = child.stdin.take().ok_or("无法写入 Codex 会话输入")?;
    *guard = Some(child);
    drop(guard);
    // Drain output even if the retained response reaches the cap, preventing a full pipe.
    let reader = std::thread::spawn(move || {
        let mut stdout = stdout;
        let mut data = Vec::new();
        let _ = stdout.by_ref().take(4 * 1024 * 1024).read_to_end(&mut data);
        let _ = std::io::copy(&mut stdout, &mut std::io::sink());
        data
    });
    let written = stdin.write_all(prompt.as_bytes());
    drop(stdin);
    if written.is_err() {
        terminate_owned_child(&shared);
        let _ = reader.join();
        return Err("Codex 会话输入失败".into());
    }
    let started = Instant::now();
    let mut success = false;
    let mut cancelled = false;
    loop {
        if !active(&shared, generation) || started.elapsed() > Duration::from_secs(600) {
            cancelled = true;
            terminate_owned_child(&shared);
            break;
        }
        let ended = {
            let mut guard = shared.child.lock().map_err(|_| "微信会话繁忙")?;
            match guard.as_mut() {
                Some(child) => child.try_wait().map_err(|_| "Codex 会话状态读取失败")?,
                None => {
                    cancelled = true;
                    break;
                }
            }
        };
        if let Some(status) = ended {
            success = status.success();
            if let Ok(mut g) = shared.child.lock() {
                g.take();
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    let output = reader.join().unwrap_or_default();
    if cancelled {
        return Err("本次处理已停止或超时，请缩小任务范围后重试".into());
    }
    let mut reply = String::new();
    let mut thread = None;
    for line in output.split(|b| *b == b'\n') {
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if value["type"] == "thread.started" {
            thread = value["thread_id"].as_str().map(str::to_owned);
        }
        if value["type"] == "item.completed" && value["item"]["type"] == "agent_message" {
            if let Some(text) = value["item"]["text"].as_str() {
                if !reply.is_empty() {
                    reply.push_str("\n\n");
                }
                reply.push_str(text);
            }
        }
    }
    if let Some(thread) = thread {
        if active(&shared, generation) {
            if let Ok(mut inner) = shared.inner.lock() {
                inner.thread = Some(thread);
            }
        }
    }
    if !success {
        return Err("Codex 处理失败，请检查助手中的供应商配置、API Key 余额和模型可用性".into());
    }
    if reply.trim().is_empty() {
        return Err("Codex 未返回文字回复，请重试".into());
    }
    Ok(reply.chars().take(24000).collect())
}
fn incoming(message: &Value, owner: &str) -> Option<(String, String, String)> {
    if message["from_user_id"].as_str() != Some(owner)
        || message["message_type"].as_i64() != Some(1)
    {
        return None;
    }
    let key = message["message_id"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| message["message_id"].as_u64().map(|id| id.to_string()))
        .or_else(|| message["seq"].as_u64().map(|id| format!("seq-{id}")))?;
    let context = message["context_token"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if context.is_empty() {
        return None;
    }
    let text = message["item_list"]
        .as_array()?
        .iter()
        .filter_map(|item| match item["type"].as_u64() {
            Some(1) => item["text_item"]["text"].as_str(),
            Some(3) => item["voice_item"]["text"].as_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some((key, context, text))
}
async fn send_reply(
    client: &reqwest::Client,
    binding: &Binding,
    context: &str,
    text: &str,
) -> Result<(), String> {
    let chars: Vec<char> = text.chars().collect();
    for chunk in chars.chunks(3200) {
        post(client,binding,"ilink/bot/sendmessage",json!({"msg":{"from_user_id":"","to_user_id":binding.owner,"client_id":format!("assistant-{}",random_id()?),"message_type":2,"message_state":2,"context_token":context,"item_list":[{"type":1,"text_item":{"text":chunk.iter().collect::<String>()}}]}})).await?;
    }
    Ok(())
}
async fn bridge(
    app: tauri::AppHandle,
    shared: Arc<Shared>,
    generation: u64,
    mut binding: Binding,
    cli: PathBuf,
    directory: PathBuf,
    writable: bool,
) -> Result<(), String> {
    let client = client()?;
    let mut cursor = binding.cursor.clone();
    let mut seen: HashSet<String> = binding.handled.iter().cloned().collect();
    let mut failures = 0u8;
    while active(&shared, generation) {
        activity(&shared, "等待微信消息");
        let update = tokio::select! {
            _=shared.changed.notified()=> { if !active(&shared,generation){break;} continue; },
            result=post(&client,&binding,"ilink/bot/getupdates",json!({"get_updates_buf":cursor}))=>result,
        };
        if !active(&shared, generation) {
            break;
        }
        let value = match update {
            Ok(value) => {
                failures = 0;
                value
            }
            Err(e) if e == "微信轮询等待超时" => continue,
            Err(e) => {
                failures += 1;
                if failures >= 5 {
                    return Err(e);
                }
                activity(&shared, "连接中断，正在重连");
                tokio::select! {_=shared.changed.notified()=>{},_=tokio::time::sleep(Duration::from_secs(3*u64::from(failures)))=>{}}
                continue;
            }
        };
        if let Some(next) = value["get_updates_buf"].as_str() {
            cursor = next.into();
        }
        let Some(messages) = value["msgs"].as_array() else {
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        };
        if messages.is_empty() {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        for message in messages {
            if !active(&shared, generation) {
                break;
            }
            let Some((id, context, text)) = incoming(message, &binding.owner) else {
                continue;
            };
            if !seen.insert(id.clone()) {
                continue;
            }
            binding.handled.push_back(id);
            while binding.handled.len() > 2048 {
                if let Some(old) = binding.handled.pop_front() {
                    seen.remove(&old);
                }
            }
            // Reserve the message before executing it; a crash/restart must not repeat file changes.
            if !checkpoint(&app, &shared, generation, &binding)? {
                break;
            }
            if let Ok(mut inner) = shared.inner.lock() {
                inner.received += 1;
            }
            let reply = if text.trim() == "/new" {
                if let Ok(mut inner) = shared.inner.lock() {
                    inner.thread = None;
                }
                "已开始新的 Codex 会话。请发送任务内容。".into()
            } else if text.trim() == "/help" {
                "直接发送文字开始对话；/new 开始新会话。目前支持文字和已识别为文字的语音；图片及文件请在电脑端处理。".into()
            } else if text.trim().is_empty() {
                "目前支持文字和已识别为文字的语音，请发送文字描述；图片和文件请在电脑端处理。"
                    .into()
            } else if text.chars().count() > 20000 {
                "消息过长，请拆分后再发送。".into()
            } else {
                activity(&shared, "Codex 正在处理消息");
                let worker = shared.clone();
                let executable = cli.clone();
                let cwd = directory.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    invoke_codex(worker, generation, &executable, &cwd, writable, &text)
                })
                .await
                .map_err(|_| "Codex 会话任务异常结束")?
                .unwrap_or_else(|e| e)
            };
            if !active(&shared, generation) {
                break;
            }
            activity(&shared, "正在发送回复");
            tokio::select! {_=shared.changed.notified()=>{if !active(&shared,generation){break;}},result=send_reply(&client,&binding,&context,&reply)=>{result?;}}
            if let Ok(mut inner) = shared.inner.lock() {
                inner.replied += 1;
            }
        }
        if active(&shared, generation) && cursor != binding.cursor {
            binding.cursor = cursor.clone();
            if !checkpoint(&app, &shared, generation, &binding)? {
                break;
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn wechat_action(
    app: tauri::AppHandle,
    state: State<'_, WeChatState>,
    action: String,
    payload: Option<Value>,
) -> Result<Value, String> {
    let body = payload.unwrap_or(json!({}));
    // Interactive directory selection must never hold the connection-operation lock.
    if action == "chooseFolder" {
        return Ok(json!({"path":super::folder_picker::choose(&app).await?}));
    }
    if action == "preflight" {
        return Ok(match prepare_cli(&app).await {
            Ok(_) => json!({"ready":true,"message":"Codex 会话组件可用"}),
            Err(message) => json!({"ready":false,"message":message}),
        });
    }
    if action == "cancelQr" {
        state.qr_generation.fetch_add(1, Ordering::SeqCst);
        state.qr_changed.notify_waiters();
        state
            .shared
            .inner
            .lock()
            .map_err(|_| "微信状态繁忙")?
            .pending = None;
        return status(&state.shared);
    }
    if action == "qr" {
        if state
            .shared
            .inner
            .lock()
            .map_err(|_| "微信状态繁忙")?
            .running
        {
            return Err("请先停止微信连接再重新绑定".into());
        }
        let generation = state.qr_generation.fetch_add(1, Ordering::SeqCst) + 1;
        state.qr_changed.notify_waiters();
        workspace_directory(&body)?;
        tokio::select! {
            _ = state.qr_changed.notified() => return Err("二维码请求已取消".into()),
            result = prepare_cli(&app) => { result?; },
        }
        if generation != state.qr_generation.load(Ordering::SeqCst) {
            return Err("二维码请求已取消".into());
        }
        let connection = client()?;
        let value = tokio::select! {
            _ = state.qr_changed.notified() => return Err("二维码请求已取消".into()),
            value = fetch_qr(&connection, BASE, Duration::from_secs(12)) => value?,
        };
        let code = value["qrcode"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("微信服务未返回二维码")?
            .to_owned();
        let content = value["qrcode_img_content"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() < 4096)
            .ok_or("微信二维码内容无效")?
            .to_owned();
        let mut inner = state.shared.inner.lock().map_err(|_| "微信状态繁忙")?;
        if generation != state.qr_generation.load(Ordering::SeqCst) || inner.running {
            return Err("二维码请求已取消，请重试".into());
        }
        inner.pending = Some(Pending {
            code,
            base: BASE.into(),
            started: Instant::now(),
        });
        return Ok(json!({"content":content}));
    }
    if action == "status" {
        let mut inner = state.shared.inner.lock().map_err(|_| "微信状态繁忙")?;
        if !inner.loaded {
            match load_binding(&app) {
                Ok(binding) => inner.binding = binding,
                Err(e) => inner.error = e,
            }
            inner.loaded = true;
        }
        drop(inner);
        return status(&state.shared);
    }
    if action == "pollQr" {
        let (code, base) = {
            let inner = state.shared.inner.lock().map_err(|_| "微信状态繁忙")?;
            let pending = inner.pending.as_ref().ok_or("请先获取连接二维码")?;
            if pending.started.elapsed() > Duration::from_secs(300) {
                return Ok(json!({"status":"expired"}));
            }
            (pending.code.clone(), pending.base.clone())
        };
        let response = client()?
            .get(format!("{base}/ilink/bot/get_qrcode_status"))
            .query(&[("qrcode", &code)])
            .header("iLink-App-Id", "bot")
            .header("iLink-App-ClientVersion", "131331")
            .timeout(Duration::from_secs(35))
            .send()
            .await;
        let value = match response {
            Ok(r) => decode(r).await?,
            Err(e) if e.is_timeout() => return Ok(json!({"status":"wait"})),
            Err(_) => return Err("微信扫码状态读取失败，请检查网络后重试".into()),
        };
        let _action = state.actions.lock().await;
        let mut inner = state.shared.inner.lock().map_err(|_| "微信状态繁忙")?;
        let current = inner.pending.as_ref().is_some_and(|p| p.code == code);
        if !current {
            return Ok(json!({"status":"expired"}));
        }
        let stage = value["status"].as_str().unwrap_or("wait");
        if stage == "scaned_but_redirect" {
            let host = value["redirect_host"]
                .as_str()
                .ok_or("微信重定向地址缺失")?;
            let base = safe_base(&format!("https://{host}"))?;
            if let Some(pending) = inner.pending.as_mut() {
                pending.base = base;
            }
        }
        if stage == "confirmed" {
            let required = |field: &str| {
                value[field]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| "微信授权不完整，请重新扫码确认".to_string())
            };
            let binding = Binding {
                token: required("bot_token")?,
                owner: required("ilink_user_id")?,
                bot: required("ilink_bot_id")?,
                base: safe_base(value["baseurl"].as_str().unwrap_or(&base))?,
                cursor: String::new(),
                handled: VecDeque::new(),
            };
            save_binding(&app, Some(&binding))?;
            inner.binding = Some(binding);
            inner.pending = None;
            inner.error.clear();
            inner.thread = None;
        }
        return Ok(json!({"status":stage}));
    }
    let _action = state.actions.lock().await;
    match action.as_str() {
        "start" => {
            let binding = {
                let inner = state.shared.inner.lock().map_err(|_| "微信状态繁忙")?;
                if inner.running {
                    return status_after_unlock(inner, &state.shared);
                }
                inner.binding.clone().ok_or("请先扫码绑定个人微信")?
            };
            let directory = workspace_directory(&body)?;
            let writable = body["writable"].as_bool().unwrap_or(false);
            let cli = prepare_cli(&app).await?;
            let generation = state.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
            {
                let mut inner = state.shared.inner.lock().map_err(|_| "微信状态繁忙")?;
                if inner.workspace != directory.to_string_lossy() || inner.writable != writable {
                    inner.thread = None;
                }
                inner.workspace = directory.to_string_lossy().into_owned();
                inner.writable = writable;
                inner.running = true;
                inner.error.clear();
                inner.phase = "正在连接微信".into();
            }
            let shared = state.shared.clone();
            tauri::async_runtime::spawn(async move {
                let result = bridge(
                    app,
                    shared.clone(),
                    generation,
                    binding,
                    cli,
                    directory,
                    writable,
                )
                .await;
                if active(&shared, generation) {
                    if let Ok(mut inner) = shared.inner.lock() {
                        inner.running = false;
                        inner.phase = "连接已停止".into();
                        if let Err(error) = result {
                            inner.error = error;
                        }
                    }
                }
            });
            status(&state.shared)
        }
        "stop" => {
            shutdown(&state);
            status(&state.shared)
        }
        "unbind" => {
            shutdown(&state);
            save_binding(&app, None)?;
            let mut inner = state.shared.inner.lock().map_err(|_| "微信状态繁忙")?;
            *inner = Inner {
                loaded: true,
                ..Inner::default()
            };
            drop(inner);
            status(&state.shared)
        }
        _ => Err("不支持的微信连接操作".into()),
    }
}
fn status_after_unlock(
    guard: std::sync::MutexGuard<'_, Inner>,
    shared: &Shared,
) -> Result<Value, String> {
    drop(guard);
    status(shared)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scan_and_start_require_an_accessible_absolute_workspace() {
        let root = std::env::temp_dir().join(format!("wechat-workspace-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("file.txt"), "fixture").unwrap();
        for body in [json!({}), json!({"workspace":"relative/path"}), json!({"workspace":root.join("missing")}), json!({"workspace":root.join("file.txt")})] {
            assert!(workspace_directory(&body).is_err());
        }
        assert_eq!(workspace_directory(&json!({"workspace":root})).unwrap(), root.canonicalize().unwrap());
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn component_probe_checks_exec_without_sending_a_prompt() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("wechat-cli-probe-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let cli = root.join("codex");
        fs::write(&cli, "#!/bin/sh\n[ \"$#\" = 2 ] && [ \"$1\" = exec ] && [ \"$2\" = --help ]\n").unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(verify_cli(&cli).is_ok());
        fs::write(&cli, "#!/bin/sh\nexit 1\n").unwrap();
        assert!(verify_cli(&cli).is_err());
        assert!(verify_cli(&root.join("missing")).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn qr_request_has_a_deadline_including_slow_response_body() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0u8; 2048];
            let _ = stream.read(&mut bytes);
            let _=stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{");
            std::thread::sleep(Duration::from_millis(300));
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let started = Instant::now();
        let error = fetch_qr(&client, &base, Duration::from_millis(100))
            .await
            .unwrap_err();
        assert!(error.contains("超时"));
        assert!(started.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }
    #[test]
    fn only_official_https_endpoints_allowed() {
        assert!(safe_base(BASE).is_ok());
        for raw in [
            "http://ilinkai.weixin.qq.com",
            "https://ilinkai.weixin.qq.com.evil.test",
            "https://evil.test",
            "https://ilinkai.weixin.qq.com:444",
            "https://ilinkai.weixin.qq.com/path",
            "https://x@ilinkai.weixin.qq.com",
        ] {
            assert!(safe_base(raw).is_err(), "{raw}");
        }
    }
    #[test]
    fn incoming_is_owner_only_and_does_not_handle_outgoing() {
        let msg = json!({"message_id":5,"from_user_id":"owner","message_type":1,"context_token":"ctx","item_list":[{"type":1,"text_item":{"text":"你好"}}]});
        assert_eq!(incoming(&msg, "owner").unwrap().2, "你好");
        assert!(incoming(&msg, "other").is_none());
        let mut sent = msg;
        sent["message_type"] = json!(2);
        assert!(incoming(&sent, "owner").is_none());
    }
    #[test]
    fn stop_invalidates_pending_scans_and_only_terminates_the_bridge_child() {
        let state = WeChatState::default();
        let mut command = if cfg!(target_os = "windows") {
            Command::new("powershell.exe")
        } else {
            Command::new("sleep")
        };
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command
                .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"])
                .creation_flags(0x08000000);
        }
        #[cfg(not(target_os = "windows"))]
        command.arg("60");
        *state.shared.child.lock().unwrap() = Some(
            command
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        {
            let mut inner = state.shared.inner.lock().unwrap();
            inner.running = true;
            inner.pending = Some(Pending {
                code: "fixture".into(),
                base: BASE.into(),
                started: Instant::now(),
            });
        }
        shutdown(&state);
        assert!(!active(&state.shared, 0));
        assert!(state.shared.child.lock().unwrap().is_none());
        let inner = state.shared.inner.lock().unwrap();
        assert!(!inner.running);
        assert!(inner.pending.is_none());
    }
}
