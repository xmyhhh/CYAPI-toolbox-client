//! Workspace-aware MCP catalog and local configuration management.
//!
//! The server only describes installable MCP servers. Every local check and
//! write happens here, scoped to the selected Codex / Claude Code / OpenCode
//! workspace. Writes preserve unrelated keys and comments, and removal requires
//! a matching receipt so an externally edited entry is never silently removed.
use crate::configuration::{no_links, stage};
use jsonc_parser::cst::{CstInputValue, CstObject, CstRootNode};
use jsonc_parser::ParseOptions;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use toml_edit::{value, Array, DocumentMut, InlineTable, Item, Table, Value as TomlValue};

type Result<T> = std::result::Result<T, String>;

const MAX_CATALOG: u64 = 100_000;
const MAX_CONFIG: u64 = 1_000_000;
const MAX_CLAUDE_CONFIG: u64 = 8_000_000;
const WORKSPACES: [&str; 3] = ["codex", "claude", "opencode"];

/// Unknown fields are ignored on purpose: a newer server may describe entries
/// this client does not understand, and that must not break the whole catalog.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub schema: u32,
    pub generated_at: String,
    pub servers: Vec<Descriptor>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub homepage: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub install_command: String,
    /// The install command of every platform. A client uses the command of
    /// the platform it runs on and falls back to `install_command`.
    #[serde(default)]
    pub install_commands: BTreeMap<String, String>,
    pub transport: Transport,
    #[serde(default)]
    pub workspaces: Vec<String>,
    /// The key used in the agent configuration. Defaults to the id, so an
    /// integration can keep its own prefixed name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_key: Option<String>,
    /// Server-declared minimum client version. Newer entries are skipped instead
    /// of invalidating the catalog.
    #[serde(default)]
    pub min_client_version: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum Transport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default = "default_startup_timeout")]
        startup_timeout_seconds: u64,
        #[serde(default = "default_tool_timeout")]
        tool_timeout_seconds: u64,
    },
    Http {
        url: String,
        #[serde(default)]
        token_source: Option<String>,
        /// Keep the credential in an environment variable instead of writing it
        /// into the agent configuration as a plaintext header.
        #[serde(default)]
        bearer_token_env_var: Option<String>,
    },
}

fn default_startup_timeout() -> u64 {
    30
}
fn default_tool_timeout() -> u64 {
    120
}

/// The key an entry uses in the agent configuration.
pub fn config_key(descriptor: &Descriptor) -> &str {
    descriptor.config_key.as_deref().unwrap_or(&descriptor.id)
}

pub struct Context {
    pub codex_config: PathBuf,
    pub claude_config: PathBuf,
    pub opencode_config: crate::opencode_config::Location,
    pub app_data: PathBuf,
}

#[derive(Clone, PartialEq)]
enum Snapshot {
    Toml {
        path: PathBuf,
        bytes: Option<Vec<u8>>,
    },
    Json {
        path: PathBuf,
        bytes: Option<Vec<u8>>,
    },
}

impl Snapshot {
    fn path(&self) -> &Path {
        match self {
            Self::Toml { path, .. } | Self::Json { path, .. } => path,
        }
    }
    fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Toml { bytes, .. } | Self::Json { bytes, .. } => bytes.as_deref(),
        }
    }
    fn revision(&self) -> String {
        let mut material = Vec::with_capacity(256);
        material.extend_from_slice(self.path().to_string_lossy().as_bytes());
        material.push(0);
        match self.bytes() {
            Some(bytes) => {
                material.push(1);
                material.extend_from_slice(bytes);
            }
            None => material.push(0),
        }
        crate::service::hash(&material)
    }
}

#[derive(Clone)]
struct LocalEntry {
    name: String,
    transport: String,
    target: String,
    enabled: bool,
    hash: String,
    value: Value,
    layout: OpenCodeLayout,
    duplicate: bool,
}

#[derive(Serialize, Deserialize)]
struct Receipt {
    schema: u32,
    workspace: String,
    config: PathBuf,
    name: String,
    hash: String,
}

fn valid_workspace(value: &str) -> Result<&str> {
    WORKSPACES
        .into_iter()
        .find(|item| *item == value)
        .ok_or_else(|| "MCP 工作区无效".into())
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn clean_text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

/// Compare dotted numeric versions such as "0.9.1". Missing parts count as zero.
fn version_at_least(value: &str, minimum: &str) -> bool {
    let parse = |text: &str| -> Vec<u32> {
        text.split('.')
            .map(|part| part.parse::<u32>().unwrap_or(0))
            .collect()
    };
    let mut left = parse(value);
    let mut right = parse(minimum);
    while left.len() < right.len() {
        left.push(0);
    }
    while right.len() < left.len() {
        right.push(0);
    }
    left >= right
}

/// The environment variable name may only hold the token, never the token itself.
fn env_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        && !value.starts_with('_')
}

fn validate_descriptor(server: &Descriptor, ids: &mut HashSet<String>, local: bool) -> Result<()> {
    if !identifier(&server.id)
        || !ids.insert(server.id.clone())
        || !clean_text(&server.name, 100)
        || !clean_text(&server.version, 40)
        || !clean_text(&server.description, 800)
        || (!local && !server.homepage.starts_with("https://"))
        || server.homepage.len() > 240
        || server.capabilities.len() > 12
        || server.steps.len() > 12
        || server.install_command.len() > 2_000
        || server.install_command.chars().any(char::is_control)
        || server.install_commands.len() > 8
        || server.install_commands.iter().any(|(platform, command)| {
            !crate::platform::known(platform)
                || command.len() > 2_000
                || command.chars().any(char::is_control)
        })
        || server
            .config_key
            .as_ref()
            .is_some_and(|key| !identifier(key))
    {
        return Err("MCP 目录包含无效条目".into());
    }
    if server.capabilities.iter().any(|item| !clean_text(item, 80))
        || server.steps.iter().any(|item| !clean_text(item, 600))
    {
        return Err("MCP 目录包含无效说明".into());
    }
    let mut workspaces = HashSet::new();
    if server.workspaces.is_empty()
        || server.workspaces.len() > WORKSPACES.len()
        || server
            .workspaces
            .iter()
            .any(|item| !WORKSPACES.contains(&item.as_str()) || !workspaces.insert(item))
    {
        return Err("MCP 目录声明的 Agent 无效".into());
    }
    match &server.transport {
        Transport::Stdio {
            command,
            args,
            startup_timeout_seconds,
            tool_timeout_seconds,
        } => {
            if command.is_empty()
                || command.len() > 240
                || command.chars().any(char::is_control)
                || args.len() > 24
                || args
                    .iter()
                    .any(|arg| arg.len() > 1_000 || arg.contains('\0'))
                || !(5..=600).contains(startup_timeout_seconds)
                || !(5..=3_600).contains(tool_timeout_seconds)
            {
                return Err("MCP stdio 启动声明无效".into());
            }
        }
        Transport::Http {
            url,
            token_source,
            bearer_token_env_var,
        } => {
            validate_http_url(url)?;
            if let Some(source) = token_source {
                validate_token_source(source)?;
            }
            if let Some(name) = bearer_token_env_var {
                if !env_name(name) {
                    return Err("MCP 令牌环境变量名无效".into());
                }
            }
        }
    }
    Ok(())
}

fn validate_catalog_bytes(bytes: &[u8]) -> Result<Catalog> {
    if bytes.len() as u64 > MAX_CATALOG {
        return Err("MCP 目录过大".into());
    }
    let mut catalog: Catalog =
        serde_json::from_slice(bytes).map_err(|_| "MCP 目录格式无效".to_string())?;
    if ![1, 2].contains(&catalog.schema) || catalog.servers.len() > 50 {
        return Err("不支持的 MCP 目录格式".into());
    }
    if !clean_text(&catalog.generated_at, 80) {
        return Err("MCP 目录生成时间无效".into());
    }
    let mut ids = HashSet::new();
    for server in &catalog.servers {
        validate_descriptor(server, &mut ids, false)?;
    }
    // Entries the running client is too old for are skipped, not rejected.
    catalog.servers.retain(|server| {
        server
            .min_client_version
            .as_ref()
            .is_none_or(|minimum| version_at_least(env!("CARGO_PKG_VERSION"), minimum))
    });
    // The install command of the platform the client runs on replaces the
    // generic one, so the user always sees the command that fits the machine.
    let platform = crate::platform::current();
    for server in &mut catalog.servers {
        if let Some(command) = server.install_commands.get(platform) {
            server.install_command = command.clone();
        }
    }
    Ok(catalog)
}

/// Token sources are declarative so a new MCP never needs a client change.
fn validate_token_source(source: &str) -> Result<()> {
    if source == "orbit-ssh" {
        return Ok(());
    }
    if let Some(path) = source.strip_prefix("file:") {
        if !path.is_empty()
            && path.len() <= 240
            && !path.contains('\\')
            && !path
                .split('/')
                .any(|part| part.is_empty() || part.starts_with('.'))
        {
            return Ok(());
        }
    }
    if let Some(name) = source.strip_prefix("env:") {
        if env_name(name) {
            return Ok(());
        }
    }
    Err("不支持的 MCP 令牌来源".into())
}

fn validate_http_url(raw: &str) -> Result<()> {
    let url = reqwest::Url::parse(raw).map_err(|_| "MCP HTTP 地址无效".to_string())?;
    let host = url.host_str().unwrap_or("");
    let loopback = matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]");
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.scheme() == "http" && !loopback
    {
        return Err("MCP HTTP 地址必须是 HTTPS，或本机回环 HTTP".into());
    }
    Ok(())
}

