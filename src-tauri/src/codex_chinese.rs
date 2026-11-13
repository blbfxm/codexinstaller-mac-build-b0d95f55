use std::{net::{TcpListener, TcpStream}, path::{Path, PathBuf}, time::{Duration, Instant}};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

const SCRIPT: &str = include_str!("codex_chinese.js");

pub(crate) fn localization_files(home: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let mut files = crate::provider_config::option_files(home, false, true)?;
    files.push(preference_file(home, true));
    Ok(files)
}

pub(crate) fn preference_file(home: &Path, enabled: bool) -> (PathBuf, Vec<u8>) {
    (home.join(".usage-assistant-chinese.json"), json!({ "enabled": enabled }).to_string().into_bytes())
}

pub(crate) fn enabled(home: &Path) -> Result<bool, String> {
    let path = preference_file(home, true).0;
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
            .map(|v| v["enabled"] == true).map_err(|_| "Codex 强制中文设置格式无效".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("读取 Codex 强制中文设置失败：{error}")),
    }
}

pub(crate) fn configured(home: &Path) -> bool {
    preference_file(home, true).0.is_file()
}

// The caller has already saved the settings and launched Codex. An unavailable
// renderer check is inconclusive, not a failure of those completed operations.
pub(crate) fn localization_result(verification: Result<(), String>, persistent: bool) -> String {
    match verification {
        Ok(()) if persistent => "Codex 已持久化为简体中文，之后从访达或程序坞直接打开也会应用；应用更新后可再次点击汉化。".into(),
        Ok(()) => "Codex 已切换为简体中文，无需重新安装。".into(),
        Err(_) => "中文设置已保存，Codex 已启动。自动界面检查尚未完成；若界面已是中文，无需重复操作。若仍为英文，请等待主界面加载后再检查。".into(),
    }
}

pub(crate) fn reserve_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    Ok(listener.local_addr().map_err(|e| e.to_string())?.port())
}

fn confirmed(status: &Value) -> bool {
    status["locale"] == true && status["gate"] == true && status["rendered"] == true
}

pub(crate) fn main_page(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else { return false; };
    matches!(url.scheme(), "app" | "file") &&
        url.path().ends_with("/index.html") &&
        !url.query_pairs().any(|(key, _)| key == "initialRoute")
}

fn wait_for_loaded_page(socket: &mut WebSocket<TcpStream>, id: &mut u64, deadline: Instant) -> Result<(), String> {
    let mut loaded_since: Option<(String, Instant)> = None;
    while Instant::now() < deadline {
        let result = call(socket, *id, "Runtime.evaluate", json!({
            "expression": "({ready: document.readyState === 'complete' && (performance.getEntriesByType('navigation')[0]?.loadEventEnd || performance.timing?.loadEventEnd || 0) > 0, url: location.href})",
            "returnByValue": true
        }));
        *id += 1;
        match result {
            Ok(result) => {
                let value = &result["result"]["value"];
                let url = value["url"].as_str().unwrap_or_default();
                if value["ready"] == true && main_page(url) {
                    match &loaded_since {
                        Some((previous, since)) if previous == url => {
                            if since.elapsed() >= Duration::from_millis(1000) { return Ok(()); }
                        }
                        _ => loaded_since = Some((url.into(), Instant::now())),
                    }
                } else {
                    loaded_since = None;
                }
            }
            Err(error) if error.contains("context") || error.contains("navigation") => loaded_since = None,
            Err(error) => return Err(error),
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err("Codex 主页面尚未完成首次加载；已跳过中文刷新，避免中断启动".into())
}

fn keep_registered(mut socket: WebSocket<TcpStream>, mut id: u64) {
    // New-document scripts belong to the DevTools session. Keep it attached across reloads.
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(3));
            let result = call(&mut socket, id, "Runtime.evaluate",
                json!({"expression": "({installed: Boolean(window.__nutCodexChinese), url: location.href})", "returnByValue": true}));
            id += 1;
            match result {
                Ok(result) if !result["result"]["value"]["url"].as_str().is_some_and(main_page) => break,
                Ok(result) if result["result"]["value"]["installed"] == true => {}
                Ok(_) => {
                    // Also cover a document created before the new-document hook could run.
                    if call(&mut socket, id, "Runtime.evaluate", json!({"expression": SCRIPT})).is_err() {
                        break;
                    }
                    id += 1;
                }
                Err(error) if error.contains("context") || error.contains("navigation") => continue,
                Err(_) => break,
            }
        }
        let _ = socket.close(None);
    });
}

pub(crate) fn call(socket: &mut WebSocket<TcpStream>, id: u64, method: &str, params: Value) -> Result<Value, String> {
    socket.send(Message::Text(json!({"id": id, "method": method, "params": params}).to_string().into()))
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        let message = socket.read().map_err(|e| e.to_string())?;
        if let Message::Text(text) = message {
            let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if value["id"] != id { continue; }
            if let Some(error) = value.get("error") { return Err(error.to_string()); }
            if let Some(error) = value["result"].get("exceptionDetails") { return Err(error.to_string()); }
            return Ok(value["result"].clone());
        }
    }
    Err("Codex 中文界面响应超时".into())
}

