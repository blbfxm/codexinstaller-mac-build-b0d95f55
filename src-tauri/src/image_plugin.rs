//! Built-in Codex plugin installer and dependency-free stdio MCP host.
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use toml_edit::{DocumentMut, value};

const NAME: &str = "jianguo-image";
const MARKET: &str = "jianguo-assistant";
const VERSION: &str = "1.0.0";
const MANIFEST: &str = include_str!("../plugins/jianguo-image/.codex-plugin/plugin.json");
const SKILL: &str = include_str!("../plugins/jianguo-image/skills/jianguo-image/SKILL.md");
const AGENT: &str =
    include_str!("../plugins/jianguo-image/skills/jianguo-image/agents/openai.yaml");
const MAX_IMAGE: u64 = 32 * 1024 * 1024;
const MAX_REFERENCE_IMAGES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BuiltinKind {
    Image,
    Vector,
    Video,
}

impl BuiltinKind {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Image => NAME,
            Self::Vector => "jianguo-vectorize",
            Self::Video => "jianguo-video",
        }
    }

    pub(super) fn server(self) -> &'static str {
        match self {
            Self::Image => "jianguo_image",
            Self::Vector => "jianguo_vectorize",
            Self::Video => "jianguo_video",
        }
    }

    pub(super) fn flag(self) -> &'static str {
        match self {
            Self::Image => "--jianguo-image-mcp",
            Self::Vector => "--jianguo-vectorize-mcp",
            Self::Video => "--jianguo-video-mcp",
        }
    }
}

fn kind_from_flag(flag: &str) -> Option<BuiltinKind> {
    [BuiltinKind::Image, BuiltinKind::Vector, BuiltinKind::Video]
        .into_iter()
        .find(|kind| kind.flag() == flag)
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(super) struct Credentials {
    pub(super) base_url: String,
    pub(super) api_key: String,
}

fn private_root(home: &Path) -> PathBuf {
    home.join(".usage-assistant").join("image-plugin")
}
fn market_root(home: &Path) -> PathBuf {
    private_root(home).join("marketplace")
}
#[cfg(test)]
fn cache_root(home: &Path) -> PathBuf {
    home.join("plugins/cache")
        .join(MARKET)
        .join(NAME)
        .join(VERSION)
}

fn normalize_base(base: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(base.trim()).map_err(|_| "坚果 Base URL 无效")?;
    if url.scheme() != "https"
        || url.host_str() != Some("api.jianguo.ltd")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path().trim_end_matches('/'), "" | "/v1" | "/api/v1")
    {
        return Err("内置插件仅连接 https://api.jianguo.ltd，已阻止向其他地址发送卡密".into());
    }
    Ok("https://api.jianguo.ltd".into())
}

