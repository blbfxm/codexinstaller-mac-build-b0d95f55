//! Community packages are reviewed remotely and installed as isolated native Codex plugins.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    time::Duration,
};
use toml_edit::{DocumentMut, value};

const API: &str = "https://api.jianguo.ltd/api/v1/codex-plugins";
const MB: usize = 1024 * 1024;
static OPERATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn error() -> String {
    "无法连接插件市场，请检查网络后重试".into()
}
fn id(payload: &Value) -> Result<u64, String> {
    payload["id"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or("插件编号无效".into())
}
pub(crate) fn config(home: &Path) -> Result<DocumentMut, String> {
    match fs::read_to_string(home.join("config.toml")) {
        Ok(text) => text
            .trim_start_matches('\u{feff}')
            .parse()
            .map_err(|_| "Codex 配置格式异常，请修复后安装插件".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(_) => Err("无法读取 Codex 配置".into()),
    }
}
fn root(home: &Path, id: u64) -> PathBuf {
    home.join(".usage-assistant/community").join(id.to_string())
}
fn safe_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 240 || path.starts_with('/') || path.contains(['\\', ':']) {
        return false;
    }
    path.split('/').all(|s| {
        let stem = s.split('.').next().unwrap_or("").to_ascii_lowercase();
        !s.is_empty()
            && s != "."
            && s != ".."
            && !s.ends_with(['.', ' '])
            && !s.chars().any(|c| c < ' ' || "<>\"|?*".contains(c))
            && !["con", "prn", "aux", "nul"].contains(&stem.as_str())
            && !(stem.len() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && stem.as_bytes()[3].is_ascii_digit())
    })
}
fn slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn version(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 60
        && s.as_bytes()[0].is_ascii_digit()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".+_-".contains(&b))
}
fn package(
    bytes: &[u8],
    expected_slug: &str,
    expected_version: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    if bytes.len() > 10 * MB {
        return Err("插件压缩包超过 10 MB".into());
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "插件压缩包损坏")?;
    if zip.len() > 400 {
        return Err("插件文件过多".into());
    }
    let mut result = Vec::new();
    let mut names = HashSet::new();
    let mut total = 0;
    for index in 0..zip.len() {
        let file = zip.by_index(index).map_err(|_| "无法读取插件文件")?;
        let path = file.name().trim_end_matches('/').to_owned();
        if !safe_path(&path)
            || !names.insert(path.to_lowercase())
            || file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err("插件包含不安全路径或重名文件".into());
        }
        if file.is_dir() {
            continue;
        }
        let mut content = Vec::new();
        file.take((2 * MB + 1) as u64)
            .read_to_end(&mut content)
            .map_err(|_| "插件文件读取失败")?;
        total += content.len();
        if content.len() > 2 * MB || total > 20 * MB {
            return Err("插件解压大小超过限制".into());
        }
        result.push((path, content));
    }
    let files: HashMap<_, _> = result.iter().map(|(p, b)| (p.to_lowercase(), b)).collect();
    for (path, _) in &result {
        for (at, _) in path.match_indices('/') {
            if files.contains_key(&path[..at].to_lowercase()) {
                return Err("插件文件路径冲突".into());
            }
        }
    }
    let manifest: Value = serde_json::from_slice(
        files
            .get(".codex-plugin/plugin.json")
            .ok_or("插件缺少清单")?,
    )
    .map_err(|_| "插件清单损坏")?;
    if manifest["name"] != expected_slug
        || manifest
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("1.0.0")
            != expected_version
    {
        return Err("插件内容与发布信息不一致".into());
    }
    Ok(result)
}
// Refuse symlink/junction ancestors before writing community-controlled paths.
pub(crate) fn regular_ancestors(home: &Path, target: &Path) -> Result<(), String> {
    if !target.starts_with(home) {
        return Err("插件目标位置无效".into());
    }
    let mut current = Some(target);
    while let Some(path) = current {
        if let Ok(meta) = fs::symlink_metadata(path) {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err("插件安装位置不能使用链接目录".into());
                }
            }
            if meta.file_type().is_symlink() {
                return Err("插件安装位置不能使用链接目录".into());
            }
        }
        if path == home {
            break;
        }
        current = path.parent();
    }
    Ok(())
}
fn install(home: &Path, id: u64, data: &Value) -> Result<Value, String> {
    let name = data["slug"]
        .as_str()
        .filter(|s| slug(s))
        .ok_or("插件名称无效")?;
    let ver = data["version"]
        .as_str()
        .filter(|s| version(s))
        .ok_or("插件版本无效")?;
    let encoded = data["archiveBase64"].as_str().ok_or("插件下载结果不完整")?;
    if encoded.len() > 14 * MB {
        return Err("插件下载内容过大".into());
    }
    let bytes = STANDARD.decode(encoded).map_err(|_| "插件下载数据损坏")?;
    if format!("{:x}", Sha256::digest(&bytes)) != data["sha256"].as_str().unwrap_or("") {
        return Err("插件校验失败，请重试；已购买的插件不会重复扣点".into());
    }
    let files = package(&bytes, name, ver)?;
    if data["kind"] == "skill" {
        let destination = home.join("skills").join(format!("{name}-jg-{id}"));
        let prefix = format!("skills/{name}/");
        if !files
            .iter()
            .any(|(path, _)| path == &format!("{prefix}SKILL.md"))
        {
            return Err("技能包缺少 SKILL.md".into());
        }
        let mut writes = Vec::new();
        for (path, bytes) in files {
            if let Some(relative) = path.strip_prefix(&prefix) {
                writes.push((destination.join(relative), bytes));
            } else if path != ".codex-plugin/plugin.json" {
                return Err("技能包包含其他目录，无法安装".into());
            }
        }
        let metadata =
            json!({"id":id,"slug":name,"version":ver,"sha256":data["sha256"],"kind":"skill"});
        writes.push((
            root(home, id).join("installed.json"),
            serde_json::to_vec(&metadata).unwrap(),
        ));
        for (path, _) in &writes {
            regular_ancestors(home, path)?;
        }
        super::provider_config::commit(writes)?;
        return Ok(metadata);
    }
    let market = format!("jg-community-{id}");
    let plugin_id = format!("{name}@{market}");
    let source = root(home, id);
    let cache = home
        .join("plugins/cache")
        .join(&market)
        .join(name)
        .join(ver);
    let mut doc = config(home)?;
    for section in ["plugins", "marketplaces"] {
        if let Some(item) = doc.get_mut(section) {
            *item = toml_edit::Item::Table(
                item.clone()
                    .into_table()
                    .map_err(|_| "Codex 插件配置不是有效配置表")?,
            );
        }
    }
    if let Some(table) = doc.get_mut("plugins").and_then(|v| v.as_table_like_mut()) {
        table.remove(&plugin_id);
    }
    if let Some(table) = doc
        .get_mut("marketplaces")
        .and_then(|v| v.as_table_like_mut())
    {
        table.remove(&market);
    }
    let mut writes = Vec::new();
    for (path, bytes) in files {
        writes.push((source.join("plugins").join(name).join(&path), bytes.clone()));
        writes.push((cache.join(path), bytes));
    }
    writes.push((source.join(".agents/plugins/marketplace.json"),serde_json::to_vec(&json!({"name":market,"interface":{"displayName":"用户分享"},"plugins":[{"name":name,"source":{"source":"local","path":format!("./plugins/{name}")},"policy":{"installation":"AVAILABLE","authentication":"ON_INSTALL"},"category":"Productivity"}]})).unwrap()));
    let metadata = json!({"id":id,"slug":name,"version":ver,"sha256":data["sha256"]});
    writes.push((
        source.join("installed.json"),
        serde_json::to_vec(&metadata).unwrap(),
    ));
    doc["plugins"][&plugin_id]["enabled"] = value(true);
    doc["marketplaces"][&market]["source_type"] = value("local");
    doc["marketplaces"][&market]["source"] = value(source.to_string_lossy().as_ref());
    writes.push((home.join("config.toml"), doc.to_string().into_bytes()));
    for (path, _) in &writes {
        regular_ancestors(home, path)?;
    }
    super::provider_config::commit(writes)?;
    Ok(metadata)
}
fn installed(home: &Path) -> Result<Value, String> {
    let doc = config(home)?;
    let mut items = Vec::new();
    if let Ok(entries) = fs::read_dir(home.join(".usage-assistant/community")) {
        for entry in entries.flatten() {
            let Ok(id) = entry.file_name().to_string_lossy().parse::<u64>() else {
                continue;
            };
            let Ok(bytes) = fs::read(entry.path().join("installed.json")) else {
                continue;
            };
            let Ok(metadata) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            let name = metadata["slug"].as_str().unwrap_or("");
            let ver = metadata["version"].as_str().unwrap_or("");
            if !slug(name) || !version(ver) {
                continue;
            }
            if metadata["kind"] == "skill" {
                if home
                    .join("skills")
                    .join(format!("{name}-jg-{id}"))
                    .join("SKILL.md")
                    .is_file()
                {
                    items.push(metadata);
                }
                continue;
            }
            let market = format!("jg-community-{id}");
            let key = format!("{name}@{market}");
            if doc
                .get("plugins")
                .and_then(|p| p.get(&key))
                .and_then(|p| p.get("enabled"))
                .and_then(|v| v.as_bool())
                == Some(true)
                && home
                    .join("plugins/cache")
                    .join(market)
                    .join(name)
                    .join(ver)
                    .join(".codex-plugin/plugin.json")
                    .is_file()
            {
                items.push(metadata);
            }
        }
    }
    Ok(json!(items))
}
fn uninstall(home: &Path, id: u64) -> Result<Value, String> {
    let source = root(home, id);
    regular_ancestors(home, &source)?;
    if let Ok(bytes) = fs::read(source.join("installed.json")) {
        if let Ok(metadata) = serde_json::from_slice::<Value>(&bytes) {
            if metadata["kind"] == "skill" {
                let name = metadata["slug"]
                    .as_str()
                    .filter(|s| slug(s))
                    .ok_or("技能记录损坏")?;
                let destination = home.join("skills").join(format!("{name}-jg-{id}"));
                regular_ancestors(home, &destination)?;
                if destination.exists() {
                    fs::remove_dir_all(destination)
                        .map_err(|_| "技能正在使用，请关闭 Codex 后重试")?;
                }
                fs::remove_dir_all(source).map_err(|_| "移除技能记录失败，请重试")?;
                return Ok(json!({"id":id}));
            }
        }
    }
    let market = format!("jg-community-{id}");
    let cache = home.join("plugins/cache").join(&market);
    for path in [&source, &cache] {
        regular_ancestors(home, path)?;
    }
    let mut doc = config(home)?;
    if let Some(plugins) = doc.get_mut("plugins").and_then(|i| i.as_table_like_mut()) {
        let keys: Vec<String> = plugins
            .iter()
            .filter(|(k, _)| k.ends_with(&format!("@{market}")))
            .map(|(k, _)| k.to_owned())
            .collect();
        for key in keys {
            plugins.remove(&key);
        }
    }
    if let Some(markets) = doc
        .get_mut("marketplaces")
        .and_then(|i| i.as_table_like_mut())
    {
        markets.remove(&market);
    }
    super::provider_config::commit(vec![(
        home.join("config.toml"),
        doc.to_string().into_bytes(),
    )])?;
    for path in [cache, source] {
        if path.exists() {
            fs::remove_dir_all(path)
                .map_err(|_| "插件已停用，部分文件正在使用，请关闭 Codex 后重试卸载")?;
        }
    }
    Ok(json!({"id":id}))
}

