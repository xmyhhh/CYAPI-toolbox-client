//! Local diagnostics: no model requests, permission changes or sandbox bypasses.
use crate::{configuration::no_links, logging};
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub name: String,
    pub status: String,
    pub message: String,
    pub os_error_code: Option<i32>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: String,
    pub title: String,
    pub status: String,
    pub message: String,
    pub path: Option<String>,
    pub advice: String,
    pub steps: Vec<Step>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub id: String,
    pub generated_at: u64,
    pub environment: String,
    pub codex_home: String,
    pub data_path: String,
    pub checks: Vec<Check>,
}

pub fn home_path() -> PathBuf {
    env::var_os("CYAPI_TOOLBOX_HOME_DIR")
        .or_else(|| env::var_os(crate::platform::home_variable()))
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
}
pub fn codex_home() -> PathBuf {
    if env::var_os("CYAPI_TOOLBOX_HOME_DIR").is_none() {
        if let Some(value) = env::var_os("CODEX_HOME") {
            return PathBuf::from(value);
        }
    }
    home_path().join(".codex")
}
fn check(
    id: &str,
    title: &str,
    status: &str,
    message: impl Into<String>,
    path: Option<&Path>,
    advice: &str,
) -> Check {
    Check {
        id: id.into(),
        title: title.into(),
        status: status.into(),
        message: message.into(),
        path: path.map(|p| p.to_string_lossy().into()),
        advice: advice.into(),
        steps: vec![],
    }
}
fn local_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.to_string_lossy().starts_with(r"\\") {
        return Err("请填写本机磁盘的绝对路径；此版本不对网络路径或设备路径执行检测".into());
    }
    no_links(path)
}

fn permission_check(path: &Path, title: &str, write_test: bool) -> Check {
    let mut result = check(
        &format!("directory-{title}"),
        title,
        "pass",
        "目录存在；尚未执行写入测试",
        Some(path),
        "写入测试仅代表当前工具箱进程；已有文件的权限与 Codex 沙盒需分别确认。",
    );
    if let Err(message) = local_path(path) {
        result.status = "unknown".into();
        result.message = message;
        return result;
    }
    match fs::metadata(path) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => {
            result.status = "fail".into();
            result.message = "此路径不是文件夹".into();
            return result;
        }
        Err(e) => {
            result.status = if e.kind() == std::io::ErrorKind::NotFound {
                "warn"
            } else {
                "fail"
            }
            .into();
            result.message = format!("无法访问目录：{}（系统错误 {:?}）", e, e.raw_os_error());
            result.advice = "确认目录存在并检查当前用户的访问权限；不会自动创建目标目录。".into();
            return result;
        }
    }
    if !write_test {
        result.status = "unknown".into();
        return result;
    }
    let probe = path.join(format!(".cyapi-permission-test-{}", uuid::Uuid::new_v4()));
    let first = probe.join("probe.txt");
    let renamed = probe.join("renamed.txt");
    let mut created = false;
    let mut execute = |name: &str, action: &mut dyn FnMut() -> std::io::Result<()>| -> bool {
        match action() {
            Ok(()) => {
                result.steps.push(Step {
                    name: name.into(),
                    status: "pass".into(),
                    message: "成功".into(),
                    os_error_code: None,
                });
                true
            }
            Err(e) => {
                result.steps.push(Step {
                    name: name.into(),
                    status: "fail".into(),
                    message: logging::redact(&e.to_string()),
                    os_error_code: e.raw_os_error(),
                });
                false
            }
        }
    };
    let mut create = || {
        fs::create_dir(&probe)?;
        created = true;
        Ok(())
    };
    let mut ok = execute("创建测试目录", &mut create);
    if ok {
        ok = execute("创建并写入文件", &mut || {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&first)?;
            file.write_all(b"CYAPI permission probe\n")?;
            file.sync_all()
        });
    }
    if ok {
        ok = execute("读回校验", &mut || {
            if fs::read(&first)? != b"CYAPI permission probe\n" {
                return Err(std::io::Error::other("写入内容校验失败"));
            }
            Ok(())
        });
    }
    if ok {
        ok = execute("重命名", &mut || fs::rename(&first, &renamed));
    }
    if ok {
        ok = execute("删除测试文件", &mut || fs::remove_file(&renamed));
    }
    drop(execute);
    // Only remove known files and the exclusively created directory, never recurse.
    if created {
        let cleanup = (|| -> std::io::Result<()> {
            no_links(&probe).map_err(std::io::Error::other)?;
            for file in [&first, &renamed] {
                match fs::remove_file(file) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
            fs::remove_dir(&probe)
        })();
        result.steps.push(match cleanup {
            Ok(()) => Step {
                name: "清理测试目录".into(),
                status: "pass".into(),
                message: "测试对象已清理".into(),
                os_error_code: None,
            },
            Err(e) => {
                ok = false;
                Step {
                    name: "清理测试目录".into(),
                    status: "fail".into(),
                    message: format!("残留路径：{}；{}", probe.display(), e),
                    os_error_code: e.raw_os_error(),
                }
            }
        });
    }
    result.status = if ok { "pass" } else { "fail" }.into();
    result.message = if ok {
        "创建、写入、校验、重命名、删除及清理均成功".into()
    } else {
        "文件操作失败；请展开查看具体步骤与系统错误码".into()
    };
    result
}

