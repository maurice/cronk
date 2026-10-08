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
    let paths: Vec<_> = demo::projects().into_iter().map(|p| p.path).collect();
    assert!(
        saved.starred.iter().all(|s| paths.contains(&s.project)),
        "stored by project path: {:?}",
        saved.starred
    );
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
    assert_eq!(
        columns[0].map(|x| x + 4),
        Some(ui.viewport().w as i16),
        "two free cells between the star and the scrollbar/edge"
    );
    key(&mut ui, KeyCode::Enter);
    let detail = ui.rect_of_key(&"detail-star".to_owned().into()).unwrap();
    assert_eq!(
        detail.x + 4,
        ui.viewport().w as i16,
        "right edge with padding"
    );
    assert_eq!(
        Some(detail.x),
        columns[0],
        "same column in lists and details"
    );
}

fn star_two_issues(ui: &mut Ui) {
    key(ui, KeyCode::Char('I'));
    star(ui);
    key(ui, KeyCode::Down);
    star(ui);
}

#[test]
fn star_colors_and_tooltips_follow_state() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    let items = ui.state().visible_items();
    let (a, b) = (items[0].key.clone(), items[1].key.clone());
    let id = |k: &ItemKey| format!("item-{}-{}-{}-star", k.project, k.kind.segment(), k.iid);
    star(&mut ui);
    let rect = ui.rect_of_key(&id(&a).into()).unwrap();
    let cell = ui
        .capture_frame()
        .cell(rect.x as u16, rect.y as u16)
        .clone();
    assert_eq!(cell.symbol, "★");
    // Unstarred rows hide the star until hovered, and then show it grey.
    let other = ui.rect_of_key(&id(&b).into()).unwrap();
    let hidden = ui
        .capture_frame()
        .cell(other.x as u16, other.y as u16)
        .clone();
    assert_eq!(hidden.symbol, " ");
    mouse(&mut ui, other.x as u16, other.y as u16, MouseKind::Moved);
    let grey = ui
        .capture_frame()
        .cell(other.x as u16, other.y as u16)
        .clone();
    assert_eq!(grey.symbol, "★");
    assert_ne!(grey.fg, cell.fg, "starred is yellow, hover star is grey");
    assert!(
        text(&ui).contains("Click to add to ★ Starred"),
        "{}",
        text(&ui)
    );
    mouse(&mut ui, rect.x as u16, rect.y as u16, MouseKind::Moved);
    assert!(text(&ui).contains("Click to remove from ★ Starred"));
}

#[test]
fn digits_never_open_starred_and_rename_reports_an_error_there() {
    let mut ui = mount(config(), None);
    star_two_issues(&mut ui);
    assert_eq!(ui.state().config.starred_tab(), Some(4));
    key(&mut ui, KeyCode::Char('1'));
    assert_eq!(
        ui.state().config.active_tab,
        2,
        "digit 1 is not a saved view"
    );
    key(&mut ui, KeyCode::Char('S'));
    press(&mut ui, KeyCode::Char('p'), KeyMods::CTRL);
    ui.send_paste("rename").unwrap();
    key(&mut ui, KeyCode::Enter);
    let error = ui.state().error.clone().unwrap_or_default();
    assert!(error.contains("Starred tabs cannot be renamed"), "{error}");
}

