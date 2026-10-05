//! Validated TOML configuration and workspace state. Call `save` after each
//! committed UI event; there is deliberately no debounce or implicit save on load.
//! Tokens are resolved by the caller from `token_env`, never by this module.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use directories::ProjectDirs;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::filter::Query;
use crate::model::{ItemKey, ItemKind, LookupOption, Project, User};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub gitlab_url: String,
    pub token_env: String,
    pub projects: Vec<Project>,
    pub views: Vec<SavedView>,
    pub theme: String,
    /// Disable transient click flashes without disabling static hover feedback.
    pub animations: bool,
    pub user_display: UserDisplay,
    pub user_name_pattern: Option<String>,
    pub user_name_format: Option<String>,
    pub colors: ThemeColors,
    pub list_refresh_secs: u64,
    pub detail_refresh_secs: u64,
    pub active_tab: usize,
    pub selections: BTreeMap<String, usize>,
    pub tab_states: BTreeMap<String, TabState>,
    pub route: Option<ItemKey>,
    pub section: Option<usize>,
    pub field: usize,
    pub filters: BTreeMap<String, String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TabState {
    pub route: Option<ItemKey>,
    pub section: Option<usize>,
    pub field: usize,
    pub section_cursor: usize,
    pub list_offset: usize,
    pub content_offset: usize,
    pub expanded: BTreeSet<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedView {
    pub name: String,
    pub kind: ItemKind,
    pub query: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserDisplay {
    #[default]
    Username,
    Name,
    Id,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeColors {
    pub background: Option<String>,
    pub surface: Option<String>,
    pub selection: Option<String>,
    pub foreground: Option<String>,
    pub muted: Option<String>,
    pub accent: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gitlab_url: "https://gitlab.com".into(),
            token_env: "GITLAB_TOKEN".into(),
            projects: Vec::new(),
            views: Vec::new(),
            theme: "midnight".into(),
            animations: true,
            user_display: UserDisplay::default(),
            user_name_pattern: None,
            user_name_format: None,
            colors: ThemeColors::default(),
            list_refresh_secs: 60,
            detail_refresh_secs: 10,
            active_tab: 0,
            selections: BTreeMap::new(),
            tab_states: BTreeMap::new(),
            route: None,
            section: None,
            field: 0,
            filters: BTreeMap::new(),
        }
    }
}

#[derive(Clone)]
pub struct UserFormatter {
    display: UserDisplay,
    pattern: Option<Regex>,
    format: Option<String>,
}

impl UserFormatter {
    pub(crate) fn from_config(config: &Config) -> Result<Self> {
        ensure!(
            config.user_name_pattern.is_some() == config.user_name_format.is_some(),
            "user_name_pattern and user_name_format must be configured together"
        );
        let pattern = config
            .user_name_pattern
            .as_deref()
            .map(|pattern| {
                ensure!(
                    pattern.len() <= 1024,
                    "user_name_pattern must be at most 1024 bytes"
                );
                RegexBuilder::new(pattern)
                    .size_limit(1 << 20)
                    .build()
                    .context("user_name_pattern is not a valid Rust regex")
            })
            .transpose()?;
        if let Some(format) = &config.user_name_format {
            ensure!(
                format.len() <= 1024,
                "user_name_format must be at most 1024 bytes"
            );
        }
        Ok(Self {
            display: config.user_display,
            pattern,
            format: config.user_name_format.clone(),
        })
    }

    pub fn render_lookup(&self, option: &LookupOption) -> String {
        if self.display == UserDisplay::Username && !option.label.trim().is_empty() {
            return option.label.clone();
        }
        self.render(&User {
            id: option.id,
            username: option.value.trim_start_matches('@').into(),
            name: option.label.clone(),
        })
    }

    pub fn render(&self, user: &User) -> String {
        match self.display {
            UserDisplay::Username if !user.username.is_empty() => format!("@{}", user.username),
            UserDisplay::Name if !user.name.trim().is_empty() => {
                if let (Some(pattern), Some(format)) = (&self.pattern, &self.format)
                    && let Some(captures) = pattern.captures(&user.name)
                {
                    let mut rendered = String::new();
                    captures.expand(format, &mut rendered);
                    if !rendered.trim().is_empty() {
                        return rendered;
                    }
                }
                user.name.clone()
            }
            UserDisplay::Id if user.id > 0 => user.id.to_string(),
            _ => fallback_user(user),
        }
    }
}

fn fallback_user(user: &User) -> String {
    if !user.username.is_empty() {
        format!("@{}", user.username)
    } else if !user.name.trim().is_empty() {
        user.name.clone()
    } else if user.id > 0 {
        user.id.to_string()
    } else {
        "unknown".into()
    }
}

// Even an invalid, not-yet-validated config might contain a pasted credential.
// Do not include any user-provided strings in diagnostic formatting.
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("projects", &self.projects.len())
            .field("views", &self.views.len())
            .field("list_refresh_secs", &self.list_refresh_secs)
            .field("detail_refresh_secs", &self.detail_refresh_secs)
            .field("active_tab", &self.active_tab)
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}

