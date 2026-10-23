//! Read-only analysis of Codex's local log database.
//!
//! Codex writes structured tracing records into `$CODEX_HOME/logs_<n>.sqlite`.
//! That database is the authoritative local record of what the client actually
//! did: which URL it called, which status came back, which retries happened and
//! why a turn failed. This module never writes to it, opens it read-only, and
//! redacts every string before it leaves the process.

use crate::logging;
use rusqlite::{params_from_iter, types::Value as SqlValue, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

const MAX_LIMIT: u32 = 2000;
const MAX_DETAIL: usize = 64 * 1024;
const OVERVIEW_LIMIT: usize = 20_000;

#[path = "codex_analysis.rs"]
mod analysis;

// ---------------------------------------------------------------------------
// Public shapes
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub wal_size: u64,
    pub modified_at: u64,
    pub rows: i64,
    pub oldest: u64,
    pub newest: u64,
    pub readable: bool,
    pub error: Option<String>,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUrl {
    pub name: String,
    pub base_url: String,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConfigInfo {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub expected_host: String,
    pub expected_scheme: String,
    pub providers: Vec<ProviderUrl>,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HttpInfo {
    pub method: String,
    pub url: String,
    pub scheme: String,
    pub host: String,
    pub path: String,
    pub status: Option<u16>,
    pub reason: Option<String>,
    pub ok: bool,
    pub timeout: bool,
    pub connect: bool,
    pub request_id: Option<String>,
    pub client_request_id: Option<String>,
    pub server: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: i64,
    pub time: u64,
    pub level: String,
    pub target: String,
    pub kind: String,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub pid: Option<u32>,
    pub model: Option<String>,
    pub summary: String,
    pub http: Option<HttpInfo>,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StatusCount {
    pub status: u16,
    pub count: usize,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RouteStat {
    pub host: String,
    pub scheme: String,
    pub requests: usize,
    pub ok: usize,
    pub failed: usize,
    pub statuses: Vec<StatusCount>,
    pub last_at: u64,
    pub last_status: Option<u16>,
    pub configured: bool,
    pub threads: usize,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStat {
    pub id: String,
    pub title: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub requests: usize,
    pub failures: usize,
    pub last_at: u64,
    pub hosts: Vec<String>,
    pub route_ok: bool,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStat {
    pub pid: u32,
    pub events: usize,
    pub requests: usize,
    pub last_at: u64,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Cluster {
    pub signature: String,
    pub title: String,
    pub count: usize,
    pub level: String,
    pub target: String,
    pub first_at: u64,
    pub last_at: u64,
    pub sample: String,
    pub advice: String,
    pub benign: bool,
    pub ids: Vec<i64>,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub requests: usize,
    pub ok: usize,
    pub failed: usize,
    pub retries: usize,
    pub turn_errors: usize,
    pub warnings: usize,
    pub errors: usize,
    pub tools: usize,
    pub threads: usize,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub status: String,
    pub title: String,
    pub message: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub generated_at: u64,
    pub window: u64,
    pub source: SourceInfo,
    pub state: SourceInfo,
    pub config: ConfigInfo,
    pub totals: Totals,
    pub routes: Vec<RouteStat>,
    pub threads: Vec<ThreadStat>,
    pub processes: Vec<ProcessStat>,
    pub errors: Vec<Cluster>,
    pub recent: Vec<Event>,
    pub verdict: Verdict,
    pub notes: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EventQuery {
    pub kinds: Option<Vec<String>>,
    pub levels: Option<Vec<String>>,
    pub thread_id: Option<String>,
    pub host: Option<String>,
    pub search: Option<String>,
    pub since: Option<u64>,
    pub until: Option<u64>,
    pub after_id: Option<i64>,
    pub before_id: Option<i64>,
    pub limit: Option<u32>,
    pub outcome: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub action: String,
    pub window: Option<u64>,
    pub query: Option<EventQuery>,
    pub id: Option<i64>,
    pub snapshot_id: Option<String>,
    pub event_id: Option<String>,
    pub source_id: Option<String>,
    pub thread_id: Option<String>,
    pub offset: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPage {
    pub events: Vec<Event>,
    pub has_more: bool,
    pub next_before: Option<i64>,
    pub next_after: Option<i64>,
    pub scanned: usize,
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn numeric_suffix(name: &str, prefix: &str) -> Option<u32> {
    let rest = name.strip_prefix(prefix)?.strip_suffix(".sqlite")?;
    rest.parse().ok()
}

fn latest_db(dir: &Path, prefix: &str) -> Option<PathBuf> {
    let mut best: Option<(u32, PathBuf)> = None;
    for entry in fs::read_dir(dir).ok()?.filter_map(Result::ok).take(4000) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(version) = numeric_suffix(&name, prefix) else {
            continue;
        };
        if best.as_ref().is_none_or(|(v, _)| version > *v) {
            best = Some((version, entry.path()));
        }
    }
    best.map(|(_, path)| path)
}

fn sqlite_home() -> PathBuf {
    let root = crate::diagnostics::codex_home();
    let Ok(text) = fs::read_to_string(root.join("config.toml")) else {
        return root;
    };
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return root;
    };
    match doc.get("sqlite_home").and_then(|v| v.as_str()) {
        Some(value) if Path::new(value).is_absolute() => PathBuf::from(value),
        _ => root,
    }
}

fn modified_at(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|v| v.as_millis() as u64)
        .unwrap_or(0)
}

fn source_info(path: Option<&Path>) -> SourceInfo {
    let Some(path) = path else {
        return SourceInfo {
            readable: false,
            error: Some("未找到 Codex 日志数据库（logs_*.sqlite）".into()),
            ..Default::default()
        };
    };
    let name = path
        .file_name()
        .map(|v| v.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut info = SourceInfo {
        path: path.to_string_lossy().into_owned(),
        name,
        size: fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        wal_size: fs::metadata(format!("{}-wal", path.display()))
            .map(|m| m.len())
            .unwrap_or(0),
        modified_at: modified_at(path),
        ..Default::default()
    };
    match open_db(path) {
        Ok(conn) => {
            let bounds = conn
                .query_row(
                    "SELECT COUNT(*), COALESCE(MIN(ts),0), COALESCE(MAX(ts),0) FROM logs",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .map_err(|e| format!("无法读取日志表：{e}"));
            match bounds {
                Ok((rows, oldest, newest)) => {
                    info.readable = true;
                    info.rows = rows;
                    info.oldest = (oldest as u64).saturating_mul(1000);
                    info.newest = (newest as u64).saturating_mul(1000);
                }
                Err(error) => info.error = Some(error),
            }
        }
        Err(error) => info.error = Some(error),
    }
    info
}

/// The state database has no `logs` table; report it without the log checks.
fn state_info(path: Option<&Path>) -> SourceInfo {
    let Some(path) = path else {
        return SourceInfo::default();
    };
    let mut info = SourceInfo {
        path: path.to_string_lossy().into_owned(),
        name: path
            .file_name()
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size: fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        wal_size: fs::metadata(format!("{}-wal", path.display()))
            .map(|m| m.len())
            .unwrap_or(0),
        modified_at: modified_at(path),
        ..Default::default()
    };
    match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(conn) => {
            let _ = conn.busy_timeout(Duration::from_millis(600));
            let _ = conn.execute_batch("PRAGMA query_only=1;");
            match conn.query_row("SELECT COUNT(*) FROM threads", [], |row| {
                row.get::<_, i64>(0)
            }) {
                Ok(rows) => {
                    info.readable = true;
                    info.rows = rows;
                }
                Err(error) => info.error = Some(format!("无法读取线程表：{error}")),
            }
        }
        Err(error) => info.error = Some(format!("无法打开线程数据库：{error}")),
    }
    info
}

fn open_db(path: &Path) -> Result<Connection, String> {
    if !path.is_absolute() {
        return Err("日志数据库路径不是绝对路径".into());
    }
    if !path.is_file() {
        return Err("日志数据库不存在".into());
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("无法以只读方式打开日志数据库：{e}"))?;
    conn.busy_timeout(Duration::from_millis(1500))
        .map_err(|e| format!("无法设置数据库超时：{e}"))?;
    let _ = conn.execute_batch("PRAGMA query_only=1;");
    let has_logs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='logs'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("无法读取数据库结构：{e}"))?;
    if has_logs == 0 {
        return Err("该数据库不包含 Codex 日志表".into());
    }
    Ok(conn)
}

// ---------------------------------------------------------------------------
// Configuration / thread metadata
// ---------------------------------------------------------------------------

fn read_config(root: &Path) -> ConfigInfo {
    let mut info = ConfigInfo::default();
    let Ok(text) = fs::read_to_string(root.join("config.toml")) else {
        return info;
    };
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return info;
    };
    info.provider = doc
        .get("model_provider")
        .and_then(|v| v.as_str())
        .unwrap_or("openai")
        .to_string();
    info.model = doc
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if let Some(providers) = doc.get("model_providers").and_then(|v| v.as_table()) {
        for (name, item) in providers.iter().take(64) {
            if let Some(url) = item.get("base_url").and_then(|v| v.as_str()) {
                info.providers.push(ProviderUrl {
                    name: name.to_string(),
                    base_url: url.to_string(),
                });
            }
        }
    }
    info.base_url = info
        .providers
        .iter()
        .find(|p| p.name == info.provider)
        .map(|p| p.base_url.clone())
        .or_else(|| {
            (info.provider == "openai")
                .then(|| doc.get("openai_base_url").and_then(|v| v.as_str()))
                .flatten()
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            if info.provider == "openai" {
                "https://api.openai.com/v1".into()
            } else {
                String::new()
            }
        });
    if let Some((scheme, host)) = split_url(&info.base_url) {
        info.expected_scheme = scheme;
        info.expected_host = host;
    }
    info
}

fn split_url(url: &str) -> Option<(String, String)> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let host = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    Some((parsed.scheme().to_string(), host))
}

#[derive(Clone, Default)]
struct ThreadMeta {
    title: String,
    provider: String,
    model: String,
}

fn read_threads(path: Option<&Path>) -> HashMap<String, ThreadMeta> {
    let mut map = HashMap::new();
    let Some(path) = path else {
        return map;
    };
    let Ok(conn) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return map;
    };
    let _ = conn.busy_timeout(Duration::from_millis(800));
    let _ = conn.execute_batch("PRAGMA query_only=1;");
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, COALESCE(title,''), COALESCE(model_provider,''), COALESCE(model,'') \
         FROM threads ORDER BY updated_at DESC LIMIT 5000",
    ) else {
        return map;
    };
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            ThreadMeta {
                title: row.get(1)?,
                provider: row.get(2)?,
                model: row.get(3)?,
            },
        ))
    });
    if let Ok(rows) = rows {
        for row in rows.flatten() {
            map.insert(row.0, row.1);
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

struct Patterns {
    url: regex::Regex,
    url_colon: regex::Regex,
    status: regex::Regex,
    method: regex::Regex,
    timeout: regex::Regex,
    connect: regex::Regex,
    thread: regex::Regex,
    thread_dot: regex::Regex,
    turn: regex::Regex,
    turn_dot: regex::Regex,
    model: regex::Regex,
    retries: regex::Regex,
    max_retries: regex::Regex,
    sampling_error: regex::Regex,
    tool_name: regex::Regex,
    total_ms: regex::Regex,
    error_payload: regex::Regex,
    uuid: regex::Regex,
    number: regex::Regex,
    duration: regex::Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| Patterns {
        url: regex::Regex::new(r"(?:^|\s)url=(\S+)").unwrap(),
        url_colon: regex::Regex::new(r"url:\s*(\S+)").unwrap(),
        status: regex::Regex::new(r"status=(\d{3})").unwrap(),
        method: regex::Regex::new(r"method=([A-Za-z]+)").unwrap(),
        timeout: regex::Regex::new(r"is_timeout=(true|false)").unwrap(),
        connect: regex::Regex::new(r"is_connect=(true|false)").unwrap(),
        thread: regex::Regex::new(r#"thread_id=\"?([0-9a-fA-F-]{20,})"#).unwrap(),
        thread_dot: regex::Regex::new(r#"thread\.id=\"?([0-9a-fA-F-]{20,})"#).unwrap(),
        turn: regex::Regex::new(r#"turn_id=\"?([0-9a-fA-F-]{20,})"#).unwrap(),
        turn_dot: regex::Regex::new(r#"turn\.id=\"?([0-9a-fA-F-]{20,})"#).unwrap(),
        model: regex::Regex::new(r"\bmodel=([^\s}]+)").unwrap(),
        retries: regex::Regex::new(r"\bretries=(\d+)").unwrap(),
        max_retries: regex::Regex::new(r"max_retries=(\d+)").unwrap(),
        sampling_error: regex::Regex::new(r"sampling_error=(.+)$").unwrap(),
        tool_name: regex::Regex::new(r#"tool_name=\"?([^\s\"}]+)"#).unwrap(),
        total_ms: regex::Regex::new(r"total_duration_ms=(\d+)").unwrap(),
        error_payload: regex::Regex::new(r#""message"\s*:\s*"([^"]+)""#).unwrap(),
        uuid: regex::Regex::new(
            r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
        )
        .unwrap(),
        number: regex::Regex::new(r"\b\d{4,}\b").unwrap(),
        duration: regex::Regex::new(r"\b\d+(?:\.\d+)?\s?(?:ns|µs|ms|s|m|h)\b").unwrap(),
    })
}

fn split_span(body: &str) -> (&str, &str) {
    match body.rfind("}: ") {
        Some(index) => (&body[..index], &body[index + 3..]),
        None => ("", body),
    }
}

fn first_capture(regex: &regex::Regex, text: &str) -> Option<String> {
    regex
        .captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

fn tail(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

fn pid_of(process: Option<&str>) -> Option<u32> {
    let value = process?;
    value.strip_prefix("pid:")?.split(':').next()?.parse().ok()
}

fn take_json_object(text: &str) -> Option<(&str, &str)> {
    let start = text.find('{')?;
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, byte) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match *byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((&text[start..=offset], &text[offset + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_http(message: &str) -> Option<HttpInfo> {
    let (ok, rest) = if let Some(rest) = message.strip_prefix("Request completed ") {
        (true, rest)
    } else if let Some(rest) = message.strip_prefix("Request failed ") {
        (false, rest)
    } else {
        return None;
    };
    let mut info = HttpInfo {
        ok,
        ..Default::default()
    };
    let mut head = rest;
    if let Some((json, after)) = take_json_object(rest) {
        head = rest[..rest.find(json).unwrap_or(0)].trim_end();
        if let Ok(serde_json::Value::Object(headers)) =
            serde_json::from_str::<serde_json::Value>(json)
        {
            let get = |key: &str| {
                headers
                    .get(key)
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            };
            info.request_id = get("x-request-id");
            info.client_request_id = get("x-client-request-id");
            info.server = get("server");
            info.content_type = get("content-type");
        }
        let _ = after;
    }
    if let Some(url) = first_capture(&patterns().url, head) {
        info.url = url;
    }
    if let Some(method) = first_capture(&patterns().method, head) {
        info.method = method;
    }
    if let Some(caps) = patterns().status.captures(head) {
        info.status = caps.get(1).and_then(|m| m.as_str().parse().ok());
        if let Some(matched) = caps.get(0) {
            let after = &head[matched.end()..];
            let end = after
                .find(" headers=")
                .or_else(|| after.find(" version="))
                .unwrap_or(after.len());
            let reason = after[..end].trim();
            info.reason = (!reason.is_empty()).then(|| reason.to_string());
        }
    }
    info.timeout = first_capture(&patterns().timeout, head).as_deref() == Some("true");
    info.connect = first_capture(&patterns().connect, head).as_deref() == Some("true");
    if let Some((scheme, host)) = split_url(&info.url) {
        info.scheme = scheme;
        info.host = host;
        if let Ok(parsed) = reqwest::Url::parse(&info.url) {
            info.path = parsed.path().to_string();
        }
    }
    // "completed" means an HTTP response arrived, including 401/429/5xx.
    info.ok = ok
        && info
            .status
            .is_some_and(|status| (200..400).contains(&status));
    Some(info)
}

fn parse_event(
    id: i64,
    ts: i64,
    ts_nanos: i64,
    level: &str,
    target: &str,
    body: &str,
    thread_column: Option<&str>,
    process: Option<&str>,
) -> Event {
    let (span, message) = split_span(body);
    let thread_id = thread_column
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or_else(|| first_capture(&patterns().thread, span))
        .or_else(|| first_capture(&patterns().thread_dot, span));
    let turn_id =
        first_capture(&patterns().turn, span).or_else(|| first_capture(&patterns().turn_dot, span));
    let model = first_capture(&patterns().model, span);
    let http = parse_http(message);
    let lower = message.to_lowercase();

    let mut event = Event {
        id,
        time: (ts as u64).saturating_mul(1000) + (ts_nanos.max(0) as u64) / 1_000_000,
        level: level.to_string(),
        target: target.to_string(),
        kind: "other".into(),
        thread_id,
        turn_id,
        pid: pid_of(process),
        model,
        summary: String::new(),
        http,
    };

    event.kind = if event.http.is_some() {
        "request".into()
    } else if target == "codex_core::responses_retry" {
        "retry".into()
    } else if message.contains("Turn error:") {
        "turn_error".into()
    } else if target.starts_with("codex_api::")
        && (target.contains("sse") || target.contains("websocket"))
        && (matches!(level, "WARN" | "ERROR")
            || lower.contains("error")
            || lower.contains("disconnected"))
    {
        "sse".into()
    } else if target.contains("mcp") && matches!(level, "WARN" | "ERROR") {
        "mcp".into()
    } else if target == "codex_core::tools::parallel" {
        "tool".into()
    } else if target == "codex_core::session::session" {
        "session".into()
    } else if matches!(level, "WARN" | "ERROR") {
        "error".into()
    } else {
        "other".into()
    };

    event.summary = match event.kind.as_str() {
        "request" => {
            let http = event.http.as_ref().unwrap();
            let route = if http.host.is_empty() {
                http.url.clone()
            } else {
                http.host.clone()
            };
            let method = if http.method.is_empty() {
                "HTTP"
            } else {
                &http.method
            };
            if http.ok {
                let status = http
                    .status
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".into());
                let reason = http.reason.clone().unwrap_or_default();
                format!("{method} {route} · {status} {reason}")
                    .trim_end()
                    .to_string()
            } else {
                let why = if http.timeout {
                    "超时"
                } else if http.connect {
                    "连接失败"
                } else {
                    "请求失败"
                };
                match http.status {
                    Some(status) => format!(
                        "{method} {route} · {status} {}",
                        http.reason.as_deref().unwrap_or(why)
                    ),
                    None => format!("{method} {route} · {why}"),
                }
            }
        }
        "retry" => {
            let retries = first_capture(&patterns().retries, message).unwrap_or_default();
            let max = first_capture(&patterns().max_retries, message).unwrap_or_default();
            let error = first_capture(&patterns().sampling_error, message)
                .or_else(|| {
                    message
                        .split("error=")
                        .nth(1)
                        .map(|v| v.split("</").next().unwrap_or(v).to_string())
                })
                .or_else(|| first_capture(&patterns().url_colon, message))
                .unwrap_or_default();
            let error = error.trim();
            if retries.is_empty() {
                if error.is_empty() {
                    "等待重试".into()
                } else {
                    format!("准备重试 · {}", tail(error, 120))
                }
            } else {
                format!("重试 {retries}/{max} · {}", tail(error, 120))
            }
        }
        "turn_error" => {
            let detail = message
                .split("Turn error:")
                .nth(1)
                .unwrap_or(message)
                .trim();
            let detail = first_capture(&patterns().error_payload, detail).unwrap_or_else(|| {
                let url = first_capture(&patterns().url_colon, detail);
                match url {
                    Some(url) => {
                        let base = detail.split(", url:").next().unwrap_or(detail);
                        format!("{base} · {url}")
                    }
                    None => detail.to_string(),
                }
            });
            format!("回合失败：{}", tail(&detail, 160))
        }
        "sse" => {
            let detail = message.strip_prefix("SSE Error:").unwrap_or(message).trim();
            format!("流中断：{}", tail(detail, 160))
        }
        "mcp" => {
            let name = message
                .split("server_name=")
                .nth(1)
                .and_then(|v| v.split_whitespace().next())
                .map(|v| v.trim_matches('"').to_string())
                .unwrap_or_default();
            let detail = message.split("error=").nth(1).unwrap_or(message);
            if name.is_empty() {
                format!("MCP：{}", tail(detail, 150))
            } else {
                format!("MCP {name}：{}", tail(detail, 140))
            }
        }
        "tool" => {
            let name =
                first_capture(&patterns().tool_name, message).unwrap_or_else(|| "工具".into());
            match first_capture(&patterns().total_ms, message) {
                Some(ms) => format!("{name} · {ms} ms"),
                None => format!("{name} 调用完成"),
            }
        }
        "session" => {
            if let Some(index) = message.find("base_url: Some(") {
                let rest = &message[index + 16..];
                let url = rest.split('"').next().unwrap_or("");
                format!("会话初始化 · {url}")
            } else if let Some(index) = message.find("Configuring session:") {
                format!("会话初始化 · {}", tail(&message[index + 20..], 120))
            } else {
                format!("会话：{}", tail(message, 140))
            }
        }
        _ => {
            if let Some(index) = message.find("Configuring session:") {
                let detail = &message[index + 20..];
                return Event {
                    summary: format!("会话初始化 · {}", tail(detail, 120)),
                    ..event
                };
            }
            let _ = lower;
            tail(message, 180)
        }
    };

    event
}

fn signature_of(message: &str) -> String {
    let p = patterns();
    let value = p.uuid.replace_all(message, "<id>");
    // Timing and retry counters vary per attempt; they must not split one cause.
    let value = p.duration.replace_all(&value, "<t>");
    let value = value
        .replace("retries=", "retries=<n>")
        .replace("max_retries=", "max_retries=<n>");
    let value = p.number.replace_all(&value, "<n>");
    let value = value.replace('\n', " ");
    tail(value.trim(), 180)
}

fn advice_for(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    if lower.contains("modelprotocolunsupported")
        || lower.contains("does not support this protocol")
    {
        "该模型不接受 Responses 协议。改用支持 Responses 的模型，或把 provider 的 wire_api 调整为 chat。"
    } else if lower.contains("401") || lower.contains("unauthorized") {
        "凭据被拒绝：核对 provider 的 API Key、auth.json 与账号额度。"
    } else if lower.contains("403") || lower.contains("forbidden") {
        "服务端拒绝访问：检查 Key 权限、IP 白名单或线路是否被限制。"
    } else if lower.contains("429") || lower.contains("rate limit") {
        "触发限流：降低并发、等待窗口重置，或切换到其他线路。"
    } else if lower.contains("502") || lower.contains("503") || lower.contains("504") {
        "上游网关错误：线路服务或反向代理未拿到模型响应；对照服务端 nginx 日志确认。"
    } else if lower.contains("timed out") || lower.contains("is_timeout=true") {
        "请求超时：检查网络与代理，确认线路节点可用。"
    } else if lower.contains("is_connect=true")
        || lower.contains("connection refused")
        || lower.contains("dns")
    {
        "连接失败：地址、端口或 DNS 不可达；先用测速确认线路。"
    } else if lower.contains("sse") || lower.contains("decoding response body") {
        "流式响应中断：中间代理可能缓冲或截断 SSE，检查 nginx 的 proxy_buffering 与超时设置。"
    } else if lower.contains("mcp") {
        "MCP 服务不可用：检查对应 MCP 进程、端口与启动命令。"
    } else if lower.contains("permission denied") || lower.contains("access is denied") {
        "权限被拒绝：核对目录权限、沙盒策略与审批结果。"
    } else {
        "查看同线程的请求与重试记录，结合发生时间定位上游与服务端日志。"
    }
}

fn is_benign(target: &str) -> bool {
    matches!(
        target,
        "codex_core_plugins::marketplace"
            | "codex_core_plugins::loader"
            | "codex_core_plugins::manager"
            | "codex_skills::interface"
            | "codex_core::shell_snapshot"
            | "codex_analytics::client"
            | "codex_models_manager::model_info"
    )
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

struct RawRow {
    id: i64,
    ts: i64,
    ts_nanos: i64,
    level: String,
    target: String,
    body: String,
    thread_id: Option<String>,
    process: Option<String>,
}

const COLUMNS: &str =
    "id, ts, ts_nanos, level, target, COALESCE(feedback_log_body,''), thread_id, process_uuid";

fn read_rows(conn: &Connection, sql: &str, params: Vec<SqlValue>) -> Result<Vec<RawRow>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| format!("查询失败：{e}"))?;
    let rows = stmt
        .query_map(params_from_iter(params), |row| {
            Ok(RawRow {
                id: row.get(0)?,
                ts: row.get(1)?,
                ts_nanos: row.get(2)?,
                level: row.get(3)?,
                target: row.get(4)?,
                body: row.get(5)?,
                thread_id: row.get(6)?,
                process: row.get(7)?,
            })
        })
        .map_err(|e| format!("查询失败：{e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("读取记录失败：{e}"))?);
    }
    Ok(out)
}

fn to_events(rows: Vec<RawRow>) -> Vec<Event> {
    rows.into_iter()
        .map(|row| {
            parse_event(
                row.id,
                row.ts,
                row.ts_nanos,
                &row.level,
                &row.target,
                &row.body,
                row.thread_id.as_deref(),
                row.process.as_deref(),
            )
        })
        .collect()
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn kind_clause(kinds: &[String]) -> Option<(String, Vec<SqlValue>)> {
    let mut clauses = Vec::new();
    for kind in kinds {
        let clause = match kind.as_str() {
            "request" => "target = 'codex_http_client::client'".to_string(),
            "retry" => "target = 'codex_core::responses_retry'".to_string(),
            "turn_error" => "feedback_log_body LIKE '%Turn error:%'".to_string(),
            "sse" => "target = 'codex_api::sse::responses'".to_string(),
            "mcp" => "(target LIKE '%mcp%' OR target LIKE 'rmcp%')".to_string(),
            "tool" => "target = 'codex_core::tools::parallel'".to_string(),
            "session" => "target = 'codex_core::session::session'".to_string(),
            "error" => "level IN ('WARN','ERROR')".to_string(),
            _ => continue,
        };
        clauses.push(clause);
    }
    if clauses.is_empty() {
        None
    } else {
        Some((format!("({})", clauses.join(" OR ")), Vec::new()))
    }
}

fn query_events(conn: &Connection, query: &EventQuery) -> Result<EventPage, String> {
    let mut where_parts: Vec<String> = Vec::new();
    let mut params: Vec<SqlValue> = Vec::new();

    if let Some(kinds) = query.kinds.as_ref().filter(|v| !v.is_empty()) {
        if let Some((clause, _)) = kind_clause(kinds) {
            where_parts.push(clause);
        }
    }
    if let Some(levels) = query.levels.as_ref().filter(|v| !v.is_empty()) {
        let placeholders = vec!["?"; levels.len()].join(",");
        where_parts.push(format!("level IN ({placeholders})"));
        for level in levels {
            params.push(SqlValue::Text(level.clone()));
        }
    }
    if let Some(thread) = query.thread_id.as_deref().filter(|v| !v.is_empty()) {
        where_parts.push("thread_id = ?".into());
        params.push(SqlValue::Text(thread.to_string()));
    }
    if let Some(host) = query.host.as_deref().filter(|v| !v.is_empty()) {
        where_parts.push("feedback_log_body LIKE ? ESCAPE '\\'".into());
        params.push(SqlValue::Text(format!("%{}%", escape_like(host))));
    }
    if let Some(search) = query.search.as_deref().filter(|v| !v.trim().is_empty()) {
        let needle = format!("%{}%", escape_like(search.trim()));
        where_parts
            .push("(feedback_log_body LIKE ? ESCAPE '\\' OR target LIKE ? ESCAPE '\\')".into());
        params.push(SqlValue::Text(needle.clone()));
        params.push(SqlValue::Text(needle));
    }
    if let Some(outcome) = query.outcome.as_deref() {
        match outcome {
            "ok" => where_parts.push("feedback_log_body LIKE '%Request completed%'".into()),
            "failed" => where_parts.push("feedback_log_body LIKE '%Request failed%'".into()),
            _ => {}
        }
    }
    if let Some(since) = query.since.filter(|v| *v > 0) {
        where_parts.push("ts >= ?".into());
        params.push(SqlValue::Integer((since / 1000) as i64));
    }
    if let Some(until) = query.until.filter(|v| *v > 0) {
        where_parts.push("ts <= ?".into());
        params.push(SqlValue::Integer((until / 1000) as i64));
    }
    let ascending = query.after_id.is_some();
    if let Some(after) = query.after_id {
        where_parts.push("id > ?".into());
        params.push(SqlValue::Integer(after));
    }
    if let Some(before) = query.before_id {
        where_parts.push("id < ?".into());
        params.push(SqlValue::Integer(before));
    }

    let limit = query.limit.unwrap_or(300).clamp(1, MAX_LIMIT);
    let where_sql = if where_parts.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_parts.join(" AND "))
    };
    let order = if ascending { "ASC" } else { "DESC" };
    let sql = format!(
        "SELECT {COLUMNS} FROM logs {where_sql} ORDER BY id {order} LIMIT {}",
        limit + 1
    );
    let mut rows = read_rows(conn, &sql, params)?;
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    if !ascending {
        // keep newest first
    }
    let scanned = rows.len();
    let events = to_events(rows);
    let next_before = events.last().map(|e| e.id);
    let next_after = events.first().map(|e| e.id);
    Ok(EventPage {
        events,
        has_more,
        next_before,
        next_after,
        scanned,
    })
}

fn overview_at(
    logs_path: Option<&Path>,
    state_path: Option<&Path>,
    config: ConfigInfo,
    window: u64,
) -> Result<Overview, String> {
    let mut overview = Overview {
        generated_at: logging::now(),
        window,
        source: source_info(logs_path),
        state: state_info(state_path),
        config,
        ..Default::default()
    };
    let Some(logs_path) = logs_path else {
        overview.verdict = Verdict {
            status: "unverified".into(),
            title: "未找到 Codex 日志数据库".into(),
            message: "当前 Codex 版本可能未启用 SQLite 日志，或 CODEX_HOME 指向了其他目录。".into(),
            evidence: vec![crate::diagnostics::codex_home().to_string_lossy().into()],
        };
        return Ok(overview);
    };
    if let Some(error) = overview.source.error.clone() {
        overview.verdict = Verdict {
            status: "unverified".into(),
            title: "日志数据库不可读".into(),
            message: error.clone(),
            evidence: vec![overview.source.path.clone()],
        };
        return Ok(overview);
    }
    let conn = open_db(logs_path)?;
    let since = if window == 0 {
        0
    } else {
        (logging::now() / 1000).saturating_sub(window) as i64
    };
    let sql = format!(
        "SELECT {COLUMNS} FROM logs \
         WHERE ts >= ? AND level <> 'TRACE' AND ( \
             target = 'codex_http_client::client' \
             OR target = 'codex_core::responses_retry' \
             OR target = 'codex_core::session::turn' \
             OR target = 'codex_api::sse::responses' \
             OR target = 'codex_core::tools::parallel' \
             OR target = 'codex_core::session::session' \
             OR target LIKE '%mcp%' \
             OR level IN ('WARN','ERROR') \
         ) ORDER BY id DESC LIMIT {OVERVIEW_LIMIT}"
    );
    let rows = read_rows(&conn, &sql, vec![SqlValue::Integer(since)])?;
    let truncated = rows.len() >= OVERVIEW_LIMIT;
    let events = to_events(rows);
    let threads_meta = read_threads(state_path);

    let mut routes: BTreeMap<String, RouteStat> = BTreeMap::new();
    let mut threads: BTreeMap<String, ThreadStat> = BTreeMap::new();
    let mut processes: BTreeMap<u32, ProcessStat> = BTreeMap::new();
    let mut clusters: BTreeMap<(String, String), Cluster> = BTreeMap::new();

    for event in &events {
        if let Some(pid) = event.pid {
            let entry = processes.entry(pid).or_insert_with(|| ProcessStat {
                pid,
                ..Default::default()
            });
            entry.events += 1;
            entry.last_at = entry.last_at.max(event.time);
        }
        if let Some(http) = &event.http {
            let key = if http.host.is_empty() {
                http.url.clone()
            } else {
                http.host.clone()
            };
            let entry = routes.entry(key.clone()).or_default();
            entry.host = key.clone();
            entry.scheme = http.scheme.clone();
            entry.requests += 1;
            entry.last_at = entry.last_at.max(event.time);
            if let Some(pid) = event.pid {
                if let Some(process) = processes.get_mut(&pid) {
                    process.requests += 1;
                }
            }
            match http.status {
                Some(status) if (200..400).contains(&status) => {
                    entry.ok += 1;
                    entry.last_status = Some(status);
                }
                Some(status) => {
                    entry.failed += 1;
                    entry.last_status = Some(status);
                }
                None => {
                    entry.failed += 1;
                }
            }
            if let Some(status) = http.status {
                match entry.statuses.iter_mut().find(|s| s.status == status) {
                    Some(item) => item.count += 1,
                    None => entry.statuses.push(StatusCount { status, count: 1 }),
                }
            }
            if let Some(thread_id) = &event.thread_id {
                let thread = threads
                    .entry(thread_id.clone())
                    .or_insert_with(|| ThreadStat {
                        id: thread_id.clone(),
                        ..Default::default()
                    });
                thread.requests += 1;
                if !http.ok {
                    thread.failures += 1;
                }
                thread.last_at = thread.last_at.max(event.time);
                if !http.host.is_empty() && !thread.hosts.contains(&http.host) {
                    thread.hosts.push(http.host.clone());
                }
            }
        }
        if matches!(event.level.as_str(), "WARN" | "ERROR") {
            let signature = signature_of(&event.summary);
            let key = (event.target.clone(), signature.clone());
            let entry = clusters.entry(key).or_insert_with(|| Cluster {
                signature: signature.clone(),
                title: tail(&event.summary, 140),
                level: event.level.clone(),
                target: event.target.clone(),
                first_at: event.time,
                last_at: event.time,
                sample: event.summary.clone(),
                advice: advice_for(&event.summary).into(),
                benign: is_benign(&event.target),
                ..Default::default()
            });
            entry.count += 1;
            entry.first_at = entry.first_at.min(event.time);
            entry.last_at = entry.last_at.max(event.time);
            if entry.ids.len() < 5 {
                entry.ids.push(event.id);
            }
        }
    }

    let expected = overview.config.expected_host.clone();
    for route in routes.values_mut() {
        route.configured = !expected.is_empty() && route.host == expected;
        route.statuses.sort_by_key(|s| s.status);
        route.threads = threads
            .values()
            .filter(|t| t.hosts.iter().any(|h| h == &route.host))
            .count();
    }

    for thread in threads.values_mut() {
        if let Some(meta) = threads_meta.get(&thread.id) {
            thread.title = Some(meta.title.clone()).filter(|v| !v.is_empty());
            thread.provider = Some(meta.provider.clone()).filter(|v| !v.is_empty());
            thread.model = Some(meta.model.clone()).filter(|v| !v.is_empty());
        }
        let uses_expected = expected.is_empty() || thread.hosts.iter().any(|h| h == &expected);
        let only_expected =
            expected.is_empty() || thread.hosts.iter().all(|h| h == &expected || h.is_empty());
        thread.route_ok = uses_expected && only_expected;
    }

    let mut route_list: Vec<RouteStat> = routes.into_values().collect();
    route_list.sort_by_key(|r| std::cmp::Reverse(r.requests));
    let mut thread_list: Vec<ThreadStat> = threads.into_values().collect();
    thread_list.sort_by_key(|t| std::cmp::Reverse(t.last_at));
    thread_list.truncate(200);
    let mut process_list: Vec<ProcessStat> =
        processes.into_values().filter(|p| p.requests > 0).collect();
    process_list.sort_by_key(|p| std::cmp::Reverse(p.requests));
    let mut cluster_list: Vec<Cluster> = clusters.into_values().collect();
    cluster_list.sort_by(|a, b| {
        a.benign
            .cmp(&b.benign)
            .then_with(|| b.count.cmp(&a.count))
            .then_with(|| b.last_at.cmp(&a.last_at))
    });
    cluster_list.truncate(60);

    overview.totals = Totals {
        requests: events.iter().filter(|e| e.kind == "request").count(),
        ok: events
            .iter()
            .filter(|e| e.http.as_ref().is_some_and(|h| h.ok))
            .count(),
        failed: events
            .iter()
            .filter(|e| e.http.as_ref().is_some_and(|h| !h.ok))
            .count(),
        retries: events.iter().filter(|e| e.kind == "retry").count(),
        turn_errors: events.iter().filter(|e| e.kind == "turn_error").count(),
        warnings: events.iter().filter(|e| e.level == "WARN").count(),
        errors: events.iter().filter(|e| e.level == "ERROR").count(),
        tools: events.iter().filter(|e| e.kind == "tool").count(),
        threads: thread_list.len(),
    };

    // Route verdict: only compare threads whose provider matches the configured one.
    let configured_provider = overview.config.provider.clone();
    let mut evidence = Vec::new();
    let mut relevant_requests = 0usize;
    let mut relevant_failures = 0usize;
    let mut unknown_requests = 0usize;
    let mut mismatched: Vec<String> = Vec::new();
    let mut expected_requests = 0usize;
    for thread in &thread_list {
        let (relevant, unverified) = match thread.provider.as_deref() {
            Some(provider) => (provider == configured_provider, false),
            None => (false, true),
        };
        if !relevant {
            if unverified {
                unknown_requests += thread.requests;
            }
            continue;
        }
        for host in &thread.hosts {
            if host.is_empty() {
                continue;
            }
            if host == &expected {
                expected_requests += 1;
            } else if !mismatched.contains(host) {
                mismatched.push(host.clone());
            }
        }
        relevant_requests += thread.requests;
        relevant_failures += thread.failures;
    }
    for route in &route_list {
        if route.configured {
            evidence.push(format!(
                "{} · {} 次请求 · {} 成功 · {} 失败 · 最近 {}",
                route.host,
                route.requests,
                route.ok,
                route.failed,
                format_time(route.last_at)
            ));
        }
    }
    for host in &mismatched {
        let requests: usize = thread_list
            .iter()
            .filter(|t| {
                t.provider.as_deref() == Some(configured_provider.as_str())
                    && t.hosts.iter().any(|h| h == host)
            })
            .map(|t| t.requests)
            .sum();
        evidence.push(format!(
            "其他线路 {host} · {requests} 次请求（同 provider 线程）"
        ));
    }
    overview.verdict = if expected.is_empty() {
        Verdict {
            status: "unverified".into(),
            title: "未读取到当前线路地址".into(),
            message: format!(
                "config.toml 的 provider「{}」没有可用的 base_url。",
                configured_provider
            ),
            evidence: vec![crate::diagnostics::codex_home()
                .join("config.toml")
                .to_string_lossy()
                .into()],
        }
    } else if relevant_requests == 0 {
        Verdict {
            status: "unverified".into(),
            title: "时间窗内没有该线路的请求".into(),
            message: format!(
                "当前配置为 {expected}，但在所选时间窗内没有找到对应 provider 的模型请求。"
            ),
            evidence,
        }
    } else if !mismatched.is_empty() {
        Verdict {
            status: "mismatch".into(),
            title: "发现其他线路的请求".into(),
            message: format!(
                "当前配置为 {expected}，但同一 provider 的线程也向 {} 发起了请求。旧会话可能仍在使用切换前的线路。",
                mismatched.join("、")
            ),
            evidence,
        }
    } else if expected_requests == 0 {
        Verdict {
            status: "mismatch".into(),
            title: "该 provider 的请求未走当前线路".into(),
            message: format!("当前配置为 {expected}，但时间窗内该 provider 的请求没有发往该地址。"),
            evidence,
        }
    } else if relevant_failures > 0 && relevant_failures >= relevant_requests {
        Verdict {
            status: "failing".into(),
            title: "线路一致，但请求全部失败".into(),
            message: format!("{expected} 的请求全部失败，请检查线路服务与凭据。"),
            evidence,
        }
    } else {
        Verdict {
            status: "confirmed".into(),
            title: "请求确实走当前线路".into(),
            message: format!(
                "时间窗内该 provider 的 {relevant_requests} 次模型请求全部发往 {expected}。"
            ),
            evidence,
        }
    };

    overview.notes.push(format!(
        "当前配置：provider「{}」，模型 {}，base_url {}",
        overview.config.provider,
        if overview.config.model.is_empty() {
            "未指定"
        } else {
            &overview.config.model
        },
        if overview.config.base_url.is_empty() {
            "未配置"
        } else {
            &overview.config.base_url
        }
    ));
    if overview.source.wal_size > 0 {
        overview.notes.push(format!(
            "数据库处于 WAL 模式，未合并的 WAL 约 {:.1} MB；已包含最新记录。",
            overview.source.wal_size as f64 / 1_048_576.0
        ));
    }
    if overview.state.readable {
        overview.notes.push(format!(
            "线程归属来自 {}，共读取 {} 个线程标题。",
            overview.state.name,
            threads_meta.len()
        ));
    } else {
        overview.notes.push(
            "未能读取 state 数据库，线程的 provider 归属无法核对，只能按实际请求地址判断。".into(),
        );
    }
    if process_list.len() > 1 {
        overview.notes.push(format!(
            "检测到 {} 个 Codex 进程：{}。不同进程可能使用不同线路。",
            process_list.len(),
            process_list
                .iter()
                .map(|p| format!("pid {}", p.pid))
                .collect::<Vec<_>>()
                .join("、")
        ));
    }
    if unknown_requests > 0 {
        overview.notes.push(format!(
            "有 {unknown_requests} 次请求来自 state 数据库中没有记录的线程，未纳入线路统计。"
        ));
    }
    if truncated {
        overview.notes.push(format!(
            "时间窗内相关记录超过 {OVERVIEW_LIMIT} 条，统计已截断；请缩小时间范围。"
        ));
    }

    overview.routes = route_list;
    overview.threads = thread_list;
    overview.processes = process_list;
    overview.errors = cluster_list;
    overview.recent = events
        .iter()
        .filter(|e| {
            matches!(e.kind.as_str(), "retry" | "turn_error" | "sse" | "mcp")
                || e.level == "ERROR"
                || e.http.as_ref().is_some_and(|h| !h.ok)
        })
        .take(40)
        .cloned()
        .collect();
    Ok(overview)
}

/// Format epoch milliseconds as an unambiguous UTC timestamp (no locale, no timezone guess).
fn format_time(ms: u64) -> String {
    if ms == 0 {
        return "—".into();
    }
    let seconds = (ms / 1000) as i64;
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days, valid for the whole proleptic Gregorian range.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        rest / 3600,
        (rest / 60) % 60,
        rest % 60
    )
}

fn detail_at(logs_path: &Path, id: i64) -> Result<serde_json::Value, String> {
    let conn = open_db(logs_path)?;
    let sql = format!("SELECT {COLUMNS} FROM logs WHERE id = ?");
    let rows = read_rows(&conn, &sql, vec![SqlValue::Integer(id)])?;
    let row = rows.into_iter().next().ok_or("记录不存在")?;
    let (span, message) = split_span(&row.body);
    Ok(serde_json::json!({
        "id": row.id,
        "time": (row.ts as u64).saturating_mul(1000) + (row.ts_nanos.max(0) as u64) / 1_000_000,
        "level": row.level,
        "target": row.target,
        "threadId": row.thread_id,
        "pid": pid_of(row.process.as_deref()),
        "span": logging::redact(&tail(span, 4000)),
        "message": logging::redact(&tail(message, MAX_DETAIL)),
    }))
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

fn logs_db() -> Option<PathBuf> {
    latest_db(&sqlite_home(), "logs_")
}

fn state_db() -> Option<PathBuf> {
    latest_db(&sqlite_home(), "state_")
}

fn sources() -> serde_json::Value {
    let logs = logs_db();
    let state = state_db();
    let config = read_config(&crate::diagnostics::codex_home());
    let threads = read_threads(state.as_deref());
    let recent: Vec<serde_json::Value> = threads
        .iter()
        .take(200)
        .map(|(id, meta)| {
            serde_json::json!({
                "id": id,
                "title": meta.title,
                "provider": meta.provider,
                "model": meta.model,
            })
        })
        .collect();
    serde_json::json!({
        "logs": source_info(logs.as_deref()),
        "state": state_info(state.as_deref()),
        "config": config,
        "threads": recent,
        "codexHome": crate::diagnostics::codex_home(),
    })
}

fn export_report(window: u64) -> Result<serde_json::Value, String> {
    let overview = overview_at(
        logs_db().as_deref(),
        state_db().as_deref(),
        read_config(&crate::diagnostics::codex_home()),
        window,
    )?;
    let Some(logs) = logs_db() else {
        return Err("未找到日志数据库".into());
    };
    let conn = open_db(&logs)?;
    let page = query_events(
        &conn,
        &EventQuery {
            limit: Some(1500),
            ..Default::default()
        },
    )?;
    let mut report = serde_json::json!({
        "schemaVersion": 1,
        "version": env!("CARGO_PKG_VERSION"),
        "generatedAt": logging::now(),
        "overview": overview,
        "events": page.events,
        "note": "仅含 Codex 本地日志数据库的脱敏摘要与最近事件；不包含请求体、响应体或凭据。",
    });
    fn sanitize(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(text) => *text = logging::export_redact(text),
            serde_json::Value::Array(items) => items.iter_mut().for_each(sanitize),
            serde_json::Value::Object(items) => items.values_mut().for_each(sanitize),
            _ => {}
        }
    }
    sanitize(&mut report);
    let dir = logging::data_path().join("reports");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    crate::configuration::no_links(&dir)?;
    let mut old: Vec<_> = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|s| s.to_string_lossy().starts_with("codex-logs-"))
                && p.extension().is_some_and(|s| s == "json")
                && crate::configuration::no_links(p).is_ok()
        })
        .collect();
    old.sort();
    for path in old.iter().take(old.len().saturating_sub(9)) {
        let _ = fs::remove_file(path);
    }
    let name = format!(
        "codex-logs-{}-{}.json",
        logging::now(),
        uuid::Uuid::new_v4()
    );
    let path = dir.join(name);
    let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    use std::io::Write;
    file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "path": path }))
}

#[tauri::command]
pub async fn codex_logs(request: Request) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || match request.action.as_str() {
        "analyze" | "analysis-events" | "analysis-detail" | "analysis-session"
        | "analysis-export" => analysis::dispatch(&request),
        "sources" => Ok(sources()),
        "overview" => {
            let window = request.window.unwrap_or(86_400);
            let config = read_config(&crate::diagnostics::codex_home());
            serde_json::to_value(overview_at(
                logs_db().as_deref(),
                state_db().as_deref(),
                config,
                window,
            )?)
            .map_err(|e| e.to_string())
        }
        "events" => {
            let Some(path) = logs_db() else {
                return Ok(serde_json::json!({
                    "events": [],
                    "hasMore": false,
                    "nextBefore": null,
                    "nextAfter": null,
                    "scanned": 0,
                    "missing": true,
                }));
            };
            let conn = open_db(&path)?;
            let query = request.query.unwrap_or_default();
            serde_json::to_value(query_events(&conn, &query)?).map_err(|e| e.to_string())
        }
        "detail" => {
            let Some(path) = logs_db() else {
                return Err("未找到日志数据库".into());
            };
            detail_at(&path, request.id.ok_or("缺少记录标识")?)
        }
        "export" => export_report(request.window.unwrap_or(86_400)),
        _ => Err("未知日志操作".into()),
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| logging::redact(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_db() -> (PathBuf, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("cyapi-codex-logs-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("logs_2.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                ts_nanos INTEGER NOT NULL,
                level TEXT NOT NULL,
                target TEXT NOT NULL,
                feedback_log_body TEXT,
                module_path TEXT,
                file TEXT,
                line INTEGER,
                thread_id TEXT,
                process_uuid TEXT,
                estimated_bytes INTEGER NOT NULL DEFAULT 0
            );",
        )
        .unwrap();
        drop(conn);
        let state = dir.join("state_5.sqlite");
        let conn = Connection::open(&state).unwrap();
        conn.execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                updated_at INTEGER NOT NULL DEFAULT 0,
                model_provider TEXT,
                model TEXT,
                title TEXT
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads (id, updated_at, model_provider, model, title) VALUES (?, 1, 'cyapi-toolbox', 'gpt-example', '示例会话')",
            rusqlite::params!["01a33333-3333-7333-8333-333333333333"],
        )
        .unwrap();
        drop(conn);
        (dir, path, state)
    }

    fn insert(
        conn: &Connection,
        ts: i64,
        level: &str,
        target: &str,
        body: &str,
        thread: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO logs (ts, ts_nanos, level, target, feedback_log_body, thread_id, process_uuid) \
             VALUES (?, 0, ?, ?, ?, ?, 'pid:4242:test')",
            rusqlite::params![ts, level, target, body, thread],
        )
        .unwrap();
    }

    #[test]
    fn parses_completed_and_failed_requests() {
        let body = "session_loop{thread_id=01a33333-3333-7333-8333-333333333333}:turn{model=gpt-example}: Request completed method=POST url=http://203.0.113.10:4080/v1/responses status=200 OK headers={\"server\": \"nginx/1.24.0\", \"x-request-id\": \"abc\"} version=HTTP/1.1";
        let event = parse_event(
            1,
            1_792_401_568,
            0,
            "DEBUG",
            "codex_http_client::client",
            body,
            None,
            Some("pid:4242:x"),
        );
        assert_eq!(event.kind, "request");
        let http = event.http.unwrap();
        assert!(http.ok);
        assert_eq!(http.status, Some(200));
        assert_eq!(http.host, "203.0.113.10:4080");
        assert_eq!(http.path, "/v1/responses");
        assert_eq!(http.request_id.as_deref(), Some("abc"));
        assert_eq!(http.reason.as_deref(), Some("OK"));
        assert_eq!(event.pid, Some(4242));

        let failed = parse_event(2, 1_792_401_569, 0, "DEBUG", "codex_http_client::client",
            "x}: Request failed method=POST url=http://203.0.113.10:4080/v1/responses is_timeout=false is_connect=true", None, None);
        let http = failed.http.unwrap();
        assert!(!http.ok);
        assert!(http.connect);
        assert_eq!(http.status, None);
    }

    #[test]
    fn parses_retry_and_turn_error() {
        let retry = parse_event(1, 10, 0, "WARN", "codex_core::responses_retry",
            "turn{thread.id=01a11111-1111-7111-8111-111111111111 turn.id=01a22222-2222-7222-8222-222222222222 model=gpt-example}: stream disconnected - retrying sampling request (5/5 in 3.057s)... turn_id=01a22222-2222-7222-8222-222222222222 retries=5 max_retries=5 sampling_error=unexpected status 502 Bad Gateway: Unknown error, url: http://198.51.100.10:8080/v1/responses",
            None, None);
        assert_eq!(retry.kind, "retry");
        assert_eq!(
            retry.thread_id.as_deref(),
            Some("01a11111-1111-7111-8111-111111111111")
        );
        assert_eq!(
            retry.turn_id.as_deref(),
            Some("01a22222-2222-7222-8222-222222222222")
        );
        assert!(retry.summary.contains("5/5"));
        assert!(retry.summary.contains("502"));

        let turn = parse_event(2, 10, 0, "INFO", "codex_core::session::turn",
            "turn{thread.id=01a11111-1111-7111-8111-111111111111 turn.id=01a22222-2222-7222-8222-222222222222 model=gpt-example}: Turn error: unexpected status 502 Bad Gateway: Unknown error, url: http://198.51.100.10:8080/v1/responses",
            None, None);
        assert_eq!(turn.kind, "turn_error");
        assert!(turn.summary.contains("502"));
    }

    #[test]
    fn clusters_repeated_errors() {
        assert_eq!(
            signature_of("unexpected status 502 for 12345"),
            signature_of("unexpected status 502 for 99999")
        );
        assert_ne!(
            signature_of("unexpected status 502"),
            signature_of("unexpected status 401")
        );
        assert!(advice_for("unexpected status 401 Unauthorized").contains("凭据"));
        assert!(advice_for("SSE Error: Transport error").contains("流式"));
    }

    #[test]
    fn reads_rows_and_reports_mismatch() {
        let (dir, path, state) = sample_db();
        let conn = Connection::open(&path).unwrap();
        let thread = "01a33333-3333-7333-8333-333333333333";
        insert(&conn, 1_792_401_500, "DEBUG", "codex_http_client::client",
            &format!("session_loop{{thread_id={thread}}}: Request completed method=POST url=http://203.0.113.10:4080/v1/responses status=200 OK headers={{}} version=HTTP/1.1"), Some(thread));
        insert(&conn, 1_792_401_560, "DEBUG", "codex_http_client::client",
            &format!("session_loop{{thread_id={thread}}}: Request failed method=POST url=http://198.51.100.10:8080/v1/responses is_timeout=false is_connect=true"), Some(thread));
        insert(&conn, 1_792_401_570, "WARN", "codex_core::responses_retry",
            &format!("session_loop{{thread_id={thread}}}: stream disconnected - retrying sampling request (2/5 in 1.4s)... retries=2 max_retries=5 sampling_error=unexpected status 502 Bad Gateway"), Some(thread));
        drop(conn);

        let config = ConfigInfo {
            provider: "cyapi-toolbox".into(),
            model: "gpt-example".into(),
            base_url: "http://203.0.113.10:4080/v1".into(),
            expected_host: "203.0.113.10:4080".into(),
            expected_scheme: "http".into(),
            ..Default::default()
        };
        let overview = overview_at(Some(&path), Some(&state), config, 0).unwrap();
        assert_eq!(overview.totals.requests, 2);
        assert_eq!(overview.totals.ok, 1);
        assert_eq!(overview.totals.failed, 1);
        assert_eq!(overview.totals.retries, 1);
        assert_eq!(overview.verdict.status, "mismatch");
        assert!(overview
            .threads
            .iter()
            .any(|t| t.provider.as_deref() == Some("cyapi-toolbox") && t.title.is_some()));
        assert!(overview
            .routes
            .iter()
            .any(|r| r.host == "203.0.113.10:4080" && r.configured));

        let conn = Connection::open(&path).unwrap();
        let page = query_events(
            &conn,
            &EventQuery {
                kinds: Some(vec!["request".into()]),
                outcome: Some("failed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(
            page.events[0].http.as_ref().unwrap().host,
            "198.51.100.10:8080"
        );

        let detail = detail_at(&path, page.events[0].id).unwrap();
        assert!(detail["message"]
            .as_str()
            .unwrap()
            .contains("Request failed"));
        drop(conn);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn incremental_cursor_only_returns_new_rows() {
        let (dir, path, _state) = sample_db();
        let conn = Connection::open(&path).unwrap();
        insert(&conn, 1, "INFO", "codex_core::session", "first", None);
        insert(&conn, 2, "INFO", "codex_core::session", "second", None);
        let first = query_events(&conn, &EventQuery::default()).unwrap();
        let newest = first.events[0].id;
        insert(&conn, 3, "INFO", "codex_core::session", "third", None);
        let page = query_events(
            &conn,
            &EventQuery {
                after_id: Some(newest),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].summary, "third");
        drop(conn);
        let _ = fs::remove_dir_all(dir);
    }
}