pub(crate) fn apply(port: u16) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder().no_proxy()
        .timeout(Duration::from_secs(2)).build().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(65);
    let mut last_error = String::new();
    let mut refreshed = false;
    while Instant::now() < deadline {
        let mut attempt = || -> Result<(), String> {
            let targets: Vec<Value> = client.get(format!("http://127.0.0.1:{port}/json/list"))
                .send().and_then(|r| r.error_for_status()).and_then(|r| r.json())
                .map_err(|_| "无法连接 Codex 的本机中文控制服务。请关闭 Codex 错误窗口并完全退出 Codex 后，再通过助手启动；这不是订阅或网络代理连接错误。".to_string())?;
            let target = targets.iter().find(|v| v["type"] == "page" &&
                v["url"].as_str().is_some_and(main_page))
                .ok_or("等待 Codex 主窗口")?;
            let url = target["webSocketDebuggerUrl"].as_str().ok_or("Codex 调试地址缺失")?;
            let parsed = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
            if parsed.scheme() != "ws" || !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost"))
                || parsed.port() != Some(port) {
                return Err("Codex 调试地址不是预期的本机端口".into());
            }
            let stream = TcpStream::connect_timeout(&format!("127.0.0.1:{port}").parse().unwrap(), Duration::from_secs(2))
                .map_err(|e| e.to_string())?;
            stream.set_read_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
            stream.set_write_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
            let (mut socket, _) = tungstenite::client(url, stream).map_err(|e| e.to_string())?;
            call(&mut socket, 1, "Page.enable", json!({}))?;
            call(&mut socket, 2, "Page.addScriptToEvaluateOnNewDocument", json!({"source": SCRIPT}))?;
            let mut id = 3;
            // A visible target may still have loadURL pending in Codex's main process.
            // Reloading it early rejects that startup promise and can trigger its error page.
            wait_for_loaded_page(&mut socket, &mut id, deadline)?;
            if !refreshed {
                refreshed = true;
                call(&mut socket, id, "Page.reload", json!({"ignoreCache": false}))?;
            } else {
                call(&mut socket, id, "Runtime.evaluate", json!({"expression": SCRIPT}))?;
            }
            id += 1;
            while Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(350));
                let result = call(&mut socket, id, "Runtime.evaluate",
                    json!({"expression": "window.__nutCodexChinese", "returnByValue": true}));
                id += 1;
                // The old execution context can disappear while the initial reload completes.
                let result = match result {
                    Ok(result) => result,
                    Err(error) if error.contains("context") || error.contains("navigation") => continue,
                    Err(error) => return Err(error),
                };
                if confirmed(&result["result"]["value"]) {
                    keep_registered(socket, id);
                    return Ok(());
                }
                if let Some(error) = result["result"]["value"]["error"].as_str().filter(|s| !s.is_empty()) {
                    last_error = format!("中文设置同步失败：{error}");
                } else if result["result"]["value"]["locale"] == true {
                    last_error = "尚未检测到中文界面及本地中文资源加载完成".into();
                }
            }
            Err(if last_error.is_empty() { "Codex 未确认中文界面加载完成".into() } else { last_error.clone() })
        };
        match attempt() {
            Ok(()) => return Ok(()),
            Err(error) => last_error = error,
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Err(format!("未能完成 Codex 中文启动：{last_error}"))
}

