use super::*;

pub(super) fn analyze(root: &Path, b: &mut Builder) {
    let projects = root.join("projects");
    let (mut paths, limited) = files(&projects, "jsonl");
    paths.retain(|p| modified(p) >= b.snapshot.since);
    paths.sort_by(|a, c| modified(c).cmp(&modified(a)).then_with(|| a.cmp(c)));
    let mut coverage = source(&projects, "conversation", "JSONL");
    coverage.partial = limited || paths.len() > MAX_FILES;
    coverage.note=format!("发现 {} 份时间窗内会话文件，最多读取最近 {MAX_FILES} 份，每份尾部 8 MB，总读取预算 64 MB。",paths.len());
    if let Err(error) = no_links(&projects) {
        coverage.status = "error".into();
        coverage.note = error;
    }
    let mut indexed = HashSet::new();
    for path in paths.iter().take(MAX_FILES) {
        if let Some(project) = path
            .strip_prefix(&projects)
            .ok()
            .and_then(|p| p.components().next())
        {
            let index = projects.join(project).join("sessions-index.json");
            if indexed.insert(index.clone()) && index.is_file() {
                read_index(&index, b);
            }
        }
        let bytes = fs::metadata(path)
            .map(|m| m.len().min(JSON_BYTES))
            .unwrap_or(0);
        if bytes > b.bytes_left {
            coverage.partial = true;
            coverage
                .note
                .push_str(" 总读取预算已用完，部分会话未读取。");
            break;
        }
        b.bytes_left = b.bytes_left.saturating_sub(bytes);
        let subagent = path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n == "subagents")
            || path
                .file_stem()
                .is_some_and(|n| n.to_string_lossy().starts_with("agent-"));
        let mut src = source(
            path,
            if subagent { "subagent" } else { "conversation" },
            "JSONL",
        );
        let fallback = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let parent = if subagent {
            path.parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };
        let mut no_time = 0;
        let result = json_lines(path, JSON_BYTES, |value, offset| {
            if at(&value["timestamp"]) == 0
                && !matches!(
                    text(&value, "type"),
                    "custom-title" | "file-history-snapshot" | "queue-operation"
                )
            {
                no_time += 1;
            }
            let n = b.events.len();
            line(
                &value,
                &fallback,
                parent.as_deref(),
                subagent,
                &src,
                offset,
                b,
            );
            if b.events.len() > n {
                src.records += b.events.len() - n;
            }
        });
        match result {
            Ok((_, partial, bad)) => {
                src.partial = partial || bad > 0 || no_time > 0;
                if partial {
                    src.note.push_str("大文件仅解析尾部 8 MB；");
                }
                if bad > 0 {
                    src.note
                        .push_str(&format!("{bad} 行无效、过大或仍在写入；"));
                }
                if no_time > 0 {
                    src.note
                        .push_str(&format!("{no_time} 行缺少时间，未纳入窗口统计；"));
                }
                src.note.push_str(
                    "仅提取消息 ID、工具状态、用量和错误类型。完整历史可能不在当前读取范围。",
                );
            }
            Err(error) => {
                src.status = "error".into();
                src.note = error;
            }
        }
        coverage.records += src.records;
        b.snapshot.sources.push(src);
    }
    b.snapshot.sources.insert(0, coverage);
    runtime::analyze(&root.join("debug"), "claude", b);
    b.snapshot.notes.push("Claude Code：相同 message.id 的流式内容块合并为一次模型响应；工具调用按 tool_use.id 去重，工具返回按 tool_use_id 关联。主会话与 subagents 独立计数。".into());
    b.snapshot.notes.push("Claude 输入总量 = 未缓存输入 + 缓存读取 + 缓存创建。模型响应记录不等于 HTTP 尝试；api_error 与重试单独显示。未启用 debug 时不会补造网络日志。".into());
}

