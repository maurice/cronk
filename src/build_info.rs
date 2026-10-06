pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GIT_SHA: Option<&str> = option_env!("CRONK_GIT_SHA");

pub fn display_version() -> String {
    format!("v{}@{}", PACKAGE_VERSION, normalize_sha(GIT_SHA))
}

fn normalize_sha(sha: Option<&str>) -> &str {
    sha.map(str::trim)
        .filter(|sha| !sha.is_empty())
        .unwrap_or("unknown")
}

#[cfg(test)]
mod tests {
    use super::normalize_sha;

    #[test]
    fn display_version_uses_cargo_package_version() {
        assert!(super::display_version().starts_with(&format!("v{}@", super::PACKAGE_VERSION)));
    }

    #[test]
    fn empty_or_missing_sha_uses_unknown() {
        assert_eq!(normalize_sha(None), "unknown");
        assert_eq!(normalize_sha(Some("")), "unknown");
        assert_eq!(normalize_sha(Some("  ")), "unknown");
    }

    #[test]
    fn explicit_sha_is_preserved() {
        assert_eq!(normalize_sha(Some("abc1234")), "abc1234");
    }
}
