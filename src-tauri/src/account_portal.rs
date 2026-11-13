use base64::Engine;
use serde_json::{Value, json};
use std::time::Duration;
use tauri::State;

const ROOT: &str = "https://api.jianguo.ltd/api/v1";

#[derive(Default)]
struct Session {
    #[cfg(test)]
    test_root: Option<String>,
    cookie: String,
    csrf: String,
    username: String,
    password_hash: String,
    account: Value,
}

#[derive(Default)]
pub struct AccountPortal {
    session: tokio::sync::Mutex<Session>,
}

impl Session {
    async fn request(
        &mut self,
        method: reqwest::Method,
        path: &str,
        body: &Value,
    ) -> Result<reqwest::Response, String> {
        let root = ROOT;
        #[cfg(test)]
        let root = self.test_root.as_deref().unwrap_or(root);
        let client = reqwest::Client::builder()
            .https_only(!cfg!(test) || root.starts_with("https:"))
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .user_agent("CodexUsageAssistant")
            .build()
            .map_err(|_| "无法初始化账户连接")?;
        let mut request = client
            .request(method.clone(), format!("{root}{path}"))
            .header("X-Client", "codex-assistant");
        if !self.cookie.is_empty() {
            request = request.header(reqwest::header::COOKIE, &self.cookie);
        }
        if !self.csrf.is_empty() {
            request = request.header("X-Account-CSRF", &self.csrf);
        }
        if path == "/installer/codex-api-key" {
            if let Ok(token) = std::env::var("CODEX_ASSISTANT_TOKEN") {
                if !token.trim().is_empty() {
                    request = request.header("X-Assistant-Token", token.trim());
                }
            }
        }
        if method == reqwest::Method::POST {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| format!("账户服务连接失败，请检查网络：{}", e.without_url()))?;
        for header in response.headers().get_all(reqwest::header::SET_COOKIE) {
            if let Ok(value) = header.to_str() {
                if let Some(cookie) = value
                    .split(';')
                    .next()
                    .filter(|s| s.starts_with("JSESSIONID="))
                {
                    self.cookie = cookie.to_owned();
                }
            }
        }
        Ok(response)
    }

    async fn json(&mut self, path: &str, body: &Value) -> Result<Value, String> {
        let response = self.request(reqwest::Method::POST, path, body).await?;
        let status = response.status();
        let mut value: Value = response
            .json()
            .await
            .map_err(|_| format!("账户服务响应无效（HTTP {status}）"))?;
        if !status.is_success() {
            return Err(value["message"]
                .as_str()
                .unwrap_or("账户请求被拒绝，请重新登录后重试")
                .to_owned());
        }
        if let Some(account) = value["account"].as_object_mut() {
            account.remove("passwordHash");
            account.remove("password_hash");
        }
        if let Some(csrf) = value["csrfToken"].as_str() {
            self.csrf = csrf.into();
        }
        if value["ok"] == true && value["account"].is_object() {
            self.account = value["account"].clone();
        }
        Ok(value)
    }
}

