use serde_json::{Value, json};
use std::time::Duration;

const BASE: &str = "https://api.jianguo.ltd/api/v1/wallet";

pub fn trusted_payment_url(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else { return false };
    let Some(host) = url.host_str() else { return false };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && (host == "alipay.com" || host.ends_with(".alipay.com") || host == "api.jianguo.ltd")
}

fn credentials(account_id: u64, kami: &str) -> Result<Value, String> {
    if kami.trim().is_empty() {
        return Err("请先使用卡密登录，再进行充值".into());
    }
    Ok(json!({ "accountId": account_id, "mainKami": kami.trim() }))
}

fn create_body(account_id: u64, kami: &str, value: f64, custom: bool) -> Result<Value, String> {
    if !value.is_finite() || value < 0.01 || (value * 100.0 - (value * 100.0).round()).abs() > 0.000001 {
        return Err("请输入有效金额，最低 0.01 元，最多两位小数".into());
    }
    let mut body = credentials(account_id, kami)?;
    body[if custom { "amount" } else { "points" }] = json!(value);
    body["channel"] = json!("alipay");
    body["paymentMode"] = json!("cashier");
    Ok(body)
}

async fn post(path: &str, body: Value) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build().map_err(|_| "无法创建支付连接")?;
    let response = client.post(format!("{BASE}/{path}")).json(&body).send().await
        .map_err(|_| "无法连接充值服务，请检查网络后重试")?;
    let status = response.status();
    let payload: Value = response.json().await.map_err(|_| "充值服务返回了无法识别的数据，请稍后重试")?;
    if !status.is_success() || payload.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(payload.get("message").and_then(Value::as_str)
            .filter(|message| message.len() <= 400)
            .unwrap_or("充值服务暂不可用，请稍后重试").to_owned());
    }
    Ok(payload)
}

fn public_order(payload: Value) -> Result<Value, String> {
    let order = payload.get("order").filter(|order| order.is_object()).ok_or("服务器未返回有效支付订单")?;
    let info = order.get("paymentInfo").or_else(|| order.get("qrCode")).and_then(Value::as_str).unwrap_or("");
    if !info.is_empty() && !trusted_payment_url(info) {
        return Err("支付服务返回了不受信任的付款地址，已阻止加载".into());
    }
    // Return only payment UI fields, never account keys or the server's raw object.
    Ok(json!({
        "orderNo": order.get("orderNo"),
        "points": order.get("points"),
        "amount": order.get("amount"),
        "status": order.get("status"),
        "paymentInfo": info,
        "paymentType": order.get("paymentType"),
        "balance": payload.get("balance"),
    }))
}

#[tauri::command]
pub async fn wallet_catalog() -> Result<Value, String> {
    let payload = post("packages", json!({})).await?;
    Ok(json!({
        "packages": payload.get("packages"),
        "paymentReady": payload.get("paymentReady").and_then(Value::as_bool) == Some(true)
            || payload.get("alipayReady").and_then(Value::as_bool) == Some(true),
    }))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn wallet_create_order(account_id: u64, kami: String, value: f64, custom: bool) -> Result<Value, String> {
    let path = if custom { "create-order-by-amount" } else { "create-order" };
    public_order(post(path, create_body(account_id, &kami, value, custom)?).await?)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn wallet_order_status(account_id: u64, kami: String, order_no: String) -> Result<Value, String> {
    if order_no.is_empty() || order_no.len() > 100 || !order_no.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_') {
        return Err("订单编号无效".into());
    }
    let mut body = credentials(account_id, &kami)?;
    body["orderNo"] = json!(order_no);
    public_order(post("order-status", body).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_reuse_existing_wallet_contract_without_credentials_in_urls() {
        let body = create_body(7, " test-key ", 300.0, false).unwrap();
        assert_eq!(body, json!({"accountId":7,"mainKami":"test-key","points":300.0,"channel":"alipay","paymentMode":"cashier"}));
        assert_eq!(create_body(7, "key", 0.01, true).unwrap()["amount"], 0.01);
        for amount in [0.0, -1.0, f64::NAN, f64::INFINITY, 1.001] {
            assert!(create_body(7, "key", amount, true).is_err());
        }
        assert_eq!(create_body(0, "key", 20.0, true).unwrap()["accountId"], 0);
        assert!(create_body(7, "", 20.0, true).is_err());
    }

    #[test]
    fn rejects_unsafe_cashier_urls_and_removes_sensitive_order_fields() {
        assert!(trusted_payment_url("https://openapi.alipay.com/gateway.do?test=1"));
        assert!(trusted_payment_url("https://qr.alipay.com/example"));
        for value in ["javascript:alert(1)", "http://alipay.com/", "https://alipay.com.evil.test/", "https://evilalipay.com/", "https://key@alipay.com/", "https://alipay.com:444/"] {
            assert!(!trusted_payment_url(value));
        }
        let result = public_order(json!({"order": {"orderNo":"TEST","status":"paid","mainKami":"secret","paymentInfo":"https://qr.alipay.com/test"},"balance":25})).unwrap();
        assert!(result.get("mainKami").is_none());
        assert_eq!(result["balance"], 25);
        assert!(public_order(json!({"order":{"paymentInfo":"https://evil.test/"}})).is_err());
    }
}