/// Build a user defined entry from the values the form submitted.
pub fn local_descriptor(values: &std::collections::BTreeMap<String, Value>) -> Result<Descriptor> {
    let text = |key: &str| {
        values
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let id = text("id");
    let name = text("name");
    let description = text("description");
    let transport = if text("transportType") == "http" {
        Transport::Http {
            url: text("url"),
            token_source: None,
            bearer_token_env_var: None,
        }
    } else {
        let args = text("args")
            .split_whitespace()
            .map(str::to_string)
            .collect();
        Transport::Stdio {
            command: text("command"),
            args,
            startup_timeout_seconds: 30,
            tool_timeout_seconds: 120,
        }
    };
    let mut workspaces: Vec<String> = WORKSPACES
        .iter()
        .filter(|workspace| {
            values
                .get(**workspace)
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .map(|workspace| workspace.to_string())
        .collect();
    if workspaces.is_empty() {
        workspaces.push("codex".into());
    }
    Ok(Descriptor {
        name: if name.is_empty() { id.clone() } else { name },
        version: "本机自定义".into(),
        description: if description.is_empty() {
            "本机自定义的 MCP 服务器".into()
        } else {
            description
        },
        homepage: String::new(),
        capabilities: vec![],
        steps: vec![],
        install_command: String::new(),
        install_commands: BTreeMap::new(),
        transport,
        workspaces,
        min_client_version: None,
        config_key: None,
        id,
    })
}

/// The entries the user defined on this machine.
pub fn local_entries(data: &Path) -> Result<Vec<Descriptor>> {
    let path = local_path(data);
    let Some(bytes) = fs::read(&path).ok() else {
        return Ok(Vec::new());
    };
    if bytes.len() as u64 > MAX_CATALOG {
        return Err("本机 MCP 条目过多".into());
    }
    let entries: Vec<Descriptor> =
        serde_json::from_slice(&bytes).map_err(|_| "本机 MCP 条目格式无效".to_string())?;
    let mut ids = HashSet::new();
    for entry in &entries {
        validate_descriptor(entry, &mut ids, true)?;
    }
    Ok(entries)
}

fn local_path(data: &Path) -> PathBuf {
    data.join("mcp/local.json")
}

/// Store or replace a user defined entry.
pub fn save_local(data: &Path, descriptor: Descriptor) -> Result<()> {
    let mut entries = local_entries(data)?;
    let mut ids = HashSet::new();
    validate_descriptor(&descriptor, &mut ids, true)?;
    entries.retain(|entry| entry.id != descriptor.id);
    entries.push(descriptor);
    write_local(data, &entries)
}

/// Remove a user defined entry.
pub fn remove_local(data: &Path, id: &str) -> Result<()> {
    let mut entries = local_entries(data)?;
    let before = entries.len();
    entries.retain(|entry| entry.id != id);
    if entries.len() == before {
        return Err("本机没有此 MCP 条目".into());
    }
    write_local(data, &entries)
}

fn write_local(data: &Path, entries: &[Descriptor]) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(entries).map_err(|_| "无法生成本机 MCP 条目".to_string())?;
    if bytes.len() as u64 > MAX_CATALOG {
        return Err("本机 MCP 条目过多".into());
    }
    let path = local_path(data);
    fs::create_dir_all(path.parent().ok_or("路径无效")?).map_err(|_| "无法创建本机 MCP 目录")?;
    let temp = stage(&path, &bytes)?;
    fs::rename(&temp, &path).map_err(|_| "无法保存本机 MCP 条目".to_string())
}

/// The merged entries with the local port and token resolved.
fn resolved_servers(data: &Path, context: &Context) -> Result<Vec<Descriptor>> {
    let (catalog, _) = available_catalog(data);
    let mut catalog = resolved_catalog(context, catalog);
    for local in local_entries(data)? {
        if !catalog.servers.iter().any(|server| server.id == local.id) {
            catalog.servers.push(local);
        }
    }
    Ok(catalog.servers)
}
pub fn cache(data: &Path, bytes: &[u8]) -> Result<()> {
    validate_catalog_bytes(bytes)?;
    let directory = data.join("mcp");
    fs::create_dir_all(&directory).map_err(|_| "无法创建 MCP 缓存目录")?;
    let path = directory.join("catalog.json");
    let temp = stage(&path, bytes)?;
    fs::rename(&temp, &path).map_err(|_| {
        let _ = fs::remove_file(&temp);
        "无法写入 MCP 目录缓存".to_string()
    })
}

pub fn catalog(data: &Path) -> Result<Catalog> {
    let path = data.join("mcp/catalog.json");
    no_links(&path)?;
    match fs::metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("MCP 目录尚未下载，请从服务端获取".into())
        }
        Ok(meta) if !meta.is_file() || meta.len() > MAX_CATALOG => {
            return Err("MCP 目录缓存无效".into())
        }
        Ok(_) => {}
        Err(_) => return Err("无法读取 MCP 目录缓存".into()),
    }
    validate_catalog_bytes(&fs::read(path).map_err(|_| "无法读取 MCP 目录缓存")?)
}

// Local configuration and user-defined entries remain usable before the first
// catalog download, or when a cached catalog can no longer be read.
fn available_catalog(data: &Path) -> (Catalog, Option<String>) {
    match catalog(data) {
        Ok(catalog) => (catalog, None),
        Err(error) => (
            Catalog {
                schema: 2,
                generated_at: String::new(),
                servers: Vec::new(),
            },
            Some(error),
        ),
    }
}

fn read_file(path: &Path, limit: u64) -> Result<Option<Vec<u8>>> {
    no_links(path)?;
    match fs::metadata(path) {
        Ok(meta) if !meta.is_file() || meta.len() > limit => {
            return Err("MCP 配置文件过大或不是普通文件".into())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法读取 MCP 配置文件".into()),
    }
    fs::read(path)
        .map(Some)
        .map_err(|_| "无法读取 MCP 配置文件".into())
}

fn snapshot(context: &Context, workspace: &str) -> Result<Snapshot> {
    match workspace {
        "codex" => Ok(Snapshot::Toml {
            path: context.codex_config.clone(),
            bytes: read_file(&context.codex_config, MAX_CONFIG)?,
        }),
        "claude" => Ok(Snapshot::Json {
            path: context.claude_config.clone(),
            bytes: read_file(&context.claude_config, MAX_CLAUDE_CONFIG)?,
        }),
        "opencode" => {
            let path = context.opencode_config.resolve()?;
            Ok(Snapshot::Json {
                bytes: read_file(&path, MAX_CONFIG)?,
                path,
            })
        }
        _ => Err("MCP 工作区无效".into()),
    }
}

fn text<'a>(bytes: Option<&'a [u8]>, label: &str) -> Result<&'a str> {
    let text = std::str::from_utf8(bytes.unwrap_or_default())
        .map_err(|_| format!("{label}不是 UTF-8 文件"))?;
    Ok(text.trim_start_matches('\u{feff}'))
}

fn parse_toml(snapshot: &Snapshot) -> Result<DocumentMut> {
    let text = text(snapshot.bytes(), "Codex 配置")?;
    if text.trim().is_empty() {
        return Ok(DocumentMut::default());
    }
    text.parse::<DocumentMut>()
        .map_err(|_| "Codex 配置格式无效，请修复后重新读取".to_string())
}

fn parse_json_value(snapshot: &Snapshot, label: &str) -> Result<Value> {
    let text = text(snapshot.bytes(), label)?;
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    let value: Value = jsonc_parser::parse_to_serde_value(text, &ParseOptions::default())
        .map_err(|_| format!("{label}格式无效，请修复后重新读取"))?;
    if !value.is_object() {
        return Err(format!("{label}必须是 JSON 对象"));
    }
    Ok(value)
}

fn parse_cst(snapshot: &Snapshot, label: &str) -> Result<CstRootNode> {
    let text = text(snapshot.bytes(), label)?;
    if text.trim().is_empty() {
        return CstRootNode::parse("{}", &ParseOptions::default())
            .map_err(|_| format!("{label}格式无效"));
    }
    let root = CstRootNode::parse(text, &ParseOptions::default())
        .map_err(|_| format!("{label}格式无效，请修复后重新读取"))?;
    if root.value().is_some() && root.object_value().is_none() {
        return Err(format!("{label}必须是 JSON 对象"));
    }
    Ok(root)
}

fn has_bom(bytes: Option<&[u8]>) -> bool {
    bytes.is_some_and(|bytes| bytes.starts_with(&[0xef, 0xbb, 0xbf]))
}

fn finish_json(root: &CstRootNode, bom: bool, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if bom {
        bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    bytes.extend_from_slice(root.to_string().as_bytes());
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    if bytes.len() as u64 > limit {
        return Err("生成的 MCP 配置文件过大".into());
    }
    Ok(bytes)
}

fn serialize_toml(doc: &DocumentMut) -> Vec<u8> {
    let mut bytes = doc.to_string().into_bytes();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    bytes
}

/// Normalize a TOML value into JSON so the entry hash depends on the content
/// instead of the formatting the user happens to use.
fn toml_json_value(value: &TomlValue) -> Value {
    match value {
        TomlValue::String(text) => json!(text.value()),
        TomlValue::Integer(number) => json!(number.value()),
        TomlValue::Float(number) => json!(number.value()),
        TomlValue::Boolean(flag) => json!(flag.value()),
        TomlValue::Datetime(stamp) => json!(stamp.value().to_string()),
        TomlValue::Array(items) => Value::Array(items.iter().map(toml_json_value).collect()),
        TomlValue::InlineTable(table) => {
            let mut map = Map::new();
            for (key, item) in table.iter() {
                map.insert(key.to_string(), toml_json_value(item));
            }
            Value::Object(map)
        }
    }
}

fn toml_json_table(table: &Table) -> Value {
    let mut map = Map::new();
    for (key, value) in table.iter() {
        map.insert(key.to_string(), toml_json_item(value));
    }
    Value::Object(map)
}

fn toml_json_item(item: &Item) -> Value {
    match item {
        Item::Value(value) => toml_json_value(value),
        Item::Table(table) => toml_json_table(table),
        Item::ArrayOfTables(array) => Value::Array(array.iter().map(toml_json_table).collect()),
        Item::None => Value::Null,
    }
}

fn codex_entry_value(item: &Item) -> Value {
    let mut map = Map::new();
    if let Some(command) = item.get("command").and_then(Item::as_str) {
        map.insert("command".into(), json!(command));
    }
    if let Some(args) = item.get("args").and_then(Item::as_array) {
        map.insert(
            "args".into(),
            Value::Array(
                args.iter()
                    .filter_map(|arg| arg.as_str().map(|item| json!(item)))
                    .collect(),
            ),
        );
    }
    if let Some(url) = item.get("url").and_then(Item::as_str) {
        map.insert("url".into(), json!(url));
    }
    if let Some(enabled) = item.get("enabled").and_then(Item::as_bool) {
        map.insert("enabled".into(), json!(enabled));
    }
    if let Some(headers) = item.get("http_headers").and_then(Item::as_table_like) {
        let mut values = Map::new();
        for (key, header) in headers.iter() {
            if let Some(header) = header.as_str() {
                values.insert(key.to_string(), json!(header));
            }
        }
        map.insert("http_headers".into(), Value::Object(values));
    }
    if let Some(env) = item.get("env").and_then(Item::as_table_like) {
        let mut values = Map::new();
        for (key, entry) in env.iter() {
            if let Some(entry) = entry.as_str() {
                values.insert(key.to_string(), json!(entry));
            }
        }
        map.insert("env".into(), Value::Object(values));
    }
    Value::Object(map)
}

fn codex_entries(doc: &DocumentMut) -> Vec<LocalEntry> {
    let mut entries = Vec::new();
    let Some(servers) = doc.get("mcp_servers").and_then(Item::as_table_like) else {
        return entries;
    };
    for (name, item) in servers.iter() {
        let value = codex_entry_value(item);
        let transport = if value.get("url").is_some() {
            "http"
        } else if value.get("command").is_some() {
            "stdio"
        } else {
            "unknown"
        };
        let target = value
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                let command = value.get("command").and_then(Value::as_str)?;
                let args = value
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                Some(if args.is_empty() {
                    command.to_string()
                } else {
                    format!("{command} {args}")
                })
            })
            .unwrap_or_default();
        entries.push(LocalEntry {
            name: name.to_string(),
            transport: transport.into(),
            target,
            enabled: value
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            // The hash covers the content, not the layout, so reformatting the
            // file does not look like an external edit.
            hash: crate::service::hash(
                &serde_json::to_vec(&toml_json_item(item)).unwrap_or_default(),
            ),
            value,
            layout: OpenCodeLayout::Flat,
            duplicate: false,
        });
    }
    entries
}

/// Both layouts are accepted by OpenCode. Preserve the shape of each entry
/// rather than guessing it from the installed CLI version.
#[derive(Clone, Copy, PartialEq)]
enum OpenCodeLayout {
    Flat,
    Servers,
}

