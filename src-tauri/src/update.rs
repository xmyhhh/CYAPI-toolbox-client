//! Desktop update reminder and in-place upgrade.
//!
//! The catalog server publishes `data/client-release.json` at
//! `/api/client-release`. The client treats that file as untrusted input: every
//! field is revalidated here before it can reach the download or launch steps.
use crate::runtime::version_tuple;
use crate::service::{fetch_server_with_progress, hash, FetchOptions, Service};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, String>;

const MANIFEST_PATH: &str = "/api/client-release";
const MANIFEST_LIMIT: u64 = 64 * 1024;
const MANIFEST_TIMEOUT: u64 = 8;
// Each route is bounded; never spend ten minutes silently trying both routes.
const PRIMARY_DOWNLOAD_TIMEOUT: u64 = 20;
const DOWNLOAD_TIMEOUT: u64 = 120;
static UPDATE_TASK: Mutex<()> = Mutex::new(());
static MANIFEST_CACHE: Mutex<()> = Mutex::new(());

/// Only immutable settings cross into an update worker. No Service lock is held
/// while requesting a manifest, downloading or hashing an installer.
pub struct UpdateContext {
    data: PathBuf,
    server_url: String,
}

impl UpdateContext {
    pub fn snapshot(service: &Service) -> Self {
        Self {
            data: service.data.clone(),
            server_url: service.server_url().to_string(),
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub operation_id: String,
    pub stage: &'static str,
    pub downloaded: u64,
    pub total: u64,
}
const MAX_INSTALLER: u64 = 400_000_000;
/// Update downloads are only ever opened on the project's own download host.
const ALLOWED_HOSTS: [&str; 1] = ["tool.cyapi.cyou"];
/// Links opened from the UI may also point at the project's official site.
const OPEN_HOSTS: [&str; 2] = ["tool.cyapi.cyou", "www.cyapi.cyou"];
/// Give the installer time to take over before this process exits.
const RESTART_DELAY: Duration = Duration::from_millis(2500);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseFile {
    pub kind: String,
    pub filename: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseManifest {
    pub schema: u32,
    #[serde(default)]
    pub channel: String,
    pub version: String,
    #[serde(default)]
    pub min_version: String,
    #[serde(default)]
    pub released_at: String,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub download_page: String,
    #[serde(default)]
    pub downloads: Vec<ReleaseFile>,
}

fn is_newer(candidate: &str, base: &str) -> bool {
    matches!(
        (version_tuple(candidate), version_tuple(base)),
        (Some(candidate), Some(base)) if candidate > base
    )
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The file name extensions a release file may use on this platform.
fn installer_extensions() -> &'static [&'static str] {
    if cfg!(windows) {
        &["exe"]
    } else if cfg!(target_os = "macos") {
        &["dmg", "zip", "pkg"]
    } else {
        &["AppImage", "deb", "rpm", "tar", "gz"]
    }
}

fn valid_filename(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && installer_extensions()
            .iter()
            .any(|extension| value.ends_with(&format!(".{extension}")))
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._".contains(&byte))
}

fn trusted_url(value: &str, hosts: &[&str]) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some_and(|host| hosts.contains(&host))
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn allowed_url(value: &str) -> bool {
    trusted_url(value, &ALLOWED_HOSTS)
}

/// Pages the user can open in the browser, including the official website.
fn openable_url(value: &str) -> bool {
    trusted_url(value, &OPEN_HOSTS)
}

fn validate(manifest: &ReleaseManifest) -> Result<()> {
    if manifest.schema != 1 {
        return Err("更新清单格式无效".into());
    }
    if version_tuple(&manifest.version).is_none() {
        return Err("更新清单版本号无效".into());
    }
    if !manifest.min_version.is_empty() && version_tuple(&manifest.min_version).is_none() {
        return Err("更新清单最低版本无效".into());
    }
    if !allowed_url(&manifest.download_page) {
        return Err("更新清单下载页地址无效".into());
    }
    if manifest.channel.len() > 32
        || manifest.released_at.len() > 40
        || manifest.notes.len() > 30
        || manifest.notes.iter().any(|note| note.len() > 600)
    {
        return Err("更新清单文本超出限制".into());
    }
    if manifest.downloads.len() > 8 {
        return Err("更新清单下载项过多".into());
    }
    for file in &manifest.downloads {
        if !["installer", "portable"].contains(&file.kind.as_str())
            || !valid_filename(&file.filename)
            || !allowed_url(&file.url)
            || !valid_digest(&file.sha256)
            || file.size == 0
            || file.size > MAX_INSTALLER
        {
            return Err("更新清单下载项无效".into());
        }
    }
    if manifest.downloads.is_empty() {
        return Err("更新清单缺少下载项".into());
    }
    Ok(())
}

fn cache_path(data: &Path) -> PathBuf {
    data.join("updates").join("client-release.json")
}

/// The manifest is published per platform, so the request names the running
/// system and receives packages that actually run on it.
fn manifest_url(base: &Url) -> Result<Url> {
    let mut url = base
        .join(MANIFEST_PATH.trim_start_matches('/'))
        .map_err(|_| "更新地址无效".to_string())?;
    url.query_pairs_mut()
        .append_pair("platform", crate::platform::current());
    Ok(url)
}

fn manifest_from_bytes(data: &Path, bytes: &[u8]) -> Result<ReleaseManifest> {
    let manifest: ReleaseManifest =
        serde_json::from_slice(bytes).map_err(|_| "更新清单解析失败".to_string())?;
    validate(&manifest)?;
    let _cache = MANIFEST_CACHE.lock().map_err(|_| "更新清单缓存暂不可用")?;
    if let Some(parent) = cache_path(data).parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(cache_path(data), bytes);
    Ok(manifest)
}

fn manifest_from_cache(data: &Path, online: String) -> Result<ReleaseManifest> {
    let _cache = MANIFEST_CACHE.lock().map_err(|_| "更新清单缓存暂不可用")?;
    let cached = fs::read(cache_path(data)).map_err(|_| online)?;
    let manifest: ReleaseManifest =
        serde_json::from_slice(&cached).map_err(|_| "本地更新清单无效".to_string())?;
    validate(&manifest)?;
    Ok(manifest)
}

/// Read the manifest from the server, falling back to the last good copy so an
/// offline start can still surface an update that was already discovered.
fn read_manifest(context: &UpdateContext) -> Result<ReleaseManifest> {
    let url = manifest_url(&crate::service::base_url_from(&context.server_url)?)?;
    crate::logging::event("info", "update", "正在获取更新清单");
    match fetch_server_with_progress(
        url,
        MANIFEST_LIMIT,
        None,
        FetchOptions {
            primary_timeout: MANIFEST_TIMEOUT,
            fallback_timeout: MANIFEST_TIMEOUT,
            cancellable: false,
        },
        &mut |_| {},
    )
    .map(|fetched| fetched.bytes)
    {
        Ok(bytes) => manifest_from_bytes(&context.data, &bytes),
        Err(online) => {
            crate::logging::event(
                "warn",
                "update",
                format!("更新清单请求失败，尝试本地缓存：{online}"),
            );
            manifest_from_cache(&context.data, online)
        }
    }
}

/// Same as [`read_manifest`] but uses bytes fetched before the service lock was
/// taken, so a slow manifest request cannot block local configuration reads.
fn read_manifest_with(
    service: &Service,
    online: Result<Vec<u8>>,
) -> Result<(ReleaseManifest, Option<String>)> {
    match online.and_then(|bytes| manifest_from_bytes(&service.data, &bytes)) {
        Ok(manifest) => Ok((manifest, None)),
        Err(error) => manifest_from_cache(&service.data, error.clone())
            .map(|manifest| (manifest, Some(error))),
    }
}

/// Fetch the release manifest without touching the service state. Callers pass
/// the server URL so the request can run while the toolbox lock stays free.
pub fn fetch_manifest(server_url: &str) -> Result<Vec<u8>> {
    let mut url = manifest_url(&crate::service::base_url_from(server_url)?)?;
    url.query_pairs_mut()
        .append_pair("currentVersion", env!("CARGO_PKG_VERSION"));
    fetch_server_with_progress(
        url,
        MANIFEST_LIMIT,
        None,
        FetchOptions {
            primary_timeout: MANIFEST_TIMEOUT,
            fallback_timeout: MANIFEST_TIMEOUT,
            cancellable: false,
        },
        &mut |_| {},
    )
    .map(|fetched| fetched.bytes)
}

/// The NSIS installer always writes `uninstall.exe` next to the main binary, so
/// its presence distinguishes an installed copy from the portable EXE. A macOS
/// build runs from an application bundle, so the bundle marks the installed
/// copy. Anything uncertain is treated as portable, which only offers the
/// download page.
pub fn install_mode() -> &'static str {
    let Some(exe) = std::env::current_exe().ok() else {
        return "portable";
    };
    let Some(directory) = exe.parent() else {
        return "portable";
    };
    if cfg!(windows) {
        if directory.join("uninstall.exe").is_file() {
            return "installed";
        }
    } else if cfg!(target_os = "macos") {
        // A bundled build keeps the executable inside `CYAPI Toolbox.app`.
        if exe
            .components()
            .any(|part| part.as_os_str() == "CYAPI Toolbox.app")
            || directory.ancestors().any(|path| {
                path.file_name()
                    .is_some_and(|name| name == "CYAPI Toolbox.app")
            })
        {
            return "installed";
        }
    }
    "portable"
}

fn isolated() -> bool {
    std::env::var_os("CYAPI_TOOLBOX_HOME_DIR").is_some()
}

fn installer_path(data: &Path, version: &str) -> Result<PathBuf> {
    if version_tuple(version).is_none() {
        return Err("更新版本号无效".into());
    }
    let name = if cfg!(windows) {
        format!("CYAPI-Toolbox-{version}-setup.exe")
    } else if cfg!(target_os = "macos") {
        format!("CYAPI-Toolbox-{version}-macos.dmg")
    } else {
        format!("CYAPI-Toolbox-{version}-linux.AppImage")
    };
    Ok(data.join("updates").join(name))
}

pub fn check(service: &Service, online: Result<Vec<u8>>) -> Result<Value> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let (manifest, check_error) = read_manifest_with(service, online)?;
    let has_update = is_newer(&manifest.version, &current);
    let mandatory =
        has_update && !manifest.min_version.is_empty() && is_newer(&manifest.min_version, &current);
    let installer = manifest
        .downloads
        .iter()
        .find(|file| file.kind == "installer")
        .cloned();
    let portable = manifest
        .downloads
        .iter()
        .find(|file| file.kind == "portable")
        .cloned();
    let mode = install_mode();
    // Development and isolated runs must never replace the running binary.
    let can_self_update = has_update
        && mode == "installed"
        && installer.is_some()
        && !cfg!(debug_assertions)
        && !isolated();
    Ok(json!({
        "schema": 1,
        "current": current,
        "latest": manifest.version,
        "hasUpdate": has_update,
        "mandatory": mandatory,
        "notes": manifest.notes,
        "releasedAt": manifest.released_at,
        "downloadPage": manifest.download_page,
        "installer": installer,
        "portable": portable,
        "installMode": mode,
        "canSelfUpdate": can_self_update,
        "source": if check_error.is_some() { "cached" } else { "online" },
        "checkError": check_error,
    }))
}

pub fn download(
    context: &UpdateContext,
    mut report: impl FnMut(&'static str, u64, u64),
) -> Result<Value> {
    let _task = UPDATE_TASK
        .try_lock()
        .map_err(|_| "已有更新任务正在进行，请稍后重试。")?;
    report("manifest", 0, 0);
    if isolated() {
        return Err("当前为隔离或调试运行，请在正式安装版中使用自动更新。".into());
    }
    let manifest = read_manifest(context)?;
    let file = manifest
        .downloads
        .iter()
        .find(|file| file.kind == "installer")
        .cloned()
        .ok_or("更新清单缺少安装版下载")?;
    let target = installer_path(&context.data, &manifest.version)?;
    crate::logging::event(
        "info",
        "update",
        format!(
            "准备下载 {}（{} 字节，主线路超时 {PRIMARY_DOWNLOAD_TIMEOUT} 秒，源站超时 {DOWNLOAD_TIMEOUT} 秒）",
            file.filename, file.size
        ),
    );
    if fs::read(&target)
        .is_ok_and(|bytes| bytes.len() as u64 == file.size && hash(&bytes) == file.sha256)
    {
        report("verified", file.size, file.size);
        crate::logging::event("info", "update", "使用已校验的本地更新包");
        return Ok(json!({
            "path": target.to_string_lossy(),
            "version": manifest.version,
            "size": file.size,
            "sha256": file.sha256,
            "cached": true,
        }));
    }
    let url = reqwest::Url::parse(&file.url).map_err(|_| "更新下载地址无效".to_string())?;
    report("download", 0, file.size);
    let mut last_report = Instant::now();
    let mut last_log = Instant::now();
    let bytes = fetch_server_with_progress(
        url,
        file.size,
        None,
        FetchOptions {
            primary_timeout: PRIMARY_DOWNLOAD_TIMEOUT,
            fallback_timeout: DOWNLOAD_TIMEOUT,
            cancellable: false,
        },
        &mut |downloaded| {
            if downloaded == 0
                || downloaded == file.size
                || last_report.elapsed() >= Duration::from_millis(250)
            {
                report("download", downloaded, file.size);
                last_report = Instant::now();
            }
            if downloaded == 0
                || downloaded == file.size
                || last_log.elapsed() >= Duration::from_secs(5)
            {
                crate::logging::event(
                    "info",
                    "update",
                    format!("更新包下载进度：{downloaded}/{} 字节", file.size),
                );
                last_log = Instant::now();
            }
        },
    )
    .map_err(|error| {
        crate::logging::event("error", "update", format!("更新包下载失败：{error}"));
        "下载更新失败或超时，请重试或点击「从官网下载」。".to_string()
    })?
    .bytes;
    report("verify", bytes.len() as u64, file.size);
    crate::logging::event("info", "update", "下载完成，正在校验更新包");
    if bytes.len() as u64 != file.size || hash(&bytes) != file.sha256 {
        return Err("更新包校验失败，已丢弃。请重试或从官网下载。".into());
    }
    let parent = target.parent().ok_or("更新目录无效")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = target.with_extension("part");
    fs::write(&temporary, &bytes).map_err(|error| error.to_string())?;
    fs::rename(&temporary, &target).map_err(|error| error.to_string())?;
    report("verified", file.size, file.size);
    crate::logging::event("info", "update", "更新包校验通过，已保存到本地");
    Ok(json!({
        "path": target.to_string_lossy(),
        "version": manifest.version,
        "size": file.size,
        "sha256": file.sha256,
        "cached": false,
    }))
}

/// Network/verification runs without the service lock. The caller supplies a
/// final commit that waits for active writes and blocks new writes before exit.
pub fn install(context: &UpdateContext, commit: impl FnOnce(&Path) -> Result<()>) -> Result<Value> {
    let _task = UPDATE_TASK
        .try_lock()
        .map_err(|_| "已有更新任务正在进行，请稍后重试。")?;
    if cfg!(debug_assertions) {
        return Err("开发构建不会自动安装更新，请使用官网安装包。".into());
    }
    if isolated() {
        return Err("当前为隔离或调试运行，请在正式安装版中使用自动更新。".into());
    }
    if install_mode() != "installed" {
        return Err("免安装版请从官网下载新版本后手动替换。".into());
    }
    let manifest = read_manifest(context)?;
    let file = manifest
        .downloads
        .iter()
        .find(|file| file.kind == "installer")
        .cloned()
        .ok_or("更新清单缺少安装版下载")?;
    let path = installer_path(&context.data, &manifest.version)?;
    let bytes = fs::read(&path).map_err(|_| "尚未下载更新安装包，请先下载。".to_string())?;
    if bytes.len() as u64 != file.size || hash(&bytes) != file.sha256 {
        return Err("更新安装包校验失败，请重新下载。".into());
    }
    commit(&path)?;
    Ok(json!({"started": true, "version": manifest.version, "path": path.to_string_lossy()}))
}

pub fn launch_and_exit(path: &Path) -> Result<()> {
    launch_installer(path)?;
    // Called only after the service's shutdown barrier is in place.
    std::thread::spawn(|| {
        std::thread::sleep(RESTART_DELAY);
        std::process::exit(0);
    });
    Ok(())
}

pub fn open(value: &str) -> Result<Value> {
    if !openable_url(value) {
        return Err("只允许打开官网地址".into());
    }
    launch_url(value)?;
    Ok(json!({"opened": value}))
}

fn launch_url(target: &str) -> Result<()> {
    crate::platform::open_url(target)
}

#[cfg(windows)]
fn launch_installer(path: &Path) -> Result<()> {
    std::process::Command::new(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法启动安装程序：{error}"))
}

/// macOS ships a disk image. Mounting it with Finder lets the user drag the
/// application into place.
#[cfg(target_os = "macos")]
fn launch_installer(path: &Path) -> Result<()> {
    std::process::Command::new("open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法启动安装程序：{error}"))
}

/// Linux has no installer bundle yet; the download page stays available.
#[cfg(all(unix, not(target_os = "macos")))]
fn launch_installer(_path: &Path) -> Result<()> {
    Err("自动更新仅支持 Windows 与 macOS 桌面客户端".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> ReleaseManifest {
        let extension = installer_extensions()[0];
        let filename = format!("CYAPI-Toolbox_0.9.0_x64.{extension}");
        ReleaseManifest {
            schema: 1,
            channel: "stable".into(),
            version: "0.9.0".into(),
            min_version: "0.8.0".into(),
            released_at: "2026-10-25T02:00:00Z".into(),
            notes: vec!["修复问题".into()],
            download_page: "https://tool.cyapi.cyou/#download".into(),
            downloads: vec![ReleaseFile {
                kind: "installer".into(),
                filename: filename.clone(),
                url: format!("https://tool.cyapi.cyou/downloads/{filename}"),
                size: 1024,
                sha256: "a".repeat(64),
            }],
        }
    }

    #[test]
    fn concurrent_update_is_rejected_instead_of_waiting_on_a_worker_lock() {
        let _task = UPDATE_TASK.lock().unwrap();
        let context = UpdateContext {
            data: PathBuf::from("unused"),
            server_url: "https://api.cyapi.cyou".into(),
        };
        let error = download(&context, |_, _, _| {
            panic!("must not start another download")
        })
        .unwrap_err();
        assert!(error.contains("已有更新任务"));
    }

    #[test]
    fn concurrent_manifest_cache_readers_never_observe_partial_writes() {
        let root =
            std::env::temp_dir().join(format!("cyapi-update-cache-{}", uuid::Uuid::new_v4()));
        let initial = serde_json::to_vec(&manifest()).unwrap();
        manifest_from_bytes(&root, &initial).unwrap();
        std::thread::scope(|scope| {
            for index in 0..4 {
                let root = &root;
                scope.spawn(move || {
                    let mut release = manifest();
                    release.notes = vec!["note".repeat(index * 40 + 1)];
                    let bytes = serde_json::to_vec(&release).unwrap();
                    for _ in 0..20 {
                        manifest_from_bytes(root, &bytes).unwrap();
                        let cached = manifest_from_cache(root, "offline".into()).unwrap();
                        assert_eq!(cached.version, release.version);
                    }
                });
            }
        });
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn progress_payload_matches_the_frontend_event_contract() {
        let progress = serde_json::to_value(UpdateProgress {
            operation_id: "update-test".into(),
            stage: "download",
            downloaded: 50,
            total: 100,
        })
        .unwrap();
        assert_eq!(
            progress,
            json!({
                "operationId": "update-test", "stage": "download", "downloaded": 50, "total": 100,
            })
        );
    }

    #[test]
    fn version_comparison_follows_semver_order() {
        assert!(is_newer("0.9.0", "0.8.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(!is_newer("0.8.0", "0.8.0"));
        assert!(!is_newer("0.7.1", "0.8.0"));
    }

    #[test]
    fn checks_distinguish_live_releases_from_offline_cache() {
        let root =
            std::env::temp_dir().join(format!("cyapi-update-check-{}", uuid::Uuid::new_v4()));
        let service = Service::new(root.join("data"), root.join("home")).unwrap();
        assert!(check(&service, Err("网络不可用".into())).is_err());

        let mut release = manifest();
        release.version = "99.0.0".into();
        release.min_version = "98.0.0".into();
        let status = check(&service, Ok(serde_json::to_vec(&release).unwrap())).unwrap();
        assert_eq!(status["source"], "online");
        assert!(status["checkError"].is_null());
        assert_eq!(status["hasUpdate"], true);
        assert_eq!(status["mandatory"], true);

        let cached = check(&service, Err("网络不可用".into())).unwrap();
        assert_eq!(cached["source"], "cached");
        assert_eq!(cached["checkError"], "网络不可用");
        assert_eq!(cached["latest"], "99.0.0");
        assert_eq!(cached["hasUpdate"], true);
        // A broken response must not replace the last valid cached release.
        assert_eq!(
            check(&service, Ok(b"{".to_vec())).unwrap()["source"],
            "cached"
        );

        release.version = env!("CARGO_PKG_VERSION").into();
        release.min_version = "0.0.0".into();
        let current = check(&service, Ok(serde_json::to_vec(&release).unwrap())).unwrap();
        assert_eq!(current["hasUpdate"], false);
        assert_eq!(current["mandatory"], false);
        assert_eq!(current["source"], "online");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manifest_rejects_untrusted_download_hosts() {
        let mut value = manifest();
        assert!(validate(&value).is_ok());
        value.downloads[0].url = "https://evil.example/downloads/setup.exe".into();
        assert!(validate(&value).is_err());
        let mut value = manifest();
        value.downloads[0].sha256 = "A".repeat(64);
        assert!(validate(&value).is_err());
        let mut value = manifest();
        value.downloads[0].filename = "../setup.exe".into();
        assert!(validate(&value).is_err());
        let mut value = manifest();
        value.downloads.clear();
        assert!(validate(&value).is_err());
    }

    #[test]
    fn manifest_requests_name_the_running_platform() {
        let base = Url::parse("https://api.cyapi.cyou/").unwrap();
        let url = manifest_url(&base).unwrap();
        assert!(url.path().ends_with("/api/client-release"));
        assert_eq!(
            url.query_pairs()
                .find(|(key, _)| key == "platform")
                .map(|(_, value)| value.into_owned()),
            Some(crate::platform::current().to_string())
        );
    }

    #[test]
    fn urls_require_https_on_the_download_host() {
        assert!(allowed_url("https://tool.cyapi.cyou/#download"));
        assert!(!allowed_url("http://tool.cyapi.cyou/#download"));
        assert!(!allowed_url("https://tool.cyapi.cyou.evil.example/"));
        assert!(!allowed_url("https://user:pass@tool.cyapi.cyou/"));
    }

    #[test]
    fn official_site_can_be_opened_but_is_not_a_download_host() {
        assert!(openable_url("https://www.cyapi.cyou"));
        assert!(openable_url("https://www.cyapi.cyou/"));
        assert!(openable_url("https://tool.cyapi.cyou/#download"));
        assert!(!openable_url("http://www.cyapi.cyou"));
        assert!(!openable_url("https://www.cyapi.cyou.evil.example/"));
        assert!(!openable_url("https://user:pass@www.cyapi.cyou/"));
        assert!(!allowed_url("https://www.cyapi.cyou"));
    }
}
