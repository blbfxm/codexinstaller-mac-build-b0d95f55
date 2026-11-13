//! Restore official authentication without moving, rewriting or deleting conversation data.
use std::{fs, path::{Path, PathBuf}};
use serde_json::{json, Value};
use toml_edit::{value, DocumentMut};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResetReport {
    pub backup_directories: Vec<PathBuf>,
}

fn read_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            // A configuration link could point into conversation data or an unrelated file.
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(format!("配置路径不是普通文件，未执行恢复：{}", path.display()));
            }
            fs::read(path).map(Some).map_err(|e| format!("读取配置失败：{e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("检查配置失败：{e}")),
    }
}

pub(crate) fn plan(home: &Path, settings: &Path) -> Result<Vec<(PathBuf, Option<Vec<u8>>)>, String> {
    let config_path = home.join("config.toml");
    let old_config = read_file(&config_path)?.unwrap_or_default();
    let mut doc = std::str::from_utf8(&old_config).map_err(|_| "Codex 配置不是 UTF-8，未执行恢复")?
        .trim_start_matches('\u{feff}').parse::<DocumentMut>()
        .map_err(|_| "Codex 配置格式无效，未执行恢复，请先修复配置文件")?;
    // Remove all sources of provider/profile overrides, but retain unrelated MCP, skills,
    // UI and permissions preferences. Conversation metadata is deliberately never opened.
    for key in ["model_provider", "model_providers", "model", "model_catalog_json",
        "model_catalog_url", "model_reasoning_effort", "model_reasoning_summary",
        "model_verbosity", "review_model", "profile", "profiles", "forced_login_method",
        "forced_chatgpt_workspace_id", "chatgpt_base_url"] {
        doc.remove(key);
    }
    if let Some(features) = doc.get_mut("features").and_then(|v| v.as_table_like_mut()) {
        features.remove("api_key_model_discovery");
    }
    doc["model_provider"] = value("openai");
    doc["forced_login_method"] = value("chatgpt");
    // Ignore retained OS-keyring credentials. Missing auth.json means logged out;
    // an empty object is NOT equivalent: account/read in Codex 0.167.0-alpha.3.2
    // fails with "plan type is required for chatgpt authentication" for {}.
    doc["cli_auth_credentials_store"] = value("file");
    read_file(&home.join("auth.json"))?;
    let mut files = vec![(config_path, Some(doc.to_string().into_bytes())),
        (home.join("auth.json"), None)];
    if let Some(bytes) = read_file(settings)? {
        let mut settings_json: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "供应商列表配置格式无效，未执行恢复")?;
        let object = settings_json.as_object_mut().ok_or("供应商列表配置格式无效，未执行恢复")?;
        for key in ["activeRelayId", "relayBaseUrl", "relayApiKey"] { object.remove(key); }
        object.insert("relayProfilesEnabled".into(), json!(false));
        files.push((settings.to_path_buf(), Some(serde_json::to_vec_pretty(&settings_json).map_err(|e| e.to_string())?)));
    }
    Ok(files)
}