fn new_entry_layout(snapshot: &Snapshot, workspace: &str) -> Result<OpenCodeLayout> {
    if workspace != "opencode" {
        return Ok(OpenCodeLayout::Flat);
    }
    // An existing `mcp.servers` group decides the layout.
    if parse_json_value(snapshot, "OpenCode 配置")?
        .pointer("/mcp/servers")
        .is_some_and(Value::is_object)
    {
        return Ok(OpenCodeLayout::Servers);
    }
    if parse_json_value(snapshot, "OpenCode 配置")?
        .get("mcp")
        .is_some_and(Value::is_object)
    {
        return Ok(OpenCodeLayout::Flat);
    }
    // Otherwise the installed CLI decides, so a fresh install on OpenCode V2
    // still writes the shape it understands.
    if crate::opencode_config::major_version() >= 2 {
        return Ok(OpenCodeLayout::Servers);
    }
    Ok(OpenCodeLayout::Flat)
}

/// The key path that holds the MCP entries of a workspace.
fn json_group_path(workspace: &str, layout: OpenCodeLayout) -> Vec<&'static str> {
    match workspace {
        "claude" => vec!["mcpServers"],
        _ => match layout {
            OpenCodeLayout::Servers => vec!["mcp", "servers"],
            OpenCodeLayout::Flat => vec!["mcp"],
        },
    }
}

fn json_enabled(value: &Value, workspace: &str, layout: OpenCodeLayout) -> bool {
    if workspace == "claude" {
        return true;
    }
    match layout {
        OpenCodeLayout::Flat => value
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        OpenCodeLayout::Servers => !value
            .get("disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

fn json_group_entries(doc: &Value, workspace: &str, layout: OpenCodeLayout) -> Vec<LocalEntry> {
    let mut entries = Vec::new();
    let mut current = doc;
    for key in json_group_path(workspace, layout) {
        match current.get(key) {
            Some(value) => current = value,
            None => return entries,
        }
    }
    let Some(group) = current.as_object() else {
        return entries;
    };
    for (name, value) in group {
        if workspace == "opencode" && layout == OpenCodeLayout::Flat && name == "servers" {
            continue;
        }
        let transport = if value.get("url").is_some() {
            "http"
        } else if value.get("command").is_some() {
            "stdio"
        } else {
            "unknown"
        };
        let target = value
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                let command = value.get("command")?;
                if let Some(command) = command.as_str() {
                    let args = value
                        .get("args")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .unwrap_or_default();
                    Some(if args.is_empty() {
                        command.to_string()
                    } else {
                        format!("{command} {args}")
                    })
                } else {
                    command.as_array().map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                }
            })
            .unwrap_or_default();
        entries.push(LocalEntry {
            name: name.to_string(),
            transport: transport.into(),
            target,
            enabled: json_enabled(value, workspace, layout),
            hash: crate::service::hash(&serde_json::to_vec(value).unwrap_or_default()),
            value: value.clone(),
            layout,
            duplicate: false,
        });
    }
    entries
}

fn json_entries(doc: &Value, workspace: &str) -> Vec<LocalEntry> {
    let flat = json_group_entries(doc, workspace, OpenCodeLayout::Flat);
    if workspace != "opencode" {
        return flat;
    }
    let mut entries = json_group_entries(doc, workspace, OpenCodeLayout::Servers);
    for mut entry in flat {
        if let Some(nested) = entries.iter_mut().find(|nested| nested.name == entry.name) {
            // OpenCode gives flat entries precedence. Hash both definitions so
            // an old receipt can never authorize editing only one of them.
            entry.hash = crate::service::hash(format!("{}:{}", nested.hash, entry.hash).as_bytes());
            entry.duplicate = true;
            *nested = entry;
        } else {
            entries.push(entry);
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

fn require_unique(entry: &LocalEntry) -> Result<()> {
    if entry.duplicate {
        return Err("同名 MCP 同时存在于 mcp 和 mcp.servers，请先在配置文件中合并重复项".into());
    }
    Ok(())
}

fn entries(snapshot: &Snapshot, workspace: &str) -> Result<Vec<LocalEntry>> {
    match snapshot {
        Snapshot::Toml { .. } => Ok(codex_entries(&parse_toml(snapshot)?)),
        Snapshot::Json { .. } => Ok(json_entries(
            &parse_json_value(
                snapshot,
                if workspace == "claude" {
                    "Claude Code 配置"
                } else {
                    "OpenCode 配置"
                },
            )?,
            workspace,
        )),
    }
}

/// The revision of a single MCP entry. An action only writes the entry it
/// names, so a background rewrite of the agent configuration must not
/// invalidate it. The agent rewrites volatile values in its own file, for
/// example the pipe of a restarted runtime, and those changes belong to
/// another entry.
fn entry_revision(entries: &[LocalEntry], path: &Path, key: &str) -> String {
    let mut material = Vec::new();
    material.extend_from_slice(path.to_string_lossy().as_bytes());
    material.push(0);
    material.extend_from_slice(key.as_bytes());
    material.push(0);
    match entries.iter().find(|entry| entry.name == key) {
        Some(entry) => {
            material.push(1);
            material.extend_from_slice(entry.hash.as_bytes());
        }
        None => material.extend_from_slice(b"absent"),
    }
    crate::service::hash(&material)
}

fn clean_token(value: String) -> Result<Option<String>> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 512
        || value
            .chars()
            .any(|item| item.is_control() || item.is_whitespace())
    {
        return Err("本机 MCP 令牌格式无效".into());
    }
    Ok(Some(value.to_string()))
}

fn token(context: &Context, source: &str) -> Result<Option<String>> {
    if let Some(name) = source.strip_prefix("env:") {
        return match std::env::var(name) {
            Ok(value) => clean_token(value),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err("无法读取本机 MCP 令牌".into()),
        };
    }
    let relative = match source {
        "orbit-ssh" => "OrbitSSH/mcp-token",
        _ => match source.strip_prefix("file:") {
            Some(path) => path,
            None => return Err("不支持的 MCP 令牌来源".into()),
        },
    };
    let path = context.app_data.join(relative);
    no_links(&path)?;
    match fs::read_to_string(&path) {
        Ok(value) => clean_token(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("无法读取本机 MCP 令牌".into()),
    }
}

/// The toolbox has to read a local token only when it must write the token
/// into the agent configuration itself.
fn needs_local_token(descriptor: &Descriptor, workspace: &str) -> bool {
    match &descriptor.transport {
        Transport::Http {
            token_source: Some(_),
            bearer_token_env_var: Some(_),
            ..
        } => {
            // Only Codex and OpenCode can resolve the credential from the
            // environment; Claude Code needs the value written out.
            workspace == "claude"
        }
        Transport::Http {
            token_source: Some(_),
            ..
        } => true,
        _ => false,
    }
}

fn descriptor_token(context: &Context, descriptor: &Descriptor) -> Result<Option<String>> {
    match &descriptor.transport {
        Transport::Http {
            token_source: Some(source),
            ..
        } => token(context, source),
        _ => Ok(None),
    }
}

fn orbit_port(context: &Context) -> Option<u16> {
    let path = context.app_data.join("OrbitSSH/workspace.json");
    no_links(&path).ok()?;
    let meta = fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > 2_000_000 {
        return None;
    }
    let state: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    let port = state
        .get("preferences")
        .and_then(|value| value.get("mcpPort"))
        .and_then(Value::as_u64)?;
    u16::try_from(port).ok().filter(|port| *port >= 1024)
}

fn resolved_descriptor(context: &Context, descriptor: &Descriptor) -> Descriptor {
    let mut descriptor = descriptor.clone();
    if let Transport::Http {
        url,
        token_source: Some(source),
        ..
    } = &mut descriptor.transport
    {
        if source == "orbit-ssh" {
            if let (Some(port), Ok(mut endpoint)) = (orbit_port(context), reqwest::Url::parse(url))
            {
                if endpoint.set_port(Some(port)).is_ok() {
                    *url = endpoint.to_string();
                }
            }
        }
    }
    descriptor
}

fn resolved_catalog(context: &Context, mut catalog: Catalog) -> Catalog {
    catalog.servers = catalog
        .servers
        .iter()
        .map(|descriptor| resolved_descriptor(context, descriptor))
        .collect();
    catalog
}

fn header<'a>(value: &'a Value, workspace: &str) -> Option<&'a str> {
    let headers = if workspace == "codex" {
        value.get("http_headers")
    } else {
        value.get("headers")
    }?
    .as_object()?;
    headers.iter().find_map(|(name, value)| {
        if name.eq_ignore_ascii_case("authorization") {
            value.as_str()
        } else {
            None
        }
    })
}

fn command_parts<'a>(value: &'a Value, workspace: &str) -> Option<(&'a str, Vec<&'a str>)> {
    if workspace == "opencode" {
        let command = value.get("command")?.as_array()?;
        let first = command.first()?.as_str()?;
        Some((
            first,
            command.iter().skip(1).filter_map(Value::as_str).collect(),
        ))
    } else {
        let command = value.get("command")?.as_str()?;
        let args = value
            .get("args")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        Some((command, args))
    }
}

fn core_matches(entry: &LocalEntry, descriptor: &Descriptor, workspace: &str) -> bool {
    match &descriptor.transport {
        Transport::Stdio { command, args, .. } => command_parts(&entry.value, workspace)
            .is_some_and(|(current, current_args)| {
                current == command
                    && current_args == args.iter().map(String::as_str).collect::<Vec<_>>()
            }),
        Transport::Http { url, .. } => entry.value.get("url").and_then(Value::as_str) == Some(url),
    }
}

/// The credential an entry asks the agent to read from the environment.
fn env_credential(entry: &LocalEntry, workspace: &str) -> Option<String> {
    if workspace == "codex" {
        return entry
            .value
            .get("bearer_token_env_var")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    let authorization = header(&entry.value, workspace)?;
    let rest = authorization.strip_prefix("Bearer ")?;
    let name = rest.strip_prefix("{env:")?.strip_suffix('}')?;
    Some(name.to_string())
}

fn token_matches(
    entry: &LocalEntry,
    descriptor: &Descriptor,
    workspace: &str,
    token: Option<&str>,
) -> bool {
    match &descriptor.transport {
        Transport::Http {
            token_source: Some(_),
            bearer_token_env_var: Some(name),
            ..
        } => env_credential(entry, workspace).is_some_and(|current| current == *name),
        Transport::Http {
            token_source: Some(_),
            ..
        } => token.is_some_and(|token| {
            header(&entry.value, workspace) == Some(format!("Bearer {token}").as_str())
        }),
        _ => true,
    }
}

fn receipt_path(data: &Path, workspace: &str, name: &str) -> PathBuf {
    data.join("mcp/receipts")
        .join(format!("{workspace}-{name}.json"))
}

fn read_receipt(
    data: &Path,
    workspace: &str,
    config: &Path,
    name: &str,
) -> Result<Option<Receipt>> {
    let path = receipt_path(data, workspace, name);
    no_links(&path)?;
    match fs::metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(meta) if !meta.is_file() || meta.len() > 20_000 => return Err("MCP 管理记录无效".into()),
        Ok(_) => {}
        Err(_) => return Err("无法读取 MCP 管理记录".into()),
    }
    let receipt: Receipt =
        serde_json::from_slice(&fs::read(&path).map_err(|_| "无法读取 MCP 管理记录")?)
            .map_err(|_| "MCP 管理记录无效")?;
    if receipt.schema != 1
        || receipt.workspace != workspace
        || receipt.config != config
        || receipt.name != name
        || receipt.hash.len() != 64
    {
        return Ok(None);
    }
    Ok(Some(receipt))
}

fn write_receipt(
    data: &Path,
    workspace: &str,
    config: &Path,
    name: &str,
    hash: &str,
) -> Result<()> {
    let receipt = Receipt {
        schema: 1,
        workspace: workspace.into(),
        config: config.into(),
        name: name.into(),
        hash: hash.into(),
    };
    let path = receipt_path(data, workspace, name);
    let bytes = serde_json::to_vec_pretty(&receipt).map_err(|_| "无法生成 MCP 管理记录")?;
    let temp = stage(&path, &bytes)?;
    fs::rename(&temp, &path).map_err(|_| {
        let _ = fs::remove_file(&temp);
        "无法写入 MCP 管理记录".to_string()
    })
}

fn delete_receipt(data: &Path, workspace: &str, name: &str) -> Result<()> {
    match fs::remove_file(receipt_path(data, workspace, name)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("无法删除 MCP 管理记录".into()),
    }
}

fn codex_table(servers: &mut dyn toml_edit::TableLike, name: &str) -> Result<Table> {
    match servers.remove(name) {
        Some(Item::Table(table)) => Ok(table),
        Some(Item::Value(TomlValue::InlineTable(table))) => Ok(table.into_table()),
        Some(_) => Err("同名 MCP 配置不是表结构，请手动检查".into()),
        None => Ok(Table::new()),
    }
}

fn codex_servers(doc: &mut DocumentMut) -> Result<&mut dyn toml_edit::TableLike> {
    if doc.get("mcp_servers").is_none() {
        let mut table = Table::new();
        table.set_implicit(true);
        doc.insert("mcp_servers", Item::Table(table));
    }
    if doc
        .get("mcp_servers")
        .and_then(Item::as_inline_table)
        .is_some()
    {
        let table = doc
            .get("mcp_servers")
            .and_then(Item::as_inline_table)
            .cloned()
            .ok_or("Codex mcp_servers 配置无效")?
            .into_table();
        doc.insert("mcp_servers", Item::Table(table));
    }
    doc.get_mut("mcp_servers")
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| "Codex mcp_servers 必须是表".to_string())
}

