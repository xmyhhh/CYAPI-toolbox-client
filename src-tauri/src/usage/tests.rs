use super::*;
use serde_json::{json, Value};
use std::io::Write;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = env::temp_dir().join(format!("cyapi-usage-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn jsonl(&self, name: &str, rows: &[Value]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(
            &path,
            rows.iter().map(|v| format!("{v}\n")).collect::<String>(),
        )
        .unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.0.is_absolute());
        assert!(self.0.starts_with(env::temp_dir()));
        assert!(self
            .0
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("cyapi-usage-test-"));
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn meta(id: &str, parent: Option<&str>) -> Value {
    json!({"timestamp":"2026-10-20T08:00:00Z","type":"session_meta","payload":{"id":id,"cwd":"F:/work/demo","forked_from_id":parent}})
}
fn context(model: &str) -> Value {
    json!({"type":"turn_context","payload":{"model":model}})
}
fn token(at: &str, total: u64, last: Option<u64>) -> Value {
    json!({"timestamp":at,"type":"event_msg","payload":{"type":"token_count","info":{
        "total_token_usage":{"input_tokens":total,"cached_input_tokens":total/2,"output_tokens":total/5,"reasoning_output_tokens":total/10},
        "last_token_usage":last.map(|n| json!({"input_tokens":n,"cached_input_tokens":n/2,"output_tokens":n/5,"reasoning_output_tokens":n/10}))
    }}})
}

#[test]
fn codex_uses_exact_last_usage_and_deduplicates_snapshots_without_resetting_at_model_changes() {
    let f = Fixture::new();
    let first = token("2026-10-20T07:00:00Z", 100, Some(100));
    let path = f.jsonl(
        "codex.jsonl",
        &[
            meta("thread", None),
            context("gpt-5"),
            first.clone(),
            first,
            context("another-model"),
            token("2026-10-20T07:01:00Z", 200, Some(30)),
            // A different rate-limit lane may have a smaller cumulative counter.
            token("2026-10-20T07:02:00Z", 150, Some(20)),
        ],
    );
    let parsed = parsers::jsonl(&path, "codex").unwrap();
    let (rows, _, notes) = merge_files(&[parsed.clone(), parsed]);
    assert!(notes.is_empty());
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.iter().map(|r| r.input).sum::<u64>(), 150);
    assert_eq!(
        rows.iter()
            .filter(|r| r.model == "another-model")
            .map(|r| r.input)
            .sum::<u64>(),
        50
    );
    assert!(rows
        .iter()
        .all(|r| r.reasoning_output <= r.output && r.cached_input <= r.input));
}

#[test]
fn codex_forks_drop_only_ancestor_replay_and_keep_identical_independent_sessions() {
    let f = Fixture::new();
    let old = token("2026-10-20T07:00:00Z", 100, Some(100));
    let a = parsers::jsonl(
        &f.jsonl("a.jsonl", &[meta("parent", None), old.clone()]),
        "codex",
    )
    .unwrap();
    let child = parsers::jsonl(
        &f.jsonl(
            "child.jsonl",
            &[
                meta("child", Some("parent")),
                old.clone(),
                token("2026-10-20T09:00:00Z", 140, Some(40)),
            ],
        ),
        "codex",
    )
    .unwrap();
    let independent = parsers::jsonl(
        &f.jsonl("b.jsonl", &[meta("unrelated", None), old]),
        "codex",
    )
    .unwrap();
    let (rows, dupes, notes) = merge_files(&[a, child, independent]);
    assert_eq!(rows.len(), 3);
    assert_eq!(dupes, 1);
    assert!(notes.is_empty());
    assert_eq!(
        rows.iter().find(|r| r.session_id == "child").unwrap().input,
        40
    );
}

#[test]
fn codex_continuation_restores_cumulative_baseline_across_files() {
    let f = Fixture::new();
    let a = parsers::jsonl(
        &f.jsonl(
            "a.jsonl",
            &[
                meta("same-thread", None),
                token("2026-10-20T07:00:00Z", 100, None),
            ],
        ),
        "codex",
    )
    .unwrap();
    let b = parsers::jsonl(
        &f.jsonl(
            "b.jsonl",
            &[
                meta("same-thread", None),
                token("2026-10-20T08:00:00Z", 160, None),
            ],
        ),
        "codex",
    )
    .unwrap();
    let (rows, _, _) = merge_files(&[b, a]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().map(|r| r.input).sum::<u64>(), 160);
}

