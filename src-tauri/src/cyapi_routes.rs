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
fn store(data: &Path, manifest: &Manifest) -> Result<()> {
    validate(manifest)?;
    let path = data.join("cyapi-routes.json");
    let bytes = serde_json::to_vec(manifest).map_err(|_| "无法保存线路清单")?;
    let temporary = crate::configuration::stage(&path, &bytes)?;
    if fs::rename(&temporary, &path).is_err() {
        let _ = fs::remove_file(temporary);
        return Err("无法更新线路缓存".into());
    }
    Ok(())
}
fn download(data: &Path) -> Result<Manifest> {
    let url = Url::parse(crate::service::SERVER_URL)
        .and_then(|base| base.join("api/cyapi-routes"))
        .map_err(|_| "无法生成线路清单地址")?;
    let bytes =
        crate::service::fetch_server_url(url, MAX_BYTES, 4).map_err(|_| "暂时无法更新线路清单")?;
    let manifest = parse(&bytes)?;
    store(data, &manifest)?;
    Ok(manifest)
}
// This path never contacts the server: the UI can match the saved configuration
// immediately, while a separate refresh replaces the cache atomically.
pub fn read_cache(data: &Path) -> View {
    match cached(data) {
        Ok(manifest) => View {
            manifest,
            source: "cache",
            message: None,
        },
        Err(_) => View {
            manifest: defaults(),
            source: "default",
            message: None,
        },
    }
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
fn models_endpoint(base_url: &str) -> Result<Url> {
    let mut url = Url::parse(base_url).map_err(|_| "线路地址无效")?;
    let path = url.path().trim_end_matches('/');
    let path = if path.is_empty() {
        "/v1/models".into()
    } else if path.ends_with("/models") {
        path.to_string()
    } else {
        format!("{path}/models")
    };
    url.set_path(&path);
    Ok(url)
}

fn measure(route: &Route) -> Measurement {
    // Time fresh unauthenticated GETs until response headers arrive, without
    // downloading the model list. HTTP errors still have measurable latency.
    const ATTEMPTS: usize = 3;
    let mut samples = Vec::new();
    let mut errors = Vec::new();
    let mut auth_required = false;
    for _ in 0..ATTEMPTS {
        // Client setup is outside the timer. Each attempt includes connection
        // setup, follows no redirects, and has no API Key or cookie store.
        let response = client().and_then(|client| {
            let request = client
                .get(models_endpoint(&route.base_url)?)
                .header(reqwest::header::ACCEPT, "application/json")
                .header(reqwest::header::CACHE_CONTROL, "no-cache");
            let started = Instant::now();
            request
                .send()
                .map(|response| {
                    (
                        response.status(),
                        started.elapsed().as_millis().max(1) as u64,
                    )
                })
                .map_err(|error| {
                    if error.is_timeout() {
                        "连接超时"
                    } else {
                        "连接失败"
                    }
                    .into()
                })
        });
        match response {
            Ok((status, latency)) => {
                samples.push(latency);
                if status.as_u16() == 401 {
                    auth_required = true;
                } else if !status.is_success() {
                    let message = format!("HTTP {}", status.as_u16());
                    if !errors.contains(&message) {
                        errors.push(message);
                    }
                }
            }
            Err(error) => {
                if !errors.contains(&error) {
                    errors.push(error);
                }
            }
        }
    }
    let latency_ms =
        (!samples.is_empty()).then(|| samples.iter().sum::<u64>() / samples.len() as u64);
    Measurement {
        route_id: route.id.clone(),
        reachable: samples.len() == ATTEMPTS && errors.is_empty(),
        latency_ms,
        message: if !errors.is_empty() {
            format!(
                "{}；响应 {}/{ATTEMPTS} 次",
                errors.join(" / "),
                samples.len()
            )
        } else if auth_required {
            "接口可达（HTTP 401，未携带 API Key）；响应 3/3 次".into()
        } else {
            "接口可达；响应 3/3 次".into()
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
    fn local_cache_survives_reloads_and_valid_refreshes_replace_it_entirely() {
        let data = std::env::temp_dir().join(format!("cyapi-routes-{}", uuid::Uuid::new_v4()));
        assert_eq!(read_cache(&data).source, "default");
        assert!(!data.exists());
        let mut manifest = defaults();
        manifest.routes.push(Route {
            id: "old-backup".into(),
            name: "Old backup".into(),
            base_url: "https://old.example.invalid/v1".into(),
        });
        store(&data, &manifest).unwrap();
        for _ in 0..2 {
            let view = read_cache(&data);
            assert_eq!(view.source, "cache");
            assert!(view.message.is_none());
            assert_eq!(view.manifest.routes[1].id, "old-backup");
        }
        manifest.routes[1].id = "new-backup".into();
        manifest.routes[1].base_url = "https://new.example.invalid/v1".into();
        store(&data, &manifest).unwrap();
        let cached_bytes = fs::read(data.join("cyapi-routes.json")).unwrap();
        assert_eq!(read_cache(&data).manifest.routes[1].id, "new-backup");
        assert!(!String::from_utf8_lossy(&cached_bytes).contains("old-backup"));
        manifest.routes[0].base_url = "https://invalid.example/v1".into();
        assert!(store(&data, &manifest).is_err());
        assert_eq!(
            fs::read(data.join("cyapi-routes.json")).unwrap(),
            cached_bytes
        );
        assert_eq!(fs::read_dir(&data).unwrap().count(), 1);
        fs::remove_dir_all(data).unwrap();
    }
    #[test]
    fn corrupt_or_oversized_caches_fall_back_without_network_or_rewriting_files() {
        let data = std::env::temp_dir().join(format!("cyapi-routes-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&data).unwrap();
        for bytes in [b"not json".to_vec(), vec![b' '; MAX_BYTES as usize + 1]] {
            fs::write(data.join("cyapi-routes.json"), &bytes).unwrap();
            let view = read_cache(&data);
            assert_eq!(view.source, "default");
            assert_eq!(view.manifest.routes[0].base_url, OFFICIAL_URL);
            assert_eq!(fs::read(data.join("cyapi-routes.json")).unwrap(), bytes);
        }
        fs::remove_dir_all(data).unwrap();
    }
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
                assert!(
                    !text.contains("authorization:")
                        && !text.contains("cookie:")
                        && !text.contains("x-api-key:")
                );
                assert!(text.contains("accept: application/json"));
                assert!(text.contains("cache-control: no-cache"));
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
        assert!(result.message.contains("401"));
    }
    #[test]
    fn redirects_and_server_errors_are_not_ranked_as_healthy_routes() {
        for status in [
            "302 Found\r\nLocation: http://127.0.0.1:1/",
            "403 Forbidden",
            "404 Not Found",
            "429 Too Many Requests",
            "500 Internal Server Error",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/v1", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                for _ in 0..3 {
                    let (mut socket, _) = listener.accept().unwrap();
                    let mut request = [0u8; 4096];
                    socket.read(&mut request).unwrap();
                    write!(
                        socket,
                        "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                }
            });
            let result = measure(&Route {
                id: "test".into(),
                name: "test".into(),
                base_url: url,
            });
            server.join().unwrap();
            assert!(!result.reachable && result.latency_ms.is_some());
            assert!(result.message.contains(&status[..3]));
        }
    }

    #[test]
    fn model_probe_paths_preserve_api_prefixes_without_duplicate_suffixes() {
        for (base, expected) in [
            ("https://example.test", "/v1/models"),
            ("https://example.test/", "/v1/models"),
            ("https://example.test/v1/", "/v1/models"),
            ("https://example.test/proxy/v1", "/proxy/v1/models"),
            ("https://example.test/v1/models/", "/v1/models"),
        ] {
            assert_eq!(models_endpoint(base).unwrap().path(), expected);
        }
    }

    #[test]
    fn a_good_first_response_does_not_hide_later_http_errors() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for status in [
                "401 Unauthorized",
                "500 Internal Server Error",
                "401 Unauthorized",
            ] {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                socket.read(&mut request).unwrap();
                write!(
                    socket,
                    "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            }
        });
        let result = measure(&Route {
            id: "test".into(),
            name: "test".into(),
            base_url: url,
        });
        server.join().unwrap();
        assert!(result.latency_ms.is_some());
        assert!(!result.reachable);
        assert!(result.message.contains("500"));
        assert!(result.message.contains("3/3"));
    }

    #[test]
    fn latency_does_not_wait_for_the_model_list_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let (done, wait) = std::sync::mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let mut sockets = Vec::new();
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                socket.read(&mut request).unwrap();
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1000000\r\n\r\n").unwrap();
                sockets.push(socket);
            }
            // Hold the responses open without sending any body bytes.
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let result = measure(&Route {
            id: "test".into(),
            name: "test".into(),
            base_url: url,
        });
        done.send(()).unwrap();
        server.join().unwrap();
        assert!(result.reachable && result.latency_ms.is_some());
    }
}