fn write_codex_entry(
    snapshot: &Snapshot,
    descriptor: &Descriptor,
    token: Option<&str>,
) -> Result<Vec<u8>> {
    let mut doc = parse_toml(snapshot)?;
    let mut table = {
        let servers = codex_servers(&mut doc)?;
        codex_table(servers, config_key(descriptor))?
    };
    table.insert("enabled", value(true));
    match &descriptor.transport {
        Transport::Stdio {
            command,
            args,
            startup_timeout_seconds,
            tool_timeout_seconds,
        } => {
            table.remove("url");
            table.remove("http_headers");
            table.remove("bearer_token_env_var");
            table.remove("http_headers_helper");
            table.insert("command", value(command));
            let mut array = Array::new();
            for arg in args {
                array.push(arg.as_str());
            }
            table.insert("args", value(array));
            table.insert(
                "startup_timeout_sec",
                value(*startup_timeout_seconds as i64),
            );
            table.insert("tool_timeout_sec", value(*tool_timeout_seconds as i64));
        }
        Transport::Http {
            url,
            bearer_token_env_var,
            ..
        } => {
            table.remove("command");
            table.remove("args");
            table.insert("url", value(url));
            match bearer_token_env_var {
                // The agent reads the credential from the environment, so it never
                // lands in the configuration file. An existing helper or header
                // written by the user is kept untouched.
                Some(name) => {
                    table.insert("bearer_token_env_var", value(name.as_str()));
                }
                None if token.is_some() => {
                    let token = token.unwrap();
                    let mut headers = InlineTable::new();
                    if let Some(existing) =
                        table.get("http_headers").and_then(Item::as_inline_table)
                    {
                        for (key, value) in existing.iter() {
                            headers.insert(key, value.clone());
                        }
                    }
                    if let Some(existing) = table.get("http_headers").and_then(Item::as_table_like)
                    {
                        for (key, value) in existing.iter() {
                            if let Some(text) = value.as_str() {
                                headers.insert(key, TomlValue::from(text));
                            }
                        }
                    }
                    headers.insert("Authorization", TomlValue::from(format!("Bearer {token}")));
                    table.insert("http_headers", Item::Value(TomlValue::InlineTable(headers)));
                }
                None => {}
            }
        }
    }
    let servers = codex_servers(&mut doc)?;
    servers.insert(config_key(descriptor), Item::Table(table));
    Ok(serialize_toml(&doc))
}

fn write_json_entry(
    snapshot: &Snapshot,
    descriptor: &Descriptor,
    workspace: &str,
    layout: OpenCodeLayout,
    token: Option<&str>,
) -> Result<Vec<u8>> {
    let label = if workspace == "claude" {
        "Claude Code 配置"
    } else {
        "OpenCode 配置"
    };
    let root = parse_cst(snapshot, label)?;
    let object = root
        .object_value_or_create()
        .ok_or_else(|| format!("{label}必须是 JSON 对象"))?;
    // The group may be nested, for example `mcp.servers` in OpenCode V2.
    let path = json_group_path(workspace, layout);
    let mut group = object;
    for (index, key) in path.iter().enumerate() {
        group = group
            .object_value_or_create(key)
            .ok_or_else(|| format!("{label}的 {} 必须是 JSON 对象", path[..=index].join(".")))?;
    }
    let entry = group
        .object_value_or_create(config_key(descriptor))
        .ok_or_else(|| format!("MCP 配置 {} 必须是 JSON 对象", config_key(descriptor)))?;
    match &descriptor.transport {
        Transport::Stdio { command, args, .. } => {
            if workspace == "opencode" {
                set_string(&entry, "type", "local");
                let mut values = vec![CstInputValue::String(command.clone())];
                values.extend(args.iter().cloned().map(CstInputValue::String));
                set_value(&entry, "command", CstInputValue::Array(values));
                set_enabled(&entry, workspace, layout, true);
                remove_if_present(&entry, "args");
                remove_if_present(&entry, "url");
                remove_if_present(&entry, "headers");
            } else {
                set_string(&entry, "type", "stdio");
                set_string(&entry, "command", command);
                set_value(
                    &entry,
                    "args",
                    CstInputValue::Array(args.iter().cloned().map(CstInputValue::String).collect()),
                );
                remove_if_present(&entry, "url");
                remove_if_present(&entry, "headers");
            }
        }
        Transport::Http {
            url,
            bearer_token_env_var,
            ..
        } => {
            if workspace == "opencode" {
                set_string(&entry, "type", "remote");
                set_string(&entry, "url", url);
                set_enabled(&entry, workspace, layout, true);
                set_value(&entry, "oauth", CstInputValue::Bool(false));
                remove_if_present(&entry, "command");
                remove_if_present(&entry, "args");
                remove_if_present(&entry, "environment");
            } else {
                set_string(&entry, "type", "http");
                set_string(&entry, "url", url);
                remove_if_present(&entry, "command");
                remove_if_present(&entry, "args");
            }
            // Public HTTP servers have no credential to write. Keep any
            // user-defined headers and allow the anonymous connection.
            if bearer_token_env_var.is_some() || token.is_some() {
                let headers = entry
                    .object_value_or_create("headers")
                    .ok_or("MCP headers 必须是 JSON 对象")?;
                match bearer_token_env_var {
                    // OpenCode resolves `{env:NAME}` itself, so the token never
                    // reaches the configuration file.
                    Some(name) if workspace == "opencode" => {
                        set_string(&headers, "Authorization", &format!("Bearer {{env:{name}}}"));
                    }
                    _ => {
                        let token = token.ok_or("缺少 MCP 访问令牌")?;
                        set_string(&headers, "Authorization", &format!("Bearer {token}"));
                    }
                }
            }
        }
    }
    finish_json(
        &root,
        has_bom(snapshot.bytes()),
        if workspace == "claude" {
            MAX_CLAUDE_CONFIG
        } else {
            MAX_CONFIG
        },
    )
}

fn set_string(object: &CstObject, key: &str, value: &str) {
    set_value(object, key, CstInputValue::String(value.into()));
}

/// Flat entries use `enabled`; nested entries use `disabled`.
fn set_enabled(object: &CstObject, workspace: &str, layout: OpenCodeLayout, enabled: bool) {
    if workspace == "claude" {
        return;
    }
    match layout {
        OpenCodeLayout::Flat => {
            remove_if_present(object, "disabled");
            set_value(object, "enabled", CstInputValue::Bool(enabled));
        }
        OpenCodeLayout::Servers => {
            remove_if_present(object, "enabled");
            set_value(object, "disabled", CstInputValue::Bool(!enabled));
        }
    }
}

fn set_value(object: &CstObject, key: &str, value: CstInputValue) {
    match object.get(key) {
        Some(prop) => prop.set_value(value),
        None => {
            object.append(key, value);
        }
    }
}

fn remove_if_present(object: &CstObject, key: &str) {
    if let Some(prop) = object.get(key) {
        prop.remove();
    }
}

fn remove_entry(
    snapshot: &Snapshot,
    workspace: &str,
    layout: OpenCodeLayout,
    name: &str,
) -> Result<Vec<u8>> {
    match snapshot {
        Snapshot::Toml { .. } => {
            let mut doc = parse_toml(snapshot)?;
            if let Some(servers) = doc.get_mut("mcp_servers").and_then(Item::as_table_like_mut) {
                servers.remove(name);
            }
            Ok(serialize_toml(&doc))
        }
        Snapshot::Json { .. } => {
            let label = if workspace == "claude" {
                "Claude Code 配置"
            } else {
                "OpenCode 配置"
            };
            let root = parse_cst(snapshot, label)?;
            if let Some(object) = root.object_value() {
                let mut group = object;
                let mut found = true;
                for key in json_group_path(workspace, layout) {
                    match group.get(key).and_then(|prop| prop.object_value()) {
                        Some(next) => group = next,
                        None => {
                            found = false;
                            break;
                        }
                    }
                }
                if found {
                    if let Some(prop) = group.get(name) {
                        prop.remove();
                    }
                }
            }
            finish_json(
                &root,
                has_bom(snapshot.bytes()),
                if workspace == "claude" {
                    MAX_CLAUDE_CONFIG
                } else {
                    MAX_CONFIG
                },
            )
        }
    }
}

fn backup(data: &Path, workspace: &str, current: &Snapshot) -> Result<()> {
    let Some(bytes) = current.bytes() else {
        return Ok(());
    };
    let directory = data.join("mcp/backups");
    fs::create_dir_all(&directory).map_err(|_| "无法创建 MCP 备份目录")?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let extension = current
        .path()
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("bak");
    let path = directory.join(format!("{workspace}-{stamp}.{extension}"));
    let temp = stage(&path, bytes)?;
    fs::rename(&temp, &path).map_err(|_| {
        let _ = fs::remove_file(&temp);
        "无法写入 MCP 配置备份".to_string()
    })
}

