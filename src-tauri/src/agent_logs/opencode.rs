use super::*;

pub(super) fn analyze(root: &Path, b: &mut Builder) {
    let path = root.join("opencode.db");
    let mut src = source(&path, "conversation", "SQLite");
    let mut staged = Builder::new("opencode", root, b.snapshot.window);
    staged.snapshot.since = b.snapshot.since;
    staged.snapshot.generated_at = b.snapshot.generated_at;
    let result = if path.is_file() {
        database(&path, &mut src, &mut staged)
    } else {
        Err("未发现 opencode.db".into())
    };
    match result {
        Ok(()) => {
            b.bytes_left = staged.bytes_left;
            b.sessions.extend(staged.sessions);
            for (key, event) in staged.events {
                b.add(key, event);
            }
            b.snapshot.notes.extend(staged.snapshot.notes);
            src.note.push_str(
                " 以只读事务同时读取会话、消息和工具状态，包含当前 WAL；未合并旧 JSON 存储。",
            );
        }
        Err(e) => {
            src.records = 0;
            if path.exists() {
                src.status = "error".into();
            }
            src.note = format!("{e}；尝试读取旧版 storage/session、message 和 part JSON。");
            legacy(root, b);
        }
    }
    b.snapshot.sources.insert(0, src);
    runtime::analyze(&root.join("log"), "opencode", b);
    b.snapshot.notes.push("OpenCode：session_message 与旧版 message/part 分别解析；模型响应使用消息级 tokens，step-finish 的重复用量不再相加。上下文压缩的独立用量单列事件。".into());
    b.snapshot.notes.push("OpenCode 输入 = tokens.input + cache.read + cache.write；输出 = tokens.output + tokens.reasoning，推理子项已包含在输出中。运行日志 HTTP 可能是本地服务调用，不作为模型响应统计。".into());
}

fn database(path: &Path, src: &mut Source, b: &mut Builder) -> Result<(), String> {
    let db = open_db(path)?;
    let v2 = columns(&db, "session_message");
    let v1 = columns(&db, "message");
    let required = |cols: &HashSet<String>| {
        ["id", "session_id", "data"]
            .iter()
            .all(|c| cols.contains(*c))
    };
    let (table, cols, session_table) = if required(&v2) && v2.contains("type") {
        ("session_message", &v2, "session_v2")
    } else if required(&v1) {
        ("message", &v1, "session")
    } else {
        return Err("数据库没有受支持的消息表（session_message 或 message）".into());
    };
    src.note = format!("已识别 {table} / {session_table} 结构。");
    if let Err(e) = sessions(&db, session_table, src, b) {
        src.partial = true;
        src.note
            .push_str(&format!(" 会话元数据读取失败：{e}；继续读取消息。"));
    }
    let time = col(cols, "time_created", "0");
    let updated = col(cols, "time_updated", &time);
    let typ = if table == "session_message" {
        "type"
    } else {
        "''"
    };
    // Evaluate the byte length in SQLite before transferring message bodies.
    let within = if cols.contains("time_created") || cols.contains("time_updated") {
        format!("{updated} >= ?1 OR {time} >= ?1")
    } else {
        "?1 >= 0".into()
    };
    let sql = format!("SELECT id, session_id, {time}, {typ}, CASE WHEN length(CAST(data AS BLOB)) <= {MAX_LINE} THEN data END, length(CAST(data AS BLOB)) FROM {table} WHERE {within} ORDER BY ({time} + 0) DESC, id DESC LIMIT ?2");
    let mut stmt = db.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            rusqlite::params![
                b.snapshot.since.min(i64::MAX as u64) as i64,
                (MAX_EVENTS + 1) as i64
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?.max(0) as u64,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?.max(0) as u64,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    let mut bad = 0;
    for (index, row) in rows.enumerate() {
        if index == MAX_EVENTS {
            src.partial = true;
            break;
        }
        let (id, sid, time, typ, data, size) = row.map_err(|e| e.to_string())?;
        if size > b.bytes_left {
            src.partial = true;
            src.note.push_str(" 消息读取预算 64 MB 已到达。");
            break;
        }
        b.bytes_left -= size;
        let Some(data) = data else {
            bad += 1;
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&data) else {
            bad += 1;
            continue;
        };
        let kind = if typ.is_empty() {
            text(&v, "role")
        } else {
            &typ
        };
        if self::time(&v, time) == 0 && matches!(kind, "assistant" | "compaction" | "user" | "idle")
        {
            bad += 1;
            continue;
        }
        message(
            &v,
            &id,
            &sid,
            time,
            kind,
            locate(src, path, format!("{table}.id = {id}")),
            b,
        );
        src.records += 1;
    }
    if table == "message" {
        if let Err(e) = parts(&db, path, src, b) {
            src.partial = true;
            src.note
                .push_str(&format!(" 工具 part 表读取失败：{e}；消息统计仍保留。"));
        }
    }
    if bad > 0 {
        src.partial = true;
        src.note
            .push_str(&format!(" {bad} 条消息无效、无时间或超过 2 MB，已跳过。"));
    }
    if !cols.contains("time_created") {
        src.partial = true;
        src.note
            .push_str(" 消息表缺少创建时间，仅采用 JSON 内时间，扫描限额可能影响覆盖。");
    }
    Ok(())
}

fn sessions(db: &Connection, table: &str, src: &mut Source, b: &mut Builder) -> Result<(), String> {
    let cols = columns(db, table);
    if !cols.contains("id") {
        src.note
            .push_str(" 会话元数据表缺失，仍从消息提取会话 ID。");
        return Ok(());
    }
    let expressions = [
        "title",
        "directory",
        "parent_id",
        "fork_session_id",
        "agent",
        "model",
    ]
    .map(|c| col(&cols, c, "''"));
    let time = col(&cols, "time_updated", "0");
    let sql = format!(
        "SELECT id, {}, {time} FROM {table} ORDER BY ({time} + 0) DESC, id LIMIT 5001",
        expressions.join(",")
    );
    let mut stmt = db.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, i64>(7)?.max(0) as u64,
            ))
        })
        .map_err(|e| e.to_string())?;
    for (index, row) in rows.enumerate() {
        if index == 5000 {
            src.partial = true;
            src.note.push_str(" 会话元数据只读取最新 5,000 条。");
            break;
        }
        let (id, title, project, parent, fork, agent, model, time) =
            row.map_err(|e| e.to_string())?;
        let s = b.session(&id);
        s.title = short(&title);
        s.project = short(&project);
        s.updated_at = at(&json!(time));
        s.parent_id = (!parent.is_empty()).then(|| short(&parent));
        s.forked_from = (!fork.is_empty()).then(|| short(&fork));
        s.agent = short(&agent);
        let value = serde_json::from_str::<Value>(&model).unwrap_or(Value::String(model));
        s.model = model_id(&value);
        s.provider = provider(&value);
    }
    Ok(())
}