pub(super) fn credentials(home: &Path) -> Result<Credentials, String> {
    #[cfg(target_os = "windows")]
    let bytes = super::protect_auto_login_kami(
        &fs::read(private_root(home).join("credentials.dpapi"))
            .map_err(|_| "请在使用助手的插件页面安装插件或更新凭证")?,
        false,
    )
    .map_err(|_| "插件凭证无法解密，请在使用助手中更新凭证")?;
    #[cfg(target_os = "macos")]
    let bytes = {
        let output = std::process::Command::new("/usr/bin/security")
            .args([
                "find-generic-password",
                "-s",
                "com.jianguo.codex.image-plugin",
                "-a",
                &home.to_string_lossy(),
                "-w",
            ])
            .output()
            .map_err(|_| "读取插件凭证失败，请更新凭证")?;
        if !output.status.success() {
            return Err("未找到插件凭证，请在使用助手中更新凭证".into());
        }
        output.stdout
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let bytes = Vec::<u8>::new();
    let result: Credentials =
        serde_json::from_slice(&bytes).map_err(|_| "插件凭证格式无效，请更新凭证")?;
    normalize_base(&result.base_url)?;
    if result.api_key.trim().is_empty() {
        return Err("插件 API Key 为空，请更新凭证".into());
    }
    Ok(result)
}

fn save_credentials(home: &Path, base: &str, key: &str) -> Result<(), String> {
    let data = Credentials {
        base_url: normalize_base(base)?,
        api_key: key.trim().to_owned(),
    };
    if data.api_key.is_empty() || data.api_key.len() > 4096 {
        return Err("请先登录并获取有效的坚果 API Key".into());
    }
    let bytes = serde_json::to_vec(&data).map_err(|_| "凭证编码失败")?;
    #[cfg(target_os = "windows")]
    return super::provider_config::commit(vec![(
        private_root(home).join("credentials.dpapi"),
        super::protect_auto_login_kami(&bytes, true)?,
    )]);
    #[cfg(target_os = "macos")]
    {
        // security reads its interactive command from stdin, keeping the secret out of argv.
        let escaped = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let mut child = std::process::Command::new("/usr/bin/security")
            .arg("-i")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|_| "无法保存插件凭证到系统钥匙串")?;
        let line = format!(
            "add-generic-password -U -s com.jianguo.codex.image-plugin -a \"{}\" -w \"{}\"\n",
            escaped(&home.to_string_lossy()),
            escaped(&String::from_utf8_lossy(&bytes))
        );
        child
            .stdin
            .take()
            .ok_or("钥匙串输入不可用")?
            .write_all(line.as_bytes())
            .map_err(|_| "保存插件凭证失败")?;
        if !child.wait().map_err(|_| "保存插件凭证失败")?.success() {
            return Err("保存插件凭证失败".into());
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    Err("当前平台暂不支持安全保存插件凭证".into())
}

#[tauri::command]
pub async fn sync_builtin_plugin_credentials(api_key: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = super::PROVIDER_CONFIG_LOCK
            .lock()
            .map_err(|_| "配置正在更新，请稍后重试")?;
        sync_credentials_at(&super::codex_home_dir()?, &api_key)
    })
    .await
    .map_err(|_| "同步插件凭证失败")?
}
pub(super) fn sync_credentials_at(home: &Path, key: &str) -> Result<(), String> {
    if !private_root(home).exists() {
        return Ok(());
    }
    if !key.trim().is_empty() {
        return save_credentials(home, "https://api.jianguo.ltd", key);
    }
    // An account with no Key must not keep spending the previous account's balance.
    #[cfg(target_os = "windows")]
    match fs::remove_file(private_root(home).join("credentials.dpapi")) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("清除旧账户插件凭证失败，请在插件页更新凭证后使用".into()),
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/usr/bin/security")
            .args([
                "delete-generic-password",
                "-s",
                "com.jianguo.codex.image-plugin",
                "-a",
                &home.to_string_lossy(),
            ])
            .output()
            .map_err(|_| "清除旧账户插件凭证失败")?;
        if !output.status.success() && output.status.code() != Some(44) {
            return Err("清除旧账户插件凭证失败".into());
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn image_plugin_status() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| status_at(&super::codex_home_dir()?))
        .await
        .map_err(|_| "读取插件状态失败")?
}
fn status_at(home: &Path) -> Result<Value, String> {
    plugin_status_at(home, false)
}
pub(super) fn plugin_status_at(home: &Path, vector: bool) -> Result<Value, String> {
    plugin_status_kind_at(
        home,
        if vector {
            BuiltinKind::Vector
        } else {
            BuiltinKind::Image
        },
    )
}
pub(super) fn plugin_status_kind_at(home: &Path, kind: BuiltinKind) -> Result<Value, String> {
    let name = kind.name();
    let plugin_id = format!("{name}@{MARKET}");
    let cache = home
        .join("plugins/cache")
        .join(MARKET)
        .join(name)
        .join(VERSION);
    let doc = read_config(home)?;
    let enabled = doc
        .get("plugins")
        .and_then(|p| p.get(&plugin_id))
        .and_then(|p| p.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let installed = [
        ".codex-plugin/plugin.json",
        ".mcp.json",
        &format!("skills/{name}/SKILL.md"),
    ]
    .iter()
    .all(|p| cache.join(p).is_file());
    let installed = installed
        && fs::read(cache.join(".mcp.json"))
            .ok()
            .and_then(|data| serde_json::from_slice::<Value>(&data).ok())
            .and_then(|data| {
                data["mcpServers"][kind.server()]["command"]
                    .as_str()
                    .map(PathBuf::from)
            })
            .is_some_and(|path| path.is_file());
    let key = credentials(home).ok();
    Ok(
        json!({"installed":installed,"enabled":enabled,"credentialReady":key.is_some(),"version":VERSION,"baseUrl":"https://api.jianguo.ltd"}),
    )
}
fn read_config(home: &Path) -> Result<DocumentMut, String> {
    match fs::read_to_string(home.join("config.toml")) {
        Ok(s) => s
            .trim_start_matches('\u{feff}')
            .parse()
            .map_err(|_| "Codex 配置格式无效，请修复后安装插件".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(_) => Err("无法读取 Codex 配置".into()),
    }
}
fn package_files(
    home: &Path,
    host: &Path,
    kind: BuiltinKind,
    include_cache: bool,
) -> Vec<(PathBuf, Vec<u8>)> {
    let name = kind.name();
    let (manifest, skill, agent) = match kind {
        BuiltinKind::Image => (MANIFEST, SKILL, AGENT),
        BuiltinKind::Vector => (
            super::vector_plugin::MANIFEST,
            super::vector_plugin::SKILL,
            super::vector_plugin::AGENT,
        ),
        BuiltinKind::Video => (
            super::video_plugin::MANIFEST,
            super::video_plugin::SKILL,
            super::video_plugin::AGENT,
        ),
    };
    let mcp =
        json!({"mcpServers":{kind.server():{"command":host,"args":[kind.flag(),"--home",home]}}})
            .to_string();
    let mut files = Vec::new();
    let mut roots = vec![market_root(home).join("plugins").join(name)];
    if include_cache {
        roots.push(
            home.join("plugins/cache")
                .join(MARKET)
                .join(name)
                .join(VERSION),
        );
    }
    for root in roots {
        for (path, data) in [
            (".codex-plugin/plugin.json".to_owned(), manifest),
            (".mcp.json".to_owned(), &mcp),
            (format!("skills/{name}/SKILL.md"), skill),
            (format!("skills/{name}/agents/openai.yaml"), agent),
        ] {
            files.push((root.join(path), data.as_bytes().to_vec()));
        }
    }
    files
}
fn install_at(home: &Path, executable: &Path, base: &str, key: &str) -> Result<Value, String> {
    install_plugin_kind_at(home, executable, base, key, BuiltinKind::Image)
}
pub(super) fn install_plugin_at(
    home: &Path,
    executable: &Path,
    base: &str,
    key: &str,
    vector: bool,
) -> Result<Value, String> {
    install_plugin_kind_at(
        home,
        executable,
        base,
        key,
        if vector {
            BuiltinKind::Vector
        } else {
            BuiltinKind::Image
        },
    )
}
pub(super) fn install_plugin_kind_at(
    home: &Path,
    executable: &Path,
    base: &str,
    key: &str,
    kind: BuiltinKind,
) -> Result<Value, String> {
    normalize_base(base)?;
    let mut doc = read_config(home)?;
    // Parse config before saving any credentials or package files.
    for section in ["plugins", "marketplaces"] {
        if let Some(item) = doc.get_mut(section) {
            *item = toml_edit::Item::Table(
                item.clone()
                    .into_table()
                    .map_err(|_| "Codex 插件配置不是有效配置表")?,
            );
        }
    }
    let bytes = fs::read(executable).map_err(|_| "无法读取内置插件运行程序")?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let host = private_root(home).join(format!(
        "host-{}{}",
        &hash[..16],
        std::env::consts::EXE_SUFFIX
    ));
    if fs::read(&host)
        .ok()
        .is_none_or(|data| format!("{:x}", Sha256::digest(&data)) != hash)
    {
        super::provider_config::commit(vec![(host.clone(), bytes)])?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&host, fs::Permissions::from_mode(0o700))
                .map_err(|_| "设置插件运行权限失败")?;
        }
    }
    save_credentials(home, base, key)?;
    doc["plugins"][format!("{}@{MARKET}", kind.name())]["enabled"] = value(true);
    doc["marketplaces"][MARKET]["source_type"] = value("local");
    doc["marketplaces"][MARKET]["source"] = value(market_root(home).to_string_lossy().as_ref());
    // Keep every marketplace entry valid while only the selected plugin gets a cache.
    let mut files = Vec::new();
    for package_kind in [BuiltinKind::Image, BuiltinKind::Vector, BuiltinKind::Video] {
        files.extend(package_files(
            home,
            &host,
            package_kind,
            package_kind == kind,
        ));
    }
    let entries: Vec<Value> = [BuiltinKind::Image, BuiltinKind::Vector, BuiltinKind::Video]
        .into_iter()
        .map(|entry| {
            json!({"name":entry.name(),"source":{"source":"local","path":format!("./plugins/{}",entry.name())},"policy":{"installation":"AVAILABLE","authentication":"ON_INSTALL"},"category":"Creativity"})
        })
        .collect();
    files.push((
        market_root(home).join(".agents/plugins/marketplace.json"),
        json!({"name":MARKET,"interface":{"displayName":"坚果内置插件"},"plugins":entries})
            .to_string()
            .into_bytes(),
    ));
    files.push((home.join("config.toml"), doc.to_string().into_bytes()));
    super::provider_config::commit(files)?;
    plugin_status_kind_at(home, kind)
}
#[tauri::command]
pub async fn install_image_plugin(api_key: String, base_url: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = super::PROVIDER_CONFIG_LOCK
            .lock()
            .map_err(|_| "配置正在更新，请稍后重试")?;
        #[cfg(target_os = "windows")]
        let installed = super::detect_codex_windows().installed;
        #[cfg(not(target_os = "windows"))]
        let installed = super::detect_codex_macos().installed;
        if !installed {
            return Err("请先安装 Codex，再安装生图插件".into());
        }
        install_at(
            &super::codex_home_dir()?,
            &std::env::current_exe().map_err(|_| "无法定位插件运行程序")?,
            &base_url,
            &api_key,
        )
    })
    .await
    .map_err(|_| "安装生图插件失败")?
}

