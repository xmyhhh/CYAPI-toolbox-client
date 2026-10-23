//! Basic Codex API configuration, with comment-preserving TOML and guarded local writes.
//! Adapted from CC Switch's codex_config.rs (MIT, Copyright (c) 2025 Jason Young).
//! Upstream revision and license: client/THIRD-PARTY-NOTICES.md.
use crate::configuration::{no_links, read_bytes, stage};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};
use toml_edit::{value, DocumentMut, Item, Table};

type Result<T> = std::result::Result<T, String>;
const PROVIDER: &str = "cyapi-toolbox";

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Input {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub models: Option<Vec<String>>,
    pub reasoning_effort: String,
    pub revision: String,
    #[serde(default)]
    pub save_revision: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
struct Snapshot {
    config: Option<Vec<u8>>,
    auth: Option<Vec<u8>>,
}
impl Snapshot {
    fn read(root: &Path) -> Result<Self> {
        let read = |name: &str| {
            let path = root.join(name);
            no_links(&path)?;
            read_bytes(&path)
        };
        Ok(Self {
            config: read("config.toml")?,
            auth: read("auth.json")?,
        })
    }
    fn revision(&self) -> String {
        // Include absence separately from an existing empty file.
        crate::service::hash(&serde_json::to_vec(self).expect("byte arrays serialize"))
    }
}

#[derive(Serialize, Deserialize)]
struct Backup {
    schema: u32,
    directory: PathBuf,
    config: Option<String>,
    auth: Option<String>,
    config_after: Option<String>,
    auth_after: Option<String>,
}
impl Backup {
    fn original(&self) -> Result<Snapshot> {
        let decode = |s: &Option<String>| -> Result<Option<Vec<u8>>> {
            s.as_ref()
                .map(|s| STANDARD.decode(s).map_err(|_| "Codex 备份无效".into()))
                .transpose()
        };
        let snapshot = Snapshot {
            config: decode(&self.config)?,
            auth: decode(&self.auth)?,
        };
        if [&snapshot.config, &snapshot.auth]
            .iter()
            .any(|b| b.as_ref().is_some_and(|b| b.len() > 1_000_000))
        {
            return Err("Codex 备份过大".into());
        }
        Ok(snapshot)
    }
    fn matches(&self, root: &Path, current: &Snapshot) -> Result<bool> {
        if self.schema != 1 || self.directory != root {
            return Ok(false);
        }
        let before = self.original()?;
        let known =
            |current: &Option<Vec<u8>>, original: &Option<Vec<u8>>, after: &Option<String>| {
                current == original || current.as_ref().map(|b| crate::service::hash(b)) == *after
            };
        Ok(known(&current.config, &before.config, &self.config_after)
            && known(&current.auth, &before.auth, &self.auth_after))
    }
}

fn backup_path(data: &Path) -> PathBuf {
    data.join("codex-config/previous.json")
}
fn read_backup(data: &Path) -> Result<Option<Backup>> {
    let path = backup_path(data);
    no_links(&path)?;
    match fs::metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(m) if m.is_file() && m.len() <= 3_000_000 => {}
        _ => return Err("无法读取 Codex 备份".into()),
    }
    let backup: Backup =
        serde_json::from_slice(&fs::read(path).map_err(|_| "无法读取 Codex 备份")?)
            .map_err(|_| "Codex 备份无效")?;
    if backup.schema != 1 {
        return Err("不支持的 Codex 备份格式".into());
    }
    backup.original()?;
    Ok(Some(backup))
}

