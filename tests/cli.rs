use std::process::Command;

use cronk::build_info;

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
