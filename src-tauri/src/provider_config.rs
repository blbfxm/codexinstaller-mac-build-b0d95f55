use std::{fs, path::{Path, PathBuf}};
use serde_json::Value;
use toml_edit::{DocumentMut, value};

pub(crate) fn gateway_active(settings: &Value) -> bool {
    settings["activeRelayId"].as_str() == Some("codex-plus-gateway")
        || settings["activeRelayId"].as_str().is_some_and(|active_id| {
            settings["relayProfiles"].as_array().is_some_and(|profiles| {
                profiles.iter().any(|profile| {
                    profile["id"].as_str() == Some(active_id)
                        && profile["relayMode"].as_str() == Some("gateway")
                })
            })
        })
}

pub(crate) fn direct_supported(profile: &Value) -> bool {
    matches!(profile["protocol"].as_str(), None | Some("responses"))
        && matches!(profile["relayMode"].as_str(), None | Some("pureApi") | Some("officialMixApi"))
        && profile["id"].as_str() != Some("codex-plus-gateway")
        // These options change transport semantics and must be applied by Codex++.
        && !["modelRoutes", "independentProxy", "requestCompatibility", "webSearchHistoryCompat",
            "localCompactionPromptAtEnd", "autoReviewModel", "stripImageGenerationTool",
            "strictResponsesReasoning", "noAuth"]
            .iter().any(|key| profile.get(*key).is_some_and(|v| {
                !v.is_null() && v != &Value::Bool(false) && v != "" && v != &serde_json::json!([])
            }))
        && !profile["modelMetadata"].as_str().is_some_and(|s| s.contains("codex_plus_reasoning_parameters"))
        && !profile["configContents"].as_str().is_some_and(|s| {
            s.parse::<DocumentMut>().ok().is_some_and(|doc| {
                doc.get("model_providers").and_then(|p| p.get("custom"))
                    .and_then(|p| p.get("base_url")).and_then(|u| u.as_str())
                    .and_then(|u| reqwest::Url::parse(u).ok())
                    .is_some_and(|u| matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")))
            })
        })
}

pub(crate) fn profile_config(name: &str, url: &str, model: &str, key: &str, mixed: bool) -> String {
    let mut doc = DocumentMut::new();
    apply_provider(&mut doc, name, url, model, key, mixed, None);
    doc.to_string()
}

pub(crate) fn gateway_files(
    home: &Path,
    name: &str,
    url: &str,
    model: &str,
    key: &str,
    access: bool,
    chinese: bool,
    model_catalog_url: Option<&str>,
    model_catalog_json: Option<&str>,
) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let path = home.join("config.toml");
    let contents = read_optional(&path)?.unwrap_or_default();
    let contents = std::str::from_utf8(&contents).map_err(|_| "Codex 配置不是 UTF-8")?;
    let mut doc = contents.trim_start_matches('\u{feff}').parse::<DocumentMut>()
        .map_err(|_| "Codex config.toml 格式无效，已停止以免覆盖原配置")?;
    for section in ["model_providers", "desktop", "features"] {
        if let Some(item) = doc.get_mut(section) {
            let table = item.clone().into_table()
                .map_err(|_| format!("Codex 配置中的 {section} 不是配置表"))?;
            *item = toml_edit::Item::Table(table);
        }
    }
    apply_provider(&mut doc, name, url.trim_end_matches('/'), model, key, false, model_catalog_url);
    if let Some(catalog_path) = model_catalog_json {
        doc["model_catalog_json"] = value(catalog_path);
    }
    doc.remove("profile");
    doc.remove("forced_chatgpt_workspace_id");
    doc["forced_login_method"] = value("api");
    doc["cli_auth_credentials_store"] = value("file");
    if access {
        doc["sandbox_mode"] = value("danger-full-access");
        doc["approval_policy"] = value("never");
        doc["default_permissions"] = value(":danger-full-access");
    }
    if chinese {
        if !doc.contains_key("desktop") {
            doc["desktop"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        doc["desktop"]["localeOverride"] = value("zh-CN");
    }
    let auth_path = home.join("auth.json");
    let mut auth: Value = match read_optional(&auth_path)? {
        Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| "Codex auth.json 格式无效")?,
        None => serde_json::json!({}),
    };
    if !auth.is_object() {
        return Err("Codex 认证文件格式无效".into());
    }
    auth["OPENAI_API_KEY"] = Value::String(key.to_owned());
    auth["auth_mode"] = Value::String("apikey".into());
    let mut files = vec![
        (path, doc.to_string().into_bytes()),
        (auth_path, serde_json::to_vec_pretty(&auth).map_err(|error| error.to_string())?),
    ];
    if access {
        files.push(desktop_access_file(home)?);
    }
    Ok(files)
}

fn apply_provider(
    doc: &mut DocumentMut,
    name: &str,
    url: &str,
    model: &str,
    key: &str,
    mixed: bool,
    model_catalog_url: Option<&str>,
) {
    doc["model_provider"] = value("custom");
    doc["model"] = value(model);
    // One-click setup should start chats at high reasoning, not the light default.
    doc["model_reasoning_effort"] = value("high");
    if !doc.contains_key("model_providers") {
        doc["model_providers"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    // Clear stale credentials / env_key only in the provider being replaced.
    doc["model_providers"]["custom"] = toml_edit::Item::Table(toml_edit::Table::new());
    let provider = &mut doc["model_providers"]["custom"];
    provider["name"] = value(codex_provider_display_name(name, url));
    provider["wire_api"] = value("responses");
    provider["requires_openai_auth"] = value(mixed);
    provider["base_url"] = value(url);
    provider["experimental_bearer_token"] = value(key);
    if let Some(catalog_url) = model_catalog_url {
        provider["model_catalog_url"] = value(catalog_url);
        if !doc.contains_key("features") {
            doc["features"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        doc["features"]["api_key_model_discovery"] = value(true);
    }
}

fn codex_provider_display_name(name: &str, url: &str) -> String {
    let is_nut_provider = reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| host == "api.jianguo.ltd")
        || matches!(name.trim(), "坚果" | "坚果中转" | "坚果供应商");

    if is_nut_provider { "设置".into() } else { name.to_owned() }
}

pub(crate) fn live_files(
    home: &Path,
    profile: &Value,
    access: bool,
    chinese: bool,
    model_catalog_url: Option<&str>,
    model_catalog_json: Option<&str>,
)
    -> Result<Vec<(PathBuf, Vec<u8>)>, String>
{
    if !direct_supported(profile) {
        return Err("该供应商需要 Codex++ 的协议转换或专用接入功能，请在 Codex++ 中启用；未修改当前 Codex 配置".into());
    }
    let name = profile["name"].as_str().ok_or("供应商名称缺失")?;
    let url = profile["upstreamBaseUrl"].as_str().ok_or("供应商地址缺失")?;
    let parsed = reqwest::Url::parse(url).map_err(|_| "供应商地址无效")?;
    if parsed.scheme() != "https" || parsed.host_str().is_none()
        || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("直连供应商必须使用不含登录信息的 HTTPS 地址".into());
    }
    let model = profile["testModel"].as_str().filter(|s| !s.trim().is_empty())
        .ok_or("供应商未设置默认模型，请先编辑")?;
    let saved_auth: Value = serde_json::from_str(profile["authContents"].as_str().ok_or("供应商没有 API Key")?)
        .map_err(|_| "供应商认证配置格式无效")?;
    let key = saved_auth["OPENAI_API_KEY"].as_str().filter(|s| !s.trim().is_empty()).ok_or("供应商没有 API Key")?;
    let config_path = home.join("config.toml");
    let contents = read_optional(&config_path)?.unwrap_or_default();
    let contents = std::str::from_utf8(&contents).map_err(|_| "Codex 配置不是 UTF-8")?;
    let mut doc = contents.trim_start_matches('\u{feff}').parse::<DocumentMut>()
        .map_err(|_| "Codex config.toml 格式无效，已停止以免覆盖原配置")?;
    for section in ["model_providers", "desktop", "features"] {
        if let Some(item) = doc.get_mut(section) {
            let table = item.clone().into_table()
                .map_err(|_| format!("Codex 配置中的 {section} 不是配置表，已停止以免覆盖原配置"))?;
            *item = toml_edit::Item::Table(table);
        }
    }
    if doc.get("model_provider").and_then(|v| v.as_str()).is_some_and(|p| p == "codex-plus-gateway")
        || doc.to_string().contains("/gw/codex/") {
        return Err("Codex 当前连接网关，请先在 Codex++ 中断开网关后再切换直连供应商".into());
    }
    let mixed = profile["relayMode"].as_str() == Some("officialMixApi");
    let auth_path = home.join("auth.json");
    let mut auth: Value = match read_optional(&auth_path)? {
        Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| "Codex auth.json 格式无效，未覆盖认证文件")?,
        None => serde_json::json!({}),
    };
    if !auth.is_object() { return Err("Codex 认证文件格式无效".into()); }
    if mixed && !auth.get("tokens").and_then(|v| v.get("access_token")).and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty()) {
        return Err("官方登录 + API 模式需要已有的官方登录，请先在 Codex++ 中完成登录，或选择纯 API".into());
    }
    apply_provider(&mut doc, name, url.trim_end_matches('/'), model, key, mixed, model_catalog_url);
    if let Some(catalog_path) = model_catalog_json {
        doc["model_catalog_json"] = value(catalog_path);
    }
    // A selected CLI profile or forced ChatGPT login can override the new root provider.
    doc.remove("profile");
    if !mixed { doc.remove("forced_chatgpt_workspace_id"); }
    doc["forced_login_method"] = value(if mixed { "chatgpt" } else { "api" });
    doc["cli_auth_credentials_store"] = value("file");
    if access {
        doc["sandbox_mode"] = value("danger-full-access");
        doc["approval_policy"] = value("never");
        doc["default_permissions"] = value(":danger-full-access");
    }
    if chinese {
        if !doc.contains_key("desktop") {
            doc["desktop"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        doc["desktop"]["localeOverride"] = value("zh-CN");
    }
    // Preserve OAuth tokens for switching back, but select API authentication explicitly.
    auth["OPENAI_API_KEY"] = serde_json::json!(key);
    auth["auth_mode"] = serde_json::json!(if mixed { "chatgpt" } else { "apikey" });
    let mut files = vec![(config_path, doc.to_string().into_bytes()),
        (auth_path, serde_json::to_vec_pretty(&auth).map_err(|e| e.to_string())?)];
    if access { files.push(desktop_access_file(home)?); }
    Ok(files)
}

fn desktop_access_file(home: &Path) -> Result<(PathBuf, Vec<u8>), String> {
    let path = home.join(".codex-global-state.json");
    let mut root: Value = match read_optional(&path)? {
        Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| "Codex 桌面设置格式无效，未覆盖原文件")?,
        None => serde_json::json!({}),
    };
    if !root.is_object() { return Err("Codex 桌面设置格式无效".into()); }
    let atoms = root.as_object_mut().unwrap().entry("electron-persisted-atom-state")
        .or_insert_with(|| serde_json::json!({}));
    if !atoms.is_object() { return Err("Codex 桌面权限设置格式无效".into()); }
    let visibility = atoms.as_object_mut().unwrap().entry("composer-permission-mode-visibility")
        .or_insert(Value::Bool(true));
    match visibility {
        Value::Bool(enabled) => *enabled = true,
        Value::Object(modes) => { modes.insert("full-access".into(), Value::Bool(true)); }
        _ => return Err("Codex 权限菜单设置格式无效，未覆盖原文件".into()),
    }
    let modes = atoms.as_object_mut().unwrap().entry("agent-mode-by-host-id")
        .or_insert_with(|| serde_json::json!({}));
    if !modes.is_object() { return Err("Codex 主机权限设置格式无效".into()); }
    modes["local"] = serde_json::json!("full-access");
    // The skipped non-coding onboarding switches the app into the ChatGPT Work
    // composer, whose permissions come from Work account settings and default
    // to "request approval". A working local profile has no non_coding role
    // state; keep the user's other onboarding fields but select coding.
    if let Some(onboarding) = atoms
        .get_mut("electron:onboarding-welcome-v2-role-state")
        .and_then(Value::as_object_mut)
    {
        onboarding.insert("roles".into(), serde_json::json!(["engineering"]));
        onboarding.insert("workMode".into(), serde_json::json!("coding"));
    }
    atoms["skip-full-access-confirm"] = Value::Bool(true);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64).unwrap_or(0);
    atoms["full-access-warning-dismissed-at-v2"] = serde_json::json!(now_ms);
    // Codex ignores the saved choice when it equals the cached config-derived
    // mode. Its own setAgentModePreference clears this cache on selection.
    let derived = atoms.as_object_mut().unwrap().entry("config-derived-agent-mode-by-host-id")
        .or_insert_with(|| serde_json::json!({}));
    if !derived.is_object() { return Err("Codex 默认权限缓存格式无效".into()); }
    derived["local"] = Value::Null;
    atoms["permission-selection-by-host-id:local"] =
        serde_json::json!({"kind":"agent-mode", "agentMode":"full-access"});
    if let Some(thread_permissions) = atoms
        .get_mut("heartbeat-thread-permissions-by-id")
        .and_then(Value::as_object_mut)
    {
        for permission in thread_permissions.values_mut() {
            if let Some(permission) = permission.as_object_mut() {
                permission.insert("activePermissionProfile".into(), serde_json::json!({
                    "id": ":danger-full-access",
                    "extends": null
                }));
                permission.insert("approvalPolicy".into(), Value::String("never".into()));
                permission.insert("approvalsReviewer".into(), Value::String("user".into()));
                permission.insert("sandboxPolicy".into(), serde_json::json!({
                    "type": "dangerFullAccess"
                }));
            }
        }
    }
    Ok((path, serde_json::to_vec_pretty(&root).map_err(|e| e.to_string())?))
}

pub(crate) fn option_files(home: &Path, access: bool, chinese: bool) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    if !access && !chinese { return Ok(Vec::new()); }
    let path = home.join("config.toml");
    let bytes = read_optional(&path)?.unwrap_or_default();
    let mut doc = std::str::from_utf8(&bytes).map_err(|_| "Codex 配置不是 UTF-8")?
        .parse::<DocumentMut>().map_err(|_| "Codex 配置格式无效")?;
    if access {
        doc["sandbox_mode"] = value("danger-full-access");
        doc["approval_policy"] = value("never");
        doc["default_permissions"] = value(":danger-full-access");
    }
    if chinese {
        let desktop = match doc.remove("desktop") {
            Some(item) => item.into_table().map_err(|_| "Codex desktop 配置格式无效")?,
            None => toml_edit::Table::new(),
        };
        doc["desktop"] = toml_edit::Item::Table(desktop);
        doc["desktop"]["localeOverride"] = value("zh-CN");
    }
    let mut files = vec![(path, doc.to_string().into_bytes())];
    if access { files.push(desktop_access_file(home)?); }
    Ok(files)
}

