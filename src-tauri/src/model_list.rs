//! Fetch the model list from an OpenAI-compatible or Anthropic-compatible API.
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT, AUTHORIZATION},
    redirect::Policy,
    StatusCode, Url,
};
use serde::Serialize;
use serde_json::Value;
use std::{collections::HashSet, io::Read, time::Duration};

const MAX_BYTES: u64 = 2_000_000;
const MAX_MODELS: usize = 5_000;
const MAX_MODEL_LEN: usize = 512;

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Api {
    OpenAi,
    Anthropic,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelList {
    models: Vec<String>,
    endpoint: String,
}

fn api(value: &str) -> Api {
    if value.eq_ignore_ascii_case("anthropic") || value.eq_ignore_ascii_case("claude") {
        Api::Anthropic
    } else {
        Api::OpenAi
    }
}

fn endpoint(base_url: &str, api: Api) -> Result<Url> {
    let text = base_url.trim();
    if text.is_empty() {
        return Err("请先填写 API 地址".into());
    }
    if text.len() > 2048
        || text
            .chars()
            .any(|value| value.is_control() || value.is_whitespace())
    {
        return Err("API 地址格式无效".into());
    }
    let mut url = Url::parse(text).map_err(|_| "API 地址无效")?;
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("API 地址无效，请填写 http(s) 服务地址".into());
    }
    let path = url.path().trim_end_matches('/');
    let path = if path.ends_with("/models") {
        path.to_string()
    } else {
        match api {
            Api::OpenAi if path.is_empty() => "/v1/models".into(),
            Api::OpenAi => format!("{path}/models"),
            Api::Anthropic => {
                let base = path.strip_suffix("/v1").unwrap_or(path);
                if base.is_empty() {
                    "/v1/models".into()
                } else {
                    format!("{base}/v1/models")
                }
            }
        }
    };
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn client() -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .redirect(Policy::limited(3))
        .user_agent(format!("CYAPI-Toolbox/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "无法初始化模型列表请求".into())
}

fn request(client: &Client, url: &Url, api: Api, key: &str, bearer: bool) -> Result<Response> {
    let mut request = client.get(url.clone()).header(ACCEPT, "application/json");
    if api == Api::Anthropic {
        request = request.header("anthropic-version", "2023-06-01");
    }
    request = if bearer {
        request.header(AUTHORIZATION, format!("Bearer {key}"))
    } else {
        request.header("x-api-key", key)
    };
    request.send().map_err(|error| {
        if error.is_timeout() {
            "获取模型列表超时，请检查 API 地址或代理".into()
        } else if error.is_connect() {
            "无法连接 API 服务，请检查地址和网络".into()
        } else {
            format!("获取模型列表失败：{error}")
        }
    })
}

fn body(response: Response) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    response
        .take(MAX_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|_| "读取模型列表响应失败".to_string())?;
    if body.len() as u64 > MAX_BYTES {
        return Err("模型列表响应过大".into());
    }
    Ok(body)
}

fn message(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    for path in ["/error/message", "/message", "/error"] {
        if let Some(text) = value.pointer(path).and_then(Value::as_str) {
            let text = text.trim();
            if !text.is_empty() {
                return Some(
                    text.chars()
                        .filter(|value| !value.is_control())
                        .take(300)
                        .collect(),
                );
            }
        }
    }
    None
}

fn status_error(status: StatusCode, body: &[u8], endpoint: &Url) -> String {
    let hint = match status.as_u16() {
        401 => "API Key 无效或没有权限",
        403 => "当前 API Key 无权访问模型列表",
        404 => "API 地址可能不正确，或服务没有模型列表接口",
        429 => "请求过于频繁，请稍后重试",
        value if (500..600).contains(&value) => "API 服务暂时不可用",
        _ => "请求失败",
    };
    match message(body) {
        Some(detail) => format!(
            "获取模型列表失败：HTTP {}（{hint}）：{detail}",
            status.as_u16()
        ),
        None => format!(
            "获取模型列表失败：HTTP {}（{hint}；请求地址 {}）",
            status.as_u16(),
            endpoint
        ),
    }
}

fn push_model(models: &mut Vec<String>, seen: &mut HashSet<String>, value: &str) {
    if models.len() >= MAX_MODELS {
        return;
    }
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_MODEL_LEN
        || value.chars().any(char::is_control)
        || !seen.insert(value.to_string())
    {
        return;
    }
    models.push(value.to_string());
}

fn extract(value: &Value, models: &mut Vec<String>, seen: &mut HashSet<String>, depth: usize) {
    if depth > 4 || models.len() >= MAX_MODELS {
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(id) => push_model(models, seen, id),
                    Value::Object(object) => {
                        for key in ["id", "name", "model", "slug"] {
                            if let Some(id) = object.get(key).and_then(Value::as_str) {
                                push_model(models, seen, id);
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Value::Object(object) => {
            for key in ["data", "models", "model_list", "result"] {
                if let Some(value) = object.get(key) {
                    extract(value, models, seen, depth + 1);
                }
            }
        }
        _ => {}
    }
}

pub fn list(base_url: &str, api_key: &str, api_name: &str) -> Result<ModelList> {
    let api = api(api_name);
    let endpoint = endpoint(base_url, api)?;
    let key = api_key.trim();
    if key.is_empty() {
        return Err("请先填写 API Key".into());
    }
    if key.len() > 4096 || key.chars().any(char::is_control) {
        return Err("API Key 格式无效".into());
    }
    let client = client()?;
    let mut response = request(&client, &endpoint, api, key, true)?;

    // Claude Code may be configured with either an API key or an OAuth-style token.
    if api == Api::Anthropic
        && matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        )
    {
        response = request(&client, &endpoint, api, key, false)?;
    }

    let status = response.status();
    let bytes = body(response)?;
    if !status.is_success() {
        return Err(status_error(status, &bytes, &endpoint));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "模型列表返回的不是有效 JSON，请确认 API 类型和地址".to_string())?;
    let mut models = Vec::new();
    let mut seen = HashSet::new();
    extract(&value, &mut models, &mut seen, 0);
    if models.is_empty() {
        return Err("服务返回了空模型列表，请确认 API 地址、密钥和模型权限".into());
    }
    Ok(ModelList {
        models,
        endpoint: endpoint.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_openai_and_anthropic_endpoints() {
        assert_eq!(
            endpoint("https://www.cyapi.cyou/v1", Api::OpenAi)
                .unwrap()
                .as_str(),
            "https://www.cyapi.cyou/v1/models"
        );
        assert_eq!(
            endpoint("https://api.openai.com", Api::OpenAi)
                .unwrap()
                .as_str(),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            endpoint("https://api.cyapi.cyou/v1", Api::Anthropic)
                .unwrap()
                .as_str(),
            "https://api.cyapi.cyou/v1/models"
        );
        assert_eq!(
            endpoint("https://api.cyapi.cyou", Api::Anthropic)
                .unwrap()
                .as_str(),
            "https://api.cyapi.cyou/v1/models"
        );
    }

    #[test]
    fn rejects_unsafe_urls() {
        assert!(endpoint("", Api::OpenAi).is_err());
        assert!(endpoint("ftp://example.com", Api::OpenAi).is_err());
        assert!(endpoint("https://user:pass@example.com", Api::OpenAi).is_err());
        assert!(endpoint("https://example.com/v1?x=1", Api::OpenAi).is_err());
    }

    #[test]
    fn extracts_common_model_shapes() {
        for value in [
            json!({"data": [{"id": "gpt-a"}, {"id": "gpt-b"}]}),
            json!({"models": [{"name": "gpt-a"}, "gpt-b"]}),
            json!({"data": {"models": [{"id": "gpt-a"}, {"id": "gpt-b"}]}}),
        ] {
            let mut models = Vec::new();
            let mut seen = HashSet::new();
            extract(&value, &mut models, &mut seen, 0);
            assert_eq!(models, vec!["gpt-a", "gpt-b"]);
        }
    }

    #[test]
    fn deduplicates_models() {
        let value = json!({"data": [{"id": "gpt-a"}, {"id": "gpt-a"}, {"id": "gpt-b"}]});
        let mut models = Vec::new();
        let mut seen = HashSet::new();
        extract(&value, &mut models, &mut seen, 0);
        assert_eq!(models, vec!["gpt-a", "gpt-b"]);
    }
}
