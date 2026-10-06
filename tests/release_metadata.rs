use std::process::Command;

#[test]
fn cross_forwards_release_tag_and_source_commit_into_build_containers() {
    let config: toml::Value = toml::from_str(include_str!("../Cross.toml")).unwrap();
    let passthrough = config["build"]["env"]["passthrough"].as_array().unwrap();
    for variable in ["CRONK_RELEASE_TAG", "GITHUB_SHA"] {
        assert!(
            passthrough
                .iter()
                .any(|entry| entry.as_str() == Some(variable)),
            "cross must forward {variable}; release tags are created after the build"
        );
    }
}

#[test]
fn build_script_embeds_explicit_release_metadata_without_git() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory
        .path()
        .join(format!("build-script{}", std::env::consts::EXE_SUFFIX));
    let compiler = Command::new("rustc")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/build.rs"))
        .arg("--edition=2024")
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(compiler.status.success(), "{compiler:?}");

    let output = Command::new(executable)
        .current_dir(directory.path())
        .env("PATH", "") // No git executable or repository is needed in the container.
        .env("CRONK_RELEASE_TAG", "v9.8.7")
        .env("GITHUB_SHA", "0123456789abcdef0123456789abcdef01234567")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout
            .lines()
            .any(|line| line == "cargo:rustc-env=CRONK_RELEASE_TAG=v9.8.7"),
        "{stdout}"
    );
    assert!(
        stdout
            .lines()
            .any(|line| line == "cargo:rustc-env=CRONK_GIT_SHA=0123456789ab"),
        "{stdout}"
    );
}
