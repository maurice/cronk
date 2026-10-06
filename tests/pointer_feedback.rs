use std::time::Duration;

use cronk::{
    config::Config,
    demo,
    model::{ItemKey, ItemKind},
    ui::{Action, Cronk, Msg, Scope},
};
use tui_lipan::{
    TestBackend,
    core::event::{MouseButton, MouseKind},
    prelude::*,
};

type Ui = TestBackend<Cronk>;
const THEMES: [&str; 11] = cronk::config::THEMES;

fn mount(config: Config) -> Ui {
    let mut ui = TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config,
            path: None,
            api: None,
        },
        (),
        Rect {
            x: 0,
            y: 0,
            w: 110,
            h: 44,
        },
    );
    settle(&mut ui);
    ui
}

fn config(theme: &str, tab: usize) -> Config {
    Config {
        projects: demo::projects(),
        theme: theme.into(),
        active_tab: tab,
        ..Config::default()
    }
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

fn point(ui: &Ui, key: &str) -> (u16, u16) {
    let rect = ui
        .rect_of_key(&key.to_owned().into())
        .unwrap_or_else(|| panic!("missing {key}"));
    assert!(
        rect.y >= 0 && rect.y < ui.viewport().h as i16,
        "{key} is offscreen: {rect:?}"
    );
    (rect.x as u16, rect.y as u16)
}

fn assert_hover(ui: &mut Ui, key: &str) {
    mouse(ui, 0, 0, MouseKind::Moved);
    let (mut x, y) = point(ui, key);
    // Selectable rows reserve their first column for the background-free edge.
    if (key.starts_with("project-") && !key.starts_with("project-visible-"))
        || key.starts_with("project-remove")
        || key.starts_with("dialog-option-")
        || key.starts_with("detail-section-")
        || key.starts_with("edit-field-")
        || key.starts_with("job-")
        || key.starts_with("discussion-")
    {
        x += 1;
    }
    let before = ui.capture_frame().cell(x, y).clone();
    let config = serde_json::to_value(&ui.state().config).unwrap();
    let focus = ui.focused_key().cloned();
    mouse(ui, x, y, MouseKind::Moved);
    let hovered = ui.capture_frame().cell(x, y).clone();
    assert_ne!(hovered.bg, before.bg, "no hover feedback for {key}");
    assert_eq!(
        hovered.fg, before.fg,
        "hover must preserve semantic foreground: {key}"
    );
    assert_eq!(serde_json::to_value(&ui.state().config).unwrap(), config);
    assert_eq!(ui.focused_key(), focus.as_ref());
    mouse(ui, 0, 0, MouseKind::Moved);
    assert_eq!(
        ui.capture_frame().cell(x, y),
        &before,
        "hover must clear for {key}"
    );
}

#[test]
fn running_status_uses_balanced_blue_in_every_theme() {
    for (theme, blue) in [
        ("midnight", 0x7bb8ff),
        ("dracula", 0x7fbaff),
        ("light", 0x0066bc),
    ] {
        let ui = mount(config(theme, 0));
        let frame = ui.capture_frame();
        let mut running = 0;
        for y in 0..frame.height {
            for x in 0..frame.width {
                let cell = frame.cell(x, y);
                if cell.symbol == "◐" {
                    assert_eq!(cell.fg, Color::hex_u24(blue), "{theme}");
                    running += 1;
                }
            }
        }
        assert!(running > 0, "running pipeline must be visible in {theme}");
    }
}

#[test]
fn tabs_lists_checkboxes_and_dialog_controls_have_hover_in_every_theme() {
    for theme in THEMES {
        let mut ui = mount(config(theme, 1));
        assert_hover(&mut ui, "workspace-tab-0");
        assert_hover(&mut ui, "workspace-tab-1");
        assert_hover(&mut ui, "project-9001");
        assert_hover(&mut ui, "project-9002");
        assert_hover(&mut ui, "project-visible-9002");
        ui.dispatch(Msg::Action(Action::Themes)).unwrap();
        settle(&mut ui);
        assert_hover(&mut ui, "dialog-option-0");
        assert_hover(&mut ui, "dialog-option-1");
        assert_hover(&mut ui, "dialog-close");
        ui.dispatch(Msg::CloseDialog).unwrap();
        ui.dispatch(Msg::Action(Action::AddProject)).unwrap();
        settle(&mut ui);
        assert_hover(&mut ui, "dialog-field-0");
        assert_hover(&mut ui, "dialog-submit");
        ui.dispatch(Msg::CloseDialog).unwrap();
        ui.dispatch(Msg::Palette).unwrap();
        settle(&mut ui);
        assert_hover(&mut ui, "dialog-option-0");
    }
}

#[test]
fn work_list_selection_edge_covers_content_rows_without_background_fill() {
    for theme in THEMES {
        for tab in [0, 2, 3] {
            let mut ui = mount(config(theme, tab));
            let keys: Vec<_> = ui
                .state()
                .visible_items()
                .iter()
                .take(2)
                .map(|item| {
                    format!(
                        "item-{}-{}-{}",
                        item.key.project,
                        item.key.kind.segment(),
                        item.key.iid
                    )
                })
                .collect();
            assert_eq!(keys.len(), 2);
            let content_rows = if tab == 0 { 3 } else { 2 };
            let (x, y) = point(&ui, &keys[0]);
            let background = ui.capture_frame().cell(x, y + content_rows).bg;
            for selected in 0..2 {
                ui.dispatch(Msg::Select(selected)).unwrap();
                settle(&mut ui);
                for hovered in [false, true] {
                    let (hx, hy) = if hovered {
                        point(&ui, &keys[selected])
                    } else {
                        (0, 0)
                    };
                    mouse(&mut ui, hx, hy, MouseKind::Moved);
                    let frame = ui.capture_frame();
                    for (index, key) in keys.iter().enumerate() {
                        let (x, y) = point(&ui, key);
                        for row in 0..content_rows {
                            let edge = frame.cell(x, y + row);
                            assert_eq!(edge.symbol, if index == selected { "▕" } else { " " });
                            assert_eq!(edge.bg, background, "{theme}, tab {tab}, row {row}");
                            if index == selected {
                                assert_ne!(frame.cell(x + 1, y + row).bg, background);
                                assert_eq!(edge.fg, frame.cell(x, y).fg);
                            }
                        }
                        assert_eq!(frame.cell(x, y + content_rows).symbol, " ");
                        assert_eq!(frame.cell(x, y + content_rows).bg, background);
                    }
                }
            }
        }
    }
}

#[test]
fn work_row_hover_is_not_selection_and_preserves_every_label_cell() {
    for theme in THEMES {
        let mut ui = mount(config(theme, 2));
        let selected = ui.state().scroll.selected;
        let items = ui.state().visible_items();
        let keys: Vec<_> = items
            .iter()
            .take(2)
            .map(|item| {
                format!(
                    "item-{}-{}-{}",
                    item.key.project,
                    item.key.kind.segment(),
                    item.key.iid
                )
            })
            .collect();
        let selected_point = point(&ui, &keys[0]);
        let selection_bg = ui
            .capture_frame()
            .cell(selected_point.0 + 1, selected_point.1)
            .bg;
        for key in keys {
            mouse(&mut ui, 0, 0, MouseKind::Moved);
            let (x, y) = point(&ui, &key);
            let before = ui.capture_frame();
            mouse(&mut ui, x, y, MouseKind::Moved);
            let hovered = ui.capture_frame();
            assert_eq!(hovered.cell(x, y), before.cell(x, y));
            assert_ne!(hovered.cell(x + 1, y).bg, before.cell(x + 1, y).bg);
            assert_ne!(
                hovered.cell(x + 1, y).bg,
                selection_bg,
                "hover is not a selection replacement"
            );
            for column in 1..ui.viewport().w - 2 {
                let cell = before.cell(column, y + 1);
                // Label pills have a background different from the row's background.
                if cell.bg != before.cell(1, y + 1).bg {
                    assert_eq!(hovered.cell(column, y + 1), cell, "label pill changed");
                }
                assert_eq!(hovered.cell(column, y).fg, before.cell(column, y).fg);
            }
            assert_eq!(ui.state().scroll.selected, selected);
            assert!(ui.state().config.route.is_none());
        }
    }
}

#[test]
fn project_details_contract_section_highlight_to_focused_fields() {
    for theme in THEMES {
        for animations in [false, true] {
            let mut cfg = config(theme, 1);
            cfg.animations = animations;
            cfg.project_route = Some(9001);
            let mut ui = mount(cfg);
            settle(&mut ui);
            assert_eq!(ui.state().scope, Scope::Details);
            let (x, start) = point(&ui, "detail-section-0");
            let frame = ui.capture_frame();
            let base = frame.cell(x, start + 1).bg; // gap-adjacent canvas under the edge column
            let selection = frame.cell(x + 1, start).bg;
            let accent = frame.cell(x, start).fg;
            assert_ne!(selection, frame.cell(x, start).bg, "{theme}");
            assert_eq!(frame.cell(x, start).symbol, "▕");
            assert_eq!(frame.cell(x + 1, start).bg, selection);

            ui.dispatch(Msg::Enter).unwrap();
            settle(&mut ui);
            assert_eq!(ui.state().scope, Scope::Section);
            assert_eq!(ui.state().config.field, 0);
            let focused = point(&ui, "edit-field-0");
            if animations {
                assert_eq!(ui.capture_frame().cell(x + 1, start).bg, selection);
                ui.advance(Duration::from_millis(80));
                settle(&mut ui);
                let middle = ui.capture_frame().cell(x + 1, start).bg;
                assert_ne!(middle, selection);
                assert_ne!(middle, base);
            }
            ui.advance(Duration::from_millis(200));
            settle(&mut ui);
            let frame = ui.capture_frame();
            assert_eq!(
                frame.cell(x + 1, start).bg,
                base,
                "{theme}: section fill must shrink"
            );
            assert_eq!(
                frame.cell(x, start).fg,
                base,
                "{theme}: section edge must clear"
            );
            assert_eq!(frame.cell(focused.0, focused.1).fg, accent);
            assert_eq!(frame.cell(focused.0, focused.1).bg, base);
            assert_eq!(frame.cell(focused.0 + 1, focused.1).bg, selection);
            assert_eq!(frame.cell(ui.viewport().w - 3, focused.1).bg, selection);

            ui.dispatch(Msg::Move(1)).unwrap();
            settle(&mut ui);
            let remove = point(&ui, "project-remove");
            let frame = ui.capture_frame();
            assert_eq!(frame.cell(remove.0, remove.1).fg, accent);
            assert_eq!(frame.cell(remove.0 + 1, remove.1).bg, selection);
            assert_eq!(frame.cell(focused.0 + 1, focused.1).bg, base);

            ui.dispatch(Msg::Back).unwrap();
            settle(&mut ui);
            ui.advance(Duration::from_millis(200));
            settle(&mut ui);
            assert_eq!(ui.state().scope, Scope::Details);
            let frame = ui.capture_frame();
            assert_eq!(
                frame.cell(x + 1, start).bg,
                selection,
                "{theme}: Esc restores broad fill"
            );
            assert_eq!(frame.cell(x, start).fg, accent);
        }
    }
}

#[test]
fn detail_sections_have_continuous_highlights_and_contract_to_focused_fields() {
    for theme in THEMES {
        for kind in [ItemKind::Issue, ItemKind::MergeRequest] {
            for animations in [false, true] {
                let mut cfg = config(theme, if kind == ItemKind::Issue { 2 } else { 3 });
                cfg.animations = animations;
                cfg.route = Some(ItemKey {
                    project: 9001,
                    iid: if kind == ItemKind::Issue { 1 } else { 101 },
                    kind,
                });
                let mut ui = mount(cfg);
                ui.set_viewport(Rect {
                    x: 0,
                    y: 0,
                    w: 110,
                    h: 400,
                });
                settle(&mut ui);
                let (x, start) = point(&ui, "detail-section-0");
                let (_, next) = point(&ui, "detail-section-1");
                assert_eq!(x, 0, "detail edges must align with list edges");
                let frame = ui.capture_frame();
                let base = frame.cell(x, next - 1).bg;
                let selection = frame.cell(x + 1, start).bg;
                let accent = frame.cell(x, start).fg;
                assert_ne!(selection, base);
                for column in 1..4 {
                    assert_eq!(frame.cell(x + column, start).symbol, " ");
                }
                assert_eq!(frame.cell(x + 4, start).symbol, "F");
                assert_eq!(frame.cell(ui.viewport().w - 3, start).symbol, "─");
                for y in start..next - 1 {
                    assert_eq!(frame.cell(x, y).symbol, "▕", "{theme} {kind:?} row {y}");
                    assert_eq!(frame.cell(x, y).bg, base);
                    assert_eq!(
                        frame.cell(ui.viewport().w - 3, y).bg,
                        selection,
                        "Fields highlight must not split at blank rows: {theme} {kind:?} row {y}"
                    );
                }
                assert_eq!(frame.cell(x, next).symbol, " ");
                assert_eq!(frame.cell(x, next).fg, base, "unselected edge is invisible");
                let (_, label_y) = point(&ui, "edit-field-2");
                let pills: Vec<_> = (1..ui.viewport().w - 2)
                    .filter_map(|x| {
                        let cell = frame.cell(x, label_y);
                        (cell.bg != selection && cell.bg != base).then(|| (x, cell.clone()))
                    })
                    .collect();
                assert!(!pills.is_empty());
                let original_offset = ui.state().content_offset;
                ui.dispatch(Msg::Enter).unwrap();
                settle(&mut ui);
                let focused = point(&ui, "edit-field-0");
                assert_eq!(focused.0, x);
                let initial = ui.capture_frame().cell(x + 1, start).bg;
                if animations {
                    assert_eq!(
                        initial, selection,
                        "focus transition should start with the broad fill"
                    );
                    ui.advance(Duration::from_millis(80));
                    settle(&mut ui);
                    let middle = ui.capture_frame().cell(x + 1, start).bg;
                    assert_ne!(middle, selection);
                    assert_ne!(middle, base);
                }
                ui.advance(Duration::from_millis(200));
                settle(&mut ui);
                let frame = ui.capture_frame();
                assert_eq!(frame.cell(x + 1, start).bg, base);
                assert_eq!(frame.cell(x, start).fg, base);
                assert_eq!(frame.cell(focused.0, focused.1).fg, accent);
                assert_eq!(frame.cell(focused.0, focused.1).bg, base);
                assert_eq!(frame.cell(focused.0 + 1, focused.1).bg, selection);
                assert_eq!(frame.cell(ui.viewport().w - 3, focused.1).bg, selection);
                for (x, pill) in pills {
                    assert_eq!(
                        frame.cell(x, label_y),
                        &pill,
                        "focus must preserve GitLab label colors"
                    );
                }
                ui.dispatch(Msg::Back).unwrap();
                settle(&mut ui);
                ui.advance(Duration::from_millis(200));
                settle(&mut ui);
                let expanded = ui.capture_frame();
                assert_eq!(expanded.cell(x, start).symbol, "▕");
                assert_eq!(expanded.cell(x, start).fg, accent);
                assert_eq!(expanded.cell(x + 1, start).bg, selection);
                assert_eq!(
                    ui.state().content_offset,
                    original_offset,
                    "focus animation must not change document geometry"
                );
            }
        }
    }
}

#[test]
fn detail_headers_fields_jobs_discussions_and_back_have_hover() {
    for theme in THEMES {
        let mut cfg = config(theme, 3);
        cfg.route = Some(ItemKey {
            project: 9001,
            iid: 101,
            kind: ItemKind::MergeRequest,
        });
        let mut ui = mount(cfg);
        assert_hover(&mut ui, "breadcrumb-back");
        assert_hover(&mut ui, "detail-section-0");
        assert_hover(&mut ui, "edit-field-0");
        ui.dispatch(Msg::Section(3)).unwrap();
        settle(&mut ui);
        let job = ui.state().details.as_ref().unwrap().jobs[0].id;
        assert_hover(&mut ui, &format!("job-{job}"));
        ui.dispatch(Msg::Section(4)).unwrap();
        settle(&mut ui);
        let discussion = ui.state().details.as_ref().unwrap().discussions[0]
            .id
            .clone();
        assert_hover(&mut ui, &format!("discussion-{discussion}"));
    }
}

#[test]
fn click_flash_expires_repeated_clicks_restart_it_and_reduced_motion_keeps_hover() {
    for animations in [true, false] {
        let mut cfg = config("midnight", 1);
        cfg.animations = animations;
        let mut ui = mount(cfg);
        let (x, y) = point(&ui, "workspace-tab-1");
        mouse(&mut ui, x, y, MouseKind::Moved);
        let resting = ui.capture_frame().cell(x, y).bg;
        for kind in [
            MouseKind::Down(MouseButton::Left),
            MouseKind::Up(MouseButton::Left),
        ] {
            mouse(&mut ui, x, y, kind);
        }
        let flashed = ui.capture_frame().cell(x, y).bg;
        assert_eq!(flashed != resting, animations);
        ui.advance(Duration::from_millis(100));
        settle(&mut ui);
        for kind in [
            MouseKind::Down(MouseButton::Left),
            MouseKind::Up(MouseButton::Left),
        ] {
            mouse(&mut ui, x, y, kind);
        }
        ui.advance(Duration::from_millis(60));
        settle(&mut ui);
        assert_eq!(
            ui.capture_frame().cell(x, y).bg,
            flashed,
            "first timer must not end the second flash"
        );
        ui.advance(Duration::from_millis(100));
        settle(&mut ui);
        assert_eq!(ui.capture_frame().cell(x, y).bg, resting);
        assert_hover(&mut ui, "workspace-tab-0");
    }
}

#[test]
fn scrollbar_hover_is_local_and_does_not_scroll_or_activate() {
    for theme in THEMES {
        let mut ui = mount(config(theme, 2));
        let rect = ui.rect_of_key(&"main-list-2".into()).unwrap();
        let x = rect.x as u16 + rect.w - 1;
        let y = rect.y as u16;
        let before = ui.capture_frame().cell(x, y).clone();
        let config = serde_json::to_value(&ui.state().config).unwrap();
        mouse(&mut ui, x, y, MouseKind::Moved);
        assert_ne!(ui.capture_frame().cell(x, y).bg, before.bg);
        assert_eq!(serde_json::to_value(&ui.state().config).unwrap(), config);
        mouse(&mut ui, x - 2, y, MouseKind::Moved);
        assert_eq!(ui.capture_frame().cell(x, y), &before);
        mouse(&mut ui, x, y, MouseKind::Moved);
        mouse(&mut ui, 0, 0, MouseKind::Moved);
        assert_eq!(ui.capture_frame().cell(x, y), &before);
    }
}

#[test]
fn closing_a_hovered_dialog_does_not_leave_stale_hover_when_reopened() {
    let mut ui = mount(config("midnight", 1));
    ui.dispatch(Msg::Action(Action::Themes)).unwrap();
    settle(&mut ui);
    let (x, y) = point(&ui, "dialog-option-0");
    let normal = ui.capture_frame().cell(x, y).bg;
    mouse(&mut ui, x, y, MouseKind::Moved);
    ui.dispatch(Msg::CloseDialog).unwrap();
    settle(&mut ui);
    mouse(&mut ui, 0, 0, MouseKind::Moved);
    ui.dispatch(Msg::Action(Action::Themes)).unwrap();
    settle(&mut ui);
    assert_eq!(ui.capture_frame().cell(x, y).bg, normal);
}
