//! Bounded, local operation logs. Never serialize requests, responses or configuration values.
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub timestamp: u64,
    pub level: String,
    pub source: String,
    pub operation_id: Option<String>,
    pub message: String,
}

struct Store {
    entries: Mutex<VecDeque<Entry>>,
    directory: Option<PathBuf>,
    error: Arc<Mutex<Option<String>>>,
    sender: mpsc::SyncSender<Entry>,
    dropped: AtomicU64,
    sequence: AtomicU64,
    session: String,
}
static STORE: OnceLock<Store> = OnceLock::new();
static SECRETS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
thread_local! { static OPERATION: RefCell<Option<String>> = const { RefCell::new(None) }; }
#[cfg(test)]
thread_local! { static CAPTURE: RefCell<Option<Vec<(String, String, String)>>> = const { RefCell::new(None) }; }

/// Capture this thread only, without initializing persistent logs or changing
/// the logging destination of concurrently running tests.
#[cfg(test)]
pub(crate) fn capture_events(f: impl FnOnce()) -> Vec<(String, String, String)> {
    CAPTURE.with(|capture| *capture.borrow_mut() = Some(Vec::new()));
    f();
    CAPTURE.with(|capture| capture.borrow_mut().take().unwrap())
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn register_secret(secret: &str) {
    if secret.len() < 6 {
        return;
    }
    if let Ok(mut secrets) = SECRETS.get_or_init(Default::default).lock() {
        if secrets.len() < 256 && !secrets.iter().any(|v| v == secret) {
            secrets.push(secret.into());
            // Mask longer credentials first so a shared prefix cannot expose a suffix.
            secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
        }
    }
}

pub fn redact(text: &str) -> String {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| vec![
        (Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+").unwrap(), "Bearer [REDACTED]"),
        (Regex::new(r"(?i)(\bAuthorization\s*[:=]\s*Basic\s+)[A-Za-z0-9+/=]+").unwrap(), "$1[REDACTED]"),
        (Regex::new(r"(?im)(\b(?:Cookie|Set-Cookie)\s*[:=]\s*)[^\r\n]+").unwrap(), "$1[REDACTED]"),
        (Regex::new(r#"(?i)([\"']?(?:[a-z_]*api[_-]?key|access[_-]?token|refresh[_-]?token|id[_-]?token|token|password|secret|authorization|cookie)[\"']?\s*[:=]\s*)(?:\"(?:\\.|[^\"\\])*\"|'(?:\\.|[^'\\])*'|[^\s,;}&]+)"#).unwrap(), "$1[REDACTED]"),
        (Regex::new(r"\bsk-[A-Za-z0-9_-]{8,}").unwrap(), "[REDACTED]"),
        (Regex::new(r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+").unwrap(), "[REDACTED]"),
        (Regex::new(r"(?i)(https?://)[^\s/@]+:[^\s/@]+@").unwrap(), "$1[REDACTED]@"),
        (Regex::new(r#"(?i)(https?://[^\s?\"<>]+)\?[^\s\"<>]*"#).unwrap(), "$1?[REDACTED]"),
    ]);
    let mut value: String = text.chars().take(262_144).collect();
    if let Ok(secrets) = SECRETS.get_or_init(Default::default).lock() {
        for secret in secrets.iter() {
            value = value.replace(secret, "[REDACTED]");
        }
    }
    for (pattern, replacement) in patterns {
        value = pattern.replace_all(&value, *replacement).into_owned();
    }
    value
}

pub fn export_redact(text: &str) -> String {
    let mut value = redact(text);
    for key in [crate::platform::home_variable(), "CYAPI_TOOLBOX_HOME_DIR"] {
        if let Some(home) = std::env::var_os(key) {
            let home = home.to_string_lossy();
            if !home.is_empty() {
                value = value
                    .replace(home.as_ref(), "<USER>")
                    .replace(&home.replace('\\', "\\\\"), "<USER>");
            }
        }
    }
    value
}

pub fn data_path() -> PathBuf {
    std::env::var_os("CYAPI_TOOLBOX_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = crate::diagnostics::home_path();
            crate::platform::data_root(&home).join("CYAPI-toolbox/data")
        })
}

fn log_files(directory: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|v| v.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|s| s.to_string_lossy().starts_with("toolbox-"))
                && p.extension().is_some_and(|s| s == "jsonl")
                && fs::symlink_metadata(p).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
        })
        .collect();
    files.sort();
    files
}

fn prune(directory: &Path, active: &Path) {
    let files = log_files(directory);
    let mut count = files.len();
    let mut total: u64 = files
        .iter()
        .filter_map(|p| fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    for path in files {
        if path == active {
            continue;
        }
        if let Ok(meta) = fs::metadata(&path) {
            let expired = meta
                .modified()
                .ok()
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|age| age.as_secs() > 7 * 86400);
            if count > 20 || total > 40 * 1024 * 1024 || expired {
                if fs::remove_file(&path).is_ok() {
                    count -= 1;
                    total = total.saturating_sub(meta.len());
                }
            }
        }
    }
}

pub fn initialize() {
    STORE.get_or_init(|| {
        let session = format!("{}-{}", now(), uuid::Uuid::new_v4());
        let primary = data_path().join("logs");
        let mut error = None;
        let directory = [
            primary.clone(),
            std::env::temp_dir().join("CYAPI-toolbox/logs"),
        ]
        .into_iter()
        .find(|p| {
            match fs::create_dir_all(p).and_then(|_| {
                OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(p.join(format!("toolbox-{session}-0000.jsonl")))
            }) {
                Ok(_) => true,
                Err(e) => {
                    error = Some(format!(
                        "日志位置不可写：{}（系统错误 {:?}）",
                        p.display(),
                        e.raw_os_error()
                    ));
                    false
                }
            }
        });
        let mut history = VecDeque::new();
        if let Some(dir) = &directory {
            for path in log_files(dir)
                .into_iter()
                .rev()
                .skip(1)
                .take(3)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
            {
                if let Ok(mut file) = fs::File::open(path) {
                    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
                    let start = size.saturating_sub(512_000);
                    let _ = file.seek(SeekFrom::Start(start));
                    let mut bytes = Vec::new();
                    let _ = file.take(512_000).read_to_end(&mut bytes);
                    for line in String::from_utf8_lossy(&bytes)
                        .lines()
                        .skip(usize::from(start > 0))
                    {
                        if let Ok(mut entry) = serde_json::from_str::<Entry>(line) {
                            entry.message = redact(&entry.message);
                            history.push_back(entry);
                        }
                    }
                }
            }
        }
        while history.len() > 4000 {
            history.pop_front();
        }
        let error = Arc::new(Mutex::new(error));
        let writer_error = error.clone();
        let writer_directory = directory.clone();
        let writer_session = session.clone();
        let (sender, receiver) = mpsc::sync_channel::<Entry>(1024);
        std::thread::spawn(move || {
            let mut part = 0u64;
            let mut size = 0;
            for entry in receiver {
                if let Some(dir) = &writer_directory {
                    if size > 2 * 1024 * 1024 {
                        part += 1;
                        size = 0;
                    }
                    let path = dir.join(format!("toolbox-{writer_session}-{part:04}.jsonl"));
                    let result = (|| -> std::io::Result<()> {
                        let mut bytes = serde_json::to_vec(&entry)?;
                        bytes.push(b'\n');
                        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
                        file.write_all(&bytes)?;
                        if entry.level == "error" {
                            file.sync_data()?;
                        }
                        size += bytes.len();
                        Ok(())
                    })();
                    if let Err(e) = result {
                        if let Ok(mut err) = writer_error.lock() {
                            *err = Some(format!(
                                "日志落盘失败（系统错误 {:?}），当前会话仍可在窗口查看",
                                e.raw_os_error()
                            ));
                        }
                    }
                    if size < 10_000 || size % 100_000 < 1000 {
                        prune(dir, &path);
                    }
                }
            }
        });
        Store {
            entries: Mutex::new(history),
            directory,
            error,
            sender,
            dropped: AtomicU64::new(0),
            sequence: AtomicU64::new(0),
            session,
        }
    });
}

pub fn event(level: &str, source: &str, message: impl AsRef<str>) {
    #[cfg(test)]
    CAPTURE.with(|capture| {
        if let Some(events) = capture.borrow_mut().as_mut() {
            events.push((level.into(), source.into(), message.as_ref().into()));
        }
    });
    let Some(store) = STORE.get() else {
        return;
    };
    let entry = Entry {
        id: format!(
            "{}:{}",
            store.session,
            store.sequence.fetch_add(1, Ordering::Relaxed)
        ),
        timestamp: now(),
        level: level.into(),
        source: source.into(),
        operation_id: OPERATION.with(|v| v.borrow().clone()),
        message: redact(message.as_ref()).chars().take(4000).collect(),
    };
    if let Ok(mut entries) = store.entries.lock() {
        entries.push_back(entry.clone());
        if entries.len() > 5000 {
            entries.pop_front();
        }
    }
    if store.sender.try_send(entry).is_err() {
        store.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn with_operation<T>(id: String, f: impl FnOnce() -> T) -> T {
    struct Reset(Option<String>);
    impl Drop for Reset {
        fn drop(&mut self) {
            OPERATION.with(|v| *v.borrow_mut() = self.0.take());
        }
    }
    let previous = OPERATION.with(|v| v.replace(Some(id)));
    let _reset = Reset(previous);
    f()
}

#[tauri::command]
pub fn logs_snapshot(after_id: Option<String>) -> serde_json::Value {
    let Some(store) = STORE.get() else {
        return serde_json::json!({"entries":[], "gap":false, "dropped":0});
    };
    let entries = store.entries.lock().unwrap_or_else(|e| e.into_inner());
    let position = after_id
        .as_ref()
        .and_then(|id| entries.iter().position(|v| &v.id == id));
    let gap = after_id.is_some() && position.is_none();
    let start = position
        .map(|p| p + 1)
        .unwrap_or_else(|| entries.len().saturating_sub(1000));
    serde_json::json!({"entries":entries.iter().skip(start).collect::<Vec<_>>(), "gap":gap,
        "dropped":store.dropped.load(Ordering::Relaxed), "directory":store.directory,
        "writeError":store.error.lock().ok().and_then(|e| e.clone())})
}

pub fn current_directory() -> Option<PathBuf> {
    STORE.get().and_then(|s| s.directory.clone())
}

pub fn export_entries(operation: Option<&str>) -> Vec<Entry> {
    STORE
        .get()
        .and_then(|store| store.entries.lock().ok())
        .map(|entries| {
            entries
                .iter()
                .filter(|e| operation.is_none_or(|id| e.operation_id.as_deref() == Some(id)))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
pub fn log_frontend_error(message: String) {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let time = now();
    if time.saturating_sub(LAST.swap(time, Ordering::Relaxed)) > 500 {
        event(
            "error",
            "frontend",
            message.chars().take(2000).collect::<String>(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redacts_credentials_without_losing_error_context() {
        let input = r#"OPENAI_API_KEY="arbitrary-dummy-value" Authorization: Bearer abc123 refresh_token='refresh-demo' https://user:pass@example.test/v1?key=secret code=5 sk-dummy123456789"#;
        let output = redact(input);
        for secret in [
            "arbitrary-dummy-value",
            "abc123",
            "refresh-demo",
            "user:pass",
            "key=secret",
            "sk-dummy123456789",
        ] {
            assert!(!output.contains(secret), "{output}");
        }
        assert!(output.contains("code=5"));
        let headers = redact("Cookie: session=fixture-cookie; other=fixture-other\nAuthorization: Basic Zml4dHVyZQ==");
        for value in ["fixture-cookie", "fixture-other", "Zml4dHVyZQ=="] {
            assert!(!headers.contains(value));
        }
        register_secret("custom-credential-without-prefix");
        assert!(!redact("error custom-credential-without-prefix").contains("custom-credential"));
    }
    #[test]
    fn operation_scope_is_restored_after_panic() {
        let _ = std::panic::catch_unwind(|| with_operation("test".into(), || panic!("fixture")));
        assert!(OPERATION.with(|v| v.borrow().is_none()));
    }

    #[test]
    fn redacts_escaped_quotes_and_overlapping_registered_credentials() {
        let output = redact(r#"{"apiKey":"prefix\"private-suffix","message":"keep context"}"#);
        assert!(!output.contains("prefix"), "{output}");
        assert!(!output.contains("private-suffix"), "{output}");
        assert!(output.contains("keep context"));
        register_secret("overlapping-fixture");
        register_secret("overlapping-fixture-private-suffix");
        assert_eq!(redact("overlapping-fixture-private-suffix"), "[REDACTED]");
    }
}