pub(crate) fn apply_in_background(port: u16) {
    std::thread::spawn(move || {
        let _ = apply(port);
    });
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    #[test]
    fn inconclusive_renderer_check_does_not_report_saved_localization_as_failed() {
        for persistent in [false, true] {
            for error in ["中文界面响应超时", "本机控制服务未连接", "语言设置服务不可用"] {
                let message = super::localization_result(Err(error.into()), persistent);
                assert!(message.contains("中文设置已保存，Codex 已启动"));
                assert!(message.contains("若界面已是中文，无需重复操作"));
                assert!(!message.contains("失败"));
                assert!(!message.contains("已切换为简体中文"));
                assert!(!message.contains("已持久化为简体中文"));
            }
        }
        assert!(super::localization_result(Ok(()), false).contains("已切换为简体中文"));
        assert!(super::localization_result(Ok(()), true).contains("已持久化为简体中文"));
    }

    #[test]
    fn standalone_localization_changes_only_language_and_preserves_user_configuration() {
        let home = std::env::temp_dir().join(format!("codex-localize-only-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let original = "model='existing-model'\nmodel_provider='custom'\napproval_policy='on-request'\n[desktop]\ntheme='dark'\nlocaleOverride='en-US'\n[model_providers.custom]\nbase_url='https://example.org/v1'\n";
        std::fs::write(home.join("config.toml"), original).unwrap();
        for name in ["auth.json", ".codex-global-state.json", "history.jsonl"] { std::fs::write(home.join(name), b"untouched fixture").unwrap(); }
        let files = super::localization_files(&home).unwrap();
        assert_eq!(files.len(), 2);
        crate::provider_config::commit(files).unwrap();
        let doc: toml_edit::DocumentMut = std::fs::read_to_string(home.join("config.toml")).unwrap().parse().unwrap();
        assert_eq!(doc["desktop"]["localeOverride"].as_str(), Some("zh-CN"));
        assert_eq!(doc["desktop"]["theme"].as_str(), Some("dark"));
        assert_eq!(doc["model"].as_str(), Some("existing-model"));
        assert_eq!(doc["model_providers"]["custom"]["base_url"].as_str(), Some("https://example.org/v1"));
        assert_eq!(doc["approval_policy"].as_str(), Some("on-request"));
        assert!(super::enabled(&home).unwrap());
        for name in ["auth.json", ".codex-global-state.json", "history.jsonl"] { assert_eq!(std::fs::read(home.join(name)).unwrap(), b"untouched fixture"); }
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn waits_for_initial_navigation_before_reloading() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            assert!(String::from_utf8_lossy(&request).starts_with("GET /json/list "));
            let body = serde_json::json!([{
                "type": "page", "url": "app://-/index.html",
                "webSocketDebuggerUrl": format!("ws://127.0.0.1:{port}/devtools/page/test")
            }]).to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            drop(stream);
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let mut ready_checks = 0;
            let mut ready_since = None;
            let mut reloaded = false;
            loop {
                let msg = socket.read().unwrap().into_text().unwrap();
                let request: serde_json::Value = serde_json::from_str(&msg).unwrap();
                let method = request["method"].as_str().unwrap();
                let result = match method {
                    "Page.enable" => {
                        assert_eq!(request["id"], 1);
                        serde_json::json!({})
                    }
                    "Page.addScriptToEvaluateOnNewDocument" => {
                        assert_eq!(request["id"], 2);
                        assert!(request["params"]["source"].as_str().unwrap().contains("enable_i18n"));
                        serde_json::json!({})
                    }
                    "Runtime.evaluate" if !reloaded => {
                        ready_checks += 1;
                        if ready_checks > 2 {
                            ready_since.get_or_insert(std::time::Instant::now());
                        }
                        assert!(request["params"]["expression"].as_str().unwrap().contains("loadEventEnd"));
                        serde_json::json!({"result": {"value": {"ready": ready_checks > 2, "url": "app://-/index.html"}}})
                    }
                    "Page.reload" => {
                        assert!(ready_since.is_some_and(|since| since.elapsed() >= std::time::Duration::from_secs(1)),
                            "must wait for both loading and a stable completed navigation");
                        assert!(!reloaded, "startup must not reload repeatedly");
                        reloaded = true;
                        serde_json::json!({})
                    }
                    "Runtime.evaluate" => serde_json::json!({"result": {"value": {
                        "locale": true, "gate": true, "rendered": true
                    }}}),
                    _ => panic!("Unexpected command {method}"),
                };
                socket.send(tungstenite::Message::Text(serde_json::json!({
                    "id": request["id"], "result": result
                }).to_string().into())).unwrap();
                if reloaded && method == "Runtime.evaluate" { break; }
            }
        });
        super::apply(port).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn error_pages_and_auxiliary_windows_are_not_reload_targets() {
        assert!(super::main_page("app://-/index.html"));
        assert!(super::main_page("file:///C:/Codex/webview/index.html"));
        assert!(!super::main_page("data:text/html;charset=utf-8,%3Chtml%3E"));
        assert!(!super::main_page("about:blank"));
        assert!(!super::main_page("app://-/index.html?initialRoute=quick-chat"));
        assert!(!super::main_page("app://-/loading.html"));
        assert!(!super::main_page("https://example.test/index.html"));
    }

    #[test]
    fn saved_settings_alone_do_not_confirm_chinese_ui() {
        assert!(!super::confirmed(&serde_json::json!({"locale": true, "gate": true})));
        assert!(!super::confirmed(&serde_json::json!({"locale": true, "gate": false, "rendered": true})));
        assert!(super::confirmed(&serde_json::json!({"locale": true, "gate": true, "rendered": true})));
    }

    #[test]
    fn preference_is_explicit_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("nut-chinese-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for enabled in [true, false] {
            let (path, bytes) = super::preference_file(&dir, enabled);
            std::fs::write(&path, bytes).unwrap();
            assert_eq!(super::enabled(&dir).unwrap(), enabled);
            std::fs::remove_file(path).unwrap();
        }
        assert!(!super::enabled(&dir).unwrap());
        std::fs::remove_dir(dir).unwrap();
    }
}
