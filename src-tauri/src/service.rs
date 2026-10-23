#[path = "service/integration_service.rs"]
mod integration_service;
use crate::{
    models::*,
    runtime::{self, Runtime},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{blocking::Client, Url};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    env,
    fs::{self, File},
    io::{Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, String>;
#[cfg(test)]
#[path = "distribution_tests.rs"]
mod distribution_tests;

const MAX_PACKAGE: u64 = 56_000_000;
/// The largest runtime download the client accepts. The macOS releases of Node
/// are a little larger than the Windows one.
const RUNTIME_SIZE_LIMIT: u64 = 200_000_000;
const MAX_FILES: usize = 512;
/// Must stay in sync with `limits.decodedBytes` in `server/skill-catalog.mjs`.
const MAX_DECODED: usize = 40_000_000;
pub(crate) const SERVER_URL: &str = "https://api.cyapi.cyou";
const SERVER_HOST: &str = "api.cyapi.cyou";
/// The download host of release files; it shares the origin with the API host.
const DOWNLOAD_HOST: &str = "tool.cyapi.cyou";
const SERVER_ORIGIN_IP: [u8; 4] = [149, 104, 31, 54];
fn err(e: impl std::fmt::Display) -> String {
    crate::logging::event("error", "native", e.to_string());
    e.to_string()
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
/// A server response that may be a conditional answer to an unchanged file.
pub(crate) struct Fetched {
    /// False when the server answered `304 Not Modified`.
    pub modified: bool,
    pub etag: Option<String>,
    pub bytes: Vec<u8>,
}
/// Stops reading a download as soon as the user cancels the task.
struct CancelReader<R> {
    inner: R,
}
impl<R: Read> Read for CancelReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if crate::cancel::is_cancelled() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                crate::cancel::MESSAGE,
            ));
        }
        self.inner.read(buffer)
    }
}
fn fetch_server_once(
    url: &Url,
    limit: u64,
    timeout: u64,
    direct_origin: bool,
    etag: Option<&str>,
) -> Result<Fetched> {
    let builder = Client::builder()
        .timeout(Duration::from_secs(timeout))
        .connect_timeout(Duration::from_secs(6))
        .redirect(reqwest::redirect::Policy::none());
    let builder = if direct_origin {
        builder
            .no_proxy()
            .resolve(
                url.host_str().unwrap_or(SERVER_HOST),
                SocketAddr::from((SERVER_ORIGIN_IP, 443)),
            )
    } else {
        builder
    };
    let client = builder.build().map_err(|error| error.to_string())?;
    let mut request = client.get(url.clone());
    if let Some(etag) = etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let response = request.send().map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value.len() <= 200);
    if status == 304 {
        return Ok(Fetched {
            modified: false,
            etag,
            bytes: Vec::new(),
        });
    }
    let response = response
        .error_for_status()
        .map_err(|error| error.to_string())?;
    if response.content_length().is_some_and(|len| len > limit) {
        return Err("下载文件过大".into());
    }
    let mut bytes = Vec::new();
    CancelReader {
        inner: response.take(limit + 1),
    }
    .read_to_end(&mut bytes)
    .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("下载超过大小限制".into());
    }
    Ok(Fetched {
        modified: true,
        etag,
        bytes,
    })
}
pub(crate) fn fetch_server_url(url: Url, limit: u64, timeout: u64) -> Result<Vec<u8>> {
    fetch_server_url_conditional(url, limit, timeout, None).map(|fetched| fetched.bytes)
}
/// Some networks cannot reach the project's Cloudflare-fronted domains, but
/// they share one origin that answers directly and presents valid TLS for both
/// names. The fallback therefore keeps the domain (SNI and certificate checks)
/// and only swaps the DNS answer. The model API routes are not covered because
/// that origin does not serve `www.cyapi.cyou`.
fn can_bypass_cloudflare(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some(SERVER_HOST) | Some(DOWNLOAD_HOST)
        )
}
/// Ask the server for a file, sending the entity tag of the cached copy so an
/// unchanged file costs one round trip instead of the whole payload.
pub(crate) fn fetch_server_url_conditional(
    url: Url,
    limit: u64,
    timeout: u64,
    etag: Option<&str>,
) -> Result<Fetched> {
    let can_fallback = can_bypass_cloudflare(&url);
    match fetch_server_once(&url, limit, timeout, false, etag) {
        Ok(fetched) => Ok(fetched),
        Err(primary_error) if can_fallback => {
            crate::logging::event(
                "warn",
                "download",
                format!("Cloudflare 线路请求失败，改为直连源站：{primary_error}"),
            );
            fetch_server_once(&url, limit, timeout, true, etag).map_err(|fallback_error| {
                format!("主线路失败：{primary_error}；源站直连失败：{fallback_error}")
            })
        }
        Err(error) => Err(error),
    }
}
pub(crate) fn base_url_from(server_url: &str) -> Result<Url> {
    let mut url = Url::parse(server_url).map_err(|_| "服务端地址无效")?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if !(url.scheme() == "https" || url.scheme() == "http" && local)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !["", "/"].contains(&url.path())
    {
        return Err("请使用 HTTPS 服务端地址，本地调试可使用 http://127.0.0.1:端口".into());
    }
    url.set_path("/");
    Ok(url)
}
fn fetch_path(server_url: &str, path: &str, limit: u64, timeout: u64) -> Result<Vec<u8>> {
    crate::logging::event(
        "info",
        "download",
        format!("请求资源：{path}（超时上限 {timeout} 秒）"),
    );
    let url = base_url_from(server_url)?
        .join(path.trim_start_matches('/'))
        .map_err(err)?;
    let bytes = fetch_server_url(url, limit, timeout).map_err(|error| {
        crate::logging::event("error", "download", format!("网络请求失败：{error}"));
        "无法连接服务端，请确认服务已启动，或使用已缓存的安装包".to_string()
    })?;
    crate::logging::event(
        "info",
        "download",
        format!("资源读取完成：{} 字节", bytes.len()),
    );
    Ok(bytes)
}

/// The entity tag of a cached file, used to ask the server for changes only.
pub(crate) fn cached_etag(data: &Path, cache: &str) -> Option<String> {
    if !data.join(cache).is_file() {
        return None;
    }
    let text = fs::read_to_string(data.join(format!("{cache}.etag"))).ok()?;
    let text = text.trim();
    if text.is_empty() || text.len() > 200 || text.chars().any(char::is_control) {
        return None;
    }
    Some(text.to_string())
}
fn store_etag(data: &Path, cache: &str, etag: Option<&str>) {
    let path = data.join(format!("{cache}.etag"));
    match etag {
        Some(etag) if !etag.is_empty() => {
            let _ = fs::write(path, etag);
        }
        _ => {
            let _ = fs::remove_file(path);
        }
    }
}
fn fetch_path_conditional(
    server_url: &str,
    path: &str,
    limit: u64,
    timeout: u64,
    etag: Option<&str>,
) -> Result<Fetched> {
    crate::logging::event(
        "info",
        "download",
        format!("请求资源：{path}（超时上限 {timeout} 秒）"),
    );
    let url = base_url_from(server_url)?
        .join(path.trim_start_matches('/'))
        .map_err(err)?;
    let fetched = fetch_server_url_conditional(url, limit, timeout, etag).map_err(|error| {
        crate::logging::event("error", "download", format!("网络请求失败：{error}"));
        "无法连接服务端，请确认服务已启动，或使用已缓存的安装包".to_string()
    })?;
    crate::logging::event(
        "info",
        "download",
        if fetched.modified {
            format!("资源读取完成：{} 字节", fetched.bytes.len())
        } else {
            "服务端内容未变化，沿用本机缓存".to_string()
        },
    );
    Ok(fetched)
}
fn catalog_cache_path(path: &str) -> &'static str {
    match path {
        "/api/catalog" => "catalog.json",
        "/api/mcp" => "mcp/catalog.json",
        _ => "integrations/catalog.json",
    }
}
fn catalog_is_valid(data: &Path, path: &str) -> bool {
    match path {
        "/api/catalog" => read_json::<Catalog>(&data.join("catalog.json"))
            .is_ok_and(|catalog| validate_catalog(&catalog).is_ok()),
        "/api/mcp" => crate::mcp::catalog(data).is_ok(),
        _ => crate::integrations::catalog(data).is_ok(),
    }
}
pub(crate) fn claim_catalog_refresh(data: &Path, path: &str, force: bool) -> bool {
    crate::catalog_cache::claim(
        data,
        catalog_cache_path(path),
        catalog_is_valid(data, path),
        force,
    )
}
/// Fetch a catalog conditionally; only a usable cache may accept a 304 response.
pub(crate) fn fetch_catalog_conditional(
    server_url: &str,
    data: &Path,
    path: &str,
) -> Result<Fetched> {
    let (cache, limit, timeout) = match path {
        "/api/catalog" => ("catalog.json", 4_000_000, 15),
        "/api/mcp" => ("mcp/catalog.json", 100_000, 8),
        _ => ("integrations/catalog.json", 100_000, 8),
    };
    // An invalid local copy must be replaced, not kept by a 304 response.
    let etag = catalog_is_valid(data, path)
        .then(|| cached_etag(data, cache))
        .flatten();
    fetch_path_conditional(server_url, path, limit, timeout, etag.as_deref())
}
fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path).map_err(err)?).map_err(err)
}
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let temp = path.with_extension("tmp");
    let mut file = File::create(&temp).map_err(err)?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(err)?)
        .map_err(err)?;
    file.sync_all().map_err(err)?;
    fs::rename(temp, path).map_err(err)
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('-')
        && !id.ends_with('-')
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
fn digest_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
pub fn safe_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 180 || path.contains('\\') {
        return false;
    }
    path.split('/').all(|part| {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with('.')
            || part.ends_with(' ')
            || part.starts_with('.')
        {
            return false;
        }
        if part
            .chars()
            .any(|c| c.is_control() || "<>:\"|?*".contains(c))
        {
            return false;
        }
        let stem = part.split('.').next().unwrap_or("").to_uppercase();
        ![
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(&stem.as_str())
    })
}
fn validate_requirements(requirements: &[Requirement]) -> Result<()> {
    let mut kinds = HashSet::new();
    if requirements.len() > 2 {
        return Err("运行环境声明过多".into());
    }
    for requirement in requirements {
        if !["node", "python"].contains(&requirement.kind.as_str())
            || runtime::version_tuple(&requirement.minimum).is_none()
            || !kinds.insert(&requirement.kind)
        {
            return Err("运行环境声明无效".into());
        }
    }
    Ok(())
}
pub fn validate_package(bytes: &[u8], skill: &Skill) -> Result<SkillPack> {
    if bytes.len() as u64 != skill.size || hash(bytes) != skill.sha256 {
        return Err("安装包 SHA256 或大小不匹配，请重新下载".into());
    }
    let pack: SkillPack = serde_json::from_slice(bytes).map_err(err)?;
    if pack.schema != 1
        || pack.id != skill.id
        || pack.revision != skill.revision
        || pack.version != skill.version
        || pack.requirements != skill.requirements
        || pack.files.len() != skill.file_count
        || pack.files.is_empty()
        || pack.files.len() > MAX_FILES
    {
        return Err("安装包元数据与目录不匹配".into());
    }
    let mut names = HashSet::new();
    let mut total = 0;
    for file in &pack.files {
        if !safe_path(&file.path) || !names.insert(file.path.to_lowercase()) {
            return Err(format!("不安全或重复的文件路径：{}", file.path));
        }
        let content = STANDARD.decode(&file.content).map_err(err)?;
        total += content.len();
        if total > MAX_DECODED || file.size != content.len() || hash(&content) != file.sha256 {
            return Err(format!("文件校验失败：{}", file.path));
        }
    }
    if !pack.files.iter().any(|f| f.path == "SKILL.md") {
        return Err("缺少 SKILL.md".into());
    }
    // A file cannot also be another file's parent (case insensitive on Windows).
    for name in &names {
        let mut parent = name.as_str();
        while let Some((p, _)) = parent.rsplit_once('/') {
            if names.contains(p) {
                return Err("文件与目录路径冲突".into());
            }
            parent = p;
        }
    }
    Ok(pack)
}
fn validate_catalog(catalog: &Catalog) -> Result<()> {
    if ![1, 2].contains(&catalog.schema)
        || catalog.skills.len() > 200
        || catalog.runtimes.len() > 20
        || catalog.skill_groups.len() > 200
    {
        return Err("不支持的目录格式".into());
    }
    let mut ids = HashSet::new();
    for skill in &catalog.skills {
        if !valid_id(&skill.id)
            || !ids.insert(&skill.id)
            || !["bundled", "source-only"].contains(&skill.distribution.as_str())
            || skill.category.is_empty()
            || skill.category.len() > 100
            || skill.setup_notes.len() > 20
        {
            return Err("Skill 目录包含无效条目".into());
        }
        if skill.distribution == "source-only" {
            if skill.size != 0
                || skill.file_count != 0
                || !skill.sha256.is_empty()
                || !skill.package_path.is_empty()
                || skill
                    .distribution_reason
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
            {
                return Err("来源条目包含无效安装信息".into());
            }
        } else if !digest_valid(&skill.sha256)
            || skill.size > MAX_PACKAGE
            || skill.size == 0
            || skill.file_count == 0
            || skill.file_count > MAX_FILES
            || skill.package_path != format!("/packages/{}.skillpack.json", skill.sha256)
        {
            return Err("Skill 目录包含无效条目".into());
        }
        validate_requirements(&skill.requirements)?;
        crate::external_tools::validate(&skill.tools)?;
        if skill.python_dependencies.len() > 16 {
            // One profile per prepared platform and Python version.
            return Err("Python 依赖配置过多".into());
        }
        for profile in &skill.python_dependencies {
            crate::python_dependencies::validate_profile(profile)?;
            if profile.id != skill.id || !skill.requirements.iter().any(|r| r.kind == "python") {
                return Err("Python 依赖与 Skill 不匹配".into());
            }
        }
        if let Some(flow) = &skill.configuration {
            crate::configuration::validate(flow)?;
        }
    }
    for skill in &catalog.skills {
        if skill.shared_dependencies.len() > 8 {
            return Err("共享依赖配置过多".into());
        }
        for dependency in &skill.shared_dependencies {
            if !valid_id(dependency) || dependency == &skill.id || !ids.contains(dependency) {
                return Err("共享依赖与 Skill 不匹配".into());
            }
        }
    }
    let mut groups = HashSet::new();
    let mut members = HashSet::new();
    let valid_text = |text: &str, limit| {
        !text.trim().is_empty() && text.len() <= limit && !text.chars().any(char::is_control)
    };
    for group in &catalog.skill_groups {
        if !valid_id(&group.id)
            || !groups.insert(&group.id)
            || !valid_text(&group.title, 200)
            || !valid_text(&group.description, 2000)
            || !valid_text(&group.source_url, 2048)
            || group.items.is_empty()
            || group.items.len() > 200
        {
            return Err("Skill 套件目录包含无效条目".into());
        }
        let url = Url::parse(&group.source_url).map_err(|_| "Skill 套件来源地址无效")?;
        if url.scheme() != "https" || url.host_str().is_none()
            || !url.username().is_empty() || url.password().is_some()
        {
            return Err("Skill 套件来源地址无效".into());
        }
        for item in &group.items {
            if !valid_id(&item.skill_id) || !ids.contains(&item.skill_id)
                || !members.insert(&item.skill_id)
            {
                return Err("Skill 套件包含未知或重复的成员".into());
            }
        }
    }
    for artifact in &catalog.runtimes {
        // The shared runtime of the Node and of the Python are published per
        // platform and version.
        let expected = match artifact.kind.as_str() {
            "node" => crate::platform::runtime_path(&artifact.version, &artifact.platform),
            "python" => crate::platform::python_runtime_path(&artifact.version, &artifact.platform),
            _ => return Err("不支持的共享运行环境包".into()),
        };
        if !crate::platform::known(&artifact.platform)
            || runtime::version_tuple(&artifact.version).is_none()
            || !digest_valid(&artifact.sha256)
            || artifact.size == 0
            || artifact.size > RUNTIME_SIZE_LIMIT
            || artifact.path != expected
        {
            return Err("不支持的共享运行环境包".into());
        }
    }
    Ok(())
}

struct SkillRoot {
    path: PathBuf,
    target: String,
    scope: String,
    visible_targets: Vec<String>,
}

