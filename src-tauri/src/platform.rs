//! Platform conventions shared by the runtime, installer and catalog code.
//!
//! The server labels every platform specific artifact with a catalog platform
//! identifier. Windows keeps the historical `windows-x64` label, macOS and
//! Linux use their own labels, so a macOS build never accepts a Windows binary
//! and a Windows build never accepts a macOS binary.

pub const WINDOWS_X64: &str = "windows-x64";
pub const MACOS_ARM64: &str = "macos-arm64";
pub const MACOS_X64: &str = "macos-x64";
pub const LINUX_X64: &str = "linux-x64";

/// The catalog platform identifier of the running machine.
pub fn current() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        WINDOWS_X64
    }
    #[cfg(target_os = "macos")]
    {
        if cfg!(target_arch = "aarch64") {
            MACOS_ARM64
        } else {
            MACOS_X64
        }
    }
    #[cfg(target_os = "linux")]
    {
        LINUX_X64
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        "unsupported"
    }
}

/// The public path of a shared runtime artifact.
pub fn runtime_path(version: &str, identifier: &str) -> String {
    format!("/runtimes/node-{}-{}", version, artifact_suffix(identifier))
}

/// The public path of a shared Python runtime.
///
/// The interpreter is published as a relocatable archive, so the client can
/// unpack it where it needs it instead of asking the user for a system wide
/// installation.
pub fn python_runtime_path(version: &str, identifier: &str) -> String {
    format!("/runtimes/python-{}-{}.tar.gz", version, identifier)
}

/// True when the identifier names a platform this build understands.
pub fn known(identifier: &str) -> bool {
    matches!(
        identifier,
        WINDOWS_X64 | MACOS_ARM64 | MACOS_X64 | LINUX_X64
    )
}

/// The file name suffix the server uses for artifacts of the given platform.
pub fn artifact_suffix(identifier: &str) -> &'static str {
    match identifier {
        WINDOWS_X64 => "win-x64.exe",
        MACOS_ARM64 => "macos-arm64",
        MACOS_X64 => "macos-x64",
        _ => "linux-x64",
    }
}

/// The file name extension of an executable of the given platform.
pub fn artifact_extension(identifier: &str) -> &'static str {
    if identifier == WINDOWS_X64 {
        ".exe"
    } else {
        ""
    }
}

/// The executable file name for a runtime, for example `node` or `node.exe`.
pub fn executable(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// The name of the generated launcher script for a runtime.
pub fn launcher_script(stem: &str) -> String {
    if cfg!(windows) {
        format!("run-{stem}.ps1")
    } else {
        format!("run-{stem}.sh")
    }
}

/// Escape a value for a POSIX shell single quoted string.
pub fn shell_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}

/// True when downloaded bytes start with the magic number of an executable
/// that can run on this platform.
pub fn is_executable(bytes: &[u8]) -> bool {
    if cfg!(windows) {
        bytes.starts_with(b"MZ")
    } else if cfg!(target_os = "macos") {
        // Mach-O 64-bit in either byte order, and universal binaries.
        bytes.starts_with(&[0xCF, 0xFA, 0xED, 0xFE])
            || bytes.starts_with(&[0xCE, 0xFA, 0xED, 0xFE])
            || bytes.starts_with(&[0xCA, 0xFE, 0xBA, 0xBE])
    } else {
        bytes.starts_with(&[0x7F, b'E', b'L', b'F'])
    }
}

/// The relative directory that holds per-user application data.
pub fn app_data_relative() -> &'static str {
    if cfg!(windows) {
        "AppData/Roaming"
    } else if cfg!(target_os = "macos") {
        "Library/Application Support"
    } else {
        ".local/share"
    }
}

/// The relative directory that holds per-user local application data.
pub fn local_app_data_relative() -> &'static str {
    if cfg!(windows) {
        "AppData/Local"
    } else if cfg!(target_os = "macos") {
        "Library/Caches"
    } else {
        ".cache"
    }
}

