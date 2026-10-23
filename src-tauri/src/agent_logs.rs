//! Application-specific, local-only Claude Code and OpenCode log analysis.
mod claude;
mod opencode;
mod runtime;
#[cfg(test)]
mod tests;

use crate::{configuration::no_links, diagnostics, logging};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
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

const MAX_EVENTS: usize = 20_000;
const MAX_FILES: usize = 200;
const JSON_BYTES: u64 = 8 * 1024 * 1024;
const TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_LINE: u64 = 2 * 1024 * 1024;

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Source {
    id: String,
    role: String,
    format: String,
    path: String,
    size: u64,
    wal_size: u64,
    records: usize,
    status: String,
    partial: bool,
    note: String,
}

#[derive(Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Location {
    source_id: String,
    path: String,
    position: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Tokens {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    cache_write_1h: u64,
    reasoning: u64,
}
impl Tokens {
    fn add(&mut self, other: &Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
        self.cache_write_1h = self.cache_write_1h.saturating_add(other.cache_write_1h);
        self.reasoning = self.reasoning.saturating_add(other.reasoning);
    }
    fn merge(&mut self, other: &Self) {
        let uncached = self
            .input
            .saturating_sub(self.cache_read)
            .saturating_sub(self.cache_write)
            .max(
                other
                    .input
                    .saturating_sub(other.cache_read)
                    .saturating_sub(other.cache_write),
            );
        let output = self
            .output
            .saturating_sub(self.reasoning)
            .max(other.output.saturating_sub(other.reasoning));
        self.cache_read = self.cache_read.max(other.cache_read);
        self.cache_write = self.cache_write.max(other.cache_write);
        self.cache_write_1h = self.cache_write_1h.max(other.cache_write_1h);
        self.reasoning = self.reasoning.max(other.reasoning);
        self.input = uncached
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write);
        self.output = output.saturating_add(self.reasoning);
    }
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Stats {
    responses: usize,
    response_errors: usize,
    api_errors: usize,
    retries: usize,
    tool_calls: usize,
    tool_errors: usize,
    compactions: usize,
    user_turns: usize,
    errors: usize,
    warnings: usize,
    usage_records: usize,
    tokens: Tokens,
}
impl Stats {
    fn observe(&mut self, e: &Event) {
        self.responses += usize::from(e.kind == "response");
        self.response_errors += usize::from(e.kind == "response" && e.failed);
        self.api_errors += usize::from(e.kind == "api_error");
        self.retries += usize::from(e.kind == "retry");
        self.tool_calls += usize::from(e.kind == "tool");
        self.tool_errors +=
            usize::from(matches!(e.kind.as_str(), "tool" | "tool_result") && e.failed);
        self.compactions += usize::from(e.kind == "compaction");
        self.user_turns += usize::from(e.kind == "user_turn");
        self.errors += usize::from(e.level == "ERROR");
        self.warnings += usize::from(e.level == "WARN");
        if let Some(tokens) = &e.usage {
            self.tokens.add(tokens);
            self.usage_records += 1;
        }
    }
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    id: String,
    title: String,
    project: String,
    model: String,
    provider: String,
    agent: String,
    parent_id: Option<String>,
    forked_from: Option<String>,
    updated_at: u64,
    stats: Stats,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    id: String,
    time: u64,
    level: String,
    kind: String,
    target: String,
    session_id: Option<String>,
    message_id: Option<String>,
    model: String,
    provider: String,
    summary: String,
    failed: bool,
    status: Option<u16>,
    request_id: Option<String>,
    duration_ms: Option<u64>,
    usage: Option<Tokens>,
    location: Location,
    mirrors: Vec<Location>,
    #[serde(skip)]
    evidence: Value,
    #[serde(skip)]
    category: String,
    #[serde(skip)]
    sequence: u64,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Issue {
    id: String,
    category: String,
    title: String,
    advice: String,
    count: usize,
    first_at: u64,
    last_at: u64,
    event_ids: Vec<String>,
    session_ids: Vec<String>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelStats {
    model: String,
    provider: String,
    stats: Stats,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    id: String,
    workspace: String,
    home: String,
    generated_at: u64,
    since: u64,
    window: u64,
    partial: bool,
    mirrored: usize,
    event_count: usize,
    totals: Stats,
    sessions: Vec<Session>,
    models: Vec<ModelStats>,
    sources: Vec<Source>,
    issues: Vec<Issue>,
    notes: Vec<String>,
    recent: Vec<Event>,
    #[serde(skip)]
    events: Vec<Event>,
}

struct Builder {
    snapshot: Snapshot,
    sessions: HashMap<String, Session>,
    events: HashMap<String, Event>,
    bytes_left: u64,
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))[..24].into()
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn short(value: &str) -> String {
    logging::redact(&value.chars().take(500).collect::<String>())
}
fn count(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0).min(1_000_000_000_000)
}
fn at(v: &Value) -> u64 {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .and_then(|d| d.timestamp_millis().try_into().ok())
        .or_else(|| {
            v.as_u64()
                .map(|n| if n < 100_000_000_000 { n * 1000 } else { n })
        })
        .unwrap_or(0)
}
fn modified(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|t| t.as_millis() as u64)
        .unwrap_or(0)
}
fn path_key(path: &Path) -> PathBuf {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.into());
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path
    }
}
fn source(path: &Path, role: &str, format: &str) -> Source {
    Source {
        id: digest(&path_key(path).to_string_lossy()),
        role: role.into(),
        format: format.into(),
        path: path.to_string_lossy().into(),
        size: fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        wal_size: fs::metadata(format!("{}-wal", path.display()))
            .map(|m| m.len())
            .unwrap_or(0),
        status: if path.exists() {
            "available"
        } else {
            "missing"
        }
        .into(),
        ..Default::default()
    }
}
fn locate(src: &Source, path: &Path, position: String) -> Location {
    Location {
        source_id: src.id.clone(),
        path: path.to_string_lossy().into(),
        position,
    }
}
fn open_db(path: &Path) -> Result<Connection, String> {
    no_links(path)?;
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    db.busy_timeout(Duration::from_secs(1))
        .map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA query_only=1; BEGIN;")
        .map_err(|e| e.to_string())?;
    Ok(db)
}
fn columns(db: &Connection, table: &str) -> HashSet<String> {
    let Ok(mut stmt) = db.prepare(&format!("PRAGMA table_info({table})")) else {
        return HashSet::new();
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) else {
        return HashSet::new();
    };
    rows.filter_map(Result::ok).collect()
}
fn col(cols: &HashSet<String>, name: &str, fallback: &str) -> String {
    if cols.contains(name) {
        format!("COALESCE({name},{fallback})")
    } else {
        fallback.into()
    }
}

