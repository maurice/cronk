use std::process::Command;

#[test]
fn cross_forwards_source_commit_into_build_containers() {
    let config: toml::Value = toml::from_str(include_str!("../Cross.toml")).unwrap();
    let passthrough = config["build"]["env"]["passthrough"].as_array().unwrap();
    assert!(
        passthrough
            .iter()
            .any(|entry| entry.as_str() == Some("GITHUB_SHA")),
        "cross must forward GITHUB_SHA so release binaries include their source commit"
    );
    assert!(
        !passthrough
            .iter()
            .any(|entry| entry.as_str() == Some("CRONK_RELEASE_TAG")),
        "the package version comes from Cargo.toml, not a separate release-tag override"
    );
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
        .env("GITHUB_SHA", "0123456789abcdef0123456789abcdef01234567")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout
            .lines()
            .any(|line| line == "cargo:rustc-env=CRONK_GIT_SHA=0123456789ab"),
        "{stdout}"
    );
}
