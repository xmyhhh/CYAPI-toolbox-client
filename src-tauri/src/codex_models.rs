//! Immutable Codex catalogs: the config/credential backup also restores the catalog
//! reference, while previous catalog files remain available to that backup.
use crate::configuration::{no_links, stage};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
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

/// Per-model effective values of the published catalog, keyed in selection order.
/// Absent optional fields read back as `None`, so the UI can distinguish "keep
/// the catalog's own metadata" from "declared and empty".
pub fn published(root: &Path, doc: &DocumentMut) -> Result<Vec<Published>> {
    let ids = selected(root, doc)?;
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let path = configured_path(root, doc).ok_or("Codex 模型目录引用无效")?;
    if path.parent() != Some(root.join("cyapi-models").as_path()) {
        return Ok(vec![]);
    }
    let entries = catalog(&path)?["models"]
        .as_array()
        .ok_or("Codex 模型目录缺少 models")?
        .clone();
    Ok(ids
        .iter()
        .map(|id| {
            let model = entries
                .iter()
                .find(|model| model.get("slug").and_then(Value::as_str) == Some(id.as_str()));
            Published {
                model: id.clone(),
                context_window: model
                    .and_then(|model| model.get("context_window"))
                    .and_then(Value::as_u64),
                reasoning_levels: model
                    .and_then(|model| model.get("supported_reasoning_levels"))
                    .and_then(Value::as_array)
                    .map(|levels| {
                        levels
                            .iter()
                            .filter_map(|level| {
                                level
                                    .get("effort")
                                    .and_then(Value::as_str)
                                    .or_else(|| level.as_str())
                                    .map(str::to_string)
                            })
                            .collect()
                    }),
                default_level: model
                    .and_then(|model| model.get("default_reasoning_level"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }
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

/// The shared input model validation cap goes elsewhere; this module only owns
/// catalog generation and reading.
const GENERIC_LEVEL_DESCRIPTION: &str = "Declared manually in the toolbox.";

/// Per-model values the user declares for the published catalog. A `None`
/// field keeps the catalog's own metadata: the upstream catalog for known
/// models, and the empty fallback for unknown model IDs.
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setting {
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub reasoning_levels: Option<Vec<String>>,
    #[serde(default)]
    pub default_level: Option<String>,
}

impl Setting {
    pub(crate) fn declared(&self) -> bool {
        self.context_window.is_some()
            || self.reasoning_levels.is_some()
            || self.default_level.is_some()
    }
}

/// One configured model plus its optional user declaration.
pub struct Choice {
    pub model: String,
    pub setting: Option<Setting>,
}

/// Effective per-model values as published in our catalog, for the settings UI.
pub struct Published {
    pub model: String,
    pub context_window: Option<u64>,
    pub reasoning_levels: Option<Vec<String>>,
    pub default_level: Option<String>,
}

fn apply_setting(entry: &mut Value, setting: &Setting, descriptions: &HashMap<String, String>) {
    let Some(object) = entry.as_object_mut() else {
        return;
    };
    match setting.context_window {
        Some(tokens) => {
            object.insert("context_window".into(), json!(tokens));
        }
        None => {
            object.remove("context_window");
        }
    }
    if let Some(levels) = &setting.reasoning_levels {
        object.insert(
            "supported_reasoning_levels".into(),
            json!(levels
                .iter()
                .map(|effort| json!({
                    "effort": effort,
                    "description": descriptions
                        .get(effort)
                        .map(String::as_str)
                        .unwrap_or(GENERIC_LEVEL_DESCRIPTION),
                }))
                .collect::<Vec<_>>()),
        );
    }
    if let Some(level) = &setting.default_level {
        object.insert("default_reasoning_level".into(), json!(level));
    }
}

pub fn write(root: &Path, data: &Path, doc: &DocumentMut, choices: &[Choice]) -> Result<PathBuf> {
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
    // Reuse official descriptions for the standard efforts so a declared list
    // stays byte-identical with the preserved upstream one when unchanged.
    let mut descriptions: HashMap<String, String> = HashMap::new();
    for source in &models {
        let Some(levels) = source
            .get("supported_reasoning_levels")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for level in levels {
            let Some(effort) = level
                .get("effort")
                .and_then(Value::as_str)
                .or_else(|| level.as_str())
            else {
                continue;
            };
            if let Some(description) = level
                .get("description")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                descriptions
                    .entry(effort.to_string())
                    .or_insert_with(|| description.to_string());
            }
        }
    }
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
    let entries: Vec<Value> = choices.iter().enumerate().map(|(priority, choice)| {
        let id = choice.model.as_str();
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
        // for unknown IDs, advertise no levels and invent no default. User
        // declarations replace those fields for that model only.
        if entry.get("supported_reasoning_levels").and_then(Value::as_array).is_none() {
            entry["supported_reasoning_levels"] = json!([]);
        }
        if let Some(setting) = &choice.setting {
            apply_setting(&mut entry, setting, &descriptions);
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
        let choices = ids
            .into_iter()
            .map(|model| Choice {
                model,
                setting: None,
            })
            .collect::<Vec<_>>();
        let mut doc = DocumentMut::new();
        doc["model_reasoning_effort"] = toml_edit::value("high");
        let path = write(&root, &data, &doc, &choices).unwrap();
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
        let cleared = write(&root, &data, &doc, &choices).unwrap();
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
            &[Choice {
                model: "known".into(),
                setting: None,
            }],
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
    fn user_declarations_replace_catalog_fields_and_read_back() {
        let root =
            std::env::temp_dir().join(format!("cyapi-codex-declare-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let data = root.join("data");
        let known = json!({
            "slug":"known", "base_instructions":"Codex instructions",
            "context_window": 272000,
            "default_reasoning_level":"medium",
            "supported_reasoning_levels":[
                {"effort":"low","description":"Low effort"},
                {"effort":"medium","description":"Balanced"}
            ]
        });
        fs::write(
            root.join("models_cache.json"),
            serde_json::to_vec(&json!({"models":[known]})).unwrap(),
        )
        .unwrap();
        let mut doc = DocumentMut::new();
        // Known model: declared values replace every field that is set; the custom
        // effort gets the generic description while "low" keeps the official one.
        let known_setting = Setting {
            context_window: Some(555),
            reasoning_levels: Some(vec!["low".into(), "provider-special".into()]),
            default_level: Some("low".into()),
        };
        let path = write(
            &root,
            &data,
            &doc,
            &[
                Choice {
                    model: "known".into(),
                    setting: Some(known_setting),
                },
                Choice {
                    model: "unknown".into(),
                    // Partial declaration: only the context window is user-owned,
                    // the unknown-model fallback still advertises an empty list.
                    setting: Some(Setting {
                        context_window: Some(1000),
                        reasoning_levels: None,
                        default_level: None,
                    }),
                },
            ],
        )
        .unwrap();
        // The doc points at the published catalog the way save() does afterwards,
        // so published() can read the entries back the same way the view does.
        doc["model_catalog_json"] = toml_edit::value(path.to_string_lossy().as_ref());
        let published = catalog(&path).unwrap();
        assert_eq!(published["models"][0]["context_window"], json!(555));
        assert_eq!(
            published["models"][0]["supported_reasoning_levels"],
            json!([
                {"effort":"low","description":"Low effort"},
                {"effort":"provider-special","description":GENERIC_LEVEL_DESCRIPTION}
            ])
        );
        assert_eq!(published["models"][0]["default_reasoning_level"], "low");
        assert_eq!(published["models"][1]["context_window"], json!(1000));
        assert_eq!(
            published["models"][1]["supported_reasoning_levels"],
            json!([])
        );

        // Read-back: the view uses these values directly.
        let state = crate::codex_models::published(&root, &doc).unwrap();
        assert_eq!(state.len(), 2);
        assert_eq!(state[0].model, "known");
        assert_eq!(state[0].context_window, Some(555));
        assert_eq!(
            state[0].reasoning_levels,
            Some(vec!["low".to_string(), "provider-special".to_string()])
        );
        assert_eq!(state[0].default_level, Some("low".to_string()));
        assert_eq!(state[1].model, "unknown");
        assert_eq!(state[1].context_window, Some(1000));
        assert_eq!(state[1].reasoning_levels, Some(vec![]));
        assert_eq!(state[1].default_level, None);

        // Unchanged declarations must keep the immutable hash identical.
        let same = write(
            &root,
            &data,
            &doc,
            &[
                Choice {
                    model: "known".into(),
                    setting: Some(Setting {
                        context_window: Some(555),
                        reasoning_levels: Some(vec!["low".into(), "provider-special".into()]),
                        default_level: Some("low".into()),
                    }),
                },
                Choice {
                    model: "unknown".into(),
                    setting: Some(Setting {
                        context_window: Some(1000),
                        reasoning_levels: None,
                        default_level: None,
                    }),
                },
            ],
        )
        .unwrap();
        assert_eq!(same, path);
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
        let choices = ids
            .iter()
            .map(|model| Choice {
                model: model.clone(),
                setting: None,
            })
            .collect::<Vec<_>>();
        let path = write(&root, &data, &DocumentMut::new(), &choices).unwrap();
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