#[test]
fn saved_views_and_the_starred_tab_keep_their_own_state_when_indices_shift() {
    let mut ui = mount(config(), None);
    star_two_issues(&mut ui);
    key(&mut ui, KeyCode::Char('S'));
    key(&mut ui, KeyCode::Char('/'));
    ui.send_paste("state:opened").unwrap();
    key(&mut ui, KeyCode::Enter);
    assert!(ui.state().config.filters.contains_key("4"));
    // Saving a view while Starred exists pushes Starred to index 5.
    key(&mut ui, KeyCode::Char('I'));
    press(&mut ui, KeyCode::Char('p'), KeyMods::CTRL);
    ui.send_paste("save filter").unwrap();
    key(&mut ui, KeyCode::Enter);
    ui.send_paste("Mine").unwrap();
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().config.views.len(), 1);
    assert_eq!(ui.state().config.active_tab, 4);
    assert_eq!(ui.state().config.starred_tab(), Some(5));
    assert!(
        !ui.state().config.filters.contains_key("4"),
        "the new view must not inherit Starred's filter"
    );
    assert_eq!(ui.state().query_text(), "");
    key(&mut ui, KeyCode::Char('S'));
    assert_eq!(ui.state().config.active_tab, 5);
    assert_eq!(ui.state().visible_items().len(), 2);
}

#[test]
fn hiding_the_project_removes_the_tab_but_keeps_the_dormant_star() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    star(&mut ui);
    let key0 = ui.state().config.starred_keys()[0].clone();
    key(&mut ui, KeyCode::Char('S'));
    key(&mut ui, KeyCode::Char('P'));
    let index = ui
        .state()
        .config
        .projects
        .iter()
        .position(|p| p.id == key0.project)
        .unwrap();
    for _ in 0..index {
        key(&mut ui, KeyCode::Down);
    }
    key(&mut ui, KeyCode::Char(' '));
    assert!(ui.state().config.starred_tab().is_none());
    assert_eq!(ui.state().config.starred.len(), 1, "star survives, dormant");
    assert!(!text(&ui).contains("Starred"));
    // Showing the project again brings the tab back.
    key(&mut ui, KeyCode::Char(' '));
    assert_eq!(ui.state().config.starred_tab(), Some(4));
}

#[test]
fn reordering_while_filtered_swaps_the_visible_neighbours() {
    let mut config = config();
    let items = demo::items();
    let projects = config.projects.clone();
    let project = |id: u64| projects.iter().find(|p| p.id == id).unwrap().clone();
    let a = items[0].key.clone();
    let other = items
        .iter()
        .find(|i| i.key.project != a.project && i.key.kind == a.kind)
        .unwrap()
        .key
        .clone();
    let c = items
        .iter()
        .find(|i| i.key.project == a.project && i.key.kind == a.kind && i.key != a)
        .unwrap()
        .key
        .clone();
    for key in [&a, &other, &c] {
        config.toggle_star(key, "");
        assert_eq!(
            config.starred.last().unwrap().project,
            project(key.project).path
        );
    }
    let path = project(a.project).path;
    let mut ui = mount(config, None);
    key(&mut ui, KeyCode::Char('S'));
    key(&mut ui, KeyCode::Char('/'));
    ui.send_paste(&format!("project:{path}")).unwrap();
    key(&mut ui, KeyCode::Enter);
    let shown: Vec<_> = ui
        .state()
        .visible_items()
        .iter()
        .map(|i| i.key.clone())
        .collect();
    assert_eq!(
        shown,
        vec![a.clone(), c.clone()],
        "the middle item is filtered out"
    );
    press(&mut ui, KeyCode::Down, KeyMods::SHIFT);
    assert_eq!(
        ui.state().scroll.selected,
        1,
        "cursor follows the moved item"
    );
    assert_eq!(ui.state().config.starred_keys(), vec![c, other, a]);
}

#[test]
fn unstarring_the_last_item_from_its_open_details_leaves_the_vanished_tab() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    star(&mut ui);
    key(&mut ui, KeyCode::Char('S'));
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().scope, Scope::Details);
    key(&mut ui, KeyCode::Char('s'));
    assert!(ui.state().config.starred.is_empty());
    assert_eq!(ui.state().config.active_tab, 0);
    assert_eq!(ui.state().scope, Scope::List);
}

#[test]
fn s_in_a_dialog_is_just_text() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    press(&mut ui, KeyCode::Char('p'), KeyMods::CTRL);
    key(&mut ui, KeyCode::Char('s'));
    assert!(ui.state().config.starred.is_empty());
}

