//! Local session usage, following CC Switch's session parsers (see THIRD-PARTY-NOTICES).
//! No proxy, credentials, prompts, or network calls are involved.
mod parsers;

use crate::{configuration::no_links, diagnostics};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_FILES: usize = 20_000;
const MAX_RECORDS: usize = 200_000;
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Record {
    id: String,
    session_id: String,
    model: String,
    project: String,
    time: i64,
    // Input includes cache reads/writes. Output includes reasoning; both are subsets.
    input: u64,
    output: u64,
    cached_input: u64,
    cache_write: u64,
    cache_write_1h: u64,
    reasoning_output: u64,
    reported_cost: Option<f64>,
    #[serde(skip)]
    signature: String,
    #[serde(skip)]
    total_snapshot: Option<[u64; 6]>,
    #[serde(skip)]
    exact: bool,
}

impl Record {
    fn nonzero(&self) -> bool {
        self.input > 0 || self.output > 0
    }
    fn merge(&mut self, other: &Self) {
        self.input = self.input.max(other.input);
        self.output = self.output.max(other.output);
        self.cached_input = self.cached_input.max(other.cached_input);
        self.cache_write = self.cache_write.max(other.cache_write);
        self.cache_write_1h = self.cache_write_1h.max(other.cache_write_1h);
        self.reasoning_output = self.reasoning_output.max(other.reasoning_output);
        self.reported_cost = other.reported_cost.or(self.reported_cost);
        if self.model == "unknown" {
            self.model.clone_from(&other.model);
        }
        if self.project.is_empty() {
            self.project.clone_from(&other.project);
        }
    }
}

#[derive(Clone, Default)]
struct Parsed {
    session_id: String,
    parent_id: Option<String>,
    fork_at: i64,
    records: Vec<Record>,
    warnings: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Source {
    path: String,
    format: String,
    files: usize,
    cached_files: usize,
    records: usize,
    status: String,
    notes: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    workspace: String,
    generated_at: u64,
    records: Vec<Record>,
    sources: Vec<Source>,
    duplicates: usize,
    partial: bool,
    notes: Vec<String>,
}

struct Cached {
    length: u64,
    modified: Option<SystemTime>,
    parsed: Parsed,
}
static CACHE: OnceLock<Mutex<HashMap<PathBuf, Cached>>> = OnceLock::new();

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))[..24].into()
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn roots(workspace: &str) -> Result<Vec<(PathBuf, &'static str)>, String> {
    let home = diagnostics::home_path();
    let isolated = env::var_os("CYAPI_TOOLBOX_HOME_DIR").is_some();
    let override_path = |key: &str| {
        env::var_os(key)
            .filter(|_| !isolated)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    match workspace {
        "codex" => {
            let root = diagnostics::codex_home();
            Ok(vec![
                (root.join("sessions"), "codex"),
                (root.join("archived_sessions"), "codex"),
            ])
        }
        "claude" => Ok(vec![(
            override_path("CLAUDE_CONFIG_DIR")
                .unwrap_or_else(|| home.join(".claude"))
                .join("projects"),
            "claude",
        )]),
        "opencode" => {
            let root = override_path("XDG_DATA_HOME")
                .unwrap_or_else(|| home.join(".local/share"))
                .join("opencode");
            // SQLite supersedes legacy JSON storage; importing both counts migrated history twice.
            if root.join("opencode.db").exists() {
                Ok(vec![(root.join("opencode.db"), "sqlite")])
            } else {
                Ok(vec![(root.join("storage/message"), "opencode")])
            }
        }
        _ => Err("不支持的用量统计工作区".into()),
    }
}

fn collect(
    dir: &Path,
    extension: &str,
    depth: usize,
    files: &mut Vec<PathBuf>,
    notes: &mut Vec<String>,
) {
    if depth > 10 {
        notes.push("子目录超过 10 层，已跳过".into());
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => {
            notes.push(format!("无法读取目录 {}", dir.display()));
            return;
        }
    };
    for entry in entries {
        if files.len() >= MAX_FILES {
            notes.push(format!("文件超过 {MAX_FILES} 份，已停止扫描"));
            break;
        }
        let Ok(entry) = entry else {
            notes.push("部分目录项无法读取".into());
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            notes.push("部分文件属性无法读取".into());
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            collect(&path, extension, depth + 1, files, notes);
        } else if kind.is_file()
            && (path.extension().is_some_and(|e| e == extension)
                || (extension == "jsonl"
                    && path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().ends_with(".jsonl.zst"))))
        {
            files.push(path);
        }
    }
}

