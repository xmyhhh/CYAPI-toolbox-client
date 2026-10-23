use super::*;

const TIME: u64 = 1_700_000_000_000;
const SECRET: &str = "PRIVATE-chat-tool-reasoning-marker";
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("cyapi-agent-logs-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn json(&self, path: &str, v: &Value) {
        self.write(path, &v.to_string());
    }
    fn write(&self, path: &str, s: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, s).unwrap();
    }
    fn lines(&self, path: &str, rows: &[Value]) {
        self.write(
            path,
            &rows.iter().map(|v| format!("{v}\n")).collect::<String>(),
        );
    }
    fn builder(&self, workspace: &str) -> Builder {
        Builder::new(workspace, &self.0, 0)
    }
    fn analyze(&self, workspace: &str) -> Snapshot {
        let mut b = self.builder(workspace);
        if workspace == "claude" {
            claude::analyze(&self.0, &mut b);
        } else {
            opencode::analyze(&self.0, &mut b);
        }
        b.finish()
    }
    fn db(&self, v2: bool) -> Connection {
        let db = Connection::open(self.0.join("opencode.db")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .unwrap();
        if v2 {
            db.execute_batch("CREATE TABLE session_v2(id TEXT PRIMARY KEY,title TEXT,directory TEXT,parent_id TEXT,fork_session_id TEXT,time_updated INTEGER); CREATE TABLE session_message(id TEXT PRIMARY KEY,session_id TEXT,type TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);").unwrap();
        } else {
            db.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY,title TEXT,time_updated INTEGER); CREATE TABLE message(id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT); CREATE TABLE part(id TEXT PRIMARY KEY,session_id TEXT,message_id TEXT,time_created INTEGER,data TEXT);").unwrap();
        }
        db
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn claude_response(id: &str, output: u64) -> Value {
    json!({"type":"assistant","sessionId":"main","uuid":format!("row-{output}"),"timestamp":TIME+output,"cwd":"F:/project","message":{"id":id,"model":"claude-sonnet","content":[{"type":"text","text":SECRET},{"type":"thinking","thinking":SECRET},{"type":"tool_use","id":"call-1","name":"Bash","input":{"command":SECRET}}],"usage":{"input_tokens":100,"output_tokens":output,"cache_read_input_tokens":40,"cache_creation_input_tokens":20,"cache_creation":{"ephemeral_1h_input_tokens":5}}}})
}
fn oc_response(output: u64) -> Value {
    json!({"time":{"created":TIME,"completed":TIME+100},"model":{"id":"model-one","providerID":"provider-one"},"content":[{"type":"text","text":SECRET},{"type":"reasoning","text":SECRET},{"type":"tool","id":"call-oc","name":"shell","state":{"status":"error","input":SECRET,"error":SECRET,"content":SECRET},"time":{"created":TIME,"completed":TIME+70}}],"tokens":{"input":10,"output":output,"reasoning":3,"cache":{"read":20,"write":30}},"finish":"tool-calls"})
}
fn insert_v2(db: &Connection, id: &str, kind: &str, data: &Value) {
    db.execute(
        "INSERT INTO session_message VALUES(?1,'ses-main',?2,?3,?3,?4)",
        rusqlite::params![id, kind, TIME as i64, data.to_string()],
    )
    .unwrap();
}
fn private_free(s: &Snapshot) {
    let mut value = report(s, None).unwrap();
    value["evidence"] = json!(s.events.iter().map(|e| &e.evidence).collect::<Vec<_>>());
    assert!(
        !value.to_string().contains(SECRET),
        "private content escaped allowlist"
    );
}

#[test]
fn claude_stream_fragments_tools_and_cache_are_not_double_counted() {
    let f = Fixture::new();
    f.lines("projects/p/main.jsonl", &[claude_response("msg-1",10),claude_response("msg-1",20),json!({"type":"user","sessionId":"main","uuid":"result-1","timestamp":TIME+40,"message":{"content":[{"type":"tool_result","tool_use_id":"call-1","is_error":true,"content":SECRET}]}})]);
    let s = f.analyze("claude");
    assert_eq!(s.totals.responses, 1);
    assert_eq!(s.totals.tool_calls, 1);
    assert_eq!(s.totals.tool_errors, 1);
    assert_eq!(s.totals.tokens.input, 160);
    assert_eq!(s.totals.tokens.output, 20);
    assert_eq!(s.totals.tokens.cache_write_1h, 5);
    assert_eq!(s.totals.user_turns, 0);
    assert!(s.mirrored >= 1);
    assert!(
        s.events
            .iter()
            .find(|e| e.kind == "response")
            .unwrap()
            .mirrors
            .len()
            == 1
    );
    private_free(&s);
}

#[test]
fn claude_partial_stream_counters_merge_before_normalizing_cache_totals() {
    let f = Fixture::new();
    let mut a = claude_response("same", 0);
    a["message"]["usage"] = json!({"input_tokens":100,"output_tokens":0});
    let mut c = claude_response("same", 10);
    c["message"]["usage"] = json!({"input_tokens":0,"output_tokens":10,"cache_read_input_tokens":40,"cache_creation_input_tokens":20});
    f.lines("projects/p/main.jsonl", &[a, c]);
    let s = f.analyze("claude");
    assert_eq!(s.totals.responses, 1);
    assert_eq!(s.totals.tokens.input, 160);
    assert_eq!(s.totals.tokens.output, 10);
}

#[test]
fn runtime_tool_arguments_cannot_create_fake_retries_or_http_failures() {
    let f = Fixture::new();
    f.write("log/opencode.log","timestamp=2023-11-14T22:13:20Z level=INFO service=tool message=\"execute command\" args=\"retry HTTP 401 connection error\" command=\"curl --retry 10\"\n");
    let mut b = f.builder("opencode");
    runtime::analyze(&f.0.join("log"), "opencode", &mut b);
    assert_eq!(b.finish().event_count, 0);
    let event = Event {
        model: "model-search".into(),
        provider: "provider-search".into(),
        ..Default::default()
    };
    assert!(matches(
        &event,
        &Query {
            search: Some("MODEL-search".into()),
            ..Default::default()
        }
    ));
    assert!(matches(
        &event,
        &Query {
            search: Some("provider-search".into()),
            ..Default::default()
        }
    ));
}

#[test]
fn claude_index_main_subagent_and_retries_keep_their_identity() {
    let f = Fixture::new();
    f.json("projects/p/sessions-index.json", &json!({"entries":[{"sessionId":"main","summary":"项目会话","projectPath":"F:/project","modified":TIME,"firstPrompt":SECRET,"fullPath":"C:/do-not-follow"}]}));
    f.lines("projects/p/main.jsonl", &[claude_response("msg-1",10),json!({"type":"system","subtype":"api_error","sessionId":"main","uuid":"api-1","timestamp":TIME+100,"retryAttempt":1,"retryInMs":500,"error":{"status":429,"type":"rate_limit_error","message":SECRET}}),json!({"type":"system","subtype":"compact_boundary","sessionId":"main","uuid":"compact-1","timestamp":TIME+200,"compactMetadata":{"trigger":"auto","preTokens":120000}})]);
    f.lines(
        "projects/p/main/subagents/agent-a.jsonl",
        &[claude_response("msg-2", 20)],
    );
    let s = f.analyze("claude");
    assert_eq!(s.sessions.len(), 2);
    assert_eq!(s.totals.responses, 2);
    assert_eq!(s.totals.api_errors, 1);
    assert_eq!(s.totals.retries, 1);
    assert_eq!(s.totals.compactions, 1);
    assert_eq!(
        s.sessions.iter().find(|s| s.id == "main").unwrap().title,
        "项目会话"
    );
    assert_eq!(
        s.sessions
            .iter()
            .find(|s| s.id == "main::agent-a")
            .unwrap()
            .parent_id
            .as_deref(),
        Some("main")
    );
    assert_eq!(
        session_detail(&s, "main").unwrap()["children"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(s.issues.iter().any(|i| i.category == "rate_limit"));
    private_free(&s);
}

#[test]
fn claude_synthetic_errors_and_unrecorded_time_do_not_become_responses() {
    let f = Fixture::new();
    f.lines("projects/p/main.jsonl", &[json!({"type":"assistant","sessionId":"main","uuid":"err","timestamp":TIME,"isApiErrorMessage":true,"error":"authentication_error","message":{"model":"<synthetic>","content":[{"type":"text","text":SECRET}]}}),json!({"type":"assistant","sessionId":"main","message":{"id":"missing-time","usage":{"input_tokens":10}}})]);
    let s = f.analyze("claude");
    assert_eq!(s.totals.responses, 0);
    assert_eq!(s.totals.api_errors, 1);
    assert!(s.partial);
    private_free(&s);
}

#[test]
fn jsonl_recovers_after_large_and_damaged_records_and_marks_tail() {
    let f = Fixture::new();
    let valid = claude_response("msg", 10).to_string();
    f.write(
        "records.jsonl",
        &format!(
            "broken\n{}\n{valid}\n{{",
            "x".repeat(MAX_LINE as usize + 40)
        ),
    );
    let mut rows = 0;
    let (_, partial, bad) =
        json_lines(&f.0.join("records.jsonl"), JSON_BYTES, |_, _| rows += 1).unwrap();
    assert!(!partial);
    assert_eq!(bad, 3);
    assert_eq!(rows, 1);
    let mut rows = 0;
    let (_, partial, _) = json_lines(
        &f.0.join("records.jsonl"),
        (valid.len() + 20) as u64,
        |_, _| rows += 1,
    )
    .unwrap();
    assert!(partial);
    assert_eq!(rows, 1);
}

#[test]
fn opencode_v2_reads_live_wal_embedded_tools_compactions_and_never_legacy_duplicates() {
    let f = Fixture::new();
    let db = f.db(true);
    db.execute(
        "INSERT INTO session_v2 VALUES('ses-main','会话','F:/project',NULL,'ses-origin',?1)",
        [TIME as i64],
    )
    .unwrap();
    insert_v2(&db, "msg-1", "assistant", &oc_response(7));
    insert_v2(
        &db,
        "msg-error",
        "assistant",
        &json!({"time":{"created":TIME+200},"error":{"type":"APIError","data":{"statusCode":401,"message":SECRET}}}),
    );
    insert_v2(
        &db,
        "compact",
        "compaction",
        &json!({"time":{"created":TIME+300},"status":"completed","summary":SECRET,"tokens":{"input":2,"output":1}}),
    );
    insert_v2(
        &db,
        "user",
        "user",
        &json!({"time":{"created":TIME},"text":SECRET}),
    );
    f.json("storage/message/ses-main/msg-1.json",&json!({"id":"msg-1","sessionID":"ses-main","role":"assistant","time":{"created":TIME},"tokens":{"input":999999}}));
    assert!(fs::metadata(f.0.join("opencode.db-wal")).unwrap().len() > 0);
    let s = f.analyze("opencode");
    assert_eq!(s.totals.responses, 2);
    assert_eq!(s.totals.response_errors, 1);
    assert_eq!(s.totals.compactions, 1);
    assert_eq!(s.totals.tool_calls, 1);
    assert_eq!(s.totals.tool_errors, 1);
    assert_eq!(s.totals.tokens.input, 62);
    assert_eq!(s.totals.tokens.output, 11);
    assert_eq!(s.totals.tokens.reasoning, 3);
    assert_eq!(s.sessions[0].forked_from.as_deref(), Some("ses-origin"));
    assert!(!s.sources.iter().any(|s| s.role == "legacy"));
    assert!(!s.partial);
    assert!(s.issues.iter().any(|i| i.category == "auth"));
    private_free(&s);
}

#[test]
fn opencode_v1_parts_link_tools_and_do_not_repeat_step_usage() {
    let f = Fixture::new();
    let db = f.db(false);
    let v = json!({"role":"assistant","modelID":"legacy-model","providerID":"legacy-provider","time":{"created":TIME,"completed":TIME+100},"tokens":{"input":10,"output":5}});
    db.execute(
        "INSERT INTO message VALUES('m','s',?1,?1,?2)",
        rusqlite::params![TIME as i64, v.to_string()],
    )
    .unwrap();
    for (id, part) in [
        (
            "p-tool",
            json!({"type":"tool","callID":"call","tool":"bash","state":{"status":"error","input":SECRET,"output":SECRET,"error":SECRET,"time":{"start":TIME,"end":TIME+100}}}),
        ),
        (
            "p-step",
            json!({"type":"step-finish","reason":"tool-calls","tokens":{"input":10,"output":5}}),
        ),
    ] {
        db.execute(
            "INSERT INTO part VALUES(?1,'s','m',?2,?3)",
            rusqlite::params![id, TIME as i64, part.to_string()],
        )
        .unwrap();
    }
    let s = f.analyze("opencode");
    assert_eq!(s.totals.responses, 1);
    assert_eq!(s.totals.tokens.input, 10);
    assert_eq!(s.totals.tokens.output, 5);
    assert_eq!(s.totals.tool_errors, 1);
    assert_eq!(
        s.events
            .iter()
            .find(|e| e.kind == "tool")
            .unwrap()
            .message_id
            .as_deref(),
        Some("m")
    );
    private_free(&s);
}

#[test]
fn opencode_optional_metadata_columns_and_json_time_are_supported() {
    let f = Fixture::new();
    let db = Connection::open(f.0.join("opencode.db")).unwrap();
    db.execute_batch("CREATE TABLE session_message(id TEXT,session_id TEXT,type TEXT,data TEXT);")
        .unwrap();
    db.execute(
        "INSERT INTO session_message VALUES('m','s','assistant',?1)",
        [oc_response(7).to_string()],
    )
    .unwrap();
    let mut b = f.builder("opencode");
    b.snapshot.since = TIME - 1;
    opencode::analyze(&f.0, &mut b);
    let s = b.finish();
    assert_eq!(s.totals.responses, 1);
    assert_eq!(s.sessions[0].id, "s");
    assert!(s.partial);
    assert!(!s.sources.iter().any(|s| s.role == "legacy"));
}

#[test]
fn opencode_damaged_optional_metadata_and_parts_do_not_discard_valid_messages() {
    let f = Fixture::new();
    let db = f.db(false);
    db.execute("INSERT INTO session VALUES('s',X'ff',?1)", [TIME as i64])
        .unwrap();
    let message = json!({"role":"assistant","time":{"created":TIME,"completed":TIME+10},"tokens":{"input":10,"output":2}});
    db.execute(
        "INSERT INTO message VALUES('m','s',?1,?1,?2)",
        rusqlite::params![TIME as i64, message.to_string()],
    )
    .unwrap();
    db.execute("INSERT INTO part VALUES('p','s','m','bad-time','{}')", [])
        .unwrap();
    let s = f.analyze("opencode");
    assert_eq!(s.totals.responses, 1);
    assert_eq!(s.totals.tokens.input, 10);
    assert!(s.partial);
    assert!(!s.sources.iter().any(|s| s.role == "legacy"));
}

#[test]
fn opencode_incompatible_db_falls_back_to_legacy_families() {
    let f = Fixture::new();
    Connection::open(f.0.join("opencode.db"))
        .unwrap()
        .execute_batch("CREATE TABLE unknown(id TEXT);")
        .unwrap();
    f.json("storage/session/project/s.json",&json!({"id":"s","title":"旧会话","directory":"F:/old","parentID":"parent","time":{"updated":TIME}}));
    let mut msg = oc_response(7);
    msg["role"] = json!("assistant");
    msg["id"] = json!("m");
    msg["sessionID"] = json!("s");
    msg["content"] = json!([]);
    f.json("storage/message/s/m.json", &msg);
    f.json("storage/part/m/p.json",&json!({"id":"p","type":"tool","sessionID":"s","messageID":"m","callID":"c","tool":"read","state":{"status":"completed","time":{"start":TIME,"end":TIME+10},"input":SECRET,"output":SECRET}}));
    let s = f.analyze("opencode");
    assert_eq!(s.totals.responses, 1);
    assert_eq!(s.totals.tool_calls, 1);
    assert_eq!(s.sessions[0].parent_id.as_deref(), Some("parent"));
    assert!(s.partial);
    private_free(&s);
}

#[test]
fn runtime_logfmt_old_text_and_claude_debug_keep_only_structural_evidence() {
    let f = Fixture::new();
    let stamp = "2023-11-14T22:13:20.000Z";
    f.write("log/opencode.log",&format!("timestamp={stamp} level=ERROR service=provider http.method=POST http.status=429 sessionID=ses-main requestId=req-1 durationMs=123 message=\"{SECRET}\" args=\"{SECRET}\" command={SECRET}\nINFO  2023-11-14T22:13:20 +12ms service=server method=GET status=200 message=\"local request\"\n"));
    let mut b = f.builder("opencode");
    runtime::analyze(&f.0.join("log"), "opencode", &mut b);
    let s = b.finish();
    assert_eq!(s.event_count, 2);
    assert!(s.events.iter().all(|e| e.time == TIME));
    assert_eq!(s.totals.responses, 0);
    assert_eq!(
        s.events.iter().find(|e| e.failed).unwrap().status,
        Some(429)
    );
    assert!(s.issues.iter().any(|i| i.category == "rate_limit"));
    private_free(&s);
    let uuid = "12345678-1234-4234-8234-123456789012";
    f.write(
        &format!("debug/{uuid}.txt"),
        &format!(
            "{stamp} [ERROR] Connection timeout {SECRET}\n{stamp} [DEBUG] Tool input {SECRET}\n"
        ),
    );
    let mut b = f.builder("claude");
    runtime::analyze(&f.0.join("debug"), "claude", &mut b);
    let s = b.finish();
    assert_eq!(s.event_count, 1);
    assert_eq!(s.events[0].session_id.as_deref(), Some(uuid));
    assert_eq!(s.issues[0].category, "network");
    private_free(&s);
}

#[test]
fn missing_sources_remain_explicit_without_inventing_data() {
    let f = Fixture::new();
    for app in ["claude", "opencode"] {
        let s = f.analyze(app);
        assert_eq!(s.event_count, 0);
        assert!(s.sources.iter().any(|s| s.status == "missing"));
    }
}

#[test]
fn window_and_unknown_times_cannot_inflate_totals() {
    let f = Fixture::new();
    let mut b = f.builder("claude");
    b.snapshot.since = TIME;
    b.snapshot.generated_at = TIME + 100;
    let src = source(&f.0.join("test.jsonl"), "conversation", "JSONL");
    for (n, t) in [(0, 0), (1, TIME - 1), (2, TIME + 50), (3, TIME + 101)] {
        let mut v = claude_response(&format!("m-{n}"), 10);
        v["timestamp"] = json!(t);
        claude::line(&v, "main", None, false, &src, n, &mut b);
    }
    let s = b.finish();
    assert_eq!(s.totals.responses, 1);
    assert_eq!(s.totals.tokens.input, 160);
}

fn request(value: Value) -> Request {
    serde_json::from_value(value).unwrap()
}
#[test]
fn immutable_snapshot_pagination_queries_and_workspace_guards() {
    let f = Fixture::new();
    let mut b = f.builder("claude");
    for i in 0..215 {
        b.add(
            format!("e-{i}"),
            Event {
                time: TIME + i,
                kind: "response".into(),
                level: "INFO".into(),
                session_id: Some("main".into()),
                summary: format!("response-{i}"),
                ..Default::default()
            },
        );
    }
    let s = Arc::new(b.finish());
    CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .push_front(s.clone());
    let page = dispatch(request(
        json!({"action":"events","workspace":"claude","snapshotId":s.id}),
    ))
    .unwrap();
    assert_eq!(page["events"].as_array().unwrap().len(), 200);
    assert_eq!(page["nextOffset"], 200);
    let next = dispatch(request(
        json!({"action":"events","workspace":"claude","snapshotId":s.id,"offset":200}),
    ))
    .unwrap();
    assert_eq!(next["events"].as_array().unwrap().len(), 15);
    assert!(next["nextOffset"].is_null());
    let search=dispatch(request(json!({"action":"events","workspace":"claude","snapshotId":s.id,"query":{"search":"response-214","sessionId":"main"}}))).unwrap();
    assert_eq!(search["total"], 1);
    assert!(dispatch(request(
        json!({"action":"events","workspace":"opencode","snapshotId":s.id})
    ))
    .is_err());
    assert!(dispatch(request(
        json!({"action":"session","workspace":"claude","snapshotId":s.id,"sessionId":"missing"})
    ))
    .is_err());
    f.write(
        "later.jsonl",
        "later file changes cannot affect the snapshot",
    );
    assert_eq!(s.event_count, 215);
    CACHE
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .retain(|a| a.id != s.id);
}

#[test]
fn report_sanitizes_credentials_and_has_explicit_session_scope() {
    let f = Fixture::new();
    let mut b = f.builder("claude");
    for id in ["a", "b"] {
        b.add(
            id.into(),
            Event {
                time: TIME,
                session_id: Some(id.into()),
                summary: "Authorization: Bearer fixture-secret-token".into(),
                ..Default::default()
            },
        );
    }
    let s = b.finish();
    let value = report(&s, Some("a")).unwrap();
    assert_eq!(value["events"].as_array().unwrap().len(), 1);
    assert_eq!(value["sessionAnalysis"]["session"]["id"], "a");
    assert!(!value.to_string().contains("fixture-secret-token"));
}

#[test]
#[ignore = "manual read-only smoke against installed OpenCode; no conversation content printed"]
fn local_opencode_read_only_smoke() {
    let root = roots("opencode").unwrap();
    let mut b = Builder::new("opencode", &root, 604800);
    opencode::analyze(&root, &mut b);
    let s = b.finish();
    println!("events={} sessions={} responses={} response_errors={} tools={} tool_errors={} usage={} partial={}",s.event_count,s.sessions.len(),s.totals.responses,s.totals.response_errors,s.totals.tool_calls,s.totals.tool_errors,s.totals.usage_records,s.partial);
    for src in s
        .sources
        .iter()
        .filter(|s| s.format == "SQLite" || s.role == "runtime")
    {
        println!(
            "source={} status={} rows={} wal={} partial={} note={}",
            src.role, src.status, src.records, src.wal_size, src.partial, src.note
        );
    }
    assert!(s
        .sources
        .iter()
        .any(|s| s.format == "SQLite" && s.status == "available" && s.records > 0));
    assert!(s.totals.responses > 0);
}

#[test]
#[ignore = "write synthetic native-parser fixtures for manual browser interaction checks"]
fn browser_fixtures() {
    let output = PathBuf::from(
        std::env::var_os("CYAPI_AGENT_LOG_FIXTURE_DIR").expect("explicit fixture output directory"),
    );
    fs::create_dir_all(&output).unwrap();
    for app in ["claude", "opencode"] {
        let f = Fixture::new();
        if app == "claude" {
            let mut rows = vec![
                json!({"type":"user","sessionId":"main","uuid":"u","timestamp":TIME,"message":{"content":SECRET}}),
            ];
            for i in 0..215 {
                let mut v = claude_response(&format!("msg-{i}"), 10);
                v["timestamp"] = json!(TIME + i);
                v["message"]["content"] = json!([]);
                rows.push(v);
            }
            rows.push(json!({"type":"system","subtype":"api_error","sessionId":"main","uuid":"api","timestamp":TIME+300,"retryAttempt":1,"error":{"status":429,"type":"rate_limit_error","message":SECRET}}));
            f.lines("projects/p/main.jsonl", &rows);
            f.lines(
                "projects/p/main/subagents/agent-a.jsonl",
                &[claude_response("child-msg", 30)],
            );
            f.json("projects/p/sessions-index.json",&json!({"entries":[{"sessionId":"main","summary":"修复连接并核对工具状态","projectPath":"F:/project","modified":TIME}]}));
        } else {
            let db = f.db(true);
            db.execute("INSERT INTO session_v2 VALUES('ses-main','修复连接并核对工具状态','F:/project',NULL,NULL,?1)",[TIME as i64]).unwrap();
            db.execute("INSERT INTO session_v2 VALUES('ses-child','子代理：检查文件','F:/project','ses-main','ses-origin',?1)",[TIME as i64]).unwrap();
            for i in 0..215 {
                let mut v = oc_response(7);
                v["content"] = json!([]);
                insert_v2(&db, &format!("msg-{i}"), "assistant", &v);
            }
            insert_v2(
                &db,
                "api-error",
                "assistant",
                &json!({"time":{"created":TIME+200},"error":{"type":"APIError","data":{"statusCode":401,"message":SECRET}}}),
            );
            insert_v2(&db, "tool-msg", "assistant", &oc_response(7));
            insert_v2(
                &db,
                "compact",
                "compaction",
                &json!({"time":{"created":TIME+300},"status":"completed","tokens":{"input":10,"output":3},"summary":SECRET}),
            );
            f.write("log/opencode.log","timestamp=2023-11-14T22:13:20.000Z level=WARN service=mcp sessionID=ses-main message=\"connection failed\"\n");
        }
        let snapshot = f.analyze(app);
        private_free(&snapshot);
        let value = json!({"analysis":snapshot,"events":snapshot.events,"details":snapshot.events.iter().map(|e|(e.id.clone(),json!({"event":e,"evidence":e.evidence,"note":"测试样本：只展示原生解析器提取的结构字段。"}))).collect::<BTreeMap<_,_>>(),"sessions":snapshot.sessions.iter().map(|s|(s.id.clone(),session_detail(&snapshot,&s.id).unwrap())).collect::<BTreeMap<_,_>>()});
        fs::write(
            output.join(format!("{app}-agent-log-fixture.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
}