fn parts(db: &Connection, path: &Path, src: &mut Source, b: &mut Builder) -> Result<(), String> {
    let cols = columns(db, "part");
    if !["id", "session_id", "message_id", "data"]
        .iter()
        .all(|c| cols.contains(*c))
    {
        src.partial = true;
        src.note
            .push_str(" 缺少兼容的 part 表，工具状态可能不完整。");
        return Ok(());
    }
    let time = col(&cols, "time_created", "0");
    let updated = col(&cols, "time_updated", &time);
    let within = if cols.contains("time_created") || cols.contains("time_updated") {
        format!("{updated} >= ?1 OR {time} >= ?1")
    } else {
        "?1 >= 0".into()
    };
    let sql = format!("SELECT id, session_id, message_id, {time}, CASE WHEN length(CAST(data AS BLOB)) <= {MAX_LINE} THEN data END, length(CAST(data AS BLOB)) FROM part WHERE {within} ORDER BY ({time} + 0) DESC, id DESC LIMIT ?2");
    let mut stmt = db.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            rusqlite::params![
                b.snapshot.since.min(i64::MAX as u64) as i64,
                (MAX_EVENTS + 1) as i64
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?.max(0) as u64,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?.max(0) as u64,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    let mut bad = 0;
    for (index, row) in rows.enumerate() {
        if index == MAX_EVENTS {
            src.partial = true;
            break;
        }
        let (id, sid, mid, time, data, size) = row.map_err(|e| e.to_string())?;
        if size > b.bytes_left {
            src.partial = true;
            break;
        }
        b.bytes_left -= size;
        let Some(v) = data.and_then(|d| serde_json::from_str::<Value>(&d).ok()) else {
            bad += 1;
            continue;
        };
        part(
            &v,
            &id,
            &sid,
            &mid,
            time,
            locate(src, path, format!("part.id = {id}")),
            b,
        );
        src.records += 1;
    }
    if bad > 0 {
        src.partial = true;
        src.note
            .push_str(&format!(" {bad} 条 part 无效或超过 2 MB。"));
    }
    Ok(())
}

fn model_id(v: &Value) -> String {
    short(v.as_str().unwrap_or_else(|| {
        if text(v, "id").is_empty() {
            text(v, "modelID")
        } else {
            text(v, "id")
        }
    }))
}
fn provider(v: &Value) -> String {
    short(text(v, "providerID"))
}
fn tokens(v: &Value) -> Option<Tokens> {
    if !v.is_object()
        || !["input", "output", "reasoning", "cache"]
            .iter()
            .any(|k| v.get(*k).is_some())
    {
        return None;
    }
    let cache_read = count(&v["cache"], "read");
    let cache_write = count(&v["cache"], "write");
    let reasoning = count(v, "reasoning");
    Some(Tokens {
        input: count(v, "input")
            .saturating_add(cache_read)
            .saturating_add(cache_write),
        output: count(v, "output").saturating_add(reasoning),
        cache_read,
        cache_write,
        reasoning,
        ..Default::default()
    })
}
fn time(v: &Value, fallback: u64) -> u64 {
    [
        at(&v["time"]["created"]),
        at(&v["time"]["start"]),
        at(&v["time"]),
        at(&json!(fallback)),
    ]
    .into_iter()
    .find(|t| *t > 0)
    .unwrap_or(0)
}
fn elapsed(v: &Value) -> Option<u64> {
    let start = [at(&v["created"]), at(&v["start"])]
        .into_iter()
        .find(|t| *t > 0)?;
    let end = [at(&v["completed"]), at(&v["end"])]
        .into_iter()
        .find(|t| *t > 0)?;
    end.checked_sub(start)
}

fn message(
    v: &Value,
    id: &str,
    sid: &str,
    fallback_time: u64,
    kind: &str,
    location: Location,
    b: &mut Builder,
) {
    let time = time(v, fallback_time);
    let mut model = if text(v, "modelID").is_empty() {
        model_id(&v["model"])
    } else {
        short(text(v, "modelID"))
    };
    let mut provider = if text(v, "providerID").is_empty() {
        provider(&v["model"])
    } else {
        short(text(v, "providerID"))
    };
    if let Some(s) = b.sessions.get(sid) {
        if model.is_empty() {
            model = s.model.clone();
        }
        if provider.is_empty() {
            provider = s.provider.clone();
        }
    }
    if !text(&v["path"], "cwd").is_empty() {
        b.session(sid).project = short(text(&v["path"], "cwd"));
    }
    if !text(v, "agent").is_empty() {
        b.session(sid).agent = short(text(v, "agent"));
    }
    let base = Event {
        time,
        session_id: Some(sid.into()),
        message_id: Some(id.into()),
        model,
        provider,
        location,
        level: "INFO".into(),
        target: "opencode::session".into(),
        ..Default::default()
    };
    if kind == "assistant" || kind == "compaction" {
        let failed = !v["error"].is_null() && v["error"] != false
            || kind == "compaction" && text(v, "status") == "failed";
        let status = runtime::status(&v["error"]);
        let category = if kind == "compaction" && failed {
            "compaction"
        } else if failed {
            error_category(status, &v["error"].to_string())
        } else {
            ""
        };
        let error_type = runtime::error_type(&v["error"]);
        let finish = short(text(v, "finish"));
        let completed = at(&v["time"]["completed"]) > 0;
        b.add(format!("{sid}:{kind}:{id}"), Event {
            kind: if kind == "assistant" { "response" } else { "compaction" }.into(),
            summary: format!("{} · {}", if kind == "compaction" { "上下文压缩" } else { "模型响应" }, if failed { advice(category).0 } else if completed || text(v, "status") == "completed" { "已记录完成" } else { "未记录完成状态" }),
            failed, level: if failed { "ERROR" } else { "INFO" }.into(), status, category: category.into(), usage: tokens(&v["tokens"]), duration_ms: elapsed(&v["time"]),
            evidence: json!({"recordType":kind,"messageId":id,"completed":completed,"finish":finish,"status":status,"errorType":error_type,"compactionStatus":if kind == "compaction" { Some(short(text(v,"status"))) } else { None }}), ..base.clone()
        });
        for (index, content) in v["content"].as_array().into_iter().flatten().enumerate() {
            if text(content, "type") == "tool" {
                tool(
                    content,
                    &format!("{id}:{index}"),
                    sid,
                    id,
                    time,
                    base.location.clone(),
                    b,
                );
            }
        }
    } else if kind == "user" {
        b.add(
            format!("{sid}:user:{id}"),
            Event {
                kind: "user_turn".into(),
                summary: "用户回合输入（正文已省略）".into(),
                evidence: json!({"recordType":kind,"messageId":id}),
                ..base
            },
        );
    } else if kind == "idle" {
        b.add(
            format!("{sid}:idle:{id}"),
            Event {
                kind: "idle".into(),
                summary: "会话空闲状态记录".into(),
                evidence: json!({"recordType":kind,"outcome":short(text(v,"outcome"))}),
                ..base
            },
        );
    }
}

fn tool(
    v: &Value,
    fallback: &str,
    sid: &str,
    mid: &str,
    fallback_time: u64,
    location: Location,
    b: &mut Builder,
) {
    let call = [text(v, "callID"), text(v, "id"), fallback]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or(fallback);
    let name = short(if text(v, "tool").is_empty() {
        text(v, "name")
    } else {
        text(v, "tool")
    });
    let state = &v["state"];
    let status = short(text(state, "status"));
    let failed = matches!(status.as_str(), "error" | "failed");
    let time = time(state, 0).max(time(v, fallback_time));
    b.add(format!("{sid}:tool:{call}"), Event {
        time, kind: "tool".into(), target: "opencode::tool".into(), session_id: Some(sid.into()), message_id: Some(mid.into()),
        summary: format!("工具 {name} · {}", if status.is_empty() { "未记录状态" } else { &status }),
        failed, level: if failed { "ERROR" } else { "INFO" }.into(), category: if failed { "tool" } else { "" }.into(), duration_ms: elapsed(&state["time"]).or_else(|| elapsed(&v["time"])), location,
        evidence: json!({"recordType":"tool","callId":call,"toolName":name,"state":status,"executed":v["executed"].as_bool()}), ..Default::default()
    });
}

fn part(
    v: &Value,
    id: &str,
    sid: &str,
    mid: &str,
    fallback_time: u64,
    location: Location,
    b: &mut Builder,
) {
    if text(v, "type") == "tool" {
        tool(v, id, sid, mid, fallback_time, location, b);
    } else if matches!(text(v, "type"), "step-start" | "step-finish") {
        b.add(format!("{sid}:step:{id}"), Event { time: time(v, fallback_time), kind: "step".into(), target: "opencode::step".into(), level: "INFO".into(), session_id: Some(sid.into()), message_id: Some(mid.into()), summary: "模型步骤（用量已由消息记录统计）".into(), location, evidence: json!({"recordType":text(v,"type"),"partId":id,"reason":short(text(v,"reason"))}), ..Default::default() });
    }
}

fn legacy(root: &Path, b: &mut Builder) {
    let storage = root.join("storage");
    let mut coverage = source(&storage, "legacy", "JSON");
    coverage.note =
        "仅在 SQLite 缺失或无法解析时回退。会话元数据最多 50 份、消息 100 份、工具 50 份，总读取预算 64 MB。".into();
    let mut loaded = 0;
    for (folder, limit) in [("session", 50), ("message", 100), ("part", 50)] {
        let (mut paths, limited) = files(&storage.join(folder), "json");
        coverage.partial |= limited;
        // Session metadata is cheap but may predate the selected messages.
        if folder != "session" {
            paths.retain(|p| modified(p) >= b.snapshot.since);
        }
        paths.sort_by(|a, c| modified(c).cmp(&modified(a)).then_with(|| a.cmp(c)));
        coverage.partial |= paths.len() > limit;
        for path in paths.into_iter().take(limit) {
            if loaded >= MAX_FILES {
                coverage.partial = true;
                break;
            }
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if size > b.bytes_left {
                coverage.partial = true;
                break;
            }
            b.bytes_left -= size;
            loaded += 1;
            let mut src = source(&path, "legacy", "JSON");
            match read_json(&path) {
                Ok(v) => {
                    let file_id = path.file_stem().unwrap_or_default().to_string_lossy();
                    let id = if text(&v, "id").is_empty() {
                        file_id.as_ref()
                    } else {
                        text(&v, "id")
                    };
                    let parent = path
                        .parent()
                        .and_then(Path::file_name)
                        .unwrap_or_default()
                        .to_string_lossy();
                    let location = locate(&src, &path, "JSON 结构字段".into());
                    if folder == "session" {
                        let s = b.session(id);
                        s.title = short(text(&v, "title"));
                        s.project = short(text(&v, "directory"));
                        s.updated_at = at(&v["time"]["updated"]);
                        s.parent_id =
                            (!text(&v, "parentID").is_empty()).then(|| short(text(&v, "parentID")));
                    } else if folder == "message" {
                        let sid = if text(&v, "sessionID").is_empty() {
                            parent.as_ref()
                        } else {
                            text(&v, "sessionID")
                        };
                        message(&v, id, sid, 0, text(&v, "role"), location, b);
                    } else {
                        let mid = if text(&v, "messageID").is_empty() {
                            parent.as_ref()
                        } else {
                            text(&v, "messageID")
                        };
                        let sid = text(&v, "sessionID");
                        if !sid.is_empty() {
                            part(&v, id, sid, mid, 0, location, b);
                        } else {
                            src.partial = true;
                            src.note = "part 缺少 sessionID，未猜测所属会话。".into();
                        }
                    }
                    src.records = 1;
                    coverage.records += 1;
                }
                Err(e) => {
                    src.status = "error".into();
                    src.note = e;
                }
            }
            b.snapshot.sources.push(src);
        }
    }
    b.snapshot.sources.push(coverage);
}
