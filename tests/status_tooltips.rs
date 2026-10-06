use cronk::{
    config::Config,
    demo,
    model::{ItemKey, ItemKind},
    ui::{Cronk, Msg, Scope},
};
use tui_lipan::{
    TestBackend,
    core::event::{MouseButton, MouseKind},
    prelude::*,
};

type Ui = TestBackend<Cronk>;
const THEMES: [&str; 3] = ["midnight", "dracula", "light"];

fn mount(theme: &str, tab: usize, route: Option<ItemKey>) -> Ui {
    let mut ui = TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config: Config {
                projects: demo::projects(),
                theme: theme.into(),
                active_tab: tab,
                route,
                onboarding: false,
                ..Config::default()
            },
            path: None,
            api: None,
            demo: true,
        },
        (),
        Rect {
            x: 0,
            y: 0,
            w: 160,
            h: 300,
        },
    );
    settle(&mut ui);
    ui
}

fn settle(ui: &mut Ui) {
    for _ in 0..3 {
        ui.pump().unwrap();
        ui.render();
    }
}

fn mouse(ui: &mut Ui, x: u16, y: u16, kind: MouseKind) {
    ui.send_mouse(MouseEvent {
        x,
        y,
        kind,
        mods: KeyMods::NONE,
    })
    .unwrap();
    settle(ui);
}

fn rect(ui: &Ui, key: &str) -> Rect {
    ui.rect_of_key(&key.to_owned().into())
        .unwrap_or_else(|| panic!("missing {key}"))
}

fn assert_closed(ui: &Ui) {
    assert!(
        ui.rect_of_key(&"status-tooltip".into()).is_none(),
        "tooltip must be hover-only"
    );
}

fn assert_tooltip(ui: &mut Ui, key: &str, text: &str) {
    mouse(ui, 0, 0, MouseKind::Moved);
    assert_closed(ui);
    let dot = rect(ui, key);
    assert_eq!(
        (dot.w, dot.h),
        (1, 1),
        "only the status glyph is the trigger: {key}"
    );
    assert!(
        dot.x >= 0 && dot.y >= 0 && dot.y < ui.viewport().h as i16,
        "{key}: {dot:?}"
    );
    let x = dot.x as u16;
    let y = dot.y as u16;
    let config = serde_json::to_value(&ui.state().config).unwrap();
    let focus = ui.focused_key().cloned();
    let selected = ui.state().scroll.selected;
    let offset = ui.state().content_offset;
    mouse(ui, x, y, MouseKind::Moved);
    assert!(ui.rect_of_key(&"status-tooltip".into()).is_some());
    let frame = ui.capture_frame().plain_text();
    assert!(
        frame.contains(text),
        "{key} tooltip missing {text:?}:\n{frame}"
    );
    assert_eq!(serde_json::to_value(&ui.state().config).unwrap(), config);
    assert_eq!(ui.focused_key(), focus.as_ref());
    assert_eq!(ui.state().scroll.selected, selected);
    assert_eq!(ui.state().content_offset, offset);
    assert!(ui.state().dialog.is_none());
    for adjacent in [x.saturating_sub(1), x + 1] {
        mouse(ui, adjacent, y, MouseKind::Moved);
        assert_closed(ui);
        mouse(ui, x, y, MouseKind::Moved);
    }
    mouse(ui, 0, 0, MouseKind::Moved);
    assert_closed(ui);
}

#[test]
fn list_status_and_pipeline_dots_have_hover_only_explanations_in_every_theme() {
    for theme in THEMES {
        for tab in [0, 2, 3] {
            let mut ui = mount(theme, tab, None);
            let item = ui.state().visible_items()[0].clone();
            let key = format!(
                "item-{}-{}-{}",
                item.key.project,
                item.key.kind.segment(),
                item.key.iid
            );
            let dot_key = format!("{key}-status");
            assert_eq!(rect(&ui, &dot_key).x, 2);
            let subject = if item.key.kind == ItemKind::Issue {
                "Issue"
            } else {
                "Merge request"
            };
            assert_tooltip(&mut ui, &dot_key, &format!("{subject}: {}", item.state));
            if let Some(pipeline) = &item.pipeline {
                assert_tooltip(
                    &mut ui,
                    &format!("{key}-pipeline-status"),
                    &format!("Pipeline: {}", pipeline.status),
                );
            }
            // Status regions are passive: clicking the dot still activates its row.
            let dot = rect(&ui, &dot_key);
            mouse(&mut ui, dot.x as u16, dot.y as u16, MouseKind::Moved);
            for kind in [
                MouseKind::Down(MouseButton::Left),
                MouseKind::Up(MouseButton::Left),
            ] {
                mouse(&mut ui, dot.x as u16, dot.y as u16, kind);
            }
            assert_eq!(ui.state().config.route.as_ref(), Some(&item.key));
            assert_eq!(ui.state().scope, Scope::Details);
            assert_closed(&ui);
        }
    }
}

