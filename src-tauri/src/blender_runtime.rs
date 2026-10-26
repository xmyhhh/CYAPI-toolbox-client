//! One-click Blender preparation. Only fixed, hash-verified uv binaries and the
//! pinned upstream wheel are used; packages live in an app-owned virtualenv.
//! Never run upstream `setup`, which rewrites every discovered AI client.
use crate::{
    configuration::{make_executable, no_links, stage},
    mcp,
    service::hash,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;
const VERSION: &str = "2.2.4";
const PACKAGE: &str = "https://files.pythonhosted.org/packages/eb/ad/1248a5f7ed872cbbe333c1d9081c0fe8a077b53083f5a24b16084fa4dc07/mcp_for_blender-2.2.4-py3-none-any.whl#sha256=c6c3286da735cd44eef43186759da7f19696b0d50991a1b64399280f9744ae14";
const CHECK: &str = "import importlib.metadata as m; assert m.version('mcp-for-blender') == '2.2.4'; import blender_mcp.server; from blender_mcp.addon_manager import get_bundled_addon_path; assert get_bundled_addon_path().is_file()";

#[derive(Deserialize, Serialize)]
struct Receipt {
    version: String,
    environment: String,
}

fn root(data: &Path) -> PathBuf {
    data.join("mcp/runtime/blender")
}
fn python(environment: &Path) -> PathBuf {
    environment.join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    })
}

/// Resolve the committed path even if the executable has since disappeared:
/// the MCP manager must still recognize its own config and offer a repair.
pub fn command(data: &Path) -> Option<PathBuf> {
    let root = root(data);
    no_links(&root.join("ready.json")).ok()?;
    let receipt: Receipt = serde_json::from_slice(&fs::read(root.join("ready.json")).ok()?).ok()?;
    if receipt.version != VERSION || uuid::Uuid::parse_str(&receipt.environment).is_err() {
        return None;
    }
    Some(python(&root.join(receipt.environment)))
}

pub fn supported(descriptor: &mcp::Descriptor) -> bool {
    descriptor.id == "blender"
        && matches!(&descriptor.transport,
        mcp::Transport::Stdio { command, args, .. } if command == "uvx" && args == &["mcp-for-blender"])
}

pub fn resolve(data: &Path, descriptor: &mut mcp::Descriptor) {
    if supported(descriptor) {
        if let (Some(path), mcp::Transport::Stdio { command, args, .. }) =
            (command(data), &mut descriptor.transport)
        {
            // Do not canonicalize virtualenv symlinks on Unix: running the base
            // interpreter would lose the environment's site-packages.
            *command = path.to_string_lossy().into();
            *args = vec!["-I".into(), "-m".into(), "blender_mcp.server".into()];
        }
    }
}

fn uv_artifact() -> Result<(&'static str, &'static str)> {
    match crate::platform::current() {
        "windows-x64" => Ok(("https://files.pythonhosted.org/packages/32/ea/771744a784b16aa21d195cb5d5d5a2878d72fa00fc26f11ea3f9646994fe/uv-0.13.5-py3-none-win_amd64.whl", "1fb29fc46ba12033e2ef95a95ac79d649e71bbb1358419bbbcbd19d86026ef2f")),
        "macos-arm64" => Ok(("https://files.pythonhosted.org/packages/d2/26/9ac45211123244ea68be852775093af487a64debc13621a2de239f8ebe73/uv-0.13.5-py3-none-macosx_11_0_arm64.whl", "a42c85fa8156e1e5e2cecdbc77bb818ea1dcf5f78b8e2a8f84c7fe4f16f0702e")),
        "macos-x64" => Ok(("https://files.pythonhosted.org/packages/e3/2a/dcae794203261e49564bb9ff7c4b64a093f69ac73476cc8ce54c1deed3a1/uv-0.13.5-py3-none-macosx_10_12_x86_64.whl", "337a14829bb499e114989df4f908f134ff37f1fb6c33497541cfb05b28080e39")),
        "linux-x64" => Ok(("https://files.pythonhosted.org/packages/3a/ad/3c2a81d66d1839063662e5b6c0002ec31f8ee987dbc5295747627ca6160a/uv-0.13.5-py3-none-manylinux_2_17_x86_64.manylinux2014_x86_64.whl", "fa8d26e08e34c29c0c4e820cf3181757086d547240b4f4d31ff1d8bd810ca946")),
        _ => Err("当前系统暂不支持自动准备 Blender MCP 运行环境".into()),
    }
}

