//! Public curated packages; independent of paid community shares and legacy skill installs.
use super::plugin_market::{config, regular_ancestors};
use futures_util::StreamExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use toml_edit::{Item, value};

const BUNDLED: &str = include_str!("../catalog/public-plugins.json");
const REPOSITORY: &str = "BigPizzaV3/CodexPlusPlusPluginCache";
static OPERATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const MB: usize = 1024 * 1024;
fn component(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 120
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b))
        && s != "."
        && s != ".."
        && !s.ends_with('.')
        && !matches!(
            s.split('.')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str(),
            "con" | "prn" | "aux" | "nul"
        )
        && !(s.len() >= 4
            && ["com", "lpt"].contains(&s[..3.min(s.len())].to_ascii_lowercase().as_str())
            && s.as_bytes()[3].is_ascii_digit())
}
fn hash(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn validate(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > 4 * MB {
        return Err("插件目录过大".into());
    }
    let root: Value = serde_json::from_slice(bytes).map_err(|_| "插件目录格式无效")?;
    if root["schemaVersion"] != 1
        || root["repository"] != REPOSITORY
        || !hash(root["revision"].as_str().unwrap_or(""))
    {
        return Err("插件目录来源或版本无效".into());
    }
    let plugins = root["plugins"]
        .as_array()
        .filter(|p| !p.is_empty() && p.len() <= 10000)
        .ok_or("插件目录为空或格式无效")?;
    let mut ids = HashSet::new();
    for p in plugins {
        let name = p["name"].as_str().unwrap_or("");
        let id = p["id"].as_str().unwrap_or("");
        if !component(name)
            || !component(id)
            || !component(p["version"].as_str().unwrap_or(""))
            || p["path"] != format!("plugins/{name}")
            || !hash(p["tree"].as_str().unwrap_or(""))
            || !ids.insert(id)
            || !p["description"].is_string()
            || !p["displayName"].is_string()
            || !p["requiresAuth"].is_boolean()
            || !p["author"].is_string()
            || !p["license"].is_string()
            || p["bytes"].as_u64().is_none()
            || p["skills"].as_u64().is_none()
            || !p["tags"]
                .as_array()
                .is_some_and(|tags| tags.iter().all(Value::is_string))
        {
            return Err("插件目录含无效条目，请稍后刷新".into());
        }
    }
    Ok(root)
}
fn root(home: &Path) -> PathBuf {
    home.join(".usage-assistant/public-plugins")
}
fn market(id: &str) -> String {
    format!("usage-public-{:x}", Sha256::digest(id.as_bytes()))[..37].to_owned()
}
fn source(home: &Path, id: &str) -> PathBuf {
    root(home).join(market(id))
}
fn market_source(home: &Path, id: &str) -> PathBuf {
    home.join("plugins/cache").join(market(id))
}
fn cache(home: &Path, p: &Value) -> PathBuf {
    home.join("plugins/cache")
        .join(market(p["id"].as_str().unwrap()))
        .join(p["name"].as_str().unwrap())
        .join(p["version"].as_str().unwrap())
}
fn read_catalog(home: &Path) -> (Value, &'static str) {
    let path = root(home).join("index.json");
    if regular_ancestors(home, &path).is_ok() {
        if let Ok(bytes) = fs::read(path) {
            if let Ok(v) = validate(&bytes) {
                return (v, "cache");
            }
        }
    }
    (
        validate(BUNDLED.as_bytes()).expect("validated bundled catalog"),
        "bundled",
    )
}
fn localize(root: &mut Value) {
    let bundled: Value = serde_json::from_str(BUNDLED).unwrap();
    for p in root["plugins"].as_array_mut().unwrap() {
        // Reuse translations only while the source description still matches.
        let known = bundled["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["id"] == p["id"] && b["description"] == p["description"]);
        p["descriptionZh"] = known
            .map(|b| b["descriptionZh"].clone())
            .unwrap_or_else(|| {
                let text = p["description"].as_str().unwrap_or("");
                if text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
                    json!(text)
                } else {
                    json!("该插件的中文介绍正在补充，请展开详情查看原文说明。")
                }
            });
    }
}
async fn fetch(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(12))
        .timeout(Duration::from_secs(120))
        .user_agent("Codex-Usage-Assistant")
        .build()
        .map_err(|_| "无法创建下载连接")?;
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github.raw+json")
        .send()
        .await
        .map_err(|_| "无法连接公开插件源，请检查网络后重试")?;
    if !response.status().is_success() {
        return Err(format!(
            "公开插件源返回 HTTP {}，请稍后重试",
            response.status().as_u16()
        ));
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err("下载内容超过大小限制".into());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "下载中断，请重试")?;
        if bytes.len() + chunk.len() > limit {
            return Err("下载内容超过大小限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn status(home: &Path, catalog: &Value) -> Result<Value, String> {
    let doc = config(home)?;
    let mut items = Vec::new();
    for p in catalog["plugins"].as_array().unwrap() {
        let id = p["id"].as_str().unwrap();
        let record = source(home, id).join("installed.json");
        regular_ancestors(home, &record)?;
        let Ok(bytes) = fs::read(record) else {
            continue;
        };
        let Ok(saved) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if saved["id"] != p["id"]
            || saved["name"] != p["name"]
            || !hash(saved["tree"].as_str().unwrap_or(""))
            || !component(saved["version"].as_str().unwrap_or(""))
        {
            continue;
        }
        let destination = cache(home, &saved);
        regular_ancestors(home, &destination)?;
        if !destination.join(".codex-plugin/plugin.json").is_file() {
            continue;
        }
        let key = format!("{}@{}", p["name"].as_str().unwrap(), market(id));
        let enabled = doc
            .get("plugins")
            .and_then(|t| t.get(&key))
            .and_then(|t| t.get("enabled"))
            .and_then(|v| v.as_bool())
            == Some(true);
        items.push(
            json!({"id":id,"version":saved["version"],"tree":saved["tree"],"enabled":enabled}),
        );
    }
    Ok(json!(items))
}
fn ensure_tables(doc: &mut toml_edit::DocumentMut) -> Result<(), String> {
    for section in ["plugins", "marketplaces"] {
        if let Some(item) = doc.get_mut(section) {
            *item = Item::Table(
                item.clone()
                    .into_table()
                    .map_err(|_| "Codex 插件配置不是有效配置表")?,
            );
        }
    }
    Ok(())
}
fn install(home: &Path, p: &Value, bytes: Vec<u8>) -> Result<Value, String> {
    let id = p["id"].as_str().ok_or("插件编号无效")?;
    let name = p["name"].as_str().ok_or("插件名称无效")?;
    let destination = cache(home, p);
    let source = source(home, id);
    let market_source = market_source(home, id);
    for path in [&destination, &source, &home.join("config.toml")] {
        regular_ancestors(home, path)?;
    }
    let original_config = fs::read(home.join("config.toml")).ok();
    let mut doc = config(home)?;
    ensure_tables(&mut doc)?;
    let market = market(id);
    let key = format!("{name}@{market}");
    if let Some(existing) = doc
        .get("marketplaces")
        .and_then(|t| t.get(&market))
        .and_then(|t| t.get("source"))
    {
        if existing.as_str() != Some(market_source.to_string_lossy().as_ref()) {
            return Err("同名插件市场已指向其他目录，未覆盖配置".into());
        }
    }
    // Validate the version in a fresh staging directory before replacing any existing cache.
    let mut random = [0; 8];
    getrandom::fill(&mut random).map_err(|_| "无法创建插件暂存目录")?;
    let staging = source.join(format!(".download-{:x}", u64::from_ne_bytes(random)));
    regular_ancestors(home, &staging)?;
    fs::create_dir_all(&source).map_err(|_| "无法创建插件暂存目录")?;
    fs::create_dir(&staging).map_err(|_| "无法创建插件暂存目录")?;
    let validated = (|| {
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(&bytes)).map_err(|_| "插件压缩包格式无效")?;
        super::skill_archive::extract_market_plugin(
            &mut archive,
            p["path"].as_str().unwrap(),
            name,
            &staging,
        )
        .map_err(|e| e.replace("技能", "插件"))?;
        let manifest: Value = serde_json::from_slice(
            &fs::read(staging.join(".codex-plugin/plugin.json")).map_err(|_| "无法读取插件清单")?,
        )
        .map_err(|_| "插件清单损坏")?;
        if manifest
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("1.0.0")
            != p["version"].as_str().unwrap()
        {
            return Err("插件版本与目录不一致，未启用插件".to_owned());
        }
        Ok(())
    })();
    if let Err(e) = validated {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    // Keep the previous cache until configuration and receipts have committed.
    let backup = staging.with_extension("previous");
    fs::create_dir_all(destination.parent().ok_or("插件缓存路径无效")?)
        .map_err(|_| "无法创建插件缓存目录")?;
    let existed = destination.exists();
    if existed {
        fs::rename(&destination, &backup).map_err(|_| "插件文件正在使用，请关闭 Codex 后重试")?;
    }
    if fs::rename(&staging, &destination).is_err() {
        if existed {
            let _ = fs::rename(&backup, &destination);
        }
        let _ = fs::remove_dir_all(&staging);
        return Err("无法保存插件缓存，原文件已保留".into());
    }
    doc["plugins"][&key]["enabled"] = value(true);
    doc["marketplaces"][&market]["source_type"] = value("local");
    doc["marketplaces"][&market]["source"] = value(market_source.to_string_lossy().as_ref());
    let relative = format!("./{name}/{}", p["version"].as_str().unwrap());
    let market_manifest = json!({"name":market,"interface":{"displayName":format!("插件市场 · {}", p["displayName"].as_str().unwrap())},"plugins":[{"name":name,"source":{"source":"local","path":relative},"policy":{"installation":"AVAILABLE","authentication":"ON_USE"},"category":"Productivity"}]});
    let writes = vec![
        (
            market_source.join(".agents/plugins/marketplace.json"),
            serde_json::to_vec(&market_manifest).unwrap(),
        ),
        (
            source.join("installed.json"),
            serde_json::to_vec(p).unwrap(),
        ),
        (home.join("config.toml"), doc.to_string().into_bytes()),
    ];
    for (path, _) in &writes {
        regular_ancestors(home, path)?;
    }
    let committed = if fs::read(home.join("config.toml")).ok() != original_config {
        Err("Codex 配置已被其他程序修改，请重试；未覆盖新配置".to_owned())
    } else {
        super::provider_config::commit(writes)
    };
    if let Err(e) = committed {
        let _ = fs::remove_dir_all(&destination);
        if existed {
            fs::rename(&backup, &destination)
                .map_err(|_| format!("{e}；原插件保留在 {}", backup.display()))?;
        }
        return Err(e);
    }
    if existed {
        let _ = fs::remove_dir_all(&backup);
    }
    // Codex discovers cached versions independently: archive old versions outside its cache.
    for entry in fs::read_dir(destination.parent().unwrap()).map_err(|_| "无法读取旧版插件缓存")?
    {
        let entry = entry.map_err(|_| "无法读取旧版插件缓存")?;
        let old = entry.path();
        if old == destination {
            continue;
        }
        regular_ancestors(home, &old)?;
        let archive = source.join(format!(
            "previous-{:x}-{}",
            u64::from_ne_bytes(random),
            entry.file_name().to_string_lossy()
        ));
        regular_ancestors(home, &archive)?;
        fs::rename(old, archive).map_err(|_| "新版已写入，旧缓存正在使用，请关闭 Codex 后重试")?;
    }
    Ok(json!({"id":id,"version":p["version"],"tree":p["tree"],"enabled":true}))
}
fn uninstall(home: &Path, id: &str) -> Result<(), String> {
    let market = market(id);
    let source = source(home, id);
    let cache = home.join("plugins/cache").join(&market);
    for path in [&source, &cache, &home.join("config.toml")] {
        regular_ancestors(home, path)?;
    }
    let mut doc = config(home)?;
    ensure_tables(&mut doc)?;
    if let Some(table) = doc.get_mut("plugins").and_then(|t| t.as_table_like_mut()) {
        let keys: Vec<_> = table
            .iter()
            .filter(|(k, _)| k.ends_with(&format!("@{market}")))
            .map(|(k, _)| k.to_owned())
            .collect();
        for key in keys {
            table.remove(&key);
        }
    }
    if let Some(table) = doc
        .get_mut("marketplaces")
        .and_then(|t| t.as_table_like_mut())
    {
        if let Some(existing) = table.get(&market).and_then(|t| t.get("source")) {
            if existing.as_str() != Some(cache.to_string_lossy().as_ref()) {
                return Err("市场来源已改变，未卸载其他来源的插件".into());
            }
        }
        table.remove(&market);
    }
    super::provider_config::commit(vec![(
        home.join("config.toml"),
        doc.to_string().into_bytes(),
    )])?;
    for path in [cache, source] {
        if path.exists() {
            fs::remove_dir_all(path)
                .map_err(|_| "插件已停用，文件正在使用，请关闭 Codex 后重试卸载")?;
        }
    }
    Ok(())
}
#[tauri::command]
pub async fn public_plugin_market(action: String, payload: Value) -> Result<Value, String> {
    let home = super::codex_home_dir()?;
    let _operation = OPERATION.lock().await;
    let (mut catalog, mut origin) = read_catalog(&home);
    let mut warning = String::new();
    if action == "list" {
        if payload["refresh"] == true {
            let mut downloaded = None;
            for url in [
                format!("https://raw.githubusercontent.com/{REPOSITORY}/main/plugin-index.json"),
                format!(
                    "https://api.github.com/repos/{REPOSITORY}/contents/plugin-index.json?ref=main"
                ),
            ] {
                match fetch(&url, 4 * MB).await.and_then(|bytes| validate(&bytes)) {
                    Ok(mut v) => {
                        localize(&mut v);
                        downloaded = Some(v);
                        break;
                    }
                    Err(e) => warning = e,
                }
            }
            if let Some(v) = downloaded {
                let path = root(&home).join("index.json");
                regular_ancestors(&home, &path)?;
                super::provider_config::commit(vec![(path, serde_json::to_vec(&v).unwrap())])?;
                catalog = v;
                origin = "online";
                warning.clear();
            } else {
                warning = format!("{warning}。已显示本地目录，可继续浏览。");
            }
        }
        localize(&mut catalog);
        let _lock = super::PROVIDER_CONFIG_LOCK.lock().map_err(|_| "配置忙碌")?;
        return Ok(
            json!({"items":catalog["plugins"],"installed":status(&home,&catalog)?,"origin":origin,"updatedAt":catalog["updatedAt"],"warning":warning}),
        );
    }
    let id = payload["id"]
        .as_str()
        .filter(|s| component(s))
        .ok_or("插件编号无效")?
        .to_owned();
    if action == "uninstall" {
        return tauri::async_runtime::spawn_blocking(move || {
            let _lock = super::PROVIDER_CONFIG_LOCK.lock().map_err(|_| "配置忙碌")?;
            uninstall(&home, &id)?;
            Ok(json!({"id":id}))
        })
        .await
        .map_err(|_| "卸载插件失败")?;
    }
    if action != "install" {
        return Err("未知插件市场操作".into());
    }
    let p = catalog["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .cloned()
        .ok_or("目录中没有此插件，请刷新后重试")?;
    if p["tree"] != payload["tree"] {
        return Err("插件目录已更新，请刷新后重试安装".into());
    }
    let installed = tauri::async_runtime::spawn_blocking(|| {
        #[cfg(target_os = "windows")]
        {
            super::detect_codex_windows().installed
        }
        #[cfg(not(target_os = "windows"))]
        {
            super::detect_codex_macos().installed
        }
    })
    .await
    .map_err(|_| "检测 Codex 失败")?;
    if !installed {
        return Err("请先安装 Codex，再安装插件".into());
    }
    let bytes = fetch(
        &format!(
            "https://codeload.github.com/{REPOSITORY}/zip/{}",
            p["tree"].as_str().unwrap()
        ),
        64 * MB,
    )
    .await?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = super::PROVIDER_CONFIG_LOCK.lock().map_err(|_| "配置忙碌")?;
        install(&home, &p, bytes)
    })
    .await
    .map_err(|_| "安装插件失败")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    fn fixture() -> Value {
        validate(BUNDLED.as_bytes()).unwrap()["plugins"][0].clone()
    }
    fn temp() -> PathBuf {
        let mut bytes = [0; 8];
        getrandom::fill(&mut bytes).unwrap();
        let path = std::env::temp_dir().join(format!(
            "public-plugin-test-{:x}",
            u64::from_ne_bytes(bytes)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn archive(p: &Value, version: Option<&str>) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("repo/.codex-plugin/plugin.json", options)
            .unwrap();
        zip.write_all(serde_json::to_string(&json!({"name":p["name"],"version":version.unwrap_or(p["version"].as_str().unwrap())})).unwrap().as_bytes()).unwrap();
        zip.start_file("repo/skills/demo/SKILL.md", options)
            .unwrap();
        zip.write_all(b"---\nname: demo\ndescription: demo skill\n---\n# Demo")
            .unwrap();
        zip.finish().unwrap().into_inner()
    }
    #[test]
    fn catalog_has_520_specific_chinese_descriptions_and_104_authorized_plugins() {
        let mut root = validate(BUNDLED.as_bytes()).unwrap();
        localize(&mut root);
        let items = root["plugins"].as_array().unwrap();
        assert_eq!(items.len(), 520);
        assert_eq!(
            items.iter().filter(|p| p["requiresAuth"] == true).count(),
            104
        );
        for p in items {
            let zh = p["descriptionZh"].as_str().unwrap();
            assert!(
                zh.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
                "{}",
                p["name"]
            );
            assert!(!zh.contains("正在补充"), "{}", p["name"]);
        }
        assert!(
            items.iter().find(|p| p["name"] == "mt-newswires").unwrap()["descriptionZh"]
                .as_str()
                .unwrap()
                .contains("金融新闻")
        );
        assert!(
            items.iter().find(|p| p["name"] == "remotion").unwrap()["descriptionZh"]
                .as_str()
                .unwrap()
                .contains("React")
        );
        assert!(
            items.last().unwrap()["descriptionZh"]
                .as_str()
                .unwrap()
                .contains("日本房地产")
        );
    }
    #[test]
    fn reject_changed_sources_unsafe_paths_and_duplicate_ids() {
        let original = validate(BUNDLED.as_bytes()).unwrap();
        for (key, value) in [
            ("name", "../outside"),
            ("id", ".."),
            ("path", "plugins/../escape"),
            ("tree", "main"),
            ("version", "C:/escape"),
        ] {
            let mut bad = original.clone();
            bad["plugins"][0][key] = json!(value);
            assert!(validate(&serde_json::to_vec(&bad).unwrap()).is_err());
        }
        let mut bad = original.clone();
        bad["repository"] = json!("attacker/repository");
        assert!(validate(&serde_json::to_vec(&bad).unwrap()).is_err());
        let mut bad = original;
        bad["plugins"][1]["id"] = bad["plugins"][0]["id"].clone();
        assert!(validate(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    #[test]
    fn changed_english_does_not_reuse_stale_chinese() {
        let mut root = validate(BUNDLED.as_bytes()).unwrap();
        root["plugins"][0]["description"] = json!("New unrelated behavior");
        localize(&mut root);
        assert!(
            root["plugins"][0]["descriptionZh"]
                .as_str()
                .unwrap()
                .contains("正在补充")
        );
    }
    #[test]
    fn native_install_update_disable_and_uninstall_preserve_other_sources() {
        let home = temp();
        fs::write(home.join("config.toml"),"model='keep-model'\n[plugins.'other@market']\nenabled=true\n[plugins.'shared@jg-community-9']\nenabled=true\n").unwrap();
        let mut p = fixture();
        install(&home, &p, archive(&p, None)).unwrap();
        assert!(cache(&home, &p).join("skills/demo/SKILL.md").is_file());
        let manifest: Value = serde_json::from_slice(
            &fs::read(
                market_source(&home, p["id"].as_str().unwrap())
                    .join(".agents/plugins/marketplace.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            manifest["plugins"][0]["source"]["path"],
            format!(
                "./{}/{}",
                p["name"].as_str().unwrap(),
                p["version"].as_str().unwrap()
            )
        );
        assert_eq!(
            status(&home, &json!({"plugins":[p.clone()]})).unwrap()[0]["enabled"],
            true
        );
        let old = cache(&home, &p);
        p["tree"] = json!("1111111111111111111111111111111111111111");
        p["version"] = json!("9.0.0");
        install(&home, &p, archive(&p, None)).unwrap();
        assert!(!old.is_dir());
        assert!(
            fs::read_dir(source(&home, p["id"].as_str().unwrap()))
                .unwrap()
                .flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("previous-"))
        );
        let mut doc = config(&home).unwrap();
        doc["plugins"][&format!(
            "{}@{}",
            p["name"].as_str().unwrap(),
            market(p["id"].as_str().unwrap())
        )]["enabled"] = value(false);
        fs::write(home.join("config.toml"), doc.to_string()).unwrap();
        assert_eq!(
            status(&home, &json!({"plugins":[p.clone()]})).unwrap()[0]["enabled"],
            false
        );
        let unrelated = home.join("plugins/cache/jg-community-9/keep.txt");
        fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
        fs::write(&unrelated, "keep").unwrap();
        uninstall(&home, p["id"].as_str().unwrap()).unwrap();
        assert!(unrelated.is_file());
        assert!(!old.exists());
        let doc = config(&home).unwrap();
        assert_eq!(doc["model"].as_str(), Some("keep-model"));
        assert_eq!(
            doc["plugins"]["other@market"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(
            doc["plugins"]["shared@jg-community-9"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(status(&home, &json!({"plugins":[p]})).unwrap(), json!([]));
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn invalid_manifest_and_config_never_enable_plugin() {
        let home = temp();
        let p = fixture();
        assert!(install(&home, &p, archive(&p, Some("999"))).is_err());
        assert!(!home.join("config.toml").exists());
        install(&home, &p, archive(&p, None)).unwrap();
        let before = fs::read(cache(&home, &p).join(".codex-plugin/plugin.json")).unwrap();
        assert!(install(&home, &p, archive(&p, Some("999"))).is_err());
        assert_eq!(
            fs::read(cache(&home, &p).join(".codex-plugin/plugin.json")).unwrap(),
            before
        );
        fs::write(home.join("config.toml"), "bad = [").unwrap();
        assert!(install(&home, &p, archive(&p, None)).is_err());
        assert_eq!(
            fs::read_to_string(home.join("config.toml")).unwrap(),
            "bad = ["
        );
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn falls_back_to_bundled_catalog_if_cache_is_invalid() {
        let home = temp();
        let (c, o) = read_catalog(&home);
        assert_eq!(o, "bundled");
        assert_eq!(c["plugins"].as_array().unwrap().len(), 520);
        fs::create_dir_all(root(&home)).unwrap();
        fs::write(root(&home).join("index.json"), "{}").unwrap();
        assert_eq!(read_catalog(&home).1, "bundled");
        fs::remove_dir_all(home).unwrap();
    }
    #[tokio::test]
    #[ignore = "explicit network smoke test; temporary Codex home only"]
    async fn live_public_package_registers_native_market() {
        let home = temp();
        let catalog = validate(BUNDLED.as_bytes()).unwrap();
        for name in ["agent-parley", "zotero"] {
            let p = catalog["plugins"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == name)
                .unwrap();
            let bytes = fetch(
                &format!(
                    "https://codeload.github.com/{REPOSITORY}/zip/{}",
                    p["tree"].as_str().unwrap()
                ),
                64 * MB,
            )
            .await
            .unwrap();
            install(&home, p, bytes).unwrap();
            assert_eq!(
                status(&home, &json!({"plugins":[p]})).unwrap()[0]["enabled"],
                true
            );
            let cli = std::env::var_os("CODEX_TEST_PLUGIN_CLI")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("USERPROFILE").map(|h| {
                        PathBuf::from(h).join(".codex/plugins/.plugin-appserver/codex.exe")
                    })
                });
            if let Some(cli) = cli.filter(|c| c.is_file()) {
                let mut command = std::process::Command::new(cli);
                command.current_dir(&home).env("CODEX_HOME", &home).args([
                    "plugin",
                    "list",
                    "--marketplace",
                    &market(p["id"].as_str().unwrap()),
                    "--json",
                ]);
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    command.creation_flags(0x08000000);
                }
                let output = command.output().unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let listed: Value = serde_json::from_slice(&output.stdout).unwrap();
                assert!(
                    listed["installed"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|i| i["pluginId"]
                            == format!("{name}@{}", market(p["id"].as_str().unwrap()))
                            && i["enabled"] == true
                            && i["version"] == p["version"]),
                    "{listed}"
                );
                eprintln!("Actual Codex CLI recognized {name}");
            }
            uninstall(&home, p["id"].as_str().unwrap()).unwrap();
        }
        fs::remove_dir_all(home).unwrap();
    }
}
