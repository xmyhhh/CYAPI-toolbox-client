//! Codex-specific evidence: runtime SQLite, state index, CLI/desktop tracing,
//! and on-demand rollout lifecycles. All input is local and read-only.
use super::{
    parse_event, read_config, split_span, tail, ConfigInfo, EventQuery, HttpInfo, Request,
};
use crate::{configuration::no_links, logging};
use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

const MAX_RECORDS: usize = 20_000;
const MAX_FILES: usize = 24;
const TEXT_BYTES: u64 = 512 * 1024;
const ROLLOUT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SESSIONS: usize = 500;

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Source {
    id: String,
    role: String,
    format: String,
    name: String,
    path: String,
    size: u64,
    wal_size: u64,
    modified_at: u64,
    status: String,
    records: usize,
    partial: bool,
    note: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Location {
    source_id: String,
    source_role: String,
    path: String,
    position: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    id: String,
    time: u64,
    level: String,
    kind: String,
    target: String,
    thread_id: Option<String>,
    turn_id: Option<String>,
    model: Option<String>,
    summary: String,
    http: Option<HttpInfo>,
    location: Location,
    mirrors: Vec<Location>,
    #[serde(skip)]
    message: String,
    #[serde(skip)]
    span: String,
    #[serde(skip)]
    sequence: u64,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    id: String,
    title: String,
    cwd: String,
    provider: String,
    model: String,
    origin: String,
    updated_at: u64,
    archived: bool,
    requests: usize,
    failures: usize,
    retries: usize,
    turn_errors: usize,
    hosts: Vec<String>,
    rollout_path: String,
    route_status: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Issue {
    id: String,
    title: String,
    category: String,
    severity: String,
    count: usize,
    first_at: u64,
    last_at: u64,
    advice: String,
    sample: String,
    event_ids: Vec<String>,
    threads: Vec<String>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Totals {
    requests: usize,
    ok: usize,
    failed: usize,
    retries: usize,
    turn_errors: usize,
    warnings: usize,
    errors: usize,
    sessions: usize,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Route {
    address: String,
    requests: usize,
    ok: usize,
    failed: usize,
    statuses: BTreeMap<String, usize>,
    last_at: u64,
    session_ids: HashSet<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    id: String,
    generated_at: u64,
    window: u64,
    since: u64,
    codex_home: String,
    config: ConfigInfo,
    sources: Vec<Source>,
    totals: Totals,
    routes: Vec<Route>,
    sessions: Vec<Session>,
    issues: Vec<Issue>,
    notes: Vec<String>,
    partial: bool,
    mirrored: usize,
    #[serde(skip)]
    records: Vec<Record>,
}

static SNAPSHOTS: OnceLock<Mutex<VecDeque<Arc<Snapshot>>>> = OnceLock::new();

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))[..20].into()
}

/// Windows Store virtualization and verbatim paths can name the same file.
/// Compare resolved paths, preserving the original path for evidence display.
fn path_key(path: &Path) -> PathBuf {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.into());
    #[cfg(windows)]
    {
        PathBuf::from(canonical.to_string_lossy().to_lowercase())
    }
    #[cfg(not(windows))]
    {
        canonical
    }
}

fn source(path: &Path, role: &str, format: &str) -> Source {
    Source {
        id: digest(&path.to_string_lossy()),
        role: role.into(),
        format: format.into(),
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
        path: path.to_string_lossy().into(),
        size: fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        wal_size: fs::metadata(format!("{}-wal", path.display()))
            .map(|m| m.len())
            .unwrap_or(0),
        modified_at: super::modified_at(path),
        status: if path.exists() {
            "available"
        } else {
            "missing"
        }
        .into(),
        ..Default::default()
    }
}

fn open_readonly(path: &Path) -> Result<Connection, String> {
    no_links(path)?;
    if !path.is_absolute() || !path.is_file() {
        return Err("数据库不存在或不是本地绝对路径".into());
    }
    // Do not use immutable: Codex keeps live records in the WAL.
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("只读打开失败：{e}"))?;
    conn.busy_timeout(Duration::from_millis(1000))
        .map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA query_only=1;")
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

fn columns(conn: &Connection, table: &str) -> Result<HashSet<String>, String> {
    // Only fixed internal table names are passed here.
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<HashSet<_>, _>>()
        .map_err(|e| e.to_string())
}

fn optional_column(cols: &HashSet<String>, name: &str, fallback: &str) -> String {
    if cols.contains(name) {
        format!("COALESCE({name},{fallback})")
    } else {
        fallback.into()
    }
}

fn setting_dir(root: &Path, name: &str) -> Option<PathBuf> {
    let text = fs::read_to_string(root.join("config.toml")).ok()?;
    let doc = text.parse::<toml_edit::DocumentMut>().ok()?;
    let value = PathBuf::from(doc.get(name)?.as_str()?);
    Some(if value.is_absolute() {
        value
    } else {
        root.join(value)
    })
}

fn text_roots(root: &Path) -> Vec<(PathBuf, &'static str)> {
    let mut dirs = vec![(root.join("log"), "cli"), (root.join("logs"), "cli")];
    if let Some(dir) = setting_dir(root, "log_dir") {
        dirs.push((dir, "cli"));
    }
    for env_name in ["APPDATA", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(env_name) {
            let base = PathBuf::from(base);
            dirs.push((base.join("Codex/logs"), "desktop"));
            if env_name == "LOCALAPPDATA" {
                if let Ok(entries) = fs::read_dir(base.join("Packages")) {
                    for entry in entries.filter_map(Result::ok).take(3000) {
                        if entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with("OpenAI.Codex_")
                        {
                            for sub in [
                                "LocalCache/Roaming/Codex/logs",
                                "LocalCache/Local/Codex/logs",
                            ] {
                                dirs.push((entry.path().join(sub), "desktop"));
                            }
                        }
                    }
                }
            }
        }
    }
    let home = crate::diagnostics::home_path();
    dirs.push((home.join("Library/Logs/Codex"), "desktop"));
    dirs.push((home.join(".config/Codex/logs"), "desktop"));
    let mut seen = HashSet::new();
    dirs.retain(|(path, _)| seen.insert(path_key(path)));
    dirs
}

/// Walk dated directories newest first, without following links or traversing
/// unrelated home directories. Return whether the traversal budget was reached.
fn walk_files(root: &Path, rollout: bool) -> (Vec<PathBuf>, bool) {
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut files = Vec::new();
    let mut visited = 0;
    while let Some((dir, depth)) = pending.pop() {
        if no_links(&dir).is_err() {
            continue;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut children = Vec::new();
        for entry in entries.filter_map(Result::ok) {
            visited += 1;
            if visited > 20_000 {
                return (files, true);
            }
            let Ok(typ) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if typ.is_symlink() {
                continue;
            }
            if typ.is_dir() && depth < 4 {
                children.push(path);
            } else if typ.is_file() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let accepted = if rollout {
                    name.starts_with("rollout-")
                        && (name.ends_with(".jsonl") || name.ends_with(".jsonl.zst"))
                } else {
                    matches!(
                        path.extension().and_then(|e| e.to_str()),
                        Some("log" | "txt" | "jsonl")
                    ) && !name.starts_with("rollout-")
                };
                if accepted {
                    files.push(path);
                }
            }
        }
        children.sort();
        pending.extend(children.into_iter().map(|p| (p, depth + 1)));
    }
    (files, false)
}

fn db_candidates(root: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut dirs = vec![setting_dir(root, "sqlite_home").unwrap_or_else(|| root.into())];
    if dirs[0] != root {
        dirs.push(root.into());
    }
    let mut files = Vec::new();
    for (priority, dir) in dirs.iter().enumerate() {
        if no_links(dir).is_err() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.filter_map(Result::ok).take(4000) {
                if let Some(version) =
                    super::numeric_suffix(&entry.file_name().to_string_lossy(), prefix)
                {
                    files.push((priority, std::cmp::Reverse(version), entry.path()));
                }
            }
        }
    }
    files.sort();
    files.into_iter().map(|(_, _, path)| path).collect()
}

fn index_sessions(path: &Path, since: u64) -> Result<(Vec<Session>, bool), String> {
    let conn = open_readonly(path)?;
    let cols = columns(&conn, "threads")?;
    if !cols.contains("id") {
        return Err("未找到兼容的 threads 会话索引".into());
    }
    let updated = if cols.contains("updated_at_ms") && cols.contains("updated_at") {
        "COALESCE(updated_at_ms,updated_at*1000)".to_string()
    } else if cols.contains("updated_at") {
        "updated_at*1000".into()
    } else {
        "0".into()
    };
    let fields: Vec<_> = [
        "title",
        "cwd",
        "model_provider",
        "model",
        "source",
        "rollout_path",
        "archived",
    ]
    .iter()
    .map(|name| optional_column(&cols, name, if *name == "archived" { "0" } else { "''" }))
    .collect();
    let sql = format!("SELECT id, {}, {updated} FROM threads WHERE {updated}>=? ORDER BY {updated} DESC, id LIMIT ?", fields.join(","));
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![since as i64, (MAX_SESSIONS + 1) as i64], |row| {
            Ok(Session {
                id: row.get(0)?,
                title: row.get(1)?,
                cwd: row.get(2)?,
                provider: row.get(3)?,
                model: row.get(4)?,
                origin: row.get(5)?,
                rollout_path: row.get(6)?,
                archived: row.get::<_, i64>(7)? != 0,
                updated_at: row.get::<_, i64>(8)?.max(0) as u64,
                ..Default::default()
            })
        })
        .map_err(|e| e.to_string())?;
    let mut sessions = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let partial = sessions.len() > MAX_SESSIONS;
    sessions.truncate(MAX_SESSIONS);
    Ok((sessions, partial))
}