fn cached_file(
    path: &Path,
    format: &str,
    refresh: bool,
    cache: &mut HashMap<PathBuf, Cached>,
) -> Result<(Parsed, bool), String> {
    no_links(path)?;
    let meta = fs::metadata(path).map_err(|_| "无法读取文件属性")?;
    if meta.len() > MAX_FILE_BYTES {
        return Err("文件超过 512 MB，已跳过".into());
    }
    if !refresh {
        if let Some(cached) = cache
            .get(path)
            .filter(|c| c.length == meta.len() && c.modified == meta.modified().ok())
        {
            return Ok((cached.parsed.clone(), true));
        }
    }
    let parsed = match format {
        "codex" | "claude" => parsers::jsonl(path, format)?,
        _ => parsers::opencode_file(path)?,
    };
    cache.insert(
        path.into(),
        Cached {
            length: meta.len(),
            modified: meta.modified().ok(),
            parsed: parsed.clone(),
        },
    );
    Ok((parsed, false))
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |r| r.get::<_, bool>(0),
    )
    .unwrap_or(false)
}

fn opencode_database(path: &Path) -> Result<Parsed, String> {
    no_links(path)?;
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| "无法只读打开 OpenCode 数据库")?;
    // Keep WAL visibility: immutable=1 would silently miss live usage.
    conn.busy_timeout(Duration::from_secs(2))
        .map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA query_only=1; BEGIN;")
        .map_err(|e| e.to_string())?;
    let v2 = table_exists(&conn, "session_message");
    let sql = if v2 {
        "SELECT id, session_id, time_created, data, type FROM session_message WHERE type IN ('assistant','compaction') ORDER BY time_created DESC LIMIT ?1"
    } else if table_exists(&conn, "message") {
        "SELECT id, session_id, time_created, data, 'assistant' FROM message WHERE json_valid(data) AND json_extract(data,'$.role')='assistant' ORDER BY time_created DESC LIMIT ?1"
    } else {
        return Err("未找到支持的 OpenCode 消息表".into());
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|_| "OpenCode 数据库结构暂不支持")?;
    let rows = stmt
        .query_map([MAX_RECORDS as i64 + 1], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(|_| "无法查询 OpenCode 用量")?;
    let mut parsed = Parsed::default();
    for (index, row) in rows.enumerate() {
        if index >= MAX_RECORDS {
            parsed
                .warnings
                .push(format!("消息超过 {MAX_RECORDS} 条，仅读取最新部分"));
            break;
        }
        let (id, session, time, data, kind) = row.map_err(|_| "无法读取 OpenCode 消息")?;
        if data.len() > parsers::MAX_LINE {
            parsed.warnings.push("跳过一条过大的消息".into());
            continue;
        }
        match serde_json::from_str(&data) {
            Ok(value) => {
                if let Some(record) = parsers::opencode(&value, &id, &session, time, &kind) {
                    parsed.records.push(record);
                }
            }
            Err(_) => parsed.warnings.push("跳过一条损坏的消息".into()),
        }
    }
    Ok(parsed)
}

