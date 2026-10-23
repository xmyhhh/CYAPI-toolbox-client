//! Read installed application metadata. Detection never launches the application.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Compatibility {
    pub supported: String,
    pub minimum_version: Option<String>,
    pub maximum_version_exclusive: Option<String>,
    #[serde(default)]
    pub tested_versions: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    pub detection: Option<Detection>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    #[serde(default)]
    windows_names: Vec<String>,
    #[serde(default)]
    windows_executables: Vec<Executable>,
    #[serde(default)]
    macos_bundles: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Executable {
    root: String,
    path: String,
}

fn version(value: &str) -> Option<Vec<u64>> {
    let numeric = value.trim().trim_start_matches(['v', 'V']);
    let numeric: String = numeric
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if numeric.is_empty() {
        return None;
    }
    let mut parts = numeric
        .trim_end_matches('.')
        .split('.')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    if parts.len() > 6 {
        return None;
    }
    parts.resize(6, 0);
    Some(parts)
}

pub fn validate(c: &Compatibility) -> bool {
    let safe_text = |s: &str| !s.is_empty() && s.len() <= 1000 && !s.chars().any(char::is_control);
    safe_text(&c.supported)
        && [&c.minimum_version, &c.maximum_version_exclusive]
            .iter()
            .all(|v| v.as_ref().is_none_or(|v| version(v).is_some()))
        && !matches!((c.minimum_version.as_deref().and_then(version), c.maximum_version_exclusive.as_deref().and_then(version)), (Some(min), Some(max)) if min >= max)
        && c.tested_versions.len() <= 16
        && c.tested_versions.iter().all(|v| version(v).is_some())
        && c.notes.len() <= 12
        && c.notes.iter().all(|s| safe_text(s))
        && c.detection.as_ref().is_none_or(|d| {
            d.windows_names.len() <= 8
                && d.windows_names.iter().all(|s| safe_text(s))
                && d.windows_executables.len() <= 8
                && d.windows_executables.iter().all(|e| {
                    ["programFiles", "programFilesX86", "localAppData"].contains(&e.root.as_str())
                        && e.path.len() <= 240
                        && !e.path.is_empty()
                        && !e.path.starts_with('/')
                        && e.path
                            .split('/')
                            .all(|part| !part.is_empty() && part != "." && part != "..")
                        && !e
                            .path
                            .chars()
                            .any(|c| c.is_control() || ['\\', ':', '[', ']', '?'].contains(&c))
                })
                && d.macos_bundles.len() <= 8
                && d.macos_bundles.iter().all(|s| {
                    safe_text(s) && s.ends_with(".app") && !s.contains(['/', '\\']) && s != "..app"
                })
        })
}

/// Distinguish a declared version range from versions actually exercised by the project.
pub fn assess(c: Option<&Compatibility>, detected: Option<&str>) -> Value {
    let Some(c) = c else {
        return json!({"state":"unknown","message":"目录尚未提供版本兼容信息"});
    };
    let Some(actual) = detected.and_then(version) else {
        return json!({"state":"unknown","message":"版本未确认，请按支持范围手动核对"});
    };
    if c.minimum_version
        .as_deref()
        .and_then(version)
        .is_some_and(|min| actual < min)
        || c.maximum_version_exclusive
            .as_deref()
            .and_then(version)
            .is_some_and(|max| actual >= max)
    {
        return json!({"state":"incompatible","message":format!("本机版本不在声明的支持范围：{}", c.supported)});
    }
    if c.tested_versions
        .iter()
        .filter_map(|v| version(v))
        .any(|tested| tested == actual)
    {
        json!({"state":"tested","message":"本机版本已有项目实测记录，仍需完成首次任务验证"})
    } else if c.minimum_version.is_none() && c.maximum_version_exclusive.is_none() {
        json!({"state":"unknown","message":"目录仅提供文字版支持范围，请手动核对本机版本"})
    } else {
        json!({"state":"untested","message":"符合声明的版本范围；本机版本尚无项目实测记录"})
    }
}

/// Probe all configured applications once per refresh, without application-specific adapters.
pub fn detect(descriptors: &[crate::integrations::Descriptor]) -> BTreeMap<String, Value> {
    let declarations: Vec<_> = descriptors
        .iter()
        .filter_map(|d| {
            Some(json!({"id":d.id,"detection":d.compatibility.as_ref()?.detection.as_ref()?}))
        })
        .collect();
    if declarations.is_empty() {
        return BTreeMap::new();
    }
    probe(&declarations).unwrap_or_default()
}