fn read_limited(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    local_path(path)?;
    let file =
        fs::File::open(path).map_err(|e| format!("读取失败（系统错误 {:?}）", e.raw_os_error()))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("请选择普通文件".into());
    }
    let mut bytes = vec![];
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err("文件超过读取上限".into());
    }
    Ok(bytes)
}

fn configuration_check(root: &Path) -> Vec<Check> {
    let path = root.join("config.toml");
    if !path.exists() {
        return vec![
            check(
                "config",
                "Codex 全局配置",
                "warn",
                "未找到 config.toml；可能使用默认值或其他执行环境",
                Some(&path),
                "确认 CODEX_HOME 与实际运行的 Codex 环境一致。",
            ),
            check(
                "sandbox",
                "沙盒与审批策略",
                "unknown",
                "未执行 Codex 沙盒内探针，当前无法验证",
                None,
                "结合实际会话的日志与生效配置核对。",
            ),
        ];
    }
    let parsed = read_limited(&path, 1024 * 1024)
        .and_then(|bytes| String::from_utf8(bytes).map_err(|_| "配置不是有效 UTF-8".into()))
        .and_then(|value| {
            value.parse::<toml_edit::DocumentMut>().map_err(|e| {
                // Parser display may contain credential-bearing source lines.
                let location = e
                    .span()
                    .map(|s| format!("，错误字节位置 {}", s.start))
                    .unwrap_or_default();
                format!("TOML 语法无效{location}；请在配置页检查，原文未写入日志")
            })
        });
    match parsed {
        Ok(doc) => {
            let policy = ["sandbox_mode", "approval_policy", "profile"]
                .iter()
                .filter_map(|key| {
                    doc.get(key)
                        .and_then(|v| v.as_str())
                        .map(|v| format!("{key} = {}", logging::redact(v)))
                })
                .collect::<Vec<_>>()
                .join("；");
            vec![check("config","Codex 全局配置","pass","TOML 语法有效；未验证所有版本字段及项目配置的覆盖关系",Some(&path),"可在 Codex 配置页查看和修改全局配置。"),
                check("sandbox","沙盒与审批策略","unknown",if policy.is_empty() {"全局文件未显式设置沙盒/审批策略。当前未执行 Codex 沙盒内探针。".into()} else {format!("文件声明：{policy}。当前未执行 Codex 沙盒内探针。")},Some(&path),"结合发生错误的会话、项目配置与日志核对实际生效策略；普通目录写入成功不能证明沙盒内可写。")]
        }
        Err(e) => vec![
            check(
                "config",
                "Codex 全局配置",
                "fail",
                e,
                Some(&path),
                "先修复配置语法或读取权限，再重新检测。",
            ),
            check(
                "sandbox",
                "沙盒与审批策略",
                "unknown",
                "配置尚不可读，无法分析策略；未执行沙盒内探针",
                None,
                "先检查配置，再结合对应会话日志分析。",
            ),
        ],
    }
}