pub struct Service {
    pub data: PathBuf,
    pub home: PathBuf,
    settings: Settings,
    catalog: Option<Catalog>,
    runtimes: Vec<Runtime>,
    records: Vec<Record>,
    online: bool,
    message: Option<String>,
}
impl Service {
    fn require_installable(skill: &Skill) -> Result<()> {
        if skill.distribution != "bundled" {
            return Err(skill
                .distribution_reason
                .clone()
                .unwrap_or_else(|| "此 Skill 仅提供官方来源".into()));
        }
        Ok(())
    }
    pub fn new(data: PathBuf, home: PathBuf) -> Result<Self> {
        crate::logging::event("info", "bootstrap", "准备数据目录与安装记录");
        for directory in ["cache", "runtimes", "backups"] {
            fs::create_dir_all(data.join(directory)).map_err(err)?;
        }
        // The service address is built in; legacy settings.json must not override it.
        let settings = Settings {
            server_url: SERVER_URL.into(),
        };
        let catalog = if data.join("catalog.json").exists() {
            let catalog = read_json(&data.join("catalog.json"))?;
            validate_catalog(&catalog)?;
            Some(catalog)
        } else {
            None
        };
        let records = if data.join("installed.json").exists() {
            read_json(&data.join("installed.json"))?
        } else {
            vec![]
        };
        let mut service = Self {
            data,
            home,
            settings,
            catalog,
            runtimes: vec![],
            records,
            online: false,
            message: None,
        };
        service.recover()?;
        service.runtimes = runtime::discover(&service.home, &service.data);
        Ok(service)
    }
    pub fn from_environment() -> Result<Self> {
        let home = env::var_os("CYAPI_TOOLBOX_HOME_DIR")
            .or_else(|| env::var_os(crate::platform::home_variable()))
            .ok_or("找不到用户目录")?;
        let home = PathBuf::from(home);
        let data = env::var_os("CYAPI_TOOLBOX_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::platform::data_root(&home).join("CYAPI-toolbox/data"));
        let mut service = Self::new(data, home)?;
        if let Ok(url) = env::var("CYAPI_TOOLBOX_SERVER_URL") {
            service.settings.server_url = url;
            service.base_url()?;
        }
        Ok(service)
    }
    fn recover(&mut self) -> Result<()> {
        let journal_path = self.data.join("pending.json");
        if !journal_path.exists() {
            return Ok(());
        }
        let journal: Value = read_json(&journal_path)?;
        crate::logging::event("warn", "installer", "发现中断的安装/卸载记录，开始恢复");
        let record: Record = serde_json::from_value(journal["record"].clone()).map_err(err)?;
        let kind = journal["kind"].as_str().unwrap_or("");
        if kind == "install" && Self::owns(&record) {
            self.records.retain(|r| r.id != record.id);
            self.records.push(record);
        } else if kind == "remove"
            && self
                .data
                .join("backups")
                .join(&record.id)
                .join(".cyapi-install.json")
                .is_file()
        {
            self.records.retain(|r| r.id != record.id);
        }
        atomic_json(&self.data.join("installed.json"), &self.records)?;
        fs::remove_file(journal_path).map_err(err)
    }
    fn owns(record: &Record) -> bool {
        read_json::<Record>(&Path::new(&record.path).join(".cyapi-install.json"))
            .is_ok_and(|marker| marker.id == record.id && marker.path == record.path)
    }
    fn package_path(&self, skill: &Skill) -> PathBuf {
        self.data
            .join("cache")
            .join(format!("{}.skillpack.json", skill.sha256))
    }
    fn artifact_path(&self, artifact: &Artifact) -> PathBuf {
        self.data.join("cache").join(format!(
            "{}{}",
            artifact.sha256,
            crate::platform::artifact_extension(&artifact.platform)
        ))
    }
    fn artifact_for<'a>(&'a self, requirement: &Requirement) -> Option<&'a Artifact> {
        self.catalog.as_ref()?.runtimes.iter().find(|a| a.kind == requirement.kind && a.platform == crate::platform::current() && matches!((runtime::version_tuple(&a.version),runtime::version_tuple(&requirement.minimum)),(Some(v),Some(m)) if v >= m))
    }
    fn runtime_for(&self, requirement: &Requirement) -> Option<Runtime> {
        self.runtimes
            .iter()
            .find(|r| runtime::compatible(r, &requirement.kind, &requirement.minimum))
            .cloned()
    }
    /// True when the interpreter version serves the requested major and minor
    /// version. The offline dependency bundles are built for one ABI, so the
    /// patch level does not matter but the major and minor version do.
    fn serves_version(version: &str, wanted: &str) -> bool {
        runtime::version_tuple(version)
            .is_some_and(|(major, minor, _)| format!("{major}.{minor}") == wanted)
    }
    /// The shared Python runtime the service publishes for a version.
    fn managed_python(&self, version: &str) -> Option<&Artifact> {
        self.catalog.as_ref()?.runtimes.iter().find(|a| {
            a.kind == "python"
                && a.platform == crate::platform::current()
                && Self::serves_version(&a.version, version)
        })
    }
    /// Prepare the Python version the offline dependency of a Skill needs.
    ///
    /// The bundle is built for one Python version and one ABI, so a machine
    /// without that interpreter cannot install it. The client unpacks the
    /// runtime the service publishes instead of asking the user for a system
    /// wide installation.
    fn prepare_python_for(&mut self, skill: &Skill) {
        if crate::cancel::is_cancelled() {
            return;
        }
        let needed: Vec<String> = skill
            .python_dependencies
            .iter()
            .filter(|profile| profile.target.platform == crate::platform::current())
            .map(|profile| profile.target.python.clone())
            .collect();
        for version in needed {
            if self
                .runtimes
                .iter()
                .any(|r| r.kind == "python" && Self::serves_version(&r.version, &version))
            {
                continue;
            }
            if let Err(error) = self.ensure_python(&version) {
                crate::logging::event(
                    "warn",
                    "python",
                    format!("准备 Python {version} 运行环境失败：{error}"),
                );
            }
        }
    }
    /// Install the shared Python runtime of a version and return it.
    fn ensure_python(&mut self, version: &str) -> Result<Runtime> {
        let artifact = self
            .managed_python(version)
            .cloned()
            .ok_or_else(|| format!("目录未提供 Python {version} 的运行环境包"))?;
        let bytes = self.fetch(&artifact.path, RUNTIME_SIZE_LIMIT, 600)?;
        if bytes.len() as u64 != artifact.size || hash(&bytes) != artifact.sha256 {
            return Err("Python 运行环境包校验失败，请重新下载".into());
        }
        crate::logging::event(
            "info",
            "python",
            format!("客户端正在安装 Python {version} 运行环境"),
        );
        let path = crate::python_runtime::install(&self.data, &bytes, &artifact.version)?;
        let runtime = Runtime {
            kind: "python".into(),
            version: artifact.version.clone(),
            path: path.to_string_lossy().to_string(),
            source: "managed".into(),
        };
        self.runtimes.insert(0, runtime.clone());
        Ok(runtime)
    }
    fn ready(&self, skill: &Skill) -> bool {
        self.offline_ready(skill, &mut HashSet::new())
    }
    fn offline_ready(&self, skill: &Skill, visiting: &mut HashSet<String>) -> bool {
        if !visiting.insert(skill.id.clone()) {
            return false;
        }
        let ready = skill.distribution == "bundled"
            && crate::external_tools::cached(&self.data, &skill.tools)
            && fs::read(self.package_path(skill))
                .is_ok_and(|b| validate_package(&b, skill).is_ok())
            && (skill.python_dependencies.is_empty()
                || crate::python_dependencies::choose(&skill.python_dependencies, &self.runtimes)
                    .is_ok_and(|(p, _)| crate::python_dependencies::cached(&self.data, p)))
            && skill.requirements.iter().all(|r| {
                self.runtime_for(r).is_some()
                    || self.artifact_for(r).is_some_and(|a| {
                        fs::metadata(self.artifact_path(a)).is_ok_and(|m| m.len() == a.size)
                    })
            })
            && skill.shared_dependencies.iter().all(|id| {
                self.skill(id)
                    .is_ok_and(|dependency| self.offline_ready(&dependency, visiting))
            });
        visiting.remove(&skill.id);
        ready
    }
    pub fn status(&self) -> Value {
        let installed = self.installations();
        let skills: Vec<_> = self.catalog.as_ref().map(|c| c.skills.iter().map(|s| {
            let entries: Vec<_> = installed.iter().filter(|entry| entry.exists && crate::installed::matches(&entry.record, s)).collect();
            let paths: Vec<_> = entries.iter().map(|entry| &entry.record.path).collect();
            // A managed installation whose package hash differs from the catalog
            // entry is outdated and can be updated in place.
            let update_available = entries.iter().any(|entry| {
                entry.managed && !entry.record.package_sha256.is_empty() && entry.record.package_sha256 != s.sha256
            });
            json!({"skill":s,"downloaded":self.package_path(s).is_file(),"offlineReady":self.ready(s),"installed":!paths.is_empty(),"installedPaths":paths,"updateAvailable":update_available})
        }).collect()).unwrap_or_default();
        let skill_groups = self.catalog.as_ref().map(|c| c.skill_groups.as_slice()).unwrap_or_default();
        json!({"settings":self.settings,"online":self.online,"message":self.message,"generatedAt":self.catalog.as_ref().map(|c|&c.generated_at),"skillGroups":skill_groups,"skills":skills,"runtimes":self.runtimes,"installed":installed,"dataPath":self.data,"homePath":self.home})
    }
    fn installations(&self) -> Vec<Installation> {
        self.discover_installations(None)
    }
    fn skill_roots(&self, project: Option<&Path>) -> Vec<SkillRoot> {
        let flag = |name| {
            self.config_env(name).is_some_and(|value| {
                value
                    .to_str()
                    .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            })
        };
        let external = !flag("OPENCODE_DISABLE_EXTERNAL_SKILLS");
        let claude = external
            && !flag("OPENCODE_DISABLE_CLAUDE_CODE")
            && !flag("OPENCODE_DISABLE_CLAUDE_CODE_SKILLS");
        let visible = |target: &str, inherited: bool| {
            let mut targets = vec![target.to_string()];
            if inherited {
                targets.push("opencode".into());
            }
            targets
        };
        let root = |path, target: &str, scope: &str, visible_targets| SkillRoot {
            path,
            target: target.into(),
            scope: scope.into(),
            visible_targets,
        };
        let codex = self.codex_home().join("skills");
        let claude_skills = self.claude_home().join("skills");
        let default_claude = self.home.join(".claude/skills");
        let same_claude = crate::installed::path_key(&claude_skills)
            == crate::installed::path_key(&default_claude);
        let mut roots = vec![
            root(codex.clone(), "codex", "global", visible("codex", false)),
            root(
                codex.join(".system"),
                "codex",
                "global",
                visible("codex", false),
            ),
            root(
                self.home.join(".agents/skills"),
                "codex",
                "global",
                visible("codex", external),
            ),
            root(
                claude_skills,
                "claude",
                "global",
                visible("claude", claude && same_claude),
            ),
            root(
                self.opencode_home().join("skills"),
                "opencode",
                "global",
                visible("opencode", false),
            ),
        ];
        // OpenCode always imports ~/.claude; CLAUDE_CONFIG_DIR only redirects
        // Claude Code itself. Keep the source separate from the visible agents.
        if claude && !same_claude {
            roots.push(root(
                default_claude,
                "claude",
                "global",
                vec!["opencode".into()],
            ));
        }
        if let Some(directory) = self.config_env("OPENCODE_CONFIG_DIR") {
            roots.push(root(
                PathBuf::from(directory).join("skills"),
                "opencode",
                "global",
                visible("opencode", false),
            ));
        }
        // A preview has one project context. Records in another (possibly
        // parent) repository must not extend discovery past its .git boundary.
        let projects: Vec<_> = match project {
            Some(project) => vec![project],
            None => self
                .records
                .iter()
                .filter(|record| record.scope == "project")
                .filter_map(|record| Path::new(&record.path).parent()?.parent()?.parent())
                .collect(),
        };
        let mut seen_projects = HashSet::new();
        for project in projects {
            for directory in project.ancestors() {
                if seen_projects.insert(crate::installed::path_key(directory)) {
                    roots.extend([
                        root(
                            directory.join(".agents/skills"),
                            "codex",
                            "project",
                            visible("codex", external),
                        ),
                        root(
                            directory.join(".claude/skills"),
                            "claude",
                            "project",
                            visible("claude", claude),
                        ),
                        root(
                            directory.join(".opencode/skills"),
                            "opencode",
                            "project",
                            visible("opencode", false),
                        ),
                    ]);
                }
                if directory.join(".git").exists() || directory == self.home {
                    break;
                }
            }
        }
        roots
    }
    fn discover_installations(&self, project: Option<&Path>) -> Vec<Installation> {
        use crate::installed::{path_key, scan};
        let mut installed: Vec<_> = self
            .records
            .iter()
            .map(|record| Installation {
                record: record.clone(),
                exists: Path::new(&record.path).join("SKILL.md").is_file(),
                managed: Self::owns(record),
                visible_targets: vec![record.target.clone()],
            })
            .collect();
        let mut seen: std::collections::HashMap<_, _> = installed
            .iter()
            .enumerate()
            .map(|(index, entry)| (path_key(Path::new(&entry.record.path)), index))
            .collect();
        for root in self.skill_roots(project) {
            for mut record in scan(&root.path, &root.target, &root.scope) {
                let key = path_key(Path::new(&record.path));
                if let Some(&index) = seen.get(&key) {
                    for target in &root.visible_targets {
                        if !installed[index].visible_targets.contains(target) {
                            installed[index].visible_targets.push(target.clone());
                        }
                    }
                    continue;
                }
                if let Some(skill) = self.catalog.as_ref().and_then(|catalog| {
                    catalog
                        .skills
                        .iter()
                        .find(|skill| crate::installed::matches(&record, skill))
                }) {
                    record.title = skill.title.clone();
                }
                seen.insert(key, installed.len());
                installed.push(Installation {
                    record,
                    exists: true,
                    managed: false,
                    visible_targets: root.visible_targets.clone(),
                });
            }
        }
        installed
    }
    fn existing_skill(
        &self,
        skill: &Skill,
        target: &str,
        scope: &str,
        path: &Path,
    ) -> Option<Record> {
        let project = (scope == "project")
            .then(|| path.parent()?.parent()?.parent())
            .flatten();
        let eligible_roots: HashSet<_> = self
            .skill_roots(project)
            .into_iter()
            .filter(|root| {
                root.scope == scope && root.visible_targets.iter().any(|item| item == target)
            })
            .filter(|root| {
                scope != "project"
                    || project.is_some_and(|project| {
                        // Only the selected project and its ancestors can satisfy this install.
                        root.path
                            .parent()
                            .and_then(Path::parent)
                            .is_some_and(|directory| project.starts_with(directory))
                    })
            })
            .filter(|root| {
                scope != "project"
                    || target == "opencode"
                    || path.parent().is_some_and(|parent| {
                        crate::installed::path_key(parent) == crate::installed::path_key(&root.path)
                    })
            })
            .map(|root| crate::installed::path_key(&root.path))
            .collect();
        let installed = self.discover_installations(project);
        installed
            .into_iter()
            .find(|entry| {
                entry.exists
                    && entry.visible_targets.iter().any(|item| item == target)
                    && entry.record.scope == scope
                    && Path::new(&entry.record.path)
                        .parent()
                        .is_some_and(|parent| {
                            eligible_roots.contains(&crate::installed::path_key(parent))
                        })
                    && crate::installed::matches(&entry.record, skill)
            })
            .map(|entry| entry.record)
    }
    pub(crate) fn server_url(&self) -> &str {
        &self.settings.server_url
    }
    pub(crate) fn base_url(&self) -> Result<Url> {
        base_url_from(&self.settings.server_url)
    }
    fn fetch(&self, path: &str, limit: u64, timeout: u64) -> Result<Vec<u8>> {
        fetch_path(&self.settings.server_url, path, limit, timeout)
    }
    fn skill(&self, id: &str) -> Result<Skill> {
        self.catalog
            .as_ref()
            .and_then(|c| c.skills.iter().find(|s| s.id == id))
            .cloned()
            .ok_or("未找到 Skill，请先同步目录".into())
    }
    pub fn refresh(&mut self) -> Result<()> {
        let etag = cached_etag(&self.data, "catalog.json");
        let fetched = fetch_path_conditional(
            self.server_url(),
            "/api/catalog",
            4_000_000,
            15,
            etag.as_deref(),
        );
        self.apply_refresh(fetched)
    }
    pub fn apply_refresh(&mut self, fetched: Result<Fetched>) -> Result<()> {
        let result = fetched.and_then(|fetched| {
            if !fetched.modified {
                // The catalog did not change, so the cached copy stays valid.
                crate::catalog_cache::checked(&self.data, "catalog.json");
                return Ok(());
            }
            let catalog: Catalog = serde_json::from_slice(&fetched.bytes).map_err(err)?;
            validate_catalog(&catalog)?;
            atomic_json(&self.data.join("catalog.json"), &catalog)?;
            store_etag(&self.data, "catalog.json", fetched.etag.as_deref());
            crate::catalog_cache::checked(&self.data, "catalog.json");
            self.catalog = Some(catalog);
            Ok(())
        });
        self.online = result.is_ok();
        self.message = result.as_ref().err().cloned();
        if let Err(error) = &result {
            crate::logging::event(
                "warn",
                "download",
                format!("目录同步失败，尝试使用本机缓存：{error}"),
            );
        }
        result
    }
    pub fn apply_update_check(&mut self, online: Result<Vec<u8>>) -> Result<Value> {
        crate::update::check(self, online)
    }
    /// Automatic refreshes retain a usable catalog when the network is offline.
    /// Manual refreshes still report failures so the user can retry explicitly.
    pub(crate) fn apply_catalog_read(
        &mut self,
        path: &str,
        fetched: Result<Fetched>,
        automatic: bool,
    ) -> Result<()> {
        let result = match path {
            "/api/catalog" => self.apply_refresh(fetched),
            "/api/mcp" => fetched.and_then(|value| self.apply_mcp(value)),
            _ => fetched.and_then(|value| self.apply_integrations(value)),
        };
        if let Err(error) = &result {
            if automatic && catalog_is_valid(&self.data, path) {
                crate::logging::event(
                    "warn",
                    "download",
                    format!("目录自动刷新未完成，沿用本机缓存：{error}"),
                );
                return Ok(());
            }
        }
        result
    }
    /// Store a freshly fetched MCP catalog. An unchanged catalog keeps the cache.
    pub fn apply_mcp(&mut self, fetched: Fetched) -> Result<()> {
        if !fetched.modified {
            crate::catalog_cache::checked(&self.data, "mcp/catalog.json");
            return Ok(());
        }
        crate::mcp::cache(&self.data, &fetched.bytes)?;
        store_etag(&self.data, "mcp/catalog.json", fetched.etag.as_deref());
        crate::catalog_cache::checked(&self.data, "mcp/catalog.json");
        Ok(())
    }
    /// Store a freshly fetched integration catalog. An unchanged catalog keeps the cache.
    pub fn apply_integrations(&mut self, fetched: Fetched) -> Result<()> {
        if !fetched.modified {
            crate::catalog_cache::checked(&self.data, "integrations/catalog.json");
            return Ok(());
        }
        crate::integrations::cache(&self.data, &fetched.bytes)?;
        store_etag(
            &self.data,
            "integrations/catalog.json",
            fetched.etag.as_deref(),
        );
        crate::catalog_cache::checked(&self.data, "integrations/catalog.json");
        Ok(())
    }
    fn download_artifact(&self, artifact: &Artifact) -> Result<()> {
        let target = self.artifact_path(artifact);
        if fs::read(&target)
            .is_ok_and(|b| b.len() as u64 == artifact.size && hash(&b) == artifact.sha256)
        {
            return Ok(());
        }
        let bytes = self.fetch(&artifact.path, RUNTIME_SIZE_LIMIT, 180)?;
        if bytes.len() as u64 != artifact.size || hash(&bytes) != artifact.sha256 {
            return Err("共享 Node 校验失败".into());
        }
        let temp = target.with_extension("part");
        fs::write(&temp, &bytes).map_err(err)?;
        fs::rename(temp, target).map_err(err)
    }
    pub fn download(&mut self, id: &str) -> Result<()> {
        self.download_skill(id, &mut HashSet::new())
    }
    fn download_skill(&mut self, id: &str, visiting: &mut HashSet<String>) -> Result<()> {
        if crate::cancel::is_cancelled() {
            return Err(crate::cancel::MESSAGE.into());
        }
        if !visiting.insert(id.to_string()) {
            return Err(format!("共享依赖存在循环引用：{id}"));
        }
        let result = (|| {
            crate::logging::event("info", "installer", format!("准备离线包：{id}"));
            let skill = self.skill(id)?;
            Self::require_installable(&skill)?;
            for dependency in &skill.shared_dependencies {
                self.download_skill(dependency, visiting)?;
            }
            if !self.package_path(&skill).exists()
                || fs::read(self.package_path(&skill))
                    .is_ok_and(|b| validate_package(&b, &skill).is_err())
            {
                let bytes = self.fetch(&skill.package_path, MAX_PACKAGE, 60)?;
                validate_package(&bytes, &skill)?;
                crate::logging::event(
                    "info",
                    "installer",
                    "离线包大小与 SHA256 校验通过，写入缓存",
                );
                let path = self.package_path(&skill);
                let temp = path.with_extension("part");
                fs::write(&temp, &bytes).map_err(err)?;
                fs::rename(temp, path).map_err(err)?;
            }
            // Cache the fallback even on a machine with Node: this makes the downloaded
            // package reusable offline if a Codex runtime disappears after an update.
            for requirement in &skill.requirements {
                if let Some(artifact) = self.artifact_for(requirement) {
                    self.download_artifact(artifact)?;
                } else if self.runtime_for(requirement).is_none() {
                    return Err(format!(
                        "缺少 {} 运行环境，目录暂未提供可离线准备的环境包",
                        requirement.kind
                    ));
                }
            }
            if !skill.python_dependencies.is_empty() {
                self.runtimes = runtime::discover(&self.home, &self.data);
                self.prepare_python_for(&skill);
                let (profile, _) =
                    crate::python_dependencies::choose(&skill.python_dependencies, &self.runtimes)?;
                crate::python_dependencies::download(&self.data, profile, |path, size| {
                    self.fetch(path, size, 180)
                })?;
            }
            crate::external_tools::download(&self.data, &skill.tools, |path, size| {
                self.fetch(path, size, 300)
            })?;
            Ok(())
        })();
        visiting.remove(id);
        result
    }
    fn codex_home(&self) -> PathBuf {
        env::var_os("CODEX_HOME")
            .filter(|_| env::var_os("CYAPI_TOOLBOX_HOME_DIR").is_none())
            .filter(|_| crate::platform::is_default_home(&self.home))
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.join(".codex"))
    }
    fn claude_home(&self) -> PathBuf {
        self.config_env("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.join(".claude"))
    }
    fn config_env(&self, name: &str) -> Option<std::ffi::OsString> {
        env::var_os(name)
            .filter(|value| !value.is_empty())
            .filter(|_| env::var_os("CYAPI_TOOLBOX_HOME_DIR").is_none())
            .filter(|_| crate::platform::is_default_home(&self.home))
    }
    fn opencode_location(&self) -> crate::opencode_config::Location {
        self.config_env("OPENCODE_CONFIG")
            .map(|path| crate::opencode_config::Location::File(path.into()))
            .unwrap_or_else(|| crate::opencode_config::Location::Directory(self.opencode_home()))
    }
    fn opencode_home(&self) -> PathBuf {
        let config_home = env::var_os("XDG_CONFIG_HOME")
            .filter(|_| env::var_os("CYAPI_TOOLBOX_HOME_DIR").is_none())
            .filter(|_| crate::platform::is_default_home(&self.home))
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| self.home.join(".config"));
        config_home.join("opencode")
    }
    fn app_data_dir(&self) -> PathBuf {
        let isolated = env::var_os("CYAPI_TOOLBOX_HOME_DIR").is_some()
            || !crate::platform::is_default_home(&self.home);
        crate::platform::app_data_dir(&self.home, isolated)
    }
    fn claude_mcp_config(&self) -> PathBuf {
        self.config_env("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.clone())
            .join(".claude.json")
    }
    fn mcp_context(&self) -> crate::mcp::Context {
        crate::mcp::Context {
            codex_config: self.codex_home().join("config.toml"),
            claude_config: self.claude_mcp_config(),
            opencode_config: self.opencode_location(),
            app_data: self.app_data_dir(),
        }
    }
    /// Retire the component builds the current catalog no longer publishes. A
    /// build an agent configuration still points at is kept.
    fn retire_components(&self) {
        let directory = self.data.join("components");
        let Ok(entries) = fs::read_dir(&directory) else {
            return;
        };
        let Ok(manifest) = crate::integrations::catalog(&self.data) else {
            return;
        };
        let published: HashSet<String> = manifest
            .integrations
            .iter()
            .flat_map(|item| {
                item.components
                    .iter()
                    .map(|c| c.sha256.clone())
                    .chain(item.component.as_ref().map(|c| c.sha256.clone()))
            })
            .collect();
        let context = self.mcp_context();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_sha = name.len() == 64
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
            if !is_sha || published.contains(&name) {
                continue;
            }
            let needle = entry.path().to_string_lossy().to_string();
            let referenced = [&context.codex_config, &context.claude_config]
                .iter()
                .any(|file| fs::read_to_string(file).is_ok_and(|text| text.contains(&needle)));
            if referenced {
                continue;
            }
            if let Err(error) = fs::remove_dir_all(entry.path()) {
                crate::logging::event("warn", "installer", format!("回收旧接入组件失败：{error}"));
            }
        }
    }
    fn target_path(
        &self,
        skill: &Skill,
        target: &str,
        scope: &str,
        project: &str,
    ) -> Result<PathBuf> {
        if !["codex", "claude", "opencode"].contains(&target)
            || !["global", "project"].contains(&scope)
        {
            return Err("安装目标无效".into());
        }
        let base = if scope == "global" {
            match target {
                "codex" => self.codex_home(),
                "opencode" => self.opencode_home(),
                _ => self.claude_home(),
            }
        } else {
            let path = PathBuf::from(project);
            if !path.is_absolute() || !path.is_dir() {
                return Err("请选择已存在的项目绝对路径".into());
            }
            let canonical = fs::canonicalize(path).map_err(err)?;
            // Keep normal Win32 paths in the UI, Markdown and PowerShell launcher.
            let text = canonical.to_string_lossy();
            if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
                PathBuf::from(format!(r"\\{}", unc))
            } else if let Some(normal) = text.strip_prefix(r"\\?\") {
                PathBuf::from(normal)
            } else {
                canonical
            }
        };
        let path = if scope == "global" {
            base.join("skills")
        } else {
            base.join(match target {
                "codex" => ".agents",
                "opencode" => ".opencode",
                _ => ".claude",
            })
            .join("skills")
        };
        Ok(path.join(&skill.id))
    }
    pub fn preview(&self, id: &str, target: &str, scope: &str, project: &str) -> Result<Value> {
        let skill = self.skill(id)?;
        Self::require_installable(&skill)?;
        let path = self.target_path(&skill, target, scope, project)?;
        let existing = self.existing_skill(&skill, target, scope, &path);
        let resolved: Vec<_> = skill.requirements.iter().map(|r| {
            if r.kind == "python" && !skill.python_dependencies.is_empty() {
                return match crate::python_dependencies::choose(&skill.python_dependencies, &self.runtimes) {
                    Ok((_, base)) => json!({"requirement":r,"reuse":base,"fallback":null}),
                    // The client prepares the runtime the service publishes, so
                    // the user does not have to install a Python themselves.
                    Err(error) => {
                        let prepared = skill
                            .python_dependencies
                            .iter()
                            .filter(|profile| {
                                profile.target.platform == crate::platform::current()
                                    && self.managed_python(&profile.target.python).is_some()
                            })
                            .map(|profile| profile.target.python.clone())
                            .collect::<Vec<_>>();
                        let message = if prepared.is_empty() {
                            error
                        } else {
                            format!(
                                "离线依赖需要 Python {}，客户端将在安装时自动准备该版本的运行环境。",
                                prepared.join(" / ")
                            )
                        };
                        json!({"requirement":r,"reuse":null,"fallback":null,"error":message})
                    }
                };
            }
            json!({"requirement":r,"reuse":self.runtime_for(r),"fallback":self.artifact_for(r)})
        }).collect();
        Ok(
            json!({"path":existing.as_ref().map(|record| Path::new(&record.path)).unwrap_or(&path),"installed":existing.is_some(),"conflict":existing.is_some() || path.try_exists().map_err(err)?,"offlineReady":self.ready(&skill),"runtimes":resolved}),
        )
    }
    fn prepare_runtime(&mut self, requirement: &Requirement) -> Result<Runtime> {
        // Recheck immediately before use, and never run package setup scripts.
        for runtime in &self.runtimes {
            if runtime::compatible(runtime, &requirement.kind, &requirement.minimum) {
                if let Some(actual) =
                    runtime::probe(Path::new(&runtime.path), &runtime.kind, &runtime.source)
                {
                    if runtime::compatible(&actual, &requirement.kind, &requirement.minimum) {
                        return Ok(actual);
                    }
                }
            }
        }
        let artifact = self
            .artifact_for(requirement)
            .ok_or("缺少兼容运行环境；请联网准备安装文件")?
            .clone();
        let bytes = fs::read(self.artifact_path(&artifact))
            .map_err(|_| "缺少离线 Node 环境包，请先下载")?;
        if bytes.len() as u64 != artifact.size || hash(&bytes) != artifact.sha256 {
            return Err("离线 Node 包校验失败，请重新下载".into());
        }
        let directory = self.data.join("runtimes/node").join(&artifact.version);
        fs::create_dir_all(&directory).map_err(err)?;
        let executable = crate::platform::executable("node");
        let exe = directory.join(&executable);
        if !fs::read(&exe).is_ok_and(|existing| hash(&existing) == artifact.sha256) {
            let temp = directory.join(format!("{}.part", executable));
            fs::write(&temp, bytes).map_err(err)?;
            fs::rename(temp, &exe).map_err(err)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&exe, fs::Permissions::from_mode(0o755));
        }
        let runtime = runtime::probe(&exe, "node", "managed").ok_or("共享 Node 无法运行")?;
        if !runtime::compatible(&runtime, &requirement.kind, &requirement.minimum) {
            return Err("共享 Node 版本不兼容".into());
        }
        self.runtimes.insert(0, runtime.clone());
        Ok(runtime)
    }
    pub fn install(
        &mut self,
        id: &str,
        target: &str,
        scope: &str,
        project: &str,
    ) -> Result<Record> {
        let mut chain = Vec::new();
        self.install_skill(id, target, scope, project, &mut chain)
    }
    fn install_skill(
        &mut self,
        id: &str,
        target: &str,
        scope: &str,
        project: &str,
        chain: &mut Vec<String>,
    ) -> Result<Record> {
        if chain.iter().any(|entry| entry == id) {
            return Err(format!("共享依赖存在循环引用：{id}"));
        }
        chain.push(id.to_string());
        let outcome = (|| -> Result<Record> {
            let skill = self.skill(id)?;
            Self::require_installable(&skill)?;
            // Shared support packages are installed first so that the `../<id>/...`
            // references inside this Skill resolve next to it.
            for dependency in &skill.shared_dependencies {
                let shared = self.skill(dependency)?;
                Self::require_installable(&shared)?;
                let shared_path = self.target_path(&shared, target, scope, project)?;
                let adjacent = shared_path.parent().is_some_and(|parent| {
                    crate::installed::scan(parent, target, scope)
                        .iter()
                        .any(|record| crate::installed::matches(record, &shared))
                });
                if !adjacent && !shared_path.try_exists().map_err(err)? {
                    crate::logging::event(
                        "info",
                        "installer",
                        format!("先安装共享依赖：{dependency}"),
                    );
                    self.install_skill(dependency, target, scope, project, chain)?;
                }
            }
            self.install_package(skill, target, scope, project, chain.len() > 1)
        })();
        chain.pop();
        outcome
    }
    fn install_package(
        &mut self,
        skill: Skill,
        target: &str,
        scope: &str,
        project: &str,
        adjacent_only: bool,
    ) -> Result<Record> {
        let id = skill.id.clone();
        crate::logging::event("info", "installer", format!("检查安装目标与同名冲突：{id}"));
        let target_path = self.target_path(&skill, target, scope, project)?;
        if self
            .existing_skill(&skill, target, scope, &target_path)
            .filter(|record| {
                !adjacent_only
                    || Path::new(&record.path)
                        .parent()
                        .map(crate::installed::path_key)
                        == target_path.parent().map(crate::installed::path_key)
            })
            .is_some()
        {
            return Err("同名 Skill 已安装，无需重复安装。".into());
        }
        // Existing user Skills are never implicitly replaced.
        if target_path.try_exists().map_err(err)? {
            return Err("目标已存在。请保留现有 Skill，或改为安装到项目目录。工具管理的版本可先卸载后重装。".into());
        }
        let bytes = fs::read(self.package_path(&skill)).map_err(|_| "请先下载离线安装文件")?;
        let pack = validate_package(&bytes, &skill)?;
        if !crate::external_tools::cached(&self.data, &skill.tools) {
            return Err("请先下载此 Skill 需要的外部工具".into());
        }
        crate::logging::event("info", "installer", "安装包校验通过，检查并准备运行环境");
        self.runtimes = runtime::discover(&self.home, &self.data);
        self.prepare_python_for(&skill);
        let mut runtimes = vec![];
        for requirement in &skill.requirements {
            if requirement.kind == "python" && !skill.python_dependencies.is_empty() {
                let (profile, base) =
                    crate::python_dependencies::choose(&skill.python_dependencies, &self.runtimes)?;
                runtimes.push(crate::python_dependencies::environment(
                    &self.data, profile, base,
                )?);
            } else {
                runtimes.push(self.prepare_runtime(requirement)?);
            }
        }
        let record = Record {
            id: format!("{}-{}", skill.id, now()),
            skill_id: skill.id.clone(),
            title: skill.title,
            version: skill.version,
            revision: skill.revision,
            target: target.into(),
            scope: scope.into(),
            path: target_path.to_string_lossy().into(),
            installed_at: now(),
            runtimes: runtimes.clone(),
            package_sha256: skill.sha256,
            configuration: skill.configuration.clone(),
        };
        let parent = target_path.parent().ok_or("目标路径无效")?;
        fs::create_dir_all(parent).map_err(err)?;
        let stage = parent.join(format!(".cyapi-stage-{}", record.id));
        fs::create_dir(&stage).map_err(err)?;
        crate::logging::event(
            "info",
            "installer",
            format!(
                "暂存目录已创建：{}；准备写入 {} 个文件",
                stage.display(),
                pack.files.len()
            ),
        );
        let result = (|| {
            for file in &pack.files {
                let path = stage.join(&file.path);
                fs::create_dir_all(path.parent().ok_or("文件路径无效")?).map_err(err)?;
                fs::write(path, STANDARD.decode(&file.content).map_err(err)?).map_err(err)?;
            }
            if let Some(node) = runtimes.iter().find(|r| r.kind == "node") {
                self.write_launcher(
                    &stage,
                    &target_path,
                    node,
                    &skill.requirements,
                    &skill.tools,
                )?;
            }
            if let Some(python) = runtimes.iter().find(|r| r.kind == "python") {
                self.write_python_launcher(
                    &stage,
                    &target_path,
                    python,
                    &skill.requirements,
                    &skill.tools,
                )?;
            }
            atomic_json(&stage.join(".cyapi-install.json"), &record)?;
            crate::logging::event("info", "installer", "文件与启动器写入完成，保存恢复记录");
            atomic_json(
                &self.data.join("pending.json"),
                &json!({"kind":"install","record":record}),
            )?;
            // Recheck after staging. A failed rename leaves the existing directory intact.
            if target_path.try_exists().map_err(err)? {
                return Err("目标目录已被其他程序创建，请重新预览".into());
            }
            fs::rename(&stage, &target_path).map_err(err)?;
            crate::logging::event("info", "installer", "目录提交成功，保存安装记录");
            let mut records = self.records.clone();
            records.push(record.clone());
            if let Err(error) = atomic_json(&self.data.join("installed.json"), &records) {
                if let Err(rollback) = fs::rename(&target_path, &stage) {
                    crate::logging::event(
                        "error",
                        "installer",
                        format!("安装记录保存失败后的目录回滚失败：{rollback}"),
                    );
                }
                return Err(error);
            }
            self.records = records;
            let _ = fs::remove_file(self.data.join("pending.json"));
            Ok(record)
        })();
        if result.is_err() {
            crate::logging::event("warn", "installer", "安装失败，清理本次暂存对象");
            if let Err(error) = fs::remove_dir_all(&stage) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    crate::logging::event("error", "installer", format!("暂存清理失败：{error}"));
                }
            }
            if let Err(error) = fs::remove_file(self.data.join("pending.json")) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    crate::logging::event(
                        "error",
                        "installer",
                        format!("恢复记录清理失败：{error}"),
                    );
                }
            }
        }
        result
    }
    fn write_launcher(
        &self,
        stage: &Path,
        target: &Path,
        node: &Runtime,
        requirements: &[Requirement],
        tools: &[crate::external_tools::Tool],
    ) -> Result<()> {
        let helper = stage.join(".cyapi");
        fs::create_dir_all(&helper).map_err(err)?;
        let minimum = &requirements
            .iter()
            .find(|r| r.kind == "node")
            .ok_or("缺少 Node 声明")?
            .minimum;
        let quote = |value: &str| value.replace('\'', "''");
        let tool_environment = crate::external_tools::launcher(&self.data, tools)?;
        let script = crate::platform::launcher_script("node");
        let launcher = if cfg!(windows) {
            // This launcher does not require globally installing Node or changing PATH.
            // It also survives a Codex runtime being moved by looking for another compatible executable.
            format!(
                r#"$ErrorActionPreference = 'Stop'
{tool_environment}
$candidates = @('{recorded}')
$runtimeRoot = Join-Path $env:USERPROFILE '.cache\codex-runtimes'
if (Test-Path -LiteralPath $runtimeRoot) {{
  $candidates += @(Get-ChildItem -LiteralPath $runtimeRoot -Directory | Where-Object {{ $_.Name -notlike 'codex-runtime-install-*' }} | ForEach-Object {{ Join-Path $_.FullName 'dependencies\node\bin\node.exe' }})
}}
$managedRoot = '{managed}'
if (Test-Path -LiteralPath $managedRoot) {{ $candidates += @(Get-ChildItem -LiteralPath $managedRoot -Directory | ForEach-Object {{ Join-Path $_.FullName 'node.exe' }}) }}
$systemNode = Get-Command node.exe -ErrorAction SilentlyContinue
if ($systemNode) {{ $candidates += $systemNode.Source }}
foreach ($candidate in ($candidates | Select-Object -Unique)) {{
  if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {{ continue }}
  try {{ $versionText = & $candidate --version 2>$null; $version = [version]($versionText -replace '^v','') }} catch {{ continue }}
  if ($LASTEXITCODE -eq 0 -and $version -ge [version]'{minimum}') {{ & $candidate @args; exit $LASTEXITCODE }}
}}
throw 'No compatible Node runtime found. Open CYAPI Toolbox and repair/reinstall this Skill.'
"#,
                recorded = quote(&node.path),
                managed = quote(&self.data.join("runtimes/node").to_string_lossy()),
                minimum = minimum
            )
        } else {
            format!(
                r#"#!/bin/sh
# Generated by CYAPI Toolbox. Runs a compatible Node without changing the system PATH.
set -e
{tool_environment}
minimum='{minimum}'
candidates='{recorded}'

runtime_root="$HOME/.cache/codex-runtimes"
if [ -d "$runtime_root" ]; then
  for directory in "$runtime_root"/*; do
    [ -d "$directory" ] || continue
    case "${{directory##*/}}" in
      codex-runtime-install-*) continue ;;
    esac
    candidates="$candidates
