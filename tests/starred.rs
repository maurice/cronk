//! Starred items: toggling, the right-aligned tab, ordering, and persistence.
use cronk::{
    config::Config,
    demo,
    model::{ItemKey, ItemKind},
    ui::{Cronk, DialogKind, Scope},
};
use tui_lipan::{
    TestBackend,
    core::event::{MouseButton, MouseKind},
    prelude::*,
};

type Ui = TestBackend<Cronk>;

fn mount(config: Config, path: Option<&std::path::Path>) -> Ui {
    if let Some(path) = path.filter(|p| !p.exists()) {
        config.save(path).unwrap();
    }
    let mut ui = TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config,
            path: path.map(std::path::Path::to_path_buf),
            api: None,
            demo: true,
        },
        (),
        Rect {
            x: 0,
            y: 0,
            w: 110,
            h: 40,
        },
    );
    settle(&mut ui);
    ui
}

fn config() -> Config {
    Config {
        projects: demo::projects(),
        onboarding: false,
        ..Config::default()
    }
}

fn settle(ui: &mut Ui) {
    for _ in 0..3 {
        ui.pump().unwrap();
        ui.render();
    }
}

fn press(ui: &mut Ui, code: KeyCode, mods: KeyMods) {
    ui.send_key(KeyEvent { code, mods }).unwrap();
    settle(ui);
}

fn key(ui: &mut Ui, code: KeyCode) {
    press(ui, code, KeyMods::NONE);
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

fn click_at(ui: &mut Ui, x: u16, y: u16) {
    mouse(ui, x, y, MouseKind::Moved);
    mouse(ui, x, y, MouseKind::Down(MouseButton::Left));
    mouse(ui, x, y, MouseKind::Up(MouseButton::Left));
}

fn text(ui: &Ui) -> String {
    ui.capture_frame().plain_text()
}

fn selected(ui: &Ui) -> ItemKey {
    let state = ui.state();
    state.visible_items()[state.scroll.selected].key.clone()
}

fn star(ui: &mut Ui) {
    key(ui, KeyCode::Char('s'));
}

#[test]
fn s_toggles_the_selected_item_and_the_starred_tab_follows() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    assert!(ui.state().config.starred_tab().is_none());
    assert!(!text(&ui).contains("Starred"));
    let first = selected(&ui);
    star(&mut ui);
    assert!(ui.state().config.is_starred(&first));
    assert_eq!(ui.state().config.starred.len(), 1);
    assert_eq!(ui.state().config.starred_tab(), Some(4));
    assert!(text(&ui).contains("★ Starred"));
    // The tab hugs the right edge of the tab strip.
    let rect = ui
        .rect_of_key(&"workspace-tab-4".to_owned().into())
        .unwrap();
    assert!(rect.x as u16 + rect.w >= 108, "{rect:?}");
    star(&mut ui);
    assert!(ui.state().config.starred.is_empty());
    assert!(!text(&ui).contains("Starred"));
}

#[test]
fn shift_s_opens_starred_and_reuses_the_details_view_and_breadcrumb() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    let first = selected(&ui);
    star(&mut ui);
    key(&mut ui, KeyCode::Down);
    let second = selected(&ui);
    star(&mut ui);
    key(&mut ui, KeyCode::Char('S'));
    assert_eq!(ui.state().config.active_tab, 4);
    assert_eq!(
        ui.state()
            .visible_items()
            .iter()
            .map(|i| i.key.clone())
            .collect::<Vec<_>>(),
        vec![first.clone(), second.clone()]
    );
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().scope, Scope::Details);
    assert_eq!(ui.state().config.route, Some(first));
    let screen = text(&ui);
    assert!(screen.contains("Starred  /"), "{screen}");
    assert!(screen.contains("Fields"), "{screen}");
    // Detail view shows the star, already starred, and `s` unstars it there.
    key(&mut ui, KeyCode::Char('s'));
    assert_eq!(ui.state().config.starred.len(), 1);
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.state().visible_items().len(), 1);
}