pub(crate) fn onboarding_complete(home: &Path) -> Result<bool, String> {
    let Some(bytes) = read_optional(&home.join(".codex-global-state.json"))? else {
        return Ok(false);
    };
    let state: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Codex 桌面设置格式无效".to_string())?;
    Ok(state["electron-persisted-atom-state"]["last_completed_onboarding"]
        .as_u64().is_some_and(|timestamp| timestamp > 0))
}

pub(crate) fn verify_full_access(home: &Path) -> Result<(), String> {
    let config = std::fs::read_to_string(home.join("config.toml"))
        .map_err(|error| format!("读取 Codex 权限配置失败：{error}"))?;
    let document = config.parse::<DocumentMut>()
        .map_err(|_| "Codex 权限配置格式无效".to_string())?;
    if document["sandbox_mode"].as_str() != Some("danger-full-access")
        || document["approval_policy"].as_str() != Some("never")
        || document.get("default_permissions").and_then(|v| v.as_str()) != Some(":danger-full-access")
    {
        return Err("Codex 完整访问权限未写入 config.toml".into());
    }
    let state: Value = serde_json::from_slice(&std::fs::read(home.join(".codex-global-state.json"))
        .map_err(|error| format!("读取 Codex 桌面权限状态失败：{error}"))?)
        .map_err(|_| "Codex 桌面权限状态格式无效".to_string())?;
    let atoms = &state["electron-persisted-atom-state"];
    if atoms["agent-mode-by-host-id"]["local"].as_str() != Some("full-access")
        || !atoms["config-derived-agent-mode-by-host-id"]["local"].is_null()
        || atoms["permission-selection-by-host-id:local"]["agentMode"].as_str() != Some("full-access")
        || !(atoms["composer-permission-mode-visibility"].as_bool() == Some(true)
            || atoms["composer-permission-mode-visibility"]["full-access"].as_bool() == Some(true))
    {
        return Err("Codex“在编写器中显示完整访问权限”开关未成功启用".into());
    }
    Ok(())
}