fn merge_files(files: &[Parsed]) -> (Vec<Record>, usize, Vec<String>) {
    // A fork replays its parent's token snapshots. Compare only explicit ancestors,
    // so unrelated sessions with identical usage remain distinct.
    let mut signatures: HashMap<&str, HashMap<&str, i64>> = HashMap::new();
    let mut parents = HashMap::new();
    for file in files {
        if let Some(parent) = &file.parent_id {
            parents
                .entry(file.session_id.as_str())
                .and_modify(|entry: &mut (&str, i64)| entry.1 = entry.1.min(file.fork_at))
                .or_insert((parent.as_str(), file.fork_at));
        }
        let own = signatures.entry(&file.session_id).or_default();
        for r in &file.records {
            if !r.signature.is_empty() {
                own.entry(&r.signature)
                    .and_modify(|t| *t = (*t).min(r.time))
                    .or_insert(r.time);
            }
        }
    }
    let mut records: HashMap<String, Record> = HashMap::new();
    let mut replayed_ids = HashSet::new();
    let mut duplicates = 0;
    let mut notes = HashSet::new();
    for file in files {
        for record in &file.records {
            let mut cursor = file.session_id.as_str();
            let mut visited = HashSet::new();
            let mut replayed = false;
            while let Some((parent, cutoff)) = parents.get(cursor) {
                if !visited.insert(cursor) {
                    notes.insert("发现循环的分叉来源，部分继承用量无法核对".to_string());
                    break;
                }
                let Some(ancestor) = signatures.get(parent) else {
                    notes.insert("部分分叉会话的父记录缺失，继承用量无法完整核对".to_string());
                    break;
                };
                if ancestor
                    .get(record.signature.as_str())
                    .is_some_and(|at| *cutoff <= 0 || *at <= *cutoff)
                {
                    replayed = true;
                    break;
                }
                cursor = parent;
            }
            if replayed {
                duplicates += 1;
                replayed_ids.insert(record.id.clone());
            }
            match records.entry(record.id.clone()) {
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    e.get_mut().merge(record);
                    duplicates += 1;
                }
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(record.clone());
                }
            }
        }
    }
    let mut records: Vec<_> = records.into_values().collect();
    records.sort_by(|a, b| a.time.cmp(&b.time).then_with(|| a.id.cmp(&b.id)));
    let mut high_water: HashMap<String, [u64; 6]> = HashMap::new();
    let mut seeded = HashSet::new();
    let mut timelines: HashMap<&str, Vec<(i64, [u64; 6])>> = HashMap::new();
    for row in &records {
        if let Some(mut total) = row.total_snapshot {
            let timeline = timelines.entry(&row.session_id).or_default();
            if let Some((_, previous)) = timeline.last() {
                for (value, prev) in total.iter_mut().zip(previous) {
                    *value = (*value).max(*prev);
                }
            }
            timeline.push((row.time, total));
        }
    }
    for (child, (parent, cutoff)) in &parents {
        let inherited = timelines
            .get(parent)
            .and_then(|timeline| {
                timeline
                    .partition_point(|(time, _)| time <= cutoff)
                    .checked_sub(1)
                    .map(|index| timeline[index].1)
            })
            .unwrap_or([0; 6]);
        high_water.insert((*child).to_string(), inherited);
        seeded.insert((*child).to_string());
    }
    drop(timelines);
    for record in &mut records {
        if !replayed_ids.contains(&record.id) && seeded.remove(&record.session_id) {
            if record
                .total_snapshot
                .is_some_and(|total| total[0] < high_water[&record.session_id][0])
            {
                // A new child may reset counters instead of inheriting the parent's.
                high_water.insert(record.session_id.clone(), [0; 6]);
            }
        }
        parsers::reconcile(
            record,
            high_water
                .entry(record.session_id.clone())
                .or_insert([0; 6]),
        );
    }
    records.retain(|r| r.nonzero() && !replayed_ids.contains(&r.id));
    records.sort_by(|a, b| b.time.cmp(&a.time).then_with(|| a.id.cmp(&b.id)));
    if records.len() > MAX_RECORDS {
        records.truncate(MAX_RECORDS);
        notes.insert(format!("用量记录超过 {MAX_RECORDS} 条，仅展示最新部分"));
    }
    let mut notes: Vec<_> = notes.into_iter().collect();
    notes.sort();
    (records, duplicates, notes)
}

