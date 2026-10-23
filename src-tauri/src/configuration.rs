//! Declarative local configuration. No network requests or server-provided code execution.
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

type Result<T> = std::result::Result<T, String>;
pub type Values = BTreeMap<String, Value>;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flow {
    pub schema: u32,
    pub revision: String,
    pub steps: Vec<Step>,
    pub files: Vec<ConfigFile>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub title: String,
    pub fields: Vec<Field>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub r#type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, rename = "default", skip_serializing_if = "Option::is_none")]
    pub default_value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    pub root: String,
    pub path: String,
    pub format: String,
    pub bindings: Vec<Binding>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub key: Vec<String>,
    pub field: String,
}
pub struct Context {
    pub home: PathBuf,
    pub app_data: PathBuf,
    pub local_app_data: PathBuf,
    pub skill: Option<PathBuf>,
}

fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn relative(s: &str) -> bool {
    s.len() <= 240
        && !s.is_empty()
        && !s.contains('\\')
        && s.split('/').all(|p| {
            p != "."
                && p != ".."
                && !p.to_lowercase().starts_with(".cyapi")
                && crate::service::safe_path(
                    p.strip_prefix('.')
                        .map(|v| format!("x{v}"))
                        .as_deref()
                        .unwrap_or(p),
                )
        })
}
pub fn validate(flow: &Flow) -> Result<()> {
    if flow.schema != 1
        || flow.revision.is_empty()
        || flow.revision.len() > 80
        || flow.steps.is_empty()
        || flow.steps.len() > 8
        || flow.files.is_empty()
        || flow.files.len() > 8
    {
        return Err("不支持的配置流程".into());
    }
    let mut ids = HashSet::new();
    let mut steps = HashSet::new();
    for step in &flow.steps {
        if !identifier(&step.id)
            || !steps.insert(&step.id)
            || step.title.is_empty()
            || step.title.len() > 160
            || step.fields.is_empty()
        {
            return Err("配置步骤无效".into());
        }
        for field in &step.fields {
            if !identifier(&field.id)
                || !ids.insert(field.id.as_str())
                || ids.len() > 40
                || field.label.is_empty()
                || field.label.len() > 160
                || !["text", "url", "secret", "select", "number", "boolean"]
                    .contains(&field.r#type.as_str())
                || field.options.len() > 80
                || field.placeholder.as_ref().is_some_and(|s| s.len() > 300)
            {
                return Err("配置字段无效".into());
            }
            if field.r#type == "select" && field.options.is_empty()
                || field.minimum.zip(field.maximum).is_some_and(|(a, b)| a > b)
            {
                return Err("配置字段选项或范围无效".into());
            }
            if let Some(value) = &field.default_value {
                if field.r#type == "secret" {
                    return Err("密钥字段不能包含默认值".into());
                }
                check_value(field, value)?;
            }
        }
    }
    let mut paths = HashSet::new();
    let mut bound = HashSet::new();
    for file in &flow.files {
        if !["home", "appData", "localAppData", "skill"].contains(&file.root.as_str())
            || !relative(&file.path)
            || !paths.insert((file.root.clone(), file.path.to_lowercase()))
            || file.bindings.is_empty()
            || file.bindings.len() > 80
        {
            return Err("配置文件位置无效".into());
        }
        let name = file.path.to_lowercase();
        if !match file.format.as_str() {
            "json" => name.ends_with(".json"),
            "yaml" => name.ends_with(".yaml") || name.ends_with(".yml"),
            "dotenv" => name.ends_with(".env"),
            _ => false,
        } {
            return Err("不支持的配置文件格式".into());
        }
        let mut keys: Vec<&Vec<String>> = vec![];
        for binding in &file.bindings {
            if !ids.contains(binding.field.as_str())
                || binding.key.is_empty()
                || binding.key.len() > 8
                || binding.key.iter().any(|k| {
                    k.is_empty()
                        || k.len() > 120
                        || ["__proto__", "prototype", "constructor"].contains(&k.as_str())
                        || k.chars().any(char::is_control)
                })
                || keys
                    .iter()
                    .any(|k| k.starts_with(&binding.key) || binding.key.starts_with(k))
            {
                return Err("配置文件字段映射无效".into());
            }
            if file.format == "dotenv" && (binding.key.len() != 1 || !env_key(&binding.key[0])) {
                return Err("环境配置字段名无效".into());
            }
            keys.push(&binding.key);
            bound.insert(binding.field.as_str());
        }
    }
    if bound != ids {
        return Err("存在未映射的配置字段".into());
    }
    Ok(())
}
fn env_key(s: &str) -> bool {
    s.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn empty(value: &Value) -> bool {
    value.is_null() || value.as_str().is_some_and(|s| s.trim().is_empty())
}
fn check_value(field: &Field, value: &Value) -> Result<()> {
    let invalid = || format!("{}：格式无效", field.label);
    if empty(value) {
        return if field.required {
            Err(format!("请填写{}", field.label))
        } else {
            Ok(())
        };
    }
    match field.r#type.as_str() {
        "number" => {
            let n = value.as_f64().ok_or_else(invalid)?;
            if field.minimum.is_some_and(|m| n < m) || field.maximum.is_some_and(|m| n > m) {
                return Err(format!("{}：超出允许范围", field.label));
            }
        }
        "boolean" => {
            if !value.is_boolean() {
                return Err(invalid());
            }
        }
        _ => {
            let s = value.as_str().ok_or_else(invalid)?;
            if s.len() > 8192 || s.chars().any(char::is_control) {
                return Err(invalid());
            }
            if field.r#type == "select" && !field.options.iter().any(|o| o == s) {
                return Err(invalid());
            }
            if field.r#type == "url" {
                let url = reqwest::Url::parse(s).map_err(|_| invalid())?;
                if !["http", "https"].contains(&url.scheme())
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                {
                    return Err(format!(
                        "{}：请输入完整 HTTP(S) 地址，且不含账号、查询参数或片段",
                        field.label
                    ));
                }
            }
        }
    }
    Ok(())
}
/// macOS keeps `/var`, `/tmp` and `/etc` as links into `/private`. They belong
/// to the operating system, not to the user, so they are resolved before the
/// symlink guard runs. Everything below the alias is still checked.
fn resolve_system_alias(path: &Path) -> PathBuf {
    for alias in ["/var", "/tmp", "/etc"] {
        let alias_path = Path::new(alias);
        let Ok(rest) = path.strip_prefix(alias_path) else {
            continue;
        };
        if !fs::symlink_metadata(alias_path).is_ok_and(|meta| meta.file_type().is_symlink()) {
            continue;
        }
        if let Ok(target) = fs::read_link(alias_path) {
            let resolved = if target.is_absolute() {
                target
            } else {
                alias_path.parent().unwrap_or(Path::new("/")).join(target)
            };
            return resolved.join(rest);
        }
    }
    path.to_path_buf()
}

