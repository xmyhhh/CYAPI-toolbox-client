//! Optional command-line tools distributed by the catalog, shared by content hash.
use crate::{
    configuration::{no_links, stage},
    service::hash,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    pub version: String,
    pub platform: String,
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub source_url: String,
    pub license_text: String,
}
pub fn validate(tools: &[Tool]) -> Result<()> {
    // The same tool is published per platform, so the identity of a tool is
    // its name together with the platform it belongs to.
    let mut names = std::collections::HashSet::new();
    if tools.len() > 16 {
        return Err("外部工具声明过多".into());
    }
    for t in tools {
        if t.name.is_empty()
            || t.name.len() > 64
            || !t
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || !names.insert((t.name.clone(), t.platform.clone()))
            || !crate::platform::known(&t.platform)
            || t.version.is_empty()
            || t.version.len() > 80
            || t.sha256.len() != 64
            || !t
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || t.path
                != format!(
                    "/tools/{}/{}{}",
                    t.sha256,
                    t.name,
                    crate::platform::artifact_extension(&t.platform)
                )
            || t.size == 0
            || t.size > 200_000_000
            || t.license_text.is_empty()
            || t.license_text.len() > 100_000
            || !reqwest::Url::parse(&t.source_url).is_ok_and(|u| {
                u.scheme() == "https" && u.username().is_empty() && u.password().is_none()
            })
        {
            return Err("外部工具声明无效".into());
        }
    }
    Ok(())
}
fn path(data: &Path, t: &Tool) -> PathBuf {
    data.join(t.path.trim_start_matches('/'))
}
fn checked(t: &Tool, bytes: &[u8]) -> bool {
    bytes.len() as u64 == t.size
        && crate::platform::is_executable(bytes)
        && hash(bytes) == t.sha256
}
/// The tools that can run on this machine. The server may declare tools for
/// other platforms; those are neither downloaded nor required here.
pub fn applicable(tools: &[Tool]) -> Vec<&Tool> {
    tools
        .iter()
        .filter(|t| t.platform == crate::platform::current())
        .collect()
}

/// The tools that are declared for another platform and cannot run here.
pub fn unavailable(tools: &[Tool]) -> Vec<&Tool> {
    tools
        .iter()
        .filter(|t| t.platform != crate::platform::current())
        .collect()
}
pub fn cached(data: &Path, tools: &[Tool]) -> bool {
    validate(tools).is_ok()
        && applicable(tools).into_iter().all(|t| {
            no_links(&path(data, t)).is_ok()
                && fs::read(path(data, t)).is_ok_and(|b| checked(t, &b))
        })
}
pub fn download(
    data: &Path,
    tools: &[Tool],
    fetch: impl Fn(&str, u64) -> Result<Vec<u8>>,
) -> Result<()> {
    validate(tools)?;
    for t in applicable(tools) {
        let target = path(data, t);
        no_links(&target)?;
        if !fs::read(&target).is_ok_and(|b| checked(t, &b)) {
            crate::logging::event(
                "info",
                "tools",
                format!("下载共享工具：{} {}", t.name, t.version),
            );
            let bytes = fetch(&t.path, t.size)?;
            if !checked(t, &bytes) {
                return Err("外部工具 SHA256 或大小校验失败".into());
            }
            let pending = stage(&target, &bytes)?;
            fs::rename(pending, &target).map_err(|e| e.to_string())?;
        }
        let notice = target.with_file_name("THIRD-PARTY-NOTICES.txt");
        let pending = stage(
            &notice,
            format!(
                "Source: {}\nVersion: {}\n\n{}",
                t.source_url, t.version, t.license_text
            )
            .as_bytes(),
        )?;
        fs::rename(pending, notice).map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn launcher(data: &Path, tools: &[Tool]) -> Result<String> {
    let usable = applicable(tools);
    if usable.is_empty() {
        return Ok(String::new());
    }
    if !cached(data, tools) {
        return Err("外部工具尚未下载完整，请先准备离线文件".into());
    }
    if cfg!(windows) {
        let dirs = usable
            .iter()
            .map(|t| {
                format!(
                    "'{}'",
                    path(data, t)
                        .parent()
                        .unwrap()
                        .to_string_lossy()
                        .replace('\'', "''")
                )
            })
            .collect::<Vec<_>>();
        return Ok(format!(
            "$toolDirectories = @({})\n$env:Path = ($toolDirectories -join [IO.Path]::PathSeparator) + [IO.Path]::PathSeparator + $env:Path\n",
            dirs.join(",")
        ));
    }
    let dirs = usable
        .iter()
        .map(|t| {
            crate::platform::shell_quote(
                &path(data, t).parent().unwrap().to_string_lossy(),
            )
        })
        .collect::<Vec<_>>();
    Ok(format!("export PATH=\"{}:$PATH\"\n", dirs.join(":")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, platform: &str) -> Tool {
        let sha = crate::service::hash(name.as_bytes());
        Tool {
            name: name.into(),
            version: "1".into(),
            platform: platform.into(),
            path: format!(
                "/tools/{}/{}{}",
                sha,
                name,
                crate::platform::artifact_extension(platform)
            ),
            sha256: sha,
            size: 10,
            source_url: "https://example.com/tool".into(),
            license_text: "GPL".into(),
        }
    }

    #[test]
    fn the_same_tool_may_be_published_for_every_platform() {
        let tools = vec![
            tool("ffmpeg", crate::platform::WINDOWS_X64),
            tool("ffprobe", crate::platform::WINDOWS_X64),
            tool("ffmpeg", crate::platform::MACOS_ARM64),
            tool("ffprobe", crate::platform::MACOS_ARM64),
            tool("ffmpeg", crate::platform::MACOS_X64),
            tool("ffprobe", crate::platform::MACOS_X64),
            tool("ffmpeg", crate::platform::LINUX_X64),
            tool("ffprobe", crate::platform::LINUX_X64),
        ];
        assert!(validate(&tools).is_ok());
        let applicable = applicable(&tools);
        assert_eq!(applicable.len(), 2);
        assert!(applicable.iter().all(|tool| tool.platform == crate::platform::current()));
        assert_eq!(unavailable(&tools).len(), 6);
        // The same platform may not publish a tool twice.
        let mut duplicate = tools.clone();
        duplicate.push(tool("ffmpeg", crate::platform::MACOS_ARM64));
        assert!(validate(&duplicate).is_err());
    }
}
