//! Read CYAPI account quotas directly from the selected API's /usage endpoint.
use reqwest::{
    blocking::Client,
    header::{ACCEPT, AUTHORIZATION},
    redirect::Policy,
    Url,
};
use serde::Serialize;
use serde_json::Value;
use std::{io::Read, time::Duration};

type Result<T> = std::result::Result<T, String>;
const MAX_BYTES: u64 = 1_048_576;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalancePlan {
    plan_name: String,
    is_valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remaining: Option<f64>,
    unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<String>,
}

fn endpoint(base_url: &str) -> Result<Url> {
    let text = base_url.trim();
    if text.is_empty() {
        return Err("请先选择 CYAPI 线路".into());
    }
    if text.len() > 2048 || text.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("CYAPI 地址格式无效".into());
    }
    let mut url = Url::parse(text).map_err(|_| "CYAPI 地址格式无效")?;
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("CYAPI 地址格式无效".into());
    }
    url.set_path(&format!("{}/usage", url.path().trim_end_matches('/')));
    Ok(url)
}

fn text<'a>(value: &'a Value, field: &str, fallback: &'a str) -> &'a str {
    value[field]
        .as_str()
        .filter(|text| !text.is_empty())
        .unwrap_or(fallback)
}

fn extract(response: &Value) -> Result<Vec<BalancePlan>> {
    if !response.is_object() {
        return Err("余额接口返回格式无效".into());
    }
    let is_valid = response["isValid"] != false
        && response["status"] != "expired"
        && response["status"] != "disabled";
    let unit = text(response, "unit", "USD");
    let mut plans = Vec::new();
    if let Some(items) = response["rate_limits"].as_array() {
        for item in items.iter().filter(|item| item.is_object()) {
            let total = item["limit"].as_f64().unwrap_or(0.0);
            let used = item["used"].as_f64().unwrap_or(0.0);
            let remaining = item["remaining"]
                .as_f64()
                .unwrap_or_else(|| (total - used).max(0.0));
            let reset = match &item["reset_at"] {
                Value::String(value) if !value.is_empty() => Some(value.clone()),
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            };
            plans.push(BalancePlan {
                plan_name: text(item, "window", "额度").into(),
                is_valid,
                total: Some(total),
                used: Some(used),
                remaining: Some(remaining),
                unit: unit.into(),
                extra: reset.map(|reset| format!("重置时间: {reset}")),
            });
        }
    }
    if plans.is_empty() {
        let (total, used, remaining) = if let Some(quota) = response["quota"].as_object() {
            let total = quota.get("limit").and_then(Value::as_f64).unwrap_or(0.0);
            let used = quota.get("used").and_then(Value::as_f64).unwrap_or(0.0);
            let remaining = quota
                .get("remaining")
                .and_then(Value::as_f64)
                .unwrap_or_else(|| (total - used).max(0.0));
            (
                (total > 0.0).then_some(total),
                (total > 0.0).then_some(used),
                remaining,
            )
        } else if let Some(remaining) = response["balance"]
            .as_f64()
            .or_else(|| response["remaining"].as_f64())
        {
            (None, None, remaining)
        } else {
            return Err("余额接口未返回可识别的额度或钱包余额".into());
        };
        plans.push(BalancePlan {
            plan_name: text(response, "planName", "钱包余额").into(),
            is_valid,
            total,
            used,
            remaining: Some(remaining),
            unit: unit.into(),
            extra: None,
        });
    }
    Ok(plans)
}