#[test]
fn shift_arrows_reorder_and_unstarring_the_last_item_removes_the_tab() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    let first = selected(&ui);
    star(&mut ui);
    key(&mut ui, KeyCode::Down);
    let second = selected(&ui);
    star(&mut ui);
    key(&mut ui, KeyCode::Char('S'));
    press(&mut ui, KeyCode::Down, KeyMods::SHIFT);
    assert_eq!(ui.state().scroll.selected, 1, "selection follows the item");
    let order: Vec<_> = ui.state().config.starred_keys();
    assert_eq!(order, vec![second.clone(), first.clone()]);
    press(&mut ui, KeyCode::Up, KeyMods::SHIFT);
    assert_eq!(ui.state().config.starred_keys(), vec![first, second]);
    // Edges do nothing.
    press(&mut ui, KeyCode::Up, KeyMods::SHIFT);
    assert_eq!(ui.state().scroll.selected, 0);
    star(&mut ui);
    star(&mut ui);
    assert!(ui.state().config.starred.is_empty());
    assert_eq!(ui.state().config.active_tab, 0, "leaves the vanished tab");
    assert_eq!(ui.state().scope, Scope::List);
}

#[test]
fn order_is_saved_in_the_portable_config_by_project_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    let mut ui = mount(config(), Some(&path));
    key(&mut ui, KeyCode::Char('I'));
    star(&mut ui);
    key(&mut ui, KeyCode::Down);
    star(&mut ui);
    key(&mut ui, KeyCode::Char('S'));
    press(&mut ui, KeyCode::Down, KeyMods::SHIFT);
    let saved = Config::load(&path).unwrap();
    assert_eq!(saved.starred, ui.state().config.starred);
    assert_eq!(saved.starred.len(), 2);
    assert!(saved.starred[0].project.contains('/') || !saved.starred[0].project.is_empty());
    let source = std::fs::read_to_string(&path).unwrap();
    assert!(source.contains("[[starred]]"), "{source}");
    // A fresh session (another machine) starts with the same tab and order.
    let ui = mount(saved.clone(), None);
    assert_eq!(ui.state().config.starred_keys(), saved.starred_keys());
    assert!(text(&ui).contains("★ Starred"));
}

#[test]
fn palette_finds_the_star_command_by_alias_and_applies_in_lists_and_details() {
    for query in ["star", "bookmark", "book", "favourite", "fav", "favorite"] {
        let mut ui = mount(config(), None);
        key(&mut ui, KeyCode::Char('I'));
        press(&mut ui, KeyCode::Char('p'), KeyMods::CTRL);
        assert!(matches!(
            ui.state().dialog.as_ref().unwrap().kind,
            DialogKind::Commands
        ));
        ui.send_paste(query).unwrap();
        settle(&mut ui);
        let options = ui.state().command_options();
        assert!(
            options
                .iter()
                .any(|(label, _)| label.starts_with("Star / unstar")),
            "{query}: {options:?}"
        );
        key(&mut ui, KeyCode::Enter);
        assert!(ui.state().dialog.is_none(), "{query}: ours should be first");
        assert_eq!(ui.state().config.starred.len(), 1, "{query}");
    }
}

#[test]
fn clicking_the_row_star_toggles_without_opening_details() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    let first = ui.state().visible_items()[0].key.clone();
    let id = format!("item-{}-{}-{}-star", first.project, "issues", first.iid);
    let id = if first.kind == ItemKind::MergeRequest {
        id.replace("issues", "merge_requests")
    } else {
        id
    };
    let rect = ui.rect_of_key(&id.clone().into()).expect("star cell");
    // Hover reveals a grey star; clicking stars it and does not navigate.
    mouse(&mut ui, rect.x as u16, rect.y as u16, MouseKind::Moved);
    assert!(text(&ui).contains('★'));
    click_at(&mut ui, rect.x as u16, rect.y as u16);
    assert!(ui.state().config.is_starred(&first));
    assert_eq!(ui.state().scope, Scope::List, "click must not navigate");
    assert_eq!(ui.state().config.active_tab, 2);
    click_at(&mut ui, rect.x as u16, rect.y as u16);
    assert!(!ui.state().config.is_starred(&first));
    assert_eq!(ui.state().scope, Scope::List);
}

#[test]
fn star_is_in_the_same_column_on_every_row_and_in_details() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    let items = ui.state().visible_items();
    let columns: Vec<_> = items
        .iter()
        .take(4)
        .map(|item| {
            let seg = item.key.kind.segment();
            let id = format!("item-{}-{}-{}-star", item.key.project, seg, item.key.iid);
            ui.rect_of_key(&id.into()).map(|r| r.x)
        })
        .collect();
    assert!(
        columns.iter().all(|c| c.is_some() && *c == columns[0]),
        "{columns:?}"
    );
    key(&mut ui, KeyCode::Enter);
    let detail = ui.rect_of_key(&"detail-star".to_owned().into()).unwrap();
    assert_eq!(
        detail.x + 3,
        ui.viewport().w as i16,
        "right edge with padding"
    );
    assert_eq!(
        Some(detail.x),
        columns[0],
        "same column in lists and details"
    );
}