#[tauri::command]
pub async fn plugin_market(
    action: String,
    payload: Value,
    api_key: Option<String>,
) -> Result<Value, String> {
    let home = super::codex_home_dir()?;
    if action == "installed" {
        return tauri::async_runtime::spawn_blocking(move || {
            let _lock = super::PROVIDER_CONFIG_LOCK.lock().map_err(|_| "配置忙碌")?;
            installed(&home)
        })
        .await
        .map_err(|_| "读取插件失败")?;
    }
    let _operation = OPERATION.lock().await;
    if action == "uninstall" {
        let number = id(&payload)?;
        return tauri::async_runtime::spawn_blocking(move || {
            let _lock = super::PROVIDER_CONFIG_LOCK.lock().map_err(|_| "配置忙碌")?;
            uninstall(&home, number)
        })
        .await
        .map_err(|_| "卸载插件失败")?;
    }
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|_| error())?;
    let private = matches!(
        action.as_str(),
        "mine" | "upload" | "install" | "end" | "edit"
    );
    let key = api_key.unwrap_or_default();
    if private && (key.len() < 8 || key.len() > 120 || key.contains(['\r', '\n'])) {
        return Err("请先登录有效的坚果账户".into());
    }
    let request = match action.as_str() {
        "list" => client.get(API).query(&[
            ("kind", payload["kind"].as_str().unwrap_or("plugin")),
            ("q", payload["q"].as_str().unwrap_or("")),
            ("sort", payload["sort"].as_str().unwrap_or("downloads")),
            (
                "page",
                &payload["page"].as_u64().unwrap_or(0).min(10000).to_string(),
            ),
        ]),
        "detail" => client.get(format!("{API}/{}", id(&payload)?)),
        "mine" => client
            .post(format!("{API}/mine"))
            .json(&json!({"kami":key,"kind":payload["kind"].as_str().unwrap_or("plugin")})),
        "end" => client
            .post(format!("{API}/{}/end", id(&payload)?))
            .json(&json!({"kami":key})),
        "edit" => client
            .post(format!("{API}/{}/edit", id(&payload)?))
            .json(&json!({"kami":key})),
        "upload" => {
            let mut body = payload.clone();
            let map = body.as_object_mut().ok_or("投稿内容无效")?;
            map.insert("kami".into(), json!(key));
            if serde_json::to_vec(&body).map_err(|_| "投稿内容无效")?.len() > 20 * MB {
                return Err("投稿文件总大小过大".into());
            }
            client.post(format!("{API}/upload")).json(&body)
        }
        "install" => {
            let home = home.clone();
            tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
                #[cfg(target_os = "windows")]
                let ready = super::detect_codex_windows().installed;
                #[cfg(not(target_os = "windows"))]
                let ready = super::detect_codex_macos().installed;
                if !ready {
                    return Err("请先安装 Codex，再安装插件".into());
                }
                let _lock = super::PROVIDER_CONFIG_LOCK.lock().map_err(|_| "配置忙碌")?;
                config(&home)?;
                let probe = home.join(".usage-assistant/community/.write-check");
                regular_ancestors(&home, &probe)?;
                fs::create_dir_all(probe.parent().unwrap()).map_err(|_| "插件安装位置不可写")?;
                fs::write(&probe, b"check").map_err(|_| "插件安装位置不可写")?;
                let _ = fs::remove_file(probe);
                Ok(())
            })
            .await
            .map_err(|_| "安装预检查失败")??;
            client
                .post(format!("{API}/{}/download", id(&payload)?))
                .json(&json!({"kami":key,"expectedPrice":payload["expectedPrice"]}))
        }
        _ => return Err("不支持的插件市场操作".into()),
    };
    let response = request
        .header("X-Client", "codex-assistant")
        .send()
        .await
        .map_err(|_| error())?;
    let status = response.status();
    if status.as_u16() == 404 {
        return Err("插件市场服务尚未开放，请稍后再试；内置插件和技能仍可使用".into());
    }
    if status.as_u16() == 429 {
        return Err("操作较频繁，请稍等一分钟后重试".into());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| error())?;
        if bytes.len() + chunk.len() > 22 * MB {
            return Err("插件市场响应过大".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let result: Value =
        serde_json::from_slice(&bytes).map_err(|_| "插件市场返回异常，请稍后重试")?;
    if !status.is_success() || result["ok"] != true {
        return Err(result["message"]
            .as_str()
            .or(result["error"].as_str())
            .unwrap_or("插件操作失败，请稍后重试")
            .chars()
            .take(300)
            .collect());
    }
    let data = result["data"].clone();
    if action == "install" {
        let number = id(&payload)?;
        return tauri::async_runtime::spawn_blocking(move || {
            let _lock = super::PROVIDER_CONFIG_LOCK.lock().map_err(|_| "配置忙碌")?;
            install(&home, number, &data)
        })
        .await
        .map_err(|_| "安装失败，已购买插件重试不会重复扣点")?;
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unsafe_and_windows_alias_paths() {
        for path in [
            "../x",
            "/x",
            "a/../b",
            "a//b",
            "C:/x",
            "a\\x",
            "a/nul.txt",
            "a/COM1.js",
            "a/x. ",
            "a/x.",
        ] {
            assert!(!safe_path(path), "{path}");
        }
        assert!(safe_path("skills/example/SKILL.md"));
    }
    #[test]
    fn validates_manifest_and_binary_integrity_before_writing() {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(
            ".codex-plugin/plugin.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(br#"{"name":"example","version":"1.0.0"}"#)
            .unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        assert!(package(&bytes, "example", "1.0.0").is_ok());
        assert!(package(&bytes, "different", "1.0.0").is_err());
    }
    #[test]
    fn install_update_remove_preserve_other_plugins_and_reject_bad_checksum() {
        use std::io::Write;
        let home = std::env::temp_dir().join(format!(
            "assistant-community-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join("config.toml"),
            "model = 'keep-model'\n[plugins.\"jianguo-image@jianguo-assistant\"]\nenabled = true\n",
        )
        .unwrap();
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(
            ".codex-plugin/plugin.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(br#"{"name":"example","version":"1.0.0"}"#)
            .unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let mut data = json!({"slug":"example","version":"1.0.0","archiveBase64":STANDARD.encode(&bytes),"sha256":"bad"});
        assert!(install(&home, 42, &data).is_err());
        assert!(!root(&home, 42).exists());
        data["sha256"] = json!(format!("{:x}", Sha256::digest(&bytes)));
        install(&home, 42, &data).unwrap();
        assert_eq!(installed(&home).unwrap().as_array().unwrap().len(), 1);
        install(&home, 42, &data).unwrap();
        assert_eq!(installed(&home).unwrap().as_array().unwrap().len(), 1);
        assert!(
            home.join("plugins/cache/jg-community-42/example/1.0.0/.codex-plugin/plugin.json")
                .is_file()
        );
        uninstall(&home, 42).unwrap();
        assert_eq!(installed(&home).unwrap(), json!([]));
        let text = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(text.contains("keep-model"));
        assert!(text.contains("jianguo-image@jianguo-assistant"));
        assert!(!text.contains("jg-community-42"));
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn shared_skills_install_as_skills_and_remove_without_affecting_plugins() {
        use std::io::Write;
        let home = std::env::temp_dir().join(format!(
            "shared-skill-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("config.toml"), "model = 'keep'\n").unwrap();
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (path, content) in [
            (
                ".codex-plugin/plugin.json",
                r#"{"name":"example","version":"1.0.0","skills":"./skills/"}"#,
            ),
            (
                "skills/example/SKILL.md",
                "---\nname: example\ndescription: Test\n---\nInstructions",
            ),
            ("skills/example/scripts/run.py", "print('test')"),
        ] {
            zip.start_file(path, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        let data = json!({"slug":"example","version":"1.0.0","kind":"skill","archiveBase64":STANDARD.encode(&bytes),"sha256":format!("{:x}",Sha256::digest(&bytes))});
        install(&home, 12, &data).unwrap();
        install(&home, 12, &data).unwrap();
        assert!(home.join("skills/example-jg-12/SKILL.md").is_file());
        assert!(home.join("skills/example-jg-12/scripts/run.py").is_file());
        assert_eq!(installed(&home).unwrap().as_array().unwrap().len(), 1);
        assert_eq!(
            fs::read_to_string(home.join("config.toml")).unwrap(),
            "model = 'keep'\n"
        );
        uninstall(&home, 12).unwrap();
        assert!(!home.join("skills/example-jg-12").exists());
        assert_eq!(installed(&home).unwrap(), json!([]));
        fs::remove_dir_all(home).unwrap();
    }
}