#[cfg(windows)]
fn probe(declarations: &[Value]) -> Option<BTreeMap<String, Value>> {
    windows_probe(declarations).map_err(|error| {
        crate::logging::event("warn", "software", error);
    }).ok()
}

#[cfg(windows)]
fn windows_probe(declarations: &[Value]) -> Result<BTreeMap<String, Value>, String> {
    use base64::Engine;
    use std::{
        io::Read,
        os::windows::process::CommandExt,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    // Only this fixed script runs. Catalog strings are JSON data supplied through an environment variable.
    let script = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$targets = ConvertFrom-Json $env:CYAPI_SOFTWARE_PROBES
$keys = @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*')
$apps = @(Get-ItemProperty -Path $keys -ErrorAction SilentlyContinue)
$results = @{}
foreach ($target in $targets) {
  $matches = @($apps | Where-Object {
    $name = $_.DisplayName
    $target.detection.windowsNames | Where-Object { $name -and ($name -eq $_ -or $name.StartsWith($_ + ' ', [StringComparison]::OrdinalIgnoreCase)) }
  } | Sort-Object @{Expression = { try { [version]($_.DisplayVersion -replace '[^0-9.].*$', '') } catch { [version]'0.0' } }} -Descending)
  $version = $null
  $source = $null
  foreach ($entry in $target.detection.windowsExecutables) {
    $base = switch ($entry.root) { 'programFiles' { $env:ProgramFiles } 'programFilesX86' { ${env:ProgramFiles(x86)} } 'localAppData' { $env:LOCALAPPDATA } }
    if (!$base) { continue }
    $files = @(Get-ChildItem -Path (Join-Path $base $entry.path) -File -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending)
    if ($files.Count) {
      $version = $files[0].VersionInfo.ProductVersion
      if (!$version) { $version = $files[0].VersionInfo.FileVersion }
      $source = $files[0].FullName
      break
    }
  }
  if (!$version -and $matches.Count) { $version = $matches[0].DisplayVersion; $source = '系统安装记录' }
  $results[$target.id] = @{ found = [bool]($source); version = $version; source = $source }
}
ConvertTo-Json -InputObject $results -Depth 5 -Compress
"#;
    let encoded = base64::engine::general_purpose::STANDARD.encode(
        script.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>(),
    );
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        .env(
            "CYAPI_SOFTWARE_PROBES",
            serde_json::to_string(declarations).map_err(|error| error.to_string())?,
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|error| format!("无法启动 Windows 元数据探测：{error}"))?;
    let output = child.stdout.take().ok_or("无法读取 Windows 元数据输出")?;
    let stderr = child.stderr.take().ok_or("无法读取 Windows 元数据错误")?;
    let errors = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.take(16_000).read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).to_string()
    });
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        output.take(256_001).read_to_end(&mut bytes).ok()?;
        (bytes.len() <= 256_000).then_some(bytes)
    });
    // A cold Windows PowerShell startup plus registry enumeration can exceed
    // five seconds on a busy machine. Keep the probe bounded and diagnose failures.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(status)) => {
                let _ = reader.join();
                return Err(format!("Windows 元数据探测失败（{status}）：{}", errors.join().unwrap_or_default()));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                let _ = errors.join();
                return Err(format!("无法等待 Windows 元数据探测：{error}"));
            }
            _ if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                let _ = errors.join();
                return Err("Windows 元数据探测超时（15 秒）".into());
            }
            _ => std::thread::sleep(Duration::from_millis(25)),
        }
    }
    let _ = errors.join();
    let bytes = reader.join().map_err(|_| "Windows 元数据读取线程失败")?
        .ok_or("Windows 元数据输出过大或无法读取")?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| format!("Windows 元数据编码无效：{error}"))?
        .trim_start_matches('\u{feff}');
    serde_json::from_str(text).map_err(|error| format!("Windows 元数据 JSON 无效：{error}"))
}