impl Config {
    /// Missing files use defaults. All other I/O, syntax, and validation errors
    /// are returned without creating or changing anything on disk.
    pub fn load(path: &Path) -> Result<Self> {
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => {
                return Err(error).with_context(|| format!("read config {}", path.display()));
            }
        };
        let config: Self =
            toml::from_str(&source).with_context(|| format!("parse config {}", path.display()))?;
        config
            .validate()
            .with_context(|| format!("validate config {}", path.display()))?;
        Ok(config)
    }

    /// Synchronously replace the complete workspace, using a temporary file on
    /// the same filesystem. Invalid state is rejected before touching the disk.
    /// Unix files are mode 0600, newly created directories are mode 0700, and the
    /// containing directory is synced after rename. Other platforms use their
    /// native tempfile permissions and file sync support.
    ///
    /// A directory-sync error can be returned *after* replacement has succeeded;
    /// the file is complete, but durability could not be confirmed.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let source = toml::to_string_pretty(self).context("serialize config")?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));

        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(parent)
            .with_context(|| format!("create config directory {}", parent.display()))?;

        let mut temporary =
            NamedTempFile::new_in(parent).context("create config temporary file")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temporary
                .as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))
                .context("restrict config permissions")?;
        }
        temporary
            .write_all(source.as_bytes())
            .context("write config temporary file")?;
        temporary
            .as_file()
            .sync_all()
            .context("sync config temporary file")?;
        temporary
            .persist(path)
            .with_context(|| format!("replace config {}", path.display()))?;
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .with_context(|| format!("sync config directory {}", parent.display()))?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.list_refresh_secs >= 15,
            "list_refresh_secs must be at least 15"
        );
        ensure!(
            self.detail_refresh_secs >= 2,
            "detail_refresh_secs must be at least 2"
        );
        ensure!(
            matches!(self.theme.as_str(), "midnight" | "dracula" | "light"),
            "theme must be midnight, dracula, or light"
        );
        UserFormatter::from_config(self)?;

        let url =
            reqwest::Url::parse(&self.gitlab_url).context("gitlab_url must be a valid URL")?;
        ensure!(
            matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
            "gitlab_url must be an absolute HTTP or HTTPS URL"
        );
        ensure!(
            url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "gitlab_url must not contain credentials, a query, or a fragment; use token_env"
        );
        let mut name = self.token_env.bytes();
        ensure!(
            name.next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
                && name.all(|c| c.is_ascii_alphanumeric() || c == b'_'),
            "token_env must be an environment variable name, not a token"
        );

        for (name, color) in [
            ("background", &self.colors.background),
            ("surface", &self.colors.surface),
            ("selection", &self.colors.selection),
            ("foreground", &self.colors.foreground),
            ("muted", &self.colors.muted),
            ("accent", &self.colors.accent),
        ] {
            if let Some(color) = color {
                ensure!(
                    color.len() == 7
                        && color.starts_with('#')
                        && color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit),
                    "colors.{name} must be a #RRGGBB hex color"
                );
            }
        }

        let mut names = BTreeSet::new();
        for (index, view) in self.views.iter().enumerate() {
            ensure!(
                !view.name.trim().is_empty(),
                "view {index} must have a nonempty name"
            );
            ensure!(
                names.insert(view.name.trim()),
                "view {index} has a duplicate name"
            );
            if let Err(error) = Query::parse(&view.query) {
                bail!("view {index} has an invalid query: {error}");
            }
        }
        // Filters are workspace input, not saved view definitions. Preserve even
        // incomplete queries so saving another event cannot discard typed input.
        Ok(())
    }
}

