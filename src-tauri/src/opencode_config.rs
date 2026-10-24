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
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, String>;

const PROVIDER: &str = "cyapi";
const PROVIDER_NPM: &str = "@ai-sdk/openai-compatible";
const PROVIDER_PACKAGE: &str = "@opencode/ai/providers/openai-compatible";
const MODEL_ALIAS: &str = "default";
// JSON numbers must round-trip exactly through the JavaScript editor. This is
// an input transport bound, not a claim about any model's context capability.
const MAX_CONTEXT: u64 = 9_007_199_254_740_991;

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
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub model_effort_changes: BTreeMap<String, String>,
    #[serde(default)]
    pub model_context_changes: BTreeMap<String, Option<u64>>,
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

#[derive(Clone, Copy, PartialEq)]
enum Layout {
    Native,
    Legacy,
}

impl Layout {
    fn of(doc: &Value) -> Self {
        // Never edit the lower-priority legacy provider when native cyapi exists.
        if doc
            .get("providers")
            .and_then(|value| value.get(PROVIDER))
            .is_some()
        {
            Self::Native
        } else if doc
            .get("provider")
            .and_then(|value| value.get(PROVIDER))
            .is_some()
        {
            Self::Legacy
        } else {
            Self::Native
        }
    }

    fn providers(self) -> &'static str {
        match self {
            Self::Native => "providers",
            Self::Legacy => "provider",
        }
    }

    fn settings(self) -> &'static str {
        match self {
            Self::Native => "settings",
            Self::Legacy => "options",
        }
    }

    fn model_id(self) -> &'static str {
        match self {
            Self::Native => "modelID",
            Self::Legacy => "id",
        }
    }
}

fn provider_of(doc: &Value) -> Option<&Value> {
    doc.get(Layout::of(doc).providers())?.get(PROVIDER)
}

fn api_key(doc: &Value) -> Option<&str> {
    provider_of(doc).and_then(|provider| text_at(provider, &[Layout::of(doc).settings(), "apiKey"]))
}

fn base_url(doc: &Value) -> Option<&str> {
    let provider = provider_of(doc)?;
    if Layout::of(doc) == Layout::Native {
        text_at(provider, &["settings", "baseURL"])
    } else {
        text_at(provider, &["api"])
            .or_else(|| text_at(provider, &["options", "baseURL"]))
            .or_else(|| text_at(provider, &["options", "api"]))
    }
}

fn model_entries(doc: &Value) -> Option<&serde_json::Map<String, Value>> {
    provider_of(doc)?.get("models")?.as_object()
}

fn model_id<'a>(layout: Layout, alias: &'a str, entry: &'a Value) -> &'a str {
    text_at(entry, &[layout.model_id()]).unwrap_or(alias)
}

fn model_alias(doc: &Value) -> Option<&str> {
    let reference = doc.get("model")?;
    let alias = if reference.is_object() {
        if text_at(reference, &["providerID"])? != PROVIDER {
            return None;
        }
        text_at(reference, &["model"])?
    } else {
        text_at(doc, &["model"])?.strip_prefix("cyapi/")?
    };
    alias.split('#').next()
}

fn model(doc: &Value) -> Option<&str> {
    if let Some(alias) = model_alias(doc) {
        return Some(
            model_entries(doc)
                .and_then(|entries| entries.get(alias))
                .map(|entry| model_id(Layout::of(doc), alias, entry))
                .unwrap_or(alias),
        );
    }
    model_entries(doc)?
        .get(MODEL_ALIAS)
        .map(|entry| model_id(Layout::of(doc), MODEL_ALIAS, entry))
}

