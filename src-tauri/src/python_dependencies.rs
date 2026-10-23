//! Hash-locked, server-prepared wheels. Reuse an existing interpreter; never alter its packages.
use crate::{
    configuration::{no_links, stage},
    runtime::{self, Runtime},
    service::hash,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub implementation: String,
    pub python: String,
    pub abi: String,
    pub platform: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub target: Target,
    path: String,
    sha256: String,
    size: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wheel {
    name: String,
    version: String,
    file_name: String,
    path: String,
    sha256: String,
    size: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Lock {
    file_name: String,
    sha256: String,
    size: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema: u32,
    kind: String,
    target: Target,
    bootstrap_pip: String,
    lock: Lock,
    wheels: Vec<Wheel>,
    verified_offline: bool,
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn token(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 200
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+!-".contains(&b))
}
pub fn validate_profile(p: &Profile) -> Result<()> {
    let parts: Vec<_> = p.target.python.split('.').collect();
    if !token(&p.id)
        || !digest(&p.sha256)
        || p.size == 0
        || p.size > 1_000_000
        || p.path != format!("/python/bundles/{}/manifest.json", p.sha256)
        || p.target.implementation != "cpython"
        || !crate::platform::known(&p.target.platform)
        || parts.len() != 2
        || parts[0] != "3"
        || parts[1].parse::<u32>().is_err()
        || p.target.abi != format!("cp{}", p.target.python.replace('.', ""))
    {
        return Err("Python 依赖配置无效".into());
    }
    Ok(())
}
pub fn choose<'a>(
    profiles: &'a [Profile],
    runtimes: &'a [Runtime],
) -> Result<(&'a Profile, &'a Runtime)> {
    // Only profiles built for this platform can be installed here.
    let usable: Vec<&Profile> = profiles
        .iter()
        .filter(|p| p.target.platform == crate::platform::current())
        .collect();
    for runtime in runtimes.iter().filter(|r| r.kind == "python") {
        if let Some((major, minor, _)) = runtime::version_tuple(&runtime.version) {
            if let Some(profile) = usable
                .iter()
                .find(|p| p.target.python == format!("{major}.{minor}"))
            {
                return Ok((profile, runtime));
            }
        }
    }
    if usable.is_empty() {
        return Err(format!(
            "此 Skill 的离线 Python 依赖只提供 {} 版本，当前系统为 {}；请在对应系统上安装，或改用不依赖离线 Python 包的 Skill。",
            profiles
                .iter()
                .map(|p| p.target.platform.as_str())
                .collect::<Vec<_>>()
                .join(" / "),
            crate::platform::current()
        ));
    }
    Err(format!(
        "离线依赖需要 Python {}，本机没有该版本；客户端会在安装时自动准备目录提供的运行环境包。",
        usable
            .iter()
            .map(|p| p.target.python.as_str())
            .collect::<Vec<_>>()
            .join(" / ")
    ))
}
fn local(data: &Path, remote: &str) -> PathBuf {
    data.join(remote.trim_start_matches('/'))
}
fn checked(bytes: &[u8], size: u64, sha256: &str) -> Result<()> {
    if bytes.len() as u64 != size || hash(bytes) != sha256 {
        return Err("Python 离线文件大小或 SHA256 不匹配".into());
    }
    Ok(())
}
fn put(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = stage(path, bytes)?;
    fs::rename(temp, path).map_err(|e| e.to_string())
}
fn manifest(p: &Profile, bytes: &[u8]) -> Result<Manifest> {
    validate_profile(p)?;
    checked(bytes, p.size, &p.sha256)?;
    let m: Manifest = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if m.schema != 1
        || m.kind != "python-wheels"
        || m.target != p.target
        || !m.verified_offline
        || m.wheels.is_empty()
        || m.wheels.len() > 128
        || m.lock.file_name != "requirements.lock"
        || !digest(&m.lock.sha256)
        || m.lock.size > 100_000
    {
        return Err("Python 离线清单无效".into());
    }
    let mut names = HashSet::new();
    let mut filenames = HashSet::new();
    let mut total = 0;
    for w in &m.wheels {
        total += w.size;
        if !token(&w.name)
            || !token(&w.version)
            || !token(&w.file_name)
            || !w.file_name.ends_with(".whl")
            || !digest(&w.sha256)
            || w.size == 0
            || w.size > 200_000_000
            || w.path != format!("/python/wheels/{}/{}", w.sha256, w.file_name)
            || !names.insert(w.name.to_lowercase().replace('_', "-"))
            || !filenames.insert(w.file_name.to_lowercase())
        {
            return Err("Python wheel 声明无效".into());
        }
    }
    if total > 500_000_000
        || !m
            .wheels
            .iter()
            .any(|w| w.name == "pip" && w.file_name == m.bootstrap_pip)
    {
        return Err("Python 离线包缺少 pip 或超过限制".into());
    }
    Ok(m)
}
fn read(data: &Path, p: &Profile) -> Result<Manifest> {
    no_links(&local(data, &p.path))?;
    let bytes = fs::read(local(data, &p.path)).map_err(|_| "请先下载 Python 离线依赖")?;
    manifest(p, &bytes)
}
pub fn cached(data: &Path, p: &Profile) -> bool {
    let Ok(m) = read(data, p) else { return false };
    std::iter::once((
        p.path.replace("manifest.json", "requirements.lock"),
        m.lock.size,
        m.lock.sha256,
    ))
    .chain(m.wheels.into_iter().map(|w| (w.path, w.size, w.sha256)))
    .all(|(path, size, sha)| {
        no_links(&local(data, &path)).is_ok()
            && fs::read(local(data, &path)).is_ok_and(|b| checked(&b, size, &sha).is_ok())
    })
}
pub fn download(
    data: &Path,
    p: &Profile,
    fetch: impl Fn(&str, u64) -> Result<Vec<u8>>,
) -> Result<()> {
    validate_profile(p)?;
    if cached(data, p) {
        return Ok(());
    }
    let bytes = fetch(&p.path, p.size)?;
    let m = manifest(p, &bytes)?;
    let artifacts = std::iter::once((
        p.path.replace("manifest.json", "requirements.lock"),
        m.lock.size,
        m.lock.sha256,
    ))
    .chain(m.wheels.into_iter().map(|w| (w.path, w.size, w.sha256)));
    for (remote, size, sha) in artifacts {
        let path = local(data, &remote);
        no_links(&path)?;
        if fs::read(&path).is_ok_and(|b| checked(&b, size, &sha).is_ok()) {
            continue;
        }
        crate::logging::event("info", "python", format!("下载离线依赖：{remote}"));
        let file = fetch(&remote, size)?;
        checked(&file, size, &sha)?;
        put(&path, &file)?;
    }
    put(&local(data, &p.path), &bytes)
}
fn run(exe: &Path, args: &[String], log: &Path) -> Result<()> {
    let output = fs::File::create(log).map_err(|e| e.to_string())?;
    let mut command = Command::new(exe);
    command
        .args(args)
        .env("PIP_NO_INDEX", "1")
        .env("PYTHONNOUSERSITE", "1")
        .env_remove("PYTHONPATH")
        .env_remove("PYTHONHOME")
        .stdin(Stdio::null())
        .stdout(output.try_clone().map_err(|e| e.to_string())?)
        .stderr(output);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("无法启动 Python：{e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(_) => {
                let text = fs::read_to_string(log).unwrap_or_default();
                return Err(format!(
                    "Python 离线环境准备失败：{}",
                    text.chars()
                        .rev()
                        .take(1800)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>()
                ));
            }
            None if start.elapsed() < Duration::from_secs(300) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Python 环境准备超时".into());
            }
        }
    }
}
/// The interpreter inside a virtual environment. Windows keeps `Scripts`, every
/// other platform uses `bin`.
fn venv_executable(root: &Path) -> PathBuf {
    if cfg!(windows) {
        return root.join("Scripts/python.exe");
    }
    for name in ["python3", "python"] {
        let candidate = root.join("bin").join(name);
        if candidate.is_file() {
            return candidate;
        }
    }
    root.join("bin/python3")
}
pub fn environment(data: &Path, p: &Profile, base: &Runtime) -> Result<Runtime> {
    if !cached(data, p) {
        return Err("Python 离线依赖尚未下载完整".into());
    }
    let m = read(data, p)?;
    let current = runtime::probe(Path::new(&base.path), "python", &base.source)
        .ok_or("共享 Python 已不可用，请重新检测")?;
    if current.version != base.version {
        return Err("Python 版本已变化，请重新检测".into());
    }
    let key = hash(format!("{}|{}", base.path, base.version).as_bytes());
    let root = data.join(format!("python/environments/{}-{}", p.sha256, &key[..12]));
    no_links(&root)?;
    let receipt = root.join(".cyapi-ready");
    if fs::read_to_string(&receipt).is_ok_and(|s| s == p.sha256) {
        return runtime::probe(&venv_executable(&root), "python", "shared-venv")
            .ok_or("Python 依赖环境已不可用，请重新检测".into());
    }
    // Only this content-addressed, uncommitted managed environment is retried.
    if root.exists() {
        fs::remove_dir_all(&root).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let log = root.join("install.log");
    crate::logging::event(
        "info",
        "python",
        format!("复用 Python {}，准备隔离依赖环境", base.version),
    );
    run(
        Path::new(&base.path),
        &[
            "-I".into(),
            "-m".into(),
            "venv".into(),
            "--without-pip".into(),
            root.to_string_lossy().into(),
        ],
        &log,
    )?;
    let exe = venv_executable(&root);
    if !exe.is_file() {
        return Err("Python 隔离环境缺少解释器".into());
    }
    let wheelhouse = root.join("wheelhouse");
    fs::create_dir_all(&wheelhouse).map_err(|e| e.to_string())?;
    for w in &m.wheels {
        let source = local(data, &w.path);
        let target = wheelhouse.join(&w.file_name);
        if fs::hard_link(&source, &target).is_err() {
            fs::copy(source, target).map_err(|e| e.to_string())?;
        }
    }
    let args = vec!["-I".into(), "-c".into(), "import runpy,sys; sys.path.insert(0,sys.argv.pop(1)); runpy.run_module('pip',run_name='__main__')".into(),
        wheelhouse.join(&m.bootstrap_pip).to_string_lossy().into(), "--isolated".into(), "--disable-pip-version-check".into(), "install".into(),
        "--no-index".into(), "--no-cache-dir".into(), "--no-compile".into(), "--only-binary=:all:".into(), "--require-hashes".into(),
        "--find-links".into(), wheelhouse.to_string_lossy().into(), "--requirement".into(),
        local(data, &p.path.replace("manifest.json", "requirements.lock")).to_string_lossy().into()];
    run(&exe, &args, &log)?;
    run(
        &exe,
        &[
            "-I".into(),
            "-m".into(),
            "pip".into(),
            "--isolated".into(),
            "check".into(),
        ],
        &log,
    )?;
    let resolved = runtime::probe(&exe, "python", "shared-venv").ok_or("离线依赖环境无法运行")?;
    fs::remove_dir_all(&wheelhouse).map_err(|e| e.to_string())?;
    put(&receipt, p.sha256.as_bytes())?;
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_python_abi_and_artifact_integrity_are_required() {
        let platform = crate::platform::current();
        let p: Profile = serde_json::from_value(serde_json::json!({"id":"example","target":{"implementation":"cpython","python":"3.12","abi":"cp312","platform":platform},"path":format!("/python/bundles/{}/manifest.json", "a".repeat(64)),"sha256":"a".repeat(64),"size":100})).unwrap();
        validate_profile(&p).unwrap();
        let mut runtime = Runtime {
            kind: "python".into(),
            version: "3.14.2".into(),
            path: "python.exe".into(),
            source: "codex".into(),
        };
        assert!(choose(&[p.clone()], &[runtime.clone()]).is_err());
        runtime.version = "3.12.14".into();
        assert!(choose(&[p.clone()], &[runtime]).is_ok());
        let mut bad = p.clone();
        bad.path = "/../../evil".into();
        assert!(validate_profile(&bad).is_err());
        assert!(manifest(&p, b"modified manifest").is_err());
    }
}