#[tauri::command]
pub async fn uninstall_builtin_plugin(plugin: String) -> Result<Value, String> {
    let kind = match plugin.as_str() {
        "image" => BuiltinKind::Image,
        "vector" => BuiltinKind::Vector,
        "video" => BuiltinKind::Video,
        _ => return Err("不支持卸载此插件".into()),
    };
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = super::PROVIDER_CONFIG_LOCK
            .lock()
            .map_err(|_| "配置正在更新，请稍后重试")?;
        uninstall_at(&super::codex_home_dir()?, kind)
    })
    .await
    .map_err(|_| "卸载插件失败")?
}
fn uninstall_at(home: &Path, kind: BuiltinKind) -> Result<Value, String> {
    let name = kind.name();
    let plugin_id = format!("{name}@{MARKET}");
    let mut doc = read_config(home)?;
    if let Some(plugins) = doc.get_mut("plugins") {
        plugins
            .as_table_like_mut()
            .ok_or("Codex 插件配置格式无效，尚未卸载")?
            .remove(&plugin_id);
    }
    let cache = home.join("plugins/cache").join(MARKET).join(name);
    let mut staged = None;
    if cache.exists() {
        // Only move this exact built-in plugin's cache; never follow a cache
        // junction out of the user's Codex home or touch the other plugin.
        let real_home = home.canonicalize().map_err(|_| "无法定位 Codex 配置目录")?;
        let real_cache = cache.canonicalize().map_err(|_| "无法定位插件文件")?;
        if !real_cache.starts_with(real_home.join("plugins/cache")) {
            return Err("插件目录指向了其他位置，尚未卸载".into());
        }
        let trash = private_root(home).join("uninstalled");
        fs::create_dir_all(&trash).map_err(|_| "无法准备卸载目录")?;
        if !trash
            .canonicalize()
            .map_err(|_| "卸载目录无效")?
            .starts_with(&real_home)
        {
            return Err("卸载目录不在 Codex 用户目录内".into());
        }
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).map_err(|_| "无法准备插件卸载")?;
        let moved = trash.join(format!("{name}-{:x}", u64::from_le_bytes(nonce)));
        fs::rename(&cache, &moved).map_err(|_| "无法移除插件文件，请重启 Codex 后再卸载")?;
        staged = Some(moved);
    }
    if let Err(error) = super::provider_config::commit(vec![(
        home.join("config.toml"),
        doc.to_string().into_bytes(),
    )]) {
        if let Some(path) = &staged {
            if fs::rename(path, &cache).is_err() {
                return Err(format!("{error}；插件文件已保留在卸载备份中，请刷新后重试"));
            }
        }
        return Err(error);
    }
    // Credentials, job history, generated files and the shared host are kept.
    // The marketplace source remains available for a later reinstall.
    if let Some(path) = staged {
        let _ = fs::remove_dir_all(path);
    }
    plugin_status_kind_at(home, kind)
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
    mime_type: Option<String>,
    #[serde(default)]
    submitted_at: i64,
}
fn job_path(home: &Path, id: &str) -> Result<PathBuf, String> {
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("生图任务编号无效".into());
    }
    Ok(private_root(home).join("jobs").join(format!("{id}.json")))
}
fn save_job(home: &Path, job: &Job) -> Result<(), String> {
    let path = job_path(home, &job.id)?;
    let bytes = serde_json::to_vec(job).map_err(|_| "任务记录编码失败")?;
    let parent = path.parent().ok_or("任务记录目录无效")?;
    fs::create_dir_all(parent).map_err(|_| "无法保存生图任务，请勿重复提交")?;
    let temp = path.with_extension("json.tmp");
    let backup = path.with_extension("json.bak");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temp)
        .map_err(|_| "无法写入生图任务")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "保存生图任务失败，请勿重复提交")?;
    if path.exists() {
        let _ = fs::remove_file(&backup);
        fs::rename(&path, &backup).map_err(|_| "更新生图任务失败；保留原任务，请勿重新生成")?;
    }
    if let Err(error) = fs::rename(&temp, &path) {
        if backup.exists() {
            let _ = fs::rename(&backup, &path);
        }
        return Err(format!("保存生图任务失败：{error}。请勿重新生成。"));
    }
    let _ = fs::remove_file(backup);
    Ok(())
}
pub(super) fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法初始化插件网络连接".into())
}
fn public_image_url(url: &reqwest::Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    // The platform's public result storage currently serves HTTP only. This
    // exception is for result bytes, never API requests or credentials.
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
pub(super) fn bytes(response: reqwest::blocking::Response, limit: u64) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    response
        .take(limit + 1)
        .read_to_end(&mut data)
        .map_err(|_| "读取插件响应失败，请继续查询原任务，不要重复提交")?;
    if data.len() as u64 > limit {
        return Err("图片或接口响应超出大小限制".into());
    }
    Ok(data)
}
pub(super) fn json_response(response: reqwest::blocking::Response) -> Result<Value, String> {
    let status = response.status();
    let data = bytes(response, 4 * 1024 * 1024)?;
    let result: Value = serde_json::from_slice(&data)
        .map_err(|_| format!("插件服务返回了无法识别的响应（HTTP {status}）"))?;
    if !status.is_success() || result["ok"] == false {
        let message = result["message"]
            .as_str()
            .or_else(|| result["error"]["message"].as_str())
            .or_else(|| result["error"].as_str())
            .unwrap_or("插件接口请求失败");
        return Err(format!("{message}（HTTP {status}）"));
    }
    Ok(result)
}
pub(super) fn image_mime(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if data.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}
fn request_body(args: &Value, key: &str) -> Result<Value, String> {
    let model = args["model"]
        .as_str()
        .filter(|v| !v.trim().is_empty())
        .ok_or("请先明确指定生图模型，不会自动选择默认模型")?;
    let prompt = args["prompt"]
        .as_str()
        .filter(|v| !v.trim().is_empty())
        .ok_or("请填写生图提示词")?;
    if model.len() > 256 || prompt.len() > 100_000 {
        return Err("模型名称或提示词过长".into());
    }
    let mut body = json!({"kami":key,"model":model,"prompt":prompt});
    for name in [
        "aspectRatio",
        "imageSize",
        "width",
        "height",
        "quality",
        "transparent",
    ] {
        if let Some(v) = args.get(name) {
            body[name] = v.clone();
        }
    }
    if let Some(size) = body.get("imageSize") {
        if !matches!(size.as_str(), Some("1K" | "2K" | "4K")) {
            return Err("imageSize 仅支持 1K、2K 或 4K".into());
        }
    }
    for name in ["width", "height"] {
        if let Some(v) = body.get(name) {
            if !v.as_u64().is_some_and(|n| (1..=16384).contains(&n)) {
                return Err("图片宽高需为 1 至 16384 的整数".into());
            }
        }
    }
    if let Some(v) = body.get("transparent") {
        if !v.is_boolean() {
            return Err("transparent 需为布尔值".into());
        }
    }
    if let Some(images) = args.get("images") {
        let images = images
            .as_array()
            .filter(|v| v.len() <= 10)
            .ok_or("参考图最多 10 张")?;
        let mut encoded = Vec::new();
        let mut total = 0usize;
        for image in images {
            let text = image
                .as_str()
                .ok_or("参考图需为本机绝对路径或图片 data URL")?;
            let data = if text.starts_with("data:image/") {
                let (header, content) = text.split_once(',').ok_or("参考图 data URL 无效")?;
                if !header.ends_with(";base64")
                    || content.len() > ((MAX_IMAGE * 4 / 3 + 4) as usize)
                {
                    return Err("参考图数据无效或过大".into());
                }
                base64::engine::general_purpose::STANDARD
                    .decode(content)
                    .map_err(|_| "参考图 base64 无效")?
            } else {
                let path = Path::new(text);
                if !path.is_absolute() {
                    return Err("参考图请使用本机绝对路径，不支持远程 URL".into());
                }
                let f = fs::File::open(path).map_err(|_| "无法读取参考图")?;
                let mut data = Vec::new();
                f.take(MAX_IMAGE + 1)
                    .read_to_end(&mut data)
                    .map_err(|_| "参考图读取失败")?;
                data
            };
            total += data.len();
            if data.len() as u64 > MAX_IMAGE || total > MAX_REFERENCE_IMAGES {
                return Err("参考图过大，单张或合计最多 32 MB".into());
            }
            let mime = image_mime(&data).ok_or("参考图需为有效的 PNG、JPEG 或 WebP 图片")?;
            encoded.push(format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(data)
            ));
        }
        body["images"] = json!(encoded);
    }
    Ok(body)
}
pub(super) fn redact(error: &str, key: &str) -> String {
    if key.is_empty() {
        error.into()
    } else {
        error.replace(key, "[已隐藏凭证]")
    }
}
fn submit(
    home: &Path,
    job: &mut Job,
    body: &Value,
    credential: &Credentials,
    client: &reqwest::blocking::Client,
) -> Result<(), String> {
    // No retry: a transport failure does not establish whether the paid POST was accepted.
    let response = client
        .post(format!("{}/api/v1/generate", credential.base_url))
        .json(body)
        .send()
        .map_err(|_| "提交结果未确认，请勿重复生成，以免重复扣点；请稍后检查平台记录")?;
    let response = json_response(response)?;
    match response["type"].as_str() {
        Some("task") => {
            job.task_id = Some(
                response["taskId"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .ok_or("服务未返回有效任务 ID，请勿重复提交")?
                    .to_owned(),
            )
        }
        Some("url") => {
            job.url = Some(
                response["url"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .ok_or("服务未返回图片地址")?
                    .to_owned(),
            )
        }
        _ => return Err("服务返回了未知的生图结果，请检查平台记录，避免重复提交".into()),
    }
    job.state = "pending".into();
    save_job(home, job)
}
fn start_generation(home: &Path, args: &Value) -> Result<Value, String> {
    let credential = credentials(home)?;
    let body = request_body(args, &credential.api_key)?;
    let output = PathBuf::from(
        args["outputFile"]
            .as_str()
            .ok_or("请指定图片保存位置 outputFile（绝对路径）")?,
    );
    if !output.is_absolute() || output.file_name().is_none() || output.exists() {
        return Err("图片保存路径需为尚不存在的绝对文件路径，插件不会覆盖已有文件".into());
    }
    fs::create_dir_all(output.parent().ok_or("图片保存路径无效")?)
        .map_err(|_| "无法创建图片输出目录")?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| "创建生图任务编号失败")?;
    let id = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let mut job = Job {
        id: id.clone(),
        model: body["model"].as_str().unwrap().into(),
        output_file: output,
        state: "submitting".into(),
        task_id: None,
        url: None,
        error: None,
        mime_type: None,
        submitted_at: chrono::Utc::now().timestamp(),
    };
    let client = http_client()?;
    save_job(home, &job)?;
    let home = home.to_owned();
    std::thread::spawn(move || {
        if let Err(error) = submit(&home, &mut job, &body, &credential, &client) {
            job.state = "submission-unconfirmed".into();
            job.error = Some(redact(&error, &credential.api_key));
            let _ = save_job(&home, &job);
        }
    });
    Ok(
        json!({"status":"submitting","jobId":id,"message":"已开始提交。请用 get_image_result 查询原任务，不要重复提交。"}),
    )
}
fn completed(home: &Path, job: &mut Job, data: &[u8]) -> Result<Value, String> {
    let mime = image_mime(data).ok_or("生图结果不是受支持的图片，已保留原任务，请稍后重试查询")?;
    let extension = match mime {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => "png",
    };
    if job.output_file.extension().and_then(|v| v.to_str()) != Some(extension) {
        job.output_file.set_extension(extension);
    }
    if job.output_file.exists() {
        if fs::read(&job.output_file).ok().as_deref() != Some(data) {
            return Err(
                "目标文件已存在，原结果已缓存；请移开冲突文件后继续查询，插件不会覆盖已有文件"
                    .into(),
            );
        }
    } else {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&job.output_file)
            .map_err(|_| "无法保存图片，原结果已缓存；请检查输出目录权限后继续查询")?;
        if file.write_all(data).and_then(|_| file.sync_all()).is_err() {
            drop(file);
            let _ = fs::remove_file(&job.output_file);
            return Err("保存图片失败，原结果已缓存，请检查磁盘空间".into());
        }
    }
    job.state = "completed".into();
    job.mime_type = Some(mime.into());
    save_job(home, job)?;
    Ok(
        json!({"status":"completed","jobId":job.id,"model":job.model,"filePath":job.output_file,"mimeType":mime}),
    )
}
fn result(
    home: &Path,
    id: &str,
    client: &reqwest::blocking::Client,
    base: &str,
) -> Result<Value, String> {
    let mut job: Job = serde_json::from_slice(
        &fs::read(job_path(home, id)?).map_err(|_| "未找到此生图任务，请核对 jobId")?,
    )
    .map_err(|_| "任务记录格式无效")?;
    if job.state == "completed" && job.output_file.is_file() {
        return Ok(
            json!({"status":"completed","jobId":id,"model":job.model,"filePath":job.output_file,"mimeType":job.mime_type}),
        );
    }
    if job.state == "completed" {
        job.state = "pending".into();
    }
    if job.state != "pending" {
        if job.state == "submitting" && chrono::Utc::now().timestamp() - job.submitted_at > 240 {
            return Ok(
                json!({"status":"submission-unconfirmed","jobId":id,"message":"提交超时或连接曾中断，无法确认结果；请检查平台记录，不要自动重新生成。"}),
            );
        }
        return Ok(
            json!({"status":job.state,"jobId":id,"message":job.error,"retryAfterSeconds":3}),
        );
    }
    let cached = job_path(home, id)?.with_extension("image");
    if cached.is_file() {
        return completed(
            home,
            &mut job,
            &fs::read(&cached).map_err(|_| "无法读取已缓存图片")?,
        );
    }
    // A saved result URL is reusable. Do not poll a consumptive task endpoint
    // again after a download interruption or a plugin upgrade.
    if let Some(task) = job.task_id.as_ref().filter(|_| job.url.is_none()) {
        let mut url = reqwest::Url::parse(base).map_err(|_| "坚果地址无效")?;
        url.path_segments_mut()
            .map_err(|_| "坚果地址无效")?
            .extend(["api", "v1", "task", task]);
        let response = client
            .get(url)
            .send()
            .map_err(|_| "查询任务暂时失败，请稍后继续查询原任务")?;
        let status = response.status();
        if status == reqwest::StatusCode::ACCEPTED
            || status.is_server_error()
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            return Ok(json!({"status":"pending","jobId":id,"retryAfterSeconds":3}));
        }
        let data = bytes(response, MAX_IMAGE)?;
        if status == reqwest::StatusCode::GONE {
            job.state = "failed".into();
            job.error = Some(String::from_utf8_lossy(&data[..data.len().min(2048)]).to_string());
            save_job(home, &job)?;
            return Ok(json!({"status":"failed","jobId":id,"message":job.error}));
        }
        if !status.is_success() {
            return Err(format!("查询任务失败（HTTP {status}），请保留原任务编号"));
        }
        if image_mime(&data).is_some() {
            super::provider_config::commit(vec![(cached, data.clone())])?;
            return completed(home, &mut job, &data);
        }
        let value: Value =
            serde_json::from_slice(&data).map_err(|_| "任务返回未知格式，请稍后继续查询")?;
        if value["type"] == "pending" {
            return Ok(json!({"status":"pending","jobId":id,"retryAfterSeconds":3}));
        }
        job.url = Some(
            value["url"]
                .as_str()
                .filter(|v| !v.is_empty())
                .ok_or("任务未返回图片地址")?
                .into(),
        );
        save_job(home, &job)?;
    }
    let url = reqwest::Url::parse(job.url.as_deref().ok_or("任务没有图片结果")?)
        .map_err(|_| "图片地址无效")?;
    if !public_image_url(&url) {
        return Err("图片下载地址无效或不受支持，已保留原任务".into());
    }
    let response = download(client, url)?;
    if !response.status().is_success() {
        return Err("图片下载暂时失败，可继续查询原任务".into());
    }
    let data = bytes(response, MAX_IMAGE)?;
    if image_mime(&data).is_none() {
        return Err("下载结果不是 PNG、JPEG 或 WebP 图片，请继续查询原任务".into());
    }
    super::provider_config::commit(vec![(cached, data.clone())])?;
    completed(home, &mut job, &data)
}
pub(super) fn download(
    client: &reqwest::blocking::Client,
    mut url: reqwest::Url,
) -> Result<reqwest::blocking::Response, String> {
    let storage_client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法初始化图片下载连接")?;
    for _ in 0..=5 {
        if !public_image_url(&url) {
            return Err("图片下载地址无效或不受支持".into());
        }
        let response = (if url.scheme() == "http" {
            &storage_client
        } else {
            client
        })
        .get(url.clone())
        .send()
        .map_err(|_| "图片下载失败，可继续查询原任务，不要重复生成")?;
        if !response.status().is_redirection() {
            return Ok(response);
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|h| h.to_str().ok())
            .ok_or("图片重定向地址缺失")?;
        url = url.join(location).map_err(|_| "图片重定向地址无效")?;
    }
    Err("图片下载重定向次数过多".into())
}

