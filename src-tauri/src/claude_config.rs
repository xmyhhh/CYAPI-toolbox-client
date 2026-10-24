//! Claude Code API configuration through the `env` block of `settings.json`.
//! The gateway, model picker, default model, effort, and context window are managed; every other
//! key in the file is preserved, and the previous file is kept for restoration.
use crate::configuration::{no_links, read_bytes, stage};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, String>;

const BASE_URL: &str = "ANTHROPIC_BASE_URL";
const AUTH_TOKEN: &str = "ANTHROPIC_AUTH_TOKEN";
const MODEL: &str = "ANTHROPIC_MODEL";
const API_KEY: &str = "ANTHROPIC_API_KEY";
const MAX_CONTEXT_TOKENS: &str = "CLAUDE_CODE_MAX_CONTEXT_TOKENS";
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextWindowChange {
    // An empty object must never accidentally clear an existing override.
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub value: Option<u64>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Input {
    pub base_url: String,
    pub auth_token: String,
    pub model: String,
    #[serde(default)]
    pub models: Option<Vec<String>>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub model_effort_changes: BTreeMap<String, String>,
    #[serde(default)]
    pub clear_effort_overrides: bool,
    #[serde(default)]
    pub context_window_change: Option<ContextWindowChange>,
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

fn default_model(doc: &Value) -> &str {
    text(env_of(doc), MODEL)
        .or_else(|| doc.get("model").and_then(Value::as_str))
        .unwrap_or("")
}

fn selected_models(doc: &Value) -> Vec<String> {
    doc.pointer("/modelPicker/options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|entry| {
                    entry
                        .get("model")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn model_settings(doc: &Value) -> Option<&Map<String, Value>> {
    doc.get("modelSettings").and_then(Value::as_object)
}

fn without_context(id: &str) -> &str {
    id.strip_suffix("[1m]").unwrap_or(id)
}

fn model_identity(id: &str) -> &str {
    let id = without_context(id);
    match id.rsplit_once('-') {
        Some((base, date)) if date.len() == 8 && date.bytes().all(|c| c.is_ascii_digit()) => base,
        _ => id,
    }
}

fn is_model_alias(id: &str) -> bool {
    let id = without_context(id);
    [
        "opus",
        "sonnet",
        "haiku",
        "fable",
        "best",
        "default",
        "opusplan",
        "sonnetplan",
    ]
    .iter()
    .any(|alias| {
        id == *alias
            || id.strip_prefix(&format!("{alias}-")).is_some_and(|suffix| {
                suffix == "latest"
                    || (suffix.bytes().any(|c| c.is_ascii_digit())
                        && suffix
                            .bytes()
                            .all(|c| c.is_ascii_digit() || c == b'-' || c == b'.'))
            })
    })
}

// Never guess a release for an alias. Only an exact settings key or an explicit
// ANTHROPIC_DEFAULT_*_MODEL mapping can establish its concrete model identity.
fn effort_key(doc: &Value, id: &str) -> Result<String> {
    let settings = model_settings(doc);
    if settings.is_some_and(|settings| settings.contains_key(id)) {
        return Ok(id.to_string());
    }
    let concrete = if is_model_alias(id) {
        let alias = without_context(id).to_ascii_uppercase().replace('-', "_");
        let env_key = format!("ANTHROPIC_DEFAULT_{alias}_MODEL");
        // In particular, `best` can select different families at runtime and
        // requires its own explicit BEST mapping, not an Opus/Fable fallback.
        text(env_of(doc), &env_key)
            .filter(|value| {
                !is_model_alias(value) && value.len() <= 200 && !value.chars().any(char::is_control)
            })
            .ok_or("无法解析模型 alias 的逐模型配置，请填写完整模型 ID")?
    } else {
        id
    };
    if settings.is_some_and(|settings| settings.contains_key(concrete)) {
        return Ok(concrete.to_string());
    }
    // Gateway IDs are opaque: do not canonicalize non-Claude model names.
    if !concrete.starts_with("claude-") {
        return Ok(concrete.to_string());
    }
    let identity = model_identity(concrete);
    for candidate in [without_context(concrete), identity] {
        if settings.is_some_and(|settings| settings.contains_key(candidate)) {
            return Ok(candidate.to_string());
        }
    }
    let matches: Vec<&String> = settings
        .into_iter()
        .flat_map(|settings| settings.keys())
        .filter(|key| model_identity(key) == identity)
        .collect();
    match matches.as_slice() {
        [key] => Ok((*key).clone()),
        [] => Ok(identity.to_string()),
        _ => Err("模型 ID 对应多个已有配置，请填写精确的完整模型 ID".into()),
    }
}

fn model_efforts(doc: &Value) -> BTreeMap<String, String> {
    model_settings(doc)
        .into_iter()
        .flat_map(|settings| settings.iter())
        .map(|(key, entry)| {
            (
                key.clone(),
                entry
                    .get("effortLevel")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect()
}

fn model_effort_keys(doc: &Value) -> BTreeMap<String, String> {
    let mut keys = BTreeMap::new();
    let ids = model_settings(doc)
        .into_iter()
        .flat_map(|settings| settings.keys().cloned())
        .chain(selected_models(doc))
        .chain([default_model(doc).to_string()]);
    for id in ids.filter(|id| !id.is_empty()) {
        if let Ok(key) = effort_key(doc, &id) {
            if model_settings(doc).is_some_and(|settings| settings.contains_key(&key)) {
                keys.insert(id, key);
            }
        }
    }
    keys
}

fn reasoning_effort(doc: &Value) -> &str {
    text(env_of(doc), "CLAUDE_CODE_EFFORT_LEVEL")
        .or_else(|| {
            effort_key(doc, default_model(doc)).ok().and_then(|key| {
                model_settings(doc)
                    .and_then(|settings| settings.get(&key))
                    .and_then(|entry| entry.get("effortLevel"))
                    .and_then(Value::as_str)
            })
        })
        .or_else(|| doc.get("effortLevel").and_then(Value::as_str))
        .unwrap_or("")
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
        "model": default_model(&doc),
        "models": selected_models(&doc),
        "contextWindow": env.and_then(|env| env.get(MAX_CONTEXT_TOKENS)).cloned().unwrap_or(Value::Null),
        "reasoningEffort": reasoning_effort(&doc),
        "modelEfforts": model_efforts(&doc),
        "modelEffortKeys": model_effort_keys(&doc),
        "effortOverrides": {
            "environment": env.and_then(|env| env.get("CLAUDE_CODE_EFFORT_LEVEL")).and_then(Value::as_str).unwrap_or(""),
            "global": doc.get("effortLevel").and_then(Value::as_str).unwrap_or("")
        },
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
    if let Some(value) = input
        .context_window_change
        .as_ref()
        .and_then(|change| change.value)
    {
        if !(1..=MAX_SAFE_INTEGER).contains(&value) {
            return Err("上下文窗口必须是 1 到 9007199254740991 之间的整数".into());
        }
    }
    crate::model_selection::models(&input.model, input.models.as_deref().unwrap_or_default())?;
    for effort in input
        .model_effort_changes
        .values()
        .chain(input.reasoning_effort.iter())
    {
        crate::model_selection::effort(effort, &["", "low", "medium", "high", "xhigh"])?;
    }
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

// This is a global Claude Code override, not modelSettings metadata. Only an
// explicit patch touches it; clearing an absent key must not create an env block.
fn apply_context_window(doc: &mut Value, change: Option<&ContextWindowChange>) -> Result<()> {
    let Some(change) = change else {
        return Ok(());
    };
    let object = doc
        .as_object_mut()
        .ok_or("settings.json 必须是 JSON 对象")?;
    if change.value.is_none() && !object.contains_key("env") {
        return Ok(());
    }
    let env = object
        .entry("env")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("settings.json 的 env 必须是 JSON 对象")?;
    match change.value {
        Some(value) => {
            env.insert(MAX_CONTEXT_TOKENS.into(), json!(value.to_string()));
        }
        None => {
            env.remove(MAX_CONTEXT_TOKENS);
        }
    }
    Ok(())
}

fn effort_changes(
    doc: &Value,
    input: &Input,
    selected: &[String],
) -> Result<BTreeMap<String, String>> {
    let mut changes = input.model_effort_changes.clone();
    if let Some(effort) = &input.reasoning_effort {
        // Compatibility with older clients is limited to the current default.
        // The explicit per-model patch wins if both fields name that model.
        changes
            .entry(input.model.trim().to_string())
            .or_insert_with(|| effort.clone());
    }
    let mut resolved = BTreeMap::new();
    for (id, effort) in changes {
        if !selected.contains(&id) {
            return Err("思考等级补丁只能修改已选择的模型".into());
        }
        let key = effort_key(doc, &id)?;
        if resolved
            .get(&key)
            .is_some_and(|previous| previous != &effort)
        {
            return Err("多个模型 alias 对应同一配置，但思考等级补丁冲突".into());
        }
        resolved.insert(key, effort);
    }
    Ok(resolved)
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
    let selected = crate::model_selection::models(
        &input.model,
        input.models.as_deref().unwrap_or(&selected_models(&doc)),
    )?;
    let changes = effort_changes(&doc, input, &selected)?;
    apply_context_window(&mut doc, input.context_window_change.as_ref())?;
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
    if input.clear_effort_overrides {
        env.remove("CLAUDE_CODE_EFFORT_LEVEL");
    }
    if input.model.trim().is_empty() {
        object.remove("model");
    } else {
        object.insert("model".into(), json!(input.model.trim()));
    }
    if input.models.is_some() {
        let picker = object
            .entry("modelPicker")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("modelPicker 必须是 JSON 对象")?;
        let previous = picker
            .get("options")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let options: Vec<Value> = selected
            .iter()
            .map(|id| {
                previous
                    .iter()
                    .find(|entry| entry.get("model").and_then(Value::as_str) == Some(id))
                    .cloned()
                    .unwrap_or_else(|| json!({"model":id}))
            })
            .collect();
        picker.insert("options".into(), json!(options));
    }
    if input.clear_effort_overrides {
        object.remove("effortLevel");
    }
    // Missing patches leave every existing value (including unknown future effort
    // levels and metadata) untouched. Clearing only removes this model's field.
    if !changes.is_empty()
        && (object.contains_key("modelSettings")
            || changes.values().any(|effort| !effort.is_empty()))
    {
        let settings = object
            .entry("modelSettings")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("modelSettings 必须是 JSON 对象")?;
        for (key, effort) in changes {
            if effort.is_empty() {
                if let Some(value) = settings.get_mut(&key) {
                    value
                        .as_object_mut()
                        .ok_or("模型思考等级配置必须是 JSON 对象")?
                        .remove("effortLevel");
                }
            } else {
                let entry = settings
                    .entry(key)
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .ok_or("模型思考等级配置必须是 JSON 对象")?;
                entry.insert("effortLevel".into(), json!(effort));
            }
        }
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
            models: None,
            reasoning_effort: None,
            model_effort_changes: BTreeMap::new(),
            clear_effort_overrides: false,
            context_window_change: None,
            model: "claude-example".into(),
            revision: view["revision"].as_str().unwrap().into(),
        }
    }

    #[test]
    fn context_window_input_contract_and_invalid_json_types() {
        let mut request = json!({
            "baseUrl":"https://example.invalid", "authToken":"dummy",
            "model":"model-a", "revision":"test"
        });
        let missing: Input = serde_json::from_value(request.clone()).unwrap();
        assert!(missing.context_window_change.is_none());
        request["contextWindowChange"] = json!({});
        assert!(serde_json::from_value::<Input>(request.clone()).is_err());
        request["contextWindowChange"] = json!({"value":null});
        let clear: Input = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(clear.context_window_change.unwrap().value, None);
        for value in [1, 200_000, MAX_SAFE_INTEGER] {
            request["contextWindowChange"] = json!({"value":value});
            let set: Input = serde_json::from_value(request.clone()).unwrap();
            validate(&set).unwrap();
            assert_eq!(set.context_window_change.unwrap().value, Some(value));
        }
        for value in [
            json!(-1),
            json!(1.5),
            json!("200000"),
            json!(true),
            json!([]),
        ] {
            request["contextWindowChange"] = json!({"value":value});
            assert!(serde_json::from_value::<Input>(request.clone()).is_err());
        }
        request["contextWindowChange"] = json!({"value":200000,"maxOutput":1000});
        assert!(serde_json::from_value::<Input>(request).is_err());
    }

    #[test]
    fn omitted_context_patch_preserves_raw_unknown_values_on_address_and_default_save() {
        let (base, root, data) = fixture();
        for raw in [
            json!("  future-window  "),
            json!("0"),
            json!("9007199254740992"),
            json!(200000),
            json!(0),
            json!(-1),
            json!(1.5),
            json!(true),
            Value::Null,
            json!({"future":[1,2]}),
            json!(["unknown"]),
        ] {
            let settings = json!({
                "env": {
                    MAX_CONTEXT_TOKENS:raw,
                    BASE_URL:"https://old.invalid", AUTH_TOKEN:"dummy-auth-token", MODEL:"model-a",
                    "CLAUDE_CODE_EFFORT_LEVEL":"future-effort"
                },
                "model":"model-a",
                "effortLevel":"future-global",
                "modelSettings":{"model-a":{"effortLevel":"low","autoCompactWindow":12345}}
            });
            fs::write(config_path(&root), serde_json::to_vec(&settings).unwrap()).unwrap();
            let before = view(&root, &data).unwrap();
            assert_eq!(before["contextWindow"], raw);
            assert_eq!(editor_view(&root, &data).unwrap()["contextWindow"], raw);
            let mut change = input(&before);
            change.model = "model-a".into();
            let saved = save(&root, &data, &change).unwrap();
            assert_eq!(saved["contextWindow"], raw);
            assert_eq!(
                parse(&snapshot(&root).unwrap()).unwrap()["env"][MAX_CONTEXT_TOKENS],
                raw
            );
            change.model = "model-b".into();
            change.revision = saved["revision"].as_str().unwrap().into();
            let saved = save(&root, &data, &change).unwrap();
            let doc = parse(&snapshot(&root).unwrap()).unwrap();
            assert_eq!(saved["contextWindow"], raw);
            assert_eq!(doc["env"][MAX_CONTEXT_TOKENS], raw);
            assert_eq!(doc["modelSettings"], settings["modelSettings"]);
            assert_eq!(doc["effortLevel"], settings["effortLevel"]);
            assert_eq!(doc["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "future-effort");
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn context_window_set_clear_preserve_all_other_fields_and_restore_exact_backup() {
        let (base, root, data) = fixture();
        let settings = json!({
            "env": {
                BASE_URL:"https://example.invalid", AUTH_TOKEN:"dummy-auth-token", MODEL:"claude-example",
                MAX_CONTEXT_TOKENS:{"unknown":"old"},
                "DISABLE_COMPACT":"1", "CLAUDE_CODE_DISABLE_1M_CONTEXT":"1",
                "CLAUDE_CODE_AUTO_COMPACT_WINDOW":"12345", "CLAUDE_CODE_MAX_OUTPUT_TOKENS":"999",
                "CLAUDE_CODE_EFFORT_LEVEL":"future-effort", "KEEP":{"nested":true}
            },
            "model":"claude-example", "effortLevel":"future-global",
            "autoCompactWindow":12345,
            "modelPicker":{"options":[{"model":"claude-example","label":"Keep"}]},
            "modelSettings":{
                "claude-example":{"effortLevel":"low","autoCompactWindow":54321,"metadata":[1,2]},
                "other":{"effortLevel":"future-effort","autoCompactWindow":67890}
            },
            "permissions":{"allow":["Read"]}, "metadata":{"keep":true}
        });
        let original = serde_json::to_vec(&settings).unwrap();
        fs::write(config_path(&root), &original).unwrap();
        for value in [Some(1), Some(200_000), Some(MAX_SAFE_INTEGER), None] {
            let mut change = input(&view(&root, &data).unwrap());
            change.context_window_change = Some(ContextWindowChange { value });
            let saved = save(&root, &data, &change).unwrap();
            let mut expected = settings.clone();
            if let Some(value) = value {
                expected["env"][MAX_CONTEXT_TOKENS] = json!(value.to_string());
                assert_eq!(saved["contextWindow"], value.to_string());
            } else {
                expected["env"]
                    .as_object_mut()
                    .unwrap()
                    .remove(MAX_CONTEXT_TOKENS);
                assert_eq!(saved["contextWindow"], Value::Null);
            }
            assert_eq!(parse(&snapshot(&root).unwrap()).unwrap(), expected);
            assert_eq!(saved["canRestore"], true);
            let restored = restore(&root, &data, saved["revision"].as_str().unwrap()).unwrap();
            assert_eq!(fs::read(config_path(&root)).unwrap(), original);
            assert_eq!(
                restored["contextWindow"],
                settings["env"][MAX_CONTEXT_TOKENS]
            );
            assert_eq!(restored["canRestore"], false);
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn context_patch_and_model_effort_patch_are_independent() {
        let (base, root, data) = fixture();
        let settings = json!({
            "env":{"CLAUDE_CODE_EFFORT_LEVEL":"medium"},
            "effortLevel":"high",
            "modelSettings":{
                "claude-example":{"effortLevel":"low","autoCompactWindow":12345},
                "other":{"effortLevel":"future"}
            }
        });
        fs::write(config_path(&root), serde_json::to_vec(&settings).unwrap()).unwrap();
        let mut change = input(&view(&root, &data).unwrap());
        change.context_window_change = Some(ContextWindowChange {
            value: Some(200_000),
        });
        change
            .model_effort_changes
            .insert("claude-example".into(), "xhigh".into());
        let saved = save(&root, &data, &change).unwrap();
        assert_eq!(saved["contextWindow"], "200000");
        assert_eq!(saved["modelEfforts"]["claude-example"], "xhigh");
        change.revision = saved["revision"].as_str().unwrap().into();
        change.context_window_change = Some(ContextWindowChange { value: None });
        change
            .model_effort_changes
            .insert("claude-example".into(), "".into());
        save(&root, &data, &change).unwrap();
        let doc = parse(&snapshot(&root).unwrap()).unwrap();
        assert!(doc["env"].get(MAX_CONTEXT_TOKENS).is_none());
        assert_eq!(
            doc["modelSettings"]["claude-example"],
            json!({"autoCompactWindow":12345})
        );
        assert_eq!(
            doc["modelSettings"]["other"],
            settings["modelSettings"]["other"]
        );
        assert_eq!(doc["effortLevel"], "high");
        assert_eq!(doc["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "medium");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn invalid_context_patch_never_writes_settings_or_backup() {
        let (base, root, data) = fixture();
        for initial in [
            None,
            Some(br#"{"env":{"CLAUDE_CODE_MAX_CONTEXT_TOKENS":"unknown"}}"#.to_vec()),
        ] {
            if let Some(bytes) = &initial {
                fs::write(config_path(&root), bytes).unwrap();
            }
            let before = view(&root, &data).unwrap();
            for value in [0, MAX_SAFE_INTEGER + 1, u64::MAX] {
                let mut change = input(&before);
                change.context_window_change = Some(ContextWindowChange { value: Some(value) });
                assert!(save(&root, &data, &change)
                    .unwrap_err()
                    .contains("上下文窗口"));
                assert_eq!(snapshot(&root).unwrap(), initial);
                assert!(!backup_path(&data).exists());
            }
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn context_clear_missing_does_not_create_env_and_non_object_env_is_rejected() {
        let mut doc = json!({"metadata":"keep"});
        let original = doc.clone();
        apply_context_window(&mut doc, Some(&ContextWindowChange { value: None })).unwrap();
        assert_eq!(doc, original);
        let (base, root, data) = fixture();
        for env in [Value::Null, json!(false), json!("keep"), json!([1])] {
            let settings = json!({"env":env,"metadata":"keep"});
            let original = serde_json::to_vec(&settings).unwrap();
            fs::write(config_path(&root), &original).unwrap();
            let before = view(&root, &data).unwrap();
            for value in [None, Some(200_000)] {
                let mut doc = settings.clone();
                let patch = ContextWindowChange { value };
                assert!(apply_context_window(&mut doc, Some(&patch))
                    .unwrap_err()
                    .contains("env"));
                assert_eq!(doc, settings);
                let mut change = input(&before);
                change.context_window_change = Some(patch);
                assert!(save(&root, &data, &change).unwrap_err().contains("env"));
                assert_eq!(fs::read(config_path(&root)).unwrap(), original);
                assert!(!backup_path(&data).exists());
            }
            let mut untouched = settings.clone();
            apply_context_window(&mut untouched, None).unwrap();
            assert_eq!(untouched, settings);
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn context_clear_absent_key_matches_ordinary_save_and_does_not_add_compaction_keys() {
        let (base, root, data) = fixture();
        let before = view(&root, &data).unwrap();
        assert_eq!(before["contextWindow"], Value::Null);
        let saved = save(&root, &data, &input(&before)).unwrap();
        let ordinary = snapshot(&root).unwrap();
        restore(&root, &data, saved["revision"].as_str().unwrap()).unwrap();
        let mut clear = input(&view(&root, &data).unwrap());
        clear.context_window_change = Some(ContextWindowChange { value: None });
        let saved = save(&root, &data, &clear).unwrap();
        assert_eq!(snapshot(&root).unwrap(), ordinary);
        assert_eq!(saved["contextWindow"], Value::Null);
        let mut set = input(&saved);
        set.context_window_change = Some(ContextWindowChange {
            value: Some(1_000_000),
        });
        save(&root, &data, &set).unwrap();
        let doc = parse(&snapshot(&root).unwrap()).unwrap();
        assert_eq!(
            doc["env"],
            json!({
                BASE_URL:"https://example.invalid", AUTH_TOKEN:"dummy-auth-token",
                MODEL:"claude-example", MAX_CONTEXT_TOKENS:"1000000"
            })
        );
        assert!(doc.get("modelSettings").is_none());
        assert!(doc.get("autoCompactWindow").is_none());
        fs::remove_dir_all(base).unwrap();
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
    #[test]
    fn multiple_models_and_effort_roundtrip_preserve_picker_metadata_and_restore() {
        let (base, root, data) = fixture();
        let original = br#"{
          "env":{"ANTHROPIC_BASE_URL":"https://old.invalid", "ANTHROPIC_MODEL":"model-a", "CLAUDE_CODE_EFFORT_LEVEL":"low", "KEEP":"yes"},
          "modelPicker":{"replaceBuiltInOptions":true,"options":[{"model":"model-a","label":"My A","description":"keep"}]},
          "modelSettings":{"model-a":{"effortLevel":"low","autoCompactWindow":200000},"other":{"effortLevel":"medium"}},
          "permissions":{"allow":["Read"]}
        }"#;
        fs::write(root.join("settings.json"), original).unwrap();
        let before = view(&root, &data).unwrap();
        assert_eq!(before["reasoningEffort"], "low");
        let mut change = input(&before);
        change.model = "model-b".into();
        change.models = Some(vec!["model-a".into(), "model-b".into(), "model-a".into()]);
        change.reasoning_effort = Some("high".into());
        let saved = save(&root, &data, &change).unwrap();
        assert_eq!(saved["model"], "model-b");
        assert_eq!(saved["models"], json!(["model-a", "model-b"]));
        assert_eq!(saved["reasoningEffort"], "low");
        assert_eq!(saved["modelEfforts"]["model-b"], "high");
        assert_eq!(saved["effortOverrides"]["environment"], "low");
        let doc: Value =
            serde_json::from_slice(&fs::read(root.join("settings.json")).unwrap()).unwrap();
        assert_eq!(doc["model"], "model-b");
        assert_eq!(doc["env"]["ANTHROPIC_MODEL"], "model-b");
        assert_eq!(doc["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "low");
        assert!(doc.get("effortLevel").is_none());
        assert_eq!(doc["modelSettings"]["model-b"]["effortLevel"], "high");
        assert_eq!(doc["env"]["KEEP"], "yes");
        assert_eq!(doc["modelPicker"]["options"][0]["label"], "My A");
        assert_eq!(doc["modelPicker"]["replaceBuiltInOptions"], true);
        assert_eq!(doc["modelSettings"]["model-a"]["effortLevel"], "low");
        assert_eq!(doc["modelSettings"]["model-a"]["autoCompactWindow"], 200000);
        assert_eq!(doc["modelSettings"]["other"]["effortLevel"], "medium");
        assert_eq!(doc["permissions"]["allow"], json!(["Read"]));
        restore(&root, &data, saved["revision"].as_str().unwrap()).unwrap();
        assert_eq!(fs::read(root.join("settings.json")).unwrap(), original);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn omitted_effort_patches_preserve_different_and_unknown_levels() {
        let (base, root, data) = fixture();
        let settings = json!({
            "env": {"CLAUDE_CODE_EFFORT_LEVEL": "future-env", "ANTHROPIC_MODEL": "model-a"},
            "model": "model-a",
            "effortLevel": "future-global",
            "modelSettings": {
                "model-a": {"effortLevel": "low", "autoCompactWindow": 12345},
                "model-b": {"effortLevel": "future-level"},
                "unselected": {"effortLevel": "xhigh"},
                "metadata-only": {"autoCompactWindow": 45678}
            }
        });
        fs::write(config_path(&root), serde_json::to_vec(&settings).unwrap()).unwrap();
        let before = view(&root, &data).unwrap();
        assert_eq!(before["modelEfforts"]["model-b"], "future-level");
        assert_eq!(before["modelEfforts"]["metadata-only"], "");
        let mut change = input(&before);
        change.model = "model-a".into();
        change.models = Some(vec!["model-a".into(), "model-b".into()]);
        let saved = save(&root, &data, &change).unwrap();
        let address_only = parse(&snapshot(&root).unwrap()).unwrap();
        assert_eq!(address_only["modelSettings"], settings["modelSettings"]);
        change.model = "model-b".into();
        change.revision = saved["revision"].as_str().unwrap().into();
        let saved = save(&root, &data, &change).unwrap();
        let doc = parse(&snapshot(&root).unwrap()).unwrap();
        assert_eq!(doc["modelSettings"], settings["modelSettings"]);
        assert_eq!(doc["effortLevel"], settings["effortLevel"]);
        assert_eq!(doc["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "future-env");
        assert_eq!(saved["modelEffortKeys"]["model-b"], "model-b");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn setting_and_clearing_one_model_preserve_other_models_and_overrides() {
        let (base, root, data) = fixture();
        let settings = json!({
            "env": {"CLAUDE_CODE_EFFORT_LEVEL": "high"},
            "effortLevel": "medium",
            "modelSettings": {
                "model-a": {"effortLevel": "low", "autoCompactWindow": 12345},
                "model-b": {"effortLevel": "xhigh"}
            }
        });
        fs::write(config_path(&root), serde_json::to_vec(&settings).unwrap()).unwrap();
        let mut change = input(&view(&root, &data).unwrap());
        change.model = "model-a".into();
        change.models = Some(vec!["model-a".into(), "model-b".into()]);
        for effort in ["high", ""] {
            change
                .model_effort_changes
                .insert("model-a".into(), effort.into());
            let saved = save(&root, &data, &change).unwrap();
            let doc = parse(&snapshot(&root).unwrap()).unwrap();
            assert_eq!(
                doc["modelSettings"]["model-b"],
                settings["modelSettings"]["model-b"]
            );
            assert_eq!(doc["modelSettings"]["model-a"]["autoCompactWindow"], 12345);
            assert_eq!(doc["effortLevel"], "medium");
            assert_eq!(doc["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "high");
            assert_eq!(saved["modelEfforts"]["model-a"], effort);
            change.revision = saved["revision"].as_str().unwrap().into();
        }
        let doc = parse(&snapshot(&root).unwrap()).unwrap();
        assert!(doc["modelSettings"]["model-a"].get("effortLevel").is_none());
        change.model_effort_changes.clear();
        change.clear_effort_overrides = true;
        let saved = save(&root, &data, &change).unwrap();
        let cleared = parse(&snapshot(&root).unwrap()).unwrap();
        assert_eq!(cleared["modelSettings"], doc["modelSettings"]);
        assert!(cleared.get("effortLevel").is_none());
        assert!(cleared["env"].get("CLAUDE_CODE_EFFORT_LEVEL").is_none());
        assert_eq!(
            saved["effortOverrides"],
            json!({"environment":"", "global":""})
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn alias_mapping_uses_only_explicit_identity_and_preserves_metadata() {
        let (base, root, data) = fixture();
        let settings = json!({
            "env": {"ANTHROPIC_DEFAULT_SONNET_MODEL": "claude-sonnet-4-6-20260101[1m]"},
            "modelPicker": {"options": [{"model":"sonnet"}]},
            "modelSettings": {"claude-sonnet-4-6": {"effortLevel":"low", "autoCompactWindow":12345}}
        });
        fs::write(config_path(&root), serde_json::to_vec(&settings).unwrap()).unwrap();
        let before = view(&root, &data).unwrap();
        assert_eq!(before["modelEffortKeys"]["sonnet"], "claude-sonnet-4-6");
        let mut change = input(&before);
        change.model = "sonnet".into();
        change
            .model_effort_changes
            .insert("sonnet".into(), "high".into());
        save(&root, &data, &change).unwrap();
        let doc = parse(&snapshot(&root).unwrap()).unwrap();
        assert_eq!(
            doc["modelSettings"]["claude-sonnet-4-6"]["effortLevel"],
            "high"
        );
        assert_eq!(
            doc["modelSettings"]["claude-sonnet-4-6"]["autoCompactWindow"],
            12345
        );
        assert!(doc["modelSettings"].get("sonnet").is_none());
        assert_eq!(reasoning_effort(&doc), "high");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn canonical_keys_are_conservative_and_gateway_ids_are_opaque() {
        let doc = json!({
            "env": {
                "ANTHROPIC_DEFAULT_SONNET_MODEL": "claude-sonnet-4-6-20260101[1m]",
                "ANTHROPIC_DEFAULT_FABLE_MODEL": "claude-fable-5-1",
                "ANTHROPIC_DEFAULT_BEST_MODEL": "claude-opus-4-6"
            },
            "modelSettings": {
                "sonnet": {"effortLevel":"low"},
                "claude-sonnet-4-6": {},
                "claude-opus-4-6-20260101": {},
                "claude-opus-4-6-20260202": {},
                "gateway-model": {},
                "claude-haiku-4-5-20260101[1m]": {}
            }
        });
        assert_eq!(effort_key(&doc, "sonnet").unwrap(), "sonnet");
        assert_eq!(
            effort_key(&doc, "claude-sonnet-4-6-20260101[1m]").unwrap(),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            effort_key(&doc, "claude-haiku-4-5").unwrap(),
            "claude-haiku-4-5-20260101[1m]"
        );
        assert_eq!(effort_key(&doc, "fable").unwrap(), "claude-fable-5-1");
        assert_eq!(
            effort_key(&doc, "sonnet-custom-gateway").unwrap(),
            "sonnet-custom-gateway"
        );
        let opus_env = json!({"env":{"ANTHROPIC_DEFAULT_OPUS_MODEL":"claude-opus-4-6"}});
        assert!(effort_key(&opus_env, "best")
            .unwrap_err()
            .contains("完整模型 ID"));
        assert_eq!(
            effort_key(&doc, "gateway-model-20260101").unwrap(),
            "gateway-model-20260101"
        );
        assert_eq!(
            effort_key(&doc, "gateway-model[1m]").unwrap(),
            "gateway-model[1m]"
        );
        assert!(
            effort_key(&doc, "best").is_err(),
            "ambiguous dated settings must not be guessed"
        );
        for alias in ["opus", "haiku", "default", "opusplan", "sonnet-4-6"] {
            assert!(effort_key(&doc, alias).unwrap_err().contains("完整模型 ID"));
        }
        let unresolved = json!({"modelSettings":{"claude-sonnet-4-6":{}}});
        assert!(
            effort_key(&unresolved, "sonnet").is_err(),
            "an existing release does not resolve an alias without env"
        );
    }

    #[test]
    fn new_dated_claude_ids_save_canonically_but_existing_exact_keys_win() {
        let (base, root, data) = fixture();
        for id in [
            "claude-sonnet-4-6-20260101",
            "claude-sonnet-4-6-20260101[1m]",
        ] {
            fs::write(config_path(&root), b"{}").unwrap();
            assert_eq!(effort_key(&json!({}), id).unwrap(), "claude-sonnet-4-6");
            let mut change = input(&view(&root, &data).unwrap());
            change.model = id.into();
            change.model_effort_changes.insert(id.into(), "high".into());
            let saved = save(&root, &data, &change).unwrap();
            let doc = parse(&snapshot(&root).unwrap()).unwrap();
            assert_eq!(
                doc["modelSettings"],
                json!({"claude-sonnet-4-6":{"effortLevel":"high"}})
            );
            assert_eq!(saved["modelEffortKeys"][id], "claude-sonnet-4-6");

            let settings = json!({"modelSettings": {
                id: {"effortLevel":"low", "autoCompactWindow":12345},
                "claude-sonnet-4-6": {"effortLevel":"medium"}
            }});
            fs::write(config_path(&root), serde_json::to_vec(&settings).unwrap()).unwrap();
            assert_eq!(effort_key(&settings, id).unwrap(), id);
            change.revision = view(&root, &data).unwrap()["revision"]
                .as_str()
                .unwrap()
                .into();
            save(&root, &data, &change).unwrap();
            let doc = parse(&snapshot(&root).unwrap()).unwrap();
            assert_eq!(doc["modelSettings"][id]["effortLevel"], "high");
            assert_eq!(doc["modelSettings"][id]["autoCompactWindow"], 12345);
            assert_eq!(
                doc["modelSettings"]["claude-sonnet-4-6"]["effortLevel"],
                "medium"
            );
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn best_requires_its_own_mapping_and_never_guesses_a_family() {
        let (base, root, data) = fixture();
        for env in [
            json!({"ANTHROPIC_DEFAULT_OPUS_MODEL":"claude-opus-4-6"}),
            json!({"ANTHROPIC_DEFAULT_FABLE_MODEL":"claude-fable-5-1"}),
            json!({"ANTHROPIC_DEFAULT_OPUS_MODEL":"claude-opus-4-6", "ANTHROPIC_DEFAULT_FABLE_MODEL":"claude-fable-5-1"}),
        ] {
            let settings = json!({"env":env});
            let original = serde_json::to_vec(&settings).unwrap();
            fs::write(config_path(&root), &original).unwrap();
            let mut change = input(&view(&root, &data).unwrap());
            change.model = "best".into();
            change
                .model_effort_changes
                .insert("best".into(), "high".into());
            assert!(save(&root, &data, &change)
                .unwrap_err()
                .contains("完整模型 ID"));
            assert_eq!(fs::read(config_path(&root)).unwrap(), original);
            assert!(!backup_path(&data).exists());
        }
        let settings = json!({"env": {
            "ANTHROPIC_DEFAULT_BEST_MODEL":"claude-fable-5-1-20260101[1m]",
            "ANTHROPIC_DEFAULT_OPUS_MODEL":"claude-opus-4-6"
        }});
        fs::write(config_path(&root), serde_json::to_vec(&settings).unwrap()).unwrap();
        let mut change = input(&view(&root, &data).unwrap());
        change.model = "best".into();
        change
            .model_effort_changes
            .insert("best".into(), "high".into());
        let saved = save(&root, &data, &change).unwrap();
        assert_eq!(saved["modelEffortKeys"]["best"], "claude-fable-5-1");
        assert_eq!(saved["modelEfforts"], json!({"claude-fable-5-1":"high"}));
        assert_eq!(
            effort_key(&json!({"modelSettings":{"best":{}}}), "best").unwrap(),
            "best"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn invalid_patches_never_write_config_or_backup() {
        let (base, root, data) = fixture();
        let original = br#"{"modelSettings":{"model-a":{"effortLevel":"future-level"}}}"#;
        fs::write(config_path(&root), original).unwrap();
        let before = view(&root, &data).unwrap();
        for (id, effort) in [
            ("unselected", "high"),
            ("model-a", "ultra"),
            ("sonnet", "high"),
        ] {
            let mut change = input(&before);
            change.model = "model-a".into();
            change.models = Some(vec!["model-a".into(), "sonnet".into()]);
            change.model_effort_changes.insert(id.into(), effort.into());
            change.clear_effort_overrides = true;
            assert!(save(&root, &data, &change).is_err());
            assert_eq!(fs::read(config_path(&root)).unwrap(), original);
            assert!(!backup_path(&data).exists());
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn legacy_empty_effort_clears_only_default_and_empty_patch_does_not_create_settings() {
        let (base, root, data) = fixture();
        let settings = json!({
            "env": {"CLAUDE_CODE_EFFORT_LEVEL":"xhigh"},
            "effortLevel":"medium",
            "modelSettings": {"model-a":{"effortLevel":"high"}, "model-b":{"effortLevel":"low"}}
        });
        fs::write(config_path(&root), serde_json::to_vec(&settings).unwrap()).unwrap();
        let mut clear = input(&view(&root, &data).unwrap());
        clear.model = "model-a".into();
        clear.models = Some(vec!["model-a".into(), "model-b".into()]);
        clear.reasoning_effort = Some("".into());
        save(&root, &data, &clear).unwrap();
        let doc = parse(&snapshot(&root).unwrap()).unwrap();
        assert!(doc["modelSettings"]["model-a"].get("effortLevel").is_none());
        assert_eq!(doc["modelSettings"]["model-b"]["effortLevel"], "low");
        assert_eq!(doc["effortLevel"], "medium");
        assert_eq!(doc["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "xhigh");
        fs::write(config_path(&root), b"{}").unwrap();
        let mut clear = input(&view(&root, &data).unwrap());
        clear.reasoning_effort = Some("".into());
        save(&root, &data, &clear).unwrap();
        assert!(parse(&snapshot(&root).unwrap())
            .unwrap()
            .get("modelSettings")
            .is_none());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn conflicting_alias_patches_never_write() {
        let (base, root, data) = fixture();
        let original = br#"{"env":{"ANTHROPIC_DEFAULT_SONNET_MODEL":"claude-sonnet-4-6"}}"#;
        fs::write(config_path(&root), original).unwrap();
        let mut change = input(&view(&root, &data).unwrap());
        change.model = "sonnet".into();
        change.models = Some(vec!["sonnet".into(), "claude-sonnet-4-6".into()]);
        change.model_effort_changes = BTreeMap::from([
            ("sonnet".into(), "high".into()),
            ("claude-sonnet-4-6".into(), "low".into()),
        ]);
        assert!(save(&root, &data, &change).unwrap_err().contains("冲突"));
        assert_eq!(fs::read(config_path(&root)).unwrap(), original);
        assert!(!backup_path(&data).exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn input_defaults_and_legacy_effort_are_explicit_patches_only() {
        let value = json!({"baseUrl":"https://example.invalid", "authToken":"dummy", "model":"model-a", "revision":"test"});
        let missing: Input = serde_json::from_value(value.clone()).unwrap();
        assert!(missing.reasoning_effort.is_none());
        assert!(missing.model_effort_changes.is_empty());
        assert!(!missing.clear_effort_overrides);
        let mut patched = value;
        patched["reasoningEffort"] = json!("low");
        patched["modelEffortChanges"] = json!({"model-a":"high", "model-b":"medium"});
        patched["clearEffortOverrides"] = json!(true);
        let explicit: Input = serde_json::from_value(patched).unwrap();
        assert!(explicit.clear_effort_overrides);
        let changes =
            effort_changes(&json!({}), &explicit, &["model-a".into(), "model-b".into()]).unwrap();
        assert_eq!(
            changes,
            BTreeMap::from([
                ("model-a".into(), "high".into()),
                ("model-b".into(), "medium".into())
            ])
        );
    }

    #[test]
    fn clearing_effort_removes_only_model_field_and_invalid_choices_never_write() {
        let (base, root, data) = fixture();
        let before = view(&root, &data).unwrap();
        let mut change = input(&before);
        change.models = Some(vec!["extra-model".into()]);
        change
            .model_effort_changes
            .insert("extra-model".into(), "xhigh".into());
        let saved = save(&root, &data, &change).unwrap();
        let mut clear = input(&saved);
        clear.models = Some(vec!["extra-model".into()]);
        clear
            .model_effort_changes
            .insert("extra-model".into(), "".into());
        let saved = save(&root, &data, &clear).unwrap();
        assert_eq!(saved["reasoningEffort"], "");
        let doc: Value =
            serde_json::from_slice(&fs::read(root.join("settings.json")).unwrap()).unwrap();
        assert!(doc.get("effortLevel").is_none());
        assert!(doc["modelSettings"]["extra-model"]
            .get("effortLevel")
            .is_none());
        let original = fs::read(root.join("settings.json")).unwrap();
        for (models, effort) in [
            (vec!["bad\nmodel".into()], "high"),
            (vec!["valid".into()], "ultra"),
        ] {
            let mut invalid = input(&saved);
            invalid.models = Some(models);
            invalid.reasoning_effort = Some(effort.into());
            assert!(save(&root, &data, &invalid).is_err());
            assert_eq!(fs::read(root.join("settings.json")).unwrap(), original);
        }
        fs::remove_dir_all(base).unwrap();
    }
}