fn extract_uv(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("uv 压缩包无效：{e}"))?;
    // Extract one fixed member, not arbitrary archive paths or scripts.
    let member = if cfg!(windows) {
        "uv-0.13.5.data/scripts/uv.exe"
    } else {
        "uv-0.13.5.data/scripts/uv"
    };
    let entry = archive
        .by_name(member)
        .map_err(|_| "uv 安装包缺少可执行文件")?;
    let mut result = Vec::new();
    entry
        .take(80_000_001)
        .read_to_end(&mut result)
        .map_err(|e| e.to_string())?;
    if result.is_empty() || result.len() > 80_000_000 {
        return Err("uv 可执行文件大小异常".into());
    }
    Ok(result)
}

fn prepare_uv(data: &Path) -> Result<PathBuf> {
    let (url, expected) = uv_artifact()?;
    let directory = data.join("mcp/runtime/uv-0.13.5");
    let wheel = directory.join("uv.whl");
    let exe = directory.join(if cfg!(windows) { "uv.exe" } else { "uv" });
    no_links(&exe)?;
    no_links(&wheel)?;
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let cached = fs::read(&wheel).ok().filter(|b| hash(b) == expected);
    let bytes = match cached {
        Some(bytes) => bytes,
        None => {
            crate::logging::event("info", "integration", "下载并校验隔离环境管理器 uv 0.13.5");
            let response = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(180))
                .build()
                .map_err(|e| e.to_string())?
                .get(url)
                .send()
                .and_then(|r| r.error_for_status())
                .map_err(|e| format!("下载 uv 失败：{e}"))?;
            let mut bytes = Vec::new();
            response
                .take(25_000_001)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 25_000_000 || hash(&bytes) != expected {
                return Err("uv 安装包完整性校验失败".into());
            }
            let temp = stage(&wheel, &bytes)?;
            fs::rename(temp, &wheel).map_err(|e| e.to_string())?;
            bytes
        }
    };
    let binary = extract_uv(&bytes)?;
    if !fs::read(&exe).is_ok_and(|current| current == binary) {
        let temp = stage(&exe, &binary)?;
        fs::rename(temp, &exe).map_err(|e| e.to_string())?;
    }
    make_executable(&exe)?;
    Ok(exe)
}

fn run(data: &Path, exe: &Path, args: &[&str], log: &Path, timeout: u64) -> Result<()> {
    no_links(log)?;
    no_links(&data.join("mcp/runtime/python"))?;
    no_links(&data.join("mcp/runtime/cache"))?;
    let output = fs::File::create(log).map_err(|e| e.to_string())?;
    let mut command = Command::new(exe);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("UV_") || key.to_string_lossy().starts_with("PIP_") {
            command.env_remove(key);
        }
    }
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(output.try_clone().map_err(|e| e.to_string())?)
        .stderr(output)
        // Ignore machine-wide uv settings, indexes and Python selections.
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .env("UV_PYTHON_INSTALL_DIR", data.join("mcp/runtime/python"))
        .env("UV_CACHE_DIR", data.join("mcp/runtime/cache"))
        .env("UV_NO_PROGRESS", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("无法启动 MCP 环境准备：{e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => {
                let tail: String = fs::read_to_string(log)
                    .unwrap_or_default()
                    .chars()
                    .rev()
                    .take(1800)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                crate::logging::event("error", "integration", &tail);
                return Err(format!("Blender MCP 环境准备失败：{tail}"));
            }
            Ok(None)
                if start.elapsed() < Duration::from_secs(timeout)
                    && !crate::cancel::is_cancelled() =>
            {
                std::thread::sleep(Duration::from_millis(100))
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(if crate::cancel::is_cancelled() {
                    crate::cancel::MESSAGE.into()
                } else if let Err(e) = result {
                    e.to_string()
                } else {
                    "Blender MCP 环境准备超时，可重试接入".into()
                });
            }
        }
    }
}