pub(crate) fn deletion_files(home: &Path, profile: &Value) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let path = home.join("config.toml");
    let Some(bytes) = read_optional(&path)? else { return Ok(Vec::new()); };
    let mut doc = std::str::from_utf8(&bytes).map_err(|_| "Codex 配置不是 UTF-8")?
        .trim_start_matches('\u{feff}').parse::<DocumentMut>()
        .map_err(|_| "Codex 配置格式无效，未删除供应商或覆盖配置")?;
    if doc.get("model_provider").and_then(|v| v.as_str()) != Some("custom") {
        return Ok(Vec::new());
    }
    let Some(provider) = doc.get("model_providers").and_then(|p| p.get("custom")) else {
        return Ok(Vec::new());
    };
    let matches = provider.get("base_url").and_then(|v| v.as_str()).zip(profile["upstreamBaseUrl"].as_str())
        .is_some_and(|(live, saved)| live.trim_end_matches('/') == saved.trim_end_matches('/'))
        && profile["name"].as_str().zip(profile["upstreamBaseUrl"].as_str())
            .is_some_and(|(name, url)| provider.get("name").and_then(|v| v.as_str())
                == Some(codex_provider_display_name(name, url).as_str()));
    if !matches { return Ok(Vec::new()); }
    let saved_auth = profile["authContents"].as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok());
    let key = saved_auth.as_ref().and_then(|auth| auth["OPENAI_API_KEY"].as_str())
        .filter(|key| !key.is_empty());
    // Match the saved secret too: another profile may share the same name and URL.
    if provider.get("experimental_bearer_token").and_then(|v| v.as_str()) != key || key.is_none() {
        return Ok(Vec::new());
    }
    doc.remove("model_provider");
    if doc.get("model").and_then(|v| v.as_str()) == profile["testModel"].as_str() {
        doc.remove("model");
    }
    if let Some(providers) = doc.get_mut("model_providers").and_then(|v| v.as_table_like_mut()) {
        providers.remove("custom");
    }
    if doc.get("forced_login_method").and_then(|v| v.as_str()) == Some("api") {
        doc.remove("forced_login_method");
    }
    let mut files = vec![(path, doc.to_string().into_bytes())];
    let auth_path = home.join("auth.json");
    if let Some(bytes) = read_optional(&auth_path)? {
        let mut auth: Value = serde_json::from_slice(&bytes).map_err(|_| "Codex 认证配置格式无效，未删除供应商")?;
        if auth["OPENAI_API_KEY"].as_str() == key {
            let object = auth.as_object_mut().ok_or("Codex 认证配置格式无效")?;
            object.remove("OPENAI_API_KEY");
            if object.get("auth_mode").and_then(Value::as_str) == Some("apikey") {
                object.remove("auth_mode");
            }
            files.push((auth_path, serde_json::to_vec_pretty(&auth).map_err(|error| error.to_string())?));
        }
    }
    Ok(files)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("无法读取 {}：{e}", path.display())),
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    file.write_all(bytes).and_then(|_| file.sync_all()).map_err(|e| e.to_string())
}

