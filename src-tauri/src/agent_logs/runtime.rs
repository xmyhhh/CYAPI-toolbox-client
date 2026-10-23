use super::*;
use regex::Regex;

/// Error payloads vary by SDK/version. Never expose the payload or its message.
pub(super) fn status(v: &Value) -> Option<u16> {
    fn read(v: &Value, depth: u8) -> Option<u16> {
        if depth > 4 || !v.is_object() {
            return None;
        }
        for key in ["status", "statusCode", "status_code"] {
            if let Some(n) = v[key].as_u64().or_else(|| v[key].as_str()?.parse().ok()) {
                if (100..600).contains(&n) {
                    return Some(n as u16);
                }
            }
        }
        for key in ["error", "data", "response", "cause"] {
            if let Some(n) = read(&v[key], depth + 1) {
                return Some(n);
            }
        }
        None
    }
    read(v, 0)
}

fn identifier(s: &str) -> String {
    if !s.is_empty()
        && s.len() <= 160
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-.:/".contains(&c))
    {
        short(s)
    } else {
        String::new()
    }
}
pub(super) fn error_type(v: &Value) -> String {
    for obj in [v, &v["error"], &v["data"]["error"]] {
        for key in ["type", "name", "code"] {
            let s = identifier(text(obj, key));
            if !s.is_empty() {
                return s;
            }
        }
    }
    String::new()
}

// Both OpenCode generations use logfmt-like fields. Quoted values may include
// whitespace and escaped quotes. Unselected fields are discarded after parsing.
fn fields(line: &str) -> HashMap<String, String> {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut result = HashMap::new();
    let mut plain = Vec::new();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        if i == start || i == bytes.len() || bytes[i] != b'=' {
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i > start && plain.len() < 100 {
                plain.push(&line[start..i]);
            }
            continue;
        }
        let key = &line[start..i];
        i += 1;
        let start = i;
        let value;
        if i < bytes.len() && bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            value = serde_json::from_str::<String>(&line[start..i]).unwrap_or_default();
        } else {
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            value = line[start..i].into();
        }
        if key.len() <= 80 && result.len() < 100 {
            result.insert(key.into(), value);
        }
    }
    if !plain.is_empty() {
        result.insert("__plain_message".into(), plain.join(" "));
    }
    result
}

pub(super) fn analyze(root: &Path, app: &str, b: &mut Builder) {
    let mut coverage = source(root, "runtime", "text/logfmt");
    if let Err(e) = no_links(root) {
        coverage.status = "error".into();
        coverage.note = e;
        b.snapshot.sources.push(coverage);
        return;
    }
    let (mut paths, a) = files(root, "log");
    let (txt, c) = files(root, "txt");
    paths.extend(txt);
    paths.retain(|p| modified(p) >= b.snapshot.since);
    paths.sort_by(|a, c| modified(c).cmp(&modified(a)).then_with(|| a.cmp(c)));
    coverage.partial = a || c || paths.len() > 12;
    coverage.note = format!("发现 {} 份运行日志，最多读取最近 12 份，每份尾部 512 KB。只提取级别、状态、模块和显式会话 ID。", paths.len());
    for path in paths.iter().take(12) {
        let mut src = source(path, "runtime", "text/logfmt");
        match read_tail(path, &src, app, b) {
            Ok((records, partial, bad)) => {
                src.records = records;
                src.partial = partial || bad > 0;
                if partial {
                    src.note.push_str("仅解析尾部 512 KB；");
                }
                if bad > 0 {
                    src.note
                        .push_str(&format!("{bad} 行缺少有效时间、过大或无法识别；"));
                }
            }
            Err(e) => {
                src.status = "error".into();
                src.note = e;
            }
        }
        coverage.records += src.records;
        b.snapshot.sources.push(src);
    }
    if app == "claude" && paths.is_empty() {
        coverage
            .note
            .push_str(" 未发现 debug 日志；Claude Code 未启用调试记录时不会产生这些文件。");
    }
    b.snapshot.sources.push(coverage);
}

fn read_tail(
    path: &Path,
    src: &Source,
    app: &str,
    b: &mut Builder,
) -> Result<(usize, bool, usize), String> {
    no_links(path)?;
    let limit = (512 * 1024).min(b.bytes_left);
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let start = size.saturating_sub(limit);
    file.seek(SeekFrom::Start(start))
        .map_err(|e| e.to_string())?;
    let mut input = BufReader::new(file.take(limit));
    let mut offset = start;
    if start > 0 {
        offset += skip_line(&mut input)?;
    }
    let mut buf = Vec::new();
    let (mut records, mut bad) = (0, 0);
    loop {
        buf.clear();
        let n = (&mut input)
            .take(64 * 1024 + 1)
            .read_until(b'\n', &mut buf)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let pos = offset;
        offset += n as u64;
        if n > 64 * 1024 {
            if !buf.ends_with(b"\n") {
                offset += skip_line(&mut input)?;
            }
            bad += 1;
            continue;
        }
        let line = String::from_utf8_lossy(&buf);
        if line.trim().is_empty() {
            continue;
        }
        match event(&line, path, src, pos, app) {
            Ok(Some(event)) => {
                b.add(format!("runtime:{}:{pos}", src.id), event);
                records += 1;
            }
            Ok(None) => {}
            Err(()) => bad += 1,
        }
    }
    b.bytes_left = b.bytes_left.saturating_sub(size.min(limit));
    Ok((records, start > 0, bad))
}