fn parse_config(bytes: &Option<Vec<u8>>) -> Result<DocumentMut> {
    let text = std::str::from_utf8(bytes.as_deref().unwrap_or_default())
        .map_err(|_| "config.toml 不是 UTF-8 文件")?;
    // Never surface parser errors: snippets can contain local keys or tokens.
    text.trim_start_matches('\u{feff}')
        .parse()
        .map_err(|_| "config.toml 格式无效，请修复后重新读取".into())
}
fn parse_auth(bytes: &Option<Vec<u8>>) -> Result<Value> {
    let auth: Value = match bytes {
        Some(bytes) => {
            serde_json::from_slice(bytes).map_err(|_| "auth.json 格式无效，请修复后重新读取")?
        }
        None => json!({}),
    };
    if !auth.is_object() {
        return Err("auth.json 必须是 JSON 对象".into());
    }
    Ok(auth)
}
fn setting<'a>(doc: &'a DocumentMut, key: &str) -> Option<&'a str> {
    doc.get(key).and_then(Item::as_str)
}
fn key_in_auth(auth: &Value) -> Option<&str> {
    if auth
        .get("auth_mode")
        .and_then(Value::as_str)
        .is_some_and(|mode| mode != "apikey")
    {
        return None;
    }
    auth.get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

// Hash TOML content without comments, key order, or quote/whitespace differences.
fn semantic_value(value: &toml_edit::Value) -> Value {
    use toml_edit::Value as TomlValue;
    match value {
        TomlValue::String(value) => json!(value.value()),
        TomlValue::Integer(value) => json!(value.value()),
        TomlValue::Float(value) => json!(value.value()),
        TomlValue::Boolean(value) => json!(value.value()),
        TomlValue::Datetime(value) => json!(value.value().to_string()),
        TomlValue::Array(values) => Value::Array(values.iter().map(semantic_value).collect()),
        TomlValue::InlineTable(table) => Value::Object(
            table
                .iter()
                .map(|(key, value)| (key.into(), semantic_value(value)))
                .collect(),
        ),
    }
}
fn semantic_item(item: Option<&Item>) -> Value {
    match item {
        Some(Item::Value(value)) => semantic_value(value),
        Some(Item::Table(table)) => Value::Object(
            table
                .iter()
                .map(|(key, item)| (key.into(), semantic_item(Some(item))))
                .collect(),
        ),
        Some(Item::ArrayOfTables(tables)) => Value::Array(
            tables
                .iter()
                .map(|table| semantic_item(Some(&Item::Table(table.clone()))))
                .collect(),
        ),
        _ => Value::Null,
    }
}
fn save_revision(root: &Path, doc: &DocumentMut, auth: &Value) -> String {
    let provider = setting(doc, "model_provider").unwrap_or("openai");
    let fields = [
        "model_provider",
        "model",
        "model_reasoning_effort",
        "model_catalog_json",
        "cli_auth_credentials_store",
        "forced_login_method",
    ];
    let config: serde_json::Map<_, _> = fields
        .into_iter()
        .map(|key| (key.into(), semantic_item(doc.get(key))))
        .collect();
    let provider_settings = |name: &str| -> Value {
        let table = doc.get("model_providers").and_then(|table| table.get(name));
        let fields = [
            "name",
            "base_url",
            "wire_api",
            "requires_openai_auth",
            "env_key",
            "env_key_instructions",
            "auth",
            "experimental_bearer_token",
            "http_headers",
            "env_http_headers",
        ];
        Value::Object(
            fields
                .into_iter()
                .map(|key| {
                    (
                        key.into(),
                        semantic_item(table.and_then(|table| table.get(key))),
                    )
                })
                .collect(),
        )
    };
    // Only these settings are read/overwritten by the editor. Project trust, MCP,
    // refreshed login tokens, and other background state must not stale the form.
    let content = json!({
        "schema": 1, "directory": root, "config": config,
        "openaiBaseUrl": (provider == "openai").then(|| semantic_item(doc.get("openai_base_url"))),
        "activeProvider": provider_settings(provider),
        "toolboxProvider": provider_settings(PROVIDER),
        "authMode": auth.get("auth_mode"), "apiKey": auth.get("OPENAI_API_KEY"),
        "hasLogin": auth.get("tokens").is_some_and(Value::is_object),
        "accountId": auth.get("tokens").and_then(|tokens| tokens.get("account_id")),
    });
    crate::service::hash(&serde_json::to_vec(&content).expect("configuration values serialize"))
}

pub fn view(root: &Path, data: &Path) -> Result<Value> {
    let snapshot = Snapshot::read(root)?;
    snapshot_view(root, data, &snapshot)
}

// Only the local Codex editor requests the credential, for the password field/reveal button.
pub fn editor_view(root: &Path, data: &Path) -> Result<Value> {
    let snapshot = Snapshot::read(root)?;
    let mut view = snapshot_view(root, data, &snapshot)?;
    let auth = parse_auth(&snapshot.auth)?;
    view["apiKey"] = json!(key_in_auth(&auth).unwrap_or(""));
    Ok(view)
}

fn snapshot_view(root: &Path, data: &Path, snapshot: &Snapshot) -> Result<Value> {
    let doc = parse_config(&snapshot.config)?;
    let auth = parse_auth(&snapshot.auth)?;
    let provider = setting(&doc, "model_provider").unwrap_or("openai");
    let provider_table = doc.get("model_providers").and_then(|p| p.get(provider));
    let base_url = provider_table
        .and_then(|p| p.get("base_url"))
        .and_then(Item::as_str)
        .or_else(|| {
            (provider == "openai")
                .then(|| setting(&doc, "openai_base_url"))
                .flatten()
        })
        .unwrap_or(if provider == "openai" {
            "https://api.openai.com/v1"
        } else {
            ""
        });
    let auth_mode = if key_in_auth(&auth).is_some() {
        "API Key"
    } else if auth.get("tokens").is_some_and(Value::is_object) {
        "ChatGPT 登录"
    } else {
        "未读取到文件凭据"
    };
    let can_restore = read_backup(data)?
        .map(|b| b.matches(root, snapshot))
        .transpose()?
        .unwrap_or(false);
    Ok(json!({
        "configPath":root.join("config.toml"), "authPath":root.join("auth.json"),
        "revision":snapshot.revision(), "saveRevision":save_revision(root, &doc, &auth),
        "provider":provider,
        "baseUrl":base_url, "model":setting(&doc, "model").unwrap_or(""),
        "reasoningEffort":setting(&doc, "model_reasoning_effort").unwrap_or(""),
        "models":crate::codex_models::selected(root, &doc)?,
        "apiKeyConfigured":key_in_auth(&auth).is_some(), "authMode":auth_mode,
        "canRestore":can_restore
    }))
}

fn validate(input: &Input) -> Result<()> {
    crate::model_selection::models(&input.model, input.models.as_deref().unwrap_or_default())?;
    crate::model_selection::effort(&input.reasoning_effort, crate::model_selection::EFFORTS)?;
    let url = reqwest::Url::parse(input.base_url.trim()).map_err(|_| "API 地址无效")?;
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
    if input.reasoning_effort.len() > 64 || input.reasoning_effort.chars().any(char::is_control) {
        return Err("不支持的推理强度".into());
    }
    if input.api_key.len() > 8192
        || input
            .api_key
            .trim()
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("API Key 格式无效".into());
    }
    Ok(())
}
fn table<'a>(doc: &'a mut DocumentMut, path: &[&str]) -> Result<&'a mut dyn toml_edit::TableLike> {
    let mut current: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for key in path {
        if !current.contains_key(key) {
            current.insert(key, Item::Table(Table::new()));
        }
        current = current
            .get_mut(key)
            .and_then(Item::as_table_like_mut)
            .ok_or("Codex 配置表结构无效")?;
    }
    Ok(current)
}
fn set(table: &mut dyn toml_edit::TableLike, key: &str, mut item: Item) {
    if let Some(previous) = table.get_mut(key) {
        if let (Some(old), Some(new)) = (previous.as_value(), item.as_value_mut()) {
            *new.decor_mut() = old.decor().clone();
        }
        // Update in place to retain key decorations, including leading file comments.
        *previous = item;
    } else {
        table.insert(key, item);
    }
}
fn put_config(doc: &mut DocumentMut, input: &Input) -> Result<()> {
    // Current Codex uses separate <name>.config.toml files for CLI profiles.
    // This editor updates the global config.toml only.
    let target = doc.as_table_mut();
    set(target, "model_provider", value(PROVIDER));
    set(target, "model", value(input.model.trim()));
    if input.reasoning_effort.is_empty() {
        target.remove("model_reasoning_effort");
    } else {
        set(
            target,
            "model_reasoning_effort",
            value(&input.reasoning_effort),
        );
    }
    // File credentials avoid a stale OS keyring overriding the new API key.
    set(
        doc.as_table_mut(),
        "cli_auth_credentials_store",
        value("file"),
    );
    set(doc.as_table_mut(), "forced_login_method", value("api"));
    let provider = table(doc, &["model_providers", PROVIDER])?;
    set(provider, "name", value("CYAPI Toolbox"));
    set(
        provider,
        "base_url",
        value(input.base_url.trim().trim_end_matches('/')),
    );
    set(provider, "wire_api", value("responses"));
    set(provider, "requires_openai_auth", value(true));
    // This provider is owned by the toolbox. Clear alternative auth sources on it only.
    for field in [
        "env_key",
        "env_key_instructions",
        "auth",
        "experimental_bearer_token",
        "http_headers",
        "env_http_headers",
    ] {
        provider.remove(field);
    }
    Ok(())
}

