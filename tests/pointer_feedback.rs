use std::time::Duration;

use cronk::{
    config::Config,
    demo,
    model::{ItemKey, ItemKind},
    ui::{Action, Cronk, Msg},
};
use tui_lipan::{
    TestBackend,
    core::event::{MouseButton, MouseKind},
    prelude::*,
};

type Ui = TestBackend<Cronk>;
const THEMES: [&str; 3] = ["midnight", "dracula", "light"];

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
    let (x, y) = point(ui, key);
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
            .cell(selected_point.0, selected_point.1)
            .bg;
        for key in keys {
            mouse(&mut ui, 0, 0, MouseKind::Moved);
            let (x, y) = point(&ui, &key);
            let before = ui.capture_frame();
            mouse(&mut ui, x, y, MouseKind::Moved);
            let hovered = ui.capture_frame();
            assert_ne!(hovered.cell(x, y).bg, before.cell(x, y).bg);
            assert_ne!(
                hovered.cell(x, y).bg,
                selection_bg,
                "hover is not a selection replacement"
            );
            for column in 0..ui.viewport().w - 2 {
                let cell = before.cell(column, y + 1);
                // Label pills have a background different from the row's background.
                if cell.bg != before.cell(0, y + 1).bg {
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
