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
            Some(3),
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
            Some(5),
            "dracula",
        ),
    ] {
        let config = Config {
            projects: demo::projects(),
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
        backend.dispatch(Msg::LoadTraces)?;
        backend.render();
        let snapshot = backend.capture_ui_snapshot();
        std::fs::write(
            output.join(format!("{name}.png")),
            snapshot.to_png_default()?,
        )?;
        std::fs::write(output.join(format!("{name}.md")), snapshot.to_markdown())?;
    }
    println!("Wrote six PNG/Markdown snapshots to {}", output.display());
    Ok(())
}