fn replace(path: &Path, bytes: Option<&[u8]>) -> Result<()> {
    no_links(path)?;
    match bytes {
        Some(bytes) => {
            let temp = stage(path, bytes)?;
            fs::rename(&temp, path).map_err(|_| {
                let _ = fs::remove_file(temp);
                "无法替换 Codex 配置文件，请检查是否被占用".into()
            })
        }
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("无法恢复 Codex 配置文件".into()),
        },
    }
}
// Validate and stage both files before committing. Roll back only files we actually replaced.
fn commit(root: &Path, original: &Snapshot, next: &Snapshot) -> Result<()> {
    let files = [
        (root.join("auth.json"), &original.auth, &next.auth),
        (root.join("config.toml"), &original.config, &next.config),
    ];
    let mut staged = Vec::new();
    for (path, before, after) in &files {
        if before == after {
            staged.push(None);
            continue;
        }
        let result = after.as_ref().map(|bytes| stage(path, bytes)).transpose();
        match result {
            Ok(temp) => staged.push(temp),
            Err(e) => {
                for temp in staged.into_iter().flatten() {
                    let _ = fs::remove_file(temp);
                }
                return Err(e);
            }
        }
    }
    let mut committed = Vec::new();
    let result = (|| {
        if Snapshot::read(root)? != *original {
            return Err("Codex 配置已被其他程序修改，请重新读取".into());
        }
        for ((path, before, after), temp) in files.iter().zip(&staged) {
            no_links(path)?;
            if &read_bytes(path)? != *before {
                return Err("Codex 配置已被其他程序修改，请重新读取".into());
            }
            if before == after {
                continue;
            }
            if let Some(temp) = temp {
                fs::rename(temp, path).map_err(|_| "无法替换 Codex 配置文件，请检查是否被占用")?;
            } else {
                replace(path, None)?;
            }
            committed.push((path, before, after));
        }
        Ok(())
    })();
    let mut failed = false;
    if result.is_err() {
        for (path, before, after) in committed.into_iter().rev() {
            // Do not overwrite a concurrent refresh by Codex while rolling back.
            if read_bytes(path).ok().as_ref() != Some(after)
                || replace(path, before.as_deref()).is_err()
            {
                failed = true;
            }
        }
    }
    for temp in staged.into_iter().flatten() {
        let _ = fs::remove_file(temp);
    }
    if failed {
        return Err("Codex 配置保存中断，请用本工具的备份恢复，或检查配置文件".into());
    }
    result
}

