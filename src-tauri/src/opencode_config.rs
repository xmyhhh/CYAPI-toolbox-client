//! OpenCode API configuration, with JSONC-preserving edits and guarded local writes.
//! Only the `cyapi` provider and the default model reference are managed; every
//! other key, comment, and formatting choice in the file is preserved.
use crate::configuration::{no_links, read_bytes, stage};
use base64::{engine::general_purpose::STANDARD, Engine};
use jsonc_parser::cst::{CstInputValue, CstObject, CstRootNode};
use jsonc_parser::ParseOptions;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, String>;

const PROVIDER: &str = "cyapi";
const PROVIDER_NPM: &str = "@ai-sdk/openai-compatible";
const MODEL_ALIAS: &str = "default";

/// A config directory and an explicit OPENCODE_CONFIG file have different
/// semantics. An explicit file may have any name and need not exist yet.
#[derive(Clone)]
pub enum Location {
    Directory(PathBuf),
    File(PathBuf),
}

impl Location {
    pub fn resolve(&self) -> Result<PathBuf> {
        match self {
            Self::Directory(root) => config_path(root),
            Self::File(path) => {
                let path = if path.is_absolute() {
                    path.clone()
                } else {
                    std::env::current_dir()
                        .map_err(|_| "无法解析 OpenCode 配置路径")?
                        .join(path)
                };
                no_links(&path)?;
                Ok(path)
            }
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Input {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub models: Option<Vec<String>>,
    #[serde(default)]
    pub reasoning_effort: String,
    pub revision: String,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
struct Snapshot {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
struct Backup {
    schema: u32,
    directory: PathBuf,
    name: String,
    original: Option<String>,
    after: Option<String>,
}

impl Backup {
    fn original(&self) -> Result<Option<Vec<u8>>> {
        let bytes = self
            .original
            .as_ref()
            .map(|value| {
                STANDARD
                    .decode(value)
                    .map_err(|_| "OpenCode 备份无效".to_string())
            })
            .transpose()?;
        if bytes.as_ref().is_some_and(|bytes| bytes.len() > 1_000_000) {
            return Err("OpenCode 备份过大".into());
        }
        Ok(bytes)
    }

    fn matches(&self, current: &Snapshot) -> Result<bool> {
        if self.schema != 1 || self.directory.join(&self.name) != current.path {
            return Ok(false);
        }
        let original = self.original()?;
        Ok(current.bytes == original
            || current
                .bytes
                .as_ref()
                .map(|bytes| crate::service::hash(bytes))
                .as_ref()
                == self.after.as_ref())
    }
}

fn backup_path(data: &Path) -> PathBuf {
    data.join("opencode-config/previous.json")
}

fn config_path(root: &Path) -> Result<PathBuf> {
    // OpenCode prefers .jsonc when both files exist. Match that precedence so the
    // editor never writes a lower-precedence file while the user sees another one.
    for name in ["opencode.jsonc", "opencode.json"] {
        let path = root.join(name);
        no_links(&path)?;
        match fs::metadata(&path) {
            Ok(meta) if meta.is_file() => return Ok(path),
            Ok(_) => return Err("OpenCode 配置路径不是普通文件".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("无法读取 OpenCode 配置路径".into()),
        }
    }
    Ok(root.join("opencode.json"))
}

fn snapshot(location: &Location) -> Result<Snapshot> {
    let path = location.resolve()?;
    Ok(Snapshot {
        bytes: read_bytes(&path)?,
        path,
    })
}

fn revision(snapshot: &Snapshot) -> String {
    crate::service::hash(&serde_json::to_vec(snapshot).unwrap_or_default())
}

fn parse_text(bytes: &Snapshot) -> Result<&str> {
    let text = std::str::from_utf8(bytes.bytes.as_deref().unwrap_or_default())
        .map_err(|_| "OpenCode 配置文件不是 UTF-8 文件")?;
    Ok(text.trim_start_matches('\u{feff}'))
}

fn parse_value(snapshot: &Snapshot) -> Result<Value> {
    let text = parse_text(snapshot)?;
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    // Never surface parser errors: a local file can contain credentials or
    // snippets that must not be copied into the toolbox log.
    let value: Value = jsonc_parser::parse_to_serde_value(text, &ParseOptions::default())
        .map_err(|_| "OpenCode 配置格式无效，请修复后重新读取")?;
    if !value.is_object() {
        return Err("OpenCode 配置文件必须是 JSON 对象".into());
    }
    Ok(value)
}

fn parse_cst(snapshot: &Snapshot) -> Result<CstRootNode> {
    let text = parse_text(snapshot)?;
    if text.trim().is_empty() {
        return CstRootNode::parse("{}", &ParseOptions::default())
            .map_err(|_| "OpenCode 配置格式无效".to_string());
    }
    let root = CstRootNode::parse(text, &ParseOptions::default())
        .map_err(|_| "OpenCode 配置格式无效，请修复后重新读取")?;
    if root.value().is_some() && root.object_value().is_none() {
        return Err("OpenCode 配置文件必须是 JSON 对象".into());
    }
    Ok(root)
}

fn read_backup(data: &Path) -> Result<Option<Backup>> {
    let path = backup_path(data);
    no_links(&path)?;
    match fs::metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(meta) if meta.is_file() && meta.len() <= 3_000_000 => {}
        _ => return Err("无法读取 OpenCode 备份".into()),
    }
    let backup: Backup =
        serde_json::from_slice(&fs::read(path).map_err(|_| "无法读取 OpenCode 备份")?)
            .map_err(|_| "OpenCode 备份无效")?;
    if backup.schema != 1 {
        return Err("不支持的 OpenCode 备份格式".into());
    }
    backup.original()?;
    Ok(Some(backup))
}

fn text_at<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    let mut node = value;
    for key in keys {
        node = node.get(*key)?;
    }
    node.as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn provider_of(doc: &Value) -> Option<&Value> {
    doc.get("provider").and_then(|value| value.get(PROVIDER))
}

fn api_key(doc: &Value) -> Option<&str> {
    provider_of(doc).and_then(|provider| text_at(provider, &["options", "apiKey"]))
}

fn base_url(doc: &Value) -> Option<&str> {
    let provider = provider_of(doc)?;
    text_at(provider, &["api"])
        .or_else(|| text_at(provider, &["options", "baseURL"]))
        .or_else(|| text_at(provider, &["options", "api"]))
}

fn model_entries(doc: &Value) -> Option<&serde_json::Map<String, Value>> {
    provider_of(doc)?.get("models")?.as_object()
}

fn model_id<'a>(alias: &'a str, entry: &'a Value) -> &'a str {
    text_at(entry, &["id"]).unwrap_or(alias)
}

fn model_alias(doc: &Value) -> Option<&str> {
    text_at(doc, &["model"])?
        .strip_prefix("cyapi/")?
        .split('#')
        .next()
}

fn model(doc: &Value) -> Option<&str> {
    if let Some(alias) = model_alias(doc) {
        return Some(
            model_entries(doc)
                .and_then(|entries| entries.get(alias))
                .map(|entry| model_id(alias, entry))
                .unwrap_or(alias),
        );
    }
    model_entries(doc)?
        .get(MODEL_ALIAS)
        .map(|entry| model_id(MODEL_ALIAS, entry))
}

fn selected_models(doc: &Value) -> Vec<String> {
    model_entries(doc)
        .map(|entries| {
            entries
                .iter()
                .map(|(alias, entry)| model_id(alias, entry).to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn reasoning_effort(doc: &Value) -> &str {
    let alias = model_alias(doc).unwrap_or(MODEL_ALIAS);
    model_entries(doc)
        .and_then(|entries| entries.get(alias))
        .and_then(|entry| text_at(entry, &["options", "reasoningEffort"]))
        .unwrap_or("")
}

pub(crate) fn detect_cli() -> (bool, String) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if let Ok(output) = std::process::Command::new("where.exe")
            .arg("opencode")
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
        if let Ok(output) = std::process::Command::new("which").arg("opencode").output() {
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

/// The major version of the installed OpenCode, or 0 when it cannot be detected.
/// OpenCode V2 moved MCP servers under `mcp.servers`, so the layout depends on it.
pub(crate) fn major_version() -> u32 {
    static VERSION: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *VERSION.get_or_init(|| {
        let (found, path) = detect_cli();
        if !found {
            return 0;
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let output = std::process::Command::new(path)
                .arg("--version")
                .creation_flags(CREATE_NO_WINDOW)
                .output();
            match output {
                Ok(output) => parse_major(&String::from_utf8_lossy(&output.stdout)),
                Err(_) => 0,
            }
        }
        #[cfg(not(windows))]
        {
            match std::process::Command::new(path).arg("--version").output() {
                Ok(output) => parse_major(&String::from_utf8_lossy(&output.stdout)),
                Err(_) => 0,
            }
        }
    })
}

fn parse_major(text: &str) -> u32 {
    text.split(|c: char| !c.is_ascii_digit())
        .find(|part| !part.is_empty())
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or(0)
}

fn snapshot_view(data: &Path, snapshot: &Snapshot, secret: bool) -> Result<Value> {
    let doc = parse_value(snapshot)?;
    let key = api_key(&doc);
    let can_restore = read_backup(data)
        .ok()
        .flatten()
        .and_then(|backup| backup.matches(snapshot).ok())
        .unwrap_or(false);
    let (cli_installed, cli_path) = detect_cli();
    let mut view = json!({
        "configPath": snapshot.path,
        "revision": revision(snapshot),
        "baseUrl": base_url(&doc).unwrap_or(""),
        "model": model(&doc).unwrap_or(""),
        "models": selected_models(&doc),
        "reasoningEffort": reasoning_effort(&doc),
        "apiKeyConfigured": key.is_some(),
        "authMode": if key.is_some() { "API Key" } else { "未配置" },
        "settingsExists": snapshot.bytes.is_some(),
        "cliInstalled": cli_installed,
        "cliPath": cli_path,
        "canRestore": can_restore
    });
    if secret {
        view["apiKey"] = json!(key.unwrap_or(""));
    }
    Ok(view)
}

// Read-only status shape; the local editor uses editor_view below to fill the password field.
#[allow(dead_code)]
pub fn view(root: &Path, data: &Path) -> Result<Value> {
    snapshot_view(data, &snapshot(&Location::Directory(root.into()))?, false)
}

// Only the local editor requests the credential for the password field.
#[allow(dead_code)]
pub fn editor_view(root: &Path, data: &Path) -> Result<Value> {
    editor_view_at(&Location::Directory(root.into()), data)
}

pub fn editor_view_at(location: &Location, data: &Path) -> Result<Value> {
    snapshot_view(data, &snapshot(location)?, true)
}

fn normalize_url(raw: &str) -> String {
    raw.trim().trim_end_matches('/').to_string()
}

fn validate(input: &Input) -> Result<()> {
    crate::model_selection::models(&input.model, input.models.as_deref().unwrap_or_default())?;
    crate::model_selection::effort(&input.reasoning_effort, crate::model_selection::EFFORTS)?;
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
    if input.model.trim().is_empty()
        || input.model.len() > 200
        || input.model.chars().any(char::is_control)
    {
        return Err("请填写有效的模型 ID".into());
    }
    if input.api_key.len() > 8192
        || (input.api_key.trim().is_empty() && input.api_key.chars().any(char::is_control))
        || (!input.api_key.trim().is_empty()
            && input
                .api_key
                .trim()
                .chars()
                .any(|c| c.is_whitespace() || c.is_control()))
    {
        return Err("API Key 格式无效".into());
    }
    Ok(())
}

fn set_string(object: &CstObject, key: &str, value: &str) {
    let value = CstInputValue::String(value.into());
    match object.get(key) {
        Some(prop) => prop.set_value(value),
        None => {
            object.append(key, value);
        }
    }
}

fn update_if_present(object: &CstObject, key: &str, value: &str) {
    if let Some(prop) = object.get(key) {
        prop.set_value(CstInputValue::String(value.into()));
    }
}

fn commit(location: &Location, current: &Snapshot, next: &Snapshot) -> Result<()> {
    if current == next {
        return Ok(());
    }
    let path = &next.path;
    no_links(&path)?;
    if &snapshot(location)? != current {
        return Err("OpenCode 配置已被其他程序修改，请重新读取".into());
    }
    match next.bytes.as_ref() {
        Some(bytes) => {
            let temp = stage(&path, bytes)?;
            fs::rename(&temp, &path).map_err(|_| {
                let _ = fs::remove_file(&temp);
                "无法替换 OpenCode 配置文件，请检查是否被占用".into()
            })
        }
        None => match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("无法恢复 OpenCode 配置文件".into()),
        },
    }
}

#[allow(dead_code)]
pub fn save(root: &Path, data: &Path, input: &Input) -> Result<Value> {
    save_at(&Location::Directory(root.into()), data, input)
}

pub fn save_at(location: &Location, data: &Path, input: &Input) -> Result<Value> {
    validate(input)?;
    let current = snapshot(location)?;
    if revision(&current) != input.revision {
        return Err("OpenCode 配置已变化，请重新读取后保存".into());
    }
    let doc = parse_value(&current)?;
    let key = if input.api_key.trim().is_empty() {
        api_key(&doc).ok_or("请填写 API Key")?.to_string()
    } else {
        input.api_key.trim().to_string()
    };
    let base_url = normalize_url(&input.base_url);
    let model = input.model.trim();

    let root_node = parse_cst(&current)?;
    let object = root_node
        .object_value_or_create()
        .ok_or("OpenCode 配置文件必须是 JSON 对象")?;
    let provider = object
        .object_value_or_create("provider")
        .ok_or("OpenCode 配置的 provider 必须是 JSON 对象")?;
    let cyapi = provider
        .object_value_or_create(PROVIDER)
        .ok_or("OpenCode 配置的 provider.cyapi 必须是 JSON 对象")?;

    set_string(&cyapi, "npm", PROVIDER_NPM);
    set_string(&cyapi, "name", "CYAPI");
    set_string(&cyapi, "api", &base_url);
    let options = cyapi
        .object_value_or_create("options")
        .ok_or("OpenCode 配置的 provider.cyapi.options 必须是 JSON 对象")?;
    set_string(&options, "apiKey", &key);
    // Some existing configs put the endpoint in options.baseURL. Keep that
    // convention in sync when it is already present, so neither value goes stale.
    update_if_present(&options, "baseURL", &base_url);
    update_if_present(&options, "api", &base_url);

    let models = cyapi
        .object_value_or_create("models")
        .ok_or("OpenCode 配置的 provider.cyapi.models 必须是 JSON 对象")?;
    let selected = crate::model_selection::models(
        model,
        input.models.as_deref().unwrap_or(&selected_models(&doc)),
    )?;
    let existing = model_entries(&doc);
    let mut aliases = Vec::new();
    let mut default_alias = String::new();
    for id in selected {
        let alias = existing
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|(alias, entry)| model_id(alias, entry) == id)
            })
            .map(|(alias, _)| alias.clone())
            .unwrap_or_else(|| {
                if id == model && existing.is_none_or(|entries| !entries.contains_key(MODEL_ALIAS))
                {
                    MODEL_ALIAS.to_string()
                } else if existing.is_some_and(|entries| entries.contains_key(&id))
                    || id == MODEL_ALIAS
                    || id.contains('#')
                {
                    format!("model-{}", crate::service::hash(id.as_bytes()))
                } else {
                    id.clone()
                }
            });
        let entry = models
            .object_value_or_create(&alias)
            .ok_or("OpenCode 模型配置必须是 JSON 对象")?;
        set_string(&entry, "id", &id);
        if entry.get("name").is_none() {
            set_string(&entry, "name", &format!("CYAPI · {id}"));
        }
        let options = entry
            .object_value_or_create("options")
            .ok_or("OpenCode 模型 options 必须是 JSON 对象")?;
        if input.reasoning_effort.is_empty() {
            if let Some(prop) = options.get("reasoningEffort") {
                prop.remove();
            }
        } else {
            set_string(&options, "reasoningEffort", &input.reasoning_effort);
            if entry.get("reasoning").is_none() {
                entry.append("reasoning", CstInputValue::Bool(true));
            }
        }
        if id == model {
            default_alias = alias.clone();
        }
        aliases.push(alias);
    }
    if let Some(existing) = existing {
        for alias in existing.keys().filter(|alias| !aliases.contains(alias)) {
            if let Some(prop) = models.get(alias) {
                prop.remove();
            }
        }
    }
    set_string(&object, "model", &format!("{PROVIDER}/{default_alias}"));

    let mut bytes = root_node.to_string().into_bytes();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    if bytes.len() > 1_000_000 {
        return Err("生成的 OpenCode 配置文件过大".into());
    }
    let next = Snapshot {
        path: current.path.clone(),
        bytes: Some(bytes),
    };
    if next == current {
        return editor_view_at(location, data);
    }
    let backup = Backup {
        schema: 1,
        directory: current.path.parent().ok_or("OpenCode 配置目录无效")?.into(),
        name: current
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("OpenCode 配置文件名无效")?
            .into(),
        original: current.bytes.as_ref().map(|bytes| STANDARD.encode(bytes)),
        after: next.bytes.as_ref().map(|bytes| crate::service::hash(bytes)),
    };
    let backup_bytes = serde_json::to_vec(&backup).map_err(|_| "无法生成 OpenCode 备份")?;
    let backup_file = backup_path(data);
    if let Some(parent) = backup_file.parent() {
        fs::create_dir_all(parent).map_err(|_| "无法创建 OpenCode 备份目录")?;
    }
    let temp = stage(&backup_file, &backup_bytes)?;
    fs::rename(&temp, &backup_file).map_err(|_| {
        let _ = fs::remove_file(&temp);
        "无法写入 OpenCode 备份".to_string()
    })?;
    commit(location, &current, &next)?;
    editor_view_at(location, data)
}

#[allow(dead_code)]
pub fn restore(root: &Path, data: &Path, revision_value: &str) -> Result<Value> {
    restore_at(&Location::Directory(root.into()), data, revision_value)
}

pub fn restore_at(location: &Location, data: &Path, revision_value: &str) -> Result<Value> {
    let current = snapshot(location)?;
    if revision(&current) != revision_value {
        return Err("OpenCode 配置已变化，请重新读取后恢复".into());
    }
    let backup = read_backup(data)?.ok_or("没有可恢复的 OpenCode 配置")?;
    if !backup.matches(&current)? {
        return Err("OpenCode 配置已在外部修改，无法直接恢复旧备份".into());
    }
    let original = backup.original()?;
    let next = Snapshot {
        path: current.path.clone(),
        bytes: original,
    };
    // Parse before writing so a damaged backup is never published.
    parse_value(&next)?;
    commit(location, &current, &next)?;
    let path = backup_path(data);
    no_links(&path)?;
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("无法删除 OpenCode 备份".into()),
    }
    editor_view_at(location, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn fixture() -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "cyapi-opencode-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let root = base.join("home/.config/opencode");
        fs::create_dir_all(&root).unwrap();
        (base.clone(), root, base.join("data"))
    }

    fn input(view: &Value) -> Input {
        Input {
            base_url: "https://example.invalid/v1/".into(),
            api_key: "dummy-api-key".into(),
            models: None,
            reasoning_effort: "".into(),
            model: "example/model".into(),
            revision: view["revision"].as_str().unwrap().into(),
        }
    }

    #[test]
    fn saves_provider_and_model_while_preserving_jsonc_comments_and_settings() {
        let (base, root, data) = fixture();
        let config = br#"{
  // keep this comment
  "$schema": "https://opencode.ai/config.json",
  "theme": "dark",
  "mcp": { "demo": { "type": "local", "command": ["demo"] } }
}"#;
        fs::write(root.join("opencode.jsonc"), config).unwrap();

        let before = view(&root, &data).unwrap();
        assert_eq!(before["apiKeyConfigured"], false);
        assert!(before.get("apiKey").is_none());
        assert_eq!(before["settingsExists"], true);

        let saved = save(&root, &data, &input(&before)).unwrap();
        assert_eq!(saved["baseUrl"], "https://example.invalid/v1");
        assert_eq!(saved["model"], "example/model");
        assert_eq!(saved["apiKeyConfigured"], true);
        assert_eq!(saved["canRestore"], true);
        assert!(saved["apiKey"].is_string(), "editor view returns the key");
        assert!(!view(&root, &data)
            .unwrap()
            .to_string()
            .contains("dummy-api-key"));

        let text = fs::read_to_string(root.join("opencode.jsonc")).unwrap();
        assert!(text.contains("// keep this comment"));
        assert!(text.contains(r#""theme": "dark""#));
        assert!(text.contains(r#""mcp""#));
        let doc: Value =
            jsonc_parser::parse_to_serde_value(&text, &ParseOptions::default()).unwrap();
        assert_eq!(
            doc["provider"]["cyapi"]["api"],
            "https://example.invalid/v1"
        );
        assert_eq!(
            doc["provider"]["cyapi"]["models"]["default"]["id"],
            "example/model"
        );
        assert_eq!(doc["model"], "cyapi/default");
        assert_eq!(doc["theme"], "dark");

        let mut repeated = input(&saved);
        repeated.api_key.clear();
        let again = save(&root, &data, &repeated).unwrap();
        assert_eq!(again["apiKeyConfigured"], true);

        let restored = restore(&root, &data, again["revision"].as_str().unwrap()).unwrap();
        assert_eq!(fs::read(root.join("opencode.jsonc")).unwrap(), config);
        assert_eq!(restored["canRestore"], false);
        assert_eq!(restored["apiKeyConfigured"], false);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn creates_and_removes_config_and_blocks_stale_or_invalid_writes() {
        let (base, root, data) = fixture();
        let before = view(&root, &data).unwrap();
        assert_eq!(before["settingsExists"], false);

        for (url, key) in [
            ("https://user:dummy@example.invalid", "dummy-key"),
            ("file:///etc/passwd", "dummy-key"),
            ("https://example.invalid", "bad\nkey"),
        ] {
            let mut invalid = input(&before);
            invalid.base_url = url.into();
            invalid.api_key = key.into();
            let error = save(&root, &data, &invalid).unwrap_err();
            assert!(!error.contains("dummy"));
            assert!(!root.join("opencode.json").exists());
            assert!(!backup_path(&data).exists());
        }

        let mut missing = input(&before);
        missing.api_key.clear();
        assert!(save(&root, &data, &missing).is_err());

        let saved = save(&root, &data, &input(&before)).unwrap();
        assert_eq!(saved["settingsExists"], true);
        assert!(root.join("opencode.json").exists());
        assert!(save(&root, &data, &input(&before))
            .unwrap_err()
            .contains("变化"));

        fs::write(
            root.join("opencode.json"),
            br#"{"provider":{"cyapi":{"api":"https://external.invalid"}}}"#,
        )
        .unwrap();
        assert!(!view(&root, &data).unwrap()["canRestore"].as_bool().unwrap());
        assert!(restore(&root, &data, saved["revision"].as_str().unwrap()).is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_invalid_roots_and_preserves_the_file() {
        let (base, root, data) = fixture();
        let config = br#"{"provider":"keep-me"}"#;
        fs::write(root.join("opencode.json"), config).unwrap();
        let before = view(&root, &data).unwrap();
        let error = save(&root, &data, &input(&before)).unwrap_err();
        assert!(error.contains("provider"));
        assert_eq!(fs::read(root.join("opencode.json")).unwrap(), config);

        fs::write(root.join("opencode.json"), b"[]").unwrap();
        assert!(view(&root, &data).unwrap_err().contains("JSON 对象"));
        fs::write(root.join("opencode.json"), b"{oops").unwrap();
        assert!(!view(&root, &data).unwrap_err().contains("oops"));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn prefers_jsonc_and_leaves_lower_precedence_json_untouched() {
        let (base, root, data) = fixture();
        let lower = br#"{"theme":"from-json"}"#;
        let higher = br#"{"theme":"from-jsonc"} // higher precedence"#;
        fs::write(root.join("opencode.json"), lower).unwrap();
        fs::write(root.join("opencode.jsonc"), higher).unwrap();

        let before = view(&root, &data).unwrap();
        assert!(before["configPath"]
            .as_str()
            .unwrap()
            .ends_with("opencode.jsonc"));
        let saved = save(&root, &data, &input(&before)).unwrap();
        assert!(saved["configPath"]
            .as_str()
            .unwrap()
            .ends_with("opencode.jsonc"));
        assert_eq!(fs::read(root.join("opencode.json")).unwrap(), lower);
        let text = fs::read_to_string(root.join("opencode.jsonc")).unwrap();
        assert!(text.contains("higher precedence"));
        assert!(text.contains("from-jsonc"));
        assert!(text.contains("cyapi"));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn explicit_files_preserve_identity_backups_and_default_config() {
        let (base, root, data) = fixture();
        let default = b"{\"theme\":\"untouched\"}";
        fs::write(root.join("opencode.json"), default).unwrap();
        let file = base.join("custom/settings-without-extension");
        let location = Location::File(file.clone());
        let before = editor_view_at(&location, &data).unwrap();
        assert_eq!(before["configPath"], json!(file));
        assert_eq!(before["settingsExists"], false);
        let saved = save_at(&location, &data, &input(&before)).unwrap();
        assert_eq!(saved["model"], "example/model");
        assert_eq!(fs::read(root.join("opencode.json")).unwrap(), default);
        let other = Location::File(base.join("other/settings-without-extension"));
        assert!(!editor_view_at(&other, &data).unwrap()["canRestore"]
            .as_bool()
            .unwrap());
        assert!(save_at(&other, &data, &input(&saved)).is_err());
        let restored = restore_at(&location, &data, saved["revision"].as_str().unwrap()).unwrap();
        assert!(!file.exists());
        assert_eq!(restored["settingsExists"], false);

        let original = b"{ // custom comment\n\"theme\": \"keep\"\n}";
        fs::write(&file, original).unwrap();
        let before = editor_view_at(&location, &data).unwrap();
        let saved = save_at(&location, &data, &input(&before)).unwrap();
        assert!(fs::read_to_string(&file)
            .unwrap()
            .contains("// custom comment"));
        restore_at(&location, &data, saved["revision"].as_str().unwrap()).unwrap();
        assert_eq!(fs::read(&file).unwrap(), original);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn reads_legacy_base_url_and_model_keys() {
        let (base, root, data) = fixture();
        fs::write(
            root.join("opencode.json"),
            br#"{"model":"cyapi/read-model","provider":{"cyapi":{"options":{"baseURL":"https://legacy.invalid/v1","apiKey":"legacy-key"},"models":{"default":{"id":"read-model"}}}}}"#,
        )
        .unwrap();
        let current = view(&root, &data).unwrap();
        assert_eq!(current["baseUrl"], "https://legacy.invalid/v1");
        assert_eq!(current["model"], "read-model");
        assert_eq!(current["apiKeyConfigured"], true);
        assert!(current.get("apiKey").is_none());
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn multiple_models_switch_default_preserve_options_and_remove_deselected_models() {
        let (base, root, data) = fixture();
        let original = br#"{ // keep comment
          "model":"cyapi/default",
          "provider":{
            "other":{"name":"keep"},
            "cyapi":{"options":{"apiKey":"dummy-key"},"models":{
              "default":{"id":"model-a","name":"My A","options":{"reasoningEffort":"low","temperature":0.2},"variants":{"deep":{"reasoningEffort":"high"}}},
              "old":{"id":"remove-me"}
            }}
          },
          "mcp":{"keep":{}}
        }"#;
        fs::write(root.join("opencode.jsonc"), original).unwrap();
        let before = view(&root, &data).unwrap();
        assert_eq!(before["model"], "model-a");
        assert_eq!(before["reasoningEffort"], "low");
        let mut change = input(&before);
        change.model = "vendor/model-b".into();
        change.models = Some(vec![
            "model-a".into(),
            "vendor/model-b".into(),
            "model-a".into(),
        ]);
        change.reasoning_effort = "xhigh".into();
        let saved = save(&root, &data, &change).unwrap();
        assert_eq!(saved["model"], "vendor/model-b");
        assert_eq!(saved["reasoningEffort"], "xhigh");
        let text = fs::read_to_string(root.join("opencode.jsonc")).unwrap();
        assert!(text.contains("// keep comment"));
        let doc: Value =
            jsonc_parser::parse_to_serde_value(&text, &ParseOptions::default()).unwrap();
        let entries = &doc["provider"]["cyapi"]["models"];
        assert_eq!(entries.as_object().unwrap().len(), 2);
        assert!(entries.get("old").is_none());
        assert_eq!(entries["default"]["name"], "My A");
        assert_eq!(entries["default"]["options"]["temperature"], 0.2);
        assert_eq!(entries["default"]["options"]["reasoningEffort"], "xhigh");
        assert_eq!(
            entries["default"]["variants"]["deep"]["reasoningEffort"],
            "high"
        );
        assert_eq!(doc["model"], "cyapi/vendor/model-b");
        assert_eq!(doc["provider"]["other"]["name"], "keep");
        let mut next = input(&saved);
        next.model = "model-a".into();
        next.models = Some(vec!["model-a".into(), "vendor/model-b".into()]);
        next.reasoning_effort.clear();
        let switched = save(&root, &data, &next).unwrap();
        assert_eq!(switched["model"], "model-a");
        assert_eq!(switched["reasoningEffort"], "");
        let doc: Value = jsonc_parser::parse_to_serde_value(
            &fs::read_to_string(root.join("opencode.jsonc")).unwrap(),
            &ParseOptions::default(),
        )
        .unwrap();
        assert_eq!(doc["model"], "cyapi/default");
        assert!(doc["provider"]["cyapi"]["models"]["default"]["options"]
            .get("reasoningEffort")
            .is_none());
        // Restore returns to the complete previous selection and its effort.
        let restored = restore(&root, &data, switched["revision"].as_str().unwrap()).unwrap();
        assert_eq!(restored["models"], saved["models"]);
        assert_eq!(restored["reasoningEffort"], "xhigh");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn invalid_model_lists_and_efforts_do_not_create_files_or_backups() {
        let (base, root, data) = fixture();
        let before = view(&root, &data).unwrap();
        for (models, effort) in [
            (vec!["bad\nmodel".into()], "high"),
            (vec!["valid".into()], "invalid"),
        ] {
            let mut invalid = input(&before);
            invalid.models = Some(models);
            invalid.reasoning_effort = effort.into();
            assert!(save(&root, &data, &invalid).is_err());
            assert!(!root.join("opencode.json").exists());
            assert!(!backup_path(&data).exists());
        }
        fs::remove_dir_all(base).unwrap();
    }
}