pub(crate) fn no_links(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err("配置路径必须是绝对路径".into());
    }
    let path = resolve_system_alias(path);
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(meta) => {
                #[cfg(windows)]
                let linked = {
                    use std::os::windows::fs::MetadataExt;
                    meta.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let linked = meta.file_type().is_symlink();
                if linked {
                    return Err("配置路径包含符号链接或目录联接，已停止写入".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("无法检查配置路径".into()),
        }
    }
    Ok(())
}

/// Mark a generated launcher script as executable. Windows has no execute
/// bit, so the call is a no-op there.
pub(crate) fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .map_err(|_| "无法读取启动器权限".to_string())?
            .permissions();
        permissions.set_mode(permissions.mode() | 0o755);
        fs::set_permissions(path, permissions).map_err(|_| "无法设置启动器权限".to_string())?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
fn target(file: &ConfigFile, ctx: &Context) -> Result<PathBuf> {
    let root = match file.root.as_str() {
        "home" => &ctx.home,
        "appData" => &ctx.app_data,
        "localAppData" => &ctx.local_app_data,
        "skill" => ctx.skill.as_ref().ok_or("请从已安装记录中配置此 Skill")?,
        _ => return Err("配置根目录无效".into()),
    };
    let path = root.join(&file.path);
    no_links(&path)?;
    Ok(path)
}
pub(crate) fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::metadata(path) {
        Ok(m) if !m.is_file() || m.len() > 1_000_000 => {
            return Err("配置文件过大或不是普通文件".into())
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法读取配置文件".into()),
    }
    fs::read(path)
        .map(Some)
        .map_err(|_| "无法读取配置文件".into())
}
fn env_pair(line: &str) -> Option<(&str, &str)> {
    let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
    let (key, value) = line.split_once('=')?;
    if env_key(key.trim()) {
        Some((key.trim(), value.trim()))
    } else {
        None
    }
}
fn parse(format: &str, bytes: Option<&[u8]>) -> Result<Value> {
    let Some(bytes) = bytes else {
        return Ok(json!({}));
    };
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "配置文件必须是 UTF-8")?
        .trim_start_matches('\u{feff}');
    let result = match format {
        "json" => {
            serde_json::from_str::<Value>(text).map_err(|_| "已有 JSON 配置无效，未覆盖文件")?
        }
        "yaml" => {
            serde_yaml_ng::from_str::<Value>(text).map_err(|_| "已有 YAML 配置无效，未覆盖文件")?
        }
        "dotenv" => {
            let mut map = Map::new();
            for line in text.lines() {
                if line.trim().is_empty() || line.trim().starts_with('#') {
                    continue;
                }
                let (key, raw) = env_pair(line).ok_or("已有 .env 配置无效，未覆盖文件")?;
                let value = if raw.starts_with('"') {
                    serde_json::from_str::<String>(raw)
                        .map_err(|_| "已有 .env 引号无效，未覆盖文件")?
                } else if raw.starts_with('\'') {
                    raw.strip_prefix('\'')
                        .and_then(|v| v.strip_suffix('\''))
                        .ok_or("已有 .env 引号无效，未覆盖文件")?
                        .to_string()
                } else {
                    raw.to_string()
                };
                map.insert(key.into(), Value::String(value));
            }
            Value::Object(map)
        }
        _ => return Err("不支持的配置格式".into()),
    };
    if !result.is_object() {
        return Err("配置文件顶层必须是对象".into());
    }
    Ok(result)
}
fn at<'a>(value: &'a Value, key: &[String]) -> Option<&'a Value> {
    let mut v = value;
    for k in key {
        v = v.get(k)?;
    }
    Some(v)
}
fn stored_value(field: &Field, format: &str, value: &Value) -> Result<Value> {
    // dotenv stores scalars as strings; restore the declared type for the form and validation.
    if format == "dotenv" && ["number", "boolean"].contains(&field.r#type.as_str()) {
        let parsed = serde_json::from_str::<Value>(value.as_str().unwrap_or(""))
            .map_err(|_| format!("{}：已有配置类型无效", field.label))?;
        check_value(field, &parsed)?;
        return Ok(parsed);
    }
    Ok(value.clone())
}
fn put(value: &mut Value, key: &[String], data: Value) -> Result<()> {
    let mut node = value;
    for k in &key[..key.len() - 1] {
        let map = node
            .as_object_mut()
            .ok_or("配置字段与已有内容结构冲突，未覆盖文件")?;
        node = map.entry(k).or_insert_with(|| json!({}));
    }
    node.as_object_mut()
        .ok_or("配置字段与已有内容结构冲突，未覆盖文件")?
        .insert(key.last().unwrap().clone(), data);
    Ok(())
}
fn encode(file: &ConfigFile, value: &Value, original: Option<&[u8]>) -> Result<Vec<u8>> {
    let text = match file.format.as_str() {
        "json" => format!(
            "{}\n",
            serde_json::to_string_pretty(value).map_err(|_| "无法生成 JSON")?
        ),
        "yaml" => serde_yaml_ng::to_string(value).map_err(|_| "无法生成 YAML")?,
        "dotenv" => {
            // Preserve comments and unbound entries; replace bound entries once, including duplicates.
            let mut seen = HashSet::new();
            let mut lines = vec![];
            for line in std::str::from_utf8(original.unwrap_or_default())
                .unwrap_or("")
                .trim_start_matches('\u{feff}')
                .lines()
            {
                if let Some((key, _)) = env_pair(line) {
                    if file.bindings.iter().any(|b| b.key[0] == key) {
                        if seen.insert(key.to_string()) {
                            lines.push(env_line(key, &value[key]));
                        }
                        continue;
                    }
                }
                lines.push(line.to_string());
            }
            for b in &file.bindings {
                if !seen.contains(&b.key[0]) && !value[&b.key[0]].is_null() {
                    lines.push(env_line(&b.key[0], &value[&b.key[0]]));
                }
            }
            format!("{}\n", lines.join("\n"))
        }
        _ => return Err("不支持的配置格式".into()),
    };
    Ok(text.into_bytes())
}
fn env_line(key: &str, value: &Value) -> String {
    let text = value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string());
    format!("{key}={}", serde_json::to_string(&text).unwrap())
}
pub fn view(flow: &Flow, ctx: &Context) -> Result<Value> {
    validate(flow)?;
    let mut values = Values::new();
    let mut secrets: BTreeMap<String, bool> = BTreeMap::new();
    let mut files = vec![];
    for step in &flow.steps {
        for field in &step.fields {
            if field.r#type != "secret" {
                if let Some(v) = &field.default_value {
                    values.insert(field.id.clone(), v.clone());
                }
            }
        }
    }
    for file in &flow.files {
        let path = target(file, ctx)?;
        let bytes = read_bytes(&path)?;
        let content = parse(&file.format, bytes.as_deref())?;
        files.push(json!({"path":path,"format":file.format,"exists":bytes.is_some()}));
        for binding in &file.bindings {
            let field = flow
                .steps
                .iter()
                .flat_map(|s| &s.fields)
                .find(|f| f.id == binding.field)
                .unwrap();
            let value = at(&content, &binding.key);
            if field.r#type == "secret" {
                let present = value.is_some_and(|v| !empty(v));
                secrets
                    .entry(field.id.clone())
                    .and_modify(|v| *v &= present)
                    .or_insert(present);
            } else if let Some(value) = value {
                values.insert(field.id.clone(), stored_value(field, &file.format, value)?);
            }
        }
    }
    Ok(
        json!({"flow":flow,"flowRevision":crate::service::hash(&serde_json::to_vec(flow).map_err(|_| "配置流程无效")?),"values":values,"configuredSecrets":secrets,"files":files}),
    )
}
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
pub(crate) fn stage(path: &Path, bytes: &[u8]) -> Result<PathBuf> {
    no_links(path)?;
    let parent = path.parent().ok_or("配置目录无效")?;
    fs::create_dir_all(parent).map_err(|_| "无法创建配置目录")?;
    no_links(path)?;
    let temp = parent.join(format!(
        ".cyapi-config-{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).map_err(|_| "无法创建临时配置文件")?;
    let result = (|| {
        // Apply permissions while the new file is still empty, before writing secrets.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let output = std::process::Command::new("whoami.exe")
                .args(["/user", "/fo", "csv", "/nh"])
                .creation_flags(0x08000000)
                .output()
                .map_err(|_| "无法获取配置文件权限身份")?;
            let sid = String::from_utf8_lossy(&output.stdout)
                .split(',')
                .next_back()
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string();
            if !output.status.success()
                || !sid.starts_with("S-1-")
                || !sid
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b == b'S' || b == b'-')
            {
                return Err("无法获取配置文件权限身份".into());
            }
            let access = std::process::Command::new("icacls.exe")
                .arg(&temp)
                .args(["/inheritance:r", "/grant:r", &format!("*{sid}:(F)")])
                .creation_flags(0x08000000)
                .output()
                .map_err(|_| "无法设置配置文件权限")?;
            if !access.status.success() {
                return Err("无法设置配置文件权限，未写入密钥".into());
            }
        }
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "无法写入配置文件".to_string())
    })();
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map(|_| temp)
}
pub fn save(flow: &Flow, ctx: &Context, values: &Values, revision: &str) -> Result<Value> {
    validate(flow)?;
    // Bind the submitted form to the exact flow rendered, even when the server is refreshed.
    if crate::service::hash(&serde_json::to_vec(flow).map_err(|_| "配置流程无效")?) != revision
    {
        return Err("配置流程已更新，请关闭后重新打开".into());
    }
    let fields: BTreeMap<_, _> = flow
        .steps
        .iter()
        .flat_map(|s| &s.fields)
        .map(|f| (f.id.as_str(), f))
        .collect();
    if values.keys().any(|k| !fields.contains_key(k.as_str())) {
        return Err("配置包含未知字段".into());
    }
    // Validate all files and inputs first. Never put secret values in errors, logs or backups.
    let mut prepared = vec![];
    let mut paths = HashSet::new();
    for file in &flow.files {
        let path = target(file, ctx)?;
        if !paths.insert(path.to_string_lossy().replace('\\', "/").to_lowercase()) {
            return Err("配置文件路径重复".into());
        }
        let original = read_bytes(&path)?;
        let mut content = parse(&file.format, original.as_deref())?;
        for binding in &file.bindings {
            let field = fields[binding.field.as_str()];
            let submitted = values
                .get(&field.id)
                .filter(|v| !(field.r#type == "secret" && empty(v)));
            let previous = at(&content, &binding.key)
                .map(|v| stored_value(field, &file.format, v))
                .transpose()?;
            let value = submitted
                .or(previous.as_ref())
                .or(field.default_value.as_ref())
                .cloned()
                .unwrap_or(Value::Null);
            check_value(field, &value)?;
            if !value.is_null() {
                put(&mut content, &binding.key, value)?;
            }
        }
        let bytes = encode(file, &content, original.as_deref())?;
        if bytes.len() > 1_000_000 {
            return Err("生成的配置文件超过大小限制".into());
        }
        prepared.push((path, original, bytes));
    }
    let mut staged: Vec<PathBuf> = vec![];
    for (path, _, bytes) in &prepared {
        match stage(path, bytes) {
            Ok(p) => staged.push(p),
            Err(e) => {
                for p in staged {
                    let _ = fs::remove_file(p);
                }
                return Err(e);
            }
        }
    }
    let mut committed = 0;
    let result = (|| {
        for ((path, original, _), temp) in prepared.iter().zip(&staged) {
            no_links(path)?;
            if &read_bytes(path)? != original {
                return Err("配置文件已被其他程序修改，请重新打开".into());
            }
            fs::rename(temp, path).map_err(|_| "无法替换配置文件，请检查文件是否被占用")?;
            committed += 1;
        }
        Ok(json!({"saved":true,"paths":prepared.iter().map(|(p,_,_)|p).collect::<Vec<_>>()}))
    })();
    let mut rollback_failed = false;
    if result.is_err() {
        for (path, original, _) in prepared.iter().take(committed).rev() {
            let restored = match original {
                Some(bytes) => stage(path, bytes).and_then(|temp| {
                    fs::rename(&temp, path).map_err(|_| {
                        let _ = fs::remove_file(temp);
                        "恢复失败".into()
                    })
                }),
                None => fs::remove_file(path).map_err(|_| "恢复失败".into()),
            };
            rollback_failed |= restored.is_err();
        }
    }
    for p in staged {
        let _ = fs::remove_file(p);
    }
    if rollback_failed {
        return Err("配置保存中断，部分文件未能恢复，请检查配置位置".into());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Flow, Context, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "cyapi-config-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let flow: Flow = serde_json::from_value(json!({"schema":1,"revision":"test","steps":[{"id":"connection","title":"连接","fields":[
            {"id":"url","label":"URL","type":"url","required":true},
            {"id":"key","label":"Key","type":"secret","required":true},
            {"id":"count","label":"数量","type":"number","default":3,"minimum":1,"maximum":10},
            {"id":"enabled","label":"启用","type":"boolean","default":false}
        ]}],"files":[{"root":"appData","path":"demo/config.json","format":"json","bindings":[
            {"key":["connection","url"],"field":"url"},{"key":["connection","key"],"field":"key"},
            {"key":["count"],"field":"count"},{"key":["enabled"],"field":"enabled"}
        ]}]})).unwrap();
        let ctx = Context {
            home: root.join("home"),
            app_data: root.join("home/AppData/Roaming"),
            local_app_data: root.join("home/AppData/Local"),
            skill: None,
        };
        (flow, ctx, root)
    }
    fn values() -> Values {
        serde_json::from_value(
            json!({"url":"https://example.invalid/v1","key":"dummy-secret-test"}),
        )
        .unwrap()
    }
    fn revision(flow: &Flow) -> String {
        crate::service::hash(&serde_json::to_vec(flow).unwrap())
    }
    #[test]
    fn json_yaml_merge_masked_secrets_and_offline_reopen() {
        for format in ["json", "yaml"] {
            let (mut flow, ctx, root) = fixture();
            flow.files[0].format = format.into();
            flow.files[0].path = format!("demo/config.{format}");
            let path = target(&flow.files[0], &ctx).unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                &path,
                if format == "json" {
                    "{\"unrelated\":{\"keep\":true}}"
                } else {
                    "unrelated:\n  keep: true\n"
                },
            )
            .unwrap();
            save(&flow, &ctx, &values(), &revision(&flow)).unwrap();
            let read = view(&flow, &ctx).unwrap();
            assert!(!read.to_string().contains("dummy-secret-test"));
            assert_eq!(read["configuredSecrets"]["key"], true);
            assert_eq!(read["values"]["url"], "https://example.invalid/v1");
            assert_eq!(read["values"]["enabled"], false);
            let mut update = values();
            update.insert("key".into(), json!(""));
            update.insert("count".into(), json!(7));
            save(&flow, &ctx, &update, &revision(&flow)).unwrap();
            let content = parse(format, Some(&fs::read(&path).unwrap())).unwrap();
            assert_eq!(content["connection"]["key"], "dummy-secret-test");
            assert_eq!(content["unrelated"]["keep"], true);
            assert_eq!(content["count"], 7);
            assert!(!fs::read_dir(path.parent().unwrap()).unwrap().any(|f| f
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp")));
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn actual_my_image_flow_matches_upstream_dotenv_and_preserves_extra_settings() {
        let (_, ctx, root) = fixture();
        let flow: Flow = serde_json::from_str(include_str!(
            "../../tests/fixtures/my-image-configuration.json"
        ))
        .unwrap();
        let path = target(&flow.files[0], &ctx).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "# local settings\nIMAGE_MODE=async\nTIMEOUT_MS=400000\n",
        )
        .unwrap();
        let v = serde_json::from_value(json!({"baseUrl":"https://example.invalid/v1","apiKey":"dummy-\"quoted\\key","model":"gpt-image-2.5"})).unwrap();
        save(&flow, &ctx, &v, &revision(&flow)).unwrap();
        let bytes = fs::read(&path).unwrap();
        let doc = parse("dotenv", Some(&bytes)).unwrap();
        assert_eq!(doc["OPENAI_API_KEY"], "dummy-\"quoted\\key");
        assert_eq!(doc["IMAGE_MODE"], "async");
        assert!(String::from_utf8(bytes)
            .unwrap()
            .contains("# local settings"));
        assert!(!view(&flow, &ctx).unwrap().to_string().contains("dummy-"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_input_stale_flow_and_malformed_files_do_not_clobber() {
        let (mut flow, ctx, root) = fixture();
        let path = target(&flow.files[0], &ctx).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bad = b"{broken, dummy-secret-test";
        fs::write(&path, bad).unwrap();
        let error = save(&flow, &ctx, &values(), &revision(&flow)).unwrap_err();
        assert!(!error.contains("dummy-secret-test"));
        assert_eq!(fs::read(&path).unwrap(), bad);
        fs::write(&path, b"{}").unwrap();
        let mut v = values();
        v.insert(
            "url".into(),
            json!("https://user:dummy-secret-test@example.invalid/v1"),
        );
        assert!(!save(&flow, &ctx, &v, &revision(&flow))
            .unwrap_err()
            .contains("dummy-secret-test"));
        v = values();
        v.insert("count".into(), json!(100));
        assert!(save(&flow, &ctx, &v, &revision(&flow)).is_err());
        assert!(save(&flow, &ctx, &values(), "stale").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{}");
        // All files are validated before any are written.
        let mut second = flow.files[0].clone();
        second.path = "demo/second.json".into();
        flow.files.push(second);
        fs::write(ctx.app_data.join("demo/second.json"), "[]").unwrap();
        assert!(save(&flow, &ctx, &values(), &revision(&flow)).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{}");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_traversal_executable_destinations_conflicting_keys_and_server_code() {
        let (flow, ctx, _) = fixture();
        for path in [
            "../x.json",
            "C:/x.json",
            "demo\\x.json",
            "demo/../x.json",
            "demo/CON.json",
            "demo/x.json:ads",
            "demo/run.ps1",
            ".cyapi-install.json",
        ] {
            let mut bad = flow.clone();
            bad.files[0].path = path.into();
            assert!(validate(&bad).is_err(), "{path}");
        }
        let mut bad = flow.clone();
        bad.files[0].bindings[1].key = vec!["connection".into()];
        assert!(validate(&bad).is_err());
        let mut bad = flow.clone();
        bad.files[0].root = "skill".into();
        assert!(view(&bad, &ctx).is_err());
        let mut raw = serde_json::to_value(&flow).unwrap();
        raw["script"] = json!("doSomething()");
        assert!(serde_json::from_value::<Flow>(raw).is_err());
    }
    #[test]
    fn refuses_directory_junctions() {
        let (flow, ctx, root) = fixture();
        fs::create_dir_all(&ctx.app_data).unwrap();
        fs::create_dir_all(root.join("outside")).unwrap();
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let output = std::process::Command::new("cmd.exe")
                .args(["/c", "mklink", "/J"])
                .arg(
                    ctx.app_data
                        .join("demo")
                        .to_string_lossy()
                        .replace('/', "\\"),
                )
                .arg(root.join("outside").to_string_lossy().replace('/', "\\"))
                .creation_flags(0x08000000)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("outside"), ctx.app_data.join("demo")).unwrap();
        }
        assert!(save(&flow, &ctx, &values(), &revision(&flow)).is_err());
        assert!(!root.join("outside/config.json").exists());
        // A Windows junction is a directory, a POSIX link is a file.
        #[cfg(windows)]
        fs::remove_dir(ctx.app_data.join("demo")).unwrap();
        #[cfg(unix)]
        fs::remove_file(ctx.app_data.join("demo")).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_same_file_reached_through_two_root_aliases() {
        let (mut flow, ctx, root) = fixture();
        let mut alias = flow.files[0].clone();
        alias.root = "home".into();
        alias.path = "AppData/Roaming/demo/config.json".into();
        flow.files.push(alias);
        assert!(save(&flow, &ctx, &values(), &revision(&flow))
            .unwrap_err()
            .contains("重复"));
        assert!(!root.exists());
    }
    #[test]
    fn dotenv_preserves_number_and_boolean_field_types_on_reopen() {
        let (mut flow, ctx, root) = fixture();
        flow.files[0].format = "dotenv".into();
        flow.files[0].path = "demo/.env".into();
        for b in &mut flow.files[0].bindings {
            b.key = vec![b.field.to_uppercase()];
        }
        save(&flow, &ctx, &values(), &revision(&flow)).unwrap();
        let read = view(&flow, &ctx).unwrap();
        assert_eq!(read["values"]["count"], 3);
        assert_eq!(read["values"]["enabled"], false);
        let v = serde_json::from_value(read["values"].clone()).unwrap();
        save(&flow, &ctx, &v, &revision(&flow)).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
