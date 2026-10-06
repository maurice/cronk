//! Debounced setup checks run on command workers, never on the UI thread.
use super::*;
use crate::demo;
use tui_lipan::CommandLink;

#[derive(Default)]
pub struct SetupState {
    pub epoch: u64,
    pub feedback: Vec<String>,
    pub validated: Option<Validation>,
}

pub struct Validation {
    pub feedback: Vec<String>,
    pub config: Config,
    pub api: Option<GitLab>,
    pub user: User,
    pub valid: bool,
}

fn validate(mut config: Config, values: Vec<String>, demo_mode: bool) -> Validation {
    let changed_host = config.gitlab_url.trim_end_matches('/')
        != values[1].trim().trim_end_matches('/')
        && !config.projects.is_empty();
    config.token_env = values[0].trim().into();
    config.gitlab_url = values[1].trim().trim_end_matches('/').into();
    let mut result = Validation {
        feedback: vec![String::new(); 3],
        config,
        api: None,
        user: User::default(),
        valid: false,
    };
    // Check each field independently so one typo doesn't hide another.
    let token_check = Config {
        token_env: result.config.token_env.clone(),
        ..Config::default()
    }
    .validate();
    let url_check = Config {
        gitlab_url: result.config.gitlab_url.clone(),
        ..Config::default()
    }
    .validate();
    result.feedback[0] = match &token_check {
        Ok(()) => "Environment variable name is valid".into(),
        Err(_) => "Use an environment variable name, not the token itself".into(),
    };
    result.feedback[1] = match &url_check {
        Ok(()) => "URL format is valid; authentication will verify this instance".into(),
        Err(_) => "Use an absolute HTTP(S) URL without credentials, query, or fragment".into(),
    };
    let paths: Vec<_> = values[2]
        .split([',', '\n'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    result.feedback[2] = if paths.is_empty() {
        "Enter at least one project path or numeric ID".into()
    } else {
        "Projects will be checked after authentication".into()
    };
    if changed_host && !demo_mode {
        result.feedback[1] =
            "This workspace has projects on another host. Use a separate --config file".into();
        return result;
    }
    if token_check.is_err() || url_check.is_err() {
        return result;
    }
    if !demo_mode {
        let token = match std::env::var(&result.config.token_env) {
            Ok(token) if !token.trim().is_empty() => token,
            _ => {
                result.feedback[0] =
                    "Variable is unset or empty. Export it before starting Cronk, then restart"
                        .into();
                return result;
            }
        };
        let api = match GitLab::new(&result.config.gitlab_url, &token) {
            Ok(api) => api,
            Err(error) => {
                result.feedback[1] = error.to_string();
                return result;
            }
        };
        match api.current_user() {
            Ok(user) => {
                result.feedback[0] = format!("Authenticated as @{}", user.username);
                result.feedback[1] = "GitLab instance connected successfully".into();
                result.user = user;
                result.api = Some(api);
            }
            Err(error) => {
                result.feedback[0] = format!("Authentication/connection failed: {error}");
                result.feedback[1] =
                    "Could not verify GitLab; check URL, token, network, and TLS trust".into();
                return result;
            }
        }
    } else {
        result.user = demo::user();
        result.feedback[0] = "Demo: valid variable name (no secret read)".into();
        result.feedback[1] = "Demo: valid URL (no network request)".into();
    }
    if paths.is_empty() {
        return result;
    }
    let mut projects = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        let resolved = if demo_mode {
            demo::projects()
                .into_iter()
                .find(|p| p.path == path || p.id.to_string() == path)
                .ok_or_else(|| "Not a fictional project; use a demo path or ID".to_string())
        } else {
            result
                .api
                .as_ref()
                .unwrap()
                .resolve_project(path)
                .map_err(|e| e.to_string())
        };
        match resolved {
            Ok(project) => {
                if !projects.iter().any(|p: &Project| p.id == project.id) {
                    projects.push(project);
                }
            }
            // Avoid echoing arbitrary pasted input (which may be a credential).
            Err(error) => errors.push(format!(
                "Project {}: {error}",
                projects.len() + errors.len() + 1
            )),
        }
    }
    if !errors.is_empty() {
        result.feedback[2] = errors.join("; ");
        return result;
    }
    result.feedback[2] = format!(
        "{} project(s) verified: {}",
        projects.len(),
        projects
            .iter()
            .map(|p| p.path.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    // Preserve local aliases and visibility when reopening setup.
    for project in &mut projects {
        if let Some(existing) = result.config.projects.iter().find(|p| p.id == project.id) {
            project.alias = existing.alias.clone();
            project.visible = existing.visible;
        }
    }
    result.config.projects = projects;
    result.valid = result.config.validate().is_ok();
    result
}

impl Cronk {
    pub(super) fn show_onboarding(&self, ctx: &mut Context<Self>) {
        let location = self.path.as_ref().map_or_else(
            || "preview only (no file will be saved)".into(),
            |path| path.display().to_string(),
        );
        let demo_help = if ctx.state.demo {
            let projects = demo::projects();
            format!(
                " Demo checks are simulated; try {} or ID {}. No credentials or network needed.",
                projects[0].path, projects[0].id
            )
        } else {
            String::new()
        };
        let config = &ctx.state.config;
        let fields = vec![
            FormField::new(
                "GitLab token environment variable (not the token)",
                &config.token_env,
                false,
            ),
            FormField::new("GitLab instance URL", &config.gitlab_url, false),
            FormField::new(
                "Projects (paths or IDs, separated by commas or newlines)",
                &config
                    .projects
                    .iter()
                    .map(|p| p.path.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                true,
            ),
        ];
        self.show_dialog(ctx, DialogKind::Onboarding, "Welcome to Cronk", &format!(
            "Set up your GitLab workspace. Config: {location}. Edit it later and restart Cronk to apply changes. Close skips setup.{demo_help}"), fields);
    }

    pub(super) fn schedule_setup(&self, ctx: &mut Context<Self>) -> Command {
        ctx.state.onboarding.epoch += 1;
        ctx.state.onboarding.validated = None;
        ctx.state.onboarding.feedback = vec!["Checking…".into(); 3];
        let epoch = ctx.state.onboarding.epoch;
        Command::after(Duration::from_millis(400), move |link: CommandLink<Msg>| {
            link.send(Msg::SetupValidate(epoch));
        })
    }

    pub(super) fn validate_setup(&self, ctx: &mut Context<Self>, epoch: u64) -> Update {
        if epoch != ctx.state.onboarding.epoch {
            return Update::none();
        }
        let Some(dialog) = ctx
            .state
            .dialog
            .as_ref()
            .filter(|d| matches!(d.kind, DialogKind::Onboarding))
        else {
            return Update::none();
        };
        let values = dialog.fields.iter().map(|f| f.value().to_owned()).collect();
        let config = ctx.state.config.clone();
        let demo_mode = ctx.state.demo;
        Update::with_command(ctx.link().command(move |link| {
            link.send(Msg::SetupValidated(
                epoch,
                Box::new(validate(config, values, demo_mode)),
            ));
        }))
    }

    pub(super) fn save_setup(&mut self, ctx: &mut Context<Self>) -> Update {
        let Some(validation) = ctx.state.onboarding.validated.take().filter(|v| v.valid) else {
            self.dialog_error(
                ctx,
                "Wait for all checks to succeed, or correct the feedback below each field",
            );
            return Update::full();
        };
        let previous = ctx.state.config.clone();
        ctx.state.config.gitlab_url = validation.config.gitlab_url.clone();
        ctx.state.config.token_env = validation.config.token_env.clone();
        ctx.state.config.projects = validation.config.projects.clone();
        ctx.state.config.onboarding = false;
        self.reset_detail(ctx);
        if !self.persist(ctx) {
            ctx.state.config = previous;
            ctx.state.onboarding.validated = Some(validation);
            self.dialog_error(
                ctx,
                "Could not save configuration; setup is retained. Check the workspace error",
            );
            return Update::full();
        }
        self.api = validation.api;
        self.enable_cache(ctx);
        ctx.state.shared_details.clear();
        ctx.state.detail_requests.clear();
        ctx.state.tab_cache.clear();
        ctx.state.sync_progress.clear();
        ctx.state.list_epoch += 1;
        ctx.state.list_pending.clear();
        ctx.state.user = validation.user;
        ctx.state.items = if ctx.state.demo {
            demo::items()
        } else {
            Vec::new()
        };
        self.close_dialog(ctx);
        ctx.link().send(Msg::Refresh);
        Update::full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn demo_checks_all_fields_and_deduplicates_projects() {
        let project = &demo::projects()[0];
        let values = vec![
            "GITLAB_TOKEN".into(),
            "https://gitlab.com".into(),
            format!("{}, {}", project.path, project.id),
        ];
        let result = validate(Config::default(), values, true);
        assert!(result.valid);
        assert!(result.api.is_none());
        assert_eq!(result.config.projects.len(), 1);
        let invalid = validate(
            Config::default(),
            vec!["glpat-secret".into(), "not-a-url".into(), "".into()],
            true,
        );
        assert!(!invalid.valid);
        assert!(invalid.feedback[0].contains("variable name"));
        assert!(invalid.feedback[1].contains("absolute"));
        assert!(invalid.feedback[2].contains("at least one"));
        let unknown = validate(
            Config::default(),
            vec![
                "TOKEN".into(),
                "https://gitlab.com".into(),
                "missing/project".into(),
            ],
            true,
        );
        assert!(!unknown.valid);
        assert!(unknown.feedback[2].contains("fictional"));
    }
}