fn tools() -> Value {
    json!([
        {"name":"list_image_models","description":"查询坚果生图模型、ID、尺寸及价格，保留接口顺序。不要自动选择列表首项。","inputSchema":{"type":"object","properties":{},"additionalProperties":false},"annotations":{"readOnlyHint":true}},
        {"name":"get_image_pricing","description":"查询坚果计费价目，生图键为 image.<模型>。","inputSchema":{"type":"object","properties":{},"additionalProperties":false},"annotations":{"readOnlyHint":true}},
        {"name":"generate_image","description":"按用户明确指定的生图模型提交一次付费生成。返回 jobId 后继续查询该任务，禁止在超时后自动重提。","inputSchema":{"type":"object","required":["model","prompt","outputFile"],"additionalProperties":false,"properties":{"model":{"type":"string","description":"用户明确指定的生图模型 ID，无默认值"},"prompt":{"type":"string"},"outputFile":{"type":"string","description":"新图片绝对保存路径，不覆盖已有文件"},"aspectRatio":{"type":"string"},"imageSize":{"type":"string","enum":["1K","2K","4K"]},"width":{"type":"integer","minimum":1,"maximum":16384},"height":{"type":"integer","minimum":1,"maximum":16384},"quality":{"type":"string"},"transparent":{"type":"boolean"},"images":{"type":"array","maxItems":10,"items":{"type":"string"},"description":"参考图绝对路径或图片 data URL，会上传到坚果平台"}}},"annotations":{"readOnlyHint":false,"idempotentHint":false}},
        {"name":"get_image_result","description":"查询既有 jobId 并保存结果。pending 时等待约 3 秒再查，最长 10 分钟。完成后重复查询复用本机图片。","inputSchema":{"type":"object","required":["jobId"],"properties":{"jobId":{"type":"string"}},"additionalProperties":false},"annotations":{"readOnlyHint":false,"idempotentHint":true}}
    ])
}
fn call_tool(home: &Path, name: &str, args: &Value) -> Result<Value, String> {
    if name == "generate_image" {
        return start_generation(home, args);
    }
    let credential = credentials(home)?;
    let client = http_client()?;
    let result = match name {
        "list_image_models" => json_response(
            client
                .get(format!("{}/api/v1/models/image", credential.base_url))
                .send()
                .map_err(|_| "生图模型列表暂时无法连接，请检查网络")?,
        ),
        "get_image_pricing" => json_response(
            client
                .get(format!("{}/api/v1/pricing", credential.base_url))
                .send()
                .map_err(|_| "计费价目暂时无法连接，请检查网络")?,
        ),
        "get_image_result" => result(
            home,
            args["jobId"].as_str().ok_or("缺少 jobId")?,
            &client,
            &credential.base_url,
        ),
        _ => Err("不支持的生图工具".into()),
    };
    result
        .map(|mut value| {
            redact_value(&mut value, &credential.api_key);
            value
        })
        .map_err(|error| redact(&error, &credential.api_key))
}
pub(super) fn redact_value(value: &mut Value, key: &str) {
    match value {
        Value::String(text) => *text = redact(text, key),
        Value::Array(values) => values.iter_mut().for_each(|v| redact_value(v, key)),
        Value::Object(values) => values.values_mut().for_each(|v| redact_value(v, key)),
        _ => {}
    }
}
#[cfg(test)]
fn dispatch(home: &Path, request: Value) -> Option<Value> {
    dispatch_plugin(home, request, BuiltinKind::Image)
}
fn dispatch_plugin(home: &Path, request: Value, kind: BuiltinKind) -> Option<Value> {
    let id = request.get("id")?.clone();
    let result = match request["method"].as_str().unwrap_or("") {
        "initialize" => {
            let instructions = match kind {
                BuiltinKind::Image => {
                    "通过坚果 API 生图。必须由用户明确指定生图模型；不要默认选择模型。付费提交后查询原 jobId，不自动重提。"
                }
                BuiltinKind::Vector => {
                    "将本机位图转换成 SVG。使用助手绑定的当前坚果凭证；付费提交后只查询原 jobId，禁止自动重提。"
                }
                BuiltinKind::Video => {
                    "通过坚果 API 生成视频。必须由用户明确指定视频模型；不要默认选择模型。付费任务提交后只查询原任务，不自动重提。"
                }
            };
            json!({"protocolVersion":request["params"]["protocolVersion"].as_str().unwrap_or("2024-11-05"),"serverInfo":{"name":kind.name(),"version":VERSION},"capabilities":{"tools":{}},"instructions":instructions})
        }
        "ping" => json!({}),
        "tools/list" => {
            json!({"tools":match kind {
                BuiltinKind::Image => tools(),
                BuiltinKind::Vector => super::vector_plugin::tools(),
                BuiltinKind::Video => super::video_plugin::tools(),
            }})
        }
        "tools/call" => {
            let tool = match kind {
                BuiltinKind::Image => call_tool,
                BuiltinKind::Vector => super::vector_plugin::call_tool,
                BuiltinKind::Video => super::video_plugin::call_tool,
            };
            match tool(
                home,
                request["params"]["name"].as_str().unwrap_or(""),
                &request["params"]["arguments"],
            ) {
                Ok(data) => {
                    json!({"content":[{"type":"text","text":data.to_string()}],"isError":false})
                }
                Err(error) => json!({"content":[{"type":"text","text":error}],"isError":true}),
            }
        }
        _ => {
            return Some(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}}),
            );
        }
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}
pub fn run_if_requested() -> Option<i32> {
    let args = std::env::args().collect::<Vec<_>>();
    let kind = kind_from_flag(args.get(1).map(String::as_str).unwrap_or(""))?;
    let Some(home) = args
        .get(3)
        .filter(|_| args.get(2).map(String::as_str) == Some("--home"))
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    else {
        return Some(2);
    };
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    loop {
        let mut line = String::new();
        match (&mut input).take(48 * 1024 * 1024).read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => return Some(1),
        };
        if line.len() >= 48 * 1024 * 1024 {
            return Some(1);
        }
        let response = match serde_json::from_str(&line) {
            Ok(request) => dispatch_plugin(&home, request, kind),
            Err(_) => Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}),
            ),
        };
        if let Some(response) = response {
            if writeln!(output, "{response}")
                .and_then(|_| output.flush())
                .is_err()
            {
                break;
            }
        }
    }
    Some(0)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::net::TcpListener;
    pub(crate) fn fixture_home() -> PathBuf {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).unwrap();
        let path = std::env::temp_dir().join(format!(
            "jianguo-image-test-{:x}",
            u64::from_le_bytes(random)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    pub(crate) fn png() -> Vec<u8> {
        base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aDakAAAAASUVORK5CYII=").unwrap()
    }
    fn fixture_job(home: &Path) -> Job {
        Job {
            id: "0123456789abcdef0123456789abcdef".into(),
            model: "user-chosen-model".into(),
            output_file: home.join("poster.png"),
            state: "submitting".into(),
            task_id: None,
            url: None,
            error: None,
            mime_type: None,
            submitted_at: chrono::Utc::now().timestamp(),
        }
    }
    pub(crate) fn server(
        responses: Vec<(u16, &'static str, Vec<u8>)>,
    ) -> (String, std::thread::JoinHandle<Vec<(String, String)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, kind, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut headers = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                    headers.push_str(&line);
                }
                let mut input = vec![0; length];
                reader.read_exact(&mut input).unwrap();
                requests.push((headers, String::from_utf8_lossy(&input).into()));
                write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
            requests
        });
        (base, handle)
    }
    pub(crate) fn client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }
    #[test]
    fn explicit_model_and_valid_reference_data_are_required_and_optional_fields_preserved() {
        assert!(request_body(&json!({"prompt":"poster"}), "secret").is_err());
        assert!(request_body(&json!({"model":"nano-banana-2","prompt":"poster","images":["https://example.com/reference.png"]}),"secret").is_err());
        assert!(request_body(&json!({"model":"nano-banana-2","prompt":"poster","images":["data:image/png;base64,not-an-image"]}),"secret").is_err());
        let data = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png())
        );
        let body=request_body(&json!({"model":"nano-banana-2","prompt":"poster","aspectRatio":"1:1","imageSize":"2K","width":1024,"height":1024,"quality":"high","transparent":true,"images":[data],"kami":"attacker-key"}),"real-key").unwrap();
        assert_eq!(body["kami"], "real-key");
        assert_eq!(body["model"], "nano-banana-2");
        assert_eq!(body["imageSize"], "2K");
        assert_eq!(body["transparent"], true);
        assert_eq!(body["images"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn submit_poll_pending_and_png_save_do_not_repeat_paid_submission() {
        let home = fixture_home();
        let mut job = fixture_job(&home);
        let (base, handle) = server(vec![
            (
                200,
                "application/json",
                br#"{"ok":true,"type":"task","taskId":"task-123"}"#.to_vec(),
            ),
            (202, "application/json", br#"{"type":"pending"}"#.to_vec()),
            (200, "image/png", png()),
        ]);
        let credential = Credentials {
            base_url: base.clone(),
            api_key: "fixture-key".into(),
        };
        let body = request_body(
            &json!({"model":"chosen-image-model","prompt":"poster"}),
            &credential.api_key,
        )
        .unwrap();
        let client = client();
        submit(&home, &mut job, &body, &credential, &client).unwrap();
        assert_eq!(
            result(&home, &job.id, &client, &base).unwrap()["status"],
            "pending"
        );
        let completed = result(&home, &job.id, &client, &base).unwrap();
        assert_eq!(completed["status"], "completed");
        assert_eq!(fs::read(&job.output_file).unwrap(), png());
        assert_eq!(result(&home, &job.id, &client, &base).unwrap(), completed);
        fs::remove_file(&job.output_file).unwrap();
        assert_eq!(result(&home, &job.id, &client, &base).unwrap(), completed);
        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[0].0.starts_with("POST /api/v1/generate "));
        assert_eq!(
            serde_json::from_str::<Value>(&requests[0].1).unwrap()["kami"],
            "fixture-key"
        );
        for (headers, _) in &requests {
            assert!(!headers.to_lowercase().contains("authorization:"));
            assert!(!headers.contains("fixture-key"));
        }
        assert!(requests[1].0.starts_with("GET /api/v1/task/task-123 "));
        assert!(
            !fs::read_to_string(job_path(&home, &job.id).unwrap())
                .unwrap()
                .contains("fixture-key")
        );
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn direct_url_results_are_retained_and_unsafe_download_targets_rejected() {
        let home = fixture_home();
        let mut job = fixture_job(&home);
        let (base, handle) = server(vec![(
            200,
            "application/json",
            br#"{"ok":true,"type":"url","url":"https://images.example.com/result.png"}"#.to_vec(),
        )]);
        let credential = Credentials {
            base_url: base,
            api_key: "fixture-key".into(),
        };
        submit(
            &home,
            &mut job,
            &json!({"kami":"fixture-key","model":"chosen","prompt":"poster"}),
            &credential,
            &client(),
        )
        .unwrap();
        handle.join().unwrap();
        assert_eq!(
            job.url.as_deref(),
            Some("https://images.example.com/result.png")
        );
        assert!(public_image_url(
            &reqwest::Url::parse(job.url.as_ref().unwrap()).unwrap()
        ));
        for url in [
            "http://images.example.com/a.png",
            "http://download.jianguo.ltd.evil.test/a.png",
            "http://secret@download.jianguo.ltd/a.png",
            "http://download.jianguo.ltd:8080/a.png",
            "https://127.0.0.1/a.png",
            "https://10.0.0.1/a.png",
            "https://[::1]/a.png",
            "https://key@example.com/a.png",
        ] {
            assert!(!public_image_url(&reqwest::Url::parse(url).unwrap()));
        }
        assert!(public_image_url(
            &reqwest::Url::parse("http://download.jianguo.ltd/image/result.png").unwrap()
        ));
        for base in [
            "https://evil.example.com/v1",
            "https://api.jianguo.ltd.evil.test/v1",
            "http://api.jianguo.ltd",
            "https://secret@api.jianguo.ltd/v1",
        ] {
            assert!(normalize_base(base).is_err());
        }
        assert_eq!(
            normalize_base("https://api.jianguo.ltd/v1").unwrap(),
            "https://api.jianguo.ltd"
        );
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn cached_results_do_not_overwrite_existing_files_and_can_be_recovered() {
        let home = fixture_home();
        let mut job = fixture_job(&home);
        job.state = "pending".into();
        save_job(&home, &job).unwrap();
        fs::write(
            job_path(&home, &job.id).unwrap().with_extension("image"),
            png(),
        )
        .unwrap();
        fs::write(&job.output_file, b"user-owned").unwrap();
        assert!(result(&home, &job.id, &client(), "http://127.0.0.1:1").is_err());
        assert_eq!(fs::read(&job.output_file).unwrap(), b"user-owned");
        fs::remove_file(&job.output_file).unwrap();
        assert_eq!(
            result(&home, &job.id, &client(), "http://127.0.0.1:1").unwrap()["status"],
            "completed"
        );
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn saved_result_url_does_not_poll_the_consumptive_task_endpoint_again() {
        let home = fixture_home();
        let mut job = fixture_job(&home);
        job.state = "pending".into();
        job.task_id = Some("already-delivered-task".into());
        job.url = Some("http://127.0.0.1/blocked.png".into());
        save_job(&home, &job).unwrap();
        // If the old task branch runs, this would fail to connect to port 1.
        let error = result(&home, &job.id, &client(), "http://127.0.0.1:1").unwrap_err();
        assert!(error.contains("图片下载地址"), "{error}");
        let retained: Job =
            serde_json::from_slice(&fs::read(job_path(&home, &job.id).unwrap()).unwrap()).unwrap();
        assert_eq!(retained.url, job.url);
        assert_eq!(retained.task_id, job.task_id);
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn failed_and_interrupted_tasks_are_terminal_without_automatic_resubmission() {
        let home = fixture_home();
        let mut job = fixture_job(&home);
        job.state = "pending".into();
        job.task_id = Some("failed-task".into());
        save_job(&home, &job).unwrap();
        let (base, handle) = server(vec![(410, "text/plain", b"generation failed".to_vec())]);
        let client = client();
        let value = result(&home, &job.id, &client, &base).unwrap();
        assert_eq!(value["status"], "failed");
        assert_eq!(
            result(&home, &job.id, &client, &base).unwrap()["status"],
            "failed"
        );
        assert_eq!(handle.join().unwrap().len(), 1);
        job.state = "submitting".into();
        job.submitted_at = chrono::Utc::now().timestamp() - 300;
        save_job(&home, &job).unwrap();
        assert_eq!(
            result(&home, &job.id, &client, &base).unwrap()["status"],
            "submission-unconfirmed"
        );
        fs::remove_dir_all(home).unwrap();
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn install_registers_real_codex_plugin_and_keeps_credentials_out_of_manifests() {
        let home = fixture_home();
        let executable = home.join("fixture.exe");
        fs::write(&executable, b"test-host-executable").unwrap();
        fs::write(
            home.join("config.toml"),
            "model = 'existing-model'\n[plugins.'existing@market']\nenabled=true\n",
        )
        .unwrap();
        for key in ["fixture-first-key", "fixture-second-key"] {
            let status = install_at(&home, &executable, "https://api.jianguo.ltd/v1", key).unwrap();
            assert_eq!(status["installed"], true);
            assert_eq!(status["enabled"], true);
            assert_eq!(status["credentialReady"], true);
            assert_eq!(credentials(&home).unwrap().api_key, key);
            let doc = read_config(&home).unwrap();
            assert_eq!(doc["model"].as_str(), Some("existing-model"));
            assert_eq!(
                doc["plugins"]["existing@market"]["enabled"].as_bool(),
                Some(true)
            );
            for path in [
                home.join("config.toml"),
                cache_root(&home).join(".codex-plugin/plugin.json"),
                cache_root(&home).join(".mcp.json"),
                cache_root(&home).join("skills/jianguo-image/SKILL.md"),
            ] {
                assert!(!fs::read_to_string(path).unwrap().contains(key));
            }
        }
        let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
        assert_eq!(manifest["interface"]["displayName"], "文生图");
        let mcp: Value =
            serde_json::from_slice(&fs::read(cache_root(&home).join(".mcp.json")).unwrap())
                .unwrap();
        assert!(
            Path::new(
                mcp["mcpServers"]["jianguo_image"]["command"]
                    .as_str()
                    .unwrap()
            )
            .is_file()
        );
        let notification = dispatch(
            &home,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        );
        assert!(notification.is_none());
        let listed =
            dispatch(&home, json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).unwrap();
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 4);
        fs::remove_dir_all(home).unwrap();
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn update_and_uninstall_are_independent_and_preserve_generated_files_and_account() {
        let home = fixture_home();
        let executable = home.join("fixture.exe");
        fs::write(&executable, b"host-v1").unwrap();
        fs::write(
            home.join("config.toml"),
            "model='keep-model'\n[plugins.'other@market']\nenabled=true\n",
        )
        .unwrap();
        install_plugin_at(
            &home,
            &executable,
            "https://api.jianguo.ltd",
            "current-key",
            false,
        )
        .unwrap();
        assert_eq!(plugin_status_at(&home, true).unwrap()["installed"], false);
        install_plugin_at(
            &home,
            &executable,
            "https://api.jianguo.ltd",
            "current-key",
            true,
        )
        .unwrap();
        let image_cache = cache_root(&home);
        let vector_cache = home.join("plugins/cache/jianguo-assistant/jianguo-vectorize/1.0.0");
        let vector_mcp = fs::read(vector_cache.join(".mcp.json")).unwrap();
        fs::write(
            image_cache.join("skills/jianguo-image/SKILL.md"),
            b"old plugin content",
        )
        .unwrap();
        fs::write(&executable, b"host-v2").unwrap();
        install_plugin_at(
            &home,
            &executable,
            "https://api.jianguo.ltd",
            "updated-current-key",
            false,
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(image_cache.join("skills/jianguo-image/SKILL.md")).unwrap(),
            SKILL
        );
        assert_eq!(
            fs::read(vector_cache.join(".mcp.json")).unwrap(),
            vector_mcp,
            "updating image plugin must not replace installed vector plugin"
        );
        let mcp: Value =
            serde_json::from_slice(&fs::read(image_cache.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(
            fs::read(
                mcp["mcpServers"]["jianguo_image"]["command"]
                    .as_str()
                    .unwrap()
            )
            .unwrap(),
            b"host-v2"
        );
        fs::write(home.join("poster.png"), png()).unwrap();
        let job = fixture_job(&home);
        save_job(&home, &job).unwrap();
        let status = uninstall_at(&home, BuiltinKind::Image).unwrap();
        assert_eq!(status["installed"], false);
        assert_eq!(status["enabled"], false);
        assert!(!image_cache.exists());
        assert!(vector_cache.is_dir());
        assert!(home.join("poster.png").is_file());
        assert!(job_path(&home, &job.id).unwrap().is_file());
        assert_eq!(credentials(&home).unwrap().api_key, "updated-current-key");
        let config = read_config(&home).unwrap();
        assert_eq!(config["model"].as_str(), Some("keep-model"));
        assert_eq!(
            config["plugins"]["other@market"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(plugin_status_at(&home, true).unwrap()["enabled"], true);
        assert_eq!(
            uninstall_at(&home, BuiltinKind::Image).unwrap()["installed"],
            false,
            "uninstall is idempotent"
        );
        install_plugin_at(
            &home,
            &executable,
            "https://api.jianguo.ltd",
            "current-key",
            false,
        )
        .unwrap();
        assert_eq!(plugin_status_at(&home, false).unwrap()["enabled"], true);
        uninstall_at(&home, BuiltinKind::Vector).unwrap();
        assert_eq!(plugin_status_at(&home, false).unwrap()["installed"], true);
        assert_eq!(plugin_status_at(&home, true).unwrap()["installed"], false);
        uninstall_at(&home, BuiltinKind::Image).unwrap();
        assert!(home.join("poster.png").is_file());
        fs::remove_dir_all(home).unwrap();
    }
}