pub fn save(root: &Path, data: &Path, input: &Input) -> Result<Value> {
    validate(input)?;
    let original = Snapshot::read(root)?;
    let mut doc = parse_config(&original.config)?;
    let mut auth = parse_auth(&original.auth)?;
    if original.revision() != input.revision
        && input.save_revision.as_deref() != Some(save_revision(root, &doc, &auth).as_str())
    {
        return Err(
            "Codex 的模型、线路或认证设置已被其他程序修改，请重新读取并核对后保存；当前输入已保留"
                .into(),
        );
    }
    let key = if input.api_key.trim().is_empty() {
        key_in_auth(&auth).ok_or("请填写 API Key")?
    } else {
        input.api_key.trim()
    }
    .to_string();
    if let Some(models) = &input.models {
        let selected = crate::model_selection::models(&input.model, models)?;
        if selected.len() > 1 || !crate::codex_models::selected(root, &doc)?.is_empty() {
            let path =
                crate::codex_models::write(root, data, &doc, &selected, &input.reasoning_effort)?;
            set(
                doc.as_table_mut(),
                "model_catalog_json",
                value(path.to_string_lossy().as_ref()),
            );
        }
    }
    put_config(&mut doc, input)?;
    auth["auth_mode"] = json!("apikey");
    auth["OPENAI_API_KEY"] = json!(key);
    let next = Snapshot {
        config: Some(doc.to_string().into_bytes()),
        auth: Some(serde_json::to_vec_pretty(&auth).map_err(|_| "无法生成 auth.json")?),
    };
    if [&next.config, &next.auth]
        .iter()
        .any(|b| b.as_ref().is_some_and(|b| b.len() > 1_000_000))
    {
        return Err("生成的 Codex 配置文件过大".into());
    }
    if next == original {
        return view(root, data);
    }
    let backup = Backup {
        schema: 1,
        directory: root.into(),
        config: original.config.as_ref().map(|b| STANDARD.encode(b)),
        auth: original.auth.as_ref().map(|b| STANDARD.encode(b)),
        config_after: next.config.as_ref().map(|b| crate::service::hash(b)),
        auth_after: next.auth.as_ref().map(|b| crate::service::hash(b)),
    };
    // Verify any existing backup before replacing it, so a damaged backup is not silently lost.
    read_backup(data)?;
    replace(
        &backup_path(data),
        Some(&serde_json::to_vec(&backup).map_err(|_| "无法生成 Codex 备份")?),
    )?;
    commit(root, &original, &next)?;
    view(root, data)
}
pub fn restore(root: &Path, data: &Path, revision: &str) -> Result<Value> {
    let current = Snapshot::read(root)?;
    if current.revision() != revision {
        return Err("Codex 配置已变化，请重新读取后恢复".into());
    }
    let backup = read_backup(data)?.ok_or("没有可恢复的 Codex 配置")?;
    if !backup.matches(root, &current)? {
        return Err("Codex 配置已在外部修改，无法直接恢复旧备份".into());
    }
    let original = backup.original()?;
    // Parse before writing; backups are local data, never executable instructions.
    parse_config(&original.config)?;
    parse_auth(&original.auth)?;
    commit(root, &current, &original)?;
    replace(&backup_path(data), None)?;
    view(root, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    fn fixture() -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "cyapi-codex-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let root = base.join("home/.codex");
        fs::create_dir_all(&root).unwrap();
        (base.clone(), root, base.join("data"))
    }
    fn input(view: &Value) -> Input {
        Input {
            base_url: "https://example.invalid/v1/".into(),
            api_key: "dummy-api-key".into(),
            models: None,
            model: "example-model".into(),
            reasoning_effort: "high".into(),
            revision: view["revision"].as_str().unwrap().into(),
            save_revision: view["saveRevision"].as_str().map(str::to_owned),
        }
    }
    #[test]
    fn saves_masked_api_configuration_preserves_comments_and_restores_login_exactly() {
        let (base, root, data) = fixture();
        let config = b"# keep my comments\nmodel = 'old-model'\ncli_auth_credentials_store = 'auto'\n\n[mcp_servers.example]\ncommand = 'node'\nargs = ['keep-me']\n";
        let auth = br#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"access_token":"dummy-login-token","refresh_token":"dummy-refresh"},"extra":"keep-me"}"#;
        fs::write(root.join("config.toml"), config).unwrap();
        fs::write(root.join("auth.json"), auth).unwrap();
        let before = view(&root, &data).unwrap();
        assert!(!before["apiKeyConfigured"].as_bool().unwrap());
        assert_eq!(editor_view(&root, &data).unwrap()["apiKey"], "");
        let saved = save(&root, &data, &input(&before)).unwrap();
        assert!(!saved.to_string().contains("dummy-"));
        assert_eq!(saved["apiKeyConfigured"], true);
        assert_eq!(saved["canRestore"], true);
        assert_eq!(saved["provider"], PROVIDER);
        assert_eq!(saved["baseUrl"], "https://example.invalid/v1");
        let snapshot = Snapshot::read(&root).unwrap();
        let text = String::from_utf8(snapshot.config.clone().unwrap()).unwrap();
        assert!(text.contains("# keep my comments"));
        let doc = parse_config(&snapshot.config).unwrap();
        assert_eq!(
            doc["mcp_servers"]["example"]["args"][0].as_str(),
            Some("keep-me")
        );
        assert_eq!(doc["cli_auth_credentials_store"].as_str(), Some("file"));
        assert_eq!(
            doc["model_providers"][PROVIDER]["wire_api"].as_str(),
            Some("responses")
        );
        let auth_doc = parse_auth(&snapshot.auth).unwrap();
        assert_eq!(auth_doc["auth_mode"], "apikey");
        assert_eq!(auth_doc["OPENAI_API_KEY"], "dummy-api-key");
        assert_eq!(auth_doc["tokens"]["access_token"], "dummy-login-token");
        let editor = editor_view(&root, &data).unwrap();
        assert_eq!(editor["apiKey"], "dummy-api-key");
        assert!(!editor.to_string().contains("dummy-login-token"));
        assert!(!editor.to_string().contains("dummy-refresh"));
        assert_eq!(Snapshot::read(&root).unwrap().config, snapshot.config);
        assert_eq!(Snapshot::read(&root).unwrap().auth, snapshot.auth);
        // An unchanged save with a blank key must retain both the key and the previous restore point.
        let mut repeated = input(&saved);
        repeated.api_key.clear();
        save(&root, &data, &repeated).unwrap();
        let current = view(&root, &data).unwrap();
        restore(&root, &data, current["revision"].as_str().unwrap()).unwrap();
        assert_eq!(fs::read(root.join("config.toml")).unwrap(), config);
        assert_eq!(fs::read(root.join("auth.json")).unwrap(), auth);
        assert_eq!(view(&root, &data).unwrap()["canRestore"], false);
        assert_eq!(editor_view(&root, &data).unwrap()["apiKey"], "");
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn edits_global_settings_preserves_profile_files_and_blocks_stale_forms() {
        let (base, root, data) = fixture();
        fs::write(root.join("config.toml"), "profile = 'work'\nmodel = 'root-model'\nmodel_provider = 'other'\n[profiles.work]\nmodel = 'profile-model'\n[profiles.other]\nmodel = 'keep-other-model'\n[model_providers.other]\nbase_url = 'https://other.invalid/v1'\nname = 'Other'\n").unwrap();
        let profile = b"model = 'profile-file-model'\n";
        fs::write(root.join("work.config.toml"), profile).unwrap();
        let before = view(&root, &data).unwrap();
        assert_eq!(before["model"], "root-model");
        let saved = save(&root, &data, &input(&before)).unwrap();
        let doc = parse_config(&Snapshot::read(&root).unwrap().config).unwrap();
        assert_eq!(doc["model"].as_str(), Some("example-model"));
        assert_eq!(
            doc["profiles"]["work"]["model"].as_str(),
            Some("profile-model")
        );
        assert_eq!(fs::read(root.join("work.config.toml")).unwrap(), profile);
        assert_eq!(
            doc["profiles"]["other"]["model"].as_str(),
            Some("keep-other-model")
        );
        assert_eq!(saved["model"], "example-model");
        let error = save(&root, &data, &input(&before)).unwrap_err();
        assert!(error.contains("已被其他程序修改"));
        let current = fs::read(root.join("config.toml")).unwrap();
        fs::write(
            root.join("config.toml"),
            [current.as_slice(), b"\n# external edit\n"].concat(),
        )
        .unwrap();
        assert!(restore(&root, &data, saved["revision"].as_str().unwrap()).is_err());
        let after_external = Snapshot::read(&root).unwrap();
        assert!(!view(&root, &data).unwrap()["canRestore"].as_bool().unwrap());
        let revision = after_external.revision();
        assert!(restore(&root, &data, &revision).is_err());
        assert_eq!(Snapshot::read(&root).unwrap().config, after_external.config);
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn background_updates_do_not_block_save_and_restore_keeps_the_latest_state() {
        let (base, root, data) = fixture();
        fs::write(
            root.join("config.toml"),
            "# original\nmodel = 'old-model'\n",
        )
        .unwrap();
        fs::write(root.join("auth.json"), br#"{"auth_mode":"chatgpt","tokens":{"account_id":"same-account","access_token":"old-token"},"last_refresh":"old"}"#).unwrap();
        let before = editor_view(&root, &data).unwrap();
        let pending = input(&before);

        // Reproduce Codex updating project trust/MCP and refreshing its login
        // between opening the form and clicking Save, without editing API settings.
        let updated_config = "# updated in the background\nmodel = \"old-model\"\n\n[projects.'C:\\work']\ntrust_level = 'trusted'\n\n[mcp_servers.example]\ncommand = 'node'\nargs = ['new-server']\n";
        let updated_auth = br#"{"last_refresh":"new","tokens":{"access_token":"new-token","refresh_token":"new-refresh","account_id":"same-account"},"auth_mode":"chatgpt","extra":"preserve-me"}"#;
        fs::write(root.join("config.toml"), updated_config).unwrap();
        fs::write(root.join("auth.json"), updated_auth).unwrap();
        let latest = Snapshot::read(&root).unwrap();
        assert_ne!(latest.revision(), pending.revision);
        let updated = editor_view(&root, &data).unwrap();
        assert_eq!(before["saveRevision"], updated["saveRevision"]);

        // Old callers without a scoped revision still receive strict protection.
        let mut legacy = pending.clone();
        legacy.save_revision = None;
        assert!(save(&root, &data, &legacy).is_err());
        assert!(!backup_path(&data).exists());

        let saved = save(&root, &data, &pending).unwrap();
        assert_eq!(saved["model"], pending.model);
        assert_eq!(saved["baseUrl"], "https://example.invalid/v1");
        let snapshot = Snapshot::read(&root).unwrap();
        let doc = parse_config(&snapshot.config).unwrap();
        assert_eq!(
            doc["projects"]["C:\\work"]["trust_level"].as_str(),
            Some("trusted")
        );
        assert_eq!(
            doc["mcp_servers"]["example"]["args"][0].as_str(),
            Some("new-server")
        );
        let auth = parse_auth(&snapshot.auth).unwrap();
        assert_eq!(auth["tokens"]["access_token"], "new-token");
        assert_eq!(auth["tokens"]["refresh_token"], "new-refresh");
        assert_eq!(auth["last_refresh"], "new");
        assert_eq!(auth["extra"], "preserve-me");
        assert!(!saved.to_string().contains("new-token"));

        restore(&root, &data, saved["revision"].as_str().unwrap()).unwrap();
        assert_eq!(
            fs::read(root.join("config.toml")).unwrap(),
            updated_config.as_bytes()
        );
        assert_eq!(fs::read(root.join("auth.json")).unwrap(), updated_auth);
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn managed_formatting_changes_do_not_block_saving_with_the_existing_key() {
        let (base, root, data) = fixture();
        fs::write(root.join("config.toml"), "model = 'old-model'\nmodel_provider = 'cyapi-toolbox'\n[model_providers.cyapi-toolbox]\nbase_url = 'https://old.invalid/v1'\nrequires_openai_auth = true\nhttp_headers = { A = 'one', B = 'two' }\n").unwrap();
        fs::write(
            root.join("auth.json"),
            br#"{"auth_mode":"apikey","OPENAI_API_KEY":"existing-key"}"#,
        )
        .unwrap();
        let before = editor_view(&root, &data).unwrap();
        let mut pending = input(&before);
        pending.api_key.clear();
        fs::write(root.join("config.toml"), "# changed layout\nmodel_provider = \"cyapi-toolbox\"\nmodel = \"old-model\"\n[model_providers.cyapi-toolbox]\nrequires_openai_auth=true\nbase_url=\"https://old.invalid/v1\" # same route\n[model_providers.cyapi-toolbox.http_headers]\nB = \"two\"\nA = \"one\"\n").unwrap();
        fs::write(
            root.join("auth.json"),
            "{\n  \"OPENAI_API_KEY\": \"existing-key\",\n  \"auth_mode\": \"apikey\"\n}\n",
        )
        .unwrap();
        assert_eq!(
            before["saveRevision"],
            editor_view(&root, &data).unwrap()["saveRevision"]
        );
        save(&root, &data, &pending).unwrap();
        assert_eq!(editor_view(&root, &data).unwrap()["apiKey"], "existing-key");
        assert!(fs::read_to_string(root.join("config.toml"))
            .unwrap()
            .contains("# changed layout"));
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn concurrent_model_route_and_auth_changes_still_block_without_writing() {
        let original_config = "model = 'old-model'\nmodel_provider = 'other'\nmodel_reasoning_effort = 'low'\ncli_auth_credentials_store = 'auto'\nforced_login_method = 'chatgpt'\n[model_providers.other]\nbase_url = 'https://old.invalid/v1'\n[model_providers.cyapi-toolbox]\nbase_url = 'https://toolbox.invalid/v1'\n";
        let original_auth = r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":"old-key","tokens":{"account_id":"original-account","access_token":"token"}}"#;
        for (from, to, auth_change) in [
            ("'old-model'", "'external-model'", false),
            (
                "model_provider = 'other'",
                "model_provider = 'openai'",
                false,
            ),
            ("'low'", "'high'", false),
            ("'auto'", "'file'", false),
            ("'chatgpt'", "'api'", false),
            (
                "https://old.invalid/v1",
                "https://changed.invalid/v1",
                false,
            ),
            (
                "https://toolbox.invalid/v1",
                "https://changed.invalid/v1",
                false,
            ),
            ("\"chatgpt\"", "\"apikey\"", true),
            ("old-key", "external-key", true),
            ("original-account", "different-account", true),
            ("\"tokens\"", "\"removed_tokens\"", true),
        ] {
            let (base, root, data) = fixture();
            fs::write(root.join("config.toml"), original_config).unwrap();
            fs::write(root.join("auth.json"), original_auth).unwrap();
            let pending = input(&editor_view(&root, &data).unwrap());
            if auth_change {
                fs::write(root.join("auth.json"), original_auth.replace(from, to)).unwrap();
            } else {
                fs::write(root.join("config.toml"), original_config.replace(from, to)).unwrap();
            }
            let latest = Snapshot::read(&root).unwrap();
            let error = save(&root, &data, &pending).unwrap_err();
            assert!(error.contains("已被其他程序修改"), "{from}: {error}");
            assert!(!error.contains("old-key") && !error.contains("external-key"));
            assert_eq!(Snapshot::read(&root).unwrap().config, latest.config);
            assert_eq!(Snapshot::read(&root).unwrap().auth, latest.auth);
            assert!(!backup_path(&data).exists());
            fs::remove_dir_all(base).unwrap();
        }
    }
    #[test]
    fn configuration_dispatch_accepts_scoped_revision_and_keeps_restore_strict() {
        let (base, root, data) = fixture();
        fs::write(root.join("config.toml"), "model = 'old-model'\n").unwrap();
        let mut service = crate::service::Service::new(data, base.join("home")).unwrap();
        let before = service
            .dispatch(serde_json::from_value(json!({"action": "codex-read"})).unwrap())
            .unwrap();
        let updated = "model = 'old-model'\n[mcp_servers.new]\ncommand = 'node'\n";
        fs::write(root.join("config.toml"), updated).unwrap();
        let saved = service
            .dispatch(
                serde_json::from_value(json!({
                    "action": "codex-save",
                    "codex": {
                        "baseUrl": "https://example.invalid/v1",
                        "apiKey": "dispatch-key",
                        "model": "dispatch-model",
                        "reasoningEffort": "high",
                        "revision": before["revision"],
                        "saveRevision": before["saveRevision"],
                    }
                }))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(saved["model"], "dispatch-model");
        assert_eq!(saved["apiKey"], "dispatch-key");
        assert!(saved["saveRevision"].is_string());
        let saved_config = fs::read_to_string(root.join("config.toml")).unwrap();
        fs::write(
            root.join("config.toml"),
            format!("{saved_config}\n# later change\n"),
        )
        .unwrap();
        assert!(service
            .dispatch(
                serde_json::from_value(json!({
                    "action": "codex-restore", "configurationRevision": saved["revision"],
                }))
                .unwrap()
            )
            .is_err());
        fs::write(root.join("config.toml"), saved_config).unwrap();
        service
            .dispatch(
                serde_json::from_value(json!({
                    "action": "codex-restore", "configurationRevision": saved["revision"],
                }))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(
            fs::read_to_string(root.join("config.toml")).unwrap(),
            updated
        );
        assert!(!root.join("auth.json").exists());
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn commit_still_rejects_changes_after_the_save_snapshot() {
        let (base, root, data) = fixture();
        let original = Snapshot::read(&root).unwrap();
        let next = Snapshot {
            config: Some(b"model = 'next-model'\n".to_vec()),
            auth: Some(br#"{"OPENAI_API_KEY":"next-key"}"#.to_vec()),
        };
        fs::write(root.join("config.toml"), "# concurrent update\n").unwrap();
        assert!(commit(&root, &original, &next).is_err());
        assert_eq!(
            fs::read_to_string(root.join("config.toml")).unwrap(),
            "# concurrent update\n"
        );
        assert!(!root.join("auth.json").exists());
        assert!(!backup_path(&data).exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn new_configuration_restores_absent_files_and_invalid_inputs_never_write() {
        let (base, root, data) = fixture();
        let before = view(&root, &data).unwrap();
        for (url, key, model) in [
            (
                "https://user:dummy-password@example.invalid/v1",
                "dummy-key",
                "model",
            ),
            ("https://example.invalid/v1", "bad\nkey", "model"),
            ("https://example.invalid/v1", "dummy-key", ""),
        ] {
            let mut invalid = input(&before);
            invalid.base_url = url.into();
            invalid.api_key = key.into();
            invalid.model = model.into();
            let error = save(&root, &data, &invalid).unwrap_err();
            assert!(!error.contains("dummy-"));
            assert!(Snapshot::read(&root).unwrap().config.is_none());
            assert!(Snapshot::read(&root).unwrap().auth.is_none());
            assert!(!backup_path(&data).exists());
        }
        let mut empty = input(&before);
        empty.api_key.clear();
        assert!(save(&root, &data, &empty).is_err());
        let saved = save(&root, &data, &input(&before)).unwrap();
        restore(&root, &data, saved["revision"].as_str().unwrap()).unwrap();
        assert!(Snapshot::read(&root).unwrap().config.is_none());
        assert!(Snapshot::read(&root).unwrap().auth.is_none());
        fs::write(root.join("config.toml"), "key = \"dummy-secret\n").unwrap();
        let error = view(&root, &data).unwrap_err();
        assert!(!error.contains("dummy-secret"));
        fs::write(root.join("config.toml"), "").unwrap();
        fs::write(root.join("auth.json"), "{\"secret\":\"dummy-secret").unwrap();
        assert!(!view(&root, &data).unwrap_err().contains("dummy-secret"));
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn multiple_models_publish_immutable_catalogs_and_restore_previous_selection() {
        let (base, root, data) = fixture();
        fs::write(root.join("models_cache.json"), serde_json::to_vec(&json!({"models":[{
            "slug":"known-model","display_name":"Known", "description":"Known model",
            "base_instructions":"Keep Codex agent instructions", "shell_type":"unified_exec",
            "default_reasoning_level":"low", "supported_reasoning_levels":[{"effort":"low","description":"low"}],
            "default_reasoning_summary":"none", "support_verbosity":false,
            "truncation_policy":{"mode":"tokens","limit":10000},
            "experimental_supported_tools":[], "input_modalities":["text"]
        }]})).unwrap()).unwrap();
        let before = view(&root, &data).unwrap();
        let mut change = input(&before);
        change.model = "known-model".into();
        change.models = Some(vec![
            "known-model".into(),
            "custom/model".into(),
            "known-model".into(),
        ]);
        let saved = save(&root, &data, &change).unwrap();
        assert_eq!(saved["models"], json!(["known-model", "custom/model"]));
        assert_eq!(saved["model"], "known-model");
        assert_eq!(saved["reasoningEffort"], "high");
        let doc = parse_config(&Snapshot::read(&root).unwrap().config).unwrap();
        let first_catalog = PathBuf::from(setting(&doc, "model_catalog_json").unwrap());
        let catalog: Value = serde_json::from_slice(&fs::read(&first_catalog).unwrap()).unwrap();
        assert_eq!(catalog["models"][0]["display_name"], "Known");
        assert_eq!(
            catalog["models"][1]["base_instructions"],
            "Keep Codex agent instructions"
        );
        assert_eq!(catalog["models"][0]["default_reasoning_level"], "low");
        assert_eq!(catalog["models"][1]["default_reasoning_level"], "medium");
        let mut next = input(&saved);
        next.model = "custom/model".into();
        next.models = Some(vec!["custom/model".into()]);
        next.reasoning_effort = "low".into();
        let switched = save(&root, &data, &next).unwrap();
        assert_eq!(switched["models"], json!(["custom/model"]));
        assert!(first_catalog.exists());
        let restored = restore(&root, &data, switched["revision"].as_str().unwrap()).unwrap();
        assert_eq!(restored["models"], saved["models"]);
        assert_eq!(restored["model"], "known-model");
        assert_eq!(restored["reasoningEffort"], "high");
        assert_eq!(
            setting(
                &parse_config(&Snapshot::read(&root).unwrap().config).unwrap(),
                "model_catalog_json"
            ),
            first_catalog.to_str()
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn invalid_model_selections_do_not_modify_codex_config_or_credentials() {
        let (base, root, data) = fixture();
        let before = view(&root, &data).unwrap();
        let mut change = input(&before);
        change.models = Some(vec!["bad\nmodel".into()]);
        assert!(save(&root, &data, &change).is_err());
        assert!(!root.join("config.toml").exists());
        assert!(!root.join("auth.json").exists());
        assert!(!backup_path(&data).exists());
        fs::remove_dir_all(base).unwrap();
    }
}
