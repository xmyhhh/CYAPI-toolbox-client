//! Public CYAPI route discovery and unauthenticated connection measurements.
use reqwest::{blocking::Client, redirect::Policy, Url};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;
pub const OFFICIAL_URL: &str = "https://www.cyapi.cyou/v1";
const MAX_BYTES: u64 = 65_536;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub id: String,
    pub name: String,
    pub base_url: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema: u32,
    pub default_route_id: String,
    pub routes: Vec<Route>,
}
#[derive(Serialize)]
pub struct View {
    manifest: Manifest,
    source: &'static str,
    message: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Measurement {
    route_id: String,
    latency_ms: Option<u64>,
    reachable: bool,
    message: String,
}

fn defaults() -> Manifest {
    Manifest {
        schema: 1,
        default_route_id: "official".into(),
        routes: vec![Route {
            id: "official".into(),
            name: "CYAPI 官方线路".into(),
            base_url: OFFICIAL_URL.into(),
        }],
    }
}
fn validate(manifest: &Manifest) -> Result<()> {
    if manifest.schema != 1
        || manifest.default_route_id != "official"
        || manifest.routes.is_empty()
        || manifest.routes.len() > 64
    {
        return Err("CYAPI 线路清单格式无效".into());
    }
    let mut ids = HashSet::new();
    let mut urls = HashSet::new();
    for route in &manifest.routes {
        let url = Url::parse(&route.base_url).map_err(|_| "CYAPI 线路地址无效")?;
        if route.id.is_empty()
            || route.id.len() > 64
            || !route
                .id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            || !ids.insert(&route.id)
            || route.name.trim().is_empty()
            || route.name.chars().count() > 80
            || route.name.chars().any(char::is_control)
            || route.base_url.len() > 2048
            || route.base_url.chars().any(char::is_whitespace)
            || route.base_url.chars().any(char::is_control)
            || !["http", "https"].contains(&url.scheme())
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !urls.insert(url.as_str().trim_end_matches('/').to_string())
        {
            return Err("CYAPI 线路名称或地址无效".into());
        }
    }
    if !manifest
        .routes
        .iter()
        .any(|r| r.id == "official" && r.base_url == OFFICIAL_URL)
    {
        return Err("CYAPI 默认线路无效".into());
    }
    Ok(())
}
fn parse(bytes: &[u8]) -> Result<Manifest> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err("CYAPI 线路清单过大".into());
    }
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|_| "CYAPI 线路清单无效")?;
    validate(&manifest)?;
    Ok(manifest)
}
fn client() -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(4))
        .redirect(Policy::none())
        .build()
        .map_err(|_| "无法初始化线路连接".into())
}
fn cached(data: &Path) -> Result<Manifest> {
    let path = data.join("cyapi-routes.json");
    crate::configuration::no_links(&path)?;
    let file = fs::File::open(path).map_err(|_| "尚无线路缓存")?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取线路缓存")?;
    parse(&bytes)
}
fn download(data: &Path) -> Result<Manifest> {
    let url = Url::parse(crate::service::SERVER_URL)
        .and_then(|base| base.join("api/cyapi-routes"))
        .map_err(|_| "无法生成线路清单地址")?;
    let bytes =
        crate::service::fetch_server_url(url, MAX_BYTES, 4).map_err(|_| "暂时无法更新线路清单")?;
    let manifest = parse(&bytes)?;
    let path = data.join("cyapi-routes.json");
    crate::configuration::no_links(&path)?;
    fs::create_dir_all(data).map_err(|_| "无法创建线路缓存目录")?;
    let temporary = data.join(format!("cyapi-routes-{}.tmp", uuid::Uuid::new_v4()));
    fs::write(
        &temporary,
        serde_json::to_vec(&manifest).map_err(|_| "无法保存线路清单")?,
    )
    .map_err(|_| "无法保存线路清单")?;
    if fs::rename(&temporary, &path).is_err() {
        let _ = fs::remove_file(temporary);
        return Err("无法更新线路缓存".into());
    }
    Ok(manifest)
}
pub fn read(data: &Path) -> View {
    match download(data) {
        Ok(manifest) => View {
            manifest,
            source: "server",
            message: None,
        },
        Err(_) => match cached(data) {
            Ok(manifest) => View {
                manifest,
                source: "cache",
                message: Some("暂时无法更新，使用上次保存的线路清单。".into()),
            },
            Err(_) => View {
                manifest: defaults(),
                source: "default",
                message: Some("暂时无法获取备用线路，可使用官方默认线路或稍后更新。".into()),
            },
        },
    }
}
fn measure(route: &Route) -> Measurement {
    // Three attempts per route; the reported latency is the mean of the ones
    // that answered. Failures are not surfaced as a packet-loss number.
    const ATTEMPTS: usize = 3;
    let mut samples = Vec::new();
    let mut message = "连接超时或暂不可用".to_string();
    for _ in 0..ATTEMPTS {
        // Never accept credentials or use the user's key. Do not follow cross-origin redirects.
        let started = Instant::now();
        let response = client().and_then(|client| {
            client
                .get(format!("{}/models", route.base_url.trim_end_matches('/')))
                .send()
                .map_err(|_| "连接失败".into())
        });
        match response {
            Ok(response) if response.status().is_success() || response.status().as_u16() == 401 => {
                samples.push(started.elapsed().as_millis().max(1) as u64);
            }
            Ok(response) => {
                message = format!("HTTP {}", response.status().as_u16());
                break;
            }
            Err(error) => {
                message = error;
            }
        }
    }
    let latency_ms =
        (!samples.is_empty()).then(|| samples.iter().sum::<u64>() / samples.len() as u64);
    Measurement {
        route_id: route.id.clone(),
        reachable: latency_ms.is_some(),
        latency_ms,
        message: if latency_ms.is_some() {
            "可连接".into()
        } else {
            message
        },
    }
}
pub fn probe(data: &Path, route_id: &str) -> Result<Measurement> {
    let manifest = cached(data).unwrap_or_else(|_| defaults());
    let route = manifest
        .routes
        .iter()
        .find(|r| r.id == route_id)
        .ok_or("线路清单已变化，请更新后重新测速")?;
    Ok(measure(route))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener};
    #[test]
    fn rejects_changed_default_duplicate_urls_and_embedded_credentials() {
        let mut manifest = defaults();
        manifest.routes.push(Route {
            id: "backup".into(),
            name: "test".into(),
            base_url: "https://user:secret@example.test/v1".into(),
        });
        assert!(validate(&manifest).is_err());
        manifest.routes[1].base_url = format!("{OFFICIAL_URL}/");
        assert!(validate(&manifest).is_err());
        manifest.routes.pop();
        manifest.routes[0].base_url = "http://example.test/v1".into();
        assert!(validate(&manifest).is_err());
        assert!(parse(&vec![b' '; MAX_BYTES as usize + 1]).is_err());
    }
    #[test]
    fn measurements_recognize_api_auth_challenge_and_never_send_keys() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0u8; 4096];
                let size = socket.read(&mut request).unwrap();
                let text = String::from_utf8_lossy(&request[..size]).to_lowercase();
                assert!(text.starts_with("get /v1/models http/1.1"));
                assert!(!text.contains("authorization:") && !text.contains("cookie:"));
                socket.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            }
        });
        let result = measure(&Route {
            id: "test".into(),
            name: "test".into(),
            base_url: url,
        });
        server.join().unwrap();
        assert!(result.reachable && result.latency_ms.is_some());
    }
    #[test]
    fn redirects_and_server_errors_are_not_ranked_as_healthy_routes() {
        for status in [
            "302 Found\r\nLocation: http://127.0.0.1:1/",
            "500 Internal Server Error",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/v1", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                socket.read(&mut request).unwrap();
                write!(
                    socket,
                    "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            });
            let result = measure(&Route {
                id: "test".into(),
                name: "test".into(),
                base_url: url,
            });
            server.join().unwrap();
            assert!(!result.reachable && result.latency_ms.is_none());
        }
    }
}
