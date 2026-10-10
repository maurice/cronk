//! Deterministic visual checks, without a terminal, network, or persisted user state.
use cronk::{
    config::Config,
    demo,
    model::{ItemKey, ItemKind},
    ui::{Cronk, Msg},
};
use std::path::PathBuf;
use tui_lipan::{TestBackend, prelude::*};

fn main() -> anyhow::Result<()> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| ".snapshots".into()),
    );
    std::fs::create_dir_all(&output)?;
    for (name, tab, route, section, theme) in [
        ("dashboard", 0, None, None, "midnight"),
        ("projects", 1, None, None, "midnight"),
        ("issues", 2, None, None, "midnight"),
        ("merge-requests", 3, None, None, "midnight"),
        ("project-details", 1, None, None, "midnight"),
        ("project-details-light", 1, None, None, "light"),
        ("project-details-partial", 1, None, None, "midnight"),
        ("project-details-hidden", 1, None, None, "midnight"),
        (
            "issue",
            2,
            Some(ItemKey {
                project: 9001,
                iid: 1,
                kind: ItemKind::Issue,
            }),
            None,
            "light",
        ),
        (
            "merge-request-fields",
            3,
            Some(ItemKey {
                project: 9001,
                iid: 102,
                kind: ItemKind::MergeRequest,
            }),
            Some(0),
            "light",
        ),
        (
            "jobs",
            3,
            Some(ItemKey {
                project: 9001,
                iid: 102,
                kind: ItemKind::MergeRequest,
            }),
            Some(2),
            "midnight",
        ),
        (
            "changes",
            3,
            Some(ItemKey {
                project: 9001,
                iid: 102,
                kind: ItemKind::MergeRequest,
            }),
            Some(4),
            "dracula",
        ),
        ("project-pipelines", 1, None, Some(1), "midnight"),
        ("project-pipeline-jobs", 1, None, Some(1), "midnight"),
    ] {
        let mut projects = demo::projects();
        if name == "project-details-partial" {
            projects[0].issues_visible = false;
        }
        if name == "project-details-hidden" {
            projects[0].visible = false;
        }
        if name == "projects" {
            projects[1].issues_visible = false;
            projects[2].visible = false;
        }
        let config = Config {
            projects,
            project_route: name.starts_with("project-").then_some(9001),
            active_tab: tab,
            route,
            section,
            theme: theme.into(),
            onboarding: false,
            ..Config::default()
        };
        let mut backend = TestBackend::new_with_app(
            App::new().focus_policy(FocusPolicy::Manual),
            Cronk {
                config,
                path: None,
                api: None,
                demo: true,
            },
            (),
        );
        backend.set_viewport(Rect {
            x: 0,
            y: 0,
            w: 120,
            h: 40,
        });
        backend.state_mut().tick = 12;
        if name == "projects" {
            backend.dispatch(Msg::LoadProbes)?;
            backend.pump()?;
        }
        if name.starts_with("project-pipeline") {
            backend.dispatch(Msg::LoadPipelines)?;
            backend.pump()?;
            backend.dispatch(Msg::Move(2))?;
            if name == "project-pipeline-jobs" {
                let newest = backend.state().project_pipelines[&9001].pipelines[0].id;
                backend.dispatch(Msg::DrillPipeline(newest))?;
                backend.pump()?;
            }
        }
        backend.dispatch(Msg::LoadTraces)?;
        backend.render();
        let snapshot = backend.capture_ui_snapshot();
        std::fs::write(
            output.join(format!("{name}.png")),
            snapshot.to_png_default()?,
        )?;
        std::fs::write(output.join(format!("{name}.md")), snapshot.to_markdown())?;
    }
    println!("Wrote PNG/Markdown snapshots to {}", output.display());
    Ok(())
}