#[tauri::command]
pub async fn account_portal(
    state: State<'_, AccountPortal>,
    action: String,
    payload: Option<Value>,
) -> Result<Value, String> {
    state.perform(&action, payload).await
}
impl AccountPortal {
    async fn perform(&self, action: &str, payload: Option<Value>) -> Result<Value, String> {
        let body = payload.unwrap_or(json!({}));
        let mut session = self.session.lock().await;
        match action {
            "captcha" => {
                let mut bytes = [0u8; 16];
                getrandom::fill(&mut bytes).map_err(|_| "生成验证码请求失败")?;
                let challenge = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
                let response = session
                    .request(
                        reqwest::Method::GET,
                        &format!("/captcha?challenge={challenge}"),
                        &Value::Null,
                    )
                    .await?;
                if !response.status().is_success() {
                    return Err("图形验证码加载失败，请重试".into());
                }
                let bytes = response.bytes().await.map_err(|_| "图形验证码读取失败")?;
                if bytes.len() > 256 * 1024 {
                    return Err("图形验证码大小异常".into());
                }
                Ok(
                    json!({"challenge": challenge, "image": format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))}),
                )
            }
            "sendCode" => session.json("/account/send-register-code", &body).await,
            "register" => {
                let value = session.json("/account/register", &body).await?;
                if value["ok"] == true {
                    session.username = body["email"].as_str().unwrap_or_default().into();
                    session.password_hash =
                        body["passwordHash"].as_str().unwrap_or_default().into();
                }
                Ok(value)
            }
            "login" => {
                #[cfg(test)]
                let test_root = session.test_root.clone();
                *session = Session::default();
                #[cfg(test)]
                {
                    session.test_root = test_root;
                }
                let username = body["username"].as_str().unwrap_or_default();
                let hash = body["passwordHash"].as_str().unwrap_or_default();
                let mut value = session.json("/account/login-password", &body).await?;
                if value["ok"] != true {
                    value = session.json("/installer/login", &json!({"loginType":"password", "username":username,"passwordHash":hash})).await?;
                }
                if value["ok"] == true {
                    session.username = username.into();
                    session.password_hash = hash.into();
                }
                Ok(value)
            }
            "profile" => {
                if session.username.is_empty() {
                    return Err("请先使用账号密码登录".into());
                }
                if session.csrf.is_empty() {
                    return Ok(json!({"ok":true,"account":session.account}));
                }
                let response = session
                    .request(reqwest::Method::GET, "/account/session", &Value::Null)
                    .await?;
                if !response.status().is_success() {
                    return Err("账户会话已过期，请重新登录".into());
                }
                let value: Value = response.json().await.map_err(|_| "读取账户信息失败")?;
                if value["ok"] == true {
                    session.account = value["account"].clone();
                }
                Ok(value)
            }
            "ensureKey" => {
                if session.username.is_empty() || session.password_hash.is_empty() {
                    return Err("请重新使用账号密码登录后获取 API Key".into());
                }
                let request =
                    json!({"account":session.username,"passwordHash":session.password_hash});
                session.json("/installer/codex-api-key", &request).await
            }
            "logout" => {
                if !session.csrf.is_empty() {
                    let _ = session.json("/account/logout", &json!({})).await;
                }
                *session = Session::default();
                Ok(json!({"ok":true}))
            }
            _ => Err("不支持的账户操作".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    #[tokio::test]
    async fn captcha_registration_key_and_logout_keep_cookie_and_csrf() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let root = format!("http://{}/api/v1", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let steps = [
                (
                    "GET /api/v1/captcha?challenge=",
                    "",
                    "",
                    "JSESSIONID=captcha; Path=/; HttpOnly",
                    "image/png",
                    "PNG",
                ),
                (
                    "POST /api/v1/account/send-register-code ",
                    "JSESSIONID=captcha",
                    "",
                    "",
                    "application/json",
                    r#"{"ok":true}"#,
                ),
                (
                    "POST /api/v1/account/register ",
                    "JSESSIONID=captcha",
                    "",
                    "JSESSIONID=registered; Path=/; HttpOnly",
                    "application/json",
                    r#"{"ok":true,"csrfToken":"csrf-register","account":{"id":11,"username":"tester","hasApiKey":false}}"#,
                ),
                (
                    "POST /api/v1/installer/codex-api-key ",
                    "JSESSIONID=registered",
                    "csrf-register",
                    "",
                    "application/json",
                    r#"{"ok":true,"apiKey":"fixture-key","created":true,"balance":0,"account":{"id":11,"username":"tester","mainKami":"fixture-key","passwordHash":"MUST-NOT-LEAK"}}"#,
                ),
                (
                    "POST /api/v1/account/logout ",
                    "JSESSIONID=registered",
                    "csrf-register",
                    "",
                    "application/json",
                    r#"{"ok":true}"#,
                ),
            ];
            for (path, cookie, csrf, set_cookie, content_type, body) in steps {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0u8; 1024];
                loop {
                    let count = stream.read(&mut byte).unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&byte[..count]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&request[..end]);
                        let len = header
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|s| s.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                let request = String::from_utf8_lossy(&request);
                assert!(request.starts_with(path), "unexpected route");
                let headers = request.to_lowercase();
                assert!(headers.contains("x-client: codex-assistant"));
                if !cookie.is_empty() {
                    assert!(headers.contains(&format!("cookie: {}", cookie.to_lowercase())));
                }
                if !csrf.is_empty() {
                    assert!(headers.contains(&format!("x-account-csrf: {csrf}")));
                }
                if path.contains("codex-api-key") {
                    let payload: Value =
                        serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
                    assert_eq!(payload["account"], "test@example.test");
                    assert_eq!(payload["passwordHash"], "a".repeat(64));
                }
                let cookie = if set_cookie.is_empty() {
                    String::new()
                } else {
                    format!("Set-Cookie: {set_cookie}\r\n")
                };
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{cookie}Connection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        let state = AccountPortal::default();
        state.session.lock().await.test_root = Some(root);
        let captcha = state.perform("captcha", None).await.unwrap();
        assert!(
            captcha["image"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert_eq!(state.perform("sendCode",Some(json!({"email":"test@example.test","captcha":"ABCD","captchaChallenge":captcha["challenge"]}))).await.unwrap()["ok"],true);
        assert_eq!(state.perform("register",Some(json!({"email":"test@example.test","emailCode":"123456","passwordHash":"a".repeat(64)}))).await.unwrap()["ok"],true);
        let key = state.perform("ensureKey", None).await.unwrap();
        assert_eq!(key["apiKey"], "fixture-key");
        assert!(key["account"]["passwordHash"].is_null());
        state.perform("logout", None).await.unwrap();
        assert!(state.perform("ensureKey", None).await.is_err());
        server.join().unwrap();
    }
}
