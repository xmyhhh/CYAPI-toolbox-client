//! Claude Code API configuration through the `env` block of `settings.json`.
//! Only the three gateway fields managed by the toolbox are touched; every other
//! key in the file is preserved, and the previous file is kept for restoration.
use crate::configuration::{no_links, read_bytes, stage};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, String>;

const BASE_URL: &str = "ANTHROPIC_BASE_URL";
const AUTH_TOKEN: &str = "ANTHROPIC_AUTH_TOKEN";
const MODEL: &str = "ANTHROPIC_MODEL";
const API_KEY: &str = "ANTHROPIC_API_KEY";

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Input {
    pub base_url: String,
    pub auth_token: String,
    pub model: String,
    pub revision: String,
}

type Snapshot = Option<Vec<u8>>;

#[derive(Serialize, Deserialize)]
struct Backup {
    schema: u32,
    directory: PathBuf,
    original: Option<String>,
    after: Option<String>,
}

impl Backup {
    fn original(&self) -> Result<Snapshot> {
        let bytes = self
            .original
            .as_ref()
            .map(|value| {
                STANDARD
                    .decode(value)
                    .map_err(|_| "Claude Code 备份无效".to_string())
            })
            .transpose()?;
        if bytes.as_ref().is_some_and(|bytes| bytes.len() > 1_000_000) {
            return Err("Claude Code 备份过大".into());
        }
        Ok(bytes)
    }

    fn matches(&self, root: &Path, current: &Snapshot) -> Result<bool> {
        if self.schema != 1 || self.directory != root {
            return Ok(false);
        }
        let original = self.original()?;
        Ok(current == &original
            || current
                .as_ref()
                .map(|bytes| crate::service::hash(bytes))
                .as_ref()
                == self.after.as_ref())
    }
}

fn config_path(root: &Path) -> PathBuf {
    root.join("settings.json")
}
fn backup_path(data: &Path) -> PathBuf {
    data.join("claude-config/previous.json")
}

fn snapshot(root: &Path) -> Result<Snapshot> {
    let path = config_path(root);
    no_links(&path)?;
    read_bytes(&path)
}
fn revision(snapshot: &Snapshot) -> String {
    // Distinguish an absent file from an existing empty file.
    crate::service::hash(&serde_json::to_vec(snapshot).unwrap_or_default())
}
fn read_backup(data: &Path) -> Result<Option<Backup>> {
    let path = backup_path(data);
    no_links(&path)?;
    match fs::metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(meta) if meta.is_file() && meta.len() <= 3_000_000 => {}
        _ => return Err("无法读取 Claude Code 备份".into()),
    }
    let backup: Backup =
        serde_json::from_slice(&fs::read(path).map_err(|_| "无法读取 Claude Code 备份")?)
            .map_err(|_| "Claude Code 备份无效")?;
    if backup.schema != 1 {
        return Err("不支持的 Claude Code 备份格式".into());
    }
    backup.original()?;
    Ok(Some(backup))
}

fn parse(bytes: &Snapshot) -> Result<Value> {
    let value: Value = match bytes {
        Some(bytes) => {
            let text = std::str::from_utf8(bytes).map_err(|_| "settings.json 不是 UTF-8 文件")?;
            serde_json::from_str(text.trim_start_matches('\u{feff}'))
                .map_err(|_| "settings.json 格式无效，请修复后重新读取")?
        }
        None => json!({}),
    };
    if !value.is_object() {
        return Err("settings.json 必须是 JSON 对象".into());
    }
    Ok(value)
}

fn env_of(doc: &Value) -> Option<&Map<String, Value>> {
    doc.get("env").and_then(Value::as_object)
}
fn text<'a>(env: Option<&'a Map<String, Value>>, key: &str) -> Option<&'a str> {
    env.and_then(|env| env.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn detect_cli() -> (bool, String) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if let Ok(output) = std::process::Command::new("where.exe")
            .arg("claude")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !path.is_empty() {
                    return (true, path);
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(output) = std::process::Command::new("which").arg("claude").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return (true, path);
                }
            }
        }
    }
    (false, String::new())
}