fn read_index(path: &Path, b: &mut Builder) {
    let mut src = source(path, "index", "JSON");
    let bytes = src.size.min(MAX_LINE + 1);
    if bytes > b.bytes_left {
        src.partial = true;
        src.note = "总读取预算已用完，未读取会话索引。".into();
        b.snapshot.sources.push(src);
        return;
    }
    b.bytes_left -= bytes;
    match read_json(path) {
        Ok(v) => {
            for entry in v["entries"].as_array().into_iter().flatten().take(1000) {
                let id = text(entry, "sessionId");
                if id.is_empty() {
                    continue;
                }
                let session = b.session(id);
                // firstPrompt and fullPath are deliberately neither returned nor followed.
                if !text(entry, "summary").is_empty() {
                    session.title = short(text(entry, "summary"));
                }
                if !text(entry, "projectPath").is_empty() {
                    session.project = short(text(entry, "projectPath"));
                }
                session.updated_at = session.updated_at.max(at(&entry["modified"]));
                src.records += 1;
            }
            src.note =
                "只读取标题、项目和会话 ID，不读取 firstPrompt，不按索引路径打开额外文件。".into();
        }
        Err(e) => {
            src.status = "error".into();
            src.note = e;
        }
    }
    b.snapshot.sources.push(src);
}

fn tokens(v: &Value) -> Option<Tokens> {
    if !v.is_object() || !(v["input_tokens"].is_number() || v["output_tokens"].is_number()) {
        return None;
    }
    let cache_read = count(v, "cache_read_input_tokens");
    let cache_write = count(v, "cache_creation_input_tokens");
    Some(Tokens {
        input: count(v, "input_tokens")
            .saturating_add(cache_read)
            .saturating_add(cache_write),
        output: count(v, "output_tokens"),
        cache_read,
        cache_write,
        cache_write_1h: count(&v["cache_creation"], "ephemeral_1h_input_tokens").min(cache_write),
        reasoning: 0,
    })
}