fn query(base_url: &str, api_key: &str) -> Result<Vec<BalancePlan>> {
    let endpoint = endpoint(base_url)?;
    let key = api_key.trim();
    if key.is_empty() {
        return Err("请先填写 API Key".into());
    }
    if key.len() > 4096 || key.chars().any(char::is_control) {
        return Err("API Key 格式无效".into());
    }
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .redirect(Policy::none())
        .user_agent(format!("CYAPI-Toolbox/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "无法初始化余额查询")?;
    let response = client
        .get(endpoint)
        .header(AUTHORIZATION, format!("Bearer {key}"))
        .header(ACCEPT, "application/json")
        .send()
        .map_err(|error| {
            if error.is_timeout() {
                "余额查询超时，请稍后刷新"
            } else {
                "无法连接余额接口，请检查线路和网络"
            }
        })?;
    let status = response.status();
    if !status.is_success() {
        let hint = match status.as_u16() {
            401 => "API Key 无效或已过期",
            403 => "当前 API Key 无权查询余额",
            404 => "当前线路没有 /usage 余额接口",
            429 => "请求过于频繁，请稍后刷新",
            500..=599 => "CYAPI 服务暂时不可用",
            _ => "余额接口请求失败",
        };
        return Err(format!("{hint}（HTTP {}）", status.as_u16()));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取余额响应失败，请稍后刷新")?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("余额接口响应过大".into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "余额接口返回的不是有效 JSON")?;
    extract(&value)
}

#[tauri::command]
pub async fn cyapi_balance(base_url: String, api_key: String) -> Result<Vec<BalancePlan>> {
    // Account requests run independently of the configuration/installation service lock.
    tauri::async_runtime::spawn_blocking(move || query(&base_url, &api_key))
        .await
        .map_err(|_| "余额查询未完成，请稍后刷新".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{io::Write, net::TcpListener, thread};

    #[test]
    fn appends_usage_to_the_selected_api_base() {
        for (base, expected) in [
            (
                " https://www.cyapi.cyou/v1/ ",
                "https://www.cyapi.cyou/v1/usage",
            ),
            ("https://example.test", "https://example.test/usage"),
            (
                "https://example.test/api/v1",
                "https://example.test/api/v1/usage",
            ),
        ] {
            assert_eq!(endpoint(base).unwrap().as_str(), expected);
        }
        for base in [
            "",
            "ftp://example.test",
            "https://user:pass@example.test",
            "https://example.test?key=secret",
            "https://example.test/#fragment",
        ] {
            assert!(endpoint(base).is_err());
        }
    }

    #[test]
    fn prefers_all_period_limits_and_preserves_zero_and_reset_time() {
        let plans = extract(&json!({
            "balance": 999,
            "rate_limits": [
                {"window": "5h", "limit": 20, "used": 5, "reset_at": "2026-10-20T12:00:00Z"},
                {"window": "7d", "limit": 100, "used": 30, "remaining": 0},
                {"limit": 10, "used": 12}
            ]
        }))
        .unwrap();
        assert_eq!(plans.len(), 3);
        assert_eq!(plans[0].plan_name, "5h");
        assert_eq!(plans[0].remaining, Some(15.0));
        assert_eq!(
            plans[0].extra.as_deref(),
            Some("重置时间: 2026-10-20T12:00:00Z")
        );
        assert_eq!(plans[1].remaining, Some(0.0));
        assert_eq!(plans[2].remaining, Some(0.0));
        assert_eq!(plans[2].plan_name, "额度");
        assert_eq!(plans[0].unit, "USD");
    }

    #[test]
    fn falls_back_to_quota_before_wallet() {
        let plans = extract(&json!({
            "rate_limits": [], "balance": 999, "planName": "套餐",
            "quota": {"limit": 100, "used": 25}, "unit": "CNY"
        }))
        .unwrap();
        assert_eq!(plans[0].plan_name, "套餐");
        assert_eq!(plans[0].total, Some(100.0));
        assert_eq!(plans[0].used, Some(25.0));
        assert_eq!(plans[0].remaining, Some(75.0));
        assert_eq!(plans[0].unit, "CNY");
        let explicit =
            extract(&json!({"quota": {"limit": 100, "used": 25, "remaining": 0}})).unwrap();
        assert_eq!(explicit[0].remaining, Some(0.0));
    }

    #[test]
    fn wallet_has_no_invented_total_and_supports_zero_and_debt() {
        for (response, expected) in [
            (json!({"balance": 0, "remaining": 10}), 0.0),
            (json!({"remaining": 12.3456}), 12.3456),
            (json!({"balance": -1}), -1.0),
        ] {
            let plan = extract(&response).unwrap().remove(0);
            assert_eq!(plan.remaining, Some(expected));
            assert_eq!(plan.total, None);
            assert_eq!(plan.used, None);
            assert_eq!(plan.plan_name, "钱包余额");
        }
    }

    #[test]
    fn expired_and_disabled_status_apply_to_every_balance_shape() {
        for mut response in [
            json!({"rate_limits": [{"limit": 10}]}),
            json!({"quota": {"limit": 10}}),
            json!({"balance": 10}),
        ] {
            assert!(extract(&response).unwrap()[0].is_valid);
            for status in ["expired", "disabled"] {
                response["status"] = json!(status);
                assert!(!extract(&response).unwrap()[0].is_valid);
            }
            response["status"] = json!("active");
            response["isValid"] = json!(false);
            assert!(!extract(&response).unwrap()[0].is_valid);
        }
    }

    #[test]
    fn rejects_unrecognized_responses_instead_of_displaying_a_fake_balance() {
        for response in [
            Value::Null,
            json!([]),
            json!({}),
            json!({"error": "invalid key"}),
            json!({"balance": "100"}),
        ] {
            assert!(extract(&response).is_err());
        }
        let plans = extract(&json!({"rate_limits": [null, false], "balance": 12})).unwrap();
        assert_eq!(plans[0].remaining, Some(12.0));
    }

    fn mock_response(status: &str, body: &str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 1024];
            while !bytes.windows(4).any(|chunk| chunk == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
            }
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8(bytes).unwrap()
        });
        (url, worker)
    }

    #[test]
    fn requests_usage_with_bearer_auth_and_json_accept() {
        let (url, worker) = mock_response("200 OK", r#"{"balance":12.5}"#);
        let plans = query(&url, " fixture-balance-key ").unwrap();
        let request = worker.join().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /v1/usage http/1.1\r\n"));
        assert!(request.contains("authorization: bearer fixture-balance-key\r\n"));
        assert!(request.contains("accept: application/json\r\n"));
        assert_eq!(plans[0].remaining, Some(12.5));
    }

    #[test]
    fn http_errors_and_invalid_json_do_not_echo_credentials() {
        for (status, body, expected) in [
            (
                "401 Unauthorized",
                r#"{"error":"fixture-secret-key"}"#,
                "HTTP 401",
            ),
            ("404 Not Found", "not found", "/usage"),
            ("200 OK", "<html>fixture-secret-key</html>", "JSON"),
            ("302 Found", "", "HTTP 302"),
        ] {
            let (url, worker) = mock_response(status, body);
            let error = query(&url, "fixture-secret-key").unwrap_err();
            worker.join().unwrap();
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("fixture-secret-key"));
        }
    }
}