fn event(
    line: &str,
    path: &Path,
    src: &Source,
    offset: u64,
    app: &str,
) -> Result<Option<Event>, ()> {
    static TIME: OnceLock<Regex> = OnceLock::new();
    static STATUS: OnceLock<Regex> = OnceLock::new();
    let f = fields(line);
    let get = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| f.get(*k))
            .map(String::as_str)
            .unwrap_or("")
    };
    let stamp = get(&["timestamp", "time"]);
    let time = if stamp.is_empty() {
        TIME.get_or_init(|| {
            Regex::new(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:\d\d)?").unwrap()
        })
        .find(line)
        .map(|m| {
            let time = at(&Value::String(m.as_str().into()));
            // Older OpenCode logger removes the zone suffix from an ISO UTC date.
            if time == 0 && app == "opencode" {
                at(&Value::String(format!("{}Z", m.as_str())))
            } else {
                time
            }
        })
        .unwrap_or(0)
    } else {
        at(&Value::String(stamp.into()))
    };
    if time == 0 {
        return Err(());
    }
    let mut level = get(&["level"]).to_uppercase();
    if level.is_empty() {
        level = ["ERROR", "WARN", "WARNING", "INFO", "DEBUG", "TRACE"]
            .iter()
            .find(|l| line.starts_with(**l) || line.contains(&format!("[{l}]")))
            .unwrap_or(&"INFO")
            .to_string();
    }
    if level == "WARNING" {
        level = "WARN".into();
    }
    if !["ERROR", "WARN", "INFO", "DEBUG", "TRACE"].contains(&level.as_str()) {
        level = "INFO".into();
    }
    let service = identifier(get(&["service", "module", "target", "type"]));
    let signal = if app == "opencode" && !f.is_empty() {
        format!(
            "{service} {}",
            get(&[
                "message",
                "msg",
                "error.type",
                "error.name",
                "__plain_message"
            ])
        )
    } else {
        line.into()
    };
    let status = get(&["http.status", "status", "statusCode"])
        .parse::<u16>()
        .ok()
        .filter(|n| (100..600).contains(n))
        .or_else(|| {
            STATUS
                .get_or_init(|| {
                    Regex::new(
                        r"(?i)\b(?:HTTP(?:/\d(?:\.\d)?)?\s+|status(?:Code)?[=: ]+)([1-5]\d\d)\b",
                    )
                    .unwrap()
                })
                .captures(&signal)
                .and_then(|c| c[1].parse().ok())
        });
    let lower = signal.to_lowercase();
    let retry = lower.contains("retry") || lower.contains("retrying");
    let mcp = service.to_lowercase().contains("mcp") || lower.contains("[mcp]");
    let http = !get(&["http.method", "method", "http.span"]).is_empty() || status.is_some();
    let failed = level == "ERROR" || status.is_some_and(|s| s >= 400);
    if !failed && level != "WARN" && !retry && !mcp && !http && get(&["event.type"]).is_empty() {
        return Ok(None);
    }
    let mut sid = identifier(get(&["sessionID", "sessionId", "session_id", "session.id"]));
    if sid.is_empty() {
        if let Some(data) = f
            .get("event.data")
            .filter(|s| s.len() <= 64 * 1024)
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
        {
            for obj in [
                &data,
                &data["properties"],
                &data["info"],
                &data["properties"]["info"],
            ] {
                sid = identifier(text(obj, "sessionID"));
                if !sid.is_empty() {
                    break;
                }
            }
        }
    }
    if sid.is_empty() && app == "claude" {
        if let Some(stem) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| uuid::Uuid::parse_str(s).is_ok())
        {
            sid = stem.into();
        }
    }
    let kind = if retry {
        "retry"
    } else if mcp {
        "mcp"
    } else if http {
        "http"
    } else {
        "runtime"
    };
    let category = if mcp {
        "mcp"
    } else if retry && !failed {
        "retry"
    } else if failed || level == "WARN" {
        let c = error_category(status, &lower);
        if c == "api" && !http {
            "runtime"
        } else {
            c
        }
    } else {
        ""
    };
    let duration = get(&["durationMs", "duration"])
        .trim_end_matches("ms")
        .parse::<u64>()
        .ok();
    let method = identifier(get(&["http.method", "method"]));
    let request = identifier(get(&["request-id", "request_id", "requestId"]));
    let summary = if http {
        format!(
            "运行日志 HTTP · {method} · {}",
            status
                .map(|s| s.to_string())
                .unwrap_or_else(|| "未记录状态".into())
        )
    } else if retry {
        "运行日志记录重试".into()
    } else if mcp {
        format!("MCP · {level}")
    } else {
        format!(
            "运行日志 · {}",
            advice(if failed || level == "WARN" {
                category
            } else {
                "runtime"
            })
            .0
        )
    };
    Ok(Some(Event {
        time,
        level,
        kind: kind.into(),
        target: if service.is_empty() {
            format!("{app}::runtime")
        } else {
            service.clone()
        },
        session_id: (!sid.is_empty()).then_some(sid),
        summary,
        failed,
        status,
        request_id: (!request.is_empty()).then_some(request.clone()),
        duration_ms: duration,
        location: locate(src, path, format!("字节偏移 {offset}")),
        sequence: offset,
        category: category.into(),
        evidence: json!({"format":"runtime","service":service,"httpMethod":method,"httpStatus":status,"requestId":request,"durationMs":duration,"eventType":identifier(get(&["event.type"])),"errorType":identifier(get(&["error.type","error.name","code"]))}),
        ..Default::default()
    }))
}