pub(super) fn line(
    v: &Value,
    fallback: &str,
    parent: Option<&str>,
    subagent: bool,
    src: &Source,
    offset: u64,
    b: &mut Builder,
) {
    let main = if text(v, "sessionId").is_empty() {
        parent.unwrap_or(fallback)
    } else {
        text(v, "sessionId")
    };
    let id = if subagent {
        format!("{main}::{fallback}")
    } else {
        main.into()
    };
    let time = at(&v["timestamp"]);
    let session = b.session(&id);
    if !text(v, "cwd").is_empty() {
        session.project = short(text(v, "cwd"));
    }
    if subagent {
        session.parent_id = Some(main.into());
        session.agent = fallback.into();
    } else {
        session.agent = "主会话".into();
    }
    if time <= b.snapshot.generated_at {
        b.session(&id).updated_at = b.session(&id).updated_at.max(time);
    }
    let typ = text(v, "type");
    if typ == "custom-title" {
        if !text(v, "customTitle").is_empty() {
            b.session(&id).title = short(text(v, "customTitle"));
        }
        return;
    }
    let location = locate(src, Path::new(&src.path), format!("字节偏移 {offset}"));
    let uuid = if text(v, "uuid").is_empty() {
        format!("{}:{offset}", src.id)
    } else {
        text(v, "uuid").into()
    };
    let base = Event {
        time,
        session_id: Some(id.clone()),
        location,
        sequence: offset,
        target: "claude::conversation".into(),
        level: "INFO".into(),
        ..Default::default()
    };
    let m = &v["message"];
    if typ == "assistant" {
        let model = text(m, "model");
        let synthetic = model == "<synthetic>" || v["isApiErrorMessage"] == true;
        let msg = if text(m, "id").is_empty() {
            uuid.as_str()
        } else {
            text(m, "id")
        };
        if synthetic {
            let status = runtime::status(&v["error"]);
            let category = error_category(status, &format!("{} {}", v["error"], text(v, "error")));
            b.add(format!("{id}:api:{uuid}"),Event {
                kind:"api_error".into(),level:"ERROR".into(),failed:true,status,category:category.into(),
                summary:format!("API 错误 · {}",advice(category).0),
                evidence:json!({"recordType":typ,"synthetic":true,"messageId":msg,"status":status,"errorCategory":category}),..base
            });
            return;
        }
        let usage = tokens(&m["usage"]);
        let request_id = nonempty(text(v, "requestId"));
        b.add(format!("{id}:response:{msg}"),Event {
            kind:"response".into(),message_id:Some(msg.into()),model:short(model),usage,
            request_id:request_id.clone(),summary:format!("模型响应 · {} · {}",if model.is_empty(){"未记录模型"}else{model},if text(m,"stop_reason").is_empty(){"未记录结束原因"}else{text(m,"stop_reason")}),
            evidence:json!({"recordType":typ,"messageId":msg,"model":model,"requestId":request_id,"stopReason":m["stop_reason"]}),..base.clone()
        });
        for block in m["content"].as_array().into_iter().flatten() {
            if text(block, "type") != "tool_use" {
                continue;
            }
            let call = text(block, "id");
            if call.is_empty() {
                continue;
            }
            let name = short(text(block, "name"));
            b.add(format!("{id}:tool:{call}"),Event {
                kind:"tool".into(),message_id:Some(msg.into()),summary:format!("调用工具 {name}"),
                evidence:json!({"recordType":"tool_use","callId":call,"toolName":name,"messageId":msg}),..base.clone()
            });
        }
    } else if typ == "user" {
        let mut tool = false;
        for block in m["content"].as_array().into_iter().flatten() {
            if text(block, "type") != "tool_result" {
                continue;
            }
            tool = true;
            let call = text(block, "tool_use_id");
            if call.is_empty() {
                continue;
            }
            let failed = block["is_error"] == true;
            b.add(
                format!("{id}:result:{call}"),
                Event {
                    kind: "tool_result".into(),
                    failed,
                    level: if failed { "ERROR" } else { "INFO" }.into(),
                    category: if failed { "tool" } else { "" }.into(),
                    summary: if failed {
                        "工具返回失败"
                    } else {
                        "工具返回"
                    }
                    .into(),
                    duration_ms: v["toolUseResult"]["durationMs"].as_u64(),
                    evidence: json!({"recordType":"tool_result","callId":call,"isError":failed}),
                    ..base.clone()
                },
            );
        }
        if !tool && v["isMeta"] != true && v["isCompactSummary"] != true {
            b.add(
                format!("{id}:user:{uuid}"),
                Event {
                    kind: "user_turn".into(),
                    summary: "用户回合输入（正文已省略）".into(),
                    evidence: json!({"recordType":typ,"uuid":uuid}),
                    ..base
                },
            );
        }
    } else if typ == "system" {
        let subtype = text(v, "subtype");
        if subtype == "api_error" {
            let status = runtime::status(&v["error"]);
            let category = error_category(status, &v["error"].to_string());
            let retry = v["retryAttempt"].as_u64();
            let error_type = runtime::error_type(&v["error"]);
            b.add(format!("{id}:api:{uuid}"),Event {
                kind:"api_error".into(),failed:true,level:"ERROR".into(),status,category:category.into(),request_id:nonempty(text(&v["error"],"request_id")),
                summary:format!("API 错误 · {}{}",advice(category).0,status.map(|s|format!(" · HTTP {s}")).unwrap_or_default()),
                evidence:json!({"recordType":typ,"subtype":subtype,"errorType":error_type,"status":status,"retryAttempt":retry,"retryInMs":v["retryInMs"].as_u64(),"maxRetries":v["maxRetries"].as_u64()}),..base.clone()
            });
            if let Some(attempt) = retry.filter(|n| *n > 0) {
                b.add(format!("{id}:retry:{uuid}"),Event {kind:"retry".into(),level:"WARN".into(),category:"retry".into(),summary:format!("准备第 {attempt} 次重试"),evidence:json!({"retryAttempt":attempt,"retryInMs":v["retryInMs"].as_u64()}),..base});
            }
        } else if subtype == "compact_boundary" {
            b.add(format!("{id}:compact:{uuid}"),Event {kind:"compaction".into(),summary:"上下文压缩边界".into(),evidence:json!({"subtype":subtype,"trigger":v["compactMetadata"]["trigger"],"preTokens":v["compactMetadata"]["preTokens"].as_u64()}),..base});
        } else if subtype == "turn_duration" {
            b.add(
                format!("{id}:duration:{uuid}"),
                Event {
                    kind: "turn_duration".into(),
                    summary: "回合耗时记录".into(),
                    duration_ms: v["durationMs"].as_u64(),
                    evidence: json!({"subtype":subtype,"durationMs":v["durationMs"].as_u64()}),
                    ..base
                },
            );
        }
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| short(value))
}
