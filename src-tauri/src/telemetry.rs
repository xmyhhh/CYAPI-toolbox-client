//! Pseudonymous public catalog operation receipts. Only typed public metadata is queued;
//! no paths, commands, API keys, free-form errors or model requests are transmitted.
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static WORKER_STARTED: AtomicBool = AtomicBool::new(false);
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
pub fn enabled(data: &Path) -> bool {
    std::env::var("CYAPI_TELEMETRY_DISABLED").ok().as_deref() != Some("1")
        && !data.join("telemetry-disabled").exists()
}
pub fn set_enabled(data: &Path, value: bool) -> Result<(), String> {
    if value {
        if data.join("telemetry-disabled").exists() {
            fs::remove_file(data.join("telemetry-disabled")).map_err(|e| e.to_string())?;
        }
    } else {
        fs::write(data.join("telemetry-disabled"), b"disabled").map_err(|e| e.to_string())?;
        if let Ok(conn) = connection(data) {
            let _ = conn.execute("DELETE FROM events", []);
        }
    }
    Ok(())
}
fn connection(data: &Path) -> Result<Connection, String> {
    fs::create_dir_all(data).map_err(|e| e.to_string())?;
    let conn = Connection::open(data.join("telemetry.sqlite")).map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_secs(2))
        .map_err(|e| e.to_string())?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS events (id TEXT PRIMARY KEY, payload TEXT NOT NULL, created INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS history (item TEXT NOT NULL, target TEXT NOT NULL, PRIMARY KEY(item,target));").map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR IGNORE INTO meta(key,value) VALUES ('device',?1)",
        [uuid::Uuid::new_v4().to_string()],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn)
}
#[derive(Clone)]
pub struct Operation {
    pub operation_id: String,
    pub item_id: String,
    pub action: String,
    pub version: String,
    pub target: String,
    pub cache_hit: bool,
}
fn error_code(error: Option<&str>) -> &'static str {
    let Some(error) = error else {
        return "none";
    };
    let lower = error.to_lowercase();
    if error.contains("取消") {
        "cancelled"
    } else if error.contains("校验") || lower.contains("checksum") || lower.contains("sha256") {
        "checksum"
    } else if error.contains("冲突")
        || error.contains("已存在")
        || error.contains("已安装")
        || error.contains("已变化")
    {
        "conflict"
    } else if error.contains("权限")
        || lower.contains("permission")
        || lower.contains("access denied")
    {
        "permission"
    } else if error.contains("缺少") || error.contains("请先下载") {
        "missing_dependency"
    } else if error.contains("下载")
        || error.contains("网络")
        || lower.contains("timeout")
        || lower.contains("connect")
    {
        "network"
    } else {
        "other"
    }
}
pub fn record(
    data: &Path,
    mut operation: Operation,
    elapsed: u64,
    error: Option<&str>,
) -> Result<(), String> {
    if !enabled(data) {
        return Ok(());
    }
    let mut conn = connection(data)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let device: String = tx
        .query_row("SELECT value FROM meta WHERE key='device'", [], |row| {
            row.get(0)
        })
        .map_err(|e| e.to_string())?;
    if operation.action == "skill.install" {
        let seen: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM history WHERE item=?1 AND target=?2)",
                params![operation.item_id, operation.target],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if seen {
            operation.action = "skill.reinstall".into();
        }
        if error.is_none() {
            tx.execute(
                "INSERT OR IGNORE INTO history(item,target) VALUES (?1,?2)",
                params![operation.item_id, operation.target],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let event = json!({ "eventId": id, "operationId": operation.operation_id, "deviceId": device,
        "itemId": operation.item_id, "action": operation.action, "version": operation.version,
        "platform": crate::platform::current(), "target": operation.target, "at": now(), "durationMs": elapsed,
        "result": if error.is_none() { "success" } else { "failure" }, "errorCode": error_code(error), "cacheHit": operation.cache_hit });
    tx.execute(
        "INSERT INTO events(id,payload,created) VALUES (?1,?2,?3)",
        params![id, event.to_string(), now()],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM events WHERE created < ?1",
        [now().saturating_sub(30 * 86400_000)],
    )
    .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM events WHERE id IN (SELECT id FROM events ORDER BY created DESC LIMIT -1 OFFSET 50000)", []).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}
fn flush(data: &Path, server: &str) -> Result<(), String> {
    if !enabled(data) {
        return Ok(());
    }
    let conn = connection(data)?;
    conn.execute(
        "DELETE FROM events WHERE created < ?1",
        [now().saturating_sub(30 * 86400_000)],
    )
    .map_err(|e| e.to_string())?;
    let events: Vec<Value> = conn
        .prepare("SELECT payload FROM events ORDER BY created LIMIT 32")
        .map_err(|e| e.to_string())?
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .filter_map(|row| row.ok().and_then(|text| serde_json::from_str(&text).ok()))
        .collect();
    if events.is_empty() {
        return Ok(());
    }
    let url = crate::service::base_url_from(server)?
        .join("api/client-events")
        .map_err(|e| e.to_string())?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    if !enabled(data) {
        return Ok(());
    }
    let response = client
        .post(url)
        .header("Content-Type", "application/json")
        .body(json!({ "events": events }).to_string())
        .send()
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err("receipt upload deferred".into());
    }
    let mut body = Vec::new();
    response
        .take(65537)
        .read_to_end(&mut body)
        .map_err(|e| e.to_string())?;
    if body.len() > 65536 {
        return Err("receipt acknowledgement too large".into());
    }
    let result: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    let acknowledged = result["data"]["acknowledged"]
        .as_array()
        .ok_or("missing receipt acknowledgement")?;
    for id in acknowledged.iter().filter_map(Value::as_str) {
        if events
            .iter()
            .any(|event| event["eventId"].as_str() == Some(id))
        {
            conn.execute("DELETE FROM events WHERE id=?1", [id])
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
pub fn start(data: PathBuf, server: String) {
    if cfg!(test) || WORKER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let mut failures = 0u32;
        loop {
            if flush(&data, &server).is_ok() {
                failures = 0;
            } else {
                failures = (failures + 1).min(4);
            }
            std::thread::sleep(Duration::from_secs((15 * 2u64.pow(failures)).min(240)));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_receipts_have_no_paths_and_reinstalls_keep_their_history() {
        let data = std::env::temp_dir().join(format!("cyapi-receipts-{}", uuid::Uuid::new_v4()));
        let operation = Operation {
            operation_id: uuid::Uuid::new_v4().to_string(),
            item_id: "my-skill".into(),
            action: "skill.install".into(),
            version: "1".into(),
            target: "codex".into(),
            cache_hit: true,
        };
        record(&data, operation.clone(), 100, None).unwrap();
        record(
            &data,
            operation,
            200,
            Some("/private/path secret-token permission denied"),
        )
        .unwrap();
        let conn = connection(&data).unwrap();
        let payloads: Vec<String> = conn
            .prepare("SELECT payload FROM events ORDER BY created")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(payloads.len(), 2);
        assert!(payloads[1].contains("skill.reinstall"));
        assert!(!payloads.join("").contains("private/path"));
        assert!(!payloads.join("").contains("secret-token"));
        set_enabled(&data, false).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM events", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(conn);
        let _ = fs::remove_dir_all(data);
    }
    #[test]
    fn outbox_retries_and_only_deletes_acknowledged_events() {
        use std::{io::Write, net::TcpListener};
        let data = std::env::temp_dir().join(format!("cyapi-retry-{}", uuid::Uuid::new_v4()));
        let operation = Operation {
            operation_id: uuid::Uuid::new_v4().to_string(),
            item_id: "skill-a".into(),
            action: "skill.download".into(),
            version: "1".into(),
            target: "none".into(),
            cache_hit: false,
        };
        record(&data, operation.clone(), 10, None).unwrap();
        record(&data, operation, 20, None).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let receiver = std::thread::spawn(move || {
            let mut batches = vec![];
            for round in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut bytes = vec![];
                let body_start = loop {
                    let mut buf = [0; 4096];
                    let count = stream.read(&mut buf).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buf[..count]);
                    if let Some(index) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..body_start]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse().unwrap())
                    })
                    .unwrap();
                while bytes.len() < body_start + length {
                    let mut buf = [0; 4096];
                    let count = stream.read(&mut buf).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buf[..count]);
                }
                let body: Value =
                    serde_json::from_slice(&bytes[body_start..body_start + length]).unwrap();
                let ids: Vec<Value> = body["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|event| event["eventId"].clone())
                    .collect();
                batches.push(ids.clone());
                let status = if round == 0 {
                    "500 Internal Server Error"
                } else {
                    "202 Accepted"
                };
                let acknowledged: Vec<Value> = if round == 1 {
                    ids.into_iter().take(1).collect()
                } else {
                    ids
                };
                let payload = json!({"data":{"acknowledged":acknowledged}}).to_string();
                write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len()).unwrap();
            }
            batches
        });
        assert!(flush(&data, &url).is_err());
        assert!(flush(&data, &url).is_ok());
        assert!(flush(&data, &url).is_ok());
        let batches = receiver.join().unwrap();
        assert_eq!(batches[0], batches[1]);
        assert_eq!(batches[2].len(), 1);
        assert!(
            flush(&data, &url).is_ok(),
            "empty outbox does not need a server"
        );
        let _ = fs::remove_dir_all(data);
    }
}