pub fn prepare(data: &Path) -> Result<()> {
    let root = root(data);
    no_links(&root)?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let log = root.join(format!("prepare-{}.log", uuid::Uuid::new_v4()));
    if let Some(exe) = command(data).filter(|p| p.is_file()) {
        if run(data, &exe, &["-I", "-c", CHECK], &log, 30).is_ok() {
            return Ok(());
        }
    }
    let uv = prepare_uv(data)?;
    let id = uuid::Uuid::new_v4().to_string();
    // An interrupted attempt is never mistaken for a completed environment;
    // retries use a fresh path, without deleting existing user or cached files.
    let environment = root.join(&id);
    no_links(&environment)?;
    crate::logging::event(
        "info",
        "integration",
        "准备 Blender MCP 的独立 Python 3.12 与依赖（首次接入需要下载）",
    );
    run(
        data,
        &uv,
        &[
            "--no-config",
            "venv",
            "--managed-python",
            "--python",
            "3.12",
            &environment.to_string_lossy(),
        ],
        &log,
        600,
    )?;
    let exe = python(&environment);
    run(
        data,
        &uv,
        &[
            "--no-config",
            "pip",
            "install",
            "--python",
            &exe.to_string_lossy(),
            "--only-binary",
            ":all:",
            "--index-url",
            "https://pypi.org/simple",
            PACKAGE,
        ],
        &log,
        600,
    )?;
    run(data, &exe, &["-I", "-c", CHECK], &log, 30)?;
    let receipt = serde_json::to_vec(&Receipt {
        version: VERSION.into(),
        environment: id,
    })
    .map_err(|e| e.to_string())?;
    let path = root.join("ready.json");
    let temp = stage(&path, &receipt)?;
    fs::rename(temp, path).map_err(|e| e.to_string())?;
    Ok(())
}

fn addon_source(data: &Path) -> Option<PathBuf> {
    let exe = command(data)?;
    let environment = exe.parent()?.parent()?;
    Some(environment.join(if cfg!(windows) {
        "Lib/site-packages/blender_mcp/bundled/addon.py"
    } else {
        "lib/python3.12/site-packages/blender_mcp/bundled/addon.py"
    }))
}

fn addon_targets(home: &Path, app_data: &Path) -> Vec<PathBuf> {
    let base = if cfg!(windows) {
        app_data.join("Blender Foundation/Blender")
    } else if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Blender")
    } else {
        home.join(".config/blender")
    };
    let mut targets: Vec<_> = fs::read_dir(base)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            let parts: Vec<_> = name.split('.').collect();
            parts.len() == 2
                && parts
                    .iter()
                    .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(|e| e.path().join("scripts/addons/blender_mcp.py"))
        .collect();
    targets.sort();
    targets
}

/// Preserve different existing addons (including hand-edited ones). The user
/// explicitly updates those in Blender; one-click must not silently replace them.
pub fn prepare_addon(data: &Path, home: &Path, app_data: &Path) -> Result<()> {
    let source = addon_source(data).ok_or("Blender MCP 运行环境尚未准备")?;
    let bytes = fs::read(source).map_err(|e| format!("无法读取 Blender 插件：{e}"))?;
    for path in addon_targets(home, app_data) {
        no_links(&path)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("无法检查 Blender 插件 {}：{e}", path.display())),
        }
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        // Publish the complete staged file without replacing an addon that
        // appeared during preparation (rename would replace it on Unix).
        let temp = stage(&path, &bytes)?;
        let result = fs::hard_link(&temp, &path);
        let _ = fs::remove_file(temp);
        if let Err(e) = result {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                continue;
            }
            return Err(format!("无法写入 Blender 插件 {}：{e}", path.display()));
        }
        crate::logging::event(
            "info",
            "integration",
            format!("已准备 Blender 插件文件：{}", path.display()),
        );
    }
    Ok(())
}