/// The directory that holds per-user application data on this platform.
/// An isolated run keeps every path inside the given home directory.
pub fn app_data_dir(home: &std::path::Path, isolated: bool) -> std::path::PathBuf {
    // macOS keeps application data inside the home directory, so there is no
    // environment variable that an isolated run has to ignore.
    #[cfg(target_os = "macos")]
    let _ = isolated;
    #[cfg(windows)]
    if !isolated {
        if let Some(value) = std::env::var_os("APPDATA") {
            return std::path::PathBuf::from(value);
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    if !isolated {
        if let Some(value) = std::env::var_os("XDG_DATA_HOME") {
            return std::path::PathBuf::from(value);
        }
    }
    home.join(app_data_relative())
}

/// The directory that holds per-user local application data on this platform.
pub fn local_app_data_dir(home: &std::path::Path, isolated: bool) -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    let _ = isolated;
    #[cfg(windows)]
    if !isolated {
        if let Some(value) = std::env::var_os("LOCALAPPDATA") {
            return std::path::PathBuf::from(value);
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    if !isolated {
        if let Some(value) = std::env::var_os("XDG_CACHE_HOME") {
            return std::path::PathBuf::from(value);
        }
    }
    home.join(local_app_data_relative())
}

/// The environment variable that holds the user's home directory.
pub fn home_variable() -> &'static str {
    if cfg!(windows) {
        "USERPROFILE"
    } else {
        "HOME"
    }
}

/// The root directory that holds the toolbox data, cache and install records.
pub fn data_root(home: &std::path::Path) -> std::path::PathBuf {
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"));
    }
    if cfg!(target_os = "macos") {
        return home.join("Library/Application Support");
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
}

/// True when the running process uses the given directory as the user's home.
/// An isolated test or portable run sets its own home and is detected here.
pub fn is_default_home(home: &std::path::Path) -> bool {
    match std::env::var_os(home_variable()) {
        Some(value) => std::path::PathBuf::from(value) == home,
        None => false,
    }
}

/// Open a file or directory with the system file manager.
pub fn open_path(path: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("explorer.exe")
            .arg(path)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("无法打开目录：{error}"))
    }
    #[cfg(not(windows))]
    {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        std::process::Command::new(program)
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("无法打开目录：{error}"))
    }
}

/// Open a URL in the default browser.
pub fn open_url(target: &str) -> Result<(), String> {
    if !reqwest::Url::parse(target).is_ok_and(|url| {
        url.scheme() == "https" && url.username().is_empty() && url.password().is_none()
    }) || target.chars().any(char::is_control)
    {
        return Err("只允许打开有效的 HTTPS 地址".into());
    }
    #[cfg(windows)]
    {
        use std::{os::windows::ffi::OsStrExt, ptr};
        #[link(name = "shell32")]
        extern "system" {
            fn ShellExecuteW(
                window: *mut std::ffi::c_void,
                operation: *const u16,
                file: *const u16,
                parameters: *const u16,
                directory: *const u16,
                show: i32,
            ) -> isize;
        }
        let target: Vec<u16> = std::ffi::OsStr::new(target)
            .encode_wide()
            .chain(Some(0))
            .collect();
        // ShellExecute passes the URL directly to the default browser; query
        // parameters such as &software= are never interpreted by cmd.exe.
        let result = unsafe {
            ShellExecuteW(
                ptr::null_mut(),
                ptr::null(),
                target.as_ptr(),
                ptr::null(),
                ptr::null(),
                1,
            )
        };
        if result > 32 {
            Ok(())
        } else {
            Err(format!("无法打开浏览器（系统错误 {result}）"))
        }
    }
    #[cfg(not(windows))]
    {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        std::process::Command::new(program)
            .arg(target)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("无法打开浏览器：{error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_running_platform_is_known() {
        assert!(known(current()));
    }

    #[test]
    fn platform_labels_map_to_distinct_artifact_names() {
        assert_eq!(artifact_suffix(WINDOWS_X64), "win-x64.exe");
        assert_eq!(artifact_suffix(MACOS_ARM64), "macos-arm64");
        assert_eq!(artifact_suffix(MACOS_X64), "macos-x64");
        assert_eq!(artifact_extension(WINDOWS_X64), ".exe");
        assert_eq!(artifact_extension(MACOS_ARM64), "");
    }

    #[test]
    fn shell_quoting_keeps_single_quotes_intact() {
        assert_eq!(shell_quote("a'b"), "a'\\''b");
        assert_eq!(shell_quote("plain"), "plain");
    }
}