#[cfg(target_os = "macos")]
fn probe(declarations: &[Value]) -> Option<BTreeMap<String, Value>> {
    use std::{path::PathBuf, process::Command};
    let mut results = BTreeMap::new();
    let roots = [
        PathBuf::from("/Applications"),
        PathBuf::from(std::env::var_os("HOME")?).join("Applications"),
    ];
    for declaration in declarations {
        let id = declaration["id"].as_str()?;
        let bundles = declaration["detection"]["macosBundles"].as_array()?;
        let mut found = json!({"found":false});
        'bundle: for root in &roots {
            for name in bundles.iter().filter_map(Value::as_str) {
                let bundle = root.join(name);
                if !bundle.is_dir() {
                    continue;
                }
                let output = Command::new("/usr/bin/plutil")
                    .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
                    .arg(bundle.join("Contents/Info.plist"))
                    .output()
                    .ok()?;
                let version = output
                    .status
                    .success()
                    .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string());
                found = json!({"found":true,"version":version,"source":bundle});
                break 'bundle;
            }
        }
        results.insert(id.to_string(), found);
    }
    Some(results)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn probe(_: &[Value]) -> Option<BTreeMap<String, Value>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_ranges_do_not_claim_unexercised_versions_are_tested() {
        let c: Compatibility = serde_json::from_value(json!({"supported":"7–10","minimumVersion":"7","maximumVersionExclusive":"11","testedVersions":["10.0.6"],"notes":[]})).unwrap();
        assert_eq!(assess(Some(&c), Some("6.9"))["state"], "incompatible");
        assert_eq!(assess(Some(&c), Some("11.0"))["state"], "incompatible");
        assert_eq!(assess(Some(&c), Some("10.0.6.0"))["state"], "tested");
        assert_eq!(assess(Some(&c), Some("9.1"))["state"], "untested");
        assert_eq!(assess(Some(&c), Some("unknown"))["state"], "unknown");
        assert_eq!(assess(None, Some("9"))["state"], "unknown");
        let text_only: Compatibility =
            serde_json::from_value(json!({"supported":"见上游说明"})).unwrap();
        assert_eq!(assess(Some(&text_only), Some("9"))["state"], "unknown");
    }

    #[test]
    fn detector_metadata_rejects_traversal_and_contradictory_ranges() {
        let base = json!({"supported":"7+","minimumVersion":"7","detection":{
            "windowsExecutables":[{"root":"localAppData","path":"Example/*/app.exe"}],
            "macosBundles":["Example.app"]
        }});
        assert!(validate(&serde_json::from_value(base.clone()).unwrap()));
        for path in ["../app.exe", "C:/app.exe", "/app.exe", "Example\\app.exe"] {
            let mut invalid = base.clone();
            invalid["detection"]["windowsExecutables"][0]["path"] = json!(path);
            assert!(!validate(&serde_json::from_value(invalid).unwrap()));
        }
        let mut invalid = base;
        invalid["maximumVersionExclusive"] = json!("6");
        assert!(!validate(&serde_json::from_value(invalid).unwrap()));
    }

    #[cfg(windows)]
    #[test]
    fn windows_probe_reads_metadata_without_launching_the_executable() {
        let relative = format!(
            "CYAPI-toolbox/version-probe-{}/probe.exe",
            uuid::Uuid::new_v4()
        );
        let executable =
            std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join(&relative);
        let root = executable.parent().unwrap();
        std::fs::create_dir_all(&root).unwrap();
        // Copy a system executable with real PE version metadata. The detector
        // reads its resource table without running the copied program.
        let source = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        std::fs::copy(source, &executable).unwrap();
        let declarations = [
            json!({"id":"fixture","detection":{
                "windowsNames":[],"windowsExecutables":[{"root":"localAppData","path":relative}]
            }}),
            json!({"id":"absent","detection":{
                "windowsNames":["CYAPI Nonexistent Test Application"],"windowsExecutables":[]
            }}),
        ];
        // The fixed probe treats catalog values as paths, never as script text.
        let result =
            windows_probe(&declarations).expect("Windows metadata probe must return structured output");
        assert_eq!(result["fixture"]["found"], true);
        assert!(result["fixture"]["version"]
            .as_str()
            .is_some_and(|s| version(s).is_some()));
        assert_eq!(result["absent"]["found"], false);
        std::fs::remove_dir_all(root).unwrap();
    }
}
