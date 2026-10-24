//! Immutable Codex catalogs: the config/credential backup also restores the catalog
//! reference, while previous catalog files remain available to that backup.
use crate::configuration::{no_links, stage};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use toml_edit::{DocumentMut, Item};

type Result<T> = std::result::Result<T, String>;
const MAX_BYTES: u64 = 16_000_000;

fn catalog(path: &Path) -> Result<Value> {
    no_links(path)?;
    let meta = fs::metadata(path).map_err(|_| "无法读取 Codex 模型目录")?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return Err("Codex 模型目录过大或不是普通文件".into());
    }
    let bytes = fs::read(path).map_err(|_| "无法读取 Codex 模型目录")?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "Codex 模型目录格式无效")?;
    if value.get("models").and_then(Value::as_array).is_none() {
        return Err("Codex 模型目录缺少 models".into());
    }
    Ok(value)
}

fn configured_path(root: &Path, doc: &DocumentMut) -> Option<PathBuf> {
    doc.get("model_catalog_json")
        .and_then(Item::as_str)
        .map(|value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        })
}

pub fn selected(root: &Path, doc: &DocumentMut) -> Result<Vec<String>> {
    let Some(path) = configured_path(root, doc) else {
        return Ok(vec![]);
    };
    // Only our catalogs represent a user's explicit selection. Other catalogs can
    // contain an entire provider's discovery results and are left as a source.
    if path.parent() != Some(root.join("cyapi-models").as_path()) {
        return Ok(vec![]);
    }
    Ok(catalog(&path)?["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|model| {
            model
                .get("slug")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

fn bundled(data: &Path) -> Option<Value> {
    let directory = data.join("codex-config/catalog-runtime");
    no_links(&directory).ok()?;
    fs::create_dir_all(&directory).ok()?;
    let output_path = directory.join(format!("catalog-{}.json", uuid::Uuid::new_v4()));
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .ok()?;
    let mut command = Command::new("codex");
    command
        .args(["debug", "models", "--bundled"])
        .env("CODEX_HOME", &directory)
        .stdout(Stdio::from(file))
        .stderr(Stdio::null())
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let result = (|| {
        let mut child = command.spawn().ok()?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return status
                        .success()
                        .then(|| catalog(&output_path).ok())
                        .flatten()
                }
                Ok(None) if started.elapsed() < Duration::from_secs(5) => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
    })();
    let _ = fs::remove_file(output_path);
    result
}

pub fn write(root: &Path, data: &Path, doc: &DocumentMut, ids: &[String]) -> Result<PathBuf> {
    let mut sources = Vec::new();
    if let Some(source) = configured_path(root, doc).and_then(|path| catalog(&path).ok()) {
        sources.push(source);
    }
    if let Ok(source) = catalog(&root.join("models_cache.json")) {
        sources.push(source);
    }
    if !sources.iter().any(|source| {
        source["models"].as_array().unwrap().iter().any(|model| {
            model
                .get("base_instructions")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.is_empty())
        })
    }) {
        sources.push(bundled(data).ok_or("无法读取 Codex 模型元数据。请先启动一次新版 Codex，或安装支持 debug models --bundled 的 Codex CLI 后重试。")?);
    }
    let models: Vec<&Value> = sources
        .iter()
        .flat_map(|source| source["models"].as_array().unwrap())
        .collect();
    // Keep Codex's own instructions. A /models response only contains IDs and must
    // never replace the coding agent's instructions with an empty placeholder.
    let template = models
        .iter()
        .copied()
        .find(|model| {
            model
                .get("base_instructions")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.is_empty())
        })
        .ok_or("Codex 模型元数据缺少代理指令，请先启动新版 Codex 更新模型目录")?;
    let entries: Vec<Value> = ids.iter().enumerate().map(|(priority, id)| {
        let mut entry = models.iter().copied().find(|model| model.get("slug").and_then(Value::as_str) == Some(id))
            .cloned().unwrap_or_else(|| json!({
                "slug": id, "display_name": id, "description": "API 自定义模型",
                "base_instructions": template["base_instructions"],
                "shell_type": "unified_exec", "default_reasoning_summary": "none",
                "support_verbosity": false, "truncation_policy": {"mode":"tokens", "limit":10000},
                "experimental_supported_tools": [], "input_modalities":["text"]
            }));
        entry["visibility"] = json!("list");
        entry["supported_in_api"] = json!(true);
        entry["priority"] = json!(priority);
        // Capabilities belong to the model, not the global model_reasoning_effort.
        // Codex ModelInfo requires a supported-levels list (which may be empty),
        // but default_reasoning_level is optional. Preserve advertised metadata;
        // for unknown IDs, advertise no levels and invent no default.
        if entry.get("supported_reasoning_levels").and_then(Value::as_array).is_none() {
            entry["supported_reasoning_levels"] = json!([]);
        }
        entry
    }).collect();
    let bytes = serde_json::to_vec_pretty(&json!({"models":entries}))
        .map_err(|_| "无法生成 Codex 模型目录")?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Codex 模型目录过大，请减少所选模型".into());
    }
    let path = root
        .join("cyapi-models")
        .join(format!("{}.json", crate::service::hash(&bytes)));
    no_links(&path)?;
    if fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
        let temp = stage(&path, &bytes)?;
        fs::rename(&temp, &path).map_err(|_| {
            let _ = fs::remove_file(temp);
            "无法保存 Codex 模型目录".to_string()
        })?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_effort_never_changes_advertised_levels_or_defaults() {
        let root =
            std::env::temp_dir().join(format!("cyapi-codex-levels-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let data = root.join("data");
        let known = json!({
            "slug":"known", "base_instructions":"Codex instructions",
            "default_reasoning_level":"low",
            "supported_reasoning_levels":[{"effort":"low","description":"Low effort"}]
        });
        fs::write(
            root.join("models_cache.json"),
            serde_json::to_vec(&json!({"models":[known]})).unwrap(),
        )
        .unwrap();
        let ids = vec!["known".into(), "unknown".into()];
        let mut doc = DocumentMut::new();
        doc["model_reasoning_effort"] = toml_edit::value("high");
        let path = write(&root, &data, &doc, &ids).unwrap();
        let published = catalog(&path).unwrap();
        assert_eq!(
            published["models"][0]["supported_reasoning_levels"],
            known["supported_reasoning_levels"]
        );
        assert_eq!(published["models"][0]["default_reasoning_level"], "low");
        assert_eq!(
            published["models"][1]["supported_reasoning_levels"],
            json!([])
        );
        assert!(published["models"][1]
            .get("default_reasoning_level")
            .is_none());

        // Re-read our published catalog when clearing the global setting: metadata
        // and the immutable catalog hash must stay identical.
        doc["model_catalog_json"] = toml_edit::value(path.to_string_lossy().as_ref());
        doc.remove("model_reasoning_effort");
        let cleared = write(&root, &data, &doc, &ids).unwrap();
        assert_eq!(cleared, path);
        assert_eq!(catalog(&cleared).unwrap(), published);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn known_models_preserve_custom_levels_and_an_absent_default() {
        let root =
            std::env::temp_dir().join(format!("cyapi-codex-custom-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let known = json!({
            "slug":"known", "base_instructions":"Codex instructions",
            "supported_reasoning_levels":[{"effort":"provider-special","description":"Custom effort"}]
        });
        fs::write(
            root.join("models_cache.json"),
            serde_json::to_vec(&json!({"models":[known]})).unwrap(),
        )
        .unwrap();
        let path = write(
            &root,
            &root.join("data"),
            &DocumentMut::new(),
            &["known".into()],
        )
        .unwrap();
        let published = catalog(&path).unwrap();
        assert_eq!(
            published["models"][0]["supported_reasoning_levels"],
            known["supported_reasoning_levels"]
        );
        assert!(published["models"][0]
            .get("default_reasoning_level")
            .is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "manual offline validation against the installed Codex CLI"]
    fn installed_codex_reads_the_generated_multi_model_catalog() {
        let root =
            std::env::temp_dir().join(format!("cyapi-codex-catalog-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let data = root.join("toolbox-data");
        let ids = vec!["cyapi-test-a".to_string(), "cyapi-test-b".to_string()];
        let path = write(&root, &data, &DocumentMut::new(), &ids).unwrap();
        let mut doc = DocumentMut::new();
        doc["model_catalog_json"] = toml_edit::value(path.to_string_lossy().as_ref());
        doc["model"] = toml_edit::value(&ids[0]);
        fs::write(root.join("config.toml"), doc.to_string()).unwrap();
        let result = Command::new("codex")
            .args(["debug", "models"])
            .env("CODEX_HOME", &root)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(
            output["models"]
                .as_array()
                .unwrap()
                .iter()
                .map(|model| model["slug"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ids
        );
        assert!(output["models"][0]["base_instructions"]
            .as_str()
            .is_some_and(|text| !text.is_empty()));
        for model in output["models"].as_array().unwrap() {
            assert_eq!(model["supported_reasoning_levels"], json!([]));
        }
        fs::remove_dir_all(root).unwrap();
    }
}
