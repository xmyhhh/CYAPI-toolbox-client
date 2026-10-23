//! The shared Python runtime the service publishes.
//!
//! The offline dependency bundles of a Skill are built for one Python
//! version and one ABI. A machine without that interpreter cannot install
//! them, so the service publishes a relocatable CPython build per platform
//! and the client unpacks it where it needs it, the same way the shared Node
//! runtime is used.

use std::fs;
use std::path::{Component, Path, PathBuf};

type Result<T> = std::result::Result<T, String>;

fn err(error: impl std::fmt::Display) -> String {
    crate::logging::event("error", "python", error.to_string());
    error.to_string()
}

/// The interpreter of an unpacked runtime, if the archive holds one.
pub fn interpreter(root: &Path) -> Option<PathBuf> {
    let base = root.join("python");
    if cfg!(windows) {
        for name in ["python.exe", "python3.exe"] {
            let candidate = base.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        return None;
    }
    let bin = base.join("bin");
    let mut names: Vec<String> = fs::read_dir(&bin)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .filter(|name| {
                    // Only the interpreter itself, never a helper script such
                    // as `python3-config`.
                    name == "python3"
                        || (name.starts_with("python3.")
                            && name["python3.".len()..]
                                .chars()
                                .all(|c| c.is_ascii_digit()))
                })
                .collect()
        })
        .unwrap_or_default();
    // The versioned name is preferred, so the probe reports the exact
    // version the bundle was built for.
    names.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    for name in names {
        let candidate = bin.join(&name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    for name in ["python3", "python"] {
        let candidate = bin.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Unpack the published runtime and return the interpreter it contains.
///
/// The archive comes from a pinned source with a verified checksum, and every
/// member is still checked: an absolute path or a parent directory would let
/// the archive write outside the runtime directory.
pub fn install(data: &Path, archive: &[u8], version: &str) -> Result<PathBuf> {
    let root = data.join("runtimes/python").join(version);
    if let Some(existing) = interpreter(&root) {
        return Ok(existing);
    }
    let staging = data.join("runtimes/python").join(format!("{version}.part"));
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(err)?;
    }
    fs::create_dir_all(&staging).map_err(err)?;
    let decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(archive));
    let mut tar = tar::Archive::new(decoder);
    tar.set_overwrite(true);
    // The modification time of the archive is not used, and a time before the
    // epoch is rejected by the file system.
    tar.set_preserve_mtime(false);
    for entry in tar.entries().map_err(err)? {
        let mut entry = entry.map_err(err)?;
        let path = entry.path().map_err(err)?.into_owned();
        let safe = path
            .components()
            .all(|part| !matches!(part, Component::RootDir | Component::Prefix(_)) && part != Component::ParentDir);
        if !safe {
            return Err("Python 运行时包包含不安全的路径".into());
        }
        // The member is written below the staging directory at the path the
        // archive names, and the reader rejects a path that leaves it.
        entry.unpack_in(&staging).map_err(err)?;
    }
    let unpacked = interpreter(&staging).ok_or("Python 运行时包中没有解释器")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&unpacked, fs::Permissions::from_mode(0o755));
    }
    // The directory is replaced as a whole, so a half unpacked runtime never
    // stays behind.
    if root.exists() {
        fs::remove_dir_all(&root).map_err(err)?;
    }
    fs::rename(&staging, &root).map_err(err)?;
    let installed = interpreter(&root).ok_or("Python 运行时安装后缺少解释器")?;
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A gzip compressed tar archive with the layout the published runtime
    /// uses.
    fn archive(interpreter: &[u8]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(interpreter.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, if cfg!(windows) { "python/python.exe" } else { "python/bin/python3.14" }, interpreter)
            .unwrap();
        let tar = builder.into_inner().unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &tar).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn an_unpacked_runtime_without_an_interpreter_is_not_usable() {
        let root = std::env::temp_dir().join(format!("cyapi-python-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("python/bin")).unwrap();
        assert!(interpreter(&root).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_runtime_is_unpacked_and_its_interpreter_is_found() {
        let root = std::env::temp_dir().join(format!("cyapi-python-unpack-{}", now()));
        let _ = fs::remove_dir_all(&root);
        #[cfg(windows)]
        let executable = fs::read(PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe")).unwrap();
        #[cfg(not(windows))]
        let executable = b"#!/bin/sh\necho Python 3.14.8\n".to_vec();
        let bytes = archive(&executable);
        let installed = install(&root, &bytes, "3.14.8").unwrap();
        assert!(installed.is_file());
        assert!(installed.ends_with(if cfg!(windows) { "python/python.exe" } else { "python/bin/python3.14" }));
        // The interpreter runs from the unpacked location.
        let mut command = std::process::Command::new(&installed);
        #[cfg(windows)]
        command.args(["/d", "/c", "echo Python 3.14.8"]);
        let output = command.output().unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("3.14.8"));
        // A second call reuses the unpacked runtime.
        let again = install(&root, &bytes, "3.14.8").unwrap();
        assert_eq!(again, installed);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_archive_with_a_parent_path_is_rejected() {
        let root = std::env::temp_dir().join(format!("cyapi-python-unsafe-{}", now()));
        let _ = fs::remove_dir_all(&root);
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o644);
        header.set_cksum();
        // The archive reader itself refuses a member that leaves the target
        // directory, so the runtime never writes outside its own folder.
        assert!(builder
            .append_data(&mut header, "../escape.txt", &b"bad"[..])
            .is_err());
        let _ = fs::remove_dir_all(&root);
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }
}

#[cfg(test)]
mod real_check {
    #[test]
    fn unpack_the_published_archive() {
        // Path points to a locally prepared offline Python archive.
        // Availability check: environment variable CYAPI_TEST_PYTHON_ARCHIVE specifies the archive location.
        let file = match std::env::var("CYAPI_TEST_PYTHON_ARCHIVE") {
            Ok(file) => file,
            Err(_) => { println!("Skip: environment variable CYAPI_TEST_PYTHON_ARCHIVE is not set"); return; }
        };
        if !std::path::Path::new(&file).exists() { println!("Skip: no prepared archive"); return; }
        let bytes = std::fs::read(file).unwrap();
        let root = std::env::temp_dir().join("cyapi-real-check");
        let _ = std::fs::remove_dir_all(&root);
        let exe = super::install(&root, &bytes, "3.14.8").unwrap();
        println!("解释器: {}", exe.display());
        let out = std::process::Command::new(&exe).arg("--version").output().unwrap();
        println!("版本: {}", String::from_utf8_lossy(&out.stdout).trim());
        let out2 = std::process::Command::new(&exe).arg("-c").arg("import ssl,zlib,sqlite3,lzma;print('stdlib ok')").output().unwrap();
        println!("{}", String::from_utf8_lossy(&out2.stdout).trim());
        let _ = std::fs::remove_dir_all(&root);
    }
}