/// The live configuration path. Demo isolation is the caller's responsibility.
pub fn default_path() -> Result<PathBuf> {
    let directories = ProjectDirs::from("", "", "cronk")
        .context("could not determine the user configuration directory")?;
    Ok(directories.config_dir().join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn defaults_are_valid_and_missing_load_is_read_only() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("missing/config.toml");
        let config = Config::load(&path).unwrap();
        config.validate().unwrap();
        assert_eq!(config.gitlab_url, "https://gitlab.com");
        assert_eq!(config.token_env, "GITLAB_TOKEN");
        assert_eq!(config.theme, "midnight");
        assert_eq!(config.user_display, UserDisplay::Username);
        assert!(config.route.is_none());
        assert!(!directory.path().join("missing").exists());
    }

    #[test]
    fn user_formatter_supports_named_capture_replacements_and_safe_fallbacks() {
        let mut config = Config {
            user_display: UserDisplay::Name,
            user_name_pattern: Some(
                r"^(?P<surname>\S+)\s+(?P<firstname>\S+)\s+(?P<staff_id>\d+)$".into(),
            ),
            user_name_format: Some("$firstname".into()),
            ..Config::default()
        };
        config.validate().unwrap();
        let formatter = UserFormatter::from_config(&config).unwrap();
        let mut user = User {
            id: 42,
            username: "jdoe".into(),
            name: "Doe John 12345678".into(),
        };
        assert_eq!(formatter.render(&user), "John");
        let option = LookupOption {
            id: user.id,
            label: user.name.clone(),
            value: "@jdoe".into(),
            api_value: "42".into(),
            description: "@jdoe · ID 42".into(),
            color: String::new(),
            text_color: String::new(),
        };
        assert_eq!(formatter.render_lookup(&option), "John");
        user.name = "Jane Example".into();
        assert_eq!(formatter.render(&user), "Jane Example");

        config.user_display = UserDisplay::Id;
        assert_eq!(
            UserFormatter::from_config(&config).unwrap().render(&user),
            "42"
        );
        config.user_display = UserDisplay::Username;
        assert_eq!(
            UserFormatter::from_config(&config).unwrap().render(&user),
            "@jdoe"
        );
    }

    #[test]
    fn user_name_pattern_and_format_are_validated_together() {
        for (pattern, format) in [
            (Some("[".into()), Some("$firstname".into())),
            (Some("(?P<name>.*)".into()), None),
            (None, Some("$firstname".into())),
        ] {
            let config = Config {
                user_name_pattern: pattern,
                user_name_format: format,
                ..Config::default()
            };
            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn partial_and_empty_configs_use_defaults() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("config.toml");
        for source in ["", "theme = 'dracula'\n[colors]\naccent = '#aAbB01'\n"] {
            fs::write(&path, source).unwrap();
            let config = Config::load(&path).unwrap();
            assert_eq!(config.list_refresh_secs, 60);
            assert_eq!(config.detail_refresh_secs, 10);
            assert!(config.projects.is_empty());
            assert!(config.colors.background.is_none());
        }
    }

    #[test]
    fn invalid_files_are_not_replaced_or_silently_defaulted() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("config.toml");
        for source in [
            "not valid TOML [",
            "theme = 'unknown'",
            "detail_refresh_secs = 1",
            "token = 'do-not-store-credentials'",
            "[[views]]\nname = 'missing kind and query'",
            "[[views]]\nname = 'bad'\nkind = 'issue'\nquery = 'unknown:value'",
            "[[views]]\nname = 'bad'\nkind = 'unknown'\nquery = ''",
        ] {
            fs::write(&path, source).unwrap();
            assert!(Config::load(&path).is_err(), "{source}");
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
        }
        fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(Config::load(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), [0xff, 0xfe]);
        assert!(Config::load(directory.path()).is_err());
    }

    #[test]
    fn complete_workspace_round_trips_and_replaces_per_event() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("nested/config.toml");
        let mut config = Config {
            gitlab_url: "https://gitlab.example.com/gitlab".into(),
            token_env: "WORK_GITLAB_TOKEN".into(),
            projects: vec![Project {
                id: 42,
                path: "team/repo".into(),
                alias: "work".into(),
                visible: true,
            }],
            views: vec![SavedView {
                name: "Reviews".into(),
                kind: ItemKind::MergeRequest,
                query: "reviewer:@me draft:false".into(),
            }],
            theme: "light".into(),
            animations: false,
            user_display: UserDisplay::Name,
            user_name_pattern: Some(
                r"^(?P<surname>\S+)\s+(?P<firstname>\S+)\s+(?P<staff_id>\d+)$".into(),
            ),
            user_name_format: Some("$firstname".into()),
            colors: ThemeColors {
                background: Some("#000001".into()),
                surface: Some("#000002".into()),
                selection: Some("#000003".into()),
                foreground: Some("#000004".into()),
                muted: Some("#000005".into()),
                accent: Some("#AABBCC".into()),
            },
            list_refresh_secs: 15,
            detail_refresh_secs: 2,
            active_tab: 3,
            selections: BTreeMap::from([("project/42:issues".into(), 17)]),
            tab_states: BTreeMap::from([(
                "3".into(),
                TabState {
                    route: Some(ItemKey {
                        project: 42,
                        iid: 7,
                        kind: ItemKind::MergeRequest,
                    }),
                    section: Some(2),
                    section_cursor: 2,
                    field: 4,
                    list_offset: 12,
                    content_offset: 8,
                    expanded: BTreeSet::from([91, 92]),
                },
            )]),
            route: Some(ItemKey {
                project: 42,
                iid: 7,
                kind: ItemKind::MergeRequest,
            }),
            section: Some(2),
            field: 4,
            filters: BTreeMap::from([("Reviews".into(), "label:\"typing".into())]),
        };
        for field in [4, 5, 0] {
            config.field = field;
            config.save(&path).unwrap();
            let loaded = Config::load(&path).unwrap();
            assert_eq!(
                toml::to_string(&loaded).unwrap(),
                toml::to_string(&config).unwrap()
            );
        }
        config.route = None;
        config.section = None;
        config.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert!(loaded.route.is_none());
        assert!(loaded.section.is_none());
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn failed_validation_does_not_touch_existing_or_missing_files() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("config.toml");
        Config::default().save(&path).unwrap();
        let before = fs::read(&path).unwrap();
        let config = Config {
            theme: "invalid".into(),
            ..Config::default()
        };
        assert!(config.save(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(
            config
                .save(&directory.path().join("new/config.toml"))
                .is_err()
        );
        assert!(!directory.path().join("new").exists());
    }

    #[test]
    fn failed_persist_cleans_temporary_file_and_preserves_destination() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("destination");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), "unchanged").unwrap();
        assert!(Config::default().save(&path).is_err());
        assert_eq!(fs::read_to_string(path.join("keep")).unwrap(), "unchanged");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn refresh_interval_boundaries() {
        for (list, detail, valid) in [
            (0, 0, false),
            (14, 2, false),
            (15, 1, false),
            (15, 2, true),
            (60, 10, true),
        ] {
            let config = Config {
                list_refresh_secs: list,
                detail_refresh_secs: detail,
                ..Config::default()
            };
            assert_eq!(config.validate().is_ok(), valid);
        }
    }

    #[test]
    fn theme_names_and_all_color_overrides_are_validated() {
        for theme in ["midnight", "dracula", "light"] {
            Config {
                theme: theme.into(),
                ..Config::default()
            }
            .validate()
            .unwrap();
        }
        for theme in ["", "dark", "Midnight", " light "] {
            assert!(
                Config {
                    theme: theme.into(),
                    ..Config::default()
                }
                .validate()
                .is_err()
            );
        }
        for color in ["", "#abc", "#12345678", "123456", "#gg0000", "#é0000"] {
            for index in 0..6 {
                let mut config = Config::default();
                let colors = &mut config.colors;
                let slots = [
                    &mut colors.background,
                    &mut colors.surface,
                    &mut colors.selection,
                    &mut colors.foreground,
                    &mut colors.muted,
                    &mut colors.accent,
                ];
                *slots.into_iter().nth(index).unwrap() = Some(color.into());
                assert!(config.validate().is_err(), "slot {index}: {color}");
            }
        }
    }

    #[test]
    fn saved_views_require_unique_nonempty_names_and_valid_queries() {
        let view = |name: &str, query: &str| SavedView {
            name: name.into(),
            kind: ItemKind::Issue,
            query: query.into(),
        };
        for views in [
            vec![view("  ", "")],
            vec![view("Inbox", ""), view(" Inbox ", "state:opened")],
            vec![view("Inbox", "draft:yes")],
            vec![view("Inbox", "label:\"unfinished")],
        ] {
            assert!(
                Config {
                    views,
                    ..Config::default()
                }
                .validate()
                .is_err()
            );
        }
        Config {
            views: vec![view("All", ""), view("Mine", "assignee:@me")],
            ..Config::default()
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn endpoint_and_token_reference_cannot_embed_authentication() {
        for url in [
            "not a URL",
            "file:///tmp/gitlab",
            "ftp://gitlab.com",
            "https://alice:secret@gitlab.com",
            "https://secret@gitlab.com",
            "https://gitlab.com?private_token=secret",
            "https://gitlab.com#secret",
        ] {
            assert!(
                Config {
                    gitlab_url: url.into(),
                    ..Config::default()
                }
                .validate()
                .is_err(),
                "{url}"
            );
        }
        for name in ["", "glpat-secret", "123_TOKEN", "TOKEN=value", "TOKEN NAME"] {
            assert!(
                Config {
                    token_env: name.into(),
                    ..Config::default()
                }
                .validate()
                .is_err()
            );
        }
        for name in ["GITLAB_TOKEN", "_TOKEN", "GitLab2"] {
            Config {
                token_env: name.into(),
                ..Config::default()
            }
            .validate()
            .unwrap();
        }
    }

    #[test]
    fn debug_never_contains_user_provided_strings() {
        let config = Config {
            gitlab_url: "https://user:sensitive@gitlab.com".into(),
            token_env: "sensitive".into(),
            theme: "sensitive".into(),
            projects: vec![Project {
                alias: "sensitive".into(),
                ..Project::default()
            }],
            views: vec![SavedView {
                name: "sensitive".into(),
                kind: ItemKind::Issue,
                query: "sensitive".into(),
            }],
            filters: BTreeMap::from([("sensitive".into(), "sensitive".into())]),
            ..Config::default()
        };
        assert!(!format!("{config:?}").contains("sensitive"));
    }

    #[test]
    fn default_path_uses_platform_config_directory() {
        if let Some(directories) = ProjectDirs::from("", "", "cronk") {
            assert_eq!(
                default_path().unwrap(),
                directories.config_dir().join("config.toml")
            );
        } else {
            assert!(default_path().is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn saves_create_private_directories_and_replace_with_private_files() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempdir().unwrap();
        let path = directory.path().join("private/nested/config.toml");
        Config::default().save(&path).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        for parent in [
            directory.path().join("private"),
            directory.path().join("private/nested"),
        ] {
            assert_eq!(
                fs::metadata(parent).unwrap().permissions().mode() & 0o077,
                0
            );
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        Config::default().save(&path).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_replacement_does_not_follow_destination_symlinks() {
        use std::os::unix::fs::symlink;
        let directory = tempdir().unwrap();
        let target = directory.path().join("target");
        let path = directory.path().join("config.toml");
        fs::write(&target, "keep target unchanged").unwrap();
        symlink(&target, &path).unwrap();
        Config::default().save(&path).unwrap();
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "keep target unchanged"
        );
        assert!(
            !fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        Config::load(&path).unwrap();
    }
}