fn snapshot_view(root: &Path, data: &Path, snapshot: &Snapshot, secret: bool) -> Result<Value> {
    let doc = parse(snapshot)?;
    let env = env_of(&doc);
    let token = text(env, AUTH_TOKEN).or_else(|| text(env, API_KEY));
    let auth_mode = if token.is_some() {
        "API Token"
    } else if root.join(".credentials.json").is_file() {
        "Claude 登录"
    } else {
        "未配置"
    };
    let can_restore = read_backup(data)
        .ok()
        .flatten()
        .and_then(|backup| backup.matches(root, snapshot).ok())
        .unwrap_or(false);
    let (cli_installed, cli_path) = detect_cli();
    let mut view = json!({
        "configPath": config_path(root),
        "revision": revision(snapshot),
        "baseUrl": text(env, BASE_URL).unwrap_or(""),
        "model": text(env, MODEL).unwrap_or(""),
        "authTokenConfigured": token.is_some(),
        "apiKeyConfigured": text(env, API_KEY).is_some(),
        "authMode": auth_mode,
        "settingsExists": snapshot.is_some(),
        "cliInstalled": cli_installed,
        "cliPath": cli_path,
        "canRestore": can_restore
    });
    if secret {
        view["authToken"] = json!(text(env, AUTH_TOKEN).unwrap_or(""));
    }
    Ok(view)
}

// Read-only status shape; the local editor uses editor_view below to fill the password field.
#[allow(dead_code)]
pub fn view(root: &Path, data: &Path) -> Result<Value> {
    snapshot_view(root, data, &snapshot(root)?, false)
}
// Only the local editor requests the credential for the password field.
pub fn editor_view(root: &Path, data: &Path) -> Result<Value> {
    snapshot_view(root, data, &snapshot(root)?, true)
}

fn normalize_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    trimmed.strip_suffix("/v1").unwrap_or(trimmed).to_string()
}

fn validate(input: &Input) -> Result<()> {
    let url = reqwest::Url::parse(&normalize_url(&input.base_url)).map_err(|_| "API 地址无效")?;
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || input.base_url.len() > 2048
        || input.base_url.chars().any(char::is_control)
    {
        return Err("请填写完整的 HTTP/HTTPS API 地址，不包含账号、参数或锚点".into());
    }
    if input.model.len() > 200 || input.model.chars().any(char::is_control) {
        return Err("请填写有效的模型 ID".into());
    }
    if input.auth_token.len() > 8192
        || input
            .auth_token
            .trim()
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("AUTH_TOKEN 格式无效".into());
    }
    Ok(())
}

fn commit(root: &Path, current: &Snapshot, next: &Snapshot) -> Result<()> {
    if current == next {
        return Ok(());
    }
    let path = config_path(root);
    no_links(&path)?;
    if &read_bytes(&path)? != current {
        return Err("Claude Code 配置已被其他程序修改，请重新读取".into());
    }
    match next {
        Some(bytes) => {
            let temp = stage(&path, bytes)?;
            fs::rename(&temp, &path).map_err(|_| {
                let _ = fs::remove_file(&temp);
                "无法替换 Claude Code 配置文件，请检查是否被占用".into()
            })
        }
        None => match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("无法恢复 Claude Code 配置文件".into()),
        },
    }
}

