//! Generic server-distributed integrations: Skill references or MCP components.
use crate::{
    configuration::{no_links, read_bytes, stage},
    service::hash,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use toml_edit::{DocumentMut, Item};
type Result<T> = std::result::Result<T, String>;

/// Unknown fields are ignored on purpose: a newer server may describe entries
/// this client does not understand, and that must not break the whole catalog.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    schema: u32,
    pub integrations: Vec<Descriptor>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub id: String,
    pub name: String,
    minimum_version: String,
    download_url: String,
    capabilities: Vec<String>,
    steps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_mcp_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guide: Option<Guide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<crate::software::Compatibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces_mcp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<Component>,
    /// The component builds of every platform. A client uses the build that
    /// matches the machine it runs on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<Component>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mcp: Option<Mcp>,
    #[serde(default)]
    status_tool: String,
    #[serde(default)]
    actions: Vec<Action>,
    /// Prerequisites that have to be satisfied before this integration is used.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<Requirement>,
    /// The agents this integration can be connected in. Defaults to all of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<String>,
}
/// A prerequisite of an integration, for example a Skill that has to be installed.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub optional: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Guide {
    pub summary: String,
    pub outcome: String,
    pub preparation: Vec<String>,
    pub steps: Vec<GuideStep>,
    pub examples: Vec<GuideExample>,
    pub limitations: Vec<String>,
    pub troubleshooting: Vec<GuideProblem>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct GuideStep {
    pub title: String,
    pub description: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct GuideExample {
    pub title: String,
    pub prompt: String,
    pub expected: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct GuideProblem {
    pub symptom: String,
    pub solution: String,
}
fn valid_guide(guide: &Guide) -> bool {
    let text = |s: &str| !s.trim().is_empty() && s.len() <= 4000;
    text(&guide.summary)
        && text(&guide.outcome)
        && !guide.steps.is_empty()
        && guide.steps.len() <= 12
        && !guide.examples.is_empty()
        && guide.examples.len() <= 8
        && [&guide.preparation, &guide.limitations]
            .iter()
            .all(|items| items.len() <= 12 && items.iter().all(|item| text(item)))
        && guide
            .steps
            .iter()
            .all(|item| text(&item.title) && text(&item.description))
        && guide
            .examples
            .iter()
            .all(|item| text(&item.title) && text(&item.prompt) && text(&item.expected))
        && guide.troubleshooting.len() <= 12
        && guide
            .troubleshooting
            .iter()
            .all(|item| text(&item.symptom) && text(&item.solution))
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    pub version: String,
    pub platform: String,
    pub path: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default)]
    license_text: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Mcp {
    name: String,
    args: Vec<String>,
    timeout_seconds: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Action {
    id: String,
    label: String,
    tool: String,
    #[serde(default)]
    args: serde_json::Map<String, Value>,
    #[serde(default)]
    fields: Vec<Field>,
    #[serde(default)]
    secondary: bool,
    #[serde(default)]
    result: Value,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Field {
    id: String,
    label: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    default: Value,
    #[serde(default)]
    placeholder: String,
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// True when a component declaration names a build this client can use.
fn valid_component(c: Option<&Component>) -> bool {
    let Some(c) = c else { return true };
    crate::platform::known(&c.platform)
        && c.license_text.len() <= 40_000
        && c.version.len() <= 40
        && c.sha256.len() == 64
        && c.sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && c.path
            == format!(
                "/integrations/components/{}{}",
                c.sha256,
                crate::platform::artifact_extension(&c.platform)
            )
        && c.size > 0
        && c.size <= 100_000_000
}
pub fn validate(bytes: &[u8]) -> Result<Manifest> {
    if bytes.len() > 100_000 {
        return Err("接入目录过大".into());
    }
    let manifest: Manifest =
        serde_json::from_slice(bytes).map_err(|_| "接入目录格式不受支持，请更新客户端")?;
    if ![2, 3, 4].contains(&manifest.schema) || manifest.integrations.len() > 50 {
        return Err("接入目录版本不受支持，请更新客户端".into());
    }
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for d in &manifest.integrations {
        let valid_url = reqwest::Url::parse(&d.download_url).ok().is_some_and(|u| {
            u.scheme() == "https" && u.username().is_empty() && u.password().is_none()
        });
        if !identifier(&d.id)
            || !ids.insert(d.id.to_lowercase())
            || d.name.is_empty()
            || d.name.len() > 100
            || !valid_url
            || d.minimum_version.len() > 40
            || d.capabilities.len() > 8
            || d.steps.len() > 8
            || d.related_mcp_id.as_ref().is_some_and(|id| !identifier(id))
            || d.guide.as_ref().is_some_and(|guide| !valid_guide(guide))
            || d.compatibility
                .as_ref()
                .is_some_and(|c| !crate::software::validate(c))
            || d.capabilities
                .iter()
                .chain(&d.steps)
                .any(|s| s.len() > 1000)
        {
            return Err("接入目录包含无效条目".into());
        }
        if d.requires.len() > 8
            || d.requires.iter().any(|requirement| {
                !["skill", "tool"].contains(&requirement.kind.as_str())
                    || !identifier(&requirement.id)
            })
            || d.workspaces.len() > 3
            || d.workspaces
                .iter()
                .any(|workspace| !["codex", "claude", "opencode"].contains(&workspace.as_str()))
        {
            return Err("接入前置条件无效".into());
        }
        if let Some(id) = &d.skill_id {
            if manifest.schema < 3
                || !identifier(id)
                || d.component.is_some()
                || !d.components.is_empty()
                || d.mcp.is_some()
                || !d.actions.is_empty()
                || !d.status_tool.is_empty()
                || d.replaces_mcp.as_ref().is_some_and(|s| !identifier(s))
            {
                return Err("Skill 接入声明无效".into());
            }
            continue;
        }
        let m = d.mcp.as_ref().ok_or("MCP 声明缺失")?;
        if !identifier(&m.name)
            || !names.insert(m.name.to_lowercase())
            || d.replaces_mcp.is_some()
            || (d.component.is_none() && d.components.is_empty())
            || !valid_component(d.component.as_ref())
            || !d.components.iter().all(|c| valid_component(Some(c)))
            || d.components.len() > 8
            || !identifier(&d.status_tool)
            || !(10..=300).contains(&m.timeout_seconds)
            || m.args.len() > 24
            || m.args.iter().any(|s| {
                s.len() > 2000
                    || s.chars().any(char::is_control)
                    || s.contains('{') && s != "{settingsPath}"
            })
            || d.actions.len() > 12
        {
            return Err("接入目录包含无效条目".into());
        }
        // The same platform may only be published once, so a client always
        // finds exactly one build it can use.
        let mut platforms = HashSet::new();
        for component in d.components.iter() {
            if !platforms.insert(component.platform.as_str()) {
                return Err("接入目录包含重复的平台组件".into());
            }
        }
        let mut actions = HashSet::new();
        for action in &d.actions {
            let mut fields = HashSet::new();
            if !identifier(&action.id)
                || !actions.insert(&action.id)
                || !identifier(&action.tool)
                || action.label.is_empty()
                || action.label.len() > 100
                || action.fields.len() > 12
            {
                return Err("接入操作无效".into());
            }
            for field in &action.fields {
                if !identifier(&field.id)
                    || !fields.insert(&field.id)
                    || field.label.len() > 100
                    || field.placeholder.len() > 1000
                    || !["text", "secret", "number", "boolean"].contains(&field.kind.as_str())
                {
                    return Err("接入配置字段无效".into());
                }
            }
        }
    }
    Ok(manifest)
}
fn replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = stage(path, bytes)?;
    fs::rename(&temp, path).map_err(|_| {
        let _ = fs::remove_file(temp);
        "无法保存接入文件".into()
    })
}
pub fn cache(data: &Path, bytes: &[u8]) -> Result<()> {
    validate(bytes)?;
    replace(&data.join("integrations/catalog.json"), bytes)
}
/// Only catalog-declared, validated HTTPS download pages can be opened by this action.
pub fn open_download(data: &Path, id: &str) -> Result<Value> {
    let descriptor = descriptor(data, id)?;
    crate::platform::open_url(&descriptor.download_url)?;
    Ok(json!({"opened":descriptor.download_url}))
}
pub fn catalog(data: &Path) -> Result<Manifest> {
    let path = data.join("integrations/catalog.json");
    no_links(&path)?;
    validate(&read_bytes(&path)?.ok_or("请先连接服务端，下载软件接入目录")?)
}
pub fn descriptor(data: &Path, id: &str) -> Result<Descriptor> {
    catalog(data)?
        .integrations
        .into_iter()
        .find(|d| d.id == id)
        .ok_or("接入目录中没有此软件".into())
}
/// The component build that matches the machine the client runs on.
///
/// The service publishes one build per platform. A client that finds no build
/// for its own platform cannot use this integration, and says so instead of
/// running a binary of a foreign platform.
pub fn platform_component(d: &Descriptor) -> Option<&Component> {
    d.components
        .iter()
        .find(|c| c.platform == crate::platform::current())
        .or_else(|| {
            d.component
                .as_ref()
                .filter(|c| c.platform == crate::platform::current())
        })
}

/// The platforms the service publishes a component build for.
pub fn component_platforms(d: &Descriptor) -> Vec<String> {
    let mut platforms: Vec<String> = d.components.iter().map(|c| c.platform.clone()).collect();
    if let Some(c) = d.component.as_ref() {
        if !platforms.iter().any(|p| p == &c.platform) {
            platforms.push(c.platform.clone());
        }
    }
    platforms
}

fn component_path(data: &Path, d: &Descriptor) -> Result<PathBuf> {
    let c = platform_component(d).ok_or_else(|| {
        format!(
            "此接入组件尚未提供 {} 版本；已发布：{}",
            crate::platform::current(),
            component_platforms(d).join(" / ")
        )
    })?;
    let name = format!("mcp{}", crate::platform::artifact_extension(&c.platform));
    Ok(data.join(format!("components/{}/{}", c.sha256, name)))
}
pub fn installed(data: &Path, d: &Descriptor) -> bool {
    component(data, d).is_ok()
}
fn component(data: &Path, d: &Descriptor) -> Result<PathBuf> {
    let path = component_path(data, d)?;
    no_links(&path)?;
    let c = platform_component(d).ok_or("MCP 组件缺失")?;
    let file = fs::File::open(&path).map_err(|_| "接入组件尚未下载，请先接入")?;
    if file.metadata().map_err(|_| "无法读取组件")?.len() != c.size {
        return Err("接入组件大小校验失败，请重新下载".into());
    }
    let mut bytes = vec![];
    file.take(c.size + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法校验组件")?;
    if hash(&bytes) != c.sha256 {
        return Err("接入组件校验失败，请重新下载".into());
    }
    Ok(path)
}
pub fn install(data: &Path, d: &Descriptor, bytes: &[u8]) -> Result<()> {
    let c = platform_component(d).ok_or("此接入在当前系统没有可用的组件")?;
    if bytes.len() as u64 != c.size
        || hash(bytes) != c.sha256
        || !crate::platform::is_executable(bytes)
    {
        return Err("服务端组件校验失败".into());
    }
    let path = component_path(data, d)?;
    if !installed(data, d) {
        replace(&path, bytes)?;
    }
    replace(
        &path.with_file_name("THIRD-PARTY-NOTICES.txt"),
        c.license_text.as_bytes(),
    )?;
    Ok(())
}
fn settings_path(data: &Path, d: &Descriptor) -> PathBuf {
    data.join(format!("integrations/{}-settings.json", d.id))
}
fn arguments(data: &Path, d: &Descriptor) -> Result<Vec<String>> {
    Ok(d.mcp
        .as_ref()
        .ok_or("此接入使用 Skill")?
        .args
        .iter()
        .map(|s| {
            if s == "{settingsPath}" {
                settings_path(data, d).to_string_lossy().to_string()
            } else {
                s.clone()
            }
        })
        .collect())
}

/// The diagnostics a component wrote to stderr, bounded so a noisy process
/// cannot flood the log.
fn diagnostics(child: &mut std::process::Child) -> String {
    let Some(stderr) = child.stderr.take() else {
        return String::new();
    };
    let mut bytes = vec![];
    let _ = stderr.take(8_001).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).trim().to_string()
}

fn describe(message: &str, detail: &str) -> String {
    let detail = detail.trim();
    if detail.is_empty() {
        return message.to_string();
    }
    let detail: String = detail.chars().take(400).collect();
    format!("{message}：{detail}")
}

/// Secret values never leave the toolbox, not even inside a component result.
fn strip_secrets(d: &Descriptor, value: &mut Value) {
    let secrets: Vec<String> = d
        .actions
        .iter()
        .flat_map(|action| action.fields.iter())
        .filter(|field| field.kind == "secret")
        .map(|field| field.id.clone())
        .collect();
    if secrets.is_empty() {
        return;
    }
    if let Some(values) = value.get_mut("values").and_then(Value::as_object_mut) {
        for key in secrets {
            values.remove(&key);
        }
    }
}

/// One bounded MCP request against a verified component. No shell, scripts or app-specific logic.
fn invoke(data: &Path, d: &Descriptor, tool: &str, args: &Value) -> Result<Value> {
    let m = d.mcp.as_ref().ok_or("此接入使用 Skill")?;
    let mut command = Command::new(component(data, d)?);
    command
        .args(arguments(data, d)?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|_| "无法启动下载的接入组件")?;
    let input=[json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"cyapi-toolbox","version":env!("CARGO_PKG_VERSION")}}}),json!({"jsonrpc":"2.0","method":"notifications/initialized"}),json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":tool,"arguments":args}})].iter().map(|v|format!("{v}\n")).collect::<String>();
    let write = child
        .stdin
        .take()
        .ok_or("接入组件输入不可用")?
        .write_all(input.as_bytes());
    if write.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("无法向接入组件发送请求".into());
    }
    let stdout = child.stdout.take().ok_or("接入组件输出不可用")?;
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut bytes = vec![];
        let read = stdout.take(4_000_001).read_to_end(&mut bytes);
        let _ = tx.send(read.map(|_| bytes));
    });
    let timeout = if tool == d.status_tool {
        15
    } else {
        m.timeout_seconds
    };
    let result = rx.recv_timeout(Duration::from_secs(timeout));
    let bytes = match result {
        Ok(Ok(bytes)) if bytes.len() <= 4_000_000 => bytes,
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(describe(
                "接入组件响应超时或数据过大",
                &diagnostics(&mut child),
            ));
        }
    };
    let _ = reader.join();
    let diagnostics = diagnostics(&mut child);
    let exit = child.wait().map_err(|_| "无法读取组件退出状态")?;
    if !exit.success() {
        return Err(describe("接入组件执行失败", &diagnostics));
    }
    let lines: Vec<Value> = bytes
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .map(serde_json::from_slice)
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| "接入组件返回了无效 MCP 数据")?;
    let response = lines
        .iter()
        .find(|v| v["id"] == 2 && v["jsonrpc"] == "2.0")
        .ok_or("接入组件没有返回工具结果")?;
    if !response["error"].is_null() {
        return Err("接入组件不支持目录声明的工具，请同步目录".into());
    }
    let result = &response["result"];
    let text = result["content"][0]["text"]
        .as_str()
        .ok_or("接入工具返回格式无效")?;
    if result["isError"] == true {
        return Err(text.chars().take(1500).collect());
    }
    serde_json::from_str(text).map_err(|_| "接入工具未返回结构化结果".into())
}
pub fn action(
    data: &Path,
    d: &Descriptor,
    id: &str,
    values: &BTreeMap<String, Value>,
) -> Result<Value> {
    let action = d
        .actions
        .iter()
        .find(|a| a.id == id)
        .ok_or("目录中没有此操作")?;
    if values
        .keys()
        .any(|key| !action.fields.iter().any(|f| f.id == *key))
    {
        return Err("存在未声明的配置字段".into());
    }
    let mut args = action.args.clone();
    for field in &action.fields {
        let value = values.get(&field.id).unwrap_or(&field.default);
        let valid = match field.kind.as_str() {
            "text" | "secret" => value.as_str().is_some_and(|s| s.len() <= 4000),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            _ => false,
        };
        if !valid {
            return Err(format!("{}格式无效", field.label));
        }
        args.insert(field.id.clone(), value.clone());
    }
    let mut result = invoke(data, d, &action.tool, &Value::Object(args))?;
    strip_secrets(d, &mut result);
    Ok(result)
}