#[test]
fn claude_streamed_content_is_one_message_with_normalized_cache_totals() {
    let f = Fixture::new();
    let line = |output, model| {
        json!({"type":"assistant","timestamp":"2026-10-20T07:00:00Z","sessionId":"claude-session","message":{
            "id":"msg-id","model":model,"usage":{"input_tokens":20,"output_tokens":output,"cache_read_input_tokens":100,"cache_creation_input_tokens":40,"cache_creation":{"ephemeral_1h_input_tokens":10}}
        }})
    };
    let parsed = parsers::jsonl(
        &f.jsonl(
            "claude.jsonl",
            &[
                line(1, "claude-sonnet"),
                line(30, "claude-sonnet"),
                line(100, "<synthetic>"),
            ],
        ),
        "claude",
    )
    .unwrap();
    let (rows, dupes, _) = merge_files(&[parsed]);
    assert_eq!(rows.len(), 1);
    assert_eq!(dupes, 1);
    let r = &rows[0];
    assert_eq!(r.input, 160);
    assert_eq!(r.output, 30);
    assert_eq!(r.cached_input, 100);
    assert_eq!(r.cache_write_1h, 10);
    let serialized = serde_json::to_value(r).unwrap();
    assert!(serialized.get("message").is_none());
    assert!(serialized.get("totalSnapshot").is_none());
}

fn oc_message(completed: bool) -> Value {
    let mut v = json!({"role":"assistant","modelID":"custom","tokens":{"input":20,"output":30,"reasoning":10,"cache":{"read":40,"write":5}},"time":{"created":1792483200000i64},"cost":0});
    if completed {
        v["time"]["completed"] = json!(1792483201000i64);
    }
    v
}