/// Back up all files before replacing any; roll back earlier replacements on failure.
pub(crate) fn commit(files: Vec<(PathBuf, Vec<u8>)>) -> Result<(), String> {
    commit_with_backups(files).map(|_| ())
}

pub(crate) fn commit_with_backups(files: Vec<(PathBuf, Vec<u8>)>) -> Result<Vec<PathBuf>, String> {
    commit_changes(files.into_iter().map(|(path, bytes)| (path, Some(bytes))).collect())
}

/// None removes only the named file after backup. Use the same rollback as replacements.
pub(crate) fn commit_changes(files: Vec<(PathBuf, Option<Vec<u8>>)>) -> Result<Vec<PathBuf>, String> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_nanos();
    let mut snapshots = Vec::new();
    let mut backups = Vec::new();
    for (path, _) in &files {
        let parent = path.parent().ok_or("配置目录不可用")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let original = read_optional(path)?;
        if let Some(bytes) = &original {
            let backup = parent.join("backup").join(format!("provider-{stamp}"));
            fs::create_dir_all(&backup).map_err(|e| e.to_string())?;
            write_private(&backup.join(path.file_name().ok_or("配置文件名无效")?), bytes)?;
            if !backups.contains(&backup) { backups.push(backup); }
        }
        snapshots.push(original);
    }
    for (index, (path, bytes)) in files.iter().enumerate() {
        let temporary = path.with_extension(format!("provider-{stamp}.tmp"));
        let result = match bytes {
            Some(bytes) => write_private(&temporary, bytes)
                .and_then(|_| fs::rename(&temporary, path).map_err(|e| e.to_string())),
            None => remove_optional(path),
        };
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            let mut rollback_errors = Vec::new();
            for rollback in (0..index).rev() {
                let target = &files[rollback].0;
                if !matches!(read_optional(target), Ok(current) if current == files[rollback].1) {
                    rollback_errors.push(format!("{} 已被其他程序修改，保留现场和备份", target.display()));
                    continue;
                }
                let restored = match &snapshots[rollback] {
                    Some(original) => fs::write(target, original),
                    None => match fs::remove_file(target) {
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        result => result,
                    },
                };
                if let Err(e) = restored { rollback_errors.push(format!("{}: {e}", target.display())); }
            }
            return Err(format!("保存供应商配置失败：{error}。回滚：{}",
                if rollback_errors.is_empty() { "完成".to_owned() } else { rollback_errors.join("; ") }));
        }
    }
    Ok(backups)
}

