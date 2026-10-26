//! Read-only addon liveness checks. Never launch MCP commands or touch a scene.
use serde::Serialize;
use serde_json::{json, Value};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// Windows can take ~2 seconds to report a refused localhost connection.
const TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RESPONSE: usize = 4096;

#[derive(Debug, PartialEq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

impl Endpoint {
    /// Match upstream's CLI > workspace environment > inherited environment >
    /// defaults precedence, including its fallback for invalid BLENDER_PORT.
    pub fn from_command(
        command: &str,
        args: &[&str],
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let executable = command.rsplit(['/', '\\']).next().unwrap_or(command);
        let executable = executable.strip_suffix(".exe").unwrap_or(executable);
        let server_args = if executable == "uvx" && args.first() == Some(&"mcp-for-blender") {
            &args[1..]
        } else if executable.starts_with("python")
            && args
                .windows(2)
                .any(|pair| pair == ["-m", "blender_mcp.server"])
        {
            let start = args
                .iter()
                .position(|arg| *arg == "blender_mcp.server")
                .unwrap();
            &args[start + 1..]
        } else if executable == "mcp-for-blender" {
            args
        } else {
            return Err(
                "当前 Blender MCP 使用自定义启动命令，无法自动确定服务地址；请在 MCP 管理中检查"
                    .into(),
            );
        };
        let mut host = None;
        let mut port = None;
        let mut args = server_args.iter().copied();
        while let Some(arg) = args.next() {
            if arg == "--" {
                break;
            }
            if arg == "--host" {
                host = Some(args.next().ok_or("--host 缺少地址")?.to_string());
            } else if let Some(value) = arg.strip_prefix("--host=") {
                host = Some(value.to_string());
            } else if arg == "--port" {
                port = Some(args.next().ok_or("--port 缺少端口")?.to_string());
            } else if let Some(value) = arg.strip_prefix("--port=") {
                port = Some(value.to_string());
            } else if arg.starts_with("--h") || arg.starts_with("--p") {
                // argparse accepts abbreviations; do not silently probe defaults
                // when a custom command cannot be resolved unambiguously.
                return Err("请使用完整的 --host / --port 参数后重新检查 Blender 服务".into());
            }
        }
        let host = host
            .filter(|value| !value.is_empty())
            .or_else(|| env("BLENDER_HOST"))
            .unwrap_or_else(|| "localhost".into());
        if host.is_empty()
            || host.len() > 253
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_:".contains(&b))
        {
            return Err("Blender 服务主机地址无效，请检查 MCP 的 --host 或 BLENDER_HOST".into());
        }
        let port = if let Some(port) = port {
            port.parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or("Blender 服务端口无效，请检查 MCP 的 --port")?
        } else {
            match env("BLENDER_PORT").filter(|value| !value.is_empty()) {
                Some(value) => match value.trim().parse::<i64>() {
                    Ok(port) => u16::try_from(port)
                        .ok()
                        .filter(|port| *port != 0)
                        .ok_or("Blender 服务端口无效，请检查 BLENDER_PORT")?,
                    Err(_) => 9876,
                },
                None => 9876,
            }
        };
        Ok(Self { host, port })
    }

    fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: &'static str,
    pub address: Option<String>,
    pub message: String,
    pub checked_at: u64,
}