#[test]
fn detail_padding_and_status_slots_align_with_lists_in_every_theme() {
    for theme in THEMES {
        for kind in [ItemKind::Issue, ItemKind::MergeRequest] {
            let route = ItemKey {
                project: 9001,
                iid: if kind == ItemKind::Issue { 1 } else { 101 },
                kind,
            };
            let mut ui = mount(
                theme,
                if kind == ItemKind::Issue { 2 } else { 3 },
                Some(route),
            );
            let frame = ui.capture_frame();
            for index in 0..ui.state().sections().len() {
                let header = rect(&ui, &format!("detail-section-{index}"));
                assert_eq!(header.x, 0);
                let y = header.y as u16;
                for x in 1..4 {
                    assert_eq!(frame.cell(x, y).symbol, " ");
                }
                assert_eq!(frame.cell(4, y).symbol, ui.state().sections()[index][..1]);
                assert_eq!(
                    frame.cell(1, y).bg,
                    frame.cell(4, y).bg,
                    "padding belongs to the selection fill"
                );
            }
            let item_dot = rect(&ui, "detail-item-status");
            assert_eq!(item_dot.x, 2);
            assert_eq!(frame.cell(1, item_dot.y as u16).symbol, " ");
            assert_eq!(frame.cell(3, item_dot.y as u16).symbol, " ");
            let title = rect(&ui, "edit-field-0");
            // Labels are right-aligned in a shared column, so the first glyph is "T"
            // of "Title" somewhere after the detail padding.
            let first = (4..40)
                .map(|x| frame.cell(x, title.y as u16).symbol.clone())
                .find(|symbol| symbol != " ");
            assert_eq!(first.as_deref(), Some("T"));
            assert_tooltip(
                &mut ui,
                "detail-item-status",
                if kind == ItemKind::Issue {
                    "Issue: opened"
                } else {
                    "Merge request: opened"
                },
            );
            let details = ui.state().details.as_ref().unwrap();
            let notes = if kind == ItemKind::Issue {
                details.notes.clone()
            } else {
                details
                    .discussions
                    .iter()
                    .flat_map(|d| d.notes.iter().cloned())
                    .collect()
            };
            for note in notes.iter().take(2) {
                let key = format!("note-{}-status", note.id);
                assert_eq!(rect(&ui, &key).x, 2);
                assert_tooltip(
                    &mut ui,
                    &key,
                    if note.system {
                        "System note:"
                    } else {
                        "Comment:"
                    },
                );
            }
            if kind == ItemKind::MergeRequest {
                assert_eq!(rect(&ui, "detail-pipeline-status").x, 2);
                assert_tooltip(&mut ui, "detail-pipeline-status", "Pipeline:");
                for status in ["running", "pending", "success", "failed"] {
                    assert_tooltip(
                        &mut ui,
                        &format!("pipeline-summary-{status}-status"),
                        &format!("Jobs: {status}"),
                    );
                }
                let jobs = ui.state().details.as_ref().unwrap().jobs.clone();
                for job in jobs {
                    let key = format!("job-{}-status", job.id);
                    assert_eq!(rect(&ui, &key).x, 2);
                    assert_tooltip(
                        &mut ui,
                        &key,
                        if job.allow_failure {
                            "Job (failure is allowed):"
                        } else {
                            "Job:"
                        },
                    );
                }
                let discussions = ui.state().details.as_ref().unwrap().discussions.clone();
                for discussion in discussions {
                    let key = format!("discussion-{}-status", discussion.id);
                    assert_eq!(rect(&ui, &key).x, 2);
                    assert_tooltip(&mut ui, &key, "Discussion:");
                }
            }
            assert_closed(&ui);
            ui.dispatch(Msg::Enter).unwrap();
            ui.dispatch(Msg::Move(1)).unwrap();
            settle(&mut ui);
            assert_closed(&ui);
        }
    }
}

#[test]
fn tooltips_explain_live_logs_sync_and_diff_warnings_and_do_not_block_job_clicks() {
    let route = ItemKey {
        project: 9001,
        iid: 102,
        kind: ItemKind::MergeRequest,
    };
    let mut ui = mount("midnight", 3, Some(route));
    ui.dispatch(Msg::LoadTraces).unwrap();
    settle(&mut ui);
    let running = ui
        .state()
        .details
        .as_ref()
        .unwrap()
        .jobs
        .iter()
        .find(|job| job.running())
        .unwrap()
        .id;
    assert_tooltip(&mut ui, &format!("job-{running}-live-status"), "Live log:");
    ui.state_mut().mutation_pending = true;
    ui.render();
    assert_tooltip(&mut ui, "sync-status", "Syncing: Requests to GitLab");
    ui.state_mut().mutation_pending = false;
    ui.state_mut().details.as_mut().unwrap().diffs[0].collapsed = true;
    ui.render();
    assert_tooltip(&mut ui, "diff-0-warning-status", "Diff warning: warning");
    let job = ui.state().details.as_ref().unwrap().jobs[0].id;
    let dot = rect(&ui, &format!("job-{job}-status"));
    assert!(!ui.state().expanded.contains(&job));
    for kind in [
        MouseKind::Down(MouseButton::Left),
        MouseKind::Up(MouseButton::Left),
    ] {
        mouse(&mut ui, dot.x as u16, dot.y as u16, kind);
    }
    assert!(ui.state().expanded.contains(&job));
    assert_eq!(ui.state().scope, Scope::Section);
    assert_eq!(ui.state().config.section, Some(3));
}