$directory/dependencies/node/bin/node"
  done
fi

managed_root='{managed}'
if [ -d "$managed_root" ]; then
  for directory in "$managed_root"/*; do
    [ -d "$directory" ] || continue
    candidates="$candidates
$directory/node"
  done
fi

if command -v node >/dev/null 2>&1; then
  candidates="$candidates
$(command -v node)"
fi

# The list is newline separated so that paths may contain spaces.
old_ifs=$IFS
IFS='
'
set -f
for candidate in $candidates; do
  [ -n "$candidate" ] || continue
  [ -f "$candidate" ] || continue
  [ -x "$candidate" ] || continue
  version_text=$("$candidate" --version 2>/dev/null) || continue
  version=${{version_text#v}}
  if awk -v a="$version" -v b="$minimum" 'BEGIN {{
    n = split(a, x, "."); m = split(b, y, ".");
    ok = 1;
    for (i = 1; i <= 3; i++) {{
      xi = (i <= n) ? x[i] + 0 : 0;
      yi = (i <= m) ? y[i] + 0 : 0;
      if (xi > yi) {{ ok = 1; break }}
      if (xi < yi) {{ ok = 0; break }}
    }}
    exit(ok ? 0 : 1)
  }}'; then
    exec "$candidate" "$@"
  fi
done
set +f
IFS=$old_ifs

echo 'No compatible Node runtime found. Open CYAPI Toolbox and repair/reinstall this Skill.' >&2
exit 1
"#,
                recorded = crate::platform::shell_quote(&node.path),
                managed = crate::platform::shell_quote(
                    &self.data.join("runtimes/node").to_string_lossy()
                ),
                minimum = minimum
            )
        };
        let staged_launcher = helper.join(&script);
        fs::write(&staged_launcher, launcher).map_err(err)?;
        crate::configuration::make_executable(&staged_launcher)?;
        let launcher_path = target.join(".cyapi").join(&script).to_string_lossy().into_owned();
        let verify_path = target
            .join("scripts/verify-config.mjs")
            .to_string_lossy()
            .into_owned();
        let mut guide = if cfg!(windows) {
            format!("\n## Windows 运行环境（CYAPI Toolbox）\n\n本 Skill 已绑定共享 Node {}。Windows 上执行 `node ...` 命令时，可在 PowerShell 将 `node` 替换为 `powershell.exe -NoProfile -ExecutionPolicy Bypass -File '{}'`，脚本参数保持不变。复用此环境，无需重复安装 Node。Skill 另外要求的 CLI、第三方依赖及账号仍按其说明准备；不要假设已安装。\n", node.version, quote(&launcher_path))
        } else {
            format!("\n## macOS / Linux 运行环境（CYAPI Toolbox）\n\n本 Skill 已绑定共享 Node {}。执行 `node ...` 命令时，将 `node` 替换为 `sh '{}'`，脚本参数保持不变。复用此环境，无需重复安装 Node。Skill 另外要求的 CLI、第三方依赖及账号仍按其说明准备；不要假设已安装。\n", node.version, launcher_path)
        };
        if stage.join("scripts/verify-config.mjs").is_file() {
            if cfg!(windows) {
                guide.push_str(&format!("\n检查配置示例：\n\n```powershell\npowershell.exe -NoProfile -ExecutionPolicy Bypass -File '{}' '{}' --json\n```\n\n可在工具箱的配置入口填写此 Skill 声明的配置。\n", quote(&launcher_path), quote(&verify_path)));
            } else {
                guide.push_str(&format!("\n检查配置示例：\n\n```bash\nsh '{}' '{}' --json\n```\n\n可在工具箱的配置入口填写此 Skill 声明的配置。\n", launcher_path, verify_path));
            }
        }
        guide.push_str(&Self::tool_platform_note(tools));
        Self::prepend_runtime_guide(stage, &guide)
    }
    /// Warn when a Skill declares command line tools that only exist for
    /// another platform. The Skill still installs; the tool has to come from
    /// the system package manager instead.
    fn tool_platform_note(tools: &[crate::external_tools::Tool]) -> String {
        let unavailable = crate::external_tools::unavailable(tools);
        if unavailable.is_empty() {
            return String::new();
        }
        let names = unavailable
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join("、");
        format!(
            "\n> 注意：此 Skill 声明的外部工具（{}）只提供 {} 版本，当前系统为 {}。请自行通过系统包管理器安装，或按上游说明准备该工具。\n",
            names,
            unavailable
                .iter()
                .map(|t| t.platform.as_str())
                .collect::<Vec<_>>()
                .join("、"),
            crate::platform::current()
        )
    }
    fn write_python_launcher(
        &self,
        stage: &Path,
        target: &Path,
        python: &Runtime,
        requirements: &[Requirement],
        tools: &[crate::external_tools::Tool],
    ) -> Result<()> {
        let helper = stage.join(".cyapi");
        fs::create_dir_all(&helper).map_err(err)?;
        let minimum = &requirements
            .iter()
            .find(|r| r.kind == "python")
            .ok_or("缺少 Python 声明")?
            .minimum;
        let quote = |value: &str| value.replace('\'', "''");
        let tool_environment = crate::external_tools::launcher(&self.data, tools)?;
        let script = crate::platform::launcher_script("python");
        if python.source == "shared-venv" {
            let launcher = if cfg!(windows) {
                format!("$ErrorActionPreference = 'Stop'\n{tool_environment}\n$python = '{}'\nif (-not (Test-Path -LiteralPath $python -PathType Leaf)) {{ throw 'Python dependency environment is unavailable. Reinstall this Skill in CYAPI Toolbox.' }}\n& $python @args\nexit $LASTEXITCODE\n", quote(&python.path))
            } else {
                format!("#!/bin/sh\n# Generated by CYAPI Toolbox. Runs the isolated dependency environment.\nset -e\n{tool_environment}\npython='{path}'\nif [ ! -f \"$python\" ]; then\n  echo 'Python dependency environment is unavailable. Reinstall this Skill in CYAPI Toolbox.' >&2\n  exit 1\nfi\nexec \"$python\" \"$@\"\n", path = crate::platform::shell_quote(&python.path))
            };
            let staged_launcher = helper.join(&script);
            fs::write(&staged_launcher, launcher).map_err(err)?;
            crate::configuration::make_executable(&staged_launcher)?;
            let launcher_path = target.join(".cyapi").join(&script).to_string_lossy().into_owned();
            let guide = if cfg!(windows) {
                format!("\n## Windows Python（CYAPI Toolbox）\n\n使用共享 Python {} 的隔离依赖环境，已离线安装服务端锁定的 Python 依赖。执行本文所有 `python` / `python3` 命令时，请替换为 `powershell.exe -NoProfile -ExecutionPolicy Bypass -File '{}'`，保留脚本及参数。不要改用系统 Python，也不要重复安装 Python 或修改 Codex 全局包。服务端声明的命令行工具已绑定到此启动器。其他外部工具、浏览器及联网服务按上游说明使用。\n", python.version, quote(&launcher_path))
            } else {
                format!("\n## macOS / Linux Python（CYAPI Toolbox）\n\n使用共享 Python {} 的隔离依赖环境，已离线安装服务端锁定的 Python 依赖。执行本文所有 `python` / `python3` 命令时，请替换为 `sh '{}'`，保留脚本及参数。不要改用系统 Python，也不要重复安装 Python 或修改 Codex 全局包。服务端声明的命令行工具已绑定到此启动器。其他外部工具、浏览器及联网服务按上游说明使用。\n", python.version, launcher_path)
            };
            return Self::prepend_runtime_guide(stage, &guide);
        }
        let launcher = if cfg!(windows) {
            format!(
                r#"$ErrorActionPreference = 'Stop'
{tool_environment}
$candidates = @('{recorded}')
$runtimeRoot = Join-Path $env:USERPROFILE '.cache\codex-runtimes'
if (Test-Path -LiteralPath $runtimeRoot) {{
  $candidates += @(Get-ChildItem -LiteralPath $runtimeRoot -Directory | Where-Object {{ $_.Name -notlike 'codex-runtime-install-*' }} | ForEach-Object {{ Join-Path $_.FullName 'dependencies\python\python.exe' }})
}}
$systemPython = Get-Command python.exe -ErrorAction SilentlyContinue
if ($systemPython) {{ $candidates += $systemPython.Source }}
foreach ($candidate in ($candidates | Select-Object -Unique)) {{
  if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {{ continue }}
  try {{ $versionText = & $candidate --version 2>$null; $version = [version]($versionText -replace '^Python ','') }} catch {{ continue }}
  if ($LASTEXITCODE -eq 0 -and $version -ge [version]'{minimum}') {{ & $candidate @args; exit $LASTEXITCODE }}
}}
throw 'No compatible Python runtime found. Rescan runtimes in CYAPI Toolbox.'
"#,
                recorded = quote(&python.path),
                minimum = minimum
            )
        } else {
            format!(
                r#"#!/bin/sh
# Generated by CYAPI Toolbox. Runs a compatible Python without changing the system PATH.
set -e
{tool_environment}
minimum='{minimum}'
candidates='{recorded}'

runtime_root="$HOME/.cache/codex-runtimes"
if [ -d "$runtime_root" ]; then
  for directory in "$runtime_root"/*; do
    [ -d "$directory" ] || continue
    case "${{directory##*/}}" in
      codex-runtime-install-*) continue ;;
    esac
    candidates="$candidates