#[derive(Serialize, Deserialize)]
struct Receipt {
    schema: u32,
    root: PathBuf,
    table: String,
}
fn receipt_path(data: &Path, id: &str) -> PathBuf {
    data.join(format!("integrations/{id}.json"))
}
fn receipt(data: &Path, id: &str) -> Result<Option<Receipt>> {
    let p = receipt_path(data, id);
    no_links(&p)?;
    read_bytes(&p)?
        .map(|b| serde_json::from_slice(&b).map_err(|_| "接入记录无效".into()))
        .transpose()
}
fn read_config(root: &Path) -> Result<(Option<Vec<u8>>, DocumentMut)> {
    let p = root.join("config.toml");
    no_links(&p)?;
    let bytes = read_bytes(&p)?;
    let text = std::str::from_utf8(bytes.as_deref().unwrap_or_default())
        .map_err(|_| "Codex 配置不是 UTF-8")?;
    let doc = text
        .trim_start_matches('\u{feff}')
        .parse()
        .map_err(|_| "Codex 配置格式无效，请先修复 config.toml")?;
    Ok((bytes, doc))
}
fn own_table<'a>(doc: &'a DocumentMut, d: &Descriptor) -> Option<&'a Item> {
    doc.get("mcp_servers")
        .and_then(|v| v.get(&d.mcp.as_ref()?.name))
}
fn managed(root: &Path, item: &Item, receipt: &Option<Receipt>) -> bool {
    receipt
        .as_ref()
        .is_some_and(|r| r.schema == 1 && r.root == root && r.table == item.to_string())
}
fn commit_config(
    data: &Path,
    root: &Path,
    before: &Option<Vec<u8>>,
    doc: &DocumentMut,
) -> Result<()> {
    let path = root.join("config.toml");
    if let Some(bytes) = before {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        replace(
            &data.join(format!("backups/integration-config-{stamp}.toml")),
            bytes,
        )?;
    }
    let temp = stage(&path, doc.to_string().as_bytes())?;
    no_links(&path)?;
    if read_bytes(&path)? != *before {
        let _ = fs::remove_file(temp);
        return Err("Codex 配置刚被其他程序修改，请重新检测".into());
    }
    fs::rename(&temp, &path).map_err(|_| {
        let _ = fs::remove_file(temp);
        "无法更新 Codex MCP 配置".into()
    })
}
/// The workspaces an integration can be connected in.
fn workspaces(d: &Descriptor) -> Vec<String> {
    if d.workspaces.is_empty() {
        vec!["codex".into(), "claude".into(), "opencode".into()]
    } else {
        d.workspaces.clone()
    }
}