fn selected_models(doc: &Value) -> Vec<String> {
    model_entries(doc)
        .map(|entries| {
            entries
                .iter()
                .map(|(alias, entry)| model_id(Layout::of(doc), alias, entry).to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn entry_effort<'a>(doc: &Value, entry: &'a Value) -> &'a str {
    entry
        .get(Layout::of(doc).settings())
        .and_then(|settings| settings.get("reasoningEffort"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn model_efforts(doc: &Value) -> (BTreeMap<String, String>, Vec<String>) {
    let mut efforts: BTreeMap<String, Option<String>> = BTreeMap::new();
    for (alias, entry) in model_entries(doc).into_iter().flatten() {
        let id = model_id(Layout::of(doc), alias, entry).to_string();
        let effort = entry_effort(doc, entry);
        efforts
            .entry(id)
            .and_modify(|current| {
                if current.as_deref() != Some(effort) {
                    *current = None;
                }
            })
            .or_insert_with(|| Some(effort.to_string()));
    }
    // This API is keyed by upstream ID, not alias: conflicting alias values
    // have no single explicit level. Return a separate conflict marker so an
    // empty display value cannot be mistaken for no explicit override. Missing
    // alias levels count as empty. Only a submitted patch changes alias values,
    // and an explicit API-ID edit intentionally affects all of its aliases.
    let conflicts = efforts
        .iter()
        .filter(|(_, effort)| effort.is_none())
        .map(|(id, _)| id.clone())
        .collect();
    let efforts = efforts
        .into_iter()
        .map(|(id, effort)| (id, effort.unwrap_or_default()))
        .collect();
    (efforts, conflicts)
}

fn model_contexts(doc: &Value) -> (BTreeMap<String, Value>, Vec<String>) {
    let mut contexts: BTreeMap<String, Option<Value>> = BTreeMap::new();
    for (alias, entry) in model_entries(doc).into_iter().flatten() {
        let id = model_id(Layout::of(doc), alias, entry).to_string();
        // Preserve arbitrary existing explicit values in the view. Missing and
        // explicit null are equivalent; do not infer limits from model metadata.
        let context = entry
            .get("limit")
            .and_then(|limit| limit.get("context"))
            .unwrap_or(&Value::Null);
        contexts
            .entry(id)
            .and_modify(|current| {
                if current.as_ref() != Some(context) {
                    *current = None;
                }
            })
            .or_insert_with(|| Some(context.clone()));
    }
    let conflicts = contexts
        .iter()
        .filter(|(_, context)| context.is_none())
        .map(|(id, _)| id.clone())
        .collect();
    let contexts = contexts
        .into_iter()
        .map(|(id, context)| (id, context.unwrap_or(Value::Null)))
        .collect();
    (contexts, conflicts)
}

fn reasoning_effort(doc: &Value) -> &str {
    let alias = model_alias(doc).unwrap_or(MODEL_ALIAS);
    model_entries(doc)
        .and_then(|entries| entries.get(alias))
        .map(|entry| entry_effort(doc, entry))
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
    let (efforts, effort_conflicts) = model_efforts(&doc);
    let (contexts, context_conflicts) = model_contexts(&doc);
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
        "modelEfforts": efforts,
        "modelEffortConflicts": effort_conflicts,
        "modelContexts": contexts,
        "modelContextConflicts": context_conflicts,
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
    // Levels are provider-specific. Validate only explicit edits, never existing
    // values, and do not constrain them to the toolbox's suggested UI choices.
    for effort in input
        .model_effort_changes
        .values()
        .chain(input.reasoning_effort.iter())
    {
        if effort.len() > 64 || effort.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err("思考等级必须是不超过 64 字节且不含空白或控制字符的字符串".into());
        }
    }
    if input
        .model_context_changes
        .values()
        .flatten()
        .any(|context| !(1..=MAX_CONTEXT).contains(context))
    {
        return Err("上下文窗口必须是 1 到 9007199254740991 之间的整数".into());
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
    let selected = crate::model_selection::models(
        model,
        input.models.as_deref().unwrap_or(&selected_models(&doc)),
    )?;
    if input
        .model_effort_changes
        .keys()
        .any(|id| !selected.contains(id))
    {
        return Err("思考等级修改只能包含已选择的模型 ID".into());
    }
    if input
        .model_context_changes
        .keys()
        .any(|id| !selected.contains(id))
    {
        return Err("上下文窗口修改只能包含已选择的模型 ID".into());
    }
    // An explicit per-model patch wins over the compatibility field. The old
    // field changes only the requested default model; it must never fan out.
    let mut effort_changes = input.model_effort_changes.clone();
    if let Some(effort) = &input.reasoning_effort {
        effort_changes
            .entry(model.to_string())
            .or_insert_with(|| effort.clone());
    }
    let layout = Layout::of(&doc);

    let root_node = parse_cst(&current)?;
    let object = root_node
        .object_value_or_create()
        .ok_or("OpenCode 配置文件必须是 JSON 对象")?;
    let provider = object
        .object_value_or_create(layout.providers())
        .ok_or("OpenCode 配置的 providers/provider 必须是 JSON 对象")?;
    let cyapi = provider
        .object_value_or_create(PROVIDER)
        .ok_or("OpenCode 配置的 cyapi provider 必须是 JSON 对象")?;

    set_string(&cyapi, "name", "CYAPI");
    let settings = cyapi
        .object_value_or_create(layout.settings())
        .ok_or("OpenCode provider 的 settings/options 必须是 JSON 对象")?;
    set_string(&settings, "apiKey", &key);
    match layout {
        Layout::Native => {
            // Runtime settings are package-specific. Never switch an existing
            // native/responses/AI SDK runtime while editing its endpoint.
            if cyapi.get("package").is_none() {
                set_string(&cyapi, "package", PROVIDER_PACKAGE);
            }
            set_string(&settings, "baseURL", &base_url);
        }
        Layout::Legacy => {
            set_string(&cyapi, "npm", PROVIDER_NPM);
            set_string(&cyapi, "api", &base_url);
            // Preserve existing legacy endpoint conventions without migration.
            update_if_present(&settings, "baseURL", &base_url);
            update_if_present(&settings, "api", &base_url);
        }
    }

    let models = cyapi
        .object_value_or_create("models")
        .ok_or("OpenCode provider 的 models 必须是 JSON 对象")?;
    let existing = model_entries(&doc);
    let mut aliases = Vec::new();
    let mut default_alias = String::new();
    for id in selected {
        // Several aliases can refer to one API model. Keep all selected aliases
        // (including their variants/options) and apply an API-ID patch to each.
        let mut model_aliases: Vec<String> = existing
            .into_iter()
            .flatten()
            .filter(|(alias, entry)| model_id(layout, alias, entry) == id)
            .map(|(alias, _)| alias.clone())
            .collect();
        if model_aliases.is_empty() {
            let alias = if layout == Layout::Legacy
                && id == model
                && existing.is_none_or(|entries| !entries.contains_key(MODEL_ALIAS))
                && !aliases.iter().any(|alias| alias == MODEL_ALIAS)
            {
                MODEL_ALIAS.to_string()
            } else if existing.is_some_and(|entries| entries.contains_key(&id))
                || aliases.contains(&id)
                || id.contains('#')
            {
                format!("model-{}", crate::service::hash(id.as_bytes()))
            } else {
                id.clone()
            };
            model_aliases.push(alias);
        }
        if id == model {
            default_alias = model_alias(&doc)
                .filter(|alias| model_aliases.iter().any(|value| value.as_str() == *alias))
                .unwrap_or(&model_aliases[0])
                .to_string();
        }
        for alias in model_aliases {
            let entry = models
                .object_value_or_create(&alias)
                .ok_or("OpenCode 模型配置必须是 JSON 对象")?;
            let previous = existing.and_then(|entries| entries.get(&alias));
            // Leave existing entries untouched unless an explicit patch was submitted.
            if previous.is_none() {
                set_string(&entry, layout.model_id(), &id);
                set_string(&entry, "name", &format!("CYAPI · {id}"));
            }
            if let Some(effort) = effort_changes.get(&id) {
                let has_effort = previous
                    .and_then(|entry| entry.get(layout.settings()))
                    .and_then(|settings| settings.get("reasoningEffort"))
                    .is_some();
                // Clearing a missing level must not create empty settings/options.
                if !effort.is_empty() || has_effort {
                    let settings = entry
                        .object_value_or_create(layout.settings())
                        .ok_or("OpenCode 模型 settings/options 必须是 JSON 对象")?;
                    if effort.is_empty() {
                        if let Some(prop) = settings.get("reasoningEffort") {
                            prop.remove();
                        }
                    } else {
                        set_string(&settings, "reasoningEffort", effort);
                    }
                }
            }
            if let Some(context) = input.model_context_changes.get(&id) {
                let previous_limit = previous.and_then(|entry| entry.get("limit"));
                // An explicit edit must not silently replace an unknown limit
                // shape, even when the submitted edit only clears context.
                if previous_limit.is_some_and(|limit| !limit.is_object()) {
                    return Err("OpenCode 模型 limit 必须是 JSON 对象，无法修改上下文窗口".into());
                }
                let has_context = previous_limit
                    .and_then(|limit| limit.get("context"))
                    .is_some();
                // Clearing a missing override must not create a limit object.
                if context.is_some() || has_context {
                    let limit = entry
                        .object_value_or_create("limit")
                        .ok_or("OpenCode 模型 limit 必须是 JSON 对象")?;
                    match context {
                        Some(context) => {
                            let value = CstInputValue::Number(context.to_string());
                            match limit.get("context") {
                                Some(prop) => prop.set_value(value),
                                None => {
                                    limit.append("context", value);
                                }
                            }
                        }
                        None => {
                            if let Some(prop) = limit.get("context") {
                                prop.remove();
                            }
                        }
                    }
                }
            }
            aliases.push(alias);
        }
    }
    if let Some(existing) = existing {
        for alias in existing.keys().filter(|alias| !aliases.contains(alias)) {
            if let Some(prop) = models.get(alias) {
                prop.remove();
            }
        }
    }
    // Preserve an existing reference verbatim when its alias is still default,
    // including object shape, additional fields, and string variant suffixes.
    if model_alias(&doc) != Some(default_alias.as_str()) {
        if doc.get("model").is_some_and(Value::is_object) {
            let reference = object
                .object_value_or_create("model")
                .ok_or("OpenCode 默认模型必须是 JSON 对象")?;
            set_string(&reference, "providerID", PROVIDER);
            set_string(&reference, "model", &default_alias);
        } else {
            set_string(&object, "model", &format!("{PROVIDER}/{default_alias}"));
        }
    }

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
            reasoning_effort: None,
            model_effort_changes: BTreeMap::new(),
            model_context_changes: BTreeMap::new(),
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
            doc["providers"]["cyapi"]["settings"]["baseURL"],
            "https://example.invalid/v1"
        );
        assert_eq!(
            doc["providers"]["cyapi"]["models"]["example/model"]["modelID"],
            "example/model"
        );
        assert_eq!(doc["model"], "cyapi/example/model");
        assert_eq!(doc["theme"], "dark");
        assert_eq!(
            doc["providers"][PROVIDER]["package"],
            "@opencode/ai/providers/openai-compatible"
        );
        assert!(doc.get("provider").is_none());
        assert!(doc["providers"][PROVIDER].get("options").is_none());
        assert!(doc["providers"][PROVIDER]["models"]["example/model"]
            .get("settings")
            .is_none());

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
        let config = br#"{"providers":"keep-me"}"#;
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
        change.reasoning_effort = Some("xhigh".into());
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
        assert_eq!(entries["default"]["options"]["reasoningEffort"], "low");
        assert_eq!(
            entries["vendor/model-b"]["options"]["reasoningEffort"],
            "xhigh"
        );
        assert!(entries["vendor/model-b"].get("reasoning").is_none());
        assert!(doc.get("providers").is_none());
        assert_eq!(
            entries["default"]["variants"]["deep"]["reasoningEffort"],
            "high"
        );
        assert_eq!(doc["model"], "cyapi/vendor/model-b");
        assert_eq!(doc["provider"]["other"]["name"], "keep");
        let mut next = input(&saved);
        next.model = "model-a".into();
        next.models = Some(vec!["model-a".into(), "vendor/model-b".into()]);
        next.reasoning_effort = Some(String::new());
        let switched = save(&root, &data, &next).unwrap();
        assert_eq!(switched["model"], "model-a");
        assert_eq!(switched["reasoningEffort"], "");
        let doc: Value = jsonc_parser::parse_to_serde_value(
            &fs::read_to_string(root.join("opencode.jsonc")).unwrap(),
            &ParseOptions::default(),
        )
        .unwrap();
        assert_eq!(doc["model"], "cyapi/default");
        assert_eq!(
            doc["provider"]["cyapi"]["models"]["vendor/model-b"]["options"]["reasoningEffort"],
            "xhigh"
        );
        assert!(doc["provider"]["cyapi"]["models"]["default"]["options"]
            .get("reasoningEffort")
            .is_none());
        // Restore returns to the complete previous selection and its effort.
        let restored = restore(&root, &data, switched["revision"].as_str().unwrap()).unwrap();
        assert_eq!(restored["models"], saved["models"]);
        assert_eq!(restored["reasoningEffort"], "xhigh");
        fs::remove_dir_all(base).unwrap();
    }

    fn read_doc(root: &Path) -> Value {
        parse_value(&snapshot(&Location::Directory(root.into())).unwrap()).unwrap()
    }

    #[test]
    fn native_and_legacy_effort_patches_preserve_unedited_models_and_restore() {
        for layout in [Layout::Native, Layout::Legacy] {
            let (base, root, data) = fixture();
            let mut doc = json!({
                "model": "cyapi/alias-a",
                "mcp": {"servers": {"keep": {"type": "local", "command": ["demo"]}}}
            });
            doc[layout.providers()] = json!({
                "other": {"name": "untouched"},
                "cyapi": {
                    "models": {
                        "alias-a": {"name": "My A", "variants": [{"id": "deep", "body": {"custom": true}}]},
                        "alias-b": {"name": "My B"},
                        "alias-c": {"name": "My C"},
                        "alias-d": {"name": "My D"}
                    }
                }
            });
            let provider = &mut doc[layout.providers()][PROVIDER];
            provider[layout.settings()] =
                json!({"apiKey": "dummy", "baseURL": "https://old.invalid", "custom": true});
            provider["models"]["alias-a"][layout.model_id()] = json!("model-a");
            provider["models"]["alias-b"][layout.model_id()] = json!("model-b");
            provider["models"]["alias-c"][layout.model_id()] = json!("model-c");
            provider["models"]["alias-d"][layout.model_id()] = json!("model-d");
            provider["models"]["alias-a"][layout.settings()] =
                json!({"reasoningEffort": "low", "temperature": 0.2});
            provider["models"]["alias-b"][layout.settings()] = json!({"reasoningEffort": "high"});
            // Even unsafe/unknown OLD values are not ours to validate or rewrite.
            let unknown = format!("future custom\n{}", "x".repeat(80));
            provider["models"]["alias-c"][layout.settings()] = json!({"reasoningEffort": unknown});
            let original = format!(
                "// preserve this comment\n{}\n",
                serde_json::to_string_pretty(&doc).unwrap()
            );
            fs::write(root.join("opencode.jsonc"), &original).unwrap();
            let before = editor_view(&root, &data).unwrap();
            assert_eq!(before["modelEfforts"]["model-a"], "low");
            assert_eq!(before["modelEfforts"]["model-b"], "high");
            assert_eq!(before["modelEfforts"]["model-c"], unknown);
            assert_eq!(before["modelEfforts"]["model-d"], "");
            let original_entries = doc[layout.providers()][PROVIDER]["models"].clone();

            let mut change = input(&before);
            change.model = "model-a".into();
            change.api_key.clear();
            change.base_url = "https://new.invalid/v1".into();
            let url_saved = save(&root, &data, &change).unwrap();
            assert_eq!(
                read_doc(&root)[layout.providers()][PROVIDER]["models"],
                original_entries
            );
            assert_eq!(url_saved["apiKey"], "dummy");
            assert_eq!(url_saved["baseUrl"], "https://new.invalid/v1");

            change.revision = url_saved["revision"].as_str().unwrap().into();
            change.model = "model-b".into();
            let switched = save(&root, &data, &change).unwrap();
            assert_eq!(switched["reasoningEffort"], "high");
            assert_eq!(
                read_doc(&root)[layout.providers()][PROVIDER]["models"],
                original_entries
            );
            let before_patch = fs::read(root.join("opencode.jsonc")).unwrap();

            change.revision = switched["revision"].as_str().unwrap().into();
            change
                .model_effort_changes
                .insert("model-a".into(), "vendor-custom".into());
            let patched = save(&root, &data, &change).unwrap();
            assert_eq!(patched["modelEfforts"]["model-a"], "vendor-custom");
            assert_eq!(patched["modelEfforts"]["model-b"], "high");
            assert_eq!(patched["modelEfforts"]["model-c"], unknown);
            let patched_doc = read_doc(&root);
            let entries = &patched_doc[layout.providers()][PROVIDER]["models"];
            assert_eq!(entries["alias-b"], original_entries["alias-b"]);
            assert_eq!(entries["alias-c"], original_entries["alias-c"]);
            assert_eq!(entries["alias-d"], original_entries["alias-d"]);
            assert_eq!(
                entries["alias-a"]["variants"],
                original_entries["alias-a"]["variants"]
            );
            assert_eq!(entries["alias-a"][layout.settings()]["temperature"], 0.2);
            assert!(entries["alias-a"].get("reasoning").is_none());
            assert!(entries["alias-a"].get("capabilities").is_none());
            let restored = restore(&root, &data, patched["revision"].as_str().unwrap()).unwrap();
            assert_eq!(fs::read(root.join("opencode.jsonc")).unwrap(), before_patch);
            assert_eq!(restored["modelEfforts"], switched["modelEfforts"]);

            change.revision = restored["revision"].as_str().unwrap().into();
            change
                .model_effort_changes
                .insert("model-a".into(), String::new());
            change
                .model_effort_changes
                .insert("model-d".into(), String::new());
            let cleared = save(&root, &data, &change).unwrap();
            assert_eq!(cleared["modelEfforts"]["model-a"], "");
            assert_eq!(cleared["modelEfforts"]["model-b"], "high");
            let cleared_doc = read_doc(&root);
            let entries = &cleared_doc[layout.providers()][PROVIDER]["models"];
            assert!(entries["alias-a"][layout.settings()]
                .get("reasoningEffort")
                .is_none());
            assert_eq!(entries["alias-a"][layout.settings()]["temperature"], 0.2);
            assert!(entries["alias-d"].get(layout.settings()).is_none());
            assert_eq!(entries["alias-b"], original_entries["alias-b"]);
            assert_eq!(entries["alias-c"], original_entries["alias-c"]);

            change.revision = cleared["revision"].as_str().unwrap().into();
            change.models = Some(vec!["model-b".into(), "model-c".into()]);
            change.model_effort_changes.clear();
            let deselected = save(&root, &data, &change).unwrap();
            assert_eq!(
                deselected["modelEfforts"],
                json!({"model-b": "high", "model-c": unknown})
            );
            let final_doc = read_doc(&root);
            assert_eq!(
                final_doc[layout.providers()][PROVIDER]["models"]
                    .as_object()
                    .unwrap()
                    .len(),
                2
            );
            assert_eq!(
                final_doc[layout.providers()]["other"],
                doc[layout.providers()]["other"]
            );
            assert_eq!(final_doc["mcp"], doc["mcp"]);
            assert_eq!(
                final_doc[layout.providers()][PROVIDER][layout.settings()]["custom"],
                true
            );
            assert!(fs::read_to_string(root.join("opencode.jsonc"))
                .unwrap()
                .contains("// preserve this comment"));
            fs::remove_dir_all(base).unwrap();
        }
    }

    #[test]
    fn native_provider_wins_over_legacy_without_editing_lower_priority_entries() {
        let (base, root, data) = fixture();
        let doc = json!({
            "model": "cyapi/custom-alias",
            "providers": {"cyapi": {
                "package": "aisdk:@ai-sdk/openai-compatible",
                "settings": {"baseURL": "https://native.invalid", "apiKey": "native-key"},
                "models": {"custom-alias": {"modelID": "api-id", "settings": {"reasoningEffort": "native-level"}}}
            }, "native-other": {"settings": {"custom": true}}},
            "provider": {"cyapi": {
                "api": "https://legacy.invalid", "options": {"apiKey": "legacy-key"},
                "models": {"custom-alias": {"id": "legacy-id", "options": {"reasoningEffort": "legacy-level"}}}
            }}
        });
        fs::write(
            root.join("opencode.json"),
            serde_json::to_vec(&doc).unwrap(),
        )
        .unwrap();
        let before = editor_view(&root, &data).unwrap();
        assert_eq!(before["model"], "api-id");
        assert_eq!(before["apiKey"], "native-key");
        assert_eq!(before["baseUrl"], "https://native.invalid");
        assert_eq!(before["reasoningEffort"], "native-level");
        let mut change = input(&before);
        change.model = "api-id".into();
        change.api_key.clear();
        change
            .model_effort_changes
            .insert("api-id".into(), "custom".into());
        save(&root, &data, &change).unwrap();
        let saved = read_doc(&root);
        assert_eq!(saved["provider"], doc["provider"]);
        assert_eq!(
            saved["providers"]["native-other"],
            doc["providers"]["native-other"]
        );
        assert_eq!(
            saved["providers"][PROVIDER]["models"]["custom-alias"]["settings"]["reasoningEffort"],
            "custom"
        );
        assert_eq!(saved["model"], "cyapi/custom-alias");
        assert!(saved["providers"][PROVIDER].get("options").is_none());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn selected_aliases_are_preserved_and_patched_by_api_id() {
        for layout in [Layout::Native, Layout::Legacy] {
            let (base, root, data) = fixture();
            let mut doc = json!({"model": "cyapi/second"});
            doc[layout.providers()] = json!({"cyapi": {"models": {
                "first": {"name": "First", "variants": {"custom": {"keep": true}}},
                "second": {"name": "Second", "custom": true}
            }}});
            let provider = &mut doc[layout.providers()][PROVIDER];
            provider[layout.settings()] = json!({"apiKey": "dummy"});
            for alias in ["first", "second"] {
                provider["models"][alias][layout.model_id()] = json!("api-id");
                provider["models"][alias][layout.settings()] = json!({"reasoningEffort": "low"});
            }
            provider["models"]["second"][layout.settings()]["reasoningEffort"] = json!("high");
            fs::write(
                root.join("opencode.json"),
                serde_json::to_vec(&doc).unwrap(),
            )
            .unwrap();
            let before = view(&root, &data).unwrap();
            let mut change = input(&before);
            change.model = "api-id".into();
            change.models = Some(vec!["api-id".into()]);
            assert_eq!(before["modelEfforts"]["api-id"], "");
            assert_eq!(before["modelEffortConflicts"], json!(["api-id"]));
            assert_eq!(before["reasoningEffort"], "high");
            let unchanged = save(&root, &data, &change).unwrap();
            assert_eq!(unchanged["modelEfforts"]["api-id"], "");
            assert_eq!(unchanged["modelEffortConflicts"], json!(["api-id"]));
            assert_eq!(unchanged["reasoningEffort"], "high");
            let saved_doc = read_doc(&root);
            assert_eq!(saved_doc["model"], "cyapi/second");
            assert_eq!(
                saved_doc[layout.providers()][PROVIDER]["models"],
                doc[layout.providers()][PROVIDER]["models"]
            );
            change.revision = unchanged["revision"].as_str().unwrap().into();
            change
                .model_effort_changes
                .insert("api-id".into(), "custom".into());
            let patched_view = save(&root, &data, &change).unwrap();
            assert_eq!(patched_view["modelEfforts"]["api-id"], "custom");
            assert_eq!(patched_view["modelEffortConflicts"], json!([]));
            let patched = read_doc(&root);
            for alias in ["first", "second"] {
                assert_eq!(
                    patched[layout.providers()][PROVIDER]["models"][alias][layout.settings()]
                        ["reasoningEffort"],
                    "custom"
                );
            }
            assert_eq!(
                patched[layout.providers()][PROVIDER]["models"]["first"]["variants"],
                doc[layout.providers()][PROVIDER]["models"]["first"]["variants"]
            );
            assert_eq!(
                patched[layout.providers()][PROVIDER]["models"]["second"]["custom"],
                true
            );
            fs::remove_dir_all(base).unwrap();
        }
    }

    #[test]
    fn effort_conflicts_include_missing_alias_levels_but_not_matching_levels() {
        for layout in [Layout::Native, Layout::Legacy] {
            let mut doc = json!({});
            doc[layout.providers()] = json!({"cyapi": {"models": {}}});
            for (alias, id, effort) in [
                ("a", "missing-versus-high", None),
                ("b", "missing-versus-high", Some("high")),
                ("c", "matching", Some("low")),
                ("d", "matching", Some("low")),
                ("e", "empty", None),
                ("f", "empty", Some("")),
                ("g", "different", Some("low")),
                ("h", "different", Some("high")),
                // A later alias matching the first must not reset a conflict.
                ("i", "different", Some("low")),
            ] {
                let mut entry = json!({});
                entry[layout.model_id()] = json!(id);
                if let Some(effort) = effort {
                    entry[layout.settings()] = json!({"reasoningEffort": effort});
                }
                doc[layout.providers()][PROVIDER]["models"][alias] = entry;
            }
            let (efforts, conflicts) = model_efforts(&doc);
            assert_eq!(conflicts, vec!["different", "missing-versus-high"]);
            assert_eq!(efforts["different"], "");
            assert_eq!(efforts["missing-versus-high"], "");
            assert_eq!(efforts["matching"], "low");
            assert_eq!(efforts["empty"], "");
        }
    }

    #[test]
    fn native_package_is_only_supplied_when_absent() {
        for package in [
            None,
            Some("@opencode/ai/providers/openai-compatible"),
            Some("@opencode/ai/providers/openai/responses"),
            Some("@opencode/ai/providers/openai-compatible/responses"),
            Some("aisdk:@ai-sdk/openai-compatible"),
            Some("@custom/provider"),
        ] {
            let (base, root, data) = fixture();
            let mut doc = json!({
                "model": "cyapi/alias",
                "providers": {"cyapi": {
                    "settings": {"apiKey": "dummy", "baseURL": "https://old.invalid", "custom": true},
                    "models": {"alias": {"modelID": "api-id", "settings": {"reasoningEffort": "custom"}}}
                }}
            });
            if let Some(package) = package {
                doc["providers"][PROVIDER]["package"] = json!(package);
            }
            fs::write(
                root.join("opencode.json"),
                serde_json::to_vec(&doc).unwrap(),
            )
            .unwrap();
            let before = editor_view(&root, &data).unwrap();
            let mut change = input(&before);
            change.model = "api-id".into();
            let saved = save(&root, &data, &change).unwrap();
            let actual = read_doc(&root);
            assert_eq!(
                actual["providers"][PROVIDER]["package"],
                package.unwrap_or(PROVIDER_PACKAGE)
            );
            assert_eq!(
                actual["providers"][PROVIDER]["models"],
                doc["providers"][PROVIDER]["models"]
            );
            assert_eq!(actual["providers"][PROVIDER]["settings"]["custom"], true);
            assert_eq!(saved["baseUrl"], "https://example.invalid/v1");
            fs::remove_dir_all(base).unwrap();
        }
    }

    #[test]
    fn native_object_and_string_model_references_preserve_default_alias() {
        for reference in [
            json!({"providerID": "cyapi", "model": "second"}),
            json!("cyapi/second"),
            json!("cyapi/second#deep"),
        ] {
            let (base, root, data) = fixture();
            let doc = json!({
                "model": reference,
                "providers": {"cyapi": {
                    "package": "@opencode/ai/providers/openai/responses",
                    "settings": {"apiKey": "dummy"},
                    "models": {
                        "first": {"modelID": "api-id", "settings": {"reasoningEffort": "low"}},
                        "second": {"modelID": "api-id", "settings": {"reasoningEffort": "high"}},
                        "other": {"modelID": "other-id", "settings": {"reasoningEffort": "custom"}}
                    }
                }}
            });
            fs::write(
                root.join("opencode.json"),
                serde_json::to_vec(&doc).unwrap(),
            )
            .unwrap();
            let before = view(&root, &data).unwrap();
            assert_eq!(before["model"], "api-id");
            assert_eq!(before["reasoningEffort"], "high");
            assert_eq!(before["modelEfforts"]["api-id"], "");
            assert_eq!(before["modelEffortConflicts"], json!(["api-id"]));
            let mut change = input(&before);
            change.model = before["model"].as_str().unwrap().into();
            let saved = save(&root, &data, &change).unwrap();
            let unchanged = read_doc(&root);
            assert_eq!(unchanged["model"], reference);
            assert_eq!(
                unchanged["providers"][PROVIDER]["models"],
                doc["providers"][PROVIDER]["models"]
            );
            change.revision = saved["revision"].as_str().unwrap().into();
            change.model = "other-id".into();
            let switched = save(&root, &data, &change).unwrap();
            assert_eq!(switched["model"], "other-id");
            assert_eq!(switched["reasoningEffort"], "custom");
            let actual = read_doc(&root);
            if reference.is_object() {
                assert_eq!(
                    actual["model"],
                    json!({"providerID": "cyapi", "model": "other"})
                );
            } else {
                assert_eq!(actual["model"], "cyapi/other");
            }
            assert_eq!(
                actual["providers"][PROVIDER]["models"],
                doc["providers"][PROVIDER]["models"]
            );
            fs::remove_dir_all(base).unwrap();
        }
    }

    #[test]
    fn effort_input_is_optional_and_invalid_patches_never_write() {
        let value = json!({
            "baseUrl": "https://example.invalid/v1", "apiKey": "dummy",
            "model": "model-a", "revision": "unused"
        });
        let decoded: Input = serde_json::from_value(value.clone()).unwrap();
        assert!(decoded.reasoning_effort.is_none());
        assert!(decoded.model_effort_changes.is_empty());
        let mut explicit = value;
        explicit["reasoningEffort"] = json!("");
        explicit["modelEffortChanges"] = json!({"model-a": "custom"});
        let decoded: Input = serde_json::from_value(explicit).unwrap();
        assert_eq!(decoded.reasoning_effort, Some(String::new()));
        assert_eq!(decoded.model_effort_changes["model-a"], "custom");

        let (base, root, data) = fixture();
        let before = view(&root, &data).unwrap();
        let mut valid = input(&before);
        valid.model = "model-a".into();
        valid.models = Some(vec!["model-a".into(), "model-b".into()]);
        valid.reasoning_effort = Some("legacy-default".into());
        valid
            .model_effort_changes
            .insert("model-a".into(), "patch-wins".into());
        let saved = save(&root, &data, &valid).unwrap();
        assert_eq!(
            saved["modelEfforts"],
            json!({"model-a": "patch-wins", "model-b": ""})
        );
        let bytes = fs::read(root.join("opencode.json")).unwrap();
        let backup = fs::read(backup_path(&data)).unwrap();
        for (id, effort) in [
            ("model-a", "bad effort".to_string()),
            ("model-a", "bad\nlevel".into()),
            ("model-a", "bad\u{00a0}level".into()),
            ("model-a", "x".repeat(65)),
            ("unselected", "custom".into()),
            ("unselected", String::new()),
            (" model-a", "low".into()),
        ] {
            let mut invalid = input(&saved);
            invalid.model = "model-a".into();
            invalid.model_effort_changes.insert(id.into(), effort);
            assert!(save(&root, &data, &invalid).is_err());
            assert_eq!(fs::read(root.join("opencode.json")).unwrap(), bytes);
            assert_eq!(fs::read(backup_path(&data)).unwrap(), backup);
        }
        fs::remove_dir_all(base).unwrap();
    }

    fn context_doc(layout: Layout) -> Value {
        let mut doc = json!({"model": "cyapi/alias-a#deep", "custom": {"keep": true}});
        doc[layout.providers()] = json!({"other": {"keep": true}, "cyapi": {"models": {
            "alias-a": {"name": "A", "limit": {"context": 128000, "input": 100000, "output": 16000, "future": true},
                "variants": [{"id": "deep", "settings": {"reasoningEffort": "high"}}],
                "capabilities": {"tools": true}, "body": {"custom": true}},
            "alias-b": {"name": "B", "limit": {"context": 256000, "input": 200000, "output": 32000}},
            "alias-c": {"limit": {"context": {"future": [0, "auto"]}, "output": 8000}},
            "alias-d": {"limit": "future-limit-format"},
            "alias-e": {},
            "alias-f": {"limit": {"input": 500, "output": 1000}}
        }}});
        let provider = &mut doc[layout.providers()][PROVIDER];
        provider[layout.settings()] =
            json!({"apiKey": "dummy", "baseURL": "https://old.invalid", "custom": true});
        for (alias, id) in [
            ("alias-a", "model-a"),
            ("alias-b", "model-b"),
            ("alias-c", "model-c"),
            ("alias-d", "model-d"),
            ("alias-e", "model-e"),
            ("alias-f", "model-f"),
        ] {
            provider["models"][alias][layout.model_id()] = json!(id);
            provider["models"][alias][layout.settings()] =
                json!({"reasoningEffort": "custom-level", "temperature": 0.3});
        }
        doc
    }

    #[test]
    fn context_patches_preserve_unedited_limits_options_and_restore_both_layouts() {
        for layout in [Layout::Native, Layout::Legacy] {
            let (base, root, data) = fixture();
            let doc = context_doc(layout);
            let original = format!(
                "// keep context comment\n{}\n",
                serde_json::to_string_pretty(&doc).unwrap()
            );
            fs::write(root.join("opencode.jsonc"), &original).unwrap();
            let before = editor_view(&root, &data).unwrap();
            assert_eq!(before["modelContexts"]["model-a"], 128000);
            assert_eq!(before["modelContexts"]["model-b"], 256000);
            assert_eq!(
                before["modelContexts"]["model-c"],
                json!({"future": [0, "auto"]})
            );
            assert_eq!(before["modelContexts"]["model-d"], Value::Null);
            assert_eq!(before["modelContexts"]["model-e"], Value::Null);
            assert_eq!(before["modelContextConflicts"], json!([]));
            let original_entries = doc[layout.providers()][PROVIDER]["models"].clone();
            let mut change = input(&before);
            change.model = "model-a".into();
            change.api_key.clear();
            let url_saved = save(&root, &data, &change).unwrap();
            assert_eq!(read_doc(&root)["model"], doc["model"]);
            assert_eq!(
                read_doc(&root)[layout.providers()][PROVIDER]["models"],
                original_entries
            );
            assert_eq!(url_saved["modelContexts"], before["modelContexts"]);
            assert_eq!(url_saved["baseUrl"], "https://example.invalid/v1");
            change.revision = url_saved["revision"].as_str().unwrap().into();
            change.model = "model-b".into();
            let switched = save(&root, &data, &change).unwrap();
            assert_eq!(
                read_doc(&root)[layout.providers()][PROVIDER]["models"],
                original_entries
            );
            assert_eq!(switched["modelContexts"], before["modelContexts"]);
            let before_patch = fs::read(root.join("opencode.jsonc")).unwrap();
            change.revision = switched["revision"].as_str().unwrap().into();
            change
                .model_context_changes
                .insert("model-a".into(), Some(512000));
            let patched = save(&root, &data, &change).unwrap();
            assert_eq!(patched["modelContexts"]["model-a"], 512000);
            let mut expected = original_entries.clone();
            expected["alias-a"]["limit"]["context"] = json!(512000);
            assert_eq!(
                read_doc(&root)[layout.providers()][PROVIDER]["models"],
                expected
            );
            let restored = restore(&root, &data, patched["revision"].as_str().unwrap()).unwrap();
            assert_eq!(fs::read(root.join("opencode.jsonc")).unwrap(), before_patch);
            assert_eq!(restored["modelContexts"], switched["modelContexts"]);

            change.revision = restored["revision"].as_str().unwrap().into();
            change.model_context_changes.insert("model-a".into(), None);
            change.model_context_changes.insert("model-e".into(), None);
            change.model_context_changes.insert("model-f".into(), None);
            let cleared = save(&root, &data, &change).unwrap();
            assert_eq!(cleared["modelContexts"]["model-a"], Value::Null);
            assert_eq!(cleared["modelContexts"]["model-b"], 256000);
            let mut expected = original_entries.clone();
            expected["alias-a"]["limit"]
                .as_object_mut()
                .unwrap()
                .remove("context");
            let final_doc = read_doc(&root);
            assert_eq!(final_doc[layout.providers()][PROVIDER]["models"], expected);
            assert!(final_doc[layout.providers()][PROVIDER]["models"]["alias-e"]
                .get("limit")
                .is_none());
            assert_eq!(
                final_doc[layout.providers()]["other"],
                doc[layout.providers()]["other"]
            );
            assert_eq!(final_doc["custom"], doc["custom"]);
            assert!(final_doc[if layout == Layout::Native {
                "provider"
            } else {
                "providers"
            }]
            .is_null());
            assert!(fs::read_to_string(root.join("opencode.jsonc"))
                .unwrap()
                .contains("// keep context comment"));
            restore(&root, &data, cleared["revision"].as_str().unwrap()).unwrap();
            assert_eq!(fs::read(root.join("opencode.jsonc")).unwrap(), before_patch);
            fs::remove_dir_all(base).unwrap();
        }
    }

    #[test]
    fn context_alias_conflicts_are_reported_preserved_and_explicitly_resolved() {
        for layout in [Layout::Native, Layout::Legacy] {
            let (base, root, data) = fixture();
            let mut doc = context_doc(layout);
            let entries = doc[layout.providers()][PROVIDER]["models"]
                .as_object_mut()
                .unwrap();
            entries.clear();
            for (alias, id, context) in [
                ("a", "different", Some(json!(100))),
                ("b", "different", Some(json!(200))),
                ("c", "different", Some(json!(100))),
                ("d", "missing-versus-value", None),
                ("e", "missing-versus-value", Some(json!(100))),
                ("f", "null-equivalent", None),
                ("g", "null-equivalent", Some(Value::Null)),
                ("h", "matching-unknown", Some(json!({"future": true}))),
                ("i", "matching-unknown", Some(json!({"future": true}))),
            ] {
                let mut entry = json!({"limit": {"input": 50, "output": 10}, "custom": alias});
                entry[layout.model_id()] = json!(id);
                if let Some(context) = context {
                    entry["limit"]["context"] = context;
                }
                entries.insert(alias.into(), entry);
            }
            doc["model"] = json!("cyapi/b#deep");
            fs::write(
                root.join("opencode.json"),
                serde_json::to_vec(&doc).unwrap(),
            )
            .unwrap();
            let before = view(&root, &data).unwrap();
            assert_eq!(
                before["modelContextConflicts"],
                json!(["different", "missing-versus-value"])
            );
            assert_eq!(before["modelContexts"]["different"], Value::Null);
            assert_eq!(before["modelContexts"]["null-equivalent"], Value::Null);
            assert_eq!(
                before["modelContexts"]["matching-unknown"],
                json!({"future": true})
            );
            let mut change = input(&before);
            change.model = "different".into();
            let unchanged = save(&root, &data, &change).unwrap();
            assert_eq!(
                unchanged["modelContextConflicts"],
                before["modelContextConflicts"]
            );
            assert_eq!(
                read_doc(&root)[layout.providers()][PROVIDER]["models"],
                doc[layout.providers()][PROVIDER]["models"]
            );
            change.revision = unchanged["revision"].as_str().unwrap().into();
            change
                .model_context_changes
                .insert("different".into(), Some(1000));
            change
                .model_context_changes
                .insert("missing-versus-value".into(), None);
            let patched = save(&root, &data, &change).unwrap();
            assert_eq!(patched["modelContextConflicts"], json!([]));
            assert_eq!(patched["modelContexts"]["different"], 1000);
            let mut expected = doc[layout.providers()][PROVIDER]["models"].clone();
            for alias in ["a", "b", "c"] {
                expected[alias]["limit"]["context"] = json!(1000);
            }
            expected["e"]["limit"]
                .as_object_mut()
                .unwrap()
                .remove("context");
            assert_eq!(
                read_doc(&root)[layout.providers()][PROVIDER]["models"],
                expected
            );
            assert_eq!(read_doc(&root)["model"], "cyapi/b#deep");
            fs::remove_dir_all(base).unwrap();
        }
    }

    #[test]
    fn unknown_existing_context_values_are_exposed_and_preserved_without_validation() {
        for layout in [Layout::Native, Layout::Legacy] {
            let (base, root, data) = fixture();
            let mut doc = context_doc(layout);
            let entries = doc[layout.providers()][PROVIDER]["models"]
                .as_object_mut()
                .unwrap();
            entries.clear();
            let values = [
                json!(0),
                json!(-1),
                json!(1.5),
                json!(MAX_CONTEXT + 1),
                json!(u64::MAX),
                json!("auto"),
                json!(false),
                json!([100]),
                json!({"future": true}),
                Value::Null,
            ];
            for (index, context) in values.iter().enumerate() {
                let alias = format!("alias-{index}");
                let mut entry = json!({"limit": {"context": context, "input": 100, "output": 10}});
                entry[layout.model_id()] = json!(format!("model-{index}"));
                entries.insert(alias, entry);
            }
            doc["model"] = json!("cyapi/alias-0");
            fs::write(
                root.join("opencode.json"),
                serde_json::to_vec(&doc).unwrap(),
            )
            .unwrap();
            let before = view(&root, &data).unwrap();
            for (index, context) in values.iter().enumerate() {
                assert_eq!(&before["modelContexts"][format!("model-{index}")], context);
            }
            let mut change = input(&before);
            change.model = "model-0".into();
            let saved = save(&root, &data, &change).unwrap();
            assert_eq!(saved["modelContexts"], before["modelContexts"]);
            assert_eq!(
                read_doc(&root)[layout.providers()][PROVIDER]["models"],
                doc[layout.providers()][PROVIDER]["models"]
            );
            fs::remove_dir_all(base).unwrap();
        }
    }

    #[test]
    fn context_input_contract_rejects_invalid_types_and_defaults_to_no_patch() {
        let value = json!({"baseUrl": "https://example.invalid", "apiKey": "dummy", "model": "model-a", "revision": "unused"});
        let decoded: Input = serde_json::from_value(value.clone()).unwrap();
        assert!(decoded.model_context_changes.is_empty());
        let mut explicit = value.clone();
        explicit["modelContextChanges"] =
            json!({"model-a": 1, "model-b": null, "model-c": MAX_CONTEXT});
        let decoded: Input = serde_json::from_value(explicit).unwrap();
        assert_eq!(decoded.model_context_changes["model-a"], Some(1));
        assert_eq!(decoded.model_context_changes["model-b"], None);
        assert_eq!(decoded.model_context_changes["model-c"], Some(MAX_CONTEXT));
        for context in [
            json!(-1),
            json!(1.5),
            json!("100"),
            json!(true),
            json!([]),
            json!({}),
            json!(18446744073709551616.0),
        ] {
            let mut invalid = value.clone();
            invalid["modelContextChanges"] = json!({"model-a": context});
            assert!(serde_json::from_value::<Input>(invalid).is_err());
        }
    }

    #[test]
    fn invalid_context_patches_never_create_or_modify_configs_or_backups() {
        for layout in [Layout::Native, Layout::Legacy] {
            let (base, root, data) = fixture();
            let empty = view(&root, &data).unwrap();
            for (id, context) in [
                ("model-a", Some(0)),
                ("model-a", Some(MAX_CONTEXT + 1)),
                ("unselected", Some(1)),
                ("unselected", None),
            ] {
                let mut invalid = input(&empty);
                invalid.model = "model-a".into();
                invalid.model_context_changes.insert(id.into(), context);
                assert!(save(&root, &data, &invalid).is_err());
                assert!(!root.join("opencode.json").exists());
                assert!(!backup_path(&data).exists());
            }
            let doc = context_doc(layout);
            fs::write(
                root.join("opencode.json"),
                serde_json::to_vec(&doc).unwrap(),
            )
            .unwrap();
            let before = view(&root, &data).unwrap();
            let mut valid = input(&before);
            valid.model = "model-a".into();
            valid
                .model_context_changes
                .insert("model-a".into(), Some(MAX_CONTEXT));
            valid
                .model_context_changes
                .insert("model-e".into(), Some(1));
            let saved = save(&root, &data, &valid).unwrap();
            assert_eq!(saved["modelContexts"]["model-a"], MAX_CONTEXT);
            assert_eq!(saved["modelContexts"]["model-e"], 1);
            let bytes = fs::read(root.join("opencode.json")).unwrap();
            let backup = fs::read(backup_path(&data)).unwrap();
            for (id, context) in [
                ("model-a", Some(0)),
                ("model-a", Some(MAX_CONTEXT + 1)),
                ("model-a", Some(u64::MAX)),
                ("unselected", Some(1)),
                ("unselected", None),
                (" model-a", Some(100)),
                ("model-d", Some(100)),
                ("model-d", None),
            ] {
                let mut invalid = input(&saved);
                invalid.model = "model-a".into();
                invalid.model_context_changes.insert(id.into(), context);
                let error = save(&root, &data, &invalid).unwrap_err();
                if id == "model-d" {
                    assert!(error.contains("limit 必须是 JSON 对象"));
                }
                assert_eq!(fs::read(root.join("opencode.json")).unwrap(), bytes);
                assert_eq!(fs::read(backup_path(&data)).unwrap(), backup);
            }
            fs::remove_dir_all(base).unwrap();
        }
    }

    #[test]
    fn context_edits_reject_every_non_object_limit_shape_for_all_aliases() {
        for layout in [Layout::Native, Layout::Legacy] {
            for limit in [
                Value::Null,
                json!(false),
                json!(100),
                json!("auto"),
                json!([]),
            ] {
                let (base, root, data) = fixture();
                let mut doc = context_doc(layout);
                let entries = &mut doc[layout.providers()][PROVIDER]["models"];
                entries["alias-b"][layout.model_id()] = json!("model-a");
                entries["alias-b"]["limit"] = limit;
                let bytes = serde_json::to_vec(&doc).unwrap();
                fs::write(root.join("opencode.json"), &bytes).unwrap();
                let before = view(&root, &data).unwrap();
                for context in [None, Some(1000)] {
                    let mut invalid = input(&before);
                    invalid.model = "model-a".into();
                    invalid
                        .model_context_changes
                        .insert("model-a".into(), context);
                    assert!(save(&root, &data, &invalid)
                        .unwrap_err()
                        .contains("limit 必须是 JSON 对象"));
                    assert_eq!(fs::read(root.join("opencode.json")).unwrap(), bytes);
                    assert!(!backup_path(&data).exists());
                }
                fs::remove_dir_all(base).unwrap();
            }
        }
    }

    #[test]
    fn invalid_model_lists_and_efforts_do_not_create_files_or_backups() {
        let (base, root, data) = fixture();
        let before = view(&root, &data).unwrap();
        for (models, effort) in [
            (vec!["bad\nmodel".into()], "high"),
            (vec!["valid".into()], "bad effort"),
        ] {
            let mut invalid = input(&before);
            invalid.models = Some(models);
            invalid.reasoning_effort = Some(effort.into());
            assert!(save(&root, &data, &invalid).is_err());
            assert!(!root.join("opencode.json").exists());
            assert!(!backup_path(&data).exists());
        }
        fs::remove_dir_all(base).unwrap();
    }
}