/// Last column of row `y` whose background is the row's selection fill.
fn selection_end(ui: &Ui, y: u16) -> u16 {
    let frame = ui.capture_frame();
    let fill = frame.cell(2, y).bg;
    (0..frame.width)
        .rev()
        .find(|x| frame.cell(*x, y).bg == fill)
        .expect("selection fill")
}

fn scrollbar_column(ui: &Ui, key: &str) -> Option<u16> {
    let rect = ui
        .rect_of_key(&key.to_owned().into())
        .unwrap_or_else(|| panic!("missing pane {key}"));
    let frame = ui.capture_frame();
    let x = (rect.x + rect.w as i16 - 1) as u16;
    (0..rect.h)
        .any(|dy| frame.cell(x, rect.y as u16 + dy).symbol == "█")
        .then_some(x)
}

/// Blank cells between the selection fill and the scrollbar track, or the pane edge.
fn right_gap(ui: &Ui, pane: &str, y: u16) -> u16 {
    let end = selection_end(ui, y);
    let limit = scrollbar_column(ui, pane).unwrap_or(ui.viewport().w);
    limit - end - 1
}

#[test]
fn lists_keep_a_one_cell_gap_before_the_scrollbar_or_the_pane_edge() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    let y = ui
        .rect_of_key(&"main-list-2-scrollbar-pointer".to_owned().into())
        .unwrap()
        .y as u16;
    assert!(
        scrollbar_column(&ui, "main-list-2-scrollbar-pointer").is_some(),
        "long list scrolls"
    );
    assert_eq!(
        right_gap(&ui, "main-list-2-scrollbar-pointer", y),
        1,
        "with a scrollbar"
    );
    star(&mut ui);
    key(&mut ui, KeyCode::Char('S'));
    assert!(scrollbar_column(&ui, "main-list-4-scrollbar-pointer").is_none());
    assert_eq!(
        right_gap(&ui, "main-list-4-scrollbar-pointer", y),
        1,
        "without a scrollbar"
    );
    // The Projects list is the same widget and follows the same rule.
    key(&mut ui, KeyCode::Char('P'));
    let y = ui
        .rect_of_key(&"main-list-1-scrollbar-pointer".to_owned().into())
        .unwrap()
        .y as u16;
    assert_eq!(
        right_gap(&ui, "main-list-1-scrollbar-pointer", y),
        1,
        "projects"
    );
}

#[test]
fn details_keep_a_one_cell_gap_before_the_scrollbar_or_the_pane_edge() {
    let section = |ui: &Ui| {
        ui.rect_of_key(&"detail-section-0".to_owned().into())
            .unwrap()
            .y as u16
    };
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    key(&mut ui, KeyCode::Enter);
    assert!(scrollbar_column(&ui, "detail-scrollbar-pointer").is_some());
    assert_eq!(right_gap(&ui, "detail-scrollbar-pointer", section(&ui)), 1);
    // A very tall terminal makes the document fit, removing the scrollbar.
    let mut tall = mount_tall();
    key(&mut tall, KeyCode::Char('I'));
    key(&mut tall, KeyCode::Enter);
    assert!(scrollbar_column(&tall, "detail-scrollbar-pointer").is_none());
    assert_eq!(
        right_gap(&tall, "detail-scrollbar-pointer", section(&tall)),
        1
    );
    // Project details are short, so they have no scrollbar either.
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('P'));
    key(&mut ui, KeyCode::Enter);
    assert!(scrollbar_column(&ui, "detail-scrollbar-pointer").is_none());
    assert_eq!(right_gap(&ui, "detail-scrollbar-pointer", section(&ui)), 1);
}

fn mount_tall() -> Ui {
    let mut ui = TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config: config(),
            path: None,
            api: None,
            demo: true,
        },
        (),
        Rect {
            x: 0,
            y: 0,
            w: 110,
            h: 200,
        },
    );
    settle(&mut ui);
    ui
}
