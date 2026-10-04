pub const RELEASE_TAG: Option<&str> = option_env!("CRONK_RELEASE_TAG");
pub const GIT_SHA: Option<&str> = option_env!("CRONK_GIT_SHA");

pub fn display_version() -> String {
    format!("{}@{}", normalize_tag(RELEASE_TAG), normalize_sha(GIT_SHA))
}

fn normalize_tag(tag: Option<&str>) -> &str {
    tag.map(str::trim)
        .filter(|tag| !tag.is_empty())
        .unwrap_or("0.0.0")
}

fn normalize_sha(sha: Option<&str>) -> &str {
    sha.map(str::trim)
        .filter(|sha| !sha.is_empty())
        .unwrap_or("unknown")
}

#[cfg(test)]
mod tests {
    use super::{normalize_sha, normalize_tag};

    #[test]
    fn empty_or_missing_tag_uses_development_version() {
        assert_eq!(normalize_tag(None), "0.0.0");
        assert_eq!(normalize_tag(Some("")), "0.0.0");
        assert_eq!(normalize_tag(Some("  ")), "0.0.0");
    }

    #[test]
    fn explicit_release_tag_is_preserved() {
        assert_eq!(normalize_tag(Some("v0.1.0")), "v0.1.0");
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