#[test]
fn opencode_reads_live_wal_and_only_completed_responses() {
    let f = Fixture::new();
    let path = f.0.join("opencode.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,data TEXT);").unwrap();
    conn.execute(
        "INSERT INTO message VALUES (?1,'s',1,?2)",
        rusqlite::params!["complete", oc_message(true).to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO message VALUES (?1,'s',2,?2)",
        rusqlite::params!["pending", oc_message(false).to_string()],
    )
    .unwrap();
    let parsed = opencode_database(&path).unwrap();
    assert_eq!(parsed.records.len(), 1);
    let r = &parsed.records[0];
    assert_eq!(r.input, 65);
    assert_eq!(r.output, 40);
    assert_eq!(r.reasoning_output, 10);
    assert_eq!(r.reported_cost, None);
    assert!(path.with_extension("db-wal").exists());
}

#[test]
fn opencode_v2_compaction_and_model_objects_are_supported() {
    let f = Fixture::new();
    let path = f.0.join("opencode.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE session_message(id TEXT,session_id TEXT,time_created INTEGER,data TEXT,type TEXT);").unwrap();
    let value = json!({"status":"completed","model":{"id":"v2-model","providerID":"gateway"},"tokens":{"input":4,"output":2,"reasoning":1},"time":{"created":1792483200000i64},"cost":0.125});
    conn.execute(
        "INSERT INTO session_message VALUES ('id','s',1,?1,'compaction')",
        [value.to_string()],
    )
    .unwrap();
    let parsed = opencode_database(&path).unwrap();
    assert_eq!(parsed.records.len(), 1);
    assert_eq!(parsed.records[0].model, "v2-model");
    assert_eq!(parsed.records[0].output, 3);
    assert_eq!(parsed.records[0].reported_cost, Some(0.125));
}

#[test]
fn cache_reuses_unchanged_files_and_recovers_after_partial_append_or_truncation() {
    let f = Fixture::new();
    let path = f.jsonl(
        "cache.jsonl",
        &[
            meta("s", None),
            token("2026-10-20T07:00:00Z", 100, Some(100)),
        ],
    );
    let mut cache = HashMap::new();
    assert!(!cached_file(&path, "codex", false, &mut cache).unwrap().1);
    assert!(cached_file(&path, "codex", false, &mut cache).unwrap().1);
    FileAppend::append(
        &path,
        b"{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\"",
    );
    let (broken, hit) = cached_file(&path, "codex", false, &mut cache).unwrap();
    assert!(!hit);
    assert!(!broken.warnings.is_empty());
    f.jsonl(
        "cache.jsonl",
        &[meta("s", None), token("2026-10-20T08:00:00Z", 20, Some(20))],
    );
    let (fixed, hit) = cached_file(&path, "codex", false, &mut cache).unwrap();
    assert!(!hit);
    assert!(fixed.warnings.is_empty());
    assert_eq!(fixed.records[0].input, 20);
}
#[test]
fn codex_fork_without_replay_uses_parent_baseline_and_supports_counter_resets() {
    let f = Fixture::new();
    let a = parsers::jsonl(
        &f.jsonl(
            "a.jsonl",
            &[
                meta("parent", None),
                token("2026-10-20T07:00:00Z", 100, None),
            ],
        ),
        "codex",
    )
    .unwrap();
    let b = parsers::jsonl(
        &f.jsonl(
            "b.jsonl",
            &[
                meta("child", Some("parent")),
                token("2026-10-20T09:00:00Z", 160, None),
            ],
        ),
        "codex",
    )
    .unwrap();
    let c = parsers::jsonl(
        &f.jsonl(
            "c.jsonl",
            &[
                meta("reset", Some("parent")),
                token("2026-10-20T09:00:00Z", 20, None),
            ],
        ),
        "codex",
    )
    .unwrap();
    let (rows, _, notes) = merge_files(&[b, a, c]);
    assert!(notes.is_empty());
    assert_eq!(
        rows.iter().find(|r| r.session_id == "child").unwrap().input,
        60
    );
    assert_eq!(
        rows.iter().find(|r| r.session_id == "reset").unwrap().input,
        20
    );
}

#[test]
fn compressed_codex_rollouts_match_plain_jsonl_and_are_discovered() {
    let f = Fixture::new();
    let plain = f.jsonl(
        "rollout-test.jsonl",
        &[
            meta("s", None),
            context("m"),
            token("2026-10-20T09:00:00Z", 160, Some(160)),
        ],
    );
    let compressed = f.0.join("rollout-test.jsonl.zst");
    fs::write(
        &compressed,
        zstd::stream::encode_all(fs::read(&plain).unwrap().as_slice(), 1).unwrap(),
    )
    .unwrap();
    let a = parsers::jsonl(&plain, "codex").unwrap();
    let b = parsers::jsonl(&compressed, "codex").unwrap();
    let (rows, dupes, notes) = merge_files(&[a, b]);
    assert!(notes.is_empty());
    assert_eq!(rows.len(), 1);
    assert_eq!(dupes, 1);
    assert_eq!(rows[0].input, 160);
    let mut paths = Vec::new();
    let mut notes = Vec::new();
    collect(&f.0, "jsonl", 0, &mut paths, &mut notes);
    assert_eq!(paths.len(), 2);
}

#[test]
#[ignore = "read-only smoke check against local agent history"]
fn local_usage_smoke() {
    for workspace in ["codex", "claude", "opencode"] {
        let start = std::time::Instant::now();
        let report = read(workspace, false).unwrap();
        eprintln!(
            "{workspace}: records={} files={} partial={} elapsed={}ms",
            report.records.len(),
            report.sources.iter().map(|s| s.files).sum::<usize>(),
            report.partial,
            start.elapsed().as_millis()
        );
        for source in &report.sources {
            for note in &source.notes {
                eprintln!("source warning: {note}");
            }
        }
        for note in &report.notes {
            eprintln!("warning: {note}");
        }
        assert!(report.records.iter().all(|r| r.time > 0
            && r.cached_input + r.cache_write <= r.input
            && r.reasoning_output <= r.output));
        let refreshed = read(workspace, false).unwrap();
        assert!(refreshed.records.len() >= report.records.len().saturating_sub(10));
    }
}

#[test]
fn oversized_tool_output_is_skipped_without_marking_valid_usage_incomplete() {
    let f = Fixture::new();
    let path = f.jsonl("large.jsonl", &[
        meta("s",None),
            json!({"type":"response_item","payload":{"type":"function_call_output","output":format!("token_count: {}","x".repeat(parsers::MAX_LINE + 100))}}),
        token("2026-10-20T09:00:00Z",100,Some(100)),
    ]);
    let parsed = parsers::jsonl(&path, "codex").unwrap();
    assert!(parsed.warnings.is_empty());
    assert_eq!(parsed.records.len(), 1);
    assert_eq!(parsed.records[0].input, 100);
}

#[test]
fn claude_usage_is_read_when_the_role_and_type_follow_a_large_message() {
    let f = Fixture::new();
    let raw = format!(
        r#"{{"parentUuid":"parent","message":{{"content":[{{"text":"{}"}}],"id":"msg-long","model":"claude-test","role":"assistant","usage":{{"input_tokens":100,"output_tokens":30}}}},"type":"assistant","timestamp":"2026-10-20T09:00:00Z","sessionId":"s"}}
"#,
        "x".repeat(1000)
    );
    let path = f.0.join("claude-long.jsonl");
    fs::write(&path, raw).unwrap();
    let parsed = parsers::jsonl(&path, "claude").unwrap();
    assert_eq!(parsed.records.len(), 1);
    assert_eq!(parsed.records[0].input, 100);
}

struct FileAppend;
impl FileAppend {
    fn append(path: &Path, bytes: &[u8]) {
        fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }
}
