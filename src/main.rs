use anyhow::{Context, Result, bail};
use clap::Parser;
use cronk::{
    build_info,
    config::{Config, default_path},
    demo,
    gitlab::GitLab,
    ui::Cronk,
};
use std::{path::PathBuf, sync::LazyLock};
use tui_lipan::{TestBackend, prelude::*};

static CLI_VERSION: LazyLock<String> = LazyLock::new(build_info::display_version);

#[derive(Parser)]
#[command(version = CLI_VERSION.as_str(), about = "A keyboard-first, multi-project GitLab workspace")]
struct Args {
    /// Use fictional, read-only data (local state uses a separate .demo.toml file).
    #[arg(long)]
    demo: bool,
    /// Reopen first-run setup (also works with --demo and --snapshot).
    #[arg(long)]
    onboarding: bool,
    /// Workspace TOML path. Defaults to the platform's user config directory.
    #[arg(long)]
    config: Option<PathBuf>,
    /// GitLab installation URL, including any enterprise subpath.
    #[arg(long, env = "GITLAB_URL")]
    host: Option<String>,
    /// Create a new configuration file without connecting to GitLab.
    #[arg(long)]
    init: bool,
    /// Verify authentication and print the authenticated username, then exit.
    #[arg(long)]
    check: bool,
    /// Render demo UI to a PNG or Markdown file without opening a terminal.
    #[arg(long, requires = "demo")]
    snapshot: Option<PathBuf>,
    /// Snapshot viewport width.
    #[arg(long, default_value_t = 120)]
    width: u16,
    /// Snapshot viewport height.
    #[arg(long, default_value_t = 40)]
    height: u16,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let mut path = match args.config {
        Some(path) => path,
        None => default_path()?,
    };
    if args.demo {
        path.set_extension("demo.toml");
    }
    let exists = path.exists();
    let mut config = Config::load(&path)?;
    if let Some(host) = args.host {
        if host.trim_end_matches('/') != config.gitlab_url.trim_end_matches('/')
            && !config.projects.is_empty()
        {
            bail!(
                "This workspace already has projects on a different host. Use a separate --config file for each GitLab installation."
            );
        }
        config.gitlab_url = host;
    }
    if args.demo && !exists {
        config.projects = demo::projects();
    }
    if args.onboarding {
        config.onboarding = true;
    }
    // Regular demo snapshots remain workspace previews; opt in to setup explicitly.
    if args.snapshot.is_some() && !args.onboarding {
        config.onboarding = false;
    }
    config.validate()?;
    if args.init {
        if exists {
            bail!(
                "Configuration already exists at {}; it was not changed",
                path.display()
            );
        }
        config.save(&path)?;
        println!("Created {}", path.display());
        println!(
            "Set {} in your environment, then run cronk. Add projects with Ctrl+P.",
            config.token_env
        );
        return Ok(());
    }
    let api = if args.demo || (config.onboarding && !args.check) {
        None
    } else {
        let token = std::env::var(&config.token_env).with_context(|| format!("Set {} to a GitLab personal access token. Use --onboarding to configure your instance, or --demo to explore without credentials.", config.token_env))?;
        Some(GitLab::new(&config.gitlab_url, &token)?)
    };
    if args.check {
        if let Some(api) = api {
            println!("Authenticated as @{}", api.current_user()?.username);
        } else {
            println!("Demo mode: no connection or authentication performed");
        }
        return Ok(());
    }
    if let Some(output) = args.snapshot {
        if args.width < 20 || args.height < 12 {
            bail!("Snapshot viewport must be at least 20×12");
        }
        let mut backend = TestBackend::new(Cronk {
            config,
            path: Some(path),
            api: None,
            demo: true,
        });
        backend.set_viewport(Rect {
            x: 0,
            y: 0,
            w: args.width,
            h: args.height,
        });
        backend.render();
        let snapshot = backend.capture_ui_snapshot();
        if output.extension().is_some_and(|ext| ext == "png") {
            std::fs::write(&output, snapshot.to_png_default()?)?;
        } else {
            std::fs::write(&output, snapshot.to_markdown())?;
        }
        println!("Wrote {}", output.display());
        return Ok(());
    }
    config.save(&path)?;
    App::new()
        .title("Cronk · GitLab workspace")
        .focus_policy(FocusPolicy::Manual)
        .mount(Cronk {
            config,
            path: Some(path),
            api,
            demo: args.demo,
        })
        .run()?;
    Ok(())
}