pub fn save(root: &Path, data: &Path, input: &Input) -> Result<Value> {
    validate(input)?;
    let current = snapshot(root)?;
    if revision(&current) != input.revision {
        return Err("Claude Code 配置已变化，请重新读取后保存".into());
    }
    let mut doc = parse(&current)?;
    let token = if input.auth_token.trim().is_empty() {
        let env = env_of(&doc);
        text(env, AUTH_TOKEN)
            .or_else(|| text(env, API_KEY))
            .ok_or("请填写 AUTH_TOKEN")?
            .to_string()
    } else {
        input.auth_token.trim().to_string()
    };
    let object = doc
        .as_object_mut()
        .ok_or("settings.json 必须是 JSON 对象")?;
    if !object.contains_key("env") {
        object.insert("env".into(), json!({}));
    }
    let env = object
        .get_mut("env")
        .and_then(Value::as_object_mut)
        .ok_or("settings.json 的 env 必须是 JSON 对象")?;
    env.insert(BASE_URL.into(), json!(normalize_url(&input.base_url)));
    env.insert(AUTH_TOKEN.into(), json!(token));
    if input.model.trim().is_empty() {
        env.remove(MODEL);
    } else {
        env.insert(MODEL.into(), json!(input.model.trim()));
    }
    let mut bytes = serde_json::to_vec_pretty(&doc).map_err(|_| "无法生成 settings.json")?;
    bytes.push(b'\n');
    if bytes.len() > 1_000_000 {
        return Err("生成的 Claude Code 配置文件过大".into());
    }
    let next = Some(bytes);
    if next == current {
        return editor_view(root, data);
    }
    let backup = Backup {
        schema: 1,
        directory: root.into(),
        original: current.as_ref().map(|bytes| STANDARD.encode(bytes)),
        after: next.as_ref().map(|bytes| crate::service::hash(bytes)),
    };
    let backup_bytes = serde_json::to_vec(&backup).map_err(|_| "无法生成 Claude Code 备份")?;
    let backup_file = backup_path(data);
    if let Some(parent) = backup_file.parent() {
        fs::create_dir_all(parent).map_err(|_| "无法创建 Claude Code 备份目录")?;
    }
    let temp = stage(&backup_file, &backup_bytes)?;
    fs::rename(&temp, &backup_file).map_err(|_| {
        let _ = fs::remove_file(&temp);
        "无法写入 Claude Code 备份".to_string()
    })?;
    commit(root, &current, &next)?;
    editor_view(root, data)
}