impl Status {
    pub fn unverified(message: impl Into<String>) -> Self {
        Self {
            state: "unverified",
            address: None,
            message: message.into(),
            checked_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }
}

pub struct Probe {
    pub id: String,
    pub revision: Option<String>,
    pub endpoint: Result<Endpoint, String>,
}

pub async fn read(workspace: &str, probes: Vec<Probe>) -> Value {
    let mut entries = Vec::new();
    for probe in probes {
        let status = match probe.endpoint {
            Ok(endpoint) => check(&endpoint, TIMEOUT).await,
            Err(message) => Status::unverified(message),
        };
        entries.push(json!({"id":probe.id,"mcpRevision":probe.revision,"service":status}));
    }
    json!({"workspace":workspace,"entries":entries})
}

async fn check(endpoint: &Endpoint, timeout: Duration) -> Status {
    let address = endpoint.address();
    let result = tokio::time::timeout(timeout, ping(endpoint)).await;
    let (state, message) = match result {
        Ok(Ok(())) => ("connected", format!("服务已连接（{address}），Blender 已响应只读 ping；AI 首次任务仍待验证")),
        Ok(Err((state, message))) => (state, format!("{message}（{address}）")),
        Err(_) => ("timeout", format!("服务检测超时（{address}）；地址可能不可达，或 Blender 正忙、未响应。请检查地址并稍后重试，不代表服务未启动")),
    };
    Status {
        state,
        address: Some(address),
        message,
        ..Status::unverified("")
    }
}

type ProbeError = (&'static str, &'static str);

async fn ping(endpoint: &Endpoint) -> Result<(), ProbeError> {
    // The upstream MCP client uses AF_INET. Do not accidentally validate an
    // IPv6-only service that the configured MCP client cannot reach.
    let addresses = tokio::net::lookup_host((endpoint.host.as_str(), endpoint.port))
        .await
        .map_err(|_| ("unverified", "无法解析 Blender 服务地址，请检查主机设置"))?;
    let mut stream = None;
    let mut refused = false;
    for address in addresses.filter(|address| address.is_ipv4()) {
        match tokio::net::TcpStream::connect(address).await {
            Ok(connected) => {
                stream = Some(connected);
                break;
            }
            Err(error) => refused |= error.kind() == std::io::ErrorKind::ConnectionRefused,
        }
    }
    let mut stream = stream.ok_or(if refused {
        ("waiting", "等待启动 Blender 服务：在 3D 视图按 N，打开 MCP for Blender，点击 Connect to MCP server；仅勾选插件不会启动服务")
    } else {
        ("unverified", "无法连接 Blender 服务，请检查地址、端口和防火墙")
    })?;
    stream
        .write_all(br#"{"type":"ping","params":{}}"#)
        .await
        .map_err(|_| ("unverified", "无法发送 Blender 服务检测请求，请重试"))?;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 1024];
    loop {
        let count = stream.read(&mut buffer).await.map_err(|_| {
            (
                "invalid-response",
                "Blender 服务连接中断，尚未收到有效 ping 响应",
            )
        })?;
        if count == 0 {
            return Err((
                "invalid-response",
                "服务已关闭连接，尚未收到有效 Blender ping 响应",
            ));
        }
        if bytes.len() + count > MAX_RESPONSE {
            return Err(("invalid-response", "服务响应过大，无法确认是 Blender 服务"));
        }
        bytes.extend_from_slice(&buffer[..count]);
        // Addon replies have no newline or length prefix, and the connection
        // stays open. Decode a complete JSON value, not a single TCP packet.
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) if value["status"] == "success" && value["result"]["pong"] == true => {
                return Ok(())
            }
            Ok(_) => return Err((
                "invalid-response",
                "端口有响应，但未返回 Blender ping 确认；请检查插件版本及端口是否被其他程序占用",
            )),
            Err(error) if error.is_eof() => continue,
            Err(_) => {
                return Err((
                    "invalid-response",
                    "端口返回的内容不是有效 Blender 响应，请检查插件和端口",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        thread,
    };

    fn run<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn server(
        reply: impl FnOnce(TcpStream) + Send + 'static,
    ) -> (Endpoint, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let expected = br#"{"type":"ping","params":{}}"#;
            let mut request = vec![0; expected.len()];
            stream.read_exact(&mut request).unwrap();
            assert_eq!(
                request, expected,
                "the only request must be a read-only ping"
            );
            reply(stream);
        });
        (
            Endpoint {
                host: "127.0.0.1".into(),
                port,
            },
            handle,
        )
    }

    #[test]
    fn cli_precedes_workspace_environment_and_defaults() {
        let environment = |name: &str| {
            Some(
                if name == "BLENDER_HOST" {
                    "192.0.2.1"
                } else {
                    "9988"
                }
                .into(),
            )
        };
        assert_eq!(
            Endpoint::from_command("uvx", &["mcp-for-blender"], |_| None).unwrap(),
            Endpoint {
                host: "localhost".into(),
                port: 9876
            }
        );
        let endpoint = Endpoint::from_command(
            r"C:\runtime\python.exe",
            &[
                "-I",
                "-m",
                "blender_mcp.server",
                "--host=127.0.0.1",
                "--port",
                "8765",
            ],
            environment,
        )
        .unwrap();
        assert_eq!(
            endpoint,
            Endpoint {
                host: "127.0.0.1".into(),
                port: 8765
            }
        );
        let endpoint = Endpoint::from_command("mcp-for-blender", &[], environment).unwrap();
        assert_eq!(endpoint.host, "192.0.2.1");
        assert_eq!(endpoint.port, 9988);
        assert_eq!(
            Endpoint::from_command("uvx", &["mcp-for-blender"], |name| (name == "BLENDER_PORT")
                .then(|| "invalid".into()))
            .unwrap()
            .port,
            9876
        );
        for args in [
            vec!["--port", "0"],
            vec!["--port=65536"],
            vec!["--host"],
            vec!["--host=http://localhost"],
            vec!["--po=9988"],
        ] {
            assert!(
                Endpoint::from_command("mcp-for-blender", &args, |_| None).is_err(),
                "{args:?}"
            );
        }
        assert!(Endpoint::from_command("custom-wrapper", &[], |_| None).is_err());
        assert!(
            Endpoint::from_command("python", &["-c", "print('not blender')"], |_| None).is_err()
        );
    }

    #[test]
    fn fragmented_pong_is_accepted_without_waiting_for_a_newline_or_socket_close() {
        let (endpoint, thread) = server(|mut stream| {
            for chunk in br#"{"status":"success","result":{"pong":true}}"#.chunks(5) {
                stream.write_all(chunk).unwrap();
                thread::sleep(Duration::from_millis(4));
            }
            let mut buffer = [0];
            assert_eq!(stream.read(&mut buffer).unwrap(), 0);
        });
        let status = run(check(&endpoint, TIMEOUT));
        assert_eq!(status.state, "connected");
        assert!(status.message.contains("首次任务仍待验证"));
        assert!(status.checked_at > 0);
        thread.join().unwrap();
    }