fn read(workspace: &str, refresh: bool) -> Result<Report, String> {
    let roots = roots(workspace)?;
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "用量缓存暂不可用")?;
    let mut sources = Vec::new();
    let mut parsed = Vec::new();
    let mut parsed_count = 0;
    let mut live_files = HashSet::new();
    for (root, format) in &roots {
        let mut source = Source {
            path: root.to_string_lossy().into(),
            format: (*format).into(),
            files: 0,
            cached_files: 0,
            records: 0,
            status: "available".into(),
            notes: vec![],
        };
        if !root.exists() {
            source.status = "missing".into();
            sources.push(source);
            continue;
        }
        if let Err(error) = no_links(root) {
            source.status = "error".into();
            source.notes.push(error);
            sources.push(source);
            continue;
        }
        if *format == "sqlite" {
            source.files = 1;
            match opencode_database(root) {
                Ok(file) => {
                    source.records = file.records.len();
                    source.notes.extend(file.warnings.iter().cloned());
                    parsed.push(file);
                }
                Err(error) => {
                    source.status = "error".into();
                    source.notes.push(error);
                }
            }
        } else {
            let mut paths = Vec::new();
            collect(
                root,
                if *format == "opencode" {
                    "json"
                } else {
                    "jsonl"
                },
                0,
                &mut paths,
                &mut source.notes,
            );
            // A hard memory bound also applies before deduplication. Read recent files first.
            paths.sort_by_cached_key(|path| {
                std::cmp::Reverse(fs::metadata(path).ok().and_then(|m| m.modified().ok()))
            });
            source.files = paths.len();
            for path in paths {
                if parsed_count >= MAX_RECORDS * 2 {
                    source
                        .notes
                        .push("原始用量超过 400,000 条，剩余文件已跳过；结果不完整".into());
                    break;
                }
                live_files.insert(path.clone());
                match cached_file(&path, format, refresh, &mut cache) {
                    Ok((file, hit)) => {
                        parsed_count += file.records.len();
                        source.cached_files += usize::from(hit);
                        source.records += file.records.len();
                        for warning in &file.warnings {
                            if source.notes.len() < 20 {
                                source.notes.push(format!(
                                    "{}：{warning}",
                                    path.file_name().unwrap_or_default().to_string_lossy()
                                ));
                            }
                        }
                        parsed.push(file);
                    }
                    Err(error) => {
                        if source.notes.len() < 20 {
                            source.notes.push(format!(
                                "{}：{error}",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            ));
                        }
                    }
                }
            }
        }
        sources.push(source);
    }
    // Remove deleted/archived files from this workspace's cache; never accumulate stale history.
    cache.retain(|path, _| {
        !roots.iter().any(|(root, _)| path.starts_with(root)) || live_files.contains(path)
    });
    let missing_time = parsed
        .iter()
        .flat_map(|p| &p.records)
        .filter(|r| r.time <= 0)
        .count();
    for file in &mut parsed {
        file.records.retain(|r| r.time > 0);
    }
    let (records, duplicates, mut notes) = merge_files(&parsed);
    if missing_time > 0 {
        notes.push(format!("{missing_time} 条用量缺少有效时间，未计入统计"));
    }
    let partial = !notes.is_empty()
        || sources
            .iter()
            .any(|s| s.status == "error" || !s.notes.is_empty());
    Ok(Report {
        workspace: workspace.into(),
        generated_at: now(),
        records,
        sources,
        duplicates,
        partial,
        notes,
    })
}

#[tauri::command]
pub async fn usage_stats(workspace: String, refresh: Option<bool>) -> Result<Report, String> {
    tauri::async_runtime::spawn_blocking(move || read(&workspace, refresh.unwrap_or(false)))
        .await
        .map_err(|_| "用量统计读取未完成".to_string())?
}

#[cfg(test)]
mod tests;