/// Describe an integration as an MCP server so the shared MCP layer writes,
/// verifies and tracks it exactly like a catalog entry.
fn as_mcp_server(data: &Path, d: &Descriptor) -> Result<crate::mcp::Descriptor> {
    let m = d.mcp.as_ref().ok_or("此接入使用 Skill")?;
    let executable = component_path(data, d)
        .unwrap_or_else(|_| data.join("components").join(&d.id).join("unavailable"));
    Ok(crate::mcp::Descriptor {
        id: d.id.clone(),
        name: d.name.clone(),
        version: platform_component(d)
            .map(|component| component.version.clone())
            .unwrap_or_default(),
        install_commands: std::collections::BTreeMap::new(),
        description: String::new(),
        homepage: d.download_url.clone(),
        capabilities: d.capabilities.clone(),
        steps: d.steps.clone(),
        install_command: String::new(),
        transport: crate::mcp::Transport::Stdio {
            command: executable.to_string_lossy().to_string(),
            args: arguments(data, d)?,
            startup_timeout_seconds: 15,
            tool_timeout_seconds: m.timeout_seconds,
        },
        workspaces: workspaces(d),
        min_client_version: None,
        config_key: Some(m.name.clone()),
    })
}

pub fn view(data: &Path, context: &crate::mcp::Context, workspace: &str) -> Result<Value> {
    let manifest = catalog(data)?;
    let mut entries = vec![];
    for d in &manifest.integrations {
        if d.skill_id.is_some() {
            continue;
        }
        let status =
            crate::mcp::descriptor_status(data, context, workspace, &as_mcp_server(data, d)?)?;
        let downloaded = installed(data, d);
        let health = if downloaded {
            invoke(data, d, &d.status_tool, &json!({}))
                .unwrap_or_else(|message| json!({"ready":false,"message":message,"rows":[]}))
        } else {
            json!({"ready":false,"message":"接入组件尚未下载","rows":[]})
        };
        entries.push(json!({
            "id":d.id,"downloaded":downloaded,"registered":status["installed"],
            "managed":status["managed"],"conflict":status["conflict"],
            "state":status["state"],"canInstall":status["canInstall"],"canAdopt":status["canAdopt"],
            "enabled":status["enabled"],"configPath":status["configPath"],
            "updateAvailable":status["managed"] == true && status["recognized"] != true,
            "usable":status["recognized"] == true && status["enabled"] == true && health["ready"] == true,
            "current":status["recognized"],"mcpRevision":status["revision"],"health":health
        }));
    }
    Ok(json!({
        "manifest":manifest,"source":"cached","revision":crate::mcp::config_revision(context, workspace)?,
        "workspace":workspace,"platform":crate::platform::current(),"entries":entries
    }))
}
pub fn connect(
    data: &Path,
    context: &crate::mcp::Context,
    workspace: &str,
    d: &Descriptor,
    expected: &str,
) -> Result<()> {
    component(data, d)?;
    let descriptor = as_mcp_server(data, d)?;
    crate::mcp::install_descriptor(data, context, workspace, &descriptor, expected)
}
pub fn adopt(
    data: &Path,
    context: &crate::mcp::Context,
    workspace: &str,
    d: &Descriptor,
    expected: &str,
) -> Result<()> {
    component(data, d)?;
    crate::mcp::adopt_descriptor(data, context, workspace, &as_mcp_server(data, d)?, expected)
}
pub fn disconnect(
    data: &Path,
    context: &crate::mcp::Context,
    workspace: &str,
    d: &Descriptor,
    expected: &str,
) -> Result<()> {
    let key = d.mcp.as_ref().map(|m| m.name.clone()).unwrap_or_default();
    crate::mcp::remove_key(data, context, workspace, &key, expected)
}