pub(crate) fn apply(files: Vec<(PathBuf, Option<Vec<u8>>)>) -> Result<ResetReport, String> {
    Ok(ResetReport { backup_directories: crate::provider_config::commit_changes(files)? })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let path = std::env::temp_dir().join(format!("codex-reset-{}-{stamp}", std::process::id()));
            fs::create_dir_all(&path).unwrap(); Self(path)
        }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
    #[test]
    fn restores_official_login_preserving_every_history_file_and_saved_provider() {
        let root = Fixture::new(); let home = root.0.join("home"); fs::create_dir_all(&home).unwrap();
        let history = ["sessions/a/old.jsonl", "archived_sessions/old.jsonl", "state_5.sqlite",
            "state_5.sqlite-wal", "state_5.sqlite-shm", "history.jsonl", "session_index.jsonl",
            ".codex-global-state.json", "skills/example/SKILL.md", "plugins/example/manifest.json",
            "attachments/photo.png"];
        for path in history {
            let path = home.join(path); fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"conversation-and-user-data\0\xff").unwrap();
        }
        let config = "model_provider='custom'\nmodel='third-party'\nprofile='old'\nforced_login_method='api'\nmodel_catalog_json='catalog.json'\n[model_providers.custom]\nexperimental_bearer_token='secret'\n[profiles.old]\nmodel_provider='custom'\n[mcp_servers.test]\ncommand='keep'\n[desktop]\nlocaleOverride='zh-CN'\n";
        fs::write(home.join("config.toml"), config).unwrap();
        fs::write(home.join("auth.json"), br#"{"OPENAI_API_KEY":"secret","tokens":{"access_token":"old-token"}}"#).unwrap();
        let settings = root.0.join("settings.json");
        let profiles = json!([{"id":"nut", "authContents":"saved-key"}]);
        fs::write(&settings, json!({"activeRelayId":"nut","relayBaseUrl":"https://example.test",
            "relayApiKey":"old-key","relayProfiles":profiles,"unrelated":true}).to_string()).unwrap();
        let report = apply(plan(&home, &settings).unwrap()).unwrap();
        let doc: DocumentMut = fs::read_to_string(home.join("config.toml")).unwrap().parse().unwrap();
        assert_eq!(doc["model_provider"].as_str(), Some("openai"));
        assert_eq!(doc["forced_login_method"].as_str(), Some("chatgpt"));
        assert_eq!(doc["cli_auth_credentials_store"].as_str(), Some("file"));
        assert!(!doc.contains_key("profile")); assert!(!doc.contains_key("model_providers"));
        assert!(!doc.contains_key("model_catalog_json"));
        assert_eq!(doc["mcp_servers"]["test"]["command"].as_str(), Some("keep"));
        assert_eq!(doc["desktop"]["localeOverride"].as_str(), Some("zh-CN"));
        assert!(!home.join("auth.json").exists());
        for path in history { assert_eq!(fs::read(home.join(path)).unwrap(), b"conversation-and-user-data\0\xff"); }
        let settings_json: Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
        assert_eq!(settings_json["relayProfiles"], profiles); assert!(settings_json["activeRelayId"].is_null());
        assert!(settings_json["relayBaseUrl"].is_null()); assert!(settings_json["relayApiKey"].is_null());
        assert_eq!(settings_json["unrelated"], true);
        let backup = report.backup_directories.iter().find(|p| p.join("config.toml").is_file()).unwrap();
        assert_eq!(fs::read_to_string(backup.join("config.toml")).unwrap(), config);
        assert!(fs::read_to_string(backup.join("auth.json")).unwrap().contains("old-token"));
    }
    #[test]
    fn invalid_settings_abort_before_any_configuration_is_changed() {
        let root = Fixture::new(); let settings = root.0.join("settings.json");
        fs::write(root.0.join("config.toml"), "model_provider='custom'\n").unwrap();
        fs::write(&settings, b"broken").unwrap();
        assert!(plan(&root.0, &settings).is_err());
        assert_eq!(fs::read_to_string(root.0.join("config.toml")).unwrap(), "model_provider='custom'\n");
        assert!(!root.0.join("auth.json").exists());
    }
    #[test]
    fn repeated_reset_is_supported_without_existing_settings_or_history() {
        let root = Fixture::new(); let settings = root.0.join("absent/settings.json");
        for _ in 0..2 { apply(plan(&root.0, &settings).unwrap()).unwrap(); }
        assert!(!settings.exists()); assert!(!root.0.join("sessions").exists());
        assert!(!root.0.join("auth.json").exists());
    }
    #[test]
    fn invalid_destination_preserves_previous_config_and_history() {
        let root = Fixture::new(); let config = root.0.join("config.toml");
        fs::write(&config, b"old").unwrap(); fs::write(root.0.join("history.jsonl"), b"keep").unwrap();
        // Preflight rejects a directory before any replacement.
        let directory = root.0.join("blocked"); fs::create_dir(&directory).unwrap();
        let files = vec![(config.clone(), Some(b"new".to_vec())), (directory, Some(b"no".to_vec()))];
        assert!(apply(files).is_err());
        assert_eq!(fs::read(config).unwrap(), b"old");
        assert_eq!(fs::read(root.0.join("history.jsonl")).unwrap(), b"keep");
    }
    #[cfg(windows)]
    #[test]
    fn locked_auth_rolls_back_config_after_replacement() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = Fixture::new(); let config = root.0.join("config.toml"); let auth = root.0.join("auth.json");
        fs::write(&config, b"model_provider='custom'\n").unwrap();
        fs::write(&auth, b"original-auth").unwrap();
        // Allow reads/writes but prevent rename/delete so backup succeeds, replacement fails.
        let _lock = fs::OpenOptions::new().read(true).share_mode(3).open(&auth).unwrap();
        assert!(apply(plan(&root.0, &root.0.join("missing.json")).unwrap()).is_err());
        assert_eq!(fs::read(&config).unwrap(), b"model_provider='custom'\n");
        assert_eq!(fs::read(&auth).unwrap(), b"original-auth");
    }
    #[test]
    fn repairs_empty_auth_left_by_previous_reset_and_preserves_history() {
        let root = Fixture::new(); let auth = root.0.join("auth.json");
        fs::write(&auth, b"{}\n").unwrap();
        fs::write(root.0.join("history.jsonl"), b"original-history").unwrap();
        let report = apply(plan(&root.0, &root.0.join("missing-settings.json")).unwrap()).unwrap();
        assert!(!auth.exists(), "Logged out must mean missing auth, never an empty object");
        assert_eq!(fs::read(root.0.join("history.jsonl")).unwrap(), b"original-history");
        assert!(report.backup_directories.iter().any(|p| fs::read(p.join("auth.json")).ok().as_deref() == Some(b"{}\n")));
    }
    #[cfg(windows)]
    #[test]
    fn later_failure_restores_deleted_auth_and_original_config() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = Fixture::new(); let auth = root.0.join("auth.json");
        let config = root.0.join("config.toml"); let settings = root.0.join("settings.json");
        fs::write(&auth, b"original-token-data").unwrap();
        fs::write(&config, b"model_provider='custom'\n").unwrap();
        fs::write(&settings, b"{\"activeRelayId\":\"nut\"}").unwrap();
        let _lock = fs::OpenOptions::new().read(true).share_mode(3).open(&settings).unwrap();
        assert!(apply(plan(&root.0, &settings).unwrap()).is_err());
        assert_eq!(fs::read(auth).unwrap(), b"original-token-data");
        assert_eq!(fs::read(config).unwrap(), b"model_provider='custom'\n");
        assert_eq!(fs::read(settings).unwrap(), b"{\"activeRelayId\":\"nut\"}");
    }
    // Opt-in integration test against the actual desktop app-server, not a mocked parser.
    #[test]
    #[ignore = "Run scripts/check-codex-reset-auth.cjs with an installed Codex engine"]
    fn real_account_api_accepts_the_reset_without_credentials() {
        use std::{io::{BufRead, BufReader, Write}, process::{Command, Stdio}, sync::mpsc, time::Duration};
        fn account_api(engine: &Path, home: &Path) -> Vec<Value> {
            let mut command = Command::new(engine);
            for (name, _) in std::env::vars_os() {
                let upper = name.to_string_lossy().to_ascii_uppercase();
                if upper.starts_with("OPENAI_") || upper.starts_with("CODEX_") { command.env_remove(name); }
            }
            command.arg("app-server").env("CODEX_HOME", home).current_dir(home)
                .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
            #[cfg(windows)]
            { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
            struct Engine(std::process::Child);
            impl Drop for Engine { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
            let mut engine = Engine(command.spawn().expect("start isolated app-server"));
            let stdout = engine.0.stdout.take().unwrap(); let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if let Ok(message) = serde_json::from_str::<Value>(&line) { if tx.send(message).is_err() { break; } }
                }
            });
            let stdin = engine.0.stdin.as_mut().unwrap();
            let initialize = json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"reset_integration","version":"0.1.0"},"capabilities":{"experimentalApi":true}}});
            writeln!(stdin, "{initialize}").unwrap(); stdin.flush().unwrap();
            loop { let message = rx.recv_timeout(Duration::from_secs(20)).expect("initialize timeout");
                if message["id"] == 1 { assert!(message.get("error").is_none(), "{message}"); break; } }
            writeln!(stdin, "{}", json!({"method":"initialized"})).unwrap();
            writeln!(stdin, "{}", json!({"id":2,"method":"account/read","params":{"refreshToken":false}})).unwrap();
            writeln!(stdin, "{}", json!({"id":3,"method":"getAuthStatus","params":{"includeToken":false,"refreshToken":false}})).unwrap();
            stdin.flush().unwrap();
            let mut responses = Vec::new();
            while responses.len() < 2 {
                let message = rx.recv_timeout(Duration::from_secs(20)).expect("account API timeout");
                if message["id"] == 2 || message["id"] == 3 { responses.push(message); }
            }
            responses.sort_by_key(|message| message["id"].as_u64()); responses
        }
        let engine = PathBuf::from(std::env::var_os("CODEX_RESET_TEST_ENGINE").expect("test engine path"));
        let root = Fixture::new(); let home = root.0.join("home"); fs::create_dir_all(&home).unwrap();
        fs::write(home.join("config.toml"), "model_provider='openai'\nforced_login_method='chatgpt'\ncli_auth_credentials_store='file'\n").unwrap();
        fs::write(home.join("auth.json"), "{}\n").unwrap();
        fs::write(home.join("history.jsonl"), "preserved-history").unwrap();
        let broken = account_api(&engine, &home);
        assert!(broken[0]["error"]["message"].as_str().is_some_and(|message| message.contains("plan type is required")), "{broken:?}");
        println!("Reproduced prior reset: {}", broken[0]);
        let report = apply(plan(&home, &root.0.join("absent-settings.json")).unwrap()).unwrap();
        assert!(!home.join("auth.json").exists());
        let restored = account_api(&engine, &home);
        for message in &restored { assert!(message.get("error").is_none(), "{message}"); }
        assert!(restored[0]["result"]["account"].is_null());
        assert_eq!(restored[0]["result"]["requiresOpenaiAuth"], true);
        assert!(restored[1]["result"]["authMethod"].is_null());
        assert_eq!(restored[1]["result"]["requiresOpenaiAuth"], true);
        assert!(!home.join("auth.json").exists());
        assert_eq!(fs::read_to_string(home.join("history.jsonl")).unwrap(), "preserved-history");
        assert!(report.backup_directories.iter().any(|p| p.join("auth.json").is_file()));
        println!("Verified actual reset account/read: {}", restored[0]);
        println!("Verified actual reset getAuthStatus: {}", restored[1]);
    }
}