    #[test]
    fn an_open_port_non_blender_json_and_incomplete_responses_are_not_connected() {
        for response in [
            br#"{"status":"success","result":{"pong":false}}"#.as_slice(),
            br#"{"status":"error","result":{"pong":true}}"#,
            br#"{"status":"success","result":{"pong":"true"}}"#,
            br#"{"ok":true}"#,
            b"HTTP/1.1 200 OK\r\n",
            b"{\"status\":",
            b"",
        ] {
            let response = response.to_vec();
            let (endpoint, thread) = server(move |mut stream| {
                stream.write_all(&response).unwrap();
            });
            assert_eq!(
                run(check(&endpoint, Duration::from_secs(1))).state,
                "invalid-response"
            );
            thread.join().unwrap();
        }
    }

    #[test]
    fn response_size_and_total_time_are_bounded_even_with_partial_packets() {
        let (endpoint, thread) = server(|mut stream| {
            let _ = stream.write_all(format!("\"{}", "x".repeat(MAX_RESPONSE + 10)).as_bytes());
        });
        let status = run(check(&endpoint, TIMEOUT));
        assert_eq!(status.state, "invalid-response");
        assert!(status.message.contains("过大"));
        thread.join().unwrap();
        let (endpoint, thread) = server(|mut stream| {
            let _ = stream.write_all(b"{");
            for _ in 0..30 {
                thread::sleep(Duration::from_millis(20));
                if stream.write_all(b" ").is_err() {
                    break;
                }
            }
        });
        let started = std::time::Instant::now();
        let status = run(check(&endpoint, Duration::from_millis(120)));
        assert_eq!(status.state, "timeout");
        assert!(status.message.contains("Blender 正忙"));
        assert!(started.elapsed() < Duration::from_millis(550));
        thread.join().unwrap();
    }

    #[test]
    fn connection_refusal_means_waiting_not_connected_and_configuration_errors_stay_unverified() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = Endpoint {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
        };
        drop(listener);
        let status = run(check(&endpoint, TIMEOUT));
        assert_eq!(status.state, "waiting");
        assert!(status.message.contains("仅勾选插件不会启动服务"));
        let view = run(read(
            "codex",
            vec![Probe {
                id: "blender".into(),
                revision: Some("entry-revision".into()),
                endpoint: Err("配置未启用".into()),
            }],
        ));
        assert_eq!(view["entries"][0]["service"]["state"], "unverified");
        assert_eq!(view["entries"][0]["mcpRevision"], "entry-revision");
    }

    #[test]
    #[ignore = "read-only ping of the user's currently running Blender addon; does not read or modify the scene"]
    fn real_local_blender_ping() {
        let endpoint = Endpoint {
            host: "localhost".into(),
            port: 9876,
        };
        let status = run(check(&endpoint, TIMEOUT));
        println!("{}", serde_json::to_string(&status).unwrap());
        assert_eq!(status.state, "connected");
    }
}