fn remove_optional(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("移除登录文件失败 {}：{e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_uses_settings_label_for_nut_provider_only() {
        let nut = profile_config(
            "坚果中转",
            "https://api.jianguo.ltd/v1",
            "model",
            "key",
            false,
        );
        let nut_doc: DocumentMut = nut.parse().unwrap();
        assert_eq!(nut_doc["model_providers"]["custom"]["name"].as_str(), Some("设置"));

        let custom = profile_config("My Provider", "https://example.test/v1", "model", "key", false);
        let custom_doc: DocumentMut = custom.parse().unwrap();
        assert_eq!(custom_doc["model_providers"]["custom"]["name"].as_str(), Some("My Provider"));
    }

    fn fixture() -> Value {
        serde_json::json!({"id":"test","name":"Test", "protocol":"responses","relayMode":"pureApi",
            "upstreamBaseUrl":"https://example.test/v1","testModel":"test-model",
            "authContents":"{\"OPENAI_API_KEY\":\"test-secret\"}"})
    }
    #[test]
    fn deletion_preserves_same_endpoint_with_other_secret_and_rejects_invalid_auth() {
        let home = std::env::temp_dir().join(format!("provider-delete-safety-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        let profile = fixture();
        let other = profile_config("Test", "https://example.test/v1", "test-model", "another-secret", false);
        fs::write(home.join("config.toml"), &other).unwrap();
        assert!(deletion_files(&home, &profile).unwrap().is_empty());
        assert_eq!(fs::read_to_string(home.join("config.toml")).unwrap(), other);
        fs::write(home.join("config.toml"), profile_config("Test", "https://example.test/v1", "test-model", "test-secret", false)).unwrap();
        fs::write(home.join("auth.json"), b"invalid json").unwrap();
        assert!(deletion_files(&home, &profile).is_err());
        assert_eq!(fs::read(home.join("auth.json")).unwrap(), b"invalid json");
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn deleting_nut_provider_matches_the_codex_display_name() {
        let home = std::env::temp_dir().join(format!("provider-delete-nut-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        let mut profile = fixture();
        profile["name"] = serde_json::json!("坚果中转");
        profile["upstreamBaseUrl"] = serde_json::json!("https://api.jianguo.ltd/v1");
        commit(live_files(&home, &profile, false, false, None, None).unwrap()).unwrap();
        commit(deletion_files(&home, &profile).unwrap()).unwrap();
        let doc = fs::read_to_string(home.join("config.toml")).unwrap().parse::<DocumentMut>().unwrap();
        assert!(doc.get("model_provider").is_none());
        assert!(doc["model_providers"].get("custom").is_none());
        let auth: Value = serde_json::from_slice(&fs::read(home.join("auth.json")).unwrap()).unwrap();
        assert!(auth.get("OPENAI_API_KEY").is_none());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn desktop_full_access_updates_only_local_host_and_preserves_gateway() {
        let home = std::env::temp_dir().join(format!("desktop-access-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("config.toml"), "model_provider = 'codex-plus-gateway'\n").unwrap();
        fs::write(home.join(".codex-global-state.json"), serde_json::to_vec(&serde_json::json!({
            "other": {"keep":true},
            "electron-persisted-atom-state": {
                "theme":"dark",
                "composer-permission-mode-visibility":{"full-access":false,"auto":true},
                "agent-mode-by-host-id":{"local":"auto", "remote-test":"read-only"},
                "electron:onboarding-welcome-v2-role-state":{"roles":["something_else"],"personalizedSuggestionsEnabled":true,"workMode":"non_coding"},
                "config-derived-agent-mode-by-host-id":{"local":"full-access", "remote-test":"read-only"},
                "permission-selection-by-host-id:remote-test":{"kind":"agent-mode","agentMode":"read-only"},
                "heartbeat-thread-permissions-by-id":{"thread-1":{"approvalPolicy":"on-request","sandboxPolicy":{"type":"workspaceWrite"}}}
            }
        })).unwrap()).unwrap();
        commit(option_files(&home, true, false).unwrap()).unwrap();
        verify_full_access(&home).unwrap();
        let doc: DocumentMut = fs::read_to_string(home.join("config.toml")).unwrap().parse().unwrap();
        assert_eq!(doc["sandbox_mode"].as_str(), Some("danger-full-access"));
        assert_eq!(doc["approval_policy"].as_str(), Some("never"));
        assert_eq!(doc["default_permissions"].as_str(), Some(":danger-full-access"));
        assert_eq!(doc["model_provider"].as_str(), Some("codex-plus-gateway"));
        let state: Value = serde_json::from_slice(&fs::read(home.join(".codex-global-state.json")).unwrap()).unwrap();
        let atoms = &state["electron-persisted-atom-state"];
        assert_eq!(atoms["agent-mode-by-host-id"]["local"], "full-access");
        assert_eq!(atoms["electron:onboarding-welcome-v2-role-state"]["roles"], serde_json::json!(["engineering"]));
        assert_eq!(atoms["electron:onboarding-welcome-v2-role-state"]["workMode"], "coding");
        assert_eq!(atoms["electron:onboarding-welcome-v2-role-state"]["personalizedSuggestionsEnabled"], true);
        assert_eq!(atoms["skip-full-access-confirm"], true);
        assert!(atoms["config-derived-agent-mode-by-host-id"]["local"].is_null());
        assert_eq!(atoms["config-derived-agent-mode-by-host-id"]["remote-test"], "read-only");
        assert_eq!(atoms["permission-selection-by-host-id:local"]["agentMode"], "full-access");
        assert!(atoms["composer-permission-mode-visibility"].as_bool() == Some(true)
            || atoms["composer-permission-mode-visibility"]["full-access"].as_bool() == Some(true));
        assert_eq!(atoms["heartbeat-thread-permissions-by-id"]["thread-1"]["approvalPolicy"], "never");
        assert_eq!(atoms["heartbeat-thread-permissions-by-id"]["thread-1"]["sandboxPolicy"]["type"], "dangerFullAccess");
        assert_eq!(atoms["composer-permission-mode-visibility"]["full-access"], true);
        assert_eq!(atoms["composer-permission-mode-visibility"]["auto"], true);
        assert_eq!(atoms["agent-mode-by-host-id"]["remote-test"], "read-only");
        assert_eq!(atoms["permission-selection-by-host-id:remote-test"]["agentMode"], "read-only");
        assert_eq!(state["other"]["keep"], true);
        assert_eq!(atoms["theme"], "dark");
        // A visible Full access option and saved full-access choice are not
        // sufficient: the legacy composer ignores a choice equal to its cache.
        let mut stale = state.clone();
        stale["electron-persisted-atom-state"]["config-derived-agent-mode-by-host-id"]["local"] =
            serde_json::json!("full-access");
        fs::write(home.join(".codex-global-state.json"), serde_json::to_vec(&stale).unwrap()).unwrap();
        assert!(verify_full_access(&home).is_err());
        commit(option_files(&home, true, false).unwrap()).unwrap();
        verify_full_access(&home).unwrap();
        assert!(option_files(&home, false, false).unwrap().is_empty());
        assert!(option_files(&home, false, true).unwrap().iter()
            .all(|(path, _)| path.file_name().unwrap() != ".codex-global-state.json"));
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn onboarding_completion_is_independent_of_configured_permissions() {
        let home = std::env::temp_dir().join(format!("onboarding-marker-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        assert!(!onboarding_complete(&home).unwrap());
        commit(option_files(&home, true, false).unwrap()).unwrap();
        verify_full_access(&home).unwrap();
        assert!(!onboarding_complete(&home).unwrap());
        let path = home.join(".codex-global-state.json");
        // Match the target machine after it finishes the non-coding onboarding.
        fs::write(&path, serde_json::to_vec(&serde_json::json!({
            "electron-persisted-atom-state": {
                "last_completed_onboarding":1794066673u64,
                "electron:onboarding-welcome-v2-role-state":{"workMode":"non_coding"},
                "composer-permission-mode-visibility":false,
                "agent-mode-by-host-id":{"local":"auto"}
            }
        })).unwrap()).unwrap();
        assert!(onboarding_complete(&home).unwrap());
        assert!(verify_full_access(&home).is_err());
        commit(live_files(&home, &fixture(), true, true, None, None).unwrap()).unwrap();
        verify_full_access(&home).unwrap();
        assert!(onboarding_complete(&home).unwrap());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn desktop_permission_visibility_supports_legacy_boolean_and_fresh_settings() {
        let home = std::env::temp_dir().join(format!("desktop-visibility-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        for legacy in [false, true] {
            fs::write(home.join(".codex-global-state.json"), serde_json::to_vec(&serde_json::json!({
                "electron-persisted-atom-state":{"composer-permission-mode-visibility":legacy}
            })).unwrap()).unwrap();
            let (_, bytes) = desktop_access_file(&home).unwrap();
            let state: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(state["electron-persisted-atom-state"]["composer-permission-mode-visibility"], true);
        }
        fs::remove_file(home.join(".codex-global-state.json")).unwrap();
        let (_, bytes) = desktop_access_file(&home).unwrap();
        let state: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(state["electron-persisted-atom-state"]["composer-permission-mode-visibility"], true);
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn direct_configuration_writes_auth_and_preserves_oauth_and_other_settings() {
        let home = std::env::temp_dir().join(format!("direct-provider-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("config.toml"), "model='old'\nprofile='old'\n[mcp_servers.keep]\ncommand='keep'\n[model_providers.custom]\nenv_key='STALE'\n").unwrap();
        fs::write(home.join("auth.json"), r#"{"tokens":{"access_token":"oauth-test"},"auth_mode":"chatgpt"}"#).unwrap();
        let files = live_files(
            &home,
            &fixture(),
            true,
            true,
            Some("http://127.0.0.1:47822/models"),
            Some("C:/Users/test/.codex/model-catalog.json"),
        ).unwrap();
        let doc: DocumentMut = std::str::from_utf8(&files[0].1).unwrap().parse().unwrap();
        assert_eq!(doc["model"].as_str(), Some("test-model"));
        assert_eq!(doc["model_reasoning_effort"].as_str(), Some("high"));
        assert_eq!(doc["model_providers"]["custom"]["requires_openai_auth"].as_bool(), Some(false));
        assert_eq!(doc["model_providers"]["custom"]["experimental_bearer_token"].as_str(), Some("test-secret"));
        assert_eq!(doc["model_providers"]["custom"]["model_catalog_url"].as_str(), Some("http://127.0.0.1:47822/models"));
        assert_eq!(doc["features"]["api_key_model_discovery"].as_bool(), Some(true));
        assert_eq!(doc["model_catalog_json"].as_str(), Some("C:/Users/test/.codex/model-catalog.json"));
        assert!(doc["model_providers"]["custom"].get("env_key").is_none());
        assert_eq!(doc["mcp_servers"]["keep"]["command"].as_str(), Some("keep"));
        let auth: Value = serde_json::from_slice(&files[1].1).unwrap();
        assert_eq!(auth["auth_mode"], "apikey");
        assert_eq!(auth["tokens"]["access_token"], "oauth-test");
        commit(files).unwrap();
        assert!(home.join("backup").is_dir());
        let mut mixed = fixture();
        mixed["relayMode"] = serde_json::json!("officialMixApi");
        let files = live_files(&home, &mixed, false, false, None, None).unwrap();
        let doc: DocumentMut = std::str::from_utf8(&files[0].1).unwrap().parse().unwrap();
        assert_eq!(doc["model_providers"]["custom"]["requires_openai_auth"].as_bool(), Some(true));
        let auth: Value = serde_json::from_slice(&files[1].1).unwrap();
        assert_eq!(auth["auth_mode"], "chatgpt");
        assert_eq!(auth["tokens"]["access_token"], "oauth-test");
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn conversion_and_gateway_are_not_silently_applied_as_direct() {
        let mut profile = fixture();
        profile["protocol"] = serde_json::json!("chatCompletions");
        assert!(!direct_supported(&profile));
        profile["protocol"] = serde_json::json!("responses");
        profile["relayMode"] = serde_json::json!("aggregate");
        assert!(!direct_supported(&profile));
        assert!(gateway_active(&serde_json::json!({"activeRelayId":"codex-plus-gateway"})));
    }
    #[test]
    fn invalid_live_config_is_not_overwritten_and_mixed_needs_login() {
        let home = std::env::temp_dir().join(format!("invalid-provider-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("config.toml"), "[broken").unwrap();
        assert!(live_files(&home, &fixture(), false, false, None, None).is_err());
        assert_eq!(fs::read_to_string(home.join("config.toml")).unwrap(), "[broken");
        fs::write(home.join("config.toml"), "model_providers = 'invalid-type'").unwrap();
        assert!(live_files(&home, &fixture(), false, false, None, None).is_err());
        fs::remove_file(home.join("config.toml")).unwrap();
        let mut profile = fixture();
        profile["relayMode"] = serde_json::json!("officialMixApi");
        assert!(live_files(&home, &profile, false, false, None, None).unwrap_err().contains("官方登录"));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn switching_updates_live_files_and_gateway_or_conversion_errors_leave_them_unchanged() {
        let home = std::env::temp_dir().join(format!("provider-switch-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        let settings = home.join("settings.json");
        fs::write(home.join("models_cache.json"), serde_json::to_vec(&serde_json::json!({
            "models":[{"slug":"native", "supported_in_api":true, "base_instructions":"Native instructions"}]
        })).unwrap()).unwrap();
        let first = fixture();
        let mut second = fixture();
        second["id"] = serde_json::json!("second");
        second["testModel"] = serde_json::json!("second-model");
        second["authContents"] = serde_json::json!("{\"OPENAI_API_KEY\":\"second-key\"}");
        let mut root = serde_json::json!({"activeRelayId":"test","relayProfiles":[first, second]});
        fs::write(&settings, serde_json::to_vec(&root).unwrap()).unwrap();
        super::super::activate_codexpp_provider_at(&settings, &home, "second").unwrap();
        let config = fs::read(home.join("config.toml")).unwrap();
        let auth = fs::read(home.join("auth.json")).unwrap();
        assert!(String::from_utf8_lossy(&config).contains("second-model"));
        assert_eq!(serde_json::from_slice::<Value>(&auth).unwrap()["OPENAI_API_KEY"], "second-key");
        root = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
        assert_eq!(root["activeRelayId"], "second");
        root["relayProfiles"][0]["protocol"] = serde_json::json!("chatCompletions");
        fs::write(&settings, serde_json::to_vec(&root).unwrap()).unwrap();
        assert!(super::super::activate_codexpp_provider_at(&settings, &home, "test").is_err());
        root["activeRelayId"] = serde_json::json!("codex-plus-gateway");
        fs::write(&settings, serde_json::to_vec(&root).unwrap()).unwrap();
        assert!(super::super::activate_codexpp_provider_at(&settings, &home, "second").is_err());
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), config);
        assert_eq!(fs::read(home.join("auth.json")).unwrap(), auth);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn write_failure_rolls_back_previously_written_files() {
        let home = std::env::temp_dir().join(format!("provider-rollback-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        let first = home.join("config.toml");
        fs::write(&first, b"original").unwrap();
        let catalog = home.join("model-catalog.json");
        fs::write(&catalog, b"original catalog").unwrap();
        // The destination component is valid, but its unique temporary name exceeds 255 characters.
        let second = home.join(format!("{}.json", "x".repeat(240)));
        let result = commit(vec![(first.clone(), b"changed".to_vec()), (catalog.clone(), b"new catalog".to_vec()), (second, b"new".to_vec())]);
        assert!(result.is_err());
        assert_eq!(fs::read(first).unwrap(), b"original");
        assert_eq!(fs::read(catalog).unwrap(), b"original catalog");
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn inline_tables_and_fresh_profiles_remain_valid_toml() {
        let home = std::env::temp_dir().join(format!("provider-inline-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("config.toml"),
            "desktop = { theme = 'dark' }\nmodel_providers = { other = { name = 'keep' } }\n").unwrap();
        let files = live_files(&home, &fixture(), false, true, None, None).unwrap();
        let doc: DocumentMut = std::str::from_utf8(&files[0].1).unwrap().parse().unwrap();
        assert_eq!(doc["desktop"]["theme"].as_str(), Some("dark"));
        assert_eq!(doc["desktop"]["localeOverride"].as_str(), Some("zh-CN"));
        assert_eq!(doc["model_providers"]["other"]["name"].as_str(), Some("keep"));
        let generated = profile_config("Test", "https://example.test", "model", "key", false);
        let doc: DocumentMut = generated.parse().unwrap();
        assert_eq!(doc["model_providers"]["custom"]["wire_api"].as_str(), Some("responses"));
        fs::remove_dir_all(home).unwrap();
    }
}
