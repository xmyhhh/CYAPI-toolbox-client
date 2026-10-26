use super::*;

fn fixture() -> (PathBuf, Service) {
    let root = env::temp_dir().join(format!("cyapi-skill-removal-{}", uuid::Uuid::new_v4()));
    let service = Service::new(root.join("data"), root.join("home")).unwrap();
    (root, service)
}

fn skill(path: &Path) {
    fs::create_dir_all(path.join("scripts")).unwrap();
    fs::write(
        path.join("SKILL.md"),
        "---\nname: local-test\n---\n# Local Skill",
    )
    .unwrap();
    fs::write(path.join("scripts/user.txt"), "keep user files").unwrap();
}

fn entry(service: &Service, path: &Path) -> Installation {
    service
        .installations()
        .into_iter()
        .find(|entry| Path::new(&entry.record.path) == path)
        .unwrap()
}

fn request(entry: &Installation) -> Request {
    serde_json::from_value(json!({
        "action": "remove-local",
        "recordId": entry.record.id,
        "installationPath": entry.record.path,
    }))
    .unwrap()
}

#[test]
fn local_skills_move_to_unique_backups_without_deleting_user_files() {
    let (root, mut service) = fixture();
    for (target, path) in [
        ("codex", service.home.join(".codex/skills/local-test")),
        ("claude", service.home.join(".claude/skills/local-test")),
        (
            "opencode",
            service.opencode_home().join("skills/local-test"),
        ),
    ] {
        skill(&path);
        let local = entry(&service, &path);
        assert_eq!(local.record.target, target);
        assert!(!local.managed);
        assert!(local.removable);
        // Existing managed uninstall still refuses unregistered local content.
        assert!(service.remove(&local.record.id).is_err());
        let result = service.dispatch(request(&local)).unwrap();
        let backup = PathBuf::from(result["backupPath"].as_str().unwrap());
        assert!(!path.exists());
        assert_eq!(
            fs::read_to_string(backup.join("scripts/user.txt")).unwrap(),
            "keep user files"
        );
        assert!(backup.join("SKILL.md").is_file());
        assert!(service.records.is_empty());
        assert!(!service.data.join("pending.json").exists());

        skill(&path);
        fs::write(path.join("scripts/user.txt"), "second version").unwrap();
        let second = service.dispatch(request(&entry(&service, &path))).unwrap();
        let second_backup = Path::new(second["backupPath"].as_str().unwrap());
        assert_ne!(second_backup, backup);
        assert_eq!(
            fs::read_to_string(backup.join("scripts/user.txt")).unwrap(),
            "keep user files"
        );
        assert_eq!(
            fs::read_to_string(second_backup.join("scripts/user.txt")).unwrap(),
            "second version"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_removal_rejects_unknown_stale_and_unconfirmed_paths() {
    let (root, mut service) = fixture();
    let path = service.home.join(".codex/skills/local-test");
    skill(&path);
    let local = entry(&service, &path);
    assert!(service.remove_local(&local.record.id, "").is_err());
    assert!(service
        .remove_local("local-unknown", &local.record.path)
        .is_err());
    let outside = root.join("outside");
    skill(&outside);
    assert!(service
        .remove_local(&local.record.id, outside.to_str().unwrap())
        .is_err());
    let no_path =
        serde_json::from_value(json!({"action":"remove-local","recordId":local.record.id}))
            .unwrap();
    assert!(service.dispatch(no_path).is_err());
    assert!(path.join("SKILL.md").is_file());
    assert!(outside.join("SKILL.md").is_file());
    fs::remove_file(path.join("SKILL.md")).unwrap();
    assert!(service.dispatch(request(&local)).is_err());
    assert!(path.join("scripts/user.txt").is_file());
    assert_eq!(
        fs::read_dir(service.data.join("backups")).unwrap().count(),
        0
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn system_skills_are_protected_even_with_a_toolbox_marker() {
    let (root, mut service) = fixture();
    let path = service.home.join(".codex/skills/.system/local-test");
    skill(&path);
    let system = entry(&service, &path);
    assert!(!system.removable);
    assert!(system
        .removal_blocked_reason
        .as_deref()
        .unwrap()
        .contains("系统内置"));
    assert!(service.dispatch(request(&system)).is_err());

    let mut record = system.record;
    record.id = "toolbox-system-test".into();
    atomic_json(&path.join(".cyapi-install.json"), &record).unwrap();
    service.records.push(record.clone());
    assert!(entry(&service, &path).managed);
    assert!(service.remove(&record.id).is_err());
    assert!(path.join("SKILL.md").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_management_marker_requires_explicit_local_removal() {
    let (root, mut service) = fixture();
    let path = service.home.join(".codex/skills/local-test");
    skill(&path);
    let mut record = entry(&service, &path).record;
    record.id = "toolbox-test".into();
    atomic_json(&path.join(".cyapi-install.json"), &record).unwrap();
    service.records.push(record.clone());
    atomic_json(&service.data.join("installed.json"), &service.records).unwrap();
    assert!(service.dispatch(request(&entry(&service, &path))).is_err());
    fs::remove_file(path.join(".cyapi-install.json")).unwrap();
    assert!(service.remove(&record.id).is_err());
    let local = entry(&service, &path);
    assert!(!local.managed);
    assert!(local.removable);
    service.dispatch(request(&local)).unwrap();
    assert!(service.records.is_empty());
    let saved: Vec<Record> = read_json(&service.data.join("installed.json")).unwrap();
    assert!(saved.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn managed_uninstall_keeps_working_for_a_previously_configured_root() {
    let (root, mut service) = fixture();
    let path = root.join("previous-agent-home/skills/local-test");
    skill(&path);
    let mut record = crate::installed::scan(path.parent().unwrap(), "codex", "global")
        .pop()
        .unwrap();
    record.id = "toolbox-previous-root".into();
    atomic_json(&path.join(".cyapi-install.json"), &record).unwrap();
    service.records.push(record.clone());
    assert!(entry(&service, &path).removable);
    let backup = service.remove(&record.id).unwrap();
    assert!(!path.exists());
    assert!(Path::new(&backup).join("scripts/user.txt").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_removal_rolls_back_when_registry_cannot_be_saved() {
    let (root, mut service) = fixture();
    let path = service.home.join(".codex/skills/local-test");
    skill(&path);
    let local = entry(&service, &path);
    // A directory at the registry path makes the final atomic replace fail.
    fs::create_dir(service.data.join("installed.json")).unwrap();
    assert!(service.dispatch(request(&local)).is_err());
    assert!(path.join("SKILL.md").is_file());
    assert!(path.join("scripts/user.txt").is_file());
    assert!(!service.data.join("pending.json").exists());
    assert_eq!(
        fs::read_dir(service.data.join("backups")).unwrap().count(),
        0
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn interrupted_local_removal_recovers_without_a_management_marker() {
    let (root, mut service) = fixture();
    let path = service.home.join(".codex/skills/local-test");
    skill(&path);
    let record = entry(&service, &path).record;
    // Also cover a toolbox record whose management marker was lost.
    service.records.push(record.clone());
    atomic_json(&service.data.join("installed.json"), &service.records).unwrap();
    let backup_name = "local-interrupted";
    let backup = service.data.join("backups").join(backup_name);
    atomic_json(
        &service.data.join("pending.json"),
        &json!({
            "kind":"remove-local", "record":record, "backupName":backup_name
        }),
    )
    .unwrap();
    fs::rename(&path, &backup).unwrap();
    let recovered = Service::new(service.data.clone(), service.home.clone()).unwrap();
    assert!(recovered.records.is_empty());
    assert!(backup.join("SKILL.md").is_file());
    assert!(!recovered.data.join("pending.json").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_removal_failure_preserves_files_and_recovery_does_not_drop_existing_records() {
    let (root, mut service) = fixture();
    let path = service.home.join(".codex/skills/local-test");
    skill(&path);
    let record = entry(&service, &path).record;
    service.records.push(record.clone());
    atomic_json(&service.data.join("installed.json"), &service.records).unwrap();
    let backup_name = "local-existing";
    let backup = service.data.join("backups").join(backup_name);
    skill(&backup);
    assert!(service
        .backup_installation(&record, &backup, "remove-local")
        .is_err());
    assert!(path.join("SKILL.md").is_file());
    atomic_json(
        &service.data.join("pending.json"),
        &json!({
            "kind":"remove-local", "record":record, "backupName":backup_name
        }),
    )
    .unwrap();
    let recovered = Service::new(service.data.clone(), service.home.clone()).unwrap();
    assert_eq!(recovered.records.len(), 1);
    assert!(path.join("SKILL.md").is_file());
    assert!(backup.join("SKILL.md").is_file());
    fs::remove_dir_all(root).unwrap();
}

// Junctions are available without Windows developer mode/admin symlink privileges.
#[cfg(windows)]
fn link(source: &Path, path: &Path) {
    let output = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(path.to_string_lossy().replace('/', "\\"))
        .arg(source.to_string_lossy().replace('/', "\\"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
fn link(source: &Path, path: &Path) {
    std::os::unix::fs::symlink(source, path).unwrap();
}

#[cfg(any(windows, unix))]
#[test]
fn linked_skills_move_only_the_link_and_system_aliases_stay_protected() {
    let (root, mut service) = fixture();
    let source = root.join("external/local-test");
    skill(&source);
    let path = service.home.join(".codex/skills/linked");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    link(&source, &path);
    let result = service.dispatch(request(&entry(&service, &path))).unwrap();
    let backup = Path::new(result["backupPath"].as_str().unwrap());
    assert!(!path.exists());
    assert!(source.join("scripts/user.txt").is_file());
    assert_eq!(
        fs::canonicalize(backup).unwrap(),
        fs::canonicalize(&source).unwrap()
    );
    // Remove only the test link before cleaning up the fixture.
    #[cfg(windows)]
    fs::remove_dir(backup).unwrap();
    #[cfg(unix)]
    fs::remove_file(backup).unwrap();

    let system = service.home.join(".codex/skills/.system/local-test");
    skill(&system);
    link(&system, &path);
    assert!(!entry(&service, &path).removable);
    assert!(service.dispatch(request(&entry(&service, &path))).is_err());
    assert!(system.join("SKILL.md").is_file());
    #[cfg(windows)]
    fs::remove_dir(&path).unwrap();
    #[cfg(unix)]
    fs::remove_file(&path).unwrap();
    fs::remove_dir_all(root).unwrap();
}