$directory/dependencies/python/bin/python3"
  done
fi

if command -v python3 >/dev/null 2>&1; then
  candidates="$candidates
$(command -v python3)"
fi

# The list is newline separated so that paths may contain spaces.
old_ifs=$IFS
IFS='
'
set -f
for candidate in $candidates; do
  [ -n "$candidate" ] || continue
  [ -f "$candidate" ] || continue
  [ -x "$candidate" ] || continue
  version_text=$("$candidate" --version 2>/dev/null) || continue
  version=${{version_text#Python }}
  if awk -v a="$version" -v b="$minimum" 'BEGIN {{
    n = split(a, x, "."); m = split(b, y, ".");
    ok = 1;
    for (i = 1; i <= 3; i++) {{
      xi = (i <= n) ? x[i] + 0 : 0;
      yi = (i <= m) ? y[i] + 0 : 0;
      if (xi > yi) {{ ok = 1; break }}
      if (xi < yi) {{ ok = 0; break }}
    }}
    exit(ok ? 0 : 1)
  }}'; then
    exec "$candidate" "$@"
  fi
done
set +f
IFS=$old_ifs

echo 'No compatible Python runtime found. Rescan runtimes in CYAPI Toolbox.' >&2
exit 1
"#,
                recorded = crate::platform::shell_quote(&python.path),
                minimum = minimum
            )
        };
        let staged_launcher = helper.join(&script);
        fs::write(&staged_launcher, launcher).map_err(err)?;
        crate::configuration::make_executable(&staged_launcher)?;
        let launcher_path = target.join(".cyapi").join(&script).to_string_lossy().into_owned();
        let guide = if cfg!(windows) {
            format!("\n## Windows Python（CYAPI Toolbox）\n\n已复用 Python {}。执行本文的 `python` / `python3` 命令时，可在 PowerShell 替换为 `powershell.exe -NoProfile -ExecutionPolicy Bypass -File '{}'`，保留原有脚本与参数。无需重复安装 Python。\n\n本次安装包含 Skill 文件与运行时绑定；第三方 Python 包、浏览器、外部程序及 API 账号不代表已经配置。缺少第三方包时，使用复用的解释器创建所需虚拟环境，勿修改 Codex 自带 Python 的全局依赖。\n", python.version, quote(&launcher_path))
        } else {
            format!("\n## macOS / Linux Python（CYAPI Toolbox）\n\n已复用 Python {}。执行本文的 `python` / `python3` 命令时，请替换为 `sh '{}'`，保留原有脚本与参数。无需重复安装 Python。\n\n本次安装包含 Skill 文件与运行时绑定；第三方 Python 包、浏览器、外部程序及 API 账号不代表已经配置。缺少第三方包时，使用复用的解释器创建所需虚拟环境，勿修改 Codex 自带 Python 的全局依赖。\n", python.version, launcher_path)
        };
        Self::prepend_runtime_guide(stage, &guide)
    }
    fn prepend_runtime_guide(stage: &Path, guide: &str) -> Result<()> {
        let skill_path = stage.join("SKILL.md");
        let original = fs::read_to_string(&skill_path).map_err(err)?;
        // Preserve YAML frontmatter; insert operational guidance before the original body.
        let end = original
            .strip_prefix("---")
            .and_then(|rest| rest.find("\n---").map(|i| i + 7))
            .ok_or("SKILL.md frontmatter 无效")?;
        let updated = format!("{}\n{}\n{}", &original[..end], guide, &original[end..]);
        fs::write(skill_path, updated).map_err(err)
    }
    /// Replace a managed installation with the version the catalog publishes now.
    /// The previous directory is moved aside first, so a failed update restores it.
    pub fn update(&mut self, id: &str, target: &str, scope: &str, project: &str) -> Result<Record> {
        crate::logging::event("info", "installer", format!("检查 Skill 更新：{id}"));
        let skill = self.skill(id)?;
        Self::require_installable(&skill)?;
        let target_path = self.target_path(&skill, target, scope, project)?;
        let record = self
            .existing_skill(&skill, target, scope, &target_path)
            .ok_or("此 Skill 尚未安装")?;
        if !Self::owns(&record) {
            return Err("此 Skill 不是工具箱管理的版本，请先手动检查后再更新".into());
        }
        if !record.package_sha256.is_empty() && record.package_sha256 == skill.sha256 {
            return Err("此 Skill 已是最新版本".into());
        }
        if !self.package_path(&skill).exists() {
            self.download(id)?;
        }
        let destination = self.data.join("backups").join(&record.id);
        fs::create_dir_all(destination.parent().ok_or("备份路径无效")?).map_err(err)?;
        fs::rename(&record.path, &destination)
            .map_err(|_| "无法备份已安装的 Skill。请关闭占用文件的程序后再更新。".to_string())?;
        let records: Vec<_> = self
            .records
            .iter()
            .filter(|entry| entry.id != record.id)
            .cloned()
            .collect();
        let previous = std::mem::replace(&mut self.records, records);
        match self.install_package(skill, target, scope, project, false) {
            Ok(installed) => {
                crate::logging::event(
                    "info",
                    "installer",
                    format!("更新完成，旧版本已备份到 {}", destination.display()),
                );
                Ok(installed)
            }
            Err(error) => {
                // Put the previous version back so the Skill keeps working.
                let _ = fs::rename(&destination, &record.path);
                self.records = previous;
                let _ = atomic_json(&self.data.join("installed.json"), &self.records);
                crate::logging::event(
                    "warn",
                    "installer",
                    format!("更新失败，已恢复原版本：{error}"),
                );
                Err(error)
            }
        }
    }

    pub fn remove(&mut self, id: &str) -> Result<String> {
        crate::logging::event(
            "info",
            "installer",
            "检查安装归属，准备将 Skill 移到备份目录",
        );
        let record = self
            .records
            .iter()
            .find(|r| r.id == id)
            .ok_or("找不到安装记录")?
            .clone();
        let destination = self.data.join("backups").join(&record.id);
        if !Self::owns(&record) {
            return Err("Skill 已被移动、删除或管理标记变化，已保留文件，请手动检查目录".into());
        }
        atomic_json(
            &self.data.join("pending.json"),
            &json!({"kind":"remove","record":record}),
        )?;
        fs::rename(&record.path,&destination).map_err(|_|"无法移动到备份目录。请关闭占用文件的程序；当前版本要求 Skill 与数据目录位于同一磁盘分区。")?;
        let records: Vec<_> = self
            .records
            .iter()
            .filter(|r| r.id != id)
            .cloned()
            .collect();
        if let Err(error) = atomic_json(&self.data.join("installed.json"), &records) {
            if let Err(rollback) = fs::rename(&destination, &record.path) {
                crate::logging::event(
                    "error",
                    "installer",
                    format!("卸载记录保存失败后的目录回滚失败：{rollback}"),
                );
            }
            return Err(error);
        }
        self.records = records;
        let _ = fs::remove_file(self.data.join("pending.json"));
        Ok(destination.to_string_lossy().into())
    }
    fn configuration(&self, request: &Request) -> Result<Value> {
        use crate::configuration::{self, Context};
        let (flow, skill_path) = if let Some(id) = &request.record_id {
            let record = self
                .records
                .iter()
                .find(|r| &r.id == id)
                .ok_or("未找到安装记录")?;
            if !Self::owns(record) {
                return Err("Skill 文件已变化，请先检查安装位置".into());
            }
            let flow = self
                .skill(&record.skill_id)
                .ok()
                .filter(|s| s.revision == record.revision)
                .and_then(|s| s.configuration)
                .or_else(|| record.configuration.clone())
                .ok_or("此 Skill 未声明配置流程")?;
            (flow, Some(PathBuf::from(&record.path)))
        } else {
            (
                self.skill(request.skill_id.as_deref().unwrap_or(""))?
                    .configuration
                    .ok_or("此 Skill 未声明配置流程")?,
                None,
            )
        };
        // An isolated home must also isolate the application data during tests and portable smoke runs.
        let isolated = env::var_os("CYAPI_TOOLBOX_HOME_DIR").is_some()
            || !crate::platform::is_default_home(&self.home);
        let context = Context {
            home: self.home.clone(),
            app_data: crate::platform::app_data_dir(&self.home, isolated),
            local_app_data: crate::platform::local_app_data_dir(&self.home, isolated),
            skill: skill_path,
        };
        if request.action == "configuration-read" {
            configuration::view(&flow, &context)
        } else {
            configuration::save(
                &flow,
                &context,
                request.values.as_ref().ok_or("缺少配置值")?,
                request.flow_revision.as_deref().ok_or("缺少流程版本")?,
            )
        }
    }
    pub fn dispatch(&mut self, request: Request) -> Result<Value> {
        let id = request.skill_id.as_deref().unwrap_or("");
        let target = request.target.as_deref().unwrap_or("codex");
        let scope = request.scope.as_deref().unwrap_or("global");
        let project = request.project_path.as_deref().unwrap_or("");
        match request.action.as_str() {
            "integrations-read"
            | "integrations-refresh"
            | "integration-connect"
            | "integration-adopt"
            | "integration-disconnect"
            | "integration-action"
            | "integration-open-download" => {
                return self.integration_request(&request);
            }
            "mcp-view" | "mcp-catalog-read" | "mcp-refresh" | "mcp-install" | "mcp-adopt"
            | "mcp-remove" | "mcp-enable" | "mcp-disable" | "mcp-local-save"
            | "mcp-local-remove" => {
                // The catalog bytes are fetched before the service lock is taken,
                // so a manual refresh never blocks the rest of the app.
                if request.action == "mcp-refresh" && crate::mcp::catalog(&self.data).is_err() {
                    let fetched = crate::service::fetch_catalog_conditional(
                        self.server_url(),
                        &self.data,
                        "/api/mcp",
                    )?;
                    self.apply_mcp(fetched)?;
                }
                let workspace = request.workspace.as_deref().unwrap_or("codex");
                let context = self.mcp_context();
                match request.action.as_str() {
                    "mcp-local-save" => {
                        let values = request.values.clone().unwrap_or_default();
                        let descriptor = crate::mcp::local_descriptor(&values)?;
                        crate::mcp::save_local(&self.data, descriptor)?;
                        return crate::mcp::view(&self.data, &context, workspace);
                    }
                    "mcp-local-remove" => {
                        let id = request.server_id.as_deref().ok_or("缺少 MCP 标识")?;
                        crate::mcp::remove_local(&self.data, id)?;
                        return crate::mcp::view(&self.data, &context, workspace);
                    }
                    _ => {}
                }
                if !matches!(
                    request.action.as_str(),
                    "mcp-view" | "mcp-catalog-read" | "mcp-refresh"
                ) {
                    let id = request.server_id.as_deref().ok_or("缺少 MCP 标识")?;
                    let revision = request
                        .configuration_revision
                        .as_deref()
                        .ok_or("缺少配置版本，请重新检测")?;
                    match request.action.as_str() {
                        "mcp-install" => {
                            crate::mcp::install(&self.data, &context, workspace, id, revision)?
                        }
                        "mcp-adopt" => {
                            crate::mcp::adopt(&self.data, &context, workspace, id, revision)?
                        }
                        "mcp-remove" => {
                            crate::mcp::remove(&self.data, &context, workspace, id, revision)?
                        }
                        "mcp-enable" => crate::mcp::toggle_enabled(
                            &self.data, &context, workspace, id, revision, true,
                        )?,
                        "mcp-disable" => crate::mcp::toggle_enabled(
                            &self.data, &context, workspace, id, revision, false,
                        )?,
                        _ => unreachable!(),
                    }
                }
                return crate::mcp::view(&self.data, &context, workspace);
            }
            "codex-read" => {
                return crate::codex_config::editor_view(&self.codex_home(), &self.data)
            }
            "codex-save" => {
                crate::codex_config::save(
                    &self.codex_home(),
                    &self.data,
                    request.codex.as_ref().ok_or("缺少 Codex 配置")?,
                )?;
                return crate::codex_config::editor_view(&self.codex_home(), &self.data);
            }
            "codex-restore" => {
                crate::codex_config::restore(
                    &self.codex_home(),
                    &self.data,
                    request
                        .configuration_revision
                        .as_deref()
                        .ok_or("缺少配置版本")?,
                )?;
                return crate::codex_config::editor_view(&self.codex_home(), &self.data);
            }
            "claude-read" => {
                return crate::claude_config::editor_view(&self.claude_home(), &self.data)
            }
            "claude-save" => {
                crate::claude_config::save(
                    &self.claude_home(),
                    &self.data,
                    request.claude.as_ref().ok_or("缺少 Claude Code 配置")?,
                )?;
                return crate::claude_config::editor_view(&self.claude_home(), &self.data);
            }
            "claude-restore" => {
                crate::claude_config::restore(
                    &self.claude_home(),
                    &self.data,
                    request
                        .configuration_revision
                        .as_deref()
                        .ok_or("缺少配置版本")?,
                )?;
                return crate::claude_config::editor_view(&self.claude_home(), &self.data);
            }
            "opencode-read" => {
                return crate::opencode_config::editor_view_at(
                    &self.opencode_location(),
                    &self.data,
                )
            }
            "models-list" => {
                return crate::model_list::list(
                    request.base_url.as_deref().ok_or("缺少 API 地址")?,
                    request.api_key.as_deref().unwrap_or(""),
                    request.model_api.as_deref().unwrap_or("openai"),
                )
                .and_then(|list| serde_json::to_value(list).map_err(err));
            }
            "opencode-save" => {
                crate::opencode_config::save_at(
                    &self.opencode_location(),
                    &self.data,
                    request.opencode.as_ref().ok_or("缺少 OpenCode 配置")?,
                )?;
                return crate::opencode_config::editor_view_at(
                    &self.opencode_location(),
                    &self.data,
                );
            }
            "opencode-restore" => {
                crate::opencode_config::restore_at(
                    &self.opencode_location(),
                    &self.data,
                    request
                        .configuration_revision
                        .as_deref()
                        .ok_or("缺少配置版本")?,
                )?;
                return crate::opencode_config::editor_view_at(
                    &self.opencode_location(),
                    &self.data,
                );
            }
            "configuration-read" | "configuration-save" => return self.configuration(&request),
            "status" | "catalog-read" => {}
            "rescan" => {
                self.runtimes = runtime::discover(&self.home, &self.data);
            }
            "refresh" => {
                let _ = self.refresh();
            }
            "download" => {
                crate::cancel::begin();
                let result = self.download(id);
                crate::cancel::end();
                result?;
            }
            "preview" => return self.preview(id, target, scope, project),
            "install" => {
                crate::cancel::begin();
                let result = self.install(id, target, scope, project);
                crate::cancel::end();
                return result.and_then(|r| serde_json::to_value(r).map_err(err));
            }
            "update-skill" => {
                crate::cancel::begin();
                let result = self.update(id, target, scope, project);
                crate::cancel::end();
                return result.and_then(|r| serde_json::to_value(r).map_err(err));
            }
            "remove" => {
                return self
                    .remove(request.record_id.as_deref().unwrap_or(""))
                    .map(|backup| json!({"backupPath":backup}))
            }
            "update-check" => {
                let online = crate::update::fetch_manifest(self.server_url());
                return crate::update::check(self, online);
            }
            "update-download" => return crate::update::download(self),
            "update-install" => return crate::update::install(self),
            "open-url" => {
                return crate::update::open(request.url.as_deref().ok_or("缺少要打开的地址")?)
            }
            _ => return Err("未知操作".into()),
        }
        Ok(self.status())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloudflare_fallback_covers_the_project_hosts_only() {
        let ok = |url: &str| {
            assert!(can_bypass_cloudflare(&Url::parse(url).unwrap()), "{url}");
        };
        ok("https://api.cyapi.cyou/api/catalog");
        ok("https://tool.cyapi.cyou/downloads/app.exe");
        for url in [
            "http://api.cyapi.cyou/api/catalog",
            "https://api.cyapi.cyou:8443/api/catalog",
            "https://www.cyapi.cyou/v1/models",
            "https://api.cyapi.cyou.evil.example/api/catalog",
            "https://user:pass@api.cyapi.cyou/api/catalog",
        ] {
            assert!(
                !can_bypass_cloudflare(&Url::parse(url).unwrap()),
                "不应回退：{url}"
            );
        }
    }

    #[test]
    fn skill_groups_survive_refresh_cache_restart_and_metadata_updates() {
        let root = env::temp_dir().join(format!("cyapi-skill-groups-{}", uuid::Uuid::new_v4()));
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        let (grouped, _) = fixture();
        let (standalone, _) = fixture_for("standalone");
        let mut payload = json!({
            "schema": 2, "generatedAt": "test", "skills": [grouped, standalone], "runtimes": [],
            "skillGroups": [{
                "id": "custom-suite", "title": "Server suite", "description": "Server description",
                "sourceUrl": "https://example.com/suite", "items": [{"skillId": "test"}]
            }]
        });
        let fetched = |value: &Value| Fetched {
            modified: true, etag: None, bytes: serde_json::to_vec(value).unwrap(),
        };
        service.apply_refresh(Ok(fetched(&payload))).unwrap();
        assert_eq!(service.status()["skillGroups"], payload["skillGroups"]);
        assert_eq!(service.status()["skills"].as_array().unwrap().len(), 2);
        let cached: Catalog = read_json(&root.join("data/catalog.json")).unwrap();
        assert_eq!(cached.skill_groups[0].items[0].skill_id, "test");
        drop(service);
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        assert_eq!(service.status()["skillGroups"][0]["title"], "Server suite");
        payload["skillGroups"][0]["title"] = json!("Renamed by server");
        payload["skillGroups"][0]["items"] = json!([{"skillId":"standalone"}]);
        service.apply_refresh(Ok(fetched(&payload))).unwrap();
        assert_eq!(service.status()["skillGroups"], payload["skillGroups"]);
        let mut invalid = payload.clone();
        invalid["skillGroups"][0]["items"] = json!([{"skillId":"missing"}]);
        assert!(service.apply_refresh(Ok(fetched(&invalid))).is_err());
        assert_eq!(service.status()["skillGroups"], payload["skillGroups"]);
        let cached: Value = read_json(&root.join("data/catalog.json")).unwrap();
        assert_eq!(cached["skillGroups"], payload["skillGroups"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_catalogs_keep_standalone_items_and_invalid_groups_are_rejected() {
        let (skill, _) = fixture();
        let legacy = json!({"schema": 1, "generatedAt": "test", "skills": [skill], "runtimes": []});
        let mut catalog: Catalog = serde_json::from_value(legacy).unwrap();
        assert!(catalog.skill_groups.is_empty());
        validate_catalog(&catalog).unwrap();
        let group: SkillGroup = serde_json::from_value(json!({
            "id":"custom-suite", "title":"Suite", "description":"Description",
            "sourceUrl":"https://example.com", "items":[{"skillId":"test"}]
        })).unwrap();
        catalog.skill_groups = vec![group.clone()];
        validate_catalog(&catalog).unwrap();
        for patch in [
            json!({"id":"../invalid"}), json!({"title":""}),
            json!({"sourceUrl":"http://example.com"}),
            json!({"sourceUrl":"https://user:secret@example.com"}),
            json!({"items":[]}), json!({"items":[{"skillId":"missing"}]}),
            json!({"items":[{"skillId":"test"},{"skillId":"test"}]}),
        ] {
            let mut value = serde_json::to_value(&group).unwrap();
            value.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
            catalog.skill_groups = vec![serde_json::from_value(value).unwrap()];
            assert!(validate_catalog(&catalog).is_err());
        }
        catalog.skill_groups = vec![group.clone(), group.clone()];
        assert!(validate_catalog(&catalog).is_err());
        let mut overlapping = group.clone();
        overlapping.id = "another-suite".into();
        catalog.skill_groups = vec![group, overlapping];
        assert!(validate_catalog(&catalog).is_err());
    }

    #[test]
    fn catalog_refresh_keeps_offline_data_and_renews_unchanged_catalogs() {
        let root = env::temp_dir().join(format!("cyapi-catalog-refresh-{}", uuid::Uuid::new_v4()));
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        let examples = [
            (
                "/api/catalog",
                "catalog.json",
                json!({"schema":2,"generatedAt":"test","skills":[],"runtimes":[]}),
            ),
            (
                "/api/mcp",
                "mcp/catalog.json",
                json!({"schema":1,"generatedAt":"test","servers":[]}),
            ),
            (
                "/api/integrations",
                "integrations/catalog.json",
                json!({"schema":4,"integrations":[{
                    "id":"blender","name":"Blender","minimumVersion":"3.0",
                    "downloadUrl":"https://www.blender.org/download/","capabilities":[],"steps":[],"skillId":"blender-director"
                }]}),
            ),
        ];
        for (endpoint, cache, manifest) in examples {
            // With no usable cache, an automatic network failure is still an error.
            assert!(service
                .apply_catalog_read(endpoint, Err("offline".into()), true)
                .is_err());
            service
                .apply_catalog_read(
                    endpoint,
                    Ok(Fetched {
                        modified: true,
                        etag: Some("\"test\"".into()),
                        bytes: serde_json::to_vec(&manifest).unwrap(),
                    }),
                    true,
                )
                .unwrap();
            let bytes = fs::read(service.data.join(cache)).unwrap();
            let modified = fs::metadata(service.data.join(cache))
                .unwrap()
                .modified()
                .unwrap();
            let checked = service.data.join(format!("{cache}.checked-at"));
            fs::write(&checked, "0").unwrap();
            service
                .apply_catalog_read(
                    endpoint,
                    Ok(Fetched {
                        modified: false,
                        etag: Some("\"test\"".into()),
                        bytes: vec![],
                    }),
                    true,
                )
                .unwrap();
            assert!(
                fs::read_to_string(&checked)
                    .unwrap()
                    .parse::<u64>()
                    .unwrap()
                    > 0
            );
            assert_eq!(
                fs::metadata(service.data.join(cache))
                    .unwrap()
                    .modified()
                    .unwrap(),
                modified
            );
            assert!(!claim_catalog_refresh(&service.data, endpoint, false));
            assert!(service
                .apply_catalog_read(endpoint, Err("offline".into()), true)
                .is_ok());
            assert!(service
                .apply_catalog_read(endpoint, Err("offline".into()), false)
                .is_err());
            // Invalid remote data must not overwrite a valid cached catalog.
            assert!(service
                .apply_catalog_read(
                    endpoint,
                    Ok(Fetched {
                        modified: true,
                        etag: None,
                        bytes: b"bad manifest".to_vec(),
                    }),
                    true
                )
                .is_ok());
            assert_eq!(fs::read(service.data.join(cache)).unwrap(), bytes);
            assert!(claim_catalog_refresh(&service.data, endpoint, true));
        }
        let manifest = crate::integrations::catalog(&service.data).unwrap();
        assert!(manifest
            .integrations
            .iter()
            .any(|item| item.id == "blender"));
        // Refreshing software entries must not depend on the Skill/MCP server.
        service.settings.server_url = "http://127.0.0.1:1".into();
        let request =
            serde_json::from_value(json!({"action":"integrations-refresh","workspace":"codex"}))
                .unwrap();
        let view = service.dispatch(request).unwrap();
        assert_eq!(view["manifest"]["integrations"][0]["id"], "blender");
        // A recent timestamp must not hide missing or corrupt local data.
        fs::write(service.data.join("integrations/catalog.json"), b"corrupt").unwrap();
        assert!(claim_catalog_refresh(
            &service.data,
            "/api/integrations",
            false
        ));
        fs::remove_dir_all(root).unwrap();
    }

    /// Run a generated launcher script the way the install guide tells the
    /// user to run it. Windows uses PowerShell, every other platform uses sh.
    fn launcher_command(script: &Path) -> std::process::Command {
        let mut command = if cfg!(windows) {
            let mut command = std::process::Command::new("powershell.exe");
            command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
            command
        } else {
            std::process::Command::new("sh")
        };
        command.arg(script);
        command
    }

    /// The name of the launcher script for a runtime on this platform.
    fn launcher_name(kind: &str) -> String {
        crate::platform::launcher_script(kind)
    }

    #[test]
    fn runtime_launchers_are_staged_before_the_install_directory_exists() {
        let root = env::temp_dir().join(format!("cyapi-launcher-stage-{}", uuid::Uuid::new_v4()));
        let service = Service::new(root.join("data"), root.join("home")).unwrap();
        let (skill, _) = fixture();
        for (kind, source, minimum) in [
            ("node", "system", "20.0.0"),
            ("python", "system", "3.14.0"),
            ("python", "shared-venv", "3.14.0"),
        ] {
            let stage = root.join(format!("stage-{kind}-{source}"));
            let target = root.join(format!("installed-{kind}-{source}"));
            fs::create_dir(&stage).unwrap();
            fs::write(stage.join("SKILL.md"), &skill.markdown).unwrap();
            let runtime = Runtime {
                kind: kind.into(),
                version: minimum.into(),
                path: root.join("runtime").join(crate::platform::executable(kind))
                    .to_string_lossy().into_owned(),
                source: source.into(),
            };
            let requirements = [Requirement { kind: kind.into(), minimum: minimum.into() }];
            if kind == "node" {
                service.write_launcher(&stage, &target, &runtime, &requirements, &[]).unwrap();
            } else {
                service.write_python_launcher(&stage, &target, &runtime, &requirements, &[]).unwrap();
            }
            assert!(!target.exists(), "Launcher generation must not publish an incomplete Skill");
            let launcher = Path::new(".cyapi").join(launcher_name(kind));
            assert!(stage.join(&launcher).is_file(), "{kind}/{source}");
            let markdown = fs::read_to_string(stage.join("SKILL.md")).unwrap();
            assert!(markdown.starts_with("---\n"));
            assert!(markdown.contains(target.join(&launcher).to_str().unwrap()));
            assert!(!markdown.contains(stage.to_str().unwrap()), "The guide must survive the directory rename");
            fs::rename(&stage, &target).unwrap();
            assert!(target.join(&launcher).is_file());
            assert!(!stage.exists());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "run npm run verify:managed-python in server/ with a prepared catalog"]
    fn integration_skill_installs_python_dependencies_with_the_shared_runtime() {
        let root = env::temp_dir().join(format!("cyapi-managed-python-{}", now()));
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        service.settings.server_url = env::var("CYAPI_TEST_SERVER")
            .expect("use server/scripts/verify-managed-python.mjs");
        service.refresh().unwrap();
        // The Skill needs an offline dependency bundle for CPython 3.14.
        let skill = service
            .skill("nature-paper-trans")
            .expect("the catalog holds the pinned Skill");
        let profile = skill
            .python_dependencies
            .iter()
            .find(|profile| profile.target.platform == crate::platform::current())
            .cloned()
            .expect("the catalog holds a profile for this platform");
        assert_eq!(profile.target.python, "3.14");
        // The package and the offline dependency bundle are downloaded first.
        service.download("nature-paper-trans").unwrap();
        // The client prepares the interpreter the bundle needs, so the machine
        // does not have to provide one.
        service.prepare_python_for(&skill);
        let (_, base) =
            crate::python_dependencies::choose(std::slice::from_ref(&profile), &service.runtimes)
                .expect("the interpreter of the bundle is available");
        assert_eq!(base.kind, "python");
        let environment = crate::python_dependencies::environment(&service.data, &profile, &base);
        assert!(
            environment.is_ok(),
            "the offline dependencies are installed: {:?}",
            environment.err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "run npm run verify:jianying in server/ with a prepared catalog and Python 3.12"]
    fn integration_skill_installs_offline_using_upstream_code() {
        let root = env::temp_dir().join(format!("cyapi-upstream-skill-{}", now()));
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        service.settings.server_url =
            env::var("CYAPI_TEST_SERVER").expect("use server/scripts/verify-jianying-skill.mjs");
        service.refresh().unwrap();
        let catalog = service.fetch("/api/integrations", 100_000, 8).unwrap();
        crate::integrations::cache(&service.data, &catalog).unwrap();
        service.download("jianying-editor").unwrap();
        assert!(service.ready(&service.skill("jianying-editor").unwrap()));
        // Every subsequent installation step must succeed with the catalog server unreachable.
        service.settings.server_url = "http://127.0.0.1:1".into();
        let request: Request = serde_json::from_value(
            json!({"action":"integration-connect","integrationId":"jianying"}),
        )
        .unwrap();
        let view = service.dispatch(request.clone()).unwrap();
        let entry = view["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == "jianying")
            .unwrap();
        assert_eq!(entry["registered"], true);
        assert!(!service.codex_home().join("config.toml").exists());
        let record = service
            .records
            .iter()
            .find(|r| r.skill_id == "jianying-editor")
            .unwrap()
            .clone();
        assert_eq!(record.runtimes[0].source, "shared-venv");
        assert!(record.runtimes[0].version.starts_with("3.12."));
        let skill_path = PathBuf::from(&record.path);
        let script = root.join("verify_upstream.py");
        fs::write(&script, r#"import sys,json
from pathlib import Path
skill, output = map(Path, sys.argv[1:3])
sys.path.insert(0, str(skill / 'scripts'))
from jy_wrapper import JyProject
project = JyProject('CYAPI-Upstream-Test', width=1080, height=1920, drafts_root=str(output), overwrite=False)
assert project.add_media_safe(str(skill / 'assets/cover.png'), '0s', '2s') is not None
assert project.add_media_safe(str(skill / 'assets/video.mp4'), '2s', '3s') is not None
assert project.add_media_safe(str(skill / 'assets/audio.mp3'), '0s', '5s', track_name='AudioTrack') is not None
project.add_text_simple('CYAPI upstream Skill test', '0s', '2s')
result = project.save()
draft = Path(result['draft_path'])
content = json.loads((draft / 'draft_info.json').read_text(encoding='utf-8'))
assert len(content['tracks']) >= 3
assert (draft / 'materials').is_dir()
print(json.dumps({'upstreamDraft': str(draft), 'tracks': len(content['tracks'])}))
"#).unwrap();
        let output = launcher_command(&skill_path.join(".cyapi").join(launcher_name("python")))
            .arg(&script)
            .arg(&skill_path)
            .arg(root.join("drafts"))
            .env("PYTHONIOENCODING", "utf-8")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        println!("{}", String::from_utf8_lossy(&output.stdout));
        let python = &record.runtimes[0];
        let profile = &service
            .skill("jianying-editor")
            .unwrap()
            .python_dependencies[0]
            .clone();
        let (_, base) =
            crate::python_dependencies::choose(std::slice::from_ref(profile), &service.runtimes)
                .unwrap();
        let reused = crate::python_dependencies::environment(&service.data, profile, base).unwrap();
        assert_eq!(python.path, reused.path);
        service.dispatch(request).unwrap();
        assert_eq!(service.records.len(), 1);
        service
            .dispatch(
                serde_json::from_value(
                    json!({"action":"integration-disconnect","integrationId":"jianying"}),
                )
                .unwrap(),
            )
            .unwrap();
        assert!(!skill_path.exists());
        assert!(
            Path::new(&python.path).is_file(),
            "shared environment survives Skill uninstall"
        );
        assert!(root
            .join("data/backups")
            .join(&record.id)
            .join("SKILL.md")
            .is_file());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn python_preview_requires_the_prepared_abi_instead_of_any_newer_runtime() {
        let root = env::temp_dir().join(format!("cyapi-python-preview-{}", uuid::Uuid::new_v4()));
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        let (mut skill, _) = fixture_for("nature-academic-search");
        skill.requirements = vec![Requirement { kind: "python".into(), minimum: "3.12.0".into() }];
        skill.python_dependencies = vec![serde_json::from_value(json!({
            "id":"nature-academic-search",
            "target":{"implementation":"cpython","python":"3.12","abi":"cp312","platform":crate::platform::current()},
            "path":format!("/python/bundles/{}/manifest.json", "a".repeat(64)),
            "sha256":"a".repeat(64),"size":1
        })).unwrap()];
        service.catalog = Some(Catalog {
            skill_groups: vec![],
            schema: 2, generated_at: "test".into(), skills: vec![skill], runtimes: vec![],
        });
        service.runtimes = vec![Runtime {
            kind: "python".into(), version: "3.14.0".into(), path: "python314".into(), source: "system".into(),
        }];
        let blocked = service.preview("nature-academic-search", "codex", "global", "").unwrap();
        assert!(blocked["runtimes"][0]["reuse"].is_null());
        assert!(blocked["runtimes"][0]["fallback"].is_null());
        assert!(blocked["runtimes"][0]["error"].as_str().unwrap().contains("Python 3.12"));
        service.runtimes.push(Runtime {
            kind: "python".into(), version: "3.12.14".into(), path: "python312".into(), source: "codex".into(),
        });
        let ready = service.preview("nature-academic-search", "codex", "global", "").unwrap();
        assert_eq!(ready["runtimes"][0]["reuse"]["path"], "python312");
        assert!(ready["runtimes"][0]["error"].is_null());
        fs::remove_dir_all(root).unwrap();
    }

    fn fixture() -> (Skill, Vec<u8>) {
        let content = b"---\nname: test\ndescription: test\n---\nTest";
        let pack = SkillPack {
            schema: 1,
            id: "test".into(),
            version: "1".into(),
            revision: "abc".into(),
            requirements: vec![],
            files: vec![PackageFile {
                path: "SKILL.md".into(),
                size: content.len(),
                sha256: hash(content),
                content: STANDARD.encode(content),
            }],
        };
        let bytes = serde_json::to_vec(&pack).unwrap();
        (
            Skill {
                id: "test".into(),
                title: "Test".into(),
                description: "Test".into(),
                version: "1".into(),
                revision: "abc".into(),
                source_url: "https://example.com".into(),
                sha256: hash(&bytes),
                size: bytes.len() as u64,
                file_count: 1,
                package_path: format!("/packages/{}.skillpack.json", hash(&bytes)),
                requirements: vec![],
                markdown: String::from_utf8(content.to_vec()).unwrap(),
                category: "开发与工程".into(),
                setup_notes: vec![],
                license: "MIT".into(),
                distribution: "bundled".into(),
                distribution_reason: None,
                recommended: false,
                configuration: None,
                python_dependencies: vec![],
                tools: vec![],
                shared_dependencies: vec![],
            },
            bytes,
        )
    }
    fn fixture_for(id: &str) -> (Skill, Vec<u8>) {
        let content = format!("---\nname: {id}\ndescription: test\n---\nTest").into_bytes();
        let pack = SkillPack {
            schema: 1,
            id: id.into(),
            version: "1".into(),
            revision: "abc".into(),
            requirements: vec![],
            files: vec![PackageFile {
                path: "SKILL.md".into(),
                size: content.len(),
                sha256: hash(&content),
                content: STANDARD.encode(&content),
            }],
        };
        let bytes = serde_json::to_vec(&pack).unwrap();
        (
            Skill {
                id: id.into(),
                title: id.into(),
                description: "Test".into(),
                version: "1".into(),
                revision: "abc".into(),
                source_url: "https://example.com".into(),
                sha256: hash(&bytes),
                size: bytes.len() as u64,
                file_count: 1,
                package_path: format!("/packages/{}.skillpack.json", hash(&bytes)),
                requirements: vec![],
                markdown: String::from_utf8(content.to_vec()).unwrap(),
                category: "开发与工程".into(),
                setup_notes: vec![],
                license: "MIT".into(),
                distribution: "bundled".into(),
                distribution_reason: None,
                recommended: false,
                configuration: None,
                python_dependencies: vec![],
                tools: vec![],
                shared_dependencies: vec![],
            },
            bytes,
        )
    }

    fn integration_fixture(service: &Service, extra: Value) {
        let mut descriptor = json!({"id":"example","name":"Example","minimumVersion":"1",
            "downloadUrl":"https://example.com/download","capabilities":[],"steps":[],"skillId":"test"});
        descriptor
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        crate::integrations::cache(
            &service.data,
            &serde_json::to_vec(&json!({"schema":4,"integrations":[descriptor]})).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn integration_installation_and_prerequisites_follow_the_selected_workspace() {
        let root = env::temp_dir().join(format!(
            "cyapi-integration-workspace-{}",
            uuid::Uuid::new_v4()
        ));
        let (skill, bytes) = fixture();
        let (prerequisite, prerequisite_bytes) = fixture_for("required-skill");
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        service.catalog = Some(Catalog {
            skill_groups: vec![],
            schema: 2,
            generated_at: "test".into(),
            skills: vec![skill.clone(), prerequisite.clone()],
            runtimes: vec![],
        });
        fs::write(service.package_path(&skill), bytes).unwrap();
        fs::write(service.package_path(&prerequisite), prerequisite_bytes).unwrap();
        integration_fixture(
            &service,
            json!({"requires":[{"kind":"skill","id":"required-skill"}]}),
        );
        let installed = service.install("test", "opencode", "global", "").unwrap();
        service
            .install("required-skill", "opencode", "global", "")
            .unwrap();
        // Install cache deletion must not invalidate a working installation.
        fs::remove_file(service.package_path(&skill)).unwrap();
        let view = service.integrations_view("opencode").unwrap();
        assert_eq!(view["entries"][0]["registered"], true);
        assert_eq!(view["entries"][0]["requirementsMet"], true);
        assert_eq!(view["entries"][0]["health"]["ready"], true);
        assert_eq!(view["entries"][0]["downloaded"], false);
        assert_eq!(view["entries"][0]["readiness"]["state"], "configured");
        for workspace in ["codex", "claude"] {
            let view = service.integrations_view(workspace).unwrap();
            assert_eq!(view["entries"][0]["registered"], false);
            assert_eq!(view["entries"][0]["requirementsMet"], false);
            let error = service.dispatch(serde_json::from_value(json!({"action":"integration-connect","integrationId":"example","workspace":workspace})).unwrap()).unwrap_err();
            assert!(error.contains("前置条件"), "{error}");
        }
        integration_fixture(&service, json!({"workspaces":["codex"]}));
        let error = service.dispatch(serde_json::from_value(json!({"action":"integration-connect","integrationId":"example","workspace":"opencode"})).unwrap()).unwrap_err();
        assert!(error.contains("不支持"));
        // Unsupported workspaces can still remove their own old installation.
        service.dispatch(serde_json::from_value(json!({"action":"integration-disconnect","integrationId":"example","workspace":"opencode"})).unwrap()).unwrap();
        assert!(!Path::new(&installed.path).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn integration_detects_missing_adjacent_dependencies_without_using_another_workspace() {
        let root = env::temp_dir().join(format!(
            "cyapi-integration-dependencies-{}",
            uuid::Uuid::new_v4()
        ));
        let (mut skill, bytes) = fixture();
        let (shared, shared_bytes) = fixture_for("support");
        skill.shared_dependencies = vec![shared.id.clone()];
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        service.catalog = Some(Catalog {
            skill_groups: vec![],
            schema: 2,
            generated_at: "test".into(),
            skills: vec![skill.clone(), shared.clone()],
            runtimes: vec![],
        });
        fs::write(service.package_path(&skill), bytes).unwrap();
        assert!(
            !service.ready(&skill),
            "uncached shared resources must prevent offline readiness"
        );
        fs::write(service.package_path(&shared), shared_bytes).unwrap();
        service.download("test").unwrap(); // Entire dependency chain is cached; no network is required.
        assert!(service.ready(&skill));
        integration_fixture(&service, json!({}));
        service.install("test", "opencode", "global", "").unwrap();
        let shared_record = service
            .records
            .iter()
            .find(|r| r.skill_id == "support")
            .unwrap()
            .clone();
        assert_eq!(
            service.integrations_view("opencode").unwrap()["entries"][0]["usable"],
            true
        );
        service.install("support", "codex", "global", "").unwrap();
        fs::remove_file(Path::new(&shared_record.path).join("SKILL.md")).unwrap();
        let view = service.integrations_view("opencode").unwrap();
        assert_eq!(view["entries"][0]["registered"], true);
        assert_eq!(view["entries"][0]["usable"], false);
        assert_eq!(view["entries"][0]["readiness"]["state"], "needs-attention");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cyclic_shared_dependencies_are_rejected_before_download() {
        let root = env::temp_dir().join(format!("cyapi-dependency-cycle-{}", uuid::Uuid::new_v4()));
        let (mut first, _) = fixture_for("first");
        let (mut second, _) = fixture_for("second");
        first.shared_dependencies = vec!["second".into()];
        second.shared_dependencies = vec!["first".into()];
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        service.catalog = Some(Catalog {
            skill_groups: vec![],
            schema: 2,
            generated_at: "test".into(),
            skills: vec![first, second],
            runtimes: vec![],
        });
        assert!(service.download("first").unwrap_err().contains("循环引用"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn opencode_inherits_skills_without_changing_ownership_or_creating_duplicates() {
        let root = env::temp_dir().join(format!("cyapi-inherited-skills-{}", now()));
        let (skill, bytes) = fixture();
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        service.catalog = Some(Catalog {
            skill_groups: vec![],
            schema: 1,
            generated_at: "test".into(),
            skills: vec![skill.clone()],
            runtimes: vec![],
        });
        fs::write(service.package_path(&skill), &bytes).unwrap();
        let owned = service.install("test", "claude", "global", "").unwrap();
        let status = service.status();
        let entries = status["installed"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["record"]["target"], "claude");
        assert_eq!(entries[0]["managed"], true);
        assert!(entries[0]["visibleTargets"]
            .as_array()
            .unwrap()
            .contains(&json!("opencode")));
        let preview = service.preview("test", "opencode", "global", "").unwrap();
        assert_eq!(preview["installed"], true);
        assert_eq!(preview["path"], owned.path);
        assert!(service
            .install("test", "opencode", "global", "")
            .unwrap_err()
            .contains("已安装"));
        assert!(!root.join("home/.config/opencode/skills/test").exists());
        assert_eq!(service.records.len(), 1);
        service.remove(&owned.id).unwrap();

        let shared = root.join("home/.agents/skills/test");
        fs::create_dir_all(&shared).unwrap();
        fs::write(shared.join("SKILL.md"), &skill.markdown).unwrap();
        let status = service.status();
        let entry = &status["installed"][0];
        assert_eq!(entry["record"]["target"], "codex");
        assert_eq!(entry["managed"], false);
        assert!(entry["visibleTargets"]
            .as_array()
            .unwrap()
            .contains(&json!("opencode")));
        assert_eq!(
            service.preview("test", "opencode", "global", "").unwrap()["installed"],
            true
        );
        assert!(service
            .remove(entry["record"]["id"].as_str().unwrap())
            .is_err());
        assert!(shared.join("SKILL.md").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opencode_checks_project_ancestors_and_respects_repository_boundaries() {
        let root = env::temp_dir().join(format!("cyapi-inherited-project-{}", now()));
        let (skill, bytes) = fixture();
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        service.catalog = Some(Catalog {
            skill_groups: vec![],
            schema: 1,
            generated_at: "test".into(),
            skills: vec![skill.clone()],
            runtimes: vec![],
        });
        let project = root.join("project");
        let nested = project.join("src/nested");
        let source = project.join(".claude/skills/test");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(project.join(".git")).unwrap();
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("SKILL.md"), &skill.markdown).unwrap();
        let preview = service
            .preview("test", "opencode", "project", nested.to_str().unwrap())
            .unwrap();
        assert_eq!(preview["installed"], true);
        assert_eq!(
            crate::installed::path_key(Path::new(preview["path"].as_str().unwrap())),
            crate::installed::path_key(&source)
        );
        let other = root.join("other-project");
        fs::create_dir_all(&other).unwrap();
        assert_eq!(
            service
                .preview("test", "opencode", "project", other.to_str().unwrap())
                .unwrap()["installed"],
            false
        );
        fs::remove_file(source.join("SKILL.md")).unwrap();
        fs::write(service.package_path(&skill), bytes).unwrap();
        let outside = service
            .install("test", "codex", "project", root.to_str().unwrap())
            .unwrap();
        assert!(Path::new(&outside.path).join("SKILL.md").is_file());
        assert_eq!(
            service
                .preview("test", "opencode", "project", nested.to_str().unwrap())
                .unwrap()["installed"],
            false
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inherited_support_packages_are_also_installed_beside_dependents() {
        let root = env::temp_dir().join(format!("cyapi-inherited-dependency-{}", now()));
        let (shared, shared_bytes) = fixture_for("nature-shared");
        let (mut dependent, dependent_bytes) = fixture_for("nature-reader");
        dependent.shared_dependencies = vec![shared.id.clone()];
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        service.catalog = Some(Catalog {
            skill_groups: vec![],
            schema: 1,
            generated_at: "test".into(),
            skills: vec![shared.clone(), dependent.clone()],
            runtimes: vec![],
        });
        fs::write(service.package_path(&shared), shared_bytes).unwrap();
        fs::write(service.package_path(&dependent), dependent_bytes).unwrap();
        let original = service
            .install("nature-shared", "claude", "global", "")
            .unwrap();
        let installed = service
            .install("nature-reader", "opencode", "global", "")
            .unwrap();
        assert!(Path::new(&installed.path)
            .parent()
            .unwrap()
            .join("nature-shared/SKILL.md")
            .is_file());
        assert!(Path::new(&original.path).join("SKILL.md").is_file());
        assert_eq!(service.records.len(), 3);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovers_same_name_skills_without_toolbox_records_and_keeps_targets_separate() {
        let root = env::temp_dir().join(format!("cyapi-local-skills-{}", now()));
        let (skill, bytes) = fixture();
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        service.catalog = Some(Catalog {
            skill_groups: vec![],
            schema: 1,
            generated_at: "test".into(),
            skills: vec![skill],
            runtimes: vec![],
        });
        let existing = root.join("home/.codex/skills/another-folder");
        fs::create_dir_all(&existing).unwrap();
        let original =
            "\u{feff}---\r\nname: 'TEST'\r\ndescription: already installed\r\n---\r\nKeep my files";
        fs::write(existing.join("SKILL.md"), original).unwrap();
        let status = service.status();
        assert_eq!(status["skills"][0]["installed"], true);
        assert_eq!(status["installed"].as_array().unwrap().len(), 1);
        assert_eq!(status["installed"][0]["managed"], false);
        assert!(service.records.is_empty());
        assert!(!service.data.join("installed.json").exists());
        let preview = service.preview("test", "codex", "global", "").unwrap();
        assert_eq!(preview["installed"], true);
        assert_eq!(
            crate::installed::path_key(Path::new(preview["path"].as_str().unwrap())),
            crate::installed::path_key(&existing)
        );
        let local: Record =
            serde_json::from_value(status["installed"][0]["record"].clone()).unwrap();
        let mut alias = service.skill("test").unwrap();
        alias.id = "catalog-alias".into();
        assert!(crate::installed::matches(&local, &alias));
        alias.markdown = "---\nname: unrelated\n---\n".into();
        assert!(!crate::installed::matches(&local, &alias));
        assert!(service
            .install("test", "codex", "global", "")
            .unwrap_err()
            .contains("已安装"));
        assert!(service
            .remove(status["installed"][0]["record"]["id"].as_str().unwrap())
            .is_err());
        assert_eq!(
            fs::read_to_string(existing.join("SKILL.md")).unwrap(),
            original
        );
        assert_eq!(
            service.preview("test", "claude", "global", "").unwrap()["installed"],
            false
        );
        assert_eq!(
            service.preview("test", "opencode", "global", "").unwrap()["path"],
            root.join("home")
                .join(".config")
                .join("opencode")
                .join("skills")
                .join("test")
                .to_string_lossy()
                .to_string()
        );

        // A directory without SKILL.md is a conflict, not an installed Skill.
        fs::create_dir_all(root.join("home/.claude/skills/test")).unwrap();
        let preview = service.preview("test", "claude", "global", "").unwrap();
        assert_eq!(preview["installed"], false);
        assert_eq!(preview["conflict"], true);

        // A chosen project is checked independently, even without an installation record.
        let project = root.join("project");
        let project_skill = project.join(".agents/skills/test");
        fs::create_dir_all(&project_skill).unwrap();
        fs::write(project_skill.join("SKILL.md"), "# existing files").unwrap();
        assert_eq!(
            service
                .preview("test", "codex", "project", project.to_str().unwrap())
                .unwrap()["installed"],
            true
        );
        let other_project = root.join("other-project");
        fs::create_dir_all(&other_project).unwrap();
        let inherited = service
            .preview("test", "opencode", "project", project.to_str().unwrap())
            .unwrap();
        assert_eq!(inherited["installed"], true);
        assert_eq!(
            crate::installed::path_key(Path::new(inherited["path"].as_str().unwrap())),
            crate::installed::path_key(&project_skill)
        );
        assert_eq!(
            service
                .preview("test", "codex", "project", other_project.to_str().unwrap())
                .unwrap()["installed"],
            false
        );

        // Bundled system Skills are found; nested unrelated files are not searched.
        fs::remove_file(existing.join("SKILL.md")).unwrap();
        let system_skill = root.join("home/.codex/skills/.system/test");
        fs::create_dir_all(&system_skill).unwrap();
        fs::write(system_skill.join("SKILL.md"), "# system skill").unwrap();
        assert_eq!(service.status()["skills"][0]["installed"], true);
        fs::remove_file(system_skill.join("SKILL.md")).unwrap();
        assert_eq!(service.status()["skills"][0]["installed"], false);

        // Toolbox installs stay deduplicated, and a lost marker never enables uninstall.
        let skill = service.skill("test").unwrap();
        fs::write(service.package_path(&skill), bytes).unwrap();
        let managed = service.install("test", "codex", "global", "").unwrap();
        let status = service.status();
        assert_eq!(status["installed"].as_array().unwrap().len(), 1);
        assert_eq!(status["installed"][0]["managed"], true);
        fs::remove_file(Path::new(&managed.path).join(".cyapi-install.json")).unwrap();
        let status = service.status();
        assert_eq!(status["skills"][0]["installed"], true);
        assert_eq!(status["installed"][0]["managed"], false);
        assert!(service.remove(&managed.id).is_err());
        fs::remove_file(Path::new(&managed.path).join("SKILL.md")).unwrap();
        assert_eq!(service.status()["skills"][0]["installed"], false);
        // This recursive cleanup targets the literal temporary fixture root created above.
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn old_catalog_defaults_and_source_only_guards() {
        let (mut skill, _) = fixture();
        let mut legacy = serde_json::to_value(&skill).unwrap();
        for field in [
            "category",
            "setupNotes",
            "license",
            "distribution",
            "distributionReason",
        ] {
            legacy.as_object_mut().unwrap().remove(field);
        }
        let restored: Skill = serde_json::from_value(legacy).unwrap();
        assert_eq!(restored.category, "其他");
        assert_eq!(restored.distribution, "bundled");
        skill.distribution = "source-only".into();
        skill.distribution_reason = Some("official source only".into());
        skill.sha256.clear();
        skill.package_path.clear();
        skill.size = 0;
        skill.file_count = 0;
        let catalog = Catalog {
            skill_groups: vec![],
            schema: 2,
            generated_at: "test".into(),
            skills: vec![skill],
            runtimes: vec![],
        };
        assert!(validate_catalog(&catalog).is_ok());
        let root = env::temp_dir().join(format!("cyapi-source-only-{}", now()));
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        service.catalog = Some(catalog);
        assert!(service.preview("test", "codex", "global", "").is_err());
        assert!(service.download("test").is_err());
        assert!(service.install("test", "codex", "global", "").is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn published_catalog_packages_and_isolated_installations() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../server");
        let server = root.join("data");
        // The private server project is not published. Skip when its data files are absent.
        if !server.join("catalog.json").is_file() {
            println!("Skipped: no locally prepared private server data");
            return;
        }
        let catalog: Catalog = read_json(&server.join("catalog.json")).unwrap();
        validate_catalog(&catalog).unwrap();
        // The published catalog holds every registered Skill plus My Image.
        let registry: serde_json::Value = read_json(&root.join("skills/registry.json")).unwrap();
        let expected = registry["skills"].as_array().unwrap().len() + 1;
        assert_eq!(catalog.skills.len(), expected);
        for skill in catalog
            .skills
            .iter()
            .filter(|skill| skill.distribution == "bundled")
        {
            validate_package(
                &fs::read(server.join(skill.package_path.trim_start_matches('/'))).unwrap(),
                skill,
            )
            .unwrap();
        }
        let root = env::temp_dir().join(format!("cyapi-published-{}", now()));
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        service.catalog = Some(catalog);
        for id in ["writing-plans", "obsidian-markdown", "copywriting"] {
            let skill = service.skill(id).unwrap();
            fs::copy(
                server.join(skill.package_path.trim_start_matches('/')),
                service.package_path(&skill),
            )
            .unwrap();
            let record = service.install(id, "codex", "global", "").unwrap();
            assert!(Path::new(&record.path).join("SKILL.md").is_file());
            assert!(Path::new(&record.path)
                .join("upstream-notices/SOURCE.json")
                .is_file());
            service.remove(&record.id).unwrap();
        }
        // A published Skill that reads a shared package installs that package beside it.
        let shared = service.skill("nature-shared").unwrap();
        fs::copy(
            server.join(shared.package_path.trim_start_matches('/')),
            service.package_path(&shared),
        )
        .unwrap();
        let reader = service.skill("nature-reader").unwrap();
        fs::copy(
            server.join(reader.package_path.trim_start_matches('/')),
            service.package_path(&reader),
        )
        .unwrap();
        let record = service
            .install("nature-reader", "codex", "global", "")
            .unwrap();
        assert!(Path::new(&record.path)
            .parent()
            .unwrap()
            .join("nature-shared")
            .join("SKILL.md")
            .is_file());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[ignore = "requires a discovered Python runtime"]
    fn python_skill_reuses_runtime_and_launcher_executes_arguments() {
        let root = env::temp_dir().join(format!("cyapi-python-{}", now()));
        fs::create_dir_all(root.join("project")).unwrap();
        let actual_home = PathBuf::from(env::var_os(crate::platform::home_variable()).unwrap());
        // Installation targets only the temporary project; discovery uses the real runtime locations.
        let mut service = Service::new(root.join("data"), actual_home).unwrap();
        let server = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../server/data");
        // The private server project is not published. Skip when its data files are absent.
        if !server.join("catalog.json").is_file() {
            println!("Skipped: no locally prepared private server data");
            return;
        }
        service.catalog = Some(read_json(&server.join("catalog.json")).unwrap());
        let skill = service.skill("ui-ux-pro-max").unwrap();
        let python = service
            .runtime_for(&skill.requirements[0])
            .expect("compatible Python");
        fs::copy(
            server.join(skill.package_path.trim_start_matches('/')),
            service.package_path(&skill),
        )
        .unwrap();
        let record = service
            .install(
                &skill.id,
                "codex",
                "project",
                root.join("project").to_str().unwrap(),
            )
            .unwrap();
        assert_eq!(record.runtimes[0].path, python.path);
        assert!(!service.data.join("runtimes/python").exists());
        let script = root.join("project/argument with spaces.py");
        fs::write(
            &script,
            "import sys; print(sys.argv[1]); raise SystemExit(7)\n",
        )
        .unwrap();
        let output = launcher_command(
            &Path::new(&record.path)
                .join(".cyapi")
                .join(launcher_name("python")),
        )
        .arg(&script)
        .arg("argument with spaces")
        .output()
        .unwrap();
        assert_eq!(
            output.status.code(),
            Some(7),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("argument with spaces"));
        let markdown = fs::read_to_string(Path::new(&record.path).join("SKILL.md")).unwrap();
        assert!(markdown.starts_with("---\n"));
        assert!(markdown.contains(&launcher_name("python")));
        assert!(!markdown.contains("verify-config.mjs"));
        service.remove(&record.id).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_windows_traversal_and_reserved_paths() {
        for path in [
            "../SKILL.md",
            "C:/x",
            "a\\b",
            "/x",
            "a//b",
            "CON.txt",
            "aux/x",
            "foo./x",
            ".cyapi/x",
            "x:y",
            "NUL",
            "a/../b",
        ] {
            assert!(!safe_path(path), "{path}");
        }
        assert!(safe_path("scripts/generate.mjs"));
    }
    #[test]
    fn detects_tampering_and_case_collisions() {
        let (mut skill, mut bytes) = fixture();
        assert!(validate_package(&bytes, &skill).is_ok());
        bytes[0] = b'X';
        assert!(validate_package(&bytes, &skill).is_err());
        let (_, bytes) = fixture();
        let mut pack: SkillPack = serde_json::from_slice(&bytes).unwrap();
        let mut duplicate = pack.files[0].clone();
        duplicate.path = "skill.md".into();
        pack.files.push(duplicate);
        let bytes = serde_json::to_vec(&pack).unwrap();
        skill.size = bytes.len() as u64;
        skill.sha256 = hash(&bytes);
        skill.file_count = 2;
        assert!(validate_package(&bytes, &skill).is_err());
    }
    #[test]
    fn reports_a_managed_skill_as_outdated_when_the_catalog_moves_on() {
        let root = env::temp_dir().join(format!("cyapi-update-{}", now()));
        let (skill, bytes) = fixture();
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        service.catalog = Some(Catalog {
            skill_groups: vec![],
            schema: 1,
            generated_at: "test".into(),
            skills: vec![skill.clone()],
            runtimes: vec![],
        });
        fs::write(service.package_path(&skill), bytes).unwrap();
        service.install("test", "codex", "global", "").unwrap();
        let status = service.status();
        assert_eq!(status["skills"][0]["updateAvailable"], false);
        // A catalog entry with a different package hash means an update is waiting.
        service.records[0].package_sha256 = "0".repeat(64);
        let status = service.status();
        assert_eq!(status["skills"][0]["updateAvailable"], true);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn offline_install_conflict_backup_and_recovery() {
        let root = env::temp_dir().join(format!("cyapi-test-{}", now()));
        let (skill, bytes) = fixture();
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        service.catalog = Some(Catalog {
            skill_groups: vec![],
            schema: 1,
            generated_at: "test".into(),
            skills: vec![skill.clone()],
            runtimes: vec![],
        });
        fs::write(service.package_path(&skill), bytes).unwrap();
        let record = service.install("test", "codex", "global", "").unwrap();
        assert!(Path::new(&record.path).join("SKILL.md").is_file());
        assert!(service.install("test", "codex", "global", "").is_err());
        // Simulate interruption after the atomic directory rename but before registry commit.
        atomic_json(&service.data.join("installed.json"), &Vec::<Record>::new()).unwrap();
        atomic_json(
            &service.data.join("pending.json"),
            &json!({"kind":"install","record":record}),
        )
        .unwrap();
        let mut recovered = Service::new(service.data.clone(), service.home.clone()).unwrap();
        assert_eq!(recovered.records.len(), 1);
        fs::write(Path::new(&record.path).join("user-notes.txt"), "keep me").unwrap();
        let backup = recovered.remove(&record.id).unwrap();
        assert_eq!(
            fs::read_to_string(Path::new(&backup).join("user-notes.txt")).unwrap(),
            "keep me"
        );
        assert!(!Path::new(&record.path).exists());
        assert!(recovered.records.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_dependencies_are_installed_next_to_the_skill_that_needs_them() {
        let root = env::temp_dir().join(format!("cyapi-shared-dep-{}", now()));
        let (shared, shared_bytes) = fixture_for("nature-shared");
        let (mut dependent, dependent_bytes) = fixture_for("nature-reader");
        dependent.shared_dependencies = vec!["nature-shared".into()];
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        service.catalog = Some(Catalog {
            skill_groups: vec![],
            schema: 1,
            generated_at: "test".into(),
            skills: vec![shared.clone(), dependent.clone()],
            runtimes: vec![],
        });
        fs::write(service.package_path(&shared), shared_bytes).unwrap();
        fs::write(service.package_path(&dependent), dependent_bytes).unwrap();
        let record = service
            .install("nature-reader", "codex", "global", "")
            .unwrap();
        // The shared package lands next to the Skill so `../nature-shared/...` resolves.
        let shared_path = Path::new(&record.path)
            .parent()
            .unwrap()
            .join("nature-shared");
        assert!(shared_path.join("SKILL.md").is_file());
        assert!(Path::new(&record.path).join("SKILL.md").is_file());
        assert_eq!(service.records.len(), 2);
        // Installing again refuses to replace either directory.
        assert!(service
            .install("nature-reader", "codex", "global", "")
            .is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configuration_flow_survives_restart_and_catalog_changes_offline() {
        let root = env::temp_dir().join(format!("cyapi-offline-config-{}", now()));
        let (mut skill, bytes) = fixture();
        skill.configuration = Some(
            serde_json::from_str(include_str!(
                "../../tests/fixtures/my-image-configuration.json"
            ))
            .unwrap(),
        );
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        service.catalog = Some(Catalog {
            skill_groups: vec![],
            schema: 1,
            generated_at: "test".into(),
            skills: vec![skill.clone()],
            runtimes: vec![],
        });
        fs::write(service.package_path(&skill), bytes).unwrap();
        let record = service.install("test", "codex", "global", "").unwrap();
        let read = |service: &mut Service| {
            service
                .dispatch(
                    serde_json::from_value(
                        json!({"action":"configuration-read","recordId":record.id}),
                    )
                    .unwrap(),
                )
                .unwrap()
        };
        let view = read(&mut service);
        // A server change for the same installed source revision becomes visible immediately.
        service.catalog.as_mut().unwrap().skills[0]
            .configuration
            .as_mut()
            .unwrap()
            .revision = "new-flow".into();
        let stale = service.dispatch(serde_json::from_value(json!({"action":"configuration-save","recordId":record.id,"values":{},"flowRevision":view["flowRevision"]})).unwrap());
        assert!(stale.unwrap_err().contains("更新"));
        // Installed records retain their compatible flow even if no catalog/server is available.
        let mut offline = Service::new(root.join("data"), root.join("home")).unwrap();
        assert!(offline.catalog.is_none());
        let view = read(&mut offline);
        offline.dispatch(serde_json::from_value(json!({"action":"configuration-save","recordId":record.id,"flowRevision":view["flowRevision"],"values":{"baseUrl":"https://example.invalid/v1","apiKey":"offline-dummy-key"}})).unwrap()).unwrap();
        assert!(!read(&mut offline).to_string().contains("offline-dummy-key"));
        assert!(!offline.status().to_string().contains("offline-dummy-key"));
        assert!(root
            .join("home")
            .join(crate::platform::app_data_relative())
            .join("my-image/.env")
            .is_file());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[ignore = "requires the local catalog server and its prepared Node artifact"]
    fn real_catalog_download_offline_install_and_shared_node() {
        let root = env::temp_dir().join(format!("cyapi-e2e-{}", now()));
        fs::create_dir_all(root.join("project")).unwrap();
        let home = PathBuf::from(env::var_os(crate::platform::home_variable()).unwrap());
        let mut service = Service::new(root.join("data"), home.clone()).unwrap();
        service.settings.server_url = "http://127.0.0.1:47821".into();
        service.refresh().unwrap();
        let requirement = Requirement {
            kind: "node".into(),
            minimum: "20.0.0".into(),
        };
        let existing = service.runtime_for(&requirement).unwrap();
        let unique_paths: HashSet<_> = service
            .runtimes
            .iter()
            .map(|r| r.path.to_lowercase())
            .collect();
        assert_eq!(
            unique_paths.len(),
            service.runtimes.len(),
            "runtime paths must be deduplicated"
        );
        service.download("my-image").unwrap();
        let skill = service.skill("my-image").unwrap();
        assert!(service.ready(&skill));
        // An unreachable server proves that installation uses only local files.
        service.settings.server_url = "http://127.0.0.1:1".into();
        assert!(service.refresh().is_err());
        let record = service
            .install(
                "my-image",
                "codex",
                "project",
                root.join("project").to_str().unwrap(),
            )
            .unwrap();
        assert_eq!(record.runtimes[0].path, existing.path);
        assert!(
            !service.data.join("runtimes/node").exists(),
            "existing runtime must not be installed again"
        );
        let markdown = fs::read_to_string(Path::new(&record.path).join("SKILL.md")).unwrap();
        assert!(markdown.starts_with("---\nname: my-image"));
        assert!(markdown.contains(&launcher_name("node")));
        let output = launcher_command(
            &Path::new(&record.path)
                .join(".cyapi")
                .join(launcher_name("node")),
        )
        .arg("--version")
        .output()
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(&existing.version));
        // Simulate a clean computer with no discovered compatible runtime.
        service.runtimes.clear();
        let first = service.prepare_runtime(&requirement).unwrap();
        assert_eq!(first.source, "managed");
        let before = fs::metadata(&first.path).unwrap().modified().unwrap();
        let second = service.prepare_runtime(&requirement).unwrap();
        assert_eq!(first.path, second.path);
        assert_eq!(
            before,
            fs::metadata(&second.path).unwrap().modified().unwrap()
        );
        assert_eq!(
            fs::read_dir(service.data.join("runtimes/node"))
                .unwrap()
                .count(),
            1
        );
        assert!(!service.data.join("runtimes/python").exists());
        service.remove(&record.id).unwrap();
        assert!(
            Path::new(&first.path).is_file(),
            "uninstall must retain shared runtime"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn claude_configuration_dispatches_read_save_and_restore() {
        let root = env::temp_dir().join(format!("cyapi-claude-dispatch-{}", now()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home/.claude")).unwrap();
        let settings = br#"{"theme":"dark"}"#;
        fs::write(root.join("home/.claude/settings.json"), settings).unwrap();
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();

        let first = service
            .dispatch(serde_json::from_value(json!({"action":"claude-read"})).unwrap())
            .unwrap();
        assert_eq!(first["authTokenConfigured"], false);
        assert_eq!(first["canRestore"], false);

        let saved = service
            .dispatch(
                serde_json::from_value(json!({
                    "action": "claude-save",
                    "claude": {
                        "baseUrl": "https://example.invalid/v1",
                        "authToken": "dummy-token",
                        "model": "claude-test",
                        "revision": first["revision"]
                    }
                }))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(saved["baseUrl"], "https://example.invalid");
        assert_eq!(saved["authTokenConfigured"], true);
        assert_eq!(saved["canRestore"], true);
        let doc: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("home/.claude/settings.json")).unwrap())
                .unwrap();
        assert_eq!(doc["theme"], "dark");
        assert_eq!(doc["env"]["ANTHROPIC_AUTH_TOKEN"], "dummy-token");

        let restored = service
            .dispatch(
                serde_json::from_value(json!({
                    "action": "claude-restore",
                    "configurationRevision": saved["revision"]
                }))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(restored["authTokenConfigured"], false);
        assert_eq!(
            fs::read(root.join("home/.claude/settings.json")).unwrap(),
            settings
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opencode_configuration_dispatches_read_save_and_restore() {
        let root = env::temp_dir().join(format!("cyapi-opencode-dispatch-{}", now()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home/.config/opencode")).unwrap();
        let settings = br#"{"theme":"dark"}"#;
        fs::write(root.join("home/.config/opencode/opencode.json"), settings).unwrap();
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();

        let first = service
            .dispatch(serde_json::from_value(json!({"action":"opencode-read"})).unwrap())
            .unwrap();
        assert_eq!(first["apiKeyConfigured"], false);
        assert_eq!(first["canRestore"], false);

        let saved = service
            .dispatch(
                serde_json::from_value(json!({
                    "action": "opencode-save",
                    "opencode": {
                        "baseUrl": "https://example.invalid/v1/",
                        "apiKey": "dummy-opencode-key",
                        "model": "example/model",
                        "revision": first["revision"]
                    }
                }))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(saved["baseUrl"], "https://example.invalid/v1");
        assert_eq!(saved["model"], "example/model");
        assert_eq!(saved["apiKeyConfigured"], true);
        assert_eq!(saved["canRestore"], true);
        let doc: Value = serde_json::from_slice(
            &fs::read(root.join("home/.config/opencode/opencode.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(doc["theme"], "dark");
        assert_eq!(
            doc["provider"]["cyapi"]["options"]["apiKey"],
            "dummy-opencode-key"
        );
        assert_eq!(
            doc["provider"]["cyapi"]["models"]["default"]["id"],
            "example/model"
        );
        assert_eq!(doc["model"], "cyapi/default");

        let restored = service
            .dispatch(
                serde_json::from_value(json!({
                    "action": "opencode-restore",
                    "configurationRevision": saved["revision"]
                }))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(restored["apiKeyConfigured"], false);
        assert_eq!(
            fs::read(root.join("home/.config/opencode/opencode.json")).unwrap(),
            settings
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mcp_local_dispatch_works_before_the_marketplace_is_downloaded() {
        let root = env::temp_dir().join(format!("cyapi-mcp-offline-dispatch-{}", now()));
        fs::create_dir_all(root.join("home/.codex")).unwrap();
        fs::write(
            root.join("home/.codex/config.toml"),
            "[mcp_servers.existing]\ncommand = \"existing\"\n",
        )
        .unwrap();
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
        let local = service
            .dispatch(
                serde_json::from_value(json!({
                    "action": "mcp-view", "workspace": "codex"
                }))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(local["source"], "local");
        assert_eq!(local["local"][0]["name"], "existing");
        let saved = service
            .dispatch(
                serde_json::from_value(json!({
                    "action": "mcp-local-save", "workspace": "claude",
                    "values": {"id": "offline-local", "command": "local-tool", "claude": true}
                }))
                .unwrap(),
            )
            .unwrap();
        assert!(saved["local"].as_array().unwrap().is_empty());
        // The action carries the revision of the entry it writes to.
        let revision = saved["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == "offline-local")
            .unwrap()["revision"]
            .clone();
        let installed = service
            .dispatch(
                serde_json::from_value(json!({
                    "action": "mcp-install", "workspace": "claude", "serverId": "offline-local",
                    "configurationRevision": revision
                }))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(installed["local"][0]["name"], "offline-local");
        assert_eq!(installed["local"][0]["managed"], true);
        let codex = service
            .dispatch(
                serde_json::from_value(json!({
                    "action": "mcp-view", "workspace": "codex"
                }))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(codex["local"].as_array().unwrap().len(), 1);
        assert_eq!(codex["local"][0]["name"], "existing");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mcp_dispatches_workspace_scoped_view_and_install() {
        let root = env::temp_dir().join(format!("cyapi-mcp-dispatch-{}", now()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home/.codex")).unwrap();
        fs::create_dir_all(root.join("home/.config/opencode")).unwrap();
        fs::create_dir_all(root.join("data/mcp")).unwrap();
        fs::write(
            root.join("home/.codex/config.toml"),
            "[mcp_servers]\n\n[mcp_servers.codegraph]\ncommand = \"codegraph\"\nargs = [\"serve\", \"--mcp\"]\nenabled = true\n",
        )
        .unwrap();
        crate::mcp::cache(
            &root.join("data"),
            br#"{"schema":1,"generatedAt":"2026-10-20T00:00:00Z","servers":[{"id":"codegraph","name":"CodeGraph","version":"1.6+","description":"code graph","homepage":"https://github.com/colbymchenry/codegraph","capabilities":["index"],"steps":["install"],"installCommand":"npm install -g @colbymchenry/codegraph","transport":{"type":"stdio","command":"codegraph","args":["serve","--mcp"],"startupTimeoutSeconds":30,"toolTimeoutSeconds":120},"workspaces":["codex","claude","opencode"]}]}"#,
        )
        .unwrap();
        let mut service = Service::new(root.join("data"), root.join("home")).unwrap();

        let codex = service
            .dispatch(
                serde_json::from_value(json!({"action":"mcp-view","workspace":"codex"})).unwrap(),
            )
            .unwrap();
        assert!(codex["configPath"].as_str().unwrap().ends_with(
            &Path::new(".codex")
                .join("config.toml")
                .to_string_lossy()
                .to_string()
        ));
        assert_eq!(codex["entries"][0]["adoptable"], true);
        let revision = codex["entries"][0]["revision"].clone();
        let installed = service
            .dispatch(
                serde_json::from_value(json!({
                    "action": "mcp-install",
                    "workspace": "codex",
                    "serverId": "codegraph",
                    "configurationRevision": revision
                }))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(installed["entries"][0]["managed"], true);
        assert!(service
            .dispatch(
                serde_json::from_value(json!({
                    "action": "mcp-install",
                    "workspace": "codex",
                    "serverId": "codegraph",
                    "configurationRevision": revision
                }))
                .unwrap(),
            )
            .is_err());

        let opencode = service
            .dispatch(
                serde_json::from_value(json!({"action":"mcp-view","workspace":"opencode"}))
                    .unwrap(),
            )
            .unwrap();
        assert!(opencode["configPath"].as_str().unwrap().ends_with(
            &Path::new(".config")
                .join("opencode")
                .join("opencode.json")
                .to_string_lossy()
                .to_string()
        ));
        assert_eq!(opencode["local"].as_array().unwrap().len(), 0);
        fs::remove_dir_all(root).unwrap();
    }
}