pub fn restore(root: &Path, data: &Path, revision_value: &str) -> Result<Value> {
    let current = snapshot(root)?;
    if revision(&current) != revision_value {
        return Err("Claude Code 配置已变化，请重新读取后恢复".into());
    }
    let backup = read_backup(data)?.ok_or("没有可恢复的 Claude Code 配置")?;
    if !backup.matches(root, &current)? {
        return Err("Claude Code 配置已在外部修改，无法直接恢复旧备份".into());
    }
    let original = backup.original()?;
    // Parse before writing so a damaged backup is never published.
    parse(&original)?;
    commit(root, &current, &original)?;
    let path = backup_path(data);
    no_links(&path)?;
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("无法删除 Claude Code 备份".into()),
    }
    editor_view(root, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn fixture() -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "cyapi-claude-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let root = base.join("home/.claude");
        fs::create_dir_all(&root).unwrap();
        (base.clone(), root, base.join("data"))
    }
    fn input(view: &Value) -> Input {
        Input {
            base_url: "https://example.invalid/v1/".into(),
            auth_token: "dummy-auth-token".into(),
            model: "claude-example".into(),
            revision: view["revision"].as_str().unwrap().into(),
        }
    }

    #[test]
    fn saves_gateway_fields_and_preserves_unrelated_settings() {
        let (base, root, data) = fixture();
        let settings =
            br#"{"env":{"KEEP_ME":"yes"},"permissions":{"allow":["Bash"]},"theme":"dark"}"#;
        fs::write(root.join("settings.json"), settings).unwrap();

        let before = view(&root, &data).unwrap();
        assert_eq!(before["authTokenConfigured"], false);
        assert!(before.get("authToken").is_none());

        let saved = save(&root, &data, &input(&before)).unwrap();
        assert_eq!(saved["baseUrl"], "https://example.invalid");
        assert_eq!(saved["model"], "claude-example");
        assert_eq!(saved["authTokenConfigured"], true);
        assert_eq!(saved["canRestore"], true);
        assert!(
            saved["authToken"].is_string(),
            "editor view returns the token"
        );
        assert!(!view(&root, &data)
            .unwrap()
            .to_string()
            .contains("dummy-auth-token"));

        let doc = parse(&snapshot(&root).unwrap()).unwrap();
        assert_eq!(doc["env"]["KEEP_ME"], "yes");
        assert_eq!(doc["env"][BASE_URL], "https://example.invalid");
        assert_eq!(doc["env"][AUTH_TOKEN], "dummy-auth-token");
        assert_eq!(doc["permissions"]["allow"][0], "Bash");
        assert_eq!(doc["theme"], "dark");

        // Blank token keeps the saved credential.
        let mut repeated = input(&saved);
        repeated.auth_token.clear();
        let again = save(&root, &data, &repeated).unwrap();
        assert_eq!(again["authTokenConfigured"], true);

        let restored = restore(&root, &data, again["revision"].as_str().unwrap()).unwrap();
        assert_eq!(fs::read(root.join("settings.json")).unwrap(), settings);
        assert_eq!(restored["canRestore"], false);
        assert_eq!(restored["authTokenConfigured"], false);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn creates_and_removes_settings_and_blocks_stale_or_invalid_writes() {
        let (base, root, data) = fixture();
        let before = view(&root, &data).unwrap();
        assert_eq!(before["settingsExists"], false);

        for (url, token) in [
            ("https://user:dummy@example.invalid", "dummy-token"),
            ("file:///etc/passwd", "dummy-token"),
            ("https://example.invalid", "bad\ntoken"),
        ] {
            let mut invalid = input(&before);
            invalid.base_url = url.into();
            invalid.auth_token = token.into();
            let error = save(&root, &data, &invalid).unwrap_err();
            assert!(!error.contains("dummy"));
            assert!(!root.join("settings.json").exists());
            assert!(!backup_path(&data).exists());
        }
        let mut missing = input(&before);
        missing.auth_token.clear();
        assert!(save(&root, &data, &missing).is_err());

        let saved = save(&root, &data, &input(&before)).unwrap();
        assert_eq!(saved["settingsExists"], true);
        assert!(save(&root, &data, &input(&before))
            .unwrap_err()
            .contains("变化"));

        fs::write(
            root.join("settings.json"),
            br#"{"env":{"ANTHROPIC_BASE_URL":"https://external.invalid"}}"#,
        )
        .unwrap();
        assert!(!view(&root, &data).unwrap()["canRestore"].as_bool().unwrap());
        assert!(restore(&root, &data, saved["revision"].as_str().unwrap()).is_err());

        let current = view(&root, &data).unwrap();
        fs::write(root.join("settings.json"), b"[]").unwrap();
        assert!(view(&root, &data).unwrap_err().contains("JSON 对象"));
        fs::write(root.join("settings.json"), b"{oops").unwrap();
        assert!(!view(&root, &data).unwrap_err().contains("oops"));
        let _ = current;
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn corrupt_backup_does_not_block_viewing_or_saving() {
        let (base, root, data) = fixture();
        save(&root, &data, &input(&view(&root, &data).unwrap())).unwrap();
        fs::write(backup_path(&data), b"{broken").unwrap();
        let current = view(&root, &data).unwrap();
        assert_eq!(current["canRestore"], false);
        let mut next = input(&current);
        next.model = "claude-next".into();
        let saved = save(&root, &data, &next).unwrap();
        assert_eq!(saved["model"], "claude-next");
        assert_eq!(saved["canRestore"], true);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_non_object_env_without_touching_the_file() {
        let (base, root, data) = fixture();
        let settings = br#"{"env":"keep-me"}"#;
        fs::write(root.join("settings.json"), settings).unwrap();
        let before = view(&root, &data).unwrap();
        let error = save(&root, &data, &input(&before)).unwrap_err();
        assert!(error.contains("env"));
        assert_eq!(fs::read(root.join("settings.json")).unwrap(), settings);
        fs::remove_dir_all(base).unwrap();
    }
}