/// Fixed application roots only; no recursive home-directory search or links.
fn files(root: &Path, extension: &str) -> (Vec<PathBuf>, bool) {
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut output = Vec::new();
    let mut visited = 0;
    let mut limited = false;
    while let Some((dir, depth)) = pending.pop() {
        if no_links(&dir).is_err() {
            limited = true;
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            limited |= dir.exists();
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            visited += 1;
            if visited > 20_000 {
                return (output, true);
            }
            let Ok(typ) = entry.file_type() else { continue };
            let path = entry.path();
            if typ.is_dir() && depth < 5 {
                pending.push((path, depth + 1));
            } else if typ.is_dir() {
                limited = true;
            } else if typ.is_file() && path.extension().is_some_and(|e| e == extension) {
                output.push(path);
            }
        }
    }
    (output, limited)
}

/// Read a bounded tail; skip incomplete/oversized records without retaining contents.
fn json_lines(
    path: &Path,
    limit: u64,
    mut read: impl FnMut(Value, u64),
) -> Result<(u64, bool, usize), String> {
    no_links(path)?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let start = len.saturating_sub(limit);
    file.seek(SeekFrom::Start(start))
        .map_err(|e| e.to_string())?;
    let mut input = BufReader::new(file.take(limit));
    let mut offset = start;
    if start > 0 {
        offset += skip_line(&mut input)?;
    }
    let mut malformed = 0;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = (&mut input)
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut buf)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let position = offset;
        offset += n as u64;
        if n as u64 > MAX_LINE {
            if !buf.ends_with(b"\n") {
                offset += skip_line(&mut input)?;
            }
            malformed += 1;
            continue;
        }
        let line = String::from_utf8_lossy(&buf);
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line.trim_start_matches('\u{feff}')) {
            Ok(value) => read(value, position),
            Err(_) => malformed += 1,
        }
    }
    Ok((len.min(limit), start > 0, malformed))
}
fn skip_line(reader: &mut impl BufRead) -> Result<u64, String> {
    let mut skipped = 0;
    loop {
        let buf = reader.fill_buf().map_err(|e| e.to_string())?;
        if buf.is_empty() {
            break;
        }
        let newline = buf.iter().position(|b| *b == b'\n');
        let n = newline.map_or(buf.len(), |n| n + 1);
        skipped += n as u64;
        reader.consume(n);
        if newline.is_some() {
            break;
        }
    }
    Ok(skipped)
}
fn read_json(path: &Path) -> Result<Value, String> {
    no_links(path)?;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_LINE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_LINE {
        return Err("JSON 超过 2 MB，已跳过".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "JSON 无效或仍在写入".into())
}

fn roots(workspace: &str) -> Result<PathBuf, String> {
    let isolated = std::env::var_os("CYAPI_TOOLBOX_HOME_DIR").is_some();
    let override_dir = |key: &str| {
        std::env::var_os(key)
            .filter(|_| !isolated)
            .map(PathBuf::from)
    };
    let path = match workspace {
        "claude" => override_dir("CLAUDE_CONFIG_DIR")
            .unwrap_or_else(|| diagnostics::home_path().join(".claude")),
        "opencode" => override_dir("XDG_DATA_HOME")
            .unwrap_or_else(|| diagnostics::home_path().join(".local/share"))
            .join("opencode"),
        _ => return Err("不支持的日志工作区".into()),
    };
    if !path.is_absolute() {
        return Err("日志目录必须是绝对路径".into());
    }
    Ok(path)
}

impl Builder {
    fn new(workspace: &str, root: &Path, window: u64) -> Self {
        let now = logging::now();
        Self {
            snapshot: Snapshot {
                id: uuid::Uuid::new_v4().to_string(),
                workspace: workspace.into(),
                home: root.to_string_lossy().into(),
                generated_at: now,
                since: if window == 0 {
                    0
                } else {
                    now.saturating_sub(window.saturating_mul(1000))
                },
                window,
                partial: false,
                mirrored: 0,
                event_count: 0,
                totals: Stats::default(),
                sessions: vec![],
                models: vec![],
                sources: vec![],
                issues: vec![],
                notes: vec![],
                recent: vec![],
                events: vec![],
            },
            sessions: HashMap::new(),
            events: HashMap::new(),
            bytes_left: TOTAL_BYTES,
        }
    }
    fn session(&mut self, id: &str) -> &mut Session {
        self.sessions.entry(id.into()).or_insert_with(|| Session {
            id: id.into(),
            ..Default::default()
        })
    }
    fn add(&mut self, key: String, mut event: Event) {
        if event.time == 0
            || event.time < self.snapshot.since
            || event.time > self.snapshot.generated_at
        {
            return;
        }
        event.id = digest(&format!("{}:{key}", self.snapshot.workspace));
        if let Some(old) = self.events.get_mut(&key) {
            self.snapshot.mirrored += 1;
            let mut mirrors = old.mirrors.clone();
            if old.location != event.location {
                mirrors.push(old.location.clone());
            }
            if let Some(previous) = &old.usage {
                if let Some(usage) = &mut event.usage {
                    usage.merge(previous);
                } else {
                    event.usage = Some(previous.clone());
                }
            }
            if old.time > event.time {
                if let Some(usage) = &event.usage {
                    if let Some(old_usage) = &mut old.usage {
                        old_usage.merge(usage);
                    }
                }
                if !old.mirrors.contains(&event.location) {
                    old.mirrors.push(event.location);
                }
                return;
            }
            mirrors.retain(|m| m != &event.location);
            mirrors.sort_by(|a, b| (&a.path, &a.position).cmp(&(&b.path, &b.position)));
            mirrors.dedup();
            event.mirrors = mirrors;
        }
        self.events.insert(key, event);
        if self.events.len() > MAX_EVENTS * 2 {
            let mut events: Vec<_> = std::mem::take(&mut self.events).into_iter().collect();
            events.sort_by(|a, b| newest(&a.1, &b.1));
            events.truncate(MAX_EVENTS);
            self.events = events.into_iter().collect();
            self.snapshot.partial = true;
        }
    }
    fn finish(mut self) -> Snapshot {
        let mut events: Vec<_> = self.events.into_values().collect();
        events.sort_by(newest);
        if events.len() > MAX_EVENTS {
            self.snapshot.partial = true;
            self.snapshot
                .notes
                .push("合并事件超过 20,000 条，仅统计最新部分；请缩小时间范围。".into());
            events.truncate(MAX_EVENTS);
        }
        let mut models: BTreeMap<(String, String), ModelStats> = BTreeMap::new();
        let mut issues: BTreeMap<String, Issue> = BTreeMap::new();
        for event in &events {
            self.snapshot.totals.observe(event);
            if let Some(id) = &event.session_id {
                let session = self.sessions.entry(id.clone()).or_insert_with(|| Session {
                    id: id.clone(),
                    ..Default::default()
                });
                session.updated_at = session.updated_at.max(event.time);
                session.stats.observe(event);
                if session.model.is_empty() {
                    session.model = event.model.clone();
                }
                if session.provider.is_empty() {
                    session.provider = event.provider.clone();
                }
            }
            if event.usage.is_some() || event.kind == "response" {
                let model = models
                    .entry((event.model.clone(), event.provider.clone()))
                    .or_insert_with(|| ModelStats {
                        model: event.model.clone(),
                        provider: event.provider.clone(),
                        ..Default::default()
                    });
                model.stats.observe(event);
            }
            if event.failed
                || matches!(event.level.as_str(), "ERROR" | "WARN")
                || event.kind == "retry"
            {
                let category = if event.category.is_empty() {
                    if event.kind.starts_with("tool") {
                        "tool"
                    } else {
                        "runtime"
                    }
                } else {
                    &event.category
                };
                let key = format!("{category}:{}:{}", event.target, event.status.unwrap_or(0));
                let (title, advice) = advice(category);
                let issue = issues.entry(key.clone()).or_insert_with(|| Issue {
                    id: digest(&key),
                    category: category.into(),
                    title: title.into(),
                    advice: advice.into(),
                    first_at: event.time,
                    last_at: event.time,
                    ..Default::default()
                });
                issue.count += 1;
                issue.first_at = issue.first_at.min(event.time);
                issue.last_at = issue.last_at.max(event.time);
                if issue.event_ids.len() < 5 {
                    issue.event_ids.push(event.id.clone());
                }
                if let Some(id) = &event.session_id {
                    if !issue.session_ids.contains(id) {
                        issue.session_ids.push(id.clone());
                    }
                }
            }
        }
        self.snapshot.sessions = self
            .sessions
            .into_values()
            .filter(|s| {
                s.updated_at >= self.snapshot.since && s.updated_at <= self.snapshot.generated_at
            })
            .collect();
        self.snapshot.sessions.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        self.snapshot.models = models.into_values().collect();
        self.snapshot.issues = issues.into_values().collect();
        self.snapshot
            .issues
            .sort_by_key(|i| std::cmp::Reverse(i.count));
        self.snapshot.partial |= self
            .snapshot
            .sources
            .iter()
            .any(|s| s.partial || s.status == "error");
        self.snapshot.event_count = events.len();
        self.snapshot.recent = events
            .iter()
            .filter(|e| {
                e.failed || matches!(e.level.as_str(), "ERROR" | "WARN") || e.kind == "retry"
            })
            .take(20)
            .cloned()
            .collect();
        self.snapshot.events = events;
        self.snapshot.notes.push("用量为所选时间窗内已读取模型响应的合计，主会话和子代理分别列出；缓存和推理子项不再重复加到总量。缺失用量不按请求次数估算。".into());
        self.snapshot.notes.push("会话与运行日志只保留结构字段，不返回聊天正文、隐藏推理、工具参数或输出；会话响应数与实际 HTTP 尝试数分别理解。".into());
        self.snapshot
    }
}
fn newest(a: &Event, b: &Event) -> std::cmp::Ordering {
    b.time
        .cmp(&a.time)
        .then_with(|| a.location.source_id.cmp(&b.location.source_id))
        .then_with(|| b.sequence.cmp(&a.sequence))
        .then_with(|| a.id.cmp(&b.id))
}
fn advice(category: &str) -> (&'static str, &'static str) {
    match category {
        "auth" => (
            "认证或访问被拒绝",
            "核对该响应对应的服务、账号权限和 request-id。",
        ),
        "rate_limit" => (
            "限流或额度限制",
            "区分并发限制与额度不足，结合重试及后续响应判断是否恢复。",
        ),
        "upstream" => (
            "上游服务错误",
            "用状态码、时间和 request-id 对照模型服务日志。",
        ),
        "network" => (
            "连接、超时或响应流异常",
            "结合错误类型核对 DNS、代理、证书和连接状态。",
        ),
        "tool" => (
            "工具执行失败",
            "沿调用 ID 对照工具状态与所属会话；工具失败不等于模型请求失败。",
        ),
        "mcp" => ("MCP 服务异常", "检查对应 MCP 服务的启动、连接与握手记录。"),
        "retry" => (
            "请求发生重试",
            "重试不代表整个会话失败，查看同会话后续响应。",
        ),
        "compaction" => (
            "上下文压缩异常",
            "查看压缩状态及后续响应；压缩的 token 单独计入已记录用量。",
        ),
        "api" => (
            "模型响应或 API 错误",
            "根据错误类型与服务端状态核对该次响应。",
        ),
        _ => (
            "运行告警",
            "查看来源、服务模块和结构证据，确认是否影响当前会话。",
        ),
    }
}
fn error_category(status: Option<u16>, text: &str) -> &'static str {
    let lower = text.to_lowercase();
    if matches!(status, Some(401 | 403))
        || lower.contains("authentication")
        || lower.contains("unauthorized")
    {
        "auth"
    } else if status == Some(429)
        || lower.contains("rate_limit")
        || lower.contains("rate limit")
        || lower.contains("quota")
    {
        "rate_limit"
    } else if status.is_some_and(|s| s >= 500) || lower.contains("overloaded") {
        "upstream"
    } else if [
        "timeout",
        "timed out",
        "econn",
        "enotfound",
        "connection",
        "socket",
        "network",
        "tls",
        "stream",
    ]
    .iter()
    .any(|s| lower.contains(s))
    {
        "network"
    } else {
        "api"
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    action: String,
    workspace: String,
    window: Option<u64>,
    snapshot_id: Option<String>,
    session_id: Option<String>,
    event_id: Option<String>,
    offset: Option<usize>,
    query: Option<Query>,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Query {
    search: Option<String>,
    session_id: Option<String>,
    source_id: Option<String>,
    kind: Option<String>,
    level: Option<String>,
    failed: Option<bool>,
}
static CACHE: OnceLock<Mutex<VecDeque<Arc<Snapshot>>>> = OnceLock::new();
fn matches(e: &Event, q: &Query) -> bool {
    if q.session_id
        .as_ref()
        .is_some_and(|id| e.session_id.as_ref() != Some(id))
        || q.kind.as_ref().is_some_and(|k| k != &e.kind)
        || q.level.as_ref().is_some_and(|l| l != &e.level)
        || q.failed == Some(true) && !e.failed
    {
        return false;
    }
    if q.source_id.as_ref().is_some_and(|id| {
        &e.location.source_id != id && !e.mirrors.iter().any(|m| &m.source_id == id)
    }) {
        return false;
    }
    !q.search.as_ref().is_some_and(|s| {
        !format!(
            "{} {} {} {} {} {} {} {}",
            e.summary,
            e.target,
            e.session_id.as_deref().unwrap_or(""),
            e.message_id.as_deref().unwrap_or(""),
            e.request_id.as_deref().unwrap_or(""),
            e.model,
            e.provider,
            e.evidence
        )
        .to_lowercase()
        .contains(&s.trim().to_lowercase())
    })
}
fn session_detail(snapshot: &Snapshot, id: &str) -> Result<Value, String> {
    let session = snapshot
        .sessions
        .iter()
        .find(|s| s.id == id)
        .ok_or("会话不在当前快照中，请刷新后重试")?;
    let events: Vec<_> = snapshot
        .events
        .iter()
        .filter(|e| e.session_id.as_deref() == Some(id))
        .take(1001)
        .collect();
    Ok(
        json!({"session":session,"events":events.iter().take(1000).collect::<Vec<_>>(),"hasMore":events.len()>1000,
      "children":snapshot.sessions.iter().filter(|s|s.parent_id.as_deref()==Some(id)).collect::<Vec<_>>(),
      "note":"用量与计数只覆盖当前时间窗和已读取记录；未记录结束状态时不判断任务仍在运行。"}),
    )
}
fn sanitize(value: &mut Value, export: bool) {
    match value {
        Value::String(s) => {
            *s = if export {
                logging::export_redact(s)
            } else {
                logging::redact(s)
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|v| sanitize(v, export)),
        Value::Object(o) => o.values_mut().for_each(|v| sanitize(v, export)),
        _ => {}
    }
}
fn report(snapshot: &Snapshot, session: Option<&str>) -> Result<Value, String> {
    let mut result = json!({"schemaVersion":1,"analysis":snapshot,"events":snapshot.events.iter().filter(|e|session.is_none()||e.session_id.as_deref()==session).take(2000).collect::<Vec<_>>(),"note":"最多导出 2,000 条结构事件，读取范围见 analysis.sources。"});
    if let Some(id) = session {
        result["sessionAnalysis"] = session_detail(snapshot, id)?;
    }
    sanitize(&mut result, true);
    Ok(result)
}
fn dispatch(request: Request) -> Result<Value, String> {
    if !matches!(request.workspace.as_str(), "claude" | "opencode") {
        return Err("不支持的日志工作区".into());
    }
    let mut result = if request.action == "analyze" {
        let root = roots(&request.workspace)?;
        let mut builder = Builder::new(&request.workspace, &root, request.window.unwrap_or(86400));
        if request.workspace == "claude" {
            claude::analyze(&root, &mut builder);
        } else {
            opencode::analyze(&root, &mut builder);
        }
        let snapshot = Arc::new(builder.finish());
        let value = serde_json::to_value(&*snapshot).map_err(|e| e.to_string())?;
        let mut cache = CACHE
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "日志快照暂不可用")?;
        cache.push_front(snapshot);
        let mut counts = HashMap::new();
        cache.retain(|s| {
            let n = counts.entry(s.workspace.clone()).or_insert(0);
            *n += 1;
            *n <= 3
        });
        value
    } else {
        let snapshot = CACHE
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "日志快照暂不可用")?
            .iter()
            .find(|s| {
                Some(&s.id) == request.snapshot_id.as_ref() && s.workspace == request.workspace
            })
            .cloned()
            .ok_or("日志快照已过期，请刷新后重试")?;
        match request.action.as_str() {
            "events" => {
                let query = request.query.unwrap_or_default();
                let events: Vec<_> = snapshot
                    .events
                    .iter()
                    .filter(|e| matches(e, &query))
                    .collect();
                let start = request.offset.unwrap_or(0).min(events.len());
                let end = (start + 200).min(events.len());
                json!({"events":events[start..end],"total":events.len(),"nextOffset":if end<events.len(){Some(end)}else{None},"snapshotId":snapshot.id})
            }
            "detail" => {
                let event = snapshot
                    .events
                    .iter()
                    .find(|e| Some(&e.id) == request.event_id.as_ref())
                    .ok_or("事件不在当前快照中")?;
                json!({"event":event,"evidence":event.evidence,"note":"只展示结构字段；聊天正文、工具参数与输出未载入分析结果。"})
            }
            "session" => session_detail(
                &snapshot,
                request.session_id.as_deref().ok_or("缺少会话 ID")?,
            )?,
            "export" => {
                let value = report(&snapshot, request.session_id.as_deref())?;
                let dir = logging::data_path().join("reports");
                no_links(&dir)?;
                fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let path = dir.join(format!(
                    "{}-analysis-{}.json",
                    request.workspace,
                    uuid::Uuid::new_v4()
                ));
                let mut file = fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&path)
                    .map_err(|e| e.to_string())?;
                file.write_all(
                    serde_json::to_string_pretty(&value)
                        .map_err(|e| e.to_string())?
                        .as_bytes(),
                )
                .map_err(|e| e.to_string())?;
                json!({"path":path})
            }
            _ => return Err("未知日志分析操作".into()),
        }
    };
    sanitize(&mut result, false);
    Ok(result)
}
#[tauri::command]
pub async fn agent_logs(request: Request) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || dispatch(request))
        .await
        .map_err(|e| logging::redact(&e.to_string()))?
        .map_err(|e| logging::redact(&e))
}
