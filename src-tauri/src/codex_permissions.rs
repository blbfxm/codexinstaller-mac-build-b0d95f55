use std::{net::TcpStream, time::{Duration, Instant}};
use serde_json::{Value, json};

const SCRIPT: &str = include_str!("codex_permissions.js");

pub(crate) fn apply(port: u16) -> Result<(), String> {
    // Best effort only: the composer variant may not expose this control.
    if port == 0 { return Ok(()); }
    let client = reqwest::blocking::Client::builder().no_proxy()
        .timeout(Duration::from_secs(2)).build().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut stage = "waiting-for-window".to_string();
    while Instant::now() < deadline {
        let attempt = || -> Result<String, String> {
            let targets: Vec<Value> = client.get(format!("http://127.0.0.1:{port}/json/list"))
                .send().and_then(|r| r.error_for_status()).and_then(|r| r.json())
                .map_err(|_| "waiting-for-control-connection".to_string())?;
            let target = targets.iter().find(|v| v["type"] == "page" &&
                v["url"].as_str().is_some_and(crate::codex_chinese::main_page))
                .ok_or("waiting-for-window")?;
            let url = target["webSocketDebuggerUrl"].as_str().ok_or("missing-control-url")?;
            let parsed = reqwest::Url::parse(url).map_err(|_| "invalid-control-url")?;
            if parsed.scheme() != "ws" || !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost"))
                || parsed.port() != Some(port) { return Err("invalid-control-url".into()); }
            let stream = TcpStream::connect_timeout(&format!("127.0.0.1:{port}").parse().unwrap(), Duration::from_secs(2))
                .map_err(|_| "waiting-for-control-connection")?;
            stream.set_read_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
            stream.set_write_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
            let (mut socket, _) = tungstenite::client(url, stream).map_err(|_| "waiting-for-control-connection")?;
            let value = crate::codex_chinese::call(&mut socket, 1, "Runtime.evaluate",
                json!({"expression": SCRIPT, "returnByValue": true}))?;
            let status = &value["result"]["value"];
            let result = if status["selected"] == true { "confirmed" } else { status["stage"].as_str().unwrap_or("unsupported-interface") };
            let _ = socket.close(None);
            Ok(result.into())
        };
        stage = attempt().unwrap_or_else(|_| "waiting-for-control-connection".into());
        if stage == "confirmed" { return Ok(()); }
        if stage == "blocked-by-policy" { return Ok(()); }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::{Read, Write}, net::TcpListener};
    use tungstenite::Message;

    #[test]
    fn live_selection_requires_renderer_confirmation() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for selected in [false, true] {
                let (mut http, _) = listener.accept().unwrap();
                http.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = [0u8; 4096];
                let length = http.read(&mut request).unwrap();
                assert!(String::from_utf8_lossy(&request[..length]).starts_with("GET /json/list "));
                let body = json!([{"type":"page","url":"app://-/index.html",
                    "webSocketDebuggerUrl":format!("ws://127.0.0.1:{port}/devtools/page/main")}]).to_string();
                write!(http, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                drop(http);
                let (stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut socket = tungstenite::accept(stream).unwrap();
                let request: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                assert_eq!(request["method"], "Runtime.evaluate");
                assert_eq!(request["params"]["expression"], SCRIPT);
                socket.send(Message::Text(json!({"id":request["id"],"result":{"result":{"value":{
                    "selected":selected,"stage":if selected { "confirmed" } else { "selecting-full-access" }
                }}}}).to_string().into())).unwrap();
                // Complete the client's WebSocket close handshake.
                let _ = socket.read();
                let _ = socket.flush();
            }
        });
        apply(port).unwrap();
        server.join().unwrap();
    }
}
