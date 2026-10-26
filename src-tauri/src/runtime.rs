use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Runtime {
    pub kind: String,
    pub version: String,
    pub path: String,
    pub source: String,
}

pub fn normalize_path(path: &Path) -> PathBuf {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = canonical.to_string_lossy();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{}", unc))
    } else if let Some(normal) = text.strip_prefix(r"\\?\") {
        PathBuf::from(normal)
    } else {
        canonical
    }
}

pub fn version_tuple(value: &str) -> Option<(u32, u32, u32)> {
    let value = value
        .trim()
        .trim_start_matches('v')
        .trim_start_matches("Python ");
    let numbers: Vec<_> = value.split('.').collect();
    if numbers.len() != 3 {
        return None;
    }
    Some((
        numbers[0].parse().ok()?,
        numbers[1].parse().ok()?,
        numbers[2].parse().ok()?,
    ))
}
pub fn compatible(runtime: &Runtime, kind: &str, minimum: &str) -> bool {
    runtime.kind == kind
        && matches!((version_tuple(&runtime.version), version_tuple(minimum)), (Some(v), Some(m)) if v >= m)
}

pub fn probe(path: &Path, kind: &str, source: &str) -> Option<Runtime> {
    if !path.is_file()
        || path
            .to_string_lossy()
            .to_lowercase()
            .contains("windowsapps")
    {
        return None;
    }
    let mut command = Command::new(path);
    command
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < Duration::from_secs(3) => {
                thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    let text = String::from_utf8_lossy(raw);
    let version = if kind == "python" {
        text.trim().strip_prefix("Python ")?
    } else {
        text.trim().strip_prefix('v')?
    };
    version_tuple(version)?;
    Some(Runtime {
        kind: kind.into(),
        version: version.into(),
        path: normalize_path(path).to_string_lossy().into(),
        source: source.into(),
    })
}

fn children(path: &Path) -> Vec<PathBuf> {
    fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .take(40)
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

/// The file names a runtime executable may use on this platform, most
/// specific first. Windows keeps the single `.exe` name, macOS and Linux also
/// accept the versioned `python3.x` names used by the python.org installer.
fn executable_names(kind: &str) -> Vec<String> {
    if cfg!(windows) {
        return vec![format!("{kind}.exe")];
    }
    if kind == "python" {
        return vec![
            "python3".to_string(),
            "python".to_string(),
            "python3.14".to_string(),
            "python3.13".to_string(),
            "python3.12".to_string(),
        ];
    }
    vec!["node".to_string()]
}

pub fn discover(home: &Path, data: &Path) -> Vec<Runtime> {
    let mut candidates: Vec<(PathBuf, &str, &str)> = vec![];
    let node_names = executable_names("node");
    let python_names = executable_names("python");
    // Reuse Codex's actual executable, without assuming it is on the user's PATH.
    for root in children(&home.join(".cache/codex-runtimes")) {
        if root
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("codex-runtime-install-"))
        {
            continue;
        }
        for name in &node_names {
            candidates.push((
                root.join("dependencies/node/bin").join(name),
                "node",
                "codex",
            ));
        }
        for name in &python_names {
            candidates.push((
                root.join("dependencies/python/bin").join(name),
                "python",
                "codex",
            ));
            // The Windows Codex bundle keeps python.exe directly in python/.
            candidates.push((
                root.join("dependencies/python").join(name),
                "python",
                "codex",
            ));
        }
    }
    for kind in ["node", "python"] {
        for root in children(&data.join("runtimes").join(kind)) {
            for name in executable_names(kind) {
                candidates.push((root.join(&name), kind, "managed"));
            }
        }
    }
    // The shared Python runtime keeps the interpreter inside its own
    // directory, the same layout the published archive uses.
    for root in children(&data.join("runtimes").join("python")) {
        for name in executable_names("python") {
            candidates.push((
                root.join("python").join("bin").join(name),
                "python",
                "managed",
            ));
        }
    }
    if let Some(paths) = env::var_os("PATH") {
        for dir in env::split_paths(&paths) {
            for name in &node_names {
                candidates.push((dir.join(name), "node", "system"));
            }
            for name in &python_names {
                candidates.push((dir.join(name), "python", "system"));
            }
        }
    }
    if cfg!(windows) {
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            for root in children(&PathBuf::from(local).join("Programs/Python")) {
                candidates.push((root.join("python.exe"), "python", "system"));
            }
        }
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = env::var_os(variable) {
                let root = PathBuf::from(root);
                candidates.push((root.join("nodejs/node.exe"), "node", "system"));
                for entry in children(&root) {
                    if entry
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("Python"))
                    {
                        candidates.push((entry.join("python.exe"), "python", "system"));
                    }
                }
            }
        }
    }
    if cfg!(target_os = "macos") {
        // The python.org installer and Homebrew keep interpreters here.
        let frameworks = PathBuf::from("/Library/Frameworks/Python.framework/Versions");
        for root in children(&frameworks) {
            for name in &python_names {
                candidates.push((root.join("bin").join(name), "python", "system"));
            }
        }
        for prefix in ["/opt/homebrew/bin", "/usr/local/bin"] {
            let prefix = PathBuf::from(prefix);
            for name in &node_names {
                candidates.push((prefix.join(name), "node", "system"));
            }
            for name in &python_names {
                candidates.push((prefix.join(name), "python", "system"));
            }
        }
    }
    let mut seen = HashSet::new();
    let mut found = vec![];
    for (path, kind, source) in candidates {
        if seen.insert(
            normalize_path(&path)
                .to_string_lossy()
                .replace('/', "\\")
                .to_lowercase(),
        ) {
            if let Some(runtime) = probe(&path, kind, source) {
                found.push(runtime);
            }
        }
    }
    // Prefer reusable managed runtimes, then Codex, then system. Each executable is probed.
    found.sort_by_key(|r| match r.source.as_str() {
        "managed" => 0,
        "codex" => 1,
        _ => 2,
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_runtime_versions() {
        let runtime = Runtime {
            kind: "node".into(),
            version: "24.14.0".into(),
            path: "test".into(),
            source: "system".into(),
        };
        assert!(compatible(&runtime, "node", "20.0.0"));
        assert!(!compatible(&runtime, "node", "25.0.0"));
        assert!(!compatible(&runtime, "python", "3.0.0"));
        assert!(version_tuple("20.1.0;evil").is_none());
    }
}