/// Retire only an MCP table whose receipt still matches this tool's previous installation.
/// The legacy table predates the shared MCP layer, so it is removed directly.
pub fn retire_previous(data: &Path, root: &Path, d: &Descriptor) -> Result<()> {
    let Some(name) = &d.replaces_mcp else {
        return Ok(());
    };
    let mut previous = d.clone();
    previous.mcp = Some(Mcp {
        name: name.clone(),
        args: vec![],
        timeout_seconds: 30,
    });
    let (bytes, mut doc) = read_config(root)?;
    if own_table(&doc, &previous)
        .is_some_and(|item| managed(root, item, &receipt(data, &d.id).ok().flatten()))
    {
        doc.get_mut("mcp_servers")
            .and_then(Item::as_table_like_mut)
            .ok_or("MCP 配置无效")?
            .remove(name);
        commit_config(data, root, &bytes, &doc)?;
        let path = receipt_path(data, &d.id);
        no_links(&path)?;
        if path.exists() {
            fs::remove_file(path).map_err(|_| "无法清理接入记录")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use toml_edit::value;

    fn context(root: &Path) -> crate::mcp::Context {
        crate::mcp::Context {
            codex_config: root.join("config.toml"),
            claude_config: root.join("claude.json"),
            opencode_config: crate::opencode_config::Location::Directory(root.join("opencode")),
            app_data: root.join("AppData"),
        }
    }

    fn current_revision(context: &crate::mcp::Context) -> String {
        crate::mcp::entry_revision_for(context, "codex", "cyapi_example").unwrap()
    }

    // A pre-MCP-manager receipt is a different on-disk format. Build that
    // historical fixture explicitly when checking migration to a Skill.
    fn legacy_registration(data: &Path, root: &Path, descriptor: &Descriptor) {
        let (_, mut doc) = read_config(root).unwrap();
        let key = &descriptor.mcp.as_ref().unwrap().name;
        if doc.get("mcp_servers").is_none() {
            doc.insert("mcp_servers", Item::Table(toml_edit::Table::new()));
        }
        let mut table = toml_edit::Table::new();
        table.insert("command", value("previous-component.exe"));
        doc["mcp_servers"][key] = Item::Table(table);
        fs::write(root.join("config.toml"), doc.to_string()).unwrap();
        let (_, doc) = read_config(root).unwrap();
        let saved = Receipt {
            schema: 1,
            root: root.to_path_buf(),
            table: own_table(&doc, descriptor).unwrap().to_string(),
        };
        fs::create_dir_all(data.join("integrations")).unwrap();
        fs::write(
            receipt_path(data, &descriptor.id),
            serde_json::to_vec(&saved).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn skill_references_migrate_only_owned_mcp_configuration() {
        let (data, root, previous) = fixture();
        legacy_registration(&data, &root, &previous);
        let mut skill = previous.clone();
        skill.skill_id = Some("example-skill".into());
        skill.replaces_mcp = Some("cyapi_example".into());
        skill.component = None;
        skill.mcp = None;
        skill.status_tool.clear();
        let manifest = json!({"schema":3,"integrations":[skill.clone()]});
        validate(&serde_json::to_vec(&manifest).unwrap()).unwrap();
        let mut invalid = manifest.clone();
        invalid["integrations"][0]["component"] = json!(previous.component);
        assert!(validate(&serde_json::to_vec(&invalid).unwrap()).is_err());
        retire_previous(&data, &root, &skill).unwrap();
        assert!(own_table(&read_config(&root).unwrap().1, &previous).is_none());
        legacy_registration(&data, &root, &previous);
        let mut doc = read_config(&root).unwrap().1;
        doc["mcp_servers"]["cyapi_example"]["command"] = value("external.exe");
        fs::write(root.join("config.toml"), doc.to_string()).unwrap();
        let original = fs::read(root.join("config.toml")).unwrap();
        retire_previous(&data, &root, &skill).unwrap();
        assert_eq!(fs::read(root.join("config.toml")).unwrap(), original);
        fs::remove_dir_all(data).unwrap();
    }
    /// Bytes that look like an executable on this platform, so the component
    /// integrity check accepts the fixture.
    fn executable_bytes() -> &'static [u8] {
        if cfg!(windows) {
            b"MZ test component"
        } else if cfg!(target_os = "macos") {
            b"\xCF\xFA\xED\xFE test component"
        } else {
            b"\x7FELF test component"
        }
    }
    fn fixture() -> (PathBuf, PathBuf, Descriptor) {
        let data = std::env::temp_dir().join(format!(
            "cyapi-components-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = data.join("home/.codex");
        fs::create_dir_all(&root).unwrap();
        let bytes = executable_bytes();
        let digest = hash(bytes);
        let platform = crate::platform::current();
        let extension = crate::platform::artifact_extension(platform);
        let d:Descriptor=serde_json::from_value(json!({"id":"example","name":"Example","minimumVersion":"1","downloadUrl":"https://example.com/","capabilities":[],"steps":[],"component":{"version":"1","platform":platform,"path":format!("/integrations/components/{digest}{extension}"),"sha256":digest,"size":bytes.len()},"mcp":{"name":"cyapi_example","args":["--settings","{settingsPath}"],"timeoutSeconds":30},"statusTool":"example_status","actions":[]})).unwrap();
        install(&data, &d, bytes).unwrap();
        (data, root, d)
    }
    #[test]
    fn the_component_of_the_running_platform_is_used() {
        let (data, root, mut d) = fixture();
        let context = context(&root);
        let platform = crate::platform::current();
        // A build of another platform is published next to the local one.
        let other = if platform == crate::platform::WINDOWS_X64 {
            crate::platform::MACOS_ARM64
        } else {
            crate::platform::WINDOWS_X64
        };
        let foreign_bytes = executable_bytes();
        let foreign = Component {
            version: "1".into(),
            platform: other.into(),
            path: format!(
                "/integrations/components/{}{}",
                hash(foreign_bytes),
                crate::platform::artifact_extension(other)
            ),
            sha256: hash(foreign_bytes),
            size: foreign_bytes.len() as u64,
            license_text: "Apache-2.0".into(),
        };
        d.components = vec![foreign.clone()];
        d.component = None;
        // The manifest stays valid, but the build of this platform is missing,
        // so the integration cannot be used on this machine.
        assert!(validate(
            &serde_json::to_vec(&json!({"schema":4,"integrations":[d.clone()]})).unwrap()
        )
        .is_ok());
        assert!(platform_component(&d).is_none());
        assert!(install(&data, &d, executable_bytes()).is_err());
        d.components.push(Component {
            version: "1".into(),
            platform: platform.into(),
            path: format!(
                "/integrations/components/{}{}",
                hash(executable_bytes()),
                crate::platform::artifact_extension(platform)
            ),
            sha256: hash(executable_bytes()),
            size: executable_bytes().len() as u64,
            license_text: "Apache-2.0".into(),
        });
        assert!(validate(
            &serde_json::to_vec(&json!({"schema":4,"integrations":[d.clone()]})).unwrap()
        )
        .is_ok());
        assert_eq!(
            platform_component(&d).map(|c| c.platform.clone()),
            Some(platform.to_string())
        );
        assert_eq!(
            component_platforms(&d),
            vec![other.to_string(), platform.to_string()]
        );
        // The build of this platform is the one that is installed and used.
        install(&data, &d, executable_bytes()).unwrap();
        assert!(installed(&data, &d));
        connect(&data, &context, "codex", &d, &current_revision(&context)).unwrap();
        let (_, doc) = read_config(&root).unwrap();
        assert_eq!(
            doc["mcp_servers"]["cyapi_example"]["command"].as_str(),
            component_path(&data, &d).unwrap().to_str()
        );
        fs::remove_dir_all(data).unwrap();
    }
    #[test]
    fn server_components_validate_hashes_and_preserve_other_mcp_settings() {
        let (data, root, d) = fixture();
        let context = context(&root);
        let original = b"# personal\nmodel='test'\nmcp_servers={other={command='other.exe'}}\n";
        fs::write(root.join("config.toml"), original).unwrap();
        connect(&data, &context, "codex", &d, &current_revision(&context)).unwrap();
        let (_, doc) = read_config(&root).unwrap();
        assert_eq!(
            doc["mcp_servers"]["other"]["command"].as_str(),
            Some("other.exe")
        );
        assert!(doc.to_string().contains("# personal"));
        assert_eq!(
            doc["mcp_servers"]["cyapi_example"]["command"].as_str(),
            component_path(&data, &d).unwrap().to_str()
        );
        assert!(connect(&data, &context, "codex", &d, "stale").is_err());
        connect(&data, &context, "codex", &d, &current_revision(&context)).unwrap();
        disconnect(&data, &context, "codex", &d, &current_revision(&context)).unwrap();
        assert!(own_table(&read_config(&root).unwrap().1, &d).is_none());
        assert!(install(&data, &d, b"tampered").is_err());
        fs::write(component_path(&data, &d).unwrap(), b"modified").unwrap();
        assert!(!installed(&data, &d));
        fs::remove_dir_all(data).unwrap();
    }
    #[test]
    fn external_edits_are_not_overwritten_and_catalog_is_not_embedded() {
        let (data, root, d) = fixture();
        let context = context(&root);
        assert!(catalog(&data).is_err());
        let manifest = json!({"schema":2,"integrations":[d.clone()]});
        cache(&data, &serde_json::to_vec(&manifest).unwrap()).unwrap();
        let mut bad = manifest.clone();
        bad["integrations"][0]["component"]["path"] = json!("/../../evil.exe");
        assert!(validate(&serde_json::to_vec(&bad).unwrap()).is_err());
        let mut duplicate = manifest.clone();
        duplicate["integrations"]
            .as_array_mut()
            .unwrap()
            .push(json!(d));
        assert!(validate(&serde_json::to_vec(&duplicate).unwrap()).is_err());
        connect(&data, &context, "codex", &d, &current_revision(&context)).unwrap();
        let mut doc = read_config(&root).unwrap().1;
        doc["mcp_servers"]["cyapi_example"]["command"] = value("external.exe");
        fs::write(root.join("config.toml"), doc.to_string()).unwrap();
        assert!(disconnect(&data, &context, "codex", &d, &current_revision(&context)).is_err());
        assert!(connect(&data, &context, "codex", &d, &current_revision(&context)).is_err());
        fs::remove_dir_all(data).unwrap();
    }
}