pub fn run(project: &str, write_test: bool) -> Report {
    let id = uuid::Uuid::new_v4().to_string();
    logging::with_operation(format!("diagnostics-{id}"), || {
        logging::event(
            "info",
            "diagnostics",
            if write_test {
                "开始本机排查与目录写入测试"
            } else {
                "开始本机只读排查"
            },
        );
        let root = codex_home();
        let data = logging::data_path();
        let mut checks = configuration_check(&root);
        for (path, title) in [
            (root.clone(), "Codex 主目录"),
            (root.join("skills"), "Codex Skill 目录"),
            (data.clone(), "工具箱数据目录"),
            (data.join("cache"), "安装缓存目录"),
            (data.join("backups"), "安装备份目录"),
            (env::temp_dir(), "系统临时目录"),
        ] {
            let result = permission_check(&path, title, write_test);
            logging::event(
                if result.status == "fail" {
                    "error"
                } else {
                    "info"
                },
                "diagnostics",
                format!("{}：{}", result.title, result.message),
            );
            checks.push(result);
        }
        if !project.trim().is_empty() {
            checks.push(permission_check(
                Path::new(project.trim()),
                "项目目录",
                write_test,
            ));
        }
        let pending = data.join("pending.json");
        checks.push(check(
            "recovery",
            "安装恢复状态",
            if pending.exists() { "warn" } else { "pass" },
            if pending.exists() {
                "检测到未完成的安装/卸载记录；请查看启动与恢复日志"
            } else {
                "未检测到待恢复的安装记录"
            },
            Some(&pending),
            "保留恢复记录和原文件，先查看工具箱日志再处理。",
        ));
        let files = discover_logs();
        checks.push(check(
            "codex-logs",
            "Codex 日志来源",
            if files.is_empty() { "unknown" } else { "pass" },
            format!(
                "找到 {} 个候选日志文件；请在「Codex 日志」中查看。",
                files.len()
            ),
            None,
            "不同版本位置可能不同；也可以手动输入日志文件绝对路径。",
        ));
        logging::event(
            "info",
            "diagnostics",
            "排查完成；沙盒内能力仍需结合实际会话验证",
        );
        Report {
            id,
            generated_at: logging::now(),
            environment: format!(
                "{} 本机 · 当前工具箱用户",
                if cfg!(windows) {
                    "Windows"
                } else if cfg!(target_os = "macos") {
                    "macOS"
                } else {
                    "Linux"
                }
            )
            .into(),
            codex_home: root.to_string_lossy().into(),
            data_path: data.to_string_lossy().into(),
            checks,
        }
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFile {
    path: String,
    name: String,
    source: String,
    size: u64,
    modified_at: u64,
}

pub fn discover_logs() -> Vec<LogFile> {
    let root = codex_home();
    let mut dirs = vec![
        (root.join("log"), "Codex 主目录候选"),
        (root.join("logs"), "Codex 主目录候选"),
    ];
    if let Some(roaming) = env::var_os("APPDATA") {
        dirs.push((PathBuf::from(roaming).join("Codex/logs"), "Codex 应用候选"));
    }
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        dirs.push((local.join("Codex/logs"), "Codex 应用候选"));
        // Inspect only Codex package directories; never scan arbitrary user data.
        for entry in fs::read_dir(local.join("Packages"))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .take(2000)
        {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with("OpenAI.Codex_")
            {
                dirs.push((
                    entry.path().join("LocalCache/Roaming/Codex/logs"),
                    "Codex Store 应用候选",
                ));
            }
        }
    }
    // A macOS build keeps the application logs in the home directory.
    let home = home_path();
    dirs.push((home.join("Library/Logs/Codex"), "Codex 应用候选"));
    dirs.push((home.join(".config/Codex/logs"), "Codex 应用候选"));
    let mut files = vec![];
    for (directory, source) in dirs {
        if local_path(&directory).is_err() {
            continue;
        }
        let mut candidates = vec![directory];
        // At most one level of dated log directories, with a strict count bound.
        if let Ok(entries) = fs::read_dir(&candidates[0]) {
            candidates.extend(
                entries
                    .filter_map(Result::ok)
                    .take(100)
                    .map(|e| e.path())
                    .filter(|p| p.is_dir() && no_links(p).is_ok()),
            );
        }
        for directory in candidates {
            for entry in fs::read_dir(directory)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .take(300)
            {
                let path = entry.path();
                if !matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("log" | "jsonl" | "txt")
                ) || no_links(&path).is_err()
                {
                    continue;
                }
                if let Ok(meta) = entry.metadata() {
                    if !meta.is_file() {
                        continue;
                    }
                    files.push(LogFile {
                        path: path.to_string_lossy().into(),
                        name: entry.file_name().to_string_lossy().into(),
                        source: source.into(),
                        size: meta.len(),
                        modified_at: meta
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|v| v.as_millis() as u64)
                            .unwrap_or(0),
                    });
                }
            }
        }
    }
    files.sort_by_key(|v| std::cmp::Reverse(v.modified_at));
    files.dedup_by(|a, b| a.path == b.path);
    files.truncate(40);
    files
}

