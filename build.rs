use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");

    if let Some(head) = git_output(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head}");
    }
    if let Some(reference) = git_output(&["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git_output(&["rev-parse", "--git-path", &reference])
    {
        println!("cargo:rerun-if-changed={path}");
    }

    if let Some(sha) = std::env::var("GITHUB_SHA")
        .ok()
        .and_then(|s| short_sha(&s))
        .or_else(|| git_output(&["rev-parse", "--short=12", "HEAD"]))
    {
        println!("cargo:rustc-env=CRONK_GIT_SHA={sha}");
    }
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn short_sha(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.len() < 7 || !trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(trimmed[..trimmed.len().min(12)].to_owned())
}
