use std::{fs, path::Path, process::Command};

use cronk::{build_info, config::Config, demo, model::ItemKind, ui::Cronk};
use tui_lipan::TestBackend;

#[test]
fn version_flags_match_the_tui_build_version_without_loading_the_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("invalid.toml");
    std::fs::write(&config, "not valid TOML [").unwrap();

    for flag in ["--version", "-V"] {
        let output = Command::new(env!("CARGO_BIN_EXE_cronk"))
            .arg("--config")
            .arg(&config)
            .arg(flag)
            .env_remove("GITLAB_TOKEN")
            .output()
            .unwrap();

        assert!(output.status.success(), "{flag}: {output:?}");
        assert!(output.stderr.is_empty(), "{flag}: {output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("cronk {}\n", build_info::display_version()),
            "{flag} must use the same release tag and commit SHA as the TUI"
        );
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "not valid TOML ["
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

fn snapshot(config: &Path, output: &Path) {
    let result = Command::new(env!("CARGO_BIN_EXE_cronk"))
        .arg("--config")
        .arg(config)
        .arg("--demo")
        .arg("--snapshot")
        .arg(output)
        .env_remove("GITLAB_URL")
        .env_remove("GITLAB_TOKEN")
        .output()
        .unwrap();
    assert!(result.status.success(), "snapshot failed: {result:?}");
    assert!(result.stderr.is_empty(), "{result:?}");
    assert!(!fs::read(output).unwrap().is_empty());
}

fn demo_config() -> Config {
    Config {
        onboarding: false,
        projects: demo::projects(),
        ..Config::default()
    }
}

#[test]
fn demo_snapshot_with_missing_workspace_creates_only_the_requested_output() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("workspace.toml");
    let output = directory.path().join("preview.md");
    snapshot(&config, &output);
    assert!(!config.exists());
    assert!(!config.with_extension("demo.toml").exists());
    assert!(!config.with_extension("demo.state").exists());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn demo_snapshot_keeps_legacy_toml_unchanged_without_creating_navigation_storage() {
    let directory = tempfile::tempdir().unwrap();
    let requested = directory.path().join("workspace.toml");
    let path = requested.with_extension("demo.toml");
    let mut config = demo_config();
    config.active_tab = 3;
    config.route = Some(
        demo::items()
            .into_iter()
            .find(|i| i.key.kind == ItemKind::MergeRequest)
            .unwrap()
            .key,
    );
    let legacy = toml::to_string(&config).unwrap();
    fs::write(&path, &legacy).unwrap();
    snapshot(&requested, &directory.path().join("legacy-preview.md"));
    assert_eq!(fs::read_to_string(&path).unwrap(), legacy);
    assert!(!path.with_extension("state").exists());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn demo_snapshot_does_not_restore_or_change_existing_database_or_migrate_legacy_toml() {
    let directory = tempfile::tempdir().unwrap();
    let requested = directory.path().join("workspace.toml");
    let path = requested.with_extension("demo.toml");
    let mut config = demo_config();
    config.save(&path).unwrap();
    config.active_tab = 3;
    config.route = Some(
        demo::items()
            .into_iter()
            .find(|i| i.key.kind == ItemKind::MergeRequest)
            .unwrap()
            .key,
    );
    let backend = TestBackend::new(Cronk {
        config,
        path: Some(path.clone()),
        api: None,
        demo: true,
    });
    backend.state().flush_navigation().unwrap();
    drop(backend);
    let state_dir = path.with_extension("state");
    let databases = || {
        let mut entries: Vec<_> = fs::read_dir(&state_dir)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                (
                    path.file_name().unwrap().to_owned(),
                    fs::read(path).unwrap(),
                )
            })
            .collect();
        entries.sort();
        entries
    };
    let before = databases();
    assert_eq!(before.len(), 1);
    // Retained legacy keys alongside a committed DB model an interrupted cleanup.
    // A read-only preview must not retry migration or select the DB's saved MR.
    let mut legacy = demo_config();
    legacy.active_tab = 2;
    let item = demo::items()
        .into_iter()
        .find(|i| i.key.kind == ItemKind::Issue)
        .unwrap();
    let title = item.title.clone();
    legacy.route = Some(item.key);
    let source = toml::to_string(&legacy).unwrap();
    fs::write(&path, &source).unwrap();
    let output = directory.path().join("existing-preview.md");
    snapshot(&requested, &output);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(databases(), before);
    assert!(
        fs::read_to_string(output).unwrap().contains(&title),
        "preview uses TOML input, not machine-local DB navigation"
    );
}