fn commit(data: &Path, current: &Snapshot, next: &Snapshot) -> Result<()> {
    if current.bytes() == next.bytes() {
        return Ok(());
    }
    let path = next.path();
    no_links(path)?;
    let latest = match next {
        Snapshot::Toml { path, .. } => Snapshot::Toml {
            path: path.clone(),
            bytes: read_file(path, MAX_CONFIG)?,
        },
        Snapshot::Json { path, .. } => Snapshot::Json {
            path: path.clone(),
            bytes: read_file(path, MAX_CLAUDE_CONFIG)?,
        },
    };
    if latest.bytes() != current.bytes() {
        return Err("MCP 配置已被其他程序修改，请重新检测".into());
    }
    backup(
        data,
        match next {
            Snapshot::Toml { .. } => "codex",
            Snapshot::Json { path, .. } if path.ends_with(".claude.json") => "claude",
            Snapshot::Json { .. } => "opencode",
        },
        current,
    )?;
    match next.bytes() {
        Some(bytes) => {
            let temp = stage(path, bytes)?;
            fs::rename(&temp, path).map_err(|_| {
                let _ = fs::remove_file(&temp);
                "无法替换 MCP 配置文件，请检查文件是否被占用".to_string()
            })
        }
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("无法删除 MCP 配置文件".into()),
        },
    }
}

fn next_snapshot(current: &Snapshot, bytes: Vec<u8>) -> Snapshot {
    match current {
        Snapshot::Toml { path, .. } => Snapshot::Toml {
            path: path.clone(),
            bytes: Some(bytes),
        },
        Snapshot::Json { path, .. } => Snapshot::Json {
            path: path.clone(),
            bytes: Some(bytes),
        },
    }
}

fn status_for(
    data: &Path,
    context: &Context,
    workspace: &str,
    descriptor: &Descriptor,
    current: &Snapshot,
    local: &[LocalEntry],
    is_local: bool,
) -> Result<Value> {
    // A missing or unreadable local token hides the actionable install button,
    // but must not prevent the rest of the catalog from being inspected.
    let token = descriptor_token(context, descriptor).ok().flatten();
    let local_entry = local
        .iter()
        .find(|entry| entry.name == config_key(descriptor));
    let receipt = read_receipt(data, workspace, current.path(), config_key(descriptor))?;
    let managed = local_entry
        .zip(receipt.as_ref())
        .is_some_and(|(entry, receipt)| !entry.duplicate && entry.hash == receipt.hash);
    let modified = local_entry.is_some() && receipt.is_some() && !managed;
    let core = local_entry.is_some_and(|entry| core_matches(entry, descriptor, workspace));
    let token_ok = local_entry
        .is_some_and(|entry| token_matches(entry, descriptor, workspace, token.as_deref()));
    let recognized = core && (!needs_local_token(descriptor, workspace) || token_ok);
    let installed = local_entry.is_some();
    let compatible = descriptor.workspaces.iter().any(|item| item == workspace);
    let token_available = !needs_local_token(descriptor, workspace) || token.is_some();
    // One mutually exclusive state per entry, so the UI never offers an
    // action the backend would refuse.
    let state = if !compatible {
        "incompatible"
    } else if local_entry.is_some_and(|entry| entry.duplicate) {
        "conflict"
    } else if !installed {
        "absent"
    } else if managed {
        "managed"
    } else if modified {
        "changed"
    } else if !core {
        "conflict"
    } else if recognized {
        "adoptable"
    } else {
        // Same target, but the credential no longer matches.
        "stale"
    };
    let can_install = matches!(state, "absent" | "managed" | "stale");
    let can_adopt = matches!(state, "adoptable" | "changed");
    Ok(json!({
        "id": descriptor.id,
        "state": state,
        "revision": entry_revision(&local, current.path(), config_key(descriptor)),
        "local": is_local,
        "installed": installed,
        "managed": managed,
        "modified": modified,
        "external": installed && !managed,
        "recognized": recognized,
        "adoptable": can_adopt,
        "conflict": state == "conflict",
        "conflictReason": if local_entry.is_some_and(|entry| entry.duplicate) {
            Some("同名 MCP 同时存在于 mcp 和 mcp.servers，请先在配置文件中合并重复项")
        } else { None },
        "compatible": compatible,
        "tokenAvailable": token_available,
        "canInstall": can_install,
        "canAdopt": can_adopt,
        "canRemove": state == "managed",
        "canEnable": workspace != "claude" && state == "managed" && !local_entry.is_some_and(|entry| entry.enabled),
        "canDisable": workspace != "claude" && state == "managed" && local_entry.is_some_and(|entry| entry.enabled),
        "transport": match &descriptor.transport {
            Transport::Stdio { .. } => "stdio",
            Transport::Http { .. } => "http",
        },
        "enabled": local_entry.map(|entry| entry.enabled).unwrap_or(false),
    }))
}

/// Use the same state calculation for catalog entries and software integrations.
pub fn descriptor_status(
    data: &Path,
    context: &Context,
    workspace: &str,
    descriptor: &Descriptor,
) -> Result<Value> {
    let workspace = valid_workspace(workspace)?;
    let current = snapshot(context, workspace)?;
    let local = entries(&current, workspace)?;
    let mut result = status_for(
        data, context, workspace, descriptor, &current, &local, false,
    )?;
    result["configPath"] = json!(current.path());
    Ok(result)
}