pub fn status(data: &Path, home: &Path, app_data: &Path) -> Value {
    let bundled = addon_source(data).and_then(|p| fs::read(p).ok());
    let runtime_ready = bundled.is_some() && command(data).is_some_and(|p| p.is_file());
    let targets = addon_targets(home, app_data);
    let addon_ready = !targets.is_empty()
        && bundled.as_ref().is_some_and(|bytes| {
            targets
                .iter()
                .all(|p| fs::read(p).is_ok_and(|current| current == *bytes))
        });
    json!({"runtimeReady":runtime_ready, "addonReady":addon_ready,
        "addonMessage": if addon_ready { "插件文件已准备；在偏好设置中启用 MCP for Blender 后，还需在 3D 视图按 N，打开 MCP for Blender，点击 Connect to MCP server" }
            else if targets.is_empty() { "请先启动一次 Blender 创建用户目录，再点击补全接入；随后启用插件并启动服务" }
            else { "插件缺失或与内置版本不同；已有插件文件会保留，请在 Blender 中检查安装或更新后启用并启动服务" }})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, PathBuf, PathBuf) {
        let data = std::env::temp_dir().join(format!("cyapi-blender-{}", uuid::Uuid::new_v4()));
        let home = data.join("home");
        let app = data.join("app");
        fs::create_dir_all(root(&data)).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let exe = python(&root(&data).join(&id));
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, "fixture").unwrap();
        fs::write(
            root(&data).join("ready.json"),
            serde_json::to_vec(&Receipt {
                version: VERSION.into(),
                environment: id,
            })
            .unwrap(),
        )
        .unwrap();
        let source = addon_source(&data).unwrap();
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(source, "# bundled addon fixture").unwrap();
        (data, home, app)
    }

    #[test]
    fn prepares_missing_addons_without_overwriting_existing_files_or_claiming_a_live_service() {
        let (data, home, app) = fixture();
        let base = if cfg!(windows) {
            app.join("Blender Foundation/Blender")
        } else if cfg!(target_os = "macos") {
            home.join("Library/Application Support/Blender")
        } else {
            home.join(".config/blender")
        };
        assert_eq!(status(&data, &home, &app)["runtimeReady"], true);
        assert_eq!(status(&data, &home, &app)["addonReady"], false);
        fs::create_dir_all(base.join("4.2")).unwrap();
        fs::create_dir_all(base.join("4.3")).unwrap();
        fs::create_dir_all(base.join("4.2.backup")).unwrap();
        prepare_addon(&data, &home, &app).unwrap();
        let first = base.join("4.2/scripts/addons/blender_mcp.py");
        assert_eq!(
            fs::read_to_string(&first).unwrap(),
            "# bundled addon fixture"
        );
        assert!(!base.join("4.2.backup/scripts").exists());
        assert_eq!(status(&data, &home, &app)["addonReady"], true);
        fs::write(&first, "# user addon edits").unwrap();
        prepare_addon(&data, &home, &app).unwrap();
        assert_eq!(fs::read_to_string(&first).unwrap(), "# user addon edits");
        assert_eq!(status(&data, &home, &app)["addonReady"], false);
        assert!(status(&data, &home, &app)["addonMessage"]
            .as_str()
            .unwrap()
            .contains("已有插件文件会保留"));
        fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn fixed_uv_archive_member_is_required_and_other_paths_are_never_extracted() {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("../../escape", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"not executable").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        assert!(extract_uv(&bytes).unwrap_err().contains("缺少可执行文件"));
        assert!(extract_uv(b"not a zip").is_err());
    }

    #[test]
    #[ignore = "downloads uv, managed Python and Blender MCP only into an isolated temporary directory"]
    fn real_isolated_blender_environment_preparation() {
        let data =
            std::env::temp_dir().join(format!("cyapi-blender-live-{}", uuid::Uuid::new_v4()));
        prepare(&data)
            .unwrap_or_else(|error| panic!("{error}; isolated test directory: {}", data.display()));
        let first = command(&data).unwrap();
        assert!(first.is_file());
        assert!(addon_source(&data).unwrap().is_file());
        prepare(&data).unwrap();
        assert_eq!(command(&data).unwrap(), first);
        fs::remove_dir_all(data).unwrap();
    }
    #[test]
    fn receipt_paths_cannot_escape_managed_environment() {
        let data = std::env::temp_dir().join(format!("cyapi-blender-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root(&data)).unwrap();
        let path = root(&data).join("ready.json");
        fs::write(
            &path,
            r#"{"version":"2.2.4","environment":"../../outside"}"#,
        )
        .unwrap();
        assert!(command(&data).is_none());
        let id = uuid::Uuid::new_v4().to_string();
        fs::write(
            &path,
            serde_json::to_vec(&Receipt {
                version: VERSION.into(),
                environment: id.clone(),
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(command(&data), Some(python(&root(&data).join(id))));
        assert_eq!(
            status(&data, &data.join("home"), &data.join("app"))["runtimeReady"],
            false
        );
        fs::remove_dir_all(data).unwrap();
    }
}