pub fn read_log(path: &Path) -> Result<serde_json::Value, String> {
    local_path(path)?;
    if !matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("log" | "jsonl" | "txt")
    ) {
        return Err("仅支持 .log、.jsonl、.txt 日志文件".into());
    }
    let mut file = fs::File::open(path).map_err(|e| format!("无法打开日志：{}", e))?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("请选择普通日志文件".into());
    }
    let start = meta.len().saturating_sub(256 * 1024) & !1;
    file.seek(SeekFrom::Start(start))
        .map_err(|e| e.to_string())?;
    let mut bytes = vec![];
    file.take(256 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let utf16 = bytes.starts_with(&[0xff, 0xfe])
        || bytes
            .iter()
            .skip(1)
            .step_by(2)
            .take(100)
            .filter(|b| **b == 0)
            .count()
            > 15;
    let text = if utf16 {
        String::from_utf16_lossy(
            &bytes
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        )
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let text = if start > 0 {
        text.split_once('\n').map(|v| v.1).unwrap_or("")
    } else {
        &text
    };
    let mut lines: Vec<_> = logging::redact(text)
        .lines()
        .map(|line| line.chars().take(4000).collect::<String>())
        .collect();
    let truncated = start > 0 || lines.len() > 1500;
    if lines.len() > 1500 {
        lines.drain(..lines.len() - 1500);
    }
    let hints = analyze(&lines.join("\n"));
    Ok(serde_json::json!({"path":path,"lines":lines,"truncated":truncated,"hints":hints}))
}

pub fn analyze(text: &str) -> Vec<Check> {
    let lower = text.to_lowercase();
    let rules = [
        (
            "sandbox",
            "沙盒相关线索",
            &["sandbox", "appcontainer", "沙盒"][..],
            "核对同一时间段的启动错误、工作目录和会话策略；关键词命中不能单独确定根因。",
        ),
        (
            "permission",
            "文件访问相关线索",
            &[
                "permission denied",
                "access is denied",
                "access denied",
                "os error 5",
                "eperm",
                "eacces",
                "拒绝访问",
            ][..],
            "对报错中的具体目录执行写入测试，检查已有文件权限及占用；沙盒内的拒绝需要另行确认。",
        ),
        (
            "approval",
            "审批相关线索",
            &["approval", "not approved", "审批"][..],
            "查看对应工具调用的审批结果与实际生效策略。",
        ),
        (
            "network",
            "网络或服务相关线索",
            &[
                "timed out",
                "connection refused",
                "tls",
                "certificate",
                "401 unauthorized",
                "403 forbidden",
            ][..],
            "检查服务地址、代理、证书和登录状态；网络错误不能用修改目录权限解决。",
        ),
    ];
    rules
        .iter()
        .filter_map(|(id, title, keywords, advice)| {
            if !keywords.iter().any(|k| lower.contains(k)) {
                return None;
            }
            let evidence = text
                .lines()
                .filter(|line| keywords.iter().any(|k| line.to_lowercase().contains(k)))
                .take(3)
                .map(|s| s.chars().take(280).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            Some(check(
                id,
                title,
                "warn",
                logging::redact(&evidence),
                None,
                advice,
            ))
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticRequest {
    pub action: String,
    pub path: Option<String>,
    pub write_test: Option<bool>,
    pub text: Option<String>,
    pub report: Option<Report>,
    pub operation_id: Option<String>,
}

#[tauri::command]
pub async fn diagnostics(request: DiagnosticRequest) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || match request.action.as_str() {
        "run"=>serde_json::to_value(run(request.path.as_deref().unwrap_or(""),request.write_test.unwrap_or(false))).map_err(|e|e.to_string()),
        "log-files"=>Ok(serde_json::json!({"files":discover_logs(),"codexHome":codex_home(),"dataPath":logging::data_path()})),
        "read-log"=>read_log(Path::new(request.path.as_deref().ok_or("缺少日志路径")?)),
        "analyze"=>Ok(serde_json::json!({"hints":analyze(&request.text.unwrap_or_default().chars().take(64_000).collect::<String>())})),
        "export"=> {
            let dir=logging::data_path().join("reports"); fs::create_dir_all(&dir).map_err(|e|e.to_string())?;
            no_links(&dir)?;
            let mut old: Vec<_> = fs::read_dir(&dir).map_err(|e|e.to_string())?.filter_map(Result::ok).map(|e|e.path()).filter(|p|p.file_name().is_some_and(|s|s.to_string_lossy().starts_with("diagnostic-")) && p.extension().is_some_and(|s|s=="json") && no_links(p).is_ok()).collect();
            old.sort();
            let remove_count=old.len().saturating_sub(19);
            for path in old.iter().take(remove_count) { let _=fs::remove_file(path); }
            let name=format!("diagnostic-{}-{}.json",logging::now(),uuid::Uuid::new_v4()); let path=dir.join(name);
            let logs=logging::logs_snapshot(None);
            let entries=logging::export_entries(request.operation_id.as_deref());
            let mut report=serde_json::json!({"schemaVersion":1,"version":env!("CARGO_PKG_VERSION"),"generatedAt":logging::now(),"report":request.report,"logs":entries,"droppedLogWrites":logs["dropped"],"note":"仅含所选检查结果与当前缓冲区内最多 5000 条脱敏工具箱日志，外部 Codex 日志未自动导出。"});
            fn sanitize(value: &mut serde_json::Value) {
                match value {
                    serde_json::Value::String(text) => *text=logging::export_redact(text),
                    serde_json::Value::Array(items) => items.iter_mut().for_each(sanitize),
                    serde_json::Value::Object(items) => items.values_mut().for_each(sanitize),
                    _ => {}
                }
            }
            sanitize(&mut report);
            let text=serde_json::to_string_pretty(&report).map_err(|e|e.to_string())?;
            let mut file=OpenOptions::new().create_new(true).write(true).open(&path).map_err(|e|e.to_string())?;file.write_all(text.as_bytes()).map_err(|e|e.to_string())?;
            Ok(serde_json::json!({"path":path}))
        },
        "open-logs"=> {
            let path=logging::current_directory().ok_or("日志尚未写入文件；可在窗口查看内存日志")?;
            crate::platform::open_path(&path)?;
            Ok(serde_json::json!({"ok":true}))
        },
        _=>Err("未知排查操作".into())
    }).await.map_err(|e|e.to_string())?.map_err(|e|logging::redact(&e))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_roundtrip_preserves_existing_files_and_cleans_up() {
        let root = env::temp_dir().join(format!("cyapi-probe-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("keep.txt"), b"keep").unwrap();
        let result = permission_check(&root, "test", true);
        assert_eq!(result.status, "pass");
        assert_eq!(result.steps.len(), 6);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        assert_eq!(fs::read(root.join("keep.txt")).unwrap(), b"keep");
        let missing = permission_check(&root.join("missing"), "missing", true);
        assert_eq!(missing.status, "warn");
        assert!(!root.join("missing").exists());
        fs::remove_file(root.join("keep.txt")).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn classifies_evidence_and_never_claims_sandbox_success() {
        let hints =
            analyze("sandbox error: permission denied\nAuthorization: Bearer fixture-secret");
        assert_eq!(hints.len(), 2);
        assert!(hints.iter().all(|h| h.status == "warn"));
        assert!(analyze("operation completed").is_empty());
    }
    #[test]
    fn manual_reader_limits_files_and_redacts() {
        let root = env::temp_dir().join(format!("cyapi-log-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let log = root.join("sample.log");
        fs::write(&log, "sandbox failure\napi_key=fixture-secret-value\n").unwrap();
        let view = read_log(&log).unwrap();
        assert!(!view.to_string().contains("fixture-secret-value"));
        assert_eq!(view["hints"].as_array().unwrap().len(), 1);
        assert!(read_log(&root.join("auth.json")).is_err());
        fs::write(
            &log,
            format!("{}\n最新记录 marker-at-tail\n", "old line\n".repeat(40000)),
        )
        .unwrap();
        let tail = read_log(&log).unwrap();
        assert_eq!(tail["truncated"], true);
        assert!(tail["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line.as_str().unwrap().contains("marker-at-tail")));
        fs::remove_file(log).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