pub fn view(data: &Path, context: &Context, workspace: &str) -> Result<Value> {
    let workspace = valid_workspace(workspace)?;
    let local_ids: HashSet<String> = local_entries(data)?
        .into_iter()
        .map(|entry| entry.id)
        .collect();
    let (catalog, catalog_error) = available_catalog(data);
    let mut catalog = resolved_catalog(context, catalog);
    // The entries the user defined on this machine join the served catalog.
    for local in local_entries(data)? {
        if !catalog.servers.iter().any(|server| server.id == local.id) {
            catalog.servers.push(local);
        }
    }
    let current = snapshot(context, workspace)?;
    let local = entries(&current, workspace)?;
    let mut statuses = Vec::new();
    for descriptor in &catalog.servers {
        statuses.push(status_for(
            data,
            context,
            workspace,
            descriptor,
            &current,
            &local,
            local_ids.contains(&descriptor.id),
        )?);
    }
    let local_status = local
        .iter()
        .map(|entry| {
            let receipt = read_receipt(data, workspace, current.path(), &entry.name)?;
            let managed = receipt
                .as_ref()
                .is_some_and(|receipt| !entry.duplicate && receipt.hash == entry.hash);
            let catalog_id = catalog
                .servers
                .iter()
                .find(|descriptor| config_key(descriptor) == entry.name)
                .map(|descriptor| descriptor.id.clone());
            Ok(json!({
                "name": entry.name,
                "transport": entry.transport,
                "target": entry.target,
                "enabled": entry.enabled,
                "managed": managed,
                "catalogId": catalog_id,
                "duplicate": entry.duplicate,
                "revision": entry_revision(&local, current.path(), &entry.name),
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({
        "catalog": catalog,
        "workspace": workspace,
        "configPath": current.path(),
        "revision": current.revision(),
        "source": if catalog_error.is_some() { "local" } else { "cached" },
        "catalogError": catalog_error,
        "entries": statuses,
        "local": local_status,
    }))
}

/// The revision of the agent configuration in a workspace.
pub fn config_revision(context: &Context, workspace: &str) -> Result<String> {
    Ok(snapshot(context, valid_workspace(workspace)?)?.revision())
}

/// The revision of the MCP entry a software integration writes to.
pub fn entry_revision_for(context: &Context, workspace: &str, key: &str) -> Result<String> {
    let workspace = valid_workspace(workspace)?;
    let current = snapshot(context, workspace)?;
    let local = entries(&current, workspace)?;
    Ok(entry_revision(&local, current.path(), key))
}

pub fn install(
    data: &Path,
    context: &Context,
    workspace: &str,
    id: &str,
    revision: &str,
) -> Result<()> {
    let workspace = valid_workspace(workspace)?;
    let descriptor = resolved_servers(data, context)?
        .into_iter()
        .find(|descriptor| descriptor.id == id)
        .ok_or("未找到 MCP 服务")?;
    install_descriptor(data, context, workspace, &descriptor, revision)
}

/// Install a descriptor that did not come from the MCP catalog, for example a
/// software integration that ships its own component.
pub fn install_descriptor(
    data: &Path,
    context: &Context,
    workspace: &str,
    descriptor: &Descriptor,
    revision: &str,
) -> Result<()> {
    let workspace = valid_workspace(workspace)?;
    if !descriptor.workspaces.iter().any(|item| item == workspace) {
        return Err("此 MCP 不支持当前工作区".into());
    }
    let current = snapshot(context, workspace)?;
    let local = entries(&current, workspace)?;
    if entry_revision(&local, current.path(), config_key(descriptor)) != revision {
        return Err("MCP 配置已变化，请重新检测".into());
    }
    let existing = local
        .iter()
        .find(|entry| entry.name == config_key(descriptor));
    let receipt = read_receipt(data, workspace, current.path(), config_key(descriptor))?;
    if let Some(entry) = existing {
        require_unique(entry)?;
        if receipt
            .as_ref()
            .is_some_and(|receipt| receipt.hash != entry.hash)
        {
            return Err("MCP 配置已在外部修改，请先手动检查或移除".into());
        }
        if receipt.is_none() && !core_matches(entry, descriptor, workspace) {
            return Err("已存在同名 MCP 配置，请先检查配置内容".into());
        }
    }
    let token = descriptor_token(context, descriptor)?;
    if needs_local_token(descriptor, workspace) && token.is_none() {
        return Err("未找到本机 MCP 令牌，请先在本机配置后再接入".into());
    }
    let bytes = match current {
        Snapshot::Toml { .. } => write_codex_entry(&current, descriptor, token.as_deref())?,
        Snapshot::Json { .. } => {
            let layout = match existing {
                Some(entry) => entry.layout,
                None => new_entry_layout(&current, workspace)?,
            };
            write_json_entry(&current, descriptor, workspace, layout, token.as_deref())?
        }
    };
    let next = next_snapshot(&current, bytes);
    let new_hash = entries(&next, workspace)?
        .into_iter()
        .find(|entry| entry.name == config_key(descriptor))
        .map(|entry| entry.hash)
        .ok_or("无法确认 MCP 配置写入结果")?;
    commit(data, &current, &next)?;
    write_receipt(
        data,
        workspace,
        current.path(),
        config_key(descriptor),
        &new_hash,
    )
}

pub fn adopt(
    data: &Path,
    context: &Context,
    workspace: &str,
    id: &str,
    revision: &str,
) -> Result<()> {
    let workspace = valid_workspace(workspace)?;
    let descriptor = resolved_servers(data, context)?
        .into_iter()
        .find(|descriptor| descriptor.id == id)
        .ok_or("未找到 MCP 服务")?;
    adopt_descriptor(data, context, workspace, &descriptor, revision)
}

pub fn adopt_descriptor(
    data: &Path,
    context: &Context,
    workspace: &str,
    descriptor: &Descriptor,
    revision: &str,
) -> Result<()> {
    let workspace = valid_workspace(workspace)?;
    if !descriptor.workspaces.iter().any(|item| item == workspace) {
        return Err("此 MCP 不支持当前工作区".into());
    }
    let current = snapshot(context, workspace)?;
    let local = entries(&current, workspace)?;
    if entry_revision(&local, current.path(), config_key(descriptor)) != revision {
        return Err("MCP 配置已变化，请重新检测".into());
    }
    let entry = local
        .iter()
        .find(|entry| entry.name == config_key(descriptor))
        .ok_or("本机没有可接管的 MCP 配置")?;
    require_unique(entry)?;
    let token = descriptor_token(context, descriptor)?;
    if !core_matches(entry, descriptor, workspace)
        || !token_matches(entry, descriptor, workspace, token.as_deref())
    {
        return Err("本机 MCP 配置与可安装目录不匹配，无法接管".into());
    }
    write_receipt(
        data,
        workspace,
        current.path(),
        config_key(descriptor),
        &entry.hash,
    )
}

pub fn remove(
    data: &Path,
    context: &Context,
    workspace: &str,
    id: &str,
    revision: &str,
) -> Result<()> {
    let workspace = valid_workspace(workspace)?;
    remove_key(data, context, workspace, id, revision)
}

/// Remove the entry that uses the given configuration key.
pub fn remove_key(
    data: &Path,
    context: &Context,
    workspace: &str,
    id: &str,
    revision: &str,
) -> Result<()> {
    let workspace = valid_workspace(workspace)?;
    let current = snapshot(context, workspace)?;
    let local = entries(&current, workspace)?;
    if entry_revision(&local, current.path(), id) != revision {
        return Err("MCP 配置已变化，请重新检测".into());
    }
    let entry = local.iter().find(|entry| entry.name == id);
    let receipt = read_receipt(data, workspace, current.path(), id)?;
    let Some(receipt) = receipt else {
        return Err("此 MCP 由外部配置管理，需先接管后才能移除".into());
    };
    let Some(entry) = entry else {
        delete_receipt(data, workspace, id)?;
        return Ok(());
    };
    require_unique(entry)?;
    if receipt.hash != entry.hash {
        return Err("MCP 配置已在外部修改，已保留配置，请手动检查".into());
    }
    let bytes = remove_entry(&current, workspace, entry.layout, id)?;
    let next = next_snapshot(&current, bytes);
    commit(data, &current, &next)?;
    delete_receipt(data, workspace, id)
}

/// Flip the enable flag of an entry the toolbox manages, leaving everything
/// else in the configuration untouched.
fn flip_enabled(
    snapshot: &Snapshot,
    workspace: &str,
    layout: OpenCodeLayout,
    name: &str,
    enabled: bool,
) -> Result<Vec<u8>> {
    match snapshot {
        Snapshot::Toml { .. } => {
            let mut doc = parse_toml(snapshot)?;
            let servers = codex_servers(&mut doc)?;
            let table = servers
                .get_mut(name)
                .and_then(Item::as_table_like_mut)
                .ok_or("本机没有此 MCP 配置")?;
            table.insert("enabled", value(enabled));
            Ok(serialize_toml(&doc))
        }
        Snapshot::Json { .. } => {
            let label = if workspace == "claude" {
                "Claude Code 配置"
            } else {
                "OpenCode 配置"
            };
            let root = parse_cst(snapshot, label)?;
            let Some(object) = root.object_value() else {
                return Err(format!("{label}必须是 JSON 对象"));
            };
            let mut group = object;
            for key in json_group_path(workspace, layout) {
                match group.get(key).and_then(|prop| prop.object_value()) {
                    Some(next) => group = next,
                    None => return Err("本机没有此 MCP 配置".into()),
                }
            }
            let entry = group
                .get(name)
                .and_then(|prop| prop.object_value())
                .ok_or("本机没有此 MCP 配置")?;
            set_enabled(&entry, workspace, layout, enabled);
            finish_json(
                &root,
                has_bom(snapshot.bytes()),
                if workspace == "claude" {
                    MAX_CLAUDE_CONFIG
                } else {
                    MAX_CONFIG
                },
            )
        }
    }
}

pub fn toggle_enabled(
    data: &Path,
    context: &Context,
    workspace: &str,
    id: &str,
    revision: &str,
    enabled: bool,
) -> Result<()> {
    let workspace = valid_workspace(workspace)?;
    if workspace == "claude" {
        return Err(
            "请在 Claude Code 的 /mcp 中为当前项目启用或停用 MCP；此处仅管理全局接入".into(),
        );
    }
    let current = snapshot(context, workspace)?;
    let local = entries(&current, workspace)?;
    if entry_revision(&local, current.path(), id) != revision {
        return Err("MCP 配置已变化，请重新检测".into());
    }
    let entry = local
        .iter()
        .find(|entry| entry.name == id)
        .ok_or("本机没有此 MCP 配置")?;
    require_unique(entry)?;
    let receipt = read_receipt(data, workspace, current.path(), id)?;
    let Some(receipt) = receipt else {
        return Err("此 MCP 由外部配置管理，需先接管后才能修改".into());
    };
    if receipt.hash != entry.hash {
        return Err("MCP 配置已在外部修改，已保留配置，请手动检查".into());
    }
    let bytes = flip_enabled(&current, workspace, entry.layout, id, enabled)?;
    let next = next_snapshot(&current, bytes);
    let new_hash = entries(&next, workspace)?
        .into_iter()
        .find(|entry| entry.name == id)
        .map(|entry| entry.hash)
        .ok_or("无法确认 MCP 配置写入结果")?;
    commit(data, &current, &next)?;
    write_receipt(data, workspace, current.path(), id, &new_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn fixture() -> (PathBuf, Context) {
        let base = std::env::temp_dir().join(format!(
            "cyapi-mcp-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let home = base.join("home");
        let data = base.join("data");
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::create_dir_all(home.join(".config/opencode")).unwrap();
        fs::create_dir_all(home.join("AppData/Roaming/OrbitSSH")).unwrap();
        fs::create_dir_all(data.join("mcp")).unwrap();
        cache(&data, br#"{"schema":1,"generatedAt":"2026-10-20T00:00:00Z","servers":[{"id":"codegraph","name":"CodeGraph","version":"1.6+","description":"code graph","homepage":"https://github.com/colbymchenry/codegraph","capabilities":["index"],"steps":["install"],"installCommand":"npm install -g @colbymchenry/codegraph","transport":{"type":"stdio","command":"codegraph","args":["serve","--mcp"],"startupTimeoutSeconds":30,"toolTimeoutSeconds":120},"workspaces":["codex","claude","opencode"]},{"id":"orbit-ssh","name":"Orbit SSH","version":"2.0+","description":"ssh","homepage":"https://github.com/xmyhhh/orbit-ssh","capabilities":["ssh"],"steps":["start"],"installCommand":"https://github.com/xmyhhh/orbit-ssh/releases","transport":{"type":"http","url":"http://127.0.0.1:8765/mcp","tokenSource":"orbit-ssh"},"workspaces":["codex","claude","opencode"]}]}"#).unwrap();
        fs::write(
            home.join("AppData/Roaming/OrbitSSH/mcp-token"),
            "test-token-123\n",
        )
        .unwrap();
        let context = Context {
            codex_config: home.join(".codex/config.toml"),
            claude_config: home.join(".claude.json"),
            opencode_config: crate::opencode_config::Location::Directory(
                home.join(".config/opencode"),
            ),
            app_data: home.join("AppData/Roaming"),
        };
        (base, context)
    }

    /// The revision of the entry an action writes to.
    fn revision(context: &Context, workspace: &str, key: &str) -> String {
        entry_revision_for(context, workspace, key).unwrap()
    }

    /// The revision a view reports for an entry, by catalog id or name.
    fn scoped_revision(view: &Value, key: &str) -> String {
        let status = view["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == json!(key))
            .map(|entry| entry["revision"].as_str().unwrap().to_string());
        let local = view["local"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == json!(key))
            .map(|entry| entry["revision"].as_str().unwrap().to_string());
        status.or(local).expect("the view reports the entry")
    }

    #[test]
    fn local_views_work_without_a_catalog_and_keep_workspaces_separate() {
        let (base, context) = fixture();
        let data = base.join("data");
        fs::remove_file(data.join("mcp/catalog.json")).unwrap();
        fs::write(
            &context.codex_config,
            "[mcp_servers.codex-only]\ncommand = \"codex-tool\"\n",
        )
        .unwrap();
        fs::write(
            &context.claude_config,
            r#"{"mcpServers":{"claude-only":{"command":"claude-tool"}}}"#,
        )
        .unwrap();
        fs::write(
            context.opencode_config.resolve().unwrap(),
            r#"{"mcp":{"opencode-only":{"type":"local","command":["opencode-tool"]}}}"#,
        )
        .unwrap();
        for workspace in WORKSPACES {
            let current = view(&data, &context, workspace).unwrap();
            assert_eq!(current["workspace"], workspace);
            assert_eq!(current["source"], "local");
            assert!(current["catalogError"].is_string());
            assert!(current["catalog"]["servers"].as_array().unwrap().is_empty());
            assert_eq!(current["local"].as_array().unwrap().len(), 1);
            assert_eq!(current["local"][0]["name"], format!("{workspace}-only"));
            assert_eq!(current["local"][0]["managed"], false);
        }
        // A malformed catalog must not hide valid local configurations either.
        fs::write(data.join("mcp/catalog.json"), "invalid").unwrap();
        assert_eq!(
            view(&data, &context, "codex").unwrap()["local"][0]["name"],
            "codex-only"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn manages_unlisted_entries_offline_using_their_workspace_and_config_key() {
        let (base, context) = fixture();
        let data = base.join("data");
        let mut descriptor = catalog(&data).unwrap().servers[0].clone();
        descriptor.config_key = Some("cyapi-codegraph".into());
        for workspace in ["codex", "opencode"] {
            install_descriptor(
                &data,
                &context,
                workspace,
                &descriptor,
                &revision(&context, workspace, "cyapi-codegraph"),
            )
            .unwrap();
        }
        let other_path = context.opencode_config.resolve().unwrap();
        let other_bytes = fs::read(&other_path).unwrap();
        fs::remove_file(data.join("mcp/catalog.json")).unwrap();
        let offline = view(&data, &context, "codex").unwrap();
        assert!(offline["entries"].as_array().unwrap().is_empty());
        assert_eq!(offline["local"][0]["managed"], true);
        assert_eq!(offline["local"][0]["catalogId"], Value::Null);
        toggle_enabled(
            &data,
            &context,
            "codex",
            "cyapi-codegraph",
            &scoped_revision(&offline, "cyapi-codegraph"),
            false,
        )
        .unwrap();
        let disabled = view(&data, &context, "codex").unwrap();
        assert_eq!(disabled["local"][0]["enabled"], false);
        remove(
            &data,
            &context,
            "codex",
            "cyapi-codegraph",
            &scoped_revision(&disabled, "cyapi-codegraph"),
        )
        .unwrap();
        assert!(view(&data, &context, "codex").unwrap()["local"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(fs::read(other_path).unwrap(), other_bytes);
        assert_eq!(
            view(&data, &context, "opencode").unwrap()["local"][0]["managed"],
            true
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn manages_both_opencode_layouts_without_migrating_existing_entries() {
        let (base, context) = fixture();
        let data = base.join("data");
        let file = context.opencode_config.resolve().unwrap();
        for nested in [false, true] {
            let entry = if nested {
                json!({"type":"local","command":["codegraph","serve","--mcp"],"disabled":true})
            } else {
                json!({"type":"local","command":["codegraph","serve","--mcp"],"enabled":false})
            };
            let group = json!({"codegraph":entry,"unrelated":{"type":"local","command":["keep"]}});
            let doc = if nested {
                json!({"mcp":{"servers":group}})
            } else {
                json!({"mcp":group})
            };
            fs::write(&file, format!("// keep comment\n{doc}\n")).unwrap();
            let before = view(&data, &context, "opencode").unwrap();
            assert_eq!(before["entries"][0]["state"], "adoptable");
            assert_eq!(before["local"].as_array().unwrap().len(), 2);
            adopt(
                &data,
                &context,
                "opencode",
                "codegraph",
                &scoped_revision(&before, "codegraph"),
            )
            .unwrap();
            let rev = revision(&context, "opencode", "codegraph");
            toggle_enabled(&data, &context, "opencode", "codegraph", &rev, true).unwrap();
            let rev = revision(&context, "opencode", "codegraph");
            toggle_enabled(&data, &context, "opencode", "codegraph", &rev, false).unwrap();
            assert_eq!(
                view(&data, &context, "opencode").unwrap()["entries"][0]["enabled"],
                false
            );
            install(
                &data,
                &context,
                "opencode",
                "codegraph",
                &revision(&context, "opencode", "codegraph"),
            )
            .unwrap();
            let current = snapshot(&context, "opencode").unwrap();
            let doc = parse_json_value(&current, "config").unwrap();
            let group = if nested {
                &doc["mcp"]["servers"]
            } else {
                &doc["mcp"]
            };
            assert_eq!(
                group["codegraph"][if nested { "disabled" } else { "enabled" }],
                !nested
            );
            assert!(group["codegraph"]
                .get(if nested { "enabled" } else { "disabled" })
                .is_none());
            assert!(fs::read_to_string(&file)
                .unwrap()
                .contains("// keep comment"));
            remove(
                &data,
                &context,
                "opencode",
                "codegraph",
                &revision(&context, "opencode", "codegraph"),
            )
            .unwrap();
            let local = entries(&snapshot(&context, "opencode").unwrap(), "opencode").unwrap();
            assert_eq!(local.len(), 1);
            assert_eq!(local[0].name, "unrelated");
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_duplicate_opencode_ids_and_preserves_both_definitions() {
        let (base, context) = fixture();
        let data = base.join("data");
        let file = context.opencode_config.resolve().unwrap();
        install(
            &data,
            &context,
            "opencode",
            "codegraph",
            &revision(&context, "opencode", "codegraph"),
        )
        .unwrap();
        let mut doc = parse_json_value(&snapshot(&context, "opencode").unwrap(), "config").unwrap();
        doc["mcp"]["servers"] = json!({"codegraph":{"type":"local","command":["external"],"disabled":true},
            "nested-only":{"type":"local","command":["keep"],"disabled":false}});
        let bytes = serde_json::to_vec(&doc).unwrap();
        fs::write(&file, &bytes).unwrap();
        let before = view(&data, &context, "opencode").unwrap();
        let status = &before["entries"][0];
        assert_eq!(status["state"], "conflict");
        for action in [
            "canInstall",
            "canAdopt",
            "canRemove",
            "canEnable",
            "canDisable",
        ] {
            assert_eq!(status[action], false);
        }
        let local = entries(&snapshot(&context, "opencode").unwrap(), "opencode").unwrap();
        let entry = local
            .iter()
            .find(|entry| entry.name == "codegraph")
            .unwrap();
        assert!(entry.duplicate);
        assert_eq!(
            entry.target, "codegraph serve --mcp",
            "flat layout has CLI precedence"
        );
        let rev = scoped_revision(&before, "codegraph");
        assert!(install(&data, &context, "opencode", "codegraph", &rev).is_err());
        assert!(adopt(&data, &context, "opencode", "codegraph", &rev).is_err());
        assert!(remove(&data, &context, "opencode", "codegraph", &rev).is_err());
        assert!(toggle_enabled(&data, &context, "opencode", "codegraph", &rev, false).is_err());
        assert_eq!(fs::read(&file).unwrap(), bytes);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn mcp_and_model_editor_share_the_explicit_opencode_file() {
        let (base, mut context) = fixture();
        let data = base.join("data");
        let file = base.join("custom-config");
        context.opencode_config = crate::opencode_config::Location::File(file.clone());
        let model =
            crate::opencode_config::editor_view_at(&context.opencode_config, &data).unwrap();
        assert_eq!(
            view(&data, &context, "opencode").unwrap()["configPath"],
            model["configPath"]
        );
        install(
            &data,
            &context,
            "opencode",
            "codegraph",
            &revision(&context, "opencode", "codegraph"),
        )
        .unwrap();
        assert!(file.is_file());
        assert!(!base.join("home/.config/opencode/opencode.json").exists());
        remove(
            &data,
            &context,
            "opencode",
            "codegraph",
            &revision(&context, "opencode", "codegraph"),
        )
        .unwrap();
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn claude_only_advertises_supported_global_mcp_actions() {
        let (base, context) = fixture();
        let data = base.join("data");
        install(
            &data,
            &context,
            "claude",
            "codegraph",
            &revision(&context, "claude", "codegraph"),
        )
        .unwrap();
        let saved = view(&data, &context, "claude").unwrap();
        let status = &saved["entries"][0];
        assert_eq!(status["managed"], true);
        assert_eq!(status["canEnable"], false);
        assert_eq!(status["canDisable"], false);
        assert_eq!(status["canRemove"], true);
        assert!(toggle_enabled(
            &data,
            &context,
            "claude",
            "codegraph",
            &scoped_revision(&saved, "codegraph"),
            false
        )
        .unwrap_err()
        .contains("/mcp"));
        remove(
            &data,
            &context,
            "claude",
            "codegraph",
            &scoped_revision(&saved, "codegraph"),
        )
        .unwrap();
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn published_catalog_installs_and_removes_every_mcp_in_each_workspace() {
        let (base, context) = fixture();
        let data = base.join("data");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../server/data/mcp.json");
        // The private server project is not published. Skip when its data files are absent.
        if !source.is_file() {
            println!("Skipped: no locally prepared private server data");
            return;
        }
        cache(&data, &fs::read(source).unwrap()).unwrap();
        let published = catalog(&data).unwrap();
        assert!(!published.servers.is_empty());
        for workspace in WORKSPACES {
            for descriptor in &published.servers {
                if !descriptor.workspaces.iter().any(|item| item == workspace) {
                    continue;
                }
                let rev = revision(&context, workspace, config_key(descriptor));
                install(&data, &context, workspace, &descriptor.id, &rev)
                    .unwrap_or_else(|error| panic!("{workspace}/{}: {error}", descriptor.id));
                let current = snapshot(&context, workspace).unwrap();
                let local = entries(&current, workspace).unwrap();
                let entry = local
                    .iter()
                    .find(|item| item.name == descriptor.id)
                    .unwrap();
                assert!(core_matches(entry, descriptor, workspace));
                match &descriptor.transport {
                    Transport::Stdio {
                        startup_timeout_seconds,
                        tool_timeout_seconds,
                        ..
                    } => {
                        if workspace == "codex" {
                            let doc = parse_toml(&current).unwrap();
                            let item = &doc["mcp_servers"][&descriptor.id];
                            assert_eq!(
                                item["startup_timeout_sec"].as_integer(),
                                Some(*startup_timeout_seconds as i64)
                            );
                            assert_eq!(
                                item["tool_timeout_sec"].as_integer(),
                                Some(*tool_timeout_seconds as i64)
                            );
                        }
                    }
                    Transport::Http {
                        token_source,
                        bearer_token_env_var,
                        ..
                    } => {
                        if token_source.is_none() && bearer_token_env_var.is_none() {
                            assert!(header(&entry.value, workspace).is_none());
                            assert!(entry.value.get("bearer_token_env_var").is_none());
                        } else {
                            let token = descriptor_token(&context, descriptor).unwrap();
                            assert!(token_matches(
                                entry,
                                descriptor,
                                workspace,
                                token.as_deref()
                            ));
                        }
                    }
                }
                let after = view(&data, &context, workspace).unwrap();
                let status = after["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|item| item["id"] == descriptor.id)
                    .unwrap();
                assert_eq!(status["managed"], true);
                remove(
                    &data,
                    &context,
                    workspace,
                    &descriptor.id,
                    &scoped_revision(&after, &descriptor.id),
                )
                .unwrap();
                let remaining =
                    entries(&snapshot(&context, workspace).unwrap(), workspace).unwrap();
                assert!(!remaining.iter().any(|item| item.name == descriptor.id));
            }
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn opencode_nested_uses_the_servers_group_and_disabled_flag() {
        let (base, _context) = fixture();
        let data = base.join("data");
        let catalog = catalog(&data).unwrap();
        let descriptor = &catalog.servers[0];
        let snapshot = Snapshot::Json {
            path: base.join("home/.config/opencode/opencode.jsonc"),
            bytes: Some(b"{\n  // keep\n  \"theme\": \"dark\"\n}\n".to_vec()),
        };
        let bytes = write_json_entry(
            &snapshot,
            descriptor,
            "opencode",
            OpenCodeLayout::Servers,
            None,
        )
        .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"mcp\""));
        assert!(text.contains("\"servers\""));
        assert!(text.contains("\"codegraph\""));
        assert!(text.contains("\"disabled\": false"));
        assert!(!text.contains("\"enabled\""));
        assert!(text.contains("// keep"));
        // The legacy layout keeps the flat group and the enabled flag.
        let bytes = write_json_entry(
            &snapshot,
            descriptor,
            "opencode",
            OpenCodeLayout::Flat,
            None,
        )
        .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"enabled\": true"));
        assert!(!text.contains("\"servers\""));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn keeps_the_environment_credential_instead_of_writing_a_plaintext_token() {
        let (base, context) = fixture();
        let data = base.join("data");
        cache(
            &data,
            br#"{"schema":2,"generatedAt":"2026-10-20T00:00:00Z","servers":[{"id":"remote","name":"Remote","version":"1","description":"d","homepage":"https://example.com","transport":{"type":"http","url":"https://example.com/mcp","tokenSource":"env:REMOTE_TOKEN","bearerTokenEnvVar":"REMOTE_TOKEN"},"workspaces":["codex","opencode","claude"]}]}"#,
        )
        .unwrap();
        let catalog = catalog(&data).unwrap();
        let descriptor = &catalog.servers[0];
        // Codex keeps the credential in the environment variable it declares.
        let snapshot = Snapshot::Toml {
            path: context.codex_config.clone(),
            bytes: Some(b"[mcp_servers.remote]\nurl = \"https://example.com/mcp\"\n".to_vec()),
        };
        let bytes = write_codex_entry(&snapshot, descriptor, Some("plain-secret")).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("bearer_token_env_var = \"REMOTE_TOKEN\""));
        assert!(!text.contains("plain-secret"));
        assert!(!text.contains("Authorization"));
        // OpenCode resolves the variable itself.
        let snapshot = Snapshot::Json {
            path: base.join("home/.config/opencode/opencode.jsonc"),
            bytes: Some(b"{}".to_vec()),
        };
        let bytes = write_json_entry(
            &snapshot,
            descriptor,
            "opencode",
            OpenCodeLayout::Flat,
            Some("plain-secret"),
        )
        .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("Bearer {env:REMOTE_TOKEN}"));
        assert!(!text.contains("plain-secret"));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn skips_entries_that_need_a_newer_client() {
        let (base, _context) = fixture();
        let data = base.join("data");
        cache(
            &data,
            br#"{"schema":2,"generatedAt":"2026-10-20T00:00:00Z","servers":[{"id":"future","name":"Future","version":"1","description":"d","homepage":"https://example.com","transport":{"type":"stdio","command":"x"},"workspaces":["codex"],"minClientVersion":"99.0.0"},{"id":"now","name":"Now","version":"1","description":"d","homepage":"https://example.com","transport":{"type":"stdio","command":"x"},"workspaces":["codex"]}]}"#,
        )
        .unwrap();
        let catalog = catalog(&data).unwrap();
        assert_eq!(catalog.servers.len(), 1);
        assert_eq!(catalog.servers[0].id, "now");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn stores_and_removes_user_defined_mcp_entries() {
        let (base, context) = fixture();
        let data = base.join("data");
        let mut values = std::collections::BTreeMap::new();
        values.insert("id".to_string(), json!("my-server"));
        values.insert("name".to_string(), json!("我的 MCP"));
        values.insert("transportType".to_string(), json!("stdio"));
        values.insert("command".to_string(), json!("npx"));
        values.insert("args".to_string(), json!("-y my-mcp"));
        values.insert("codex".to_string(), json!(true));
        let descriptor = local_descriptor(&values).unwrap();
        save_local(&data, descriptor).unwrap();
        let entries = local_entries(&data).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "my-server");
        // The entry joins the served catalog and installs like any other entry.
        let view = view(&data, &context, "codex").unwrap();
        assert_eq!(view["catalog"]["servers"].as_array().unwrap().len(), 3);
        let revision = scoped_revision(&view, "my-server");
        install(&data, &context, "codex", "my-server", &revision).unwrap();
        let text = fs::read_to_string(&context.codex_config).unwrap();
        assert!(text.contains("[mcp_servers.my-server]"));
        assert!(text.contains("command = \"npx\""));
        // Removing the entry leaves the agent configuration untouched.
        remove_local(&data, "my-server").unwrap();
        assert!(local_entries(&data).unwrap().is_empty());
        assert!(fs::read_to_string(&context.codex_config)
            .unwrap()
            .contains("my-server"));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_a_user_defined_entry_that_is_not_a_valid_identifier() {
        let (base, _context) = fixture();
        let data = base.join("data");
        let mut values = std::collections::BTreeMap::new();
        values.insert("id".to_string(), json!("Not Valid"));
        values.insert("command".to_string(), json!("npx"));
        let descriptor = local_descriptor(&values).unwrap();
        assert!(save_local(&data, descriptor).is_err());
        assert!(local_entries(&data).unwrap().is_empty());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn installs_and_removes_codex_mcp_without_touching_other_tables() {
        let (base, context) = fixture();
        fs::write(
            &context.codex_config,
            "# keep this comment\n[mcp_servers]\n\n[mcp_servers.other]\ncommand = \"other\"\nargs = []\n",
        )
        .unwrap();
        let rev = revision(&context, "codex", "codegraph");
        install(&base.join("data"), &context, "codex", "codegraph", &rev).unwrap();
        let text = fs::read_to_string(&context.codex_config).unwrap();
        assert!(text.contains("# keep this comment"));
        assert!(text.contains("[mcp_servers.codegraph]"));
        assert!(text.contains("command = \"codegraph\""));
        assert!(text.contains("[mcp_servers.other]"));
        let view = view(&base.join("data"), &context, "codex").unwrap();
        assert_eq!(view["entries"][0]["managed"], true);
        let rev = scoped_revision(&view, "codegraph");
        remove(&base.join("data"), &context, "codex", "codegraph", &rev).unwrap();
        let text = fs::read_to_string(&context.codex_config).unwrap();
        assert!(!text.contains("codegraph"));
        assert!(text.contains("[mcp_servers.other]"));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn installs_orbit_ssh_in_each_json_workspace_and_uses_local_token() {
        let (base, context) = fixture();
        let data = base.join("data");
        for workspace in ["claude", "opencode"] {
            fs::write(
                context.claude_config.clone(),
                "{\n  // keep this comment\n  \"theme\": \"dark\"\n}\n",
            )
            .unwrap();
            fs::write(
                base.join("home/.config/opencode/opencode.jsonc"),
                "{\n  // keep this comment\n  \"theme\": \"dark\",\n  \"mcp\": {\n    \"orbit-ssh\": {\n      \"type\": \"remote\",\n      \"url\": \"http://127.0.0.1:8765/mcp\",\n      \"headers\": { \"Authorization\": \"Bearer old\" }\n    }\n  }\n}\n",
            )
            .unwrap();
            let rev = revision(&context, workspace, "orbit-ssh");
            install(&data, &context, workspace, "orbit-ssh", &rev).unwrap();
            let path = if workspace == "claude" {
                context.claude_config.clone()
            } else {
                base.join("home/.config/opencode/opencode.jsonc")
            };
            let text = fs::read_to_string(path).unwrap();
            assert!(text.contains("// keep this comment"));
            assert!(text.contains("Bearer test-token-123"));
            assert!(!text.contains("Bearer old"));
            assert!(text.contains("\"theme\": \"dark\""));
            let view = view(&data, &context, workspace).unwrap();
            assert_eq!(view["entries"][1]["managed"], true);
            let rev = scoped_revision(&view, "orbit-ssh");
            remove(&data, &context, workspace, "orbit-ssh", &rev).unwrap();
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn updates_and_removes_orbit_ssh_codex_headers_without_losing_other_mcp() {
        let (base, context) = fixture();
        let data = base.join("data");
        fs::write(
            &context.codex_config,
            "[mcp_servers]\n\n[mcp_servers.orbit-ssh]\nenabled = true\nurl = \"http://127.0.0.1:8765/mcp\"\n\n[mcp_servers.orbit-ssh.http_headers]\nAuthorization = \"Bearer stale\"\nX-Keep = \"yes\"\n\n[mcp_servers.other]\ncommand = \"other\"\nargs = []\n",
        )
        .unwrap();
        let before = view(&data, &context, "codex").unwrap();
        assert_eq!(before["entries"][1]["adoptable"], false);
        let rev = scoped_revision(&before, "orbit-ssh");
        install(&data, &context, "codex", "orbit-ssh", &rev).unwrap();
        let text = fs::read_to_string(&context.codex_config).unwrap();
        assert!(text.contains("Bearer test-token-123"));
        assert!(text.contains("X-Keep = \"yes\""));
        assert!(text.contains("[mcp_servers.other]"));
        let after = view(&data, &context, "codex").unwrap();
        assert_eq!(after["entries"][1]["managed"], true);
        remove(
            &data,
            &context,
            "codex",
            "orbit-ssh",
            &scoped_revision(&after, "orbit-ssh"),
        )
        .unwrap();
        let text = fs::read_to_string(&context.codex_config).unwrap();
        assert!(!text.contains("orbit-ssh"));
        assert!(!text.contains("Bearer"));
        assert!(text.contains("[mcp_servers.other]"));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn adopts_external_entry_and_rejects_modified_receipts() {
        let (base, context) = fixture();
        let data = base.join("data");
        fs::write(
            &context.codex_config,
            "[mcp_servers]\n\n[mcp_servers.codegraph]\ncommand = \"codegraph\"\nargs = [\"serve\", \"--mcp\"]\nenabled = true\n",
        )
        .unwrap();
        let rev = revision(&context, "codex", "codegraph");
        let before = view(&data, &context, "codex").unwrap();
        assert_eq!(before["entries"][0]["adoptable"], true);
        adopt(&data, &context, "codex", "codegraph", &rev).unwrap();
        assert_eq!(
            view(&data, &context, "codex").unwrap()["entries"][0]["managed"],
            true
        );
        fs::write(
            &context.codex_config,
            "[mcp_servers]\n\n[mcp_servers.codegraph]\ncommand = \"codegraph\"\nargs = [\"serve\", \"--mcp\"]\nenabled = true\nstartup_timeout_sec = 999\n",
        )
        .unwrap();
        let changed = view(&data, &context, "codex").unwrap();
        assert_eq!(changed["entries"][0]["managed"], false);
        assert_eq!(changed["entries"][0]["modified"], true);
        assert_eq!(changed["entries"][0]["canAdopt"], true);
        assert_eq!(changed["entries"][0]["canRemove"], false);
        assert!(remove(
            &data,
            &context,
            "codex",
            "codegraph",
            &scoped_revision(&changed, "codegraph")
        )
        .unwrap_err()
        .contains("外部修改"));
        adopt(
            &data,
            &context,
            "codex",
            "codegraph",
            &scoped_revision(&changed, "codegraph"),
        )
        .unwrap();
        let adopted = view(&data, &context, "codex").unwrap();
        assert_eq!(adopted["entries"][0]["managed"], true);
        remove(
            &data,
            &context,
            "codex",
            "codegraph",
            &scoped_revision(&adopted, "codegraph"),
        )
        .unwrap();
        assert!(!fs::read_to_string(&context.codex_config)
            .unwrap()
            .contains("codegraph"));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn resolves_orbit_port_from_local_workspace_preferences() {
        let (base, context) = fixture();
        let data = base.join("data");
        fs::write(
            context.app_data.join("OrbitSSH/workspace.json"),
            "{\"preferences\":{\"mcpPort\":9876}}",
        )
        .unwrap();
        fs::write(
            &context.codex_config,
            "[mcp_servers]\n\n[mcp_servers.orbit-ssh]\nurl = \"http://127.0.0.1:9876/mcp\"\n[http_headers]\n",
        )
        .unwrap();
        let first = view(&data, &context, "codex").unwrap();
        assert_eq!(
            first["catalog"]["servers"][1]["transport"]["url"],
            "http://127.0.0.1:9876/mcp"
        );
        assert_eq!(first["entries"][1]["adoptable"], false);
        fs::write(
            &context.codex_config,
            "[mcp_servers]\n\n[mcp_servers.orbit-ssh]\nurl = \"http://127.0.0.1:9876/mcp\"\n[mcp_servers.orbit-ssh.http_headers]\nAuthorization = \"Bearer test-token-123\"\n",
        )
        .unwrap();
        let second = view(&data, &context, "codex").unwrap();
        assert_eq!(second["entries"][1]["adoptable"], true);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_stale_revisions_and_invalid_catalog() {
        let (base, context) = fixture();
        let data = base.join("data");
        assert!(cache(&data, b"{}").is_err());
        assert!(install(&data, &context, "codex", "codegraph", "stale").is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_background_rewrite_of_another_entry_keeps_the_action_valid() {
        let (base, context) = fixture();
        let data = base.join("data");
        fs::write(
            &context.codex_config,
            "[mcp_servers.other]\ncommand = \"other\"\nargs = []\n",
        )
        .unwrap();
        let rev = revision(&context, "codex", "codegraph");
        let other = revision(&context, "codex", "other");
        let before = view(&data, &context, "codex").unwrap()["revision"]
            .as_str()
            .unwrap()
            .to_string();
        // The agent rewrites its own configuration, for example the pipe of a
        // restarted runtime, while the user decides on another entry.
        fs::write(
            &context.codex_config,
            "[mcp_servers.other]\ncommand = \"other\"\nargs = []\n\n[mcp_servers.other.env]\nPIPE = \"pipe-2\"\n",
        )
        .unwrap();
        let after = view(&data, &context, "codex").unwrap()["revision"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(before, after, "the configuration file changed");
        install(&data, &context, "codex", "codegraph", &rev).unwrap();
        let text = fs::read_to_string(&context.codex_config).unwrap();
        assert!(text.contains("[mcp_servers.codegraph]"));
        assert!(text.contains("PIPE = \"pipe-2\""));
        // The entry the action writes to still guards itself.
        assert!(remove(&data, &context, "codex", "other", &other).is_err());
        fs::remove_dir_all(base).unwrap();
    }
}