fn record(
    id: String,
    time: u64,
    level: &str,
    target: &str,
    body: &str,
    thread: Option<&str>,
    location: Location,
) -> Record {
    let body = logging::redact(body);
    let parsed = parse_event(
        0,
        (time / 1000) as i64,
        ((time % 1000) * 1_000_000) as i64,
        level,
        target,
        &body,
        thread,
        None,
    );
    let (span, message) = split_span(&body);
    static THREAD_ID: OnceLock<regex::Regex> = OnceLock::new();
    static TURN_ID: OnceLock<regex::Regex> = OnceLock::new();
    let thread_id = parsed.thread_id.or_else(|| super::first_capture(THREAD_ID.get_or_init(|| regex::Regex::new(r#"(?i)\b(?:thread[_\.]?id|conversation[_\.]?id)\"?\s*[:=]\s*\"?([0-9a-f-]{36})"#).unwrap()), &body));
    let turn_id = parsed.turn_id.or_else(|| {
        super::first_capture(
            TURN_ID.get_or_init(|| {
                regex::Regex::new(r#"(?i)\bturn[_\.]?id\"?\s*[:=]\s*\"?([0-9a-f-]{36})"#).unwrap()
            }),
            &body,
        )
    });
    let mut kind = parsed.kind;
    if let Some(http) = &parsed.http {
        // Model statistics must not count marketplace, telemetry or control-plane HTTP.
        let path = http.path.trim_end_matches('/');
        if !(path.ends_with("/responses")
            || path.ends_with("/chat/completions")
            || path.ends_with("/messages"))
        {
            kind = "http".into();
        }
    }
    if kind == "other" && target.contains("mcp") {
        kind = "mcp".into();
    }
    if kind == "other" && location.source_role == "desktop" {
        kind = "desktop".into();
    }
    let sequence = id
        .rsplit(':')
        .next()
        .and_then(|part| part.parse().ok())
        .unwrap_or(0);
    Record {
        id,
        time,
        level: level.into(),
        target: target.into(),
        kind,
        thread_id,
        turn_id,
        model: parsed.model.map(|m| m.trim_matches('"').to_string()),
        summary: parsed.summary,
        http: parsed.http,
        location,
        message: tail(message, 8192),
        span: tail(span, 4096),
        sequence,
        ..Default::default()
    }
}

fn runtime_records(
    path: &Path,
    src: &Source,
    since: u64,
    until: u64,
) -> Result<(Vec<Record>, bool), String> {
    let conn = open_readonly(path)?;
    let cols = columns(&conn, "logs")?;
    if !["id", "ts", "level", "target", "feedback_log_body"]
        .iter()
        .all(|name| cols.contains(*name))
    {
        return Err(
            "logs 表结构发生变化；缺少 id / ts / level / target / feedback_log_body".into(),
        );
    }
    let nanos = optional_column(&cols, "ts_nanos", "0");
    let thread = optional_column(&cols, "thread_id", "''");
    let mut stmt = conn
        .prepare(&format!(
        "SELECT id,ts,{nanos},level,target,substr(COALESCE(feedback_log_body,''),1,12288),{thread} \
         FROM logs WHERE ts>=? AND ts<=? AND level<>'TRACE' ORDER BY id DESC LIMIT ?"
    ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            params![
                (since / 1000) as i64,
                (until / 1000) as i64,
                (MAX_RECORDS + 1) as i64
            ],
            |row| {
                let id: i64 = row.get(0)?;
                let seconds: i64 = row.get(1)?;
                let nanos: i64 = row.get(2)?;
                let thread: String = row.get(6)?;
                Ok(record(
                    format!("{}:{id}", src.id),
                    seconds.max(0) as u64 * 1000 + nanos.max(0) as u64 / 1_000_000,
                    &row.get::<_, String>(3)?,
                    &row.get::<_, String>(4)?,
                    &row.get::<_, String>(5)?,
                    (!thread.is_empty()).then_some(thread.as_str()),
                    Location {
                        source_id: src.id.clone(),
                        source_role: src.role.clone(),
                        path: src.path.clone(),
                        position: format!("SQLite row #{id}"),
                    },
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    let mut records = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let partial = records.len() > MAX_RECORDS;
    records.truncate(MAX_RECORDS);
    records.retain(|e| e.time >= since && e.time <= until);
    Ok((records, partial))
}

fn timestamp(text: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()?
        .timestamp_millis()
        .try_into()
        .ok()
}

fn text_pattern() -> &'static regex::Regex {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(|| regex::Regex::new(r"^\[?(\d{4}-\d\d-\d\dT[^\s\]]+)\]?\s+\[?(TRACE|DEBUG|INFO|WARN|WARNING|ERROR|trace|debug|info|warn|warning|error)\]?\s+(.*)$").unwrap())
}

fn parse_text(line: &str) -> Option<(u64, String, String, String)> {
    static ANSI: OnceLock<regex::Regex> = OnceLock::new();
    let clean = ANSI
        .get_or_init(|| regex::Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").unwrap())
        .replace_all(line, "");
    if let Ok(obj) = serde_json::from_str::<Value>(&clean) {
        let time_value = obj.get("timestamp").or_else(|| obj.get("time"))?;
        let time = time_value.as_str().and_then(timestamp).or_else(|| {
            time_value
                .as_u64()
                .map(|v| if v < 100_000_000_000 { v * 1000 } else { v })
        })?;
        let level = obj
            .get("level")?
            .as_str()?
            .to_uppercase()
            .replace("WARNING", "WARN");
        let target = obj
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or("codex_text")
            .to_string();
        let mut message = obj
            .pointer("/fields/message")
            .or_else(|| obj.get("message"))
            .or_else(|| obj.get("msg"))?
            .as_str()?
            .to_string();
        for key in ["method", "url", "status", "is_timeout", "is_connect"] {
            if let Some(value) = obj.get("fields").and_then(|f| f.get(key)) {
                if !message.contains(&format!("{key}=")) {
                    let value = value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| value.to_string());
                    message.push_str(&format!(" {key}={value}"));
                }
            }
        }
        if let Some(headers) = obj.pointer("/fields/headers").and_then(Value::as_object) {
            let kept: serde_json::Map<String, Value> = headers
                .iter()
                .filter(|(key, _)| {
                    [
                        "x-request-id",
                        "x-client-request-id",
                        "server",
                        "content-type",
                    ]
                    .contains(&key.as_str())
                })
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            message.push_str(&format!(" headers={}", Value::Object(kept)));
        }
        let mut context = Vec::new();
        for span in obj
            .get("spans")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .chain(obj.get("span").into_iter())
        {
            for key in ["thread_id", "thread.id", "turn_id", "turn.id", "model"] {
                if let Some(value) = span[key].as_str() {
                    context.push(format!("{key}={value}"));
                }
            }
        }
        if !context.is_empty() {
            message = format!("span{{{}}}: {message}", context.join(" "));
        }
        return Some((time, level, target, message));
    }
    let caps = text_pattern().captures(&clean)?;
    let time = timestamp(caps.get(1)?.as_str())?;
    let level = caps
        .get(2)?
        .as_str()
        .to_uppercase()
        .replace("WARNING", "WARN");
    let rest = caps.get(3)?.as_str().trim();
    let (target, body) = if rest.starts_with('[') {
        match rest.find(']') {
            Some(end) => (&rest[1..end], rest[end + 1..].trim()),
            None => ("codex_desktop", rest),
        }
    } else {
        // tracing prints span contexts before its Rust target. Only split a
        // module-shaped target, never a colon in a URL or a JSON payload.
        static TARGET: OnceLock<regex::Regex> = OnceLock::new();
        match TARGET
            .get_or_init(|| {
                regex::Regex::new(r"(?:^|\s)([a-z_][a-z0-9_]*(?:::[a-z0-9_]+)+):\s").unwrap()
            })
            .captures(rest)
        {
            Some(c) => {
                let whole = c.get(0)?;
                let target = c.get(1)?.as_str();
                let body = format!(
                    "{} {}",
                    rest[..whole.start()].trim_end(),
                    &rest[whole.end()..]
                );
                return Some((time, level, target.to_string(), body));
            }
            None => ("codex_desktop", rest),
        }
    };
    Some((time, level, target.into(), body.into()))
}

fn text_records(
    path: &Path,
    src: &Source,
    since: u64,
    until: u64,
) -> Result<(Vec<Record>, bool, usize), String> {
    no_links(path)?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let start = len.saturating_sub(TEXT_BYTES);
    file.seek(SeekFrom::Start(start))
        .map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file.take(TEXT_BYTES));
    let mut offset = start;
    let mut line = Vec::new();
    if start > 0 {
        offset += reader
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())? as u64;
    }
    let mut records: Vec<Record> = Vec::new();
    let mut unparsed = 0;
    loop {
        line.clear();
        let bytes = reader
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        if bytes == 0 {
            break;
        }
        let text = String::from_utf8_lossy(&line);
        if let Some((time, level, target, body)) = parse_text(text.trim_end()) {
            // Keep the timestamp on continuation records; never invent one from mtime.
            if time >= since && time <= until {
                records.push(record(
                    format!("{}:{offset}", src.id),
                    time,
                    &level,
                    &target,
                    &body,
                    None,
                    Location {
                        source_id: src.id.clone(),
                        source_role: src.role.clone(),
                        path: src.path.clone(),
                        position: format!("字节偏移 {offset}"),
                    },
                ));
            } else {
                // Prevent an out-of-window continuation being attached to an older record.
                records.push(Record {
                    time,
                    ..Default::default()
                });
            }
        } else if !text.trim().is_empty() {
            if let Some(last) = records.last_mut().filter(|r| !r.id.is_empty()) {
                if last.message.len() < 8192 {
                    last.message.push_str(&logging::redact(&tail(&text, 1024)));
                }
            } else {
                unparsed += 1;
            }
        }
        offset += bytes as u64;
    }
    records.retain(|r| !r.id.is_empty());
    Ok((records, start > 0, unparsed))
}

fn rollout_path(path: &Path) -> PathBuf {
    if path.exists() {
        return path.into();
    }
    let compressed = PathBuf::from(format!("{}.zst", path.display()));
    if compressed.exists() {
        compressed
    } else {
        path.into()
    }
}

fn rollout_reader(path: &Path) -> Result<Box<dyn Read>, String> {
    no_links(path)?;
    let file = File::open(path).map_err(|e| format!("会话记录无法读取：{e}"))?;
    if path.extension().is_some_and(|e| e == "zst") {
        let reader = zstd::stream::read::Decoder::new(file).map_err(|e| e.to_string())?;
        Ok(Box::new(reader))
    } else {
        Ok(Box::new(file))
    }
}

fn validate_rollout(root: &Path, path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("会话索引未提供有效的记录文件路径，当前仅展示运行日志".into());
    }
    no_links(path)?;
    let canonical = fs::canonicalize(path).map_err(|e| format!("会话记录无法定位：{e}"))?;
    no_links(&canonical)?;
    let key = path_key(&canonical);
    if !["sessions", "archived_sessions"].iter().any(|name| {
        let dir = root.join(name);
        dir.is_dir() && key.starts_with(path_key(&dir))
    }) {
        return Err("会话记录不在当前 Codex 的 sessions 或 archived_sessions 目录中".into());
    }
    if !canonical.is_file() {
        return Err("会话记录不是普通文件".into());
    }
    Ok(())
}

fn rollout_meta(path: &Path) -> Result<Session, String> {
    let reader = rollout_reader(path)?;
    let mut reader = BufReader::new(reader.take(256 * 1024));
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    let obj: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    if obj["type"] != "session_meta" {
        return Err("缺少 session_meta".into());
    }
    let p = &obj["payload"];
    let string = |key: &str| p[key].as_str().unwrap_or("").to_string();
    let id = p["id"]
        .as_str()
        .or_else(|| p["session_id"].as_str())
        .ok_or("缺少会话 ID")?
        .to_string();
    Ok(Session {
        id,
        cwd: string("cwd"),
        provider: string("model_provider"),
        origin: string("originator"),
        rollout_path: path.to_string_lossy().into(),
        updated_at: super::modified_at(path),
        ..Default::default()
    })
}

fn newest_first(a: &Record, b: &Record) -> std::cmp::Ordering {
    b.time
        .cmp(&a.time)
        .then_with(|| a.location.source_id.cmp(&b.location.source_id))
        .then_with(|| b.sequence.cmp(&a.sequence))
        .then_with(|| a.id.cmp(&b.id))
}

fn mirror_records(mut records: Vec<Record>) -> (Vec<Record>, usize) {
    records.sort_by(|a, b| {
        (a.location.source_role != "runtime").cmp(&(b.location.source_role != "runtime"))
    });
    let mut out: Vec<Record> = Vec::new();
    let mut primary: HashMap<String, VecDeque<usize>> = HashMap::new();
    let mut mirrored = 0;
    for rec in records {
        let key = format!(
            "{}|{}|{}|{}|{}",
            rec.time,
            rec.target,
            rec.level,
            rec.thread_id.as_deref().unwrap_or(""),
            rec.message
        );
        if rec.location.source_role == "cli" {
            if let Some(index) = primary.get_mut(&key).and_then(VecDeque::pop_front) {
                out[index].mirrors.push(rec.location);
                mirrored += 1;
                continue;
            }
        }
        if rec.location.source_role == "runtime" {
            primary.entry(key).or_default().push_back(out.len());
        }
        out.push(rec);
    }
    out.sort_by(newest_first);
    (out, mirrored)
}

fn issue_kind(rec: &Record) -> Option<(&'static str, &'static str, &'static str)> {
    let lower = rec.message.to_lowercase();
    let bad_http = rec.http.as_ref().is_some_and(|h| !h.ok);
    if !bad_http
        && !matches!(rec.level.as_str(), "ERROR" | "WARN")
        && !matches!(rec.kind.as_str(), "retry" | "turn_error")
    {
        return None;
    }
    let status = rec.http.as_ref().and_then(|h| h.status);
    if status == Some(401) || lower.contains("unauthorized") {
        Some((
            "auth",
            "认证失败",
            "检查对应会话 provider 的凭据、登录状态和服务端返回的 request-id。",
        ))
    } else if status == Some(403) {
        Some(("auth", "访问被拒绝", "核对账号、模型权限和服务端访问规则。"))
    } else if status == Some(429) || lower.contains("rate limit") {
        Some((
            "rate_limit",
            "请求受到限流",
            "结合响应正文区分并发限制和额度不足，检查对应服务的限制。",
        ))
    } else if status.is_some_and(|s| s >= 500) {
        Some((
            "upstream",
            "上游服务返回错误",
            "使用 request-id 和时间对照网关或模型服务日志。",
        ))
    } else if rec.kind == "sse"
        || lower.contains("stream disconnected")
        || lower.contains("stream closed")
        || lower.contains("decoding response body")
    {
        Some((
            "stream",
            "响应流中断",
            "查看同一回合的请求、重试和结束事件，再检查网络、代理和上游 SSE 连接。",
        ))
    } else if rec.kind == "mcp" || rec.target.contains("mcp") {
        Some((
            "mcp",
            "MCP 服务异常",
            "检查记录中对应服务的启动命令、进程状态及握手错误。",
        ))
    } else if lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("is_timeout=true")
    {
        Some((
            "network",
            "请求或操作超时",
            "根据来源和目标模块定位超时发生在模型请求、工具还是桌面连接。",
        ))
    } else if rec.http.as_ref().is_some_and(|http| http.connect)
        || [
            "connection refused",
            "connection reset",
            "failed to lookup address",
            "dns error",
            "dns resolution",
            "tls handshake",
            "certificate verify",
            "proxy connect",
        ]
        .iter()
        .any(|text| lower.contains(text))
    {
        Some((
            "network",
            "网络连接失败",
            "根据原始错误核对 DNS、代理、证书或目标服务的连接状态，再查看同回合的重试与结束记录。",
        ))
    } else if lower.contains("permission denied") || lower.contains("access is denied") {
        Some((
            "tool",
            "工具或文件访问被拒绝",
            "查看具体文件路径、工具退出信息和该会话的审批结果。",
        ))
    } else if rec.kind == "retry" {
        Some((
            "retry",
            "模型请求发生重试",
            "重试本身不代表回合失败；查看同回合后续完成或失败记录。",
        ))
    } else if rec.kind == "turn_error" {
        Some((
            "turn",
            "Codex 回合失败",
            "打开会话时间线，查看该回合失败前的请求和工具记录。",
        ))
    } else if rec.location.source_role == "desktop" {
        Some((
            "desktop",
            "桌面应用告警",
            "结合桌面进程、连接和 app-server 记录排查；该告警不直接证明模型请求失败。",
        ))
    } else if super::is_benign(&rec.target) {
        Some((
            "background",
            "插件或后台功能告警",
            "核对对应插件或后台功能；结合回合记录判断是否影响当前任务。",
        ))
    } else {
        Some((
            "runtime",
            "Codex 运行告警",
            "查看来源模块和原始记录，并与同会话事件对照。",
        ))
    }
}

fn aggregate(snapshot: &mut Snapshot) {
    let mut sessions: BTreeMap<String, Session> = std::mem::take(&mut snapshot.sessions)
        .into_iter()
        .map(|s| (s.id.clone(), s))
        .collect();
    let mut routes: BTreeMap<String, Route> = BTreeMap::new();
    let mut issues: BTreeMap<String, Issue> = BTreeMap::new();
    for rec in &snapshot.records {
        snapshot.totals.warnings += usize::from(rec.level == "WARN");
        snapshot.totals.errors += usize::from(rec.level == "ERROR");
        snapshot.totals.retries += usize::from(rec.kind == "retry");
        snapshot.totals.turn_errors += usize::from(rec.kind == "turn_error");
        let session = rec.thread_id.as_ref().map(|id| {
            sessions.entry(id.clone()).or_insert_with(|| Session {
                id: id.clone(),
                ..Default::default()
            })
        });
        if let Some(session) = session {
            session.updated_at = session.updated_at.max(rec.time);
            if session.model.is_empty() {
                session.model = rec.model.clone().unwrap_or_default();
            }
            session.retries += usize::from(rec.kind == "retry");
            session.turn_errors += usize::from(rec.kind == "turn_error");
            if rec.kind == "request" {
                session.requests += 1;
                if let Some(http) = &rec.http {
                    session.failures += usize::from(!http.ok);
                    let address = format!("{}://{}", http.scheme, http.host);
                    if !session.hosts.contains(&address) {
                        session.hosts.push(address);
                    }
                }
            }
        }
        if rec.kind == "request" {
            if let Some(http) = &rec.http {
                snapshot.totals.requests += 1;
                snapshot.totals.ok += usize::from(http.ok);
                snapshot.totals.failed += usize::from(!http.ok);
                let address = format!("{}://{}", http.scheme, http.host);
                let route = routes.entry(address.clone()).or_insert_with(|| Route {
                    address,
                    ..Default::default()
                });
                route.requests += 1;
                route.ok += usize::from(http.ok);
                route.failed += usize::from(!http.ok);
                route.last_at = route.last_at.max(rec.time);
                *route
                    .statuses
                    .entry(
                        http.status
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "无响应".into()),
                    )
                    .or_default() += 1;
                if let Some(id) = &rec.thread_id {
                    route.session_ids.insert(id.clone());
                }
            }
        }
        if let Some((category, title, advice)) = issue_kind(rec) {
            let key = digest(&format!(
                "{category}|{}|{}",
                rec.target,
                super::signature_of(&rec.summary)
            ));
            let issue = issues.entry(key.clone()).or_insert_with(|| Issue {
                id: key,
                category: category.into(),
                title: title.into(),
                advice: advice.into(),
                severity: if rec.level == "ERROR"
                    || rec.kind == "turn_error"
                    || rec.http.as_ref().is_some_and(|h| !h.ok)
                {
                    "error"
                } else {
                    "warning"
                }
                .into(),
                first_at: rec.time,
                last_at: rec.time,
                sample: rec.summary.clone(),
                ..Default::default()
            });
            issue.count += 1;
            issue.first_at = issue.first_at.min(rec.time);
            issue.last_at = issue.last_at.max(rec.time);
            if issue.event_ids.len() < 5 {
                issue.event_ids.push(rec.id.clone());
            }
            if let Some(id) = &rec.thread_id {
                if !issue.threads.contains(id) {
                    issue.threads.push(id.clone());
                }
            }
        }
    }
    for session in sessions.values_mut() {
        session.route_status = if session.provider != snapshot.config.provider
            || session.hosts.is_empty()
            || snapshot.config.expected_host.is_empty()
        {
            "unverified"
        } else if session.hosts.iter().all(|h| {
            h == &format!(
                "{}://{}",
                snapshot.config.expected_scheme, snapshot.config.expected_host
            )
        }) {
            "matches"
        } else {
            "different"
        }
        .into();
    }
    snapshot.sessions = sessions.into_values().collect();
    snapshot.sessions.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    snapshot.totals.sessions = snapshot.sessions.len();
    snapshot.routes = routes.into_values().collect();
    snapshot
        .routes
        .sort_by_key(|r| std::cmp::Reverse(r.requests));
    snapshot.issues = issues.into_values().collect();
    snapshot.issues.sort_by(|a, b| {
        (a.category == "background")
            .cmp(&(b.category == "background"))
            .then_with(|| (b.severity == "error").cmp(&(a.severity == "error")))
            .then_with(|| b.count.cmp(&a.count))
    });
}

fn analyze(root: &Path, dirs: Vec<(PathBuf, &str)>, window: u64) -> Snapshot {
    let now = logging::now();
    let since = if window == 0 {
        0
    } else {
        now.saturating_sub(window.saturating_mul(1000))
    };
    let mut snapshot = Snapshot {
        id: uuid::Uuid::new_v4().to_string(),
        generated_at: now,
        window,
        since,
        codex_home: root.to_string_lossy().into(),
        config: read_config(root),
        sources: vec![],
        totals: Totals::default(),
        routes: vec![],
        sessions: vec![],
        issues: vec![],
        notes: vec![],
        partial: false,
        mirrored: 0,
        records: vec![],
    };
    let mut index_available = false;
    for (prefix, role) in [("logs_", "runtime"), ("state_", "index")] {
        let paths = db_candidates(root, prefix);
        if paths.is_empty() {
            let mut src = source(
                &setting_dir(root, "sqlite_home")
                    .unwrap_or_else(|| root.into())
                    .join(format!("{prefix}*.sqlite")),
                role,
                "SQLite",
            );
            src.note = "未发现；其他来源仍可分析".into();
            snapshot.sources.push(src);
        }
        let mut selected = false;
        for path in paths {
            let mut src = source(&path, role, "SQLite");
            if selected {
                src.status = "skipped".into();
                src.note = "旧版本数据库，避免与当前数据库重复统计".into();
            } else if role == "runtime" {
                match runtime_records(&path, &src, since, now) {
                    Ok((records, partial)) => {
                        src.records = records.len();
                        src.partial = partial;
                        snapshot.records.extend(records);
                        selected = true;
                    }
                    Err(error) => {
                        src.status = "error".into();
                        src.note = error;
                    }
                }
            } else {
                match index_sessions(&path, since) {
                    Ok((sessions, partial)) => {
                        src.records = sessions.len();
                        src.partial = partial;
                        snapshot.sessions = sessions;
                        selected = true;
                        index_available = true;
                    }
                    Err(error) => {
                        src.status = "error".into();
                        src.note = error;
                    }
                }
            }
            if src.partial {
                src.note = if role == "runtime" {
                    "最多读取最近 20,000 条非 TRACE 记录"
                } else {
                    "会话索引最多读取最近 500 个会话"
                }
                .into();
            }
            snapshot.sources.push(src);
        }
    }
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for (dir, role) in dirs {
        let (paths, limited) = walk_files(&dir, false);
        if limited {
            snapshot.partial = true;
            snapshot
                .notes
                .push(format!("{} 的目录扫描达到 20,000 项上限", dir.display()));
        }
        for path in paths {
            if super::modified_at(&path) >= since && seen.insert(path_key(&path)) {
                files.push((path, role.to_string()));
            }
        }
    }
    files.sort_by_key(|(p, _)| std::cmp::Reverse(super::modified_at(p)));
    if files.len() > MAX_FILES {
        snapshot.partial = true;
        snapshot.notes.push(format!(
            "发现 {} 份时间窗内文本日志，只读取最近 {MAX_FILES} 份",
            files.len()
        ));
    }
    for (path, role) in files.into_iter().take(MAX_FILES) {
        let mut src = source(&path, &role, "Text");
        match text_records(&path, &src, since, now) {
            Ok((records, partial, unparsed)) => {
                src.records = records.len();
                src.partial = partial;
                snapshot.records.extend(records);
                if partial {
                    src.note.push_str("已截取最近 512 KB；");
                }
                if unparsed > 0 {
                    src.note
                        .push_str(&format!("{unparsed} 行未识别，未推测其时间或级别；"));
                    src.partial = true;
                }
            }
            Err(error) => {
                src.status = "error".into();
                src.note = error;
            }
        }
        snapshot.sources.push(src);
    }
    if !snapshot
        .sources
        .iter()
        .any(|s| matches!(s.role.as_str(), "cli" | "desktop"))
    {
        let mut src = source(&root.join("log"), "cli", "Text");
        src.status = "missing".into();
        src.note = "所选时间窗内未发现 CLI 或桌面文本日志".into();
        snapshot.sources.push(src);
    }
    for dir_name in ["sessions", "archived_sessions"] {
        let dir = root.join(dir_name);
        let mut src = source(&dir, "rollout", "JSONL / Zstandard");
        src.note = "打开会话时解析回合、工具与用量；不读取聊天正文".into();
        if !index_available {
            let (mut paths, limited) = walk_files(&dir, true);
            paths.retain(|p| super::modified_at(p) >= since);
            paths.sort_by_key(|p| std::cmp::Reverse(super::modified_at(p)));
            src.partial = limited || paths.len() > MAX_SESSIONS;
            for path in paths.iter().take(MAX_SESSIONS) {
                match rollout_meta(path) {
                    Ok(mut session) => {
                        session.archived = dir_name == "archived_sessions";
                        snapshot.sessions.push(session);
                        src.records += 1;
                    }
                    Err(_) => {
                        src.partial = true;
                    }
                }
            }
            src.note = format!(
                "会话数据库不可读，从文件恢复 {} 个会话；详情按需解析{}",
                src.records,
                if src.partial {
                    "；部分文件未读取或格式不兼容"
                } else {
                    ""
                }
            );
        }
        snapshot.sources.push(src);
    }
    let (records, mirrored) = mirror_records(std::mem::take(&mut snapshot.records));
    snapshot.records = records;
    snapshot.mirrored = mirrored;
    if snapshot.records.len() > MAX_RECORDS {
        snapshot.partial = true;
        snapshot
            .notes
            .push("合并事件超过 20,000 条，只统计最新 20,000 条；请缩小时间范围".into());
        snapshot.records.truncate(MAX_RECORDS);
    }
    snapshot.partial |= snapshot
        .sources
        .iter()
        .any(|s| s.partial || s.status == "error");
    snapshot.notes.push("请求数仅统计实际记录到的模型 HTTP 响应或连接失败；HTTP 200 不代表整个回合完成。会话索引、工具调用和重试不作为模型请求累加。".into());
    snapshot.notes.push("配置对照使用当前全局 config.toml。不同地址可能来自旧会话、项目或 profile 覆盖；没有对应会话 provider 时不判断线路归属。".into());
    if mirrored > 0 {
        snapshot.notes.push(format!(
            "已合并 {mirrored} 条 SQLite 与 CLI 中时间、模块、线程和正文完全相同的镜像记录"
        ));
    }
    aggregate(&mut snapshot);
    snapshot
}

fn find_snapshot(id: Option<&str>) -> Result<Arc<Snapshot>, String> {
    let cache = SNAPSHOTS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "日志快照暂不可用")?;
    cache
        .iter()
        .find(|s| Some(s.id.as_str()) == id)
        .cloned()
        .ok_or_else(|| "分析快照已过期，请刷新后重试".into())
}

fn summary(snapshot: &Snapshot) -> Result<Value, String> {
    let mut result = serde_json::to_value(snapshot).map_err(|e| e.to_string())?;
    result["eventCount"] = json!(snapshot.records.len());
    result["recent"] = json!(snapshot
        .records
        .iter()
        .filter(|r| issue_kind(r).is_some())
        .take(20)
        .collect::<Vec<_>>());
    Ok(result)
}

fn matches_query(rec: &Record, query: &EventQuery, source_id: Option<&str>) -> bool {
    if source_id.is_some_and(|id| {
        rec.location.source_id != id
            && rec.location.source_role != id
            && !rec
                .mirrors
                .iter()
                .any(|m| m.source_id == id || m.source_role == id)
    }) {
        return false;
    }
    if query
        .thread_id
        .as_ref()
        .is_some_and(|v| rec.thread_id.as_ref() != Some(v))
    {
        return false;
    }
    if query
        .kinds
        .as_ref()
        .is_some_and(|v| !v.is_empty() && !v.contains(&rec.kind))
    {
        return false;
    }
    if query
        .levels
        .as_ref()
        .is_some_and(|v| !v.is_empty() && !v.contains(&rec.level))
    {
        return false;
    }
    if query.since.is_some_and(|v| rec.time < v) || query.until.is_some_and(|v| rec.time > v) {
        return false;
    }
    if query.outcome.as_deref() == Some("failed") && !rec.http.as_ref().is_some_and(|h| !h.ok) {
        return false;
    }
    if query.outcome.as_deref() == Some("ok") && !rec.http.as_ref().is_some_and(|h| h.ok) {
        return false;
    }
    if let Some(host) = query.host.as_deref() {
        if !rec.http.as_ref().is_some_and(|h| h.host == host) {
            return false;
        }
    }
    if let Some(search) = query.search.as_deref().filter(|s| !s.trim().is_empty()) {
        let haystack = format!(
            "{} {} {} {} {}",
            rec.message,
            rec.target,
            rec.thread_id.as_deref().unwrap_or(""),
            rec.http
                .as_ref()
                .and_then(|h| h.request_id.as_deref())
                .unwrap_or(""),
            rec.http.as_ref().map(|h| h.url.as_str()).unwrap_or("")
        )
        .to_lowercase();
        if !haystack.contains(&search.trim().to_lowercase()) {
            return false;
        }
    }
    true
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Usage {
    input: u64,
    cached_input: u64,
    output: u64,
    reasoning_output: u64,
    total: u64,
    context_window: Option<u64>,
    observed_at: u64,
}

fn usage(value: &Value, time: u64) -> Usage {
    Usage {
        input: value["input_tokens"].as_u64().unwrap_or(0),
        cached_input: value["cached_input_tokens"].as_u64().unwrap_or(0),
        output: value["output_tokens"].as_u64().unwrap_or(0),
        reasoning_output: value["reasoning_output_tokens"].as_u64().unwrap_or(0),
        total: value["total_tokens"].as_u64().unwrap_or(0),
        observed_at: time,
        ..Default::default()
    }
}

fn parse_rollout(
    obj: &Value,
    src: &Source,
    offset: u64,
    session_id: &str,
    current_turn: &mut Option<String>,
) -> Option<Record> {
    let time = timestamp(obj["timestamp"].as_str()?)?;
    let p = &obj["payload"];
    let typ = obj["type"].as_str()?;
    let event_type = p["type"].as_str().unwrap_or("");
    if let Some(turn_id) = p["turn_id"].as_str() {
        *current_turn = Some(turn_id.into());
    }
    let (kind, level, summary) = match (typ, event_type) {
        ("event_msg", "task_started" | "turn_started") => {
            *current_turn = p["turn_id"].as_str().map(str::to_string);
            ("turn_started", "INFO", "回合开始".to_string())
        }
        ("event_msg", "task_complete" | "task_completed" | "turn_completed") => {
            ("turn_completed", "INFO", "回合完成".into())
        }
        ("event_msg", "turn_aborted") => (
            "turn_aborted",
            "WARN",
            format!(
                "回合中止 · {}",
                p["reason"].as_str().unwrap_or("未记录原因")
            ),
        ),
        ("event_msg", "error") => (
            "turn_error",
            "ERROR",
            format!(
                "回合错误 · {}",
                tail(p["message"].as_str().unwrap_or("未记录原因"), 500)
            ),
        ),
        ("event_msg", "context_compacted") | ("compacted", _) => {
            ("compaction", "INFO", "上下文已压缩".into())
        }
        ("response_item", "function_call" | "custom_tool_call") => (
            "tool",
            "INFO",
            format!("调用工具 {}", p["name"].as_str().unwrap_or("unknown")),
        ),
        ("response_item", "function_call_output" | "custom_tool_call_output") => (
            "tool_result",
            "INFO",
            format!(
                "工具返回 · call_id {}",
                p["call_id"].as_str().unwrap_or("unknown")
            ),
        ),
        ("turn_context", _) => (
            "context",
            "INFO",
            format!(
                "回合配置 · 模型 {} · 推理 {}",
                p["model"].as_str().unwrap_or("未记录"),
                p["effort"].as_str().unwrap_or("默认")
            ),
        ),
        _ => return None,
    };
    let turn_id = p["turn_id"]
        .as_str()
        .map(str::to_string)
        .or_else(|| current_turn.clone());
    // Deliberately retain structural fields only, never prompts, tool arguments,
    // tool output, encrypted reasoning or assistant message contents.
    let message = json!({"recordType":typ,"eventType":event_type,"turnId":turn_id,"toolName":p["name"],"callId":p["call_id"],"reason":p["reason"],"model":p["model"],"summary":summary}).to_string();
    Some(Record {
        id: format!("{}:{offset}", src.id),
        time,
        level: level.into(),
        kind: kind.into(),
        target: format!("rollout::{typ}"),
        thread_id: Some(session_id.into()),
        turn_id,
        model: p["model"].as_str().map(str::to_string),
        summary: logging::redact(&summary),
        message: logging::redact(&message),
        sequence: offset,
        location: Location {
            source_id: src.id.clone(),
            source_role: "rollout".into(),
            path: src.path.clone(),
            position: format!(
                "{}偏移 {offset}",
                if src.format == "JSONL.zst" {
                    "解压后字节"
                } else {
                    "字节"
                }
            ),
        },
        ..Default::default()
    })
}

fn session_detail(snapshot: &Snapshot, id: &str) -> Result<Value, String> {
    let mut session = snapshot
        .sessions
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .ok_or("未找到该会话，请刷新分析")?;
    let mut records: Vec<Record> = snapshot
        .records
        .iter()
        .filter(|r| r.thread_id.as_deref() == Some(id))
        .cloned()
        .collect();
    let root = Path::new(&snapshot.codex_home);
    let path = rollout_path(Path::new(&session.rollout_path));
    let mut src = source(
        &path,
        "rollout",
        if path.extension().is_some_and(|e| e == "zst") {
            "JSONL.zst"
        } else {
            "JSONL"
        },
    );
    let mut token_usage: Option<Usage> = None;
    let mut tool_calls = 0;
    let mut started = 0;
    let mut completed = 0;
    let mut aborted = 0;
    let mut skipped = 0;
    let mut latest_new_usage = 0;
    let mut last_model = None;
    let result = (|| -> Result<(), String> {
        validate_rollout(root, &path)?;
        let compressed = path.extension().is_some_and(|e| e == "zst");
        let (reader, mut offset): (Box<dyn Read>, u64) = if compressed {
            (rollout_reader(&path)?, 0)
        } else {
            no_links(&path)?;
            let mut file = File::open(&path).map_err(|e| e.to_string())?;
            let len = file.metadata().map_err(|e| e.to_string())?.len();
            let start = len.saturating_sub(ROLLOUT_BYTES);
            file.seek(SeekFrom::Start(start))
                .map_err(|e| e.to_string())?;
            src.partial = start > 0;
            if src.partial {
                src.note = "大文件仅解析最近 32 MB；回合与工具计数为已读取部分".into();
            }
            (Box::new(file), start)
        };
        let mut reader = BufReader::new(reader.take(ROLLOUT_BYTES + 1));
        let mut line = Vec::new();
        if offset > 0 {
            offset += reader
                .read_until(b'\n', &mut line)
                .map_err(|e| e.to_string())? as u64;
        }
        let mut read_bytes = 0;
        let mut current_turn = None;
        loop {
            line.clear();
            let n = reader
                .read_until(b'\n', &mut line)
                .map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            read_bytes += n as u64;
            if read_bytes > ROLLOUT_BYTES {
                src.partial = true;
                if compressed {
                    src.note =
                        "解压数据超过 32 MB，仅解析前 32 MB；最新事件和用量可能尚未读到".into();
                }
                break;
            }
            let obj = match serde_json::from_slice::<Value>(&line) {
                Ok(value) => value,
                Err(_) => {
                    skipped += 1;
                    offset += n as u64;
                    continue;
                }
            };
            if let Some(time) = obj["timestamp"]
                .as_str()
                .and_then(timestamp)
                .filter(|t| *t <= snapshot.generated_at)
            {
                let p = &obj["payload"];
                if obj["type"] == "turn_context" {
                    if let Some(model) = p["model"].as_str() {
                        last_model = Some(model.to_string());
                    }
                }
                if obj["type"] == "token_usage_record"
                    && p["thread_token_usage"].is_object()
                    && time >= latest_new_usage
                {
                    token_usage = Some(usage(&p["thread_token_usage"], time));
                    latest_new_usage = time;
                } else if obj["type"] == "event_msg"
                    && p["type"] == "token_count"
                    && time > latest_new_usage
                {
                    if p["info"]["total_token_usage"].is_object() {
                        let mut u = usage(&p["info"]["total_token_usage"], time);
                        u.context_window = p["info"]["model_context_window"].as_u64();
                        token_usage = Some(u);
                    }
                }
                if let Some(mut rec) = parse_rollout(&obj, &src, offset, id, &mut current_turn) {
                    if rec.time >= snapshot.since {
                        started += usize::from(rec.kind == "turn_started");
                        completed += usize::from(rec.kind == "turn_completed");
                        aborted += usize::from(rec.kind == "turn_aborted");
                        tool_calls += usize::from(rec.kind == "tool");
                        // Session evidence remains bounded even for long tool-heavy runs.
                        rec.message = tail(&rec.message, 4096);
                        records.push(rec);
                        src.records += 1;
                        if records.len() > 4000 {
                            records.sort_by(newest_first);
                            records.truncate(2000);
                        }
                    }
                }
            }
            offset += n as u64;
        }
        Ok(())
    })();
    if let Err(error) = result {
        src.status = "error".into();
        src.note = error;
    }
    if skipped > 0 {
        src.partial = true;
        if !src.note.is_empty() {
            src.note.push('；');
        }
        src.note.push_str(&format!(
            "{skipped} 行 JSON 不完整或无法解析（可能仍在写入）"
        ));
    }
    if session.model.is_empty() {
        session.model = last_model.unwrap_or_default();
    }
    records.sort_by(newest_first);
    let has_more = records.len() > 1000 || src.records > 1000;
    records.truncate(1000);
    let event_details: BTreeMap<_, _> = records
        .iter()
        .filter(|r| r.location.source_role == "rollout")
        .map(|r| (r.id.clone(), r.message.clone()))
        .collect();
    Ok(
        json!({"session": session, "source": src, "usage": token_usage, "eventDetails":event_details,
        "turns": {"started":started,"completed":completed,"aborted":aborted}, "toolCalls":tool_calls,
        "events": records, "hasMore":has_more,
        "note":"回合与工具计数使用所选时间窗和已读取文件范围；用量取最后一条累计快照，输入已包含缓存输入，输出已包含推理输出，不重复相加。"}),
    )
}

fn sanitize(value: &mut Value, export: bool) {
    match value {
        Value::String(text) => {
            *text = if export {
                logging::export_redact(text)
            } else {
                logging::redact(text)
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| sanitize(item, export)),
        Value::Object(items) => items.values_mut().for_each(|item| sanitize(item, export)),
        _ => {}
    }
}

fn report(snapshot: &Snapshot, thread_id: Option<&str>) -> Result<Value, String> {
    let mut report = json!({"schemaVersion":2,"version":env!("CARGO_PKG_VERSION"),"analysis":summary(snapshot)?,"events":snapshot.records.iter().take(2000).collect::<Vec<_>>(),"exportedEvents":snapshot.records.len().min(2000),"note":"本地分析与最多 2,000 条事件摘要；读取范围和缺失来源详见 analysis.sources。不包含聊天正文、工具参数或输出。"});
    if let Some(id) = thread_id {
        report["sessionAnalysis"] = session_detail(snapshot, id)?;
    }
    sanitize(&mut report, true);
    Ok(report)
}

fn export(snapshot: &Snapshot, thread_id: Option<&str>) -> Result<Value, String> {
    let report = report(snapshot, thread_id)?;
    let dir = logging::data_path().join("reports");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    no_links(&dir)?;
    let path = dir.join(format!(
        "codex-analysis-{}-{}.json",
        snapshot.generated_at,
        uuid::Uuid::new_v4()
    ));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.write_all(
        serde_json::to_string_pretty(&report)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    Ok(json!({"path":path}))
}

pub(super) fn dispatch(request: &Request) -> Result<Value, String> {
    let mut value = if request.action == "analyze" {
        let root = crate::diagnostics::codex_home();
        let snapshot = Arc::new(analyze(
            &root,
            text_roots(&root),
            request.window.unwrap_or(86_400),
        ));
        let result = summary(&snapshot)?;
        let mut cache = SNAPSHOTS
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "日志快照暂不可用")?;
        cache.push_front(snapshot);
        cache.truncate(3);
        result
    } else {
        let snapshot = find_snapshot(request.snapshot_id.as_deref())?;
        match request.action.as_str() {
            "analysis-events" => {
                let default_query = EventQuery::default();
                let query = request.query.as_ref().unwrap_or(&default_query);
                let records: Vec<_> = snapshot
                    .records
                    .iter()
                    .filter(|r| matches_query(r, query, request.source_id.as_deref()))
                    .collect();
                let offset = request.offset.unwrap_or(0).min(records.len());
                let end =
                    (offset + query.limit.unwrap_or(200).clamp(1, 500) as usize).min(records.len());
                json!({"events":records[offset..end],"total":records.len(),"nextOffset":if end < records.len() { Some(end) } else { None },"snapshotId":snapshot.id})
            }
            "analysis-detail" => {
                let rec = snapshot
                    .records
                    .iter()
                    .find(|r| Some(r.id.as_str()) == request.event_id.as_deref())
                    .ok_or("事件不在当前分析快照内")?;
                json!({"event":rec,"message":rec.message,"span":rec.span,"note":"正文最多保留 8,192 字符，跨度上下文最多 4,096 字符。"})
            }
            "analysis-session" => session_detail(
                &snapshot,
                request.thread_id.as_deref().ok_or("缺少会话 ID")?,
            )?,
            "analysis-export" => export(&snapshot, request.thread_id.as_deref())?,
            _ => return Err("未知 Codex 分析操作".into()),
        }
    };
    sanitize(&mut value, false);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    const THREAD: &str = "01a44444-4444-7444-8444-444444444444";
    const TURN: &str = "01a55555-5555-7555-8555-555555555555";

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("cyapi-codex-analysis-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(root.join("sessions/2026/10/20")).unwrap();
            fs::create_dir_all(root.join("desktop/2026/10/20")).unwrap();
            fs::create_dir_all(root.join("log")).unwrap();
            fs::write(root.join("config.toml"), "model_provider='fixture'\nmodel='fixture-model'\n[model_providers.fixture]\nbase_url='https://gateway.example/v1'\n").unwrap();
            Self(root)
        }
        fn rollout(&self, compressed: bool) -> PathBuf {
            let path = self.0.join("sessions/2026/10/20/rollout-fixture.jsonl");
            let time = logging::now() - 5000;
            let stamp = chrono::DateTime::from_timestamp_millis(time as i64)
                .unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            let events = [
                json!({"type":"session_meta","payload":{"id":THREAD,"cwd":"F:/fixture/project","model_provider":"fixture","originator":"desktop"}}),
                json!({"type":"event_msg","payload":{"type":"task_started","turn_id":TURN}}),
                json!({"type":"turn_context","payload":{"turn_id":TURN,"model":"fixture-model","effort":"high"}}),
                json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"PRIVATE_PROMPT"}]}}),
                json!({"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"PRIVATE_ARGUMENTS","call_id":"fixture-call"}}),
                json!({"type":"response_item","payload":{"type":"function_call_output","output":"PRIVATE_OUTPUT","call_id":"fixture-call"}}),
                json!({"type":"token_usage_record","payload":{"thread_token_usage":{"input_tokens":100,"cached_input_tokens":60,"output_tokens":10,"reasoning_output_tokens":4,"total_tokens":110}}}),
                json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":60,"output_tokens":10,"reasoning_output_tokens":4,"total_tokens":110}}}}),
                json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":TURN}}),
            ];
            let mut text = String::new();
            for mut event in events {
                event["timestamp"] = json!(stamp);
                text.push_str(&event.to_string());
                text.push('\n');
            }
            text.push_str("{\"partial_write\":");
            if compressed {
                let zst = PathBuf::from(format!("{}.zst", path.display()));
                fs::write(&zst, zstd::stream::encode_all(text.as_bytes(), 1).unwrap()).unwrap();
            } else {
                fs::write(&path, text).unwrap();
            }
            path
        }
        fn state(&self, rollout: &Path) {
            let conn = Connection::open(self.0.join("state_5.sqlite")).unwrap();
            // Deliberately omit model, archived, and updated_at_ms: old schemas
            // must retain titles, providers and the rollout link.
            conn.execute_batch("CREATE TABLE threads(id TEXT, title TEXT, cwd TEXT, model_provider TEXT, source TEXT, rollout_path TEXT, updated_at INTEGER)").unwrap();
            conn.execute(
                "INSERT INTO threads VALUES(?,?,?,?,?,?,?)",
                params![
                    THREAD,
                    "验证模型请求与会话回合",
                    "F:/fixture/project",
                    "fixture",
                    "desktop",
                    rollout.to_string_lossy(),
                    (logging::now() / 1000) as i64
                ],
            )
            .unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert!(self.0.starts_with(std::env::temp_dir()));
            assert!(self
                .0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("cyapi-codex-analysis-"));
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn message(status: u16) -> String {
        format!("session_loop{{thread_id=\"{THREAD}\"}}:turn{{turn.id={TURN} model=fixture-model}}: Request completed method=POST url=https://gateway.example/v1/responses status={status} Response headers={{\"x-request-id\":\"fixture-request\"}}")
    }

    #[test]
    fn multi_source_analysis_reads_live_wal_deduplicates_and_correlates() {
        let f = Fixture::new();
        let rollout = f.rollout(false);
        f.state(&rollout);
        let time = (logging::now() / 1000 - 10) * 1000;
        let stamp = chrono::DateTime::from_timestamp_millis(time as i64)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let db_path = f.0.join("logs_2.sqlite");
        let writer = Connection::open(&db_path).unwrap();
        writer.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE logs(id INTEGER PRIMARY KEY,ts INTEGER,ts_nanos INTEGER,level TEXT,target TEXT,feedback_log_body TEXT,thread_id TEXT)").unwrap();
        writer
            .execute(
                "INSERT INTO logs VALUES(1,?,0,'DEBUG','codex_http_client::client',?,NULL)",
                params![(time / 1000) as i64, message(429)],
            )
            .unwrap();
        let text = message(429).replace(
            ": Request completed",
            ": codex_http_client::client: Request completed",
        );
        fs::write(
            f.0.join("log/codex-tui.log"),
            format!("{stamp} DEBUG {text}\n"),
        )
        .unwrap();
        fs::write(
            f.0.join("desktop/2026/10/20/main.log"),
            format!("{stamp} warn [StdioConnection] transport closed\n"),
        )
        .unwrap();
        let snapshot = analyze(
            &f.0,
            vec![(f.0.join("log"), "cli"), (f.0.join("desktop"), "desktop")],
            86400,
        );
        assert_eq!(snapshot.totals.requests, 1);
        assert_eq!(snapshot.totals.failed, 1);
        assert_eq!(snapshot.totals.ok, 0);
        assert_eq!(snapshot.mirrored, 1);
        assert!(snapshot
            .sources
            .iter()
            .any(|s| s.role == "desktop" && s.records == 1));
        assert!(snapshot
            .sources
            .iter()
            .any(|s| s.role == "runtime" && s.wal_size > 0));
        let session = snapshot.sessions.iter().find(|s| s.id == THREAD).unwrap();
        assert_eq!(session.title, "验证模型请求与会话回合");
        assert_eq!(session.provider, "fixture");
        assert_eq!(session.route_status, "matches");
        assert!(snapshot.issues.iter().any(|i| i.category == "rate_limit"));
        let detail = session_detail(&snapshot, THREAD).unwrap();
        assert_eq!(detail["turns"]["completed"], 1);
        assert_eq!(detail["toolCalls"], 1);
        let lifecycle: Vec<_> = detail["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["location"]["sourceRole"] == "rollout")
            .map(|e| e["kind"].as_str().unwrap())
            .collect();
        assert_eq!(
            lifecycle,
            vec![
                "turn_completed",
                "tool_result",
                "tool",
                "context",
                "turn_started"
            ]
        );
        assert_eq!(detail["usage"]["total"], 110);
        assert_eq!(detail["source"]["partial"], true);
        let serialized = detail.to_string();
        for secret in ["PRIVATE_PROMPT", "PRIVATE_ARGUMENTS", "PRIVATE_OUTPUT"] {
            assert!(!serialized.contains(secret));
        }
        // Optional synthetic fixture for browser verification, never real user data.
        if let Some(out) = std::env::var_os("CYAPI_ANALYSIS_UI_FIXTURE") {
            fs::write(out,json!({"analysis":summary(&snapshot).unwrap(),"events":snapshot.records,"session":detail}).to_string()).unwrap();
        }
        drop(writer);
    }

    #[test]
    fn snapshot_pagination_and_session_export_remain_consistent() {
        let f = Fixture::new();
        f.state(&f.rollout(false));
        let stamp = chrono::DateTime::from_timestamp_millis((logging::now() - 4000) as i64)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let line = format!(
            "{stamp} DEBUG codex_http_client::client: {}\n",
            message(429)
        );
        let log = f.0.join("log/codex-tui.log");
        fs::write(&log, line.repeat(205)).unwrap();
        let snapshot = Arc::new(analyze(&f.0, vec![(f.0.join("log"), "cli")], 86400));
        assert_eq!(snapshot.records.len(), 205);
        SNAPSHOTS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .push_front(snapshot.clone());
        // Later writes must not change page totals, ordering or evidence IDs.
        fs::write(&log, line.repeat(210)).unwrap();
        let page = |offset| {
            dispatch(
                &serde_json::from_value(json!({
                    "action":"analysis-events", "snapshotId":snapshot.id,
                    "sourceId":"cli", "offset":offset, "query":{"kinds":["request"]}
                }))
                .unwrap(),
            )
            .unwrap()
        };
        let first = page(0);
        let second = page(first["nextOffset"].as_u64().unwrap());
        assert_eq!(first["total"], 205);
        assert_eq!(second["total"], 205);
        assert_eq!(first["events"].as_array().unwrap().len(), 200);
        assert_eq!(second["events"].as_array().unwrap().len(), 5);
        assert!(second["nextOffset"].is_null());
        let ids: Vec<_> = first["events"]
            .as_array()
            .unwrap()
            .iter()
            .chain(second["events"].as_array().unwrap())
            .map(|e| e["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            snapshot
                .records
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>()
        );
        let report = report(&snapshot, Some(THREAD)).unwrap();
        assert_eq!(report["analysis"]["eventCount"], 205);
        assert_eq!(report["exportedEvents"], 205);
        assert_eq!(report["sessionAnalysis"]["usage"]["total"], 110);
        let serialized = report.to_string();
        for secret in ["PRIVATE_PROMPT", "PRIVATE_ARGUMENTS", "PRIVATE_OUTPUT"] {
            assert!(!serialized.contains(secret));
        }
        SNAPSHOTS
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .retain(|s| s.id != snapshot.id);
    }

    #[test]
    fn directory_aliases_do_not_duplicate_text_events() {
        let f = Fixture::new();
        let time = logging::now() - 4000;
        let stamp = chrono::DateTime::from_timestamp_millis(time as i64)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        fs::write(
            f.0.join("log/codex-tui.log"),
            format!(
                "{stamp} DEBUG codex_http_client::client: {}\n",
                message(429)
            ),
        )
        .unwrap();
        let snapshot = analyze(
            &f.0,
            vec![
                (f.0.join("log"), "cli"),
                (fs::canonicalize(f.0.join("log")).unwrap(), "cli"),
                (f.0.join("log/../log"), "cli"),
            ],
            86400,
        );
        assert_eq!(snapshot.totals.requests, 1);
        assert_eq!(snapshot.totals.failed, 1);
        assert_eq!(
            snapshot
                .sources
                .iter()
                .filter(|s| s.format == "Text")
                .count(),
            1
        );
    }

    #[test]
    fn resolved_rollout_paths_accept_verbatim_and_reject_parent_escape() {
        let f = Fixture::new();
        let rollout = f.rollout(false);
        // canonicalize produces the verbatim \\?\ path stored by Codex on Windows.
        f.state(&fs::canonicalize(&rollout).unwrap());
        let mut snapshot = analyze(&f.0, vec![], 86400);
        let detail = session_detail(&snapshot, THREAD).unwrap();
        assert_eq!(detail["source"]["status"], "available");
        assert_eq!(detail["usage"]["total"], 110);
        let external = f.0.join("outside.jsonl");
        fs::write(&external, "PRIVATE_OUTSIDE_CONTENT").unwrap();
        snapshot.sessions[0].rollout_path =
            f.0.join("sessions/../outside.jsonl")
                .to_string_lossy()
                .into();
        let detail = session_detail(&snapshot, THREAD).unwrap();
        assert_eq!(detail["source"]["status"], "error");
        assert!(detail["source"]["note"]
            .as_str()
            .unwrap()
            .contains("不在当前 Codex"));
        assert!(!detail.to_string().contains("PRIVATE_OUTSIDE_CONTENT"));
    }

    #[test]
    fn missing_or_incompatible_databases_do_not_block_text_or_compressed_sessions() {
        let f = Fixture::new();
        let rollout = f.rollout(true);
        let incompatible = Connection::open(f.0.join("logs_99.sqlite")).unwrap();
        incompatible
            .execute_batch("CREATE TABLE logs(unrecognized TEXT)")
            .unwrap();
        drop(incompatible);
        let time = logging::now() - 4000;
        let stamp = chrono::DateTime::from_timestamp_millis(time as i64)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        fs::write(
            f.0.join("log/codex-tui.log"),
            format!("{stamp} ERROR codex_core::session::turn: Turn error: fixture failure\n"),
        )
        .unwrap();
        let snapshot = analyze(&f.0, vec![(f.0.join("log"), "cli")], 86400);
        assert_eq!(snapshot.totals.turn_errors, 1);
        assert!(snapshot
            .sources
            .iter()
            .any(|s| s.role == "runtime" && s.status == "error"));
        assert!(snapshot.sessions.iter().any(|s| s.id == THREAD));
        assert!(snapshot.partial);
        let detail = session_detail(&snapshot, THREAD).unwrap();
        assert_eq!(detail["usage"]["total"], 110);
        assert_eq!(detail["turns"]["started"], 1);
        assert!(detail["source"]["path"]
            .as_str()
            .unwrap()
            .ends_with(".jsonl.zst"));
        assert!(rollout_path(&rollout).exists());
    }

    #[test]
    fn http_outcomes_and_non_model_requests_are_separate() {
        let src = source(Path::new("fixture.log"), "cli", "Text");
        for status in [200, 401, 403, 429, 502] {
            let rec = record(
                status.to_string(),
                1000,
                "DEBUG",
                "codex_http_client::client",
                &message(status),
                None,
                Location::default(),
            );
            assert_eq!(rec.kind, "request");
            assert_eq!(rec.http.as_ref().unwrap().ok, status == 200);
            assert_eq!(rec.thread_id.as_deref(), Some(THREAD));
            assert_eq!(rec.turn_id.as_deref(), Some(TURN));
        }
        let other = record(
            src.id,
            1000,
            "DEBUG",
            "codex_http_client::client",
            &message(200).replace("/v1/responses", "/marketplace"),
            None,
            Location::default(),
        );
        assert_eq!(other.kind, "http");
        let success_sse = record(
            "sse".into(),
            1000,
            "INFO",
            "codex_api::sse::responses",
            "response completed",
            None,
            Location::default(),
        );
        assert_ne!(success_sse.kind, "sse");
        let connect_failure = record(
            "connect".into(),
            1000,
            "DEBUG",
            "codex_http_client::client",
            "Request failed method=POST url=https://gateway.example/v1/responses is_timeout=false is_connect=true error=connection refused",
            None,
            Location::default(),
        );
        assert_eq!(connect_failure.kind, "request");
        assert!(!connect_failure.http.as_ref().unwrap().ok);
        assert_eq!(issue_kind(&connect_failure).unwrap().0, "network");
    }

    #[test]
    fn text_parsers_preserve_spans_and_do_not_invent_time() {
        let line = format!("\x1b[32m2026-10-20T10:00:00.123+08:00\x1b[0m WARN turn{{thread.id={THREAD} turn.id={TURN}}}: codex_core::responses_retry: retrying request");
        let (time, level, target, body) = parse_text(&line).unwrap();
        let rec = record(
            "text".into(),
            time,
            &level,
            &target,
            &body,
            None,
            Location::default(),
        );
        assert_eq!(rec.thread_id.as_deref(), Some(THREAD));
        assert_eq!(rec.kind, "retry");
        let obj = json!({"timestamp":"2026-10-20T02:00:00.123Z","level":"warn","target":"codex_core::responses_retry","fields":{"message":"retrying request"},"spans":[{"thread_id":THREAD,"turn_id":TURN}]}).to_string();
        let (_, _, _, body) = parse_text(&obj).unwrap();
        assert!(body.contains(THREAD));
        let obj = json!({"timestamp":"2026-10-20T02:00:00.123Z","level":"debug","target":"codex_http_client::client","fields":{"message":"Request completed","method":"POST","url":"https://gateway.example/v1/responses","status":401,"headers":{"x-request-id":"structured-id"}}}).to_string();
        let (time, level, target, body) = parse_text(&obj).unwrap();
        let rec = record(
            "json-http".into(),
            time,
            &level,
            &target,
            &body,
            None,
            Location::default(),
        );
        assert_eq!(rec.kind, "request");
        assert!(!rec.http.as_ref().unwrap().ok);
        assert_eq!(
            rec.http.as_ref().unwrap().request_id.as_deref(),
            Some("structured-id")
        );
        assert!(parse_text("warn no timestamp").is_none());
        assert!(parse_text("2026-10-20T10:00:00 INFO missing timezone").is_none());
    }

    #[test]
    fn filtering_covers_search_levels_thread_source_and_http_failure() {
        let rec = record(
            "id".into(),
            1000,
            "DEBUG",
            "codex_http_client::client",
            &message(401),
            None,
            Location {
                source_role: "runtime".into(),
                ..Default::default()
            },
        );
        let mut q = EventQuery {
            search: Some("fixture-request".into()),
            thread_id: Some(THREAD.into()),
            outcome: Some("failed".into()),
            ..Default::default()
        };
        assert!(matches_query(&rec, &q, Some("runtime")));
        assert!(!matches_query(&rec, &q, Some("desktop")));
        q.search = Some("%_literal".into());
        assert!(!matches_query(&rec, &q, None));
        q.search = None;
        q.levels = Some(vec!["WARN".into()]);
        assert!(!matches_query(&rec, &q, None));
        let mut v = json!({"body":"Authorization: Bearer fixture-secret-value","nested":["https://gateway.example/v1?api_key=fixture-secret"]});
        sanitize(&mut v, false);
        assert!(!v.to_string().contains("fixture-secret"));
    }

    #[test]
    fn window_is_applied_to_events_and_session_lifecycle() {
        let f = Fixture::new();
        let rollout = f.rollout(false);
        f.state(&rollout);
        let snapshot = analyze(&f.0, vec![], 1);
        let detail = session_detail(&snapshot, THREAD).unwrap();
        assert_eq!(detail["turns"]["started"], 0);
        assert_eq!(detail["toolCalls"], 0);
        assert_eq!(detail["usage"]["total"], 110); // A cumulative snapshot is not a window delta.
    }

    #[test]
    #[ignore = "manual read-only smoke test against the currently installed Codex"]
    fn local_codex_read_only_smoke() {
        let root = crate::diagnostics::codex_home();
        let snapshot = analyze(&root, text_roots(&root), 86400);
        println!(
            "events={} sessions={} requests={} failures={} partial={}",
            snapshot.records.len(),
            snapshot.sessions.len(),
            snapshot.totals.requests,
            snapshot.totals.failed,
            snapshot.partial
        );
        for src in &snapshot.sources {
            println!(
                "source={} format={} status={} records={} partial={} path={} note={}",
                src.role, src.format, src.status, src.records, src.partial, src.path, src.note
            );
        }
        assert!(snapshot
            .sources
            .iter()
            .any(|s| s.role == "runtime" && s.status == "available"));
        assert!(!snapshot.sessions.is_empty());
        let mut seen = HashSet::new();
        for src in snapshot.sources.iter().filter(|s| s.format == "Text") {
            assert!(
                seen.insert(path_key(Path::new(&src.path))),
                "duplicate physical source"
            );
        }
        let mut checked = 0;
        for selected in snapshot
            .sessions
            .iter()
            .filter(|s| !s.rollout_path.is_empty())
        {
            let detail = session_detail(&snapshot, &selected.id).unwrap();
            println!(
                "session: rollout={} events={} usage_present={} path={} note={}",
                detail["source"]["status"],
                detail["events"].as_array().unwrap().len(),
                !detail["usage"].is_null(),
                detail["source"]["path"],
                detail["source"]["note"]
            );
            assert_eq!(detail["source"]["status"], "available");
            checked += 1;
        }
        assert!(checked > 0, "no indexed local rollout files found");
    }
}
