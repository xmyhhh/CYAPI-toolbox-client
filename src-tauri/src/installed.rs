//! Read-only discovery of Skills installed outside this toolbox.
use crate::models::{Record, Skill};
use std::{fs, io::Read, path::Path};

pub fn manifest_name(markdown: &str) -> Option<String> {
    if markdown.len() > 262_144 {
        return None;
    }
    let mut lines = markdown.trim_start_matches('\u{feff}').lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut header = String::new();
    for line in lines {
        if line.trim() == "---" {
            let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&header).ok()?;
            return value
                .get("name")?
                .as_str()
                .map(str::trim)
                .filter(|name| {
                    !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
                })
                .map(str::to_owned);
        }
        header.push_str(line);
        header.push('\n');
    }
    None
}

pub fn path_key(path: &Path) -> String {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

pub fn matches(record: &Record, skill: &Skill) -> bool {
    let folder = Path::new(&record.path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let same = |name: &str| {
        name.eq_ignore_ascii_case(&record.skill_id) || name.eq_ignore_ascii_case(folder)
    };
    same(&skill.id) || manifest_name(&skill.markdown).is_some_and(|name| same(&name))
}

pub fn scan(root: &Path, target: &str, scope: &str) -> Vec<Record> {
    let Ok(entries) = fs::read_dir(root) else {
        return vec![];
    };
    let mut records = Vec::new();
    // Scan one level only, including a Skill directory linked from another location.
    for entry in entries.flatten() {
        let path = entry.path();
        let folder = entry.file_name().to_string_lossy().into_owned();
        if folder.starts_with('.') || !path.is_dir() || !path.join("SKILL.md").is_file() {
            continue;
        }
        let mut bytes = Vec::new();
        if let Ok(file) = fs::File::open(path.join("SKILL.md")) {
            let _ = file.take(262_145).read_to_end(&mut bytes);
        }
        let name = std::str::from_utf8(&bytes)
            .ok()
            .and_then(manifest_name)
            .unwrap_or(folder);
        records.push(Record {
            id: format!("local-{}", crate::service::hash(path_key(&path).as_bytes())),
            skill_id: name.clone(),
            title: name,
            version: String::new(),
            revision: String::new(),
            target: target.into(),
            scope: scope.into(),
            path: path.to_string_lossy().into_owned(),
            installed_at: 0,
            runtimes: vec![],
            package_sha256: String::new(),
            configuration: None,
        });
    }
    records.sort_by(|a, b| a.path.cmp(&b.path));
    records
}
