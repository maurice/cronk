use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Duration, SystemTime},
};

use cronk::{
    build_info,
    config::{Config, SavedView, TabState, UserDisplay},
    demo,
    model::{CurrentIteration, ItemKey, ItemKind, Mutation, TraceChunk, User},
    ui::{Confirmation, Cronk, Dialog, DialogKind, Msg, Scope, State},
};
use tui_lipan::{
    TestBackend,
    core::event::{MouseButton, MouseKind},
    prelude::*,
};

type Ui = TestBackend<Cronk>;

fn config() -> Config {
    Config {
        projects: demo::projects(),
        ..Config::default()
    }
}

fn mount(config: Config, path: Option<&Path>) -> Ui {
    mount_with_viewport(
        config,
        path,
        Rect {
            x: 0,
            y: 0,
            w: 80,
            h: 24,
        },
    )
}

fn mount_with_viewport(config: Config, path: Option<&Path>, viewport: Rect) -> Ui {
    let app = App::new().focus_policy(FocusPolicy::Manual);
    let mut ui = TestBackend::new_with_app_and_viewport(
        app,
        Cronk {
            config,
            path: path.map(Path::to_path_buf),
            api: None,
        },
        (),
        viewport,
    );
    ui.pump().unwrap();
    ui.render();
    ui
}

fn settle_layout(ui: &mut Ui) {
    // Rendering queues viewport callbacks; drain them before checking persisted scroll offsets.
    ui.render();
    ui.pump().unwrap();
}

fn key(ui: &mut Ui, code: KeyCode) {
    modified(ui, code, KeyMods::NONE);
}

fn modified(ui: &mut Ui, code: KeyCode, mods: KeyMods) {
    ui.send_key(KeyEvent { code, mods }).unwrap();
}

fn type_text(ui: &mut Ui, text: &str) {
    for ch in text.chars() {
        key(ui, KeyCode::Char(ch));
    }
}

fn dialog(ui: &Ui) -> &Dialog {
    ui.state().dialog.as_ref().expect("dialog should be open")
}

fn focused_field(ui: &Ui, index: usize) {
    assert_eq!(
        ui.focused_key(),
        Some(&format!("dialog-field-{index}").into()),
        "the actual input, not just the dialog selection, must have focus"
    );
}

fn replace_input(ui: &mut Ui, text: &str) {
    key(ui, KeyCode::Home);
    modified(ui, KeyCode::End, KeyMods::SHIFT);
    ui.send_paste(text).unwrap();
}

fn palette_command(ui: &mut Ui, command: &str) {
    modified(ui, KeyCode::Char('p'), KeyMods::CTRL);
    assert!(matches!(dialog(ui).kind, DialogKind::Commands));
    focused_field(ui, 0);
    ui.send_paste(command).unwrap();
    assert_eq!(ui.state().command_options().len(), 1, "{command}");
    key(ui, KeyCode::Enter);
}

fn selected_key(state: &State) -> ItemKey {
    state.visible_items()[state.scroll.selected].key.clone()
}

fn persisted(ui: &Ui, path: &Path) -> Config {
    assert!(path.is_file(), "the event must save before shutdown");
    let saved = Config::load(path).unwrap();
    assert_eq!(
        serde_json::to_value(&saved).unwrap(),
        serde_json::to_value(&ui.state().config).unwrap(),
        "disk must reflect the complete committed workspace"
    );
    saved
}

fn tab_state(state: &State) -> TabState {
    TabState {
        route: state.config.route.clone(),
        section: state.config.section,
        field: state.config.field,
        section_cursor: state.section_cursor,
        list_offset: state.scroll.offset,
        content_offset: state.content_offset,
        expanded: state.expanded.iter().copied().collect(),
        collapsed: state.collapsed.iter().copied().collect(),
    }
}

/// The state a drilled-down tab collapses to when its own tab is selected again.
fn popped_to_list(expected: &TabState) -> TabState {
    if expected.route.is_none() {
        return expected.clone();
    }
    TabState {
        list_offset: expected.list_offset,
        ..TabState::default()
    }
}

fn assert_tab_state(ui: &Ui, expected: &TabState, selected: usize) {
    let state = ui.state();
    assert_eq!(
        state.config.route,
        expected.route,
        "{} route",
        state.tab_name()
    );
    assert_eq!(
        state.config.section,
        expected.section,
        "{} section",
        state.tab_name()
    );
    assert_eq!(
        state.config.field,
        expected.field,
        "{} field",
        state.tab_name()
    );
    assert_eq!(
        state.section_cursor,
        expected.section_cursor,
        "{} section cursor",
        state.tab_name()
    );
    assert_eq!(
        state.scroll.selected,
        selected,
        "{} list selection",
        state.tab_name()
    );
    assert_eq!(
        state.scroll.offset,
        expected.list_offset,
        "{} list offset",
        state.tab_name()
    );
    assert_eq!(
        state.content_offset,
        expected.content_offset,
        "{} content offset",
        state.tab_name()
    );
    assert_eq!(
        state.expanded.iter().copied().collect::<BTreeSet<_>>(),
        expected.expanded
    );
    assert_eq!(
        state.scope,
        match (&expected.route, expected.section) {
            (None, _) => Scope::List,
            (Some(_), None) => Scope::Details,
            (Some(_), Some(_)) => Scope::Section,
        },
        "{} scope",
        state.tab_name()
    );
    assert_eq!(
        state.details.as_ref().map(|d| &d.item.key),
        expected.route.as_ref()
    );
    assert!(state.dialog.is_none());
    assert_eq!(ui.focused_key(), None);
}

fn persisted_tab(ui: &Ui, path: &Path) -> Config {
    let saved = persisted(ui, path);
    let tab = ui.state().tab_key();
    assert_eq!(saved.selections[&tab], ui.state().scroll.selected);
    assert_eq!(
        serde_json::to_value(&saved.tab_states[&tab]).unwrap(),
        serde_json::to_value(tab_state(ui.state())).unwrap(),
        "the active tab's complete navigation must be saved on each committed event"
    );
    saved
}

fn open_section(ui: &mut Ui, name: &str) {
    assert_eq!(ui.state().scope, Scope::Details);
    let index = ui
        .state()
        .sections()
        .iter()
        .position(|s| *s == name)
        .unwrap();
    let cursor = ui.state().section_cursor;
    let code = if cursor < index {
        KeyCode::Down
    } else {
        KeyCode::Up
    };
    for _ in 0..cursor.abs_diff(index) {
        key(ui, code);
    }
    assert_eq!(ui.state().section_cursor, index);
    key(ui, KeyCode::Enter);
    assert_eq!(ui.state().scope, Scope::Section);
    assert_eq!(ui.state().section_name(), name);
}

// Find real rendered cells rather than assuming widget geometry or dispatching a click message.
fn click_text(ui: &mut Ui, text: &str, occurrence: usize) {
    ui.render();
    let frame = ui.capture_frame();
    let (x, y) = frame
        .to_lines()
        .iter()
        .enumerate()
        .flat_map(|(y, line)| {
            line.match_indices(text)
                .map(move |(byte, _)| (line[..byte].chars().count() as u16, y as u16))
        })
        .nth(occurrence)
        .unwrap_or_else(|| panic!("missing click target {text:?}:\n{}", frame.plain_text()));
    for kind in [
        MouseKind::Down(MouseButton::Left),
        MouseKind::Up(MouseButton::Left),
    ] {
        ui.send_mouse(MouseEvent {
            x,
            y,
            kind,
            mods: KeyMods::NONE,
        })
        .unwrap();
    }
}

fn numbered_views_config(count: usize) -> Config {
    Config {
        views: (1..=count)
            .map(|index| SavedView {
                name: format!("Queue{index:02}"),
                kind: ItemKind::Issue,
                query: String::new(),
            })
            .collect(),
        ..config()
    }
}

fn modifier_combinations() -> impl Iterator<Item = KeyMods> {
    (0..16).map(|bits| KeyMods {
        ctrl: bits & 1 != 0,
        alt: bits & 2 != 0,
        shift: bits & 4 != 0,
        super_key: bits & 8 != 0,
    })
}

fn active_list_responds(ui: &mut Ui) {
    assert_eq!(ui.state().scope, Scope::List);
    assert!(ui.state().config.route.is_none());
    assert!(ui.state().config.section.is_none());
    assert!(ui.state().details.is_none());
    assert_eq!(ui.focused_key(), None);
    let tab = ui.state().config.active_tab;
    key(ui, KeyCode::Home);
    assert_eq!(ui.state().scroll.selected, 0);
    key(ui, KeyCode::Down);
    assert_eq!(ui.state().scroll.selected, 1);
    key(ui, KeyCode::Up);
    assert_eq!(ui.state().scroll.selected, 0);
    key(ui, KeyCode::Tab);
    assert_eq!(ui.state().scroll.selected, 1);
    modified(ui, KeyCode::Tab, KeyMods::SHIFT);
    assert_eq!(ui.state().scroll.selected, 0);
    assert_eq!(ui.state().config.active_tab, tab);
    assert_eq!(ui.state().scope, Scope::List);
    assert_eq!(ui.focused_key(), None);
}

#[test]
fn builtin_tab_shortcuts_accept_all_terminal_shift_encodings() {
    let mut ui = mount(numbered_views_config(1), None);
    for (index, letter, name) in [
        (0, 'D', "Dashboard"),
        (1, 'P', "Projects"),
        (2, 'I', "Issues"),
        (3, 'M', "Merge Requests"),
    ] {
        for (ch, mods) in [
            (letter, KeyMods::NONE),
            (letter, KeyMods::SHIFT),
            (letter.to_ascii_lowercase(), KeyMods::SHIFT),
        ] {
            key(&mut ui, KeyCode::Char('1'));
            assert_eq!(ui.state().config.active_tab, 4);
            modified(&mut ui, KeyCode::Char(ch), mods);
            assert_eq!(ui.state().config.active_tab, index, "{ch:?} {mods:?}");
            assert_eq!(ui.state().tab_name(), name);
            active_list_responds(&mut ui);
        }
    }
}

#[test]
fn lowercase_and_control_alt_super_modified_shortcuts_do_not_switch_tabs() {
    let mut config = numbered_views_config(10);
    config.active_tab = 4;
    let mut ui = mount(config, None);
    type_text(&mut ui, "dpim");
    assert_eq!(ui.state().config.active_tab, 4);
    assert_eq!(ui.state().scope, Scope::List);
    assert!(ui.state().dialog.is_none());

    for mods in modifier_combinations().filter(|mods| mods.ctrl || mods.alt || mods.super_key) {
        for ch in "DPIMdpim1234567890".chars() {
            modified(&mut ui, KeyCode::Char(ch), mods);
            assert_eq!(ui.state().config.active_tab, 4, "{ch:?} {mods:?}");
            assert_eq!(ui.state().scope, Scope::List);
            if ui.state().dialog.is_some() {
                assert_eq!((ch, mods), ('p', KeyMods::CTRL));
                assert!(matches!(dialog(&ui).kind, DialogKind::Commands));
                key(&mut ui, KeyCode::Esc);
            }
        }
    }
}

#[test]
fn modified_horizontal_arrows_never_switch_tabs_in_any_scope() {
    let mut ui = mount(numbered_views_config(2), None);
    key(&mut ui, KeyCode::Char('I'));
    key(&mut ui, KeyCode::Tab);
    for scope in [Scope::List, Scope::Details, Scope::Section] {
        settle_layout(&mut ui);
        assert_eq!(ui.state().scope, scope);
        let before = serde_json::to_value(&ui.state().config).unwrap();
        let scroll = ui.state().scroll.clone();
        for mods in modifier_combinations().filter(|mods| *mods != KeyMods::NONE) {
            for code in [KeyCode::Left, KeyCode::Right] {
                modified(&mut ui, code, mods);
                settle_layout(&mut ui);
                assert_eq!(ui.state().scope, scope, "{code:?} {mods:?}");
                assert_eq!(serde_json::to_value(&ui.state().config).unwrap(), before);
                assert_eq!(ui.state().scroll, scroll);
            }
        }
        if scope != Scope::Section {
            key(&mut ui, KeyCode::Enter);
        }
    }
}

#[test]
fn horizontal_arrows_switch_tabs_from_every_scope_and_reach_unnumbered_views() {
    for depth in 0..=2 {
        for (arrow, expected) in [(KeyCode::Left, 1), (KeyCode::Right, 3)] {
            let mut ui = mount(numbered_views_config(12), None);
            key(&mut ui, KeyCode::Char('I'));
            for _ in 0..depth {
                key(&mut ui, KeyCode::Enter);
            }
            key(&mut ui, KeyCode::Down);
            settle_layout(&mut ui);
            let navigation = tab_state(ui.state());
            let selected = ui.state().scroll.selected;
            key(&mut ui, arrow);
            assert_eq!(ui.state().config.active_tab, expected);
            active_list_responds(&mut ui);
            key(
                &mut ui,
                if arrow == KeyCode::Left {
                    KeyCode::Right
                } else {
                    KeyCode::Left
                },
            );
            settle_layout(&mut ui);
            assert_eq!(ui.state().config.active_tab, 2);
            assert_tab_state(&ui, &navigation, selected);
        }
    }
    let mut ui = mount(numbered_views_config(12), None);
    key(&mut ui, KeyCode::Left);
    assert_eq!(ui.state().config.active_tab, 0);
    for index in 1..16 {
        key(&mut ui, KeyCode::Right);
        assert_eq!(ui.state().config.active_tab, index);
    }
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Down);
    settle_layout(&mut ui);
    let navigation = tab_state(ui.state());
    let selected = ui.state().scroll.selected;
    let epoch = ui.state().detail_epoch;
    key(&mut ui, KeyCode::Right);
    settle_layout(&mut ui);
    assert_eq!(ui.state().config.active_tab, 15);
    assert_tab_state(&ui, &navigation, selected);
    assert_eq!(ui.state().detail_epoch, epoch);
    key(&mut ui, KeyCode::Left);
    assert_eq!(ui.state().config.active_tab, 14);
    active_list_responds(&mut ui);
    key(&mut ui, KeyCode::Right);
    settle_layout(&mut ui);
    assert_eq!(ui.state().config.active_tab, 15);
    assert_tab_state(&ui, &navigation, selected);
}

#[test]
fn horizontal_arrows_edit_dialog_text_without_switching_tabs() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('/'));
    type_text(&mut ui, "abc");
    key(&mut ui, KeyCode::Left);
    type_text(&mut ui, "X");
    key(&mut ui, KeyCode::Right);
    type_text(&mut ui, "Y");
    assert_eq!(dialog(&ui).fields[0].value(), "abXcY");
    assert_eq!(ui.state().config.active_tab, 0);
}

#[test]
fn escape_is_a_noop_and_keeps_every_list_active() {
    let mut ui = mount(numbered_views_config(1), None);
    for ch in "DPIM1".chars() {
        key(&mut ui, KeyCode::Char(ch));
        key(&mut ui, KeyCode::Down);
        assert_eq!(ui.state().scroll.selected, 1);
        let before = serde_json::to_value(&ui.state().config).unwrap();
        let scroll = ui.state().scroll.clone();
        for _ in 0..3 {
            key(&mut ui, KeyCode::Esc);
            assert_eq!(ui.state().scope, Scope::List);
            assert_eq!(serde_json::to_value(&ui.state().config).unwrap(), before);
            assert_eq!(ui.state().scroll, scroll);
        }
        active_list_responds(&mut ui);
    }
}

#[test]
fn all_ten_saved_numeric_shortcuts_map_to_indices_four_through_thirteen() {
    // Extra views must neither steal a digit nor make 0 wrap to the first view.
    let mut config = numbered_views_config(12);
    config.active_tab = 15;
    let mut ui = mount(config, None);
    assert_eq!(ui.state().tab_name(), "Queue12");
    for (index, ch) in "1234567890".chars().enumerate() {
        key(&mut ui, KeyCode::Char(ch));
        assert_eq!(ui.state().config.active_tab, index + 4, "key {ch}");
        assert_eq!(ui.state().tab_name(), format!("Queue{:02}", index + 1));
        active_list_responds(&mut ui);
    }
}

#[test]
fn missing_saved_view_numbers_preserve_the_current_list_detail_and_section() {
    for count in 0..10 {
        let mut config = numbered_views_config(count);
        config.active_tab = 2;
        let mut ui = mount(config, None);
        key(&mut ui, KeyCode::Tab);
        for scope in [Scope::List, Scope::Details, Scope::Section] {
            settle_layout(&mut ui);
            assert_eq!(ui.state().scope, scope);
            let before = serde_json::to_value(&ui.state().config).unwrap();
            let scroll = ui.state().scroll.clone();
            let epoch = ui.state().detail_epoch;
            for ch in "1234567890".chars().skip(count) {
                key(&mut ui, KeyCode::Char(ch));
                settle_layout(&mut ui);
                assert_eq!(ui.state().scope, scope, "key {ch}, {count} saved views");
                assert_eq!(serde_json::to_value(&ui.state().config).unwrap(), before);
                assert_eq!(ui.state().scroll, scroll);
                assert_eq!(ui.state().detail_epoch, epoch);
                if scope != Scope::List {
                    assert_eq!(
                        ui.state().details.as_ref().unwrap().item.key,
                        ui.state().config.route.clone().unwrap()
                    );
                }
            }
            if scope != Scope::Section {
                key(&mut ui, KeyCode::Enter);
            }
        }
    }
}

#[test]
fn keyboard_tab_switches_restore_each_tabs_list_details_and_section() {
    for depth in 0..=2 {
        let mut ui = mount(numbered_views_config(10), None);
        let mut visits = Vec::new();
        for (index, ch) in "DPIM1234567890".chars().enumerate() {
            key(&mut ui, KeyCode::Char(ch));
            assert_eq!(ui.state().config.active_tab, index, "key {ch}");
            active_list_responds(&mut ui);
            for _ in 0..=index % 3 {
                key(&mut ui, KeyCode::Down);
            }
            // Projects is a list whose Enter action opens a filtered Issues tab.
            if index != 1 && depth > 0 {
                key(&mut ui, KeyCode::Enter);
                if depth == 2 {
                    key(&mut ui, KeyCode::Enter);
                }
                key(&mut ui, KeyCode::Down);
            }
            settle_layout(&mut ui);
            visits.push((tab_state(ui.state()), ui.state().scroll.selected));
        }
        for (index, ch) in "DPIM1234567890"
            .chars()
            .collect::<Vec<_>>()
            .into_iter()
            .enumerate()
            .rev()
        {
            // The last visited tab is already active, so its first press pops it.
            let already_active = ui.state().config.active_tab == index;
            key(&mut ui, KeyCode::Char(ch));
            settle_layout(&mut ui);
            assert_eq!(ui.state().config.active_tab, index);
            let (expected, selected) = &visits[index];
            let popped = popped_to_list(expected);
            assert_tab_state(
                &ui,
                if already_active { &popped } else { expected },
                *selected,
            );
            let shortcuts = if ch.is_ascii_uppercase() {
                vec![
                    (ch, KeyMods::NONE),
                    (ch, KeyMods::SHIFT),
                    (ch.to_ascii_lowercase(), KeyMods::SHIFT),
                ]
            } else {
                vec![(ch, KeyMods::NONE)]
            };
            for (ch, mods) in shortcuts {
                modified(&mut ui, KeyCode::Char(ch), mods);
                settle_layout(&mut ui);
                // The first press pops a drilled-down tab to its list; further presses are no-ops.
                assert_tab_state(&ui, &popped, *selected);
                assert_eq!(ui.state().scope == Scope::List, popped.route.is_none());
                assert!(ui.state().config.route.is_none() || expected.route.is_none());
            }
        }
    }
}

#[test]
fn mouse_tab_switches_restore_each_tabs_list_details_and_section() {
    let labels = [
        "Dashboard",
        "Projects",
        "Issues",
        "Merge Requests",
        "1 Queue01",
        "2 Queue02",
    ];
    for depth in 0..=2 {
        let mut ui = mount(numbered_views_config(2), None);
        ui.set_viewport(Rect {
            x: 0,
            y: 0,
            w: 160,
            h: 30,
        });
        let mut visits = Vec::new();
        for (index, label) in labels.iter().enumerate() {
            // The first occurrence is in the header, not the breadcrumb below it.
            click_text(&mut ui, label, 0);
            assert_eq!(ui.state().config.active_tab, index, "clicked {label}");
            active_list_responds(&mut ui);
            key(&mut ui, KeyCode::Down);
            if index != 1 && depth > 0 {
                key(&mut ui, KeyCode::Enter);
                if depth == 2 {
                    key(&mut ui, KeyCode::Enter);
                }
                key(&mut ui, KeyCode::Down);
            }
            settle_layout(&mut ui);
            visits.push((tab_state(ui.state()), ui.state().scroll.selected));
        }
        for (index, label) in labels.iter().enumerate().rev() {
            let already_active = ui.state().config.active_tab == index;
            click_text(&mut ui, label, 0);
            settle_layout(&mut ui);
            assert_eq!(ui.state().config.active_tab, index);
            let (expected, selected) = &visits[index];
            let popped = popped_to_list(expected);
            assert_tab_state(
                &ui,
                if already_active { &popped } else { expected },
                *selected,
            );
            click_text(&mut ui, label, 0);
            settle_layout(&mut ui);
            assert_tab_state(&ui, &popped, *selected);
            assert_eq!(ui.state().scope == Scope::List, popped.route.is_none());
            // Clicking again from the list changes nothing.
            click_text(&mut ui, label, 0);
            settle_layout(&mut ui);
            assert_tab_state(&ui, &popped, *selected);
        }
    }
}

#[test]
fn dialogs_and_palette_type_direct_shortcuts_without_switching_tabs() {
    for palette in [false, true] {
        let mut config = numbered_views_config(10);
        config.active_tab = 4;
        let mut ui = mount(config, None);
        let field = if palette {
            modified(&mut ui, KeyCode::Char('p'), KeyMods::CTRL);
            assert!(matches!(dialog(&ui).kind, DialogKind::Commands));
            0
        } else {
            key(&mut ui, KeyCode::Char('n'));
            assert!(matches!(dialog(&ui).kind, DialogKind::NewIssue));
            key(&mut ui, KeyCode::Tab);
            1
        };
        let mut expected = String::new();
        for (text, mods) in [
            ("DPIM1234567890", KeyMods::NONE),
            ("DPIM", KeyMods::SHIFT),
            ("dpim", KeyMods::SHIFT),
        ] {
            for ch in text.chars() {
                modified(&mut ui, KeyCode::Char(ch), mods);
                expected.push(ch);
                assert_eq!(dialog(&ui).fields[field].value(), expected);
                assert_eq!(ui.state().config.active_tab, 4);
                assert_eq!(ui.state().scope, Scope::List);
                focused_field(&ui, field);
            }
        }
        key(&mut ui, KeyCode::Esc);
        active_list_responds(&mut ui);
    }
}

#[test]
fn header_shows_build_revision_on_the_right() {
    let ui = mount(config(), None);
    let header = ui.capture_frame().to_lines()[0].clone();
    assert!(
        header.contains(&build_info::display_version()),
        "missing build version in header: {header}"
    );
}

#[test]
fn builtin_tab_headers_underline_the_actual_first_letter_and_footer_shows_shortcuts() {
    let mut ui = mount(config(), None);
    ui.set_viewport(Rect {
        x: 0,
        y: 0,
        w: 180,
        h: 24,
    });
    for ch in "DPIM".chars() {
        key(&mut ui, KeyCode::Char(ch));
        ui.render();
        let frame = ui.capture_frame();
        let lines = frame.to_lines();
        let (y, header) = lines
            .iter()
            .enumerate()
            .find(|(_, line)| line.contains("Dashboard") && line.contains("Merge Requests"))
            .expect("all built-in tab labels should fit in the header");
        for name in ["Dashboard", "Projects", "Issues", "Merge Requests"] {
            let byte = header.find(name).unwrap();
            let x = header[..byte].chars().count();
            for (offset, letter) in name.chars().enumerate() {
                let cell = &frame.cells[y * usize::from(frame.width) + x + offset];
                assert_eq!(cell.symbol, letter.to_string());
                assert_eq!(
                    cell.modifiers.underline.is_some(),
                    offset == 0,
                    "only the first letter of {name} should be underlined, active tab {ch}"
                );
            }
        }
        let footer = lines.last().unwrap();
        assert!(footer.contains("D/P/I/M"), "{footer}");
        assert!(footer.contains("1–0"), "{footer}");
        assert!(!footer.contains("Esc tabs"), "{footer}");
        assert!(!frame.plain_text().contains("‹ Back"));
    }
}

#[test]
fn saved_tab_headers_hint_only_the_first_ten_views_and_extra_views_remain_clickable() {
    let mut ui = mount(numbered_views_config(12), None);
    ui.set_viewport(Rect {
        x: 0,
        y: 0,
        w: 240,
        h: 24,
    });
    ui.render();
    let frame = ui.capture_frame();
    let lines = frame.to_lines();
    let header = lines
        .iter()
        .find(|line| line.contains("Dashboard") && line.contains("Queue12"))
        .expect("all saved tab labels should fit in the header");
    for (index, ch) in "1234567890".chars().enumerate() {
        let label = format!("{ch} Queue{:02}", index + 1);
        assert!(header.contains(&label), "missing {label:?}: {header}");
    }
    for (name, previous) in [("Queue11", "Queue10"), ("Queue12", "Queue11")] {
        let start = header.find(name).unwrap();
        assert!(
            header[..start].trim_end().ends_with(previous),
            "extra tabs must have no numeric prefix: {header}"
        );
    }
    for (index, name) in [(14, "Queue11"), (15, "Queue12")] {
        click_text(&mut ui, name, 0);
        assert_eq!(ui.state().config.active_tab, index);
        active_list_responds(&mut ui);
        for (saved, ch) in "1234567890".chars().enumerate() {
            key(&mut ui, KeyCode::Char(ch));
            assert_eq!(ui.state().config.active_tab, saved + 4, "key {ch}");
            assert_ne!(
                ui.state().tab_name(),
                name,
                "extra views must not alias a digit"
            );
        }
    }
}

#[test]
fn narrow_tab_strip_reveals_the_tenth_saved_view_and_returns_to_dashboard() {
    for width in [24, 40] {
        let mut ui = mount(numbered_views_config(12), None);
        ui.set_viewport(Rect {
            x: 0,
            y: 0,
            w: width,
            h: 14,
        });
        ui.render();
        let tabs = ui.rect_of_key(&"workspace-tabs".into()).unwrap();
        let row = usize::try_from(tabs.y).unwrap();
        let header = ui.capture_frame().to_lines()[row].clone();
        assert!(header.contains("Dashboard"), "width {width}: {header}");
        assert!(
            !header.contains("Queue10"),
            "the tenth view must start offscreen"
        );

        for _ in 0..2 {
            key(&mut ui, KeyCode::Char('0'));
            assert_eq!(ui.state().config.active_tab, 13);
            active_list_responds(&mut ui);
            ui.render();
            let header = ui.capture_frame().to_lines()[row].clone();
            assert!(header.contains("0 Queue10"), "width {width}: {header}");
            assert!(!header.contains("Dashboard"), "width {width}: {header}");

            key(&mut ui, KeyCode::Char('D'));
            assert_eq!(ui.state().config.active_tab, 0);
            active_list_responds(&mut ui);
            ui.render();
            let header = ui.capture_frame().to_lines()[row].clone();
            assert!(header.contains("Dashboard"), "width {width}: {header}");
            assert!(!header.contains("Queue10"), "width {width}: {header}");
        }
    }
}

#[test]
fn narrow_tab_strip_wheel_scrolls_without_switching_and_reaches_unnumbered_views() {
    let mut ui = mount(numbered_views_config(12), None);
    ui.set_viewport(Rect {
        x: 0,
        y: 0,
        w: 40,
        h: 14,
    });
    key(&mut ui, KeyCode::Down);
    ui.render();
    let tabs = ui.rect_of_key(&"workspace-tabs".into()).unwrap();
    let row = usize::try_from(tabs.y).unwrap();
    let before = serde_json::to_value(&ui.state().config).unwrap();
    let scroll = ui.state().scroll.clone();
    assert_eq!(ui.state().config.active_tab, 0);
    assert_eq!(scroll.selected, 1);
    let header = ui.capture_frame().to_lines()[row].clone();
    assert!(header.contains("Dashboard"), "{header}");
    assert!(!header.contains("Queue11") && !header.contains("Queue12"));

    for (kind, target) in [
        (MouseKind::ScrollDown, "Queue12"),
        (MouseKind::ScrollUp, "Dashboard"),
        (MouseKind::ScrollDown, "Queue11"),
    ] {
        // Bound the search without depending on the widget's wheel step size.
        for _ in 0..256 {
            ui.render();
            if ui.capture_frame().to_lines()[row].contains(target) {
                break;
            }
            ui.send_mouse(MouseEvent {
                x: u16::try_from(tabs.x).unwrap() + tabs.w / 2,
                y: u16::try_from(tabs.y).unwrap(),
                kind,
                mods: KeyMods::NONE,
            })
            .unwrap();
            assert_eq!(serde_json::to_value(&ui.state().config).unwrap(), before);
            assert_eq!(
                ui.state().scroll,
                scroll,
                "tab-strip scrolling must not move the list"
            );
            assert_eq!(ui.state().scope, Scope::List);
            assert_eq!(ui.focused_key(), None);
        }
        ui.render();
        let header = ui.capture_frame().to_lines()[row].clone();
        assert!(
            header.contains(target),
            "wheel {kind:?} did not reveal {target}: {header}"
        );
    }

    click_text(&mut ui, "Queue11", 0);
    assert_eq!(ui.state().config.active_tab, 14);
    active_list_responds(&mut ui);
    key(&mut ui, KeyCode::Char('D'));
    assert_eq!(ui.state().config.active_tab, 0);
    assert_eq!(ui.state().scroll, scroll);
    ui.render();
    let header = ui.capture_frame().to_lines()[row].clone();
    assert!(header.contains("Dashboard"), "{header}");
}

#[test]
fn tab_and_both_shift_tab_encodings_move_the_list_not_widget_focus() {
    let mut ui = mount(config(), None);
    assert_eq!(ui.state().scope, Scope::List);
    assert_eq!(ui.focused_key(), None);
    let first = selected_key(ui.state());
    key(&mut ui, KeyCode::Tab);
    assert_eq!(ui.state().scroll.selected, 1);
    assert_ne!(selected_key(ui.state()), first);
    assert_eq!(ui.state().config.active_tab, 0);
    assert_eq!(ui.focused_key(), None);
    modified(&mut ui, KeyCode::Tab, KeyMods::SHIFT);
    assert_eq!(ui.state().scroll.selected, 0);
    key(&mut ui, KeyCode::Tab);
    modified(&mut ui, KeyCode::BackTab, KeyMods::SHIFT);
    assert_eq!(selected_key(ui.state()), first);

    key(&mut ui, KeyCode::End);
    let last = ui.state().visible_items().len() - 1;
    assert_eq!(ui.state().scroll.selected, last);
    assert!(ui.state().scroll.offset > 0);
    key(&mut ui, KeyCode::Tab);
    assert_eq!(ui.state().scroll.selected, last, "selection must not wrap");
    ui.render();
    assert!(ui.capture_frame().plain_text().contains('▕'));
    key(&mut ui, KeyCode::Home);
    assert_eq!(
        (ui.state().scroll.selected, ui.state().scroll.offset),
        (0, 0)
    );

    key(&mut ui, KeyCode::Tab);
    key(&mut ui, KeyCode::Char('P'));
    assert_eq!(ui.state().config.active_tab, 1);
    key(&mut ui, KeyCode::Char('D'));
    assert_eq!(
        ui.state().scroll.selected,
        1,
        "tab switches restore each list's cursor"
    );
}

#[test]
fn enter_and_escape_traverse_scopes_and_cancel_a_focused_field_edit() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    key(&mut ui, KeyCode::Tab);
    let selected = selected_key(ui.state());
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().scope, Scope::Details);
    assert_eq!(ui.state().config.route.as_ref(), Some(&selected));
    let original = ui.state().details.as_ref().unwrap().item.title.clone();
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().scope, Scope::Section);
    assert_eq!(ui.state().config.section, Some(0));
    assert_eq!(ui.state().section_name(), "Fields");
    key(&mut ui, KeyCode::Enter);
    assert!(
        matches!(&dialog(&ui).kind, DialogKind::Edit(key, field) if key == &selected && field == "title")
    );
    focused_field(&ui, 0);
    replace_input(&mut ui, "Draft λ, not a remote change");
    assert_eq!(
        dialog(&ui).fields[0].value(),
        "Draft λ, not a remote change"
    );
    key(&mut ui, KeyCode::Esc);
    assert!(ui.state().dialog.is_none());
    assert_eq!(ui.state().scope, Scope::Section);
    assert_eq!(ui.state().details.as_ref().unwrap().item.title, original);
    assert!(!ui.state().mutation_pending);
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.state().scope, Scope::Details);
    assert_eq!(ui.state().config.section, None);
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.state().scope, Scope::List);
    assert!(ui.state().config.route.is_none());
    assert_eq!(selected_key(ui.state()), selected);
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.state().scope, Scope::List);
    assert_eq!(ui.state().config.active_tab, 2);
    assert_eq!(selected_key(ui.state()), selected);
    key(&mut ui, KeyCode::Down);
    assert_eq!(ui.state().scroll.selected, 2);
    key(&mut ui, KeyCode::Char('P'));
    assert_eq!(ui.state().scope, Scope::List);
    assert_eq!(ui.state().config.active_tab, 1);
    key(&mut ui, KeyCode::Tab);
    assert_eq!(ui.state().scroll.selected, 1);
}

#[test]
fn form_tab_cycles_real_focus_and_multiline_typing_does_not_trigger_shortcuts() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    let before = ui.state().items.clone();
    key(&mut ui, KeyCode::Char('n'));
    assert!(matches!(dialog(&ui).kind, DialogKind::NewIssue));
    focused_field(&ui, 0);
    let project = dialog(&ui).fields[0].value().to_owned();
    key(&mut ui, KeyCode::Tab);
    focused_field(&ui, 1);
    type_text(&mut ui, "jkr:/123");
    ui.send_paste(" λ").unwrap();
    assert_eq!(dialog(&ui).fields[1].value(), "jkr:/123 λ");
    assert_eq!(ui.state().config.active_tab, 2);
    key(&mut ui, KeyCode::Tab);
    focused_field(&ui, 2);
    ui.send_paste("first line").unwrap();
    modified(&mut ui, KeyCode::Char('j'), KeyMods::CTRL);
    type_text(&mut ui, "second DPIM1234567890");
    assert_eq!(
        dialog(&ui).fields[2].value(),
        "first line\nsecond DPIM1234567890"
    );
    assert_eq!(ui.state().config.active_tab, 2);
    key(&mut ui, KeyCode::Tab);
    focused_field(&ui, 0);
    modified(&mut ui, KeyCode::BackTab, KeyMods::SHIFT);
    focused_field(&ui, 2);
    modified(&mut ui, KeyCode::Tab, KeyMods::SHIFT);
    focused_field(&ui, 1);
    assert_eq!(dialog(&ui).fields[0].value(), project);
    assert_eq!(dialog(&ui).fields[1].value(), "jkr:/123 λ");
    key(&mut ui, KeyCode::Enter);
    assert!(dialog(&ui).error.as_deref().unwrap().contains("read-only"));
    assert_eq!(
        dialog(&ui).fields[2].value(),
        "first line\nsecond DPIM1234567890"
    );
    assert_eq!(ui.state().items, before);
    key(&mut ui, KeyCode::Esc);
    assert!(ui.state().dialog.is_none());
    key(&mut ui, KeyCode::Tab);
    assert_eq!(
        ui.state().scroll.selected,
        1,
        "closing the form releases focus"
    );
}

#[test]
fn palette_theme_and_project_rows_share_selection_edges() {
    let mut config = config();
    config.colors.background = Some("#010203".into());
    config.colors.surface = Some("#111213".into());
    config.colors.selection = Some("#212223".into());
    config.colors.accent = Some("#515253".into());
    let mut ui = mount(config, None);
    key(&mut ui, KeyCode::Char('P'));
    let project = ui.state().config.projects[0].id;
    let rect = ui
        .rect_of_key(&format!("project-{project}").into())
        .unwrap();
    let frame = ui.capture_frame();
    let x = u16::try_from(rect.x).unwrap();
    let top = u16::try_from(rect.y).unwrap();
    for y in top..top + 2 {
        assert_eq!(frame.cell(x, y).symbol, "▕");
        assert_eq!(frame.cell(x, y).bg, Color::hex_u24(0x010203));
        assert_eq!(frame.cell(x + rect.w - 2, y).bg, Color::hex_u24(0x212223));
    }
    assert_eq!(frame.cell(x, top + 2).symbol, " ");

    modified(&mut ui, KeyCode::Char('p'), KeyMods::CTRL);
    ui.send_paste("theme").unwrap();
    for themes in [false, true] {
        if themes {
            key(&mut ui, KeyCode::Enter);
            assert!(matches!(dialog(&ui).kind, DialogKind::Themes));
        }
        for selected in 0..=usize::from(themes) {
            if selected == 1 {
                key(&mut ui, KeyCode::Down);
            }
            let rect = ui
                .rect_of_key(&format!("dialog-option-{selected}").into())
                .unwrap();
            let frame = ui.capture_frame();
            let x = u16::try_from(rect.x).unwrap();
            let y = u16::try_from(rect.y).unwrap();
            assert_eq!(frame.cell(x, y).symbol, "▕");
            assert_eq!(frame.cell(x, y).fg, Color::hex_u24(0x515253));
            assert_eq!(frame.cell(x, y).bg, Color::hex_u24(0x111213));
            assert_eq!(frame.cell(x + rect.w - 2, y).bg, Color::hex_u24(0x212223));
            assert!(!frame.to_lines()[rect.y as usize].contains('›'));
        }
    }
}

#[test]
fn theme_picker_starts_on_active_theme_previews_navigation_and_escape_reverts() {
    let mut config = config();
    config.theme = "dracula".into();
    let mut ui = mount(config, None);

    palette_command(&mut ui, "Choose theme");
    assert!(matches!(dialog(&ui).kind, DialogKind::Themes));
    assert_eq!(dialog(&ui).selected, 1);
    assert_eq!(ui.state().config.theme, "dracula");

    key(&mut ui, KeyCode::Down);
    assert_eq!(dialog(&ui).selected, 2);
    assert_eq!(ui.state().config.theme, "light");

    key(&mut ui, KeyCode::Esc);
    assert!(ui.state().dialog.is_none());
    assert_eq!(ui.state().config.theme, "dracula");
}

#[test]
fn theme_picker_enter_commits_live_preview_and_mouse_selection_does_not_submit() {
    let mut ui = mount(config(), None);
    palette_command(&mut ui, "Choose theme");

    key(&mut ui, KeyCode::Down);
    assert_eq!(ui.state().config.theme, "dracula");
    key(&mut ui, KeyCode::Enter);
    assert!(ui.state().dialog.is_none());
    assert_eq!(ui.state().config.theme, "dracula");

    palette_command(&mut ui, "Choose theme");
    click_text(&mut ui, "Light", 0);
    assert!(matches!(dialog(&ui).kind, DialogKind::Themes));
    assert_eq!(dialog(&ui).selected, 2);
    assert_eq!(ui.state().config.theme, "light");
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.state().config.theme, "dracula");
}

#[test]
fn palette_arrow_navigation_keeps_query_focus_and_runs_the_filtered_selection() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('P'));
    key(&mut ui, KeyCode::Tab);
    modified(&mut ui, KeyCode::Char('p'), KeyMods::CTRL);
    assert!(matches!(dialog(&ui).kind, DialogKind::Commands));
    focused_field(&ui, 0);
    type_text(&mut ui, "project");
    assert_eq!(
        dialog(&ui).fields[0].value(),
        "project",
        "j must be typed, not navigate"
    );
    assert_eq!(ui.state().command_options().len(), 4);
    key(&mut ui, KeyCode::Down);
    assert_eq!(
        dialog(&ui).selected,
        1,
        "Down must reach palette navigation through Input"
    );
    focused_field(&ui, 0);
    key(&mut ui, KeyCode::Down);
    assert_eq!(dialog(&ui).selected, 2);
    key(&mut ui, KeyCode::Up);
    assert_eq!(
        dialog(&ui).selected,
        1,
        "Up must move one option, not reset the query selection"
    );
    ui.send_paste(" alias").unwrap();
    assert_eq!(dialog(&ui).fields[0].value(), "project alias");
    assert_eq!(
        dialog(&ui).selected,
        0,
        "editing the query resets the option cursor"
    );
    assert_eq!(ui.state().command_options().len(), 1);
    key(&mut ui, KeyCode::Enter);
    assert!(
        matches!(dialog(&ui).kind, DialogKind::AliasProject(id) if id == demo::projects()[1].id)
    );
    focused_field(&ui, 0);
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.state().scroll.selected, 1);
}

#[test]
fn palette_tab_navigation_and_caret_movement_preserve_the_selected_option() {
    let mut ui = mount(config(), None);
    modified(&mut ui, KeyCode::Char('p'), KeyMods::CTRL);
    ui.send_paste("tab").unwrap();
    assert_eq!(ui.state().command_options().len(), 3);
    key(&mut ui, KeyCode::Tab);
    assert_eq!(dialog(&ui).selected, 1);
    focused_field(&ui, 0);
    key(&mut ui, KeyCode::Tab);
    assert_eq!(dialog(&ui).selected, 2);
    modified(&mut ui, KeyCode::BackTab, KeyMods::SHIFT);
    assert_eq!(dialog(&ui).selected, 1);
    key(&mut ui, KeyCode::Left);
    assert_eq!(dialog(&ui).fields[0].value(), "tab");
    assert_eq!(
        dialog(&ui).selected,
        1,
        "moving the caret is not a query edit"
    );
    modified(&mut ui, KeyCode::Tab, KeyMods::SHIFT);
    assert_eq!(dialog(&ui).selected, 0);
    key(&mut ui, KeyCode::Esc);
    assert!(ui.state().dialog.is_none());
    key(&mut ui, KeyCode::Tab);
    assert_eq!(ui.state().scroll.selected, 1);
}

#[test]
fn keyboard_filter_applies_on_enter_and_invalid_drafts_cancel_without_losing_the_list() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    key(&mut ui, KeyCode::End);
    key(&mut ui, KeyCode::Char('/'));
    focused_field(&ui, 0);
    ui.send_paste("state:opened").unwrap();
    key(&mut ui, KeyCode::Enter);
    assert!(ui.state().dialog.is_none());
    assert_eq!(ui.state().query_text(), "state:opened");
    assert_eq!(ui.state().scroll.selected, 0);
    let filtered: Vec<_> = ui.state().visible_items().into_iter().cloned().collect();
    assert!(!filtered.is_empty());
    assert!(
        filtered
            .iter()
            .all(|i| i.key.kind == ItemKind::Issue && i.state == "opened")
    );
    key(&mut ui, KeyCode::Char('/'));
    replace_input(&mut ui, "label:\"");
    key(&mut ui, KeyCode::Enter);
    assert!(dialog(&ui).error.is_some());
    assert_eq!(dialog(&ui).fields[0].value(), "label:\"");
    assert_eq!(ui.state().query_text(), "state:opened");
    key(&mut ui, KeyCode::Esc);
    assert_eq!(
        ui.state()
            .visible_items()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>(),
        filtered
    );
}

#[test]
fn saved_views_can_keep_iteration_current_symbolic_and_resolve_per_project() {
    let mut config = config();
    config.views = vec![SavedView {
        name: "Current sprint".into(),
        kind: ItemKind::Issue,
        query: "iteration:Current".into(),
    }];
    config.active_tab = 4;
    let mut ui = mount(config, None);
    ui.state_mut().current_iterations = [(
        9001,
        Some(CurrentIteration {
            id: 1200,
            title: "Demo iteration 12 · predictable concurrency".into(),
            description: "Demo iteration 12 · predictable concurrency · Demo · ID 1200".into(),
        }),
    )]
    .into();
    assert_eq!(ui.state().query_text(), "iteration:Current");
    let visible = ui.state().visible_items();
    assert!(!visible.is_empty());
    assert!(visible.iter().all(|item| item.key.project == 9001));
}

#[test]
fn save_and_rename_a_view_through_focused_keyboard_dialogs() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    key(&mut ui, KeyCode::Char('/'));
    ui.send_paste("state:opened").unwrap();
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Char('s'));
    ui.send_paste("Open issues").unwrap();
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().config.active_tab, 4);
    assert_eq!(ui.state().config.views.len(), 1);
    assert_eq!(ui.state().config.views[0].query, "state:opened");
    assert_eq!(ui.state().config.views[0].kind, ItemKind::Issue);
    ui.render();
    assert!(ui.capture_frame().plain_text().contains("1 Open issues"));
    key(&mut ui, KeyCode::Tab);
    let selected = selected_key(ui.state());
    palette_command(&mut ui, "Rename saved tab");
    assert!(matches!(dialog(&ui).kind, DialogKind::RenameView));
    replace_input(&mut ui, "My queue");
    key(&mut ui, KeyCode::Enter);
    assert!(ui.state().dialog.is_none());
    assert_eq!(ui.state().tab_name(), "My queue");
    assert_eq!(ui.state().query_text(), "state:opened");
    assert_eq!(selected_key(ui.state()), selected);
    ui.render();
    assert!(ui.capture_frame().plain_text().contains("1 My queue"));
}

fn saved_views_config() -> Config {
    let mut config = config();
    config.views = ["First", "Middle", "Last"]
        .map(|name| SavedView {
            name: name.into(),
            kind: ItemKind::Issue,
            query: String::new(),
        })
        .into();
    config.filters = [
        ("2", "state:opened"),
        ("4", "state:opened"),
        ("5", "state:closed"),
        ("6", "state:opened"),
    ]
    .map(|(k, v)| (k.into(), v.into()))
    .into();
    config.selections = [("2", 3), ("4", 1), ("5", 2), ("6", 7)]
        .map(|(k, v)| (k.into(), v))
        .into();
    config
}

#[test]
fn deleting_a_middle_saved_view_reindexes_filters_and_selections_without_overwriting_its_neighbor()
{
    let mut config = saved_views_config();
    config.active_tab = 5;
    let mut ui = mount(config, None);
    palette_command(&mut ui, "Delete saved tab");
    assert!(matches!(
        dialog(&ui).kind,
        DialogKind::Confirm(Confirmation::DeleteView(1))
    ));
    click_text(&mut ui, " Close ", 0);
    assert!(ui.state().dialog.is_none());
    assert_eq!(
        ui.state().config.views.len(),
        3,
        "cancel must leave the view intact"
    );
    palette_command(&mut ui, "Delete saved tab");
    click_text(&mut ui, " Confirm ", 0);
    assert!(ui.state().dialog.is_none());
    assert_eq!(ui.state().config.active_tab, 0);
    assert_eq!(
        ui.state()
            .config
            .views
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>(),
        ["First", "Last"]
    );
    assert_eq!(ui.state().config.filters["5"], "state:opened");
    assert!(!ui.state().config.filters.contains_key("6"));
    assert_eq!(ui.state().config.selections["2"], 3);
    assert_eq!(ui.state().config.selections["4"], 1);
    assert!(!ui.state().config.selections.contains_key("6"));
    assert_eq!(
        ui.state().config.selections["5"],
        7,
        "the removed tab's cursor must not overwrite the shifted Last tab"
    );
    key(&mut ui, KeyCode::Char('2'));
    assert_eq!(ui.state().tab_name(), "Last");
    assert_eq!(ui.state().scroll.selected, 7);
    assert_eq!(ui.state().query_text(), "state:opened");
}

#[test]
fn deleting_a_saved_view_reindexes_navigation_and_caches_without_overwriting_its_neighbor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let mut config = saved_views_config();
    config.views[2].kind = ItemKind::MergeRequest;
    // The cached and reloaded job panels must both overflow at the same viewport size.
    let viewport = Rect {
        x: 0,
        y: 0,
        w: 80,
        h: 14,
    };
    let mut ui = mount_with_viewport(config, Some(&path), viewport);
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Down);
    settle_layout(&mut ui);
    let dashboard = tab_state(ui.state());
    let dashboard_selected = ui.state().scroll.selected;

    key(&mut ui, KeyCode::Char('1'));
    key(&mut ui, KeyCode::Enter);
    open_section(&mut ui, "Fields");
    key(&mut ui, KeyCode::Down);
    key(&mut ui, KeyCode::Down);
    settle_layout(&mut ui);
    let first = tab_state(ui.state());
    let first_selected = ui.state().scroll.selected;
    key(&mut ui, KeyCode::Char('3'));
    key(&mut ui, KeyCode::Enter);
    cache_finished_job(&mut ui, "Last view cached detail");
    key(&mut ui, KeyCode::Down);
    ui.dispatch(Msg::ContentScroll(4)).unwrap();
    settle_layout(&mut ui);
    assert_eq!(
        ui.state().content_offset,
        4,
        "the fixture must support a real scroll offset"
    );
    let last = tab_state(ui.state());
    let last_selected = ui.state().scroll.selected;
    let last_traces = traces(ui.state());
    assert!(last.list_offset > 0);
    assert_eq!(last.field, 1);
    persisted_tab(&ui, &path);

    key(&mut ui, KeyCode::Char('2'));
    key(&mut ui, KeyCode::Enter);
    open_section(&mut ui, "Fields");
    key(&mut ui, KeyCode::Down);
    assert_ne!(ui.state().config.route, last.route);
    settle_layout(&mut ui);
    let before = persisted_tab(&ui, &path);
    palette_command(&mut ui, "Delete saved tab");
    assert!(matches!(
        dialog(&ui).kind,
        DialogKind::Confirm(Confirmation::DeleteView(1))
    ));
    key(&mut ui, KeyCode::Esc);
    settle_layout(&mut ui);
    assert_eq!(
        serde_json::to_value(persisted(&ui, &path)).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    palette_command(&mut ui, "Delete saved tab");
    key(&mut ui, KeyCode::Enter);
    settle_layout(&mut ui);
    assert_eq!(ui.state().config.active_tab, 0);
    assert_tab_state(&ui, &dashboard, dashboard_selected);
    let saved = persisted_tab(&ui, &path);
    assert_eq!(
        saved
            .views
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>(),
        ["First", "Last"]
    );
    assert_eq!(saved.filters["5"], "state:opened");
    assert_eq!(saved.selections["5"], last_selected);
    assert_eq!(saved.selections["4"], first_selected);
    assert_eq!(
        serde_json::to_value(&saved.tab_states["5"]).unwrap(),
        serde_json::to_value(&last).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&saved.tab_states["4"]).unwrap(),
        serde_json::to_value(&first).unwrap()
    );
    assert!(!saved.filters.contains_key("6"));
    assert!(!saved.selections.contains_key("6"));
    assert!(!saved.tab_states.contains_key("6"));

    key(&mut ui, KeyCode::Char('2'));
    settle_layout(&mut ui);
    assert_eq!(ui.state().tab_name(), "Last");
    assert_tab_state(&ui, &last, last_selected);
    assert_eq!(
        ui.state().details.as_ref().unwrap().item.title,
        "Last view cached detail"
    );
    assert_eq!(traces(ui.state()), last_traces);
    key(&mut ui, KeyCode::Char('1'));
    settle_layout(&mut ui);
    assert_tab_state(&ui, &first, first_selected);
    persisted_tab(&ui, &path);
    drop(ui);

    let mut restarted = mount_with_viewport(Config::load(&path).unwrap(), Some(&path), viewport);
    settle_layout(&mut restarted);
    assert_tab_state(&restarted, &first, first_selected);
    key(&mut restarted, KeyCode::Char('2'));
    settle_layout(&mut restarted);
    assert_tab_state(&restarted, &last, last_selected);
    assert!(
        !persisted_tab(&restarted, &path)
            .tab_states
            .contains_key("6")
    );
}

#[test]
fn deleting_the_last_saved_view_does_not_resurrect_its_selection_key() {
    let mut config = saved_views_config();
    config.active_tab = 6;
    let mut ui = mount(config, None);
    key(&mut ui, KeyCode::Enter);
    open_section(&mut ui, "Fields");
    key(&mut ui, KeyCode::Down);
    assert!(ui.state().config.tab_states.contains_key("6"));
    palette_command(&mut ui, "Delete saved tab");
    click_text(&mut ui, " Confirm ", 0);
    assert_eq!(ui.state().config.views.len(), 2);
    assert!(!ui.state().config.filters.contains_key("6"));
    assert!(
        !ui.state().config.selections.contains_key("6"),
        "switching away must not recreate the deleted tab's cursor"
    );
    assert!(!ui.state().config.tab_states.contains_key("6"));
    key(&mut ui, KeyCode::Char('1'));
    key(&mut ui, KeyCode::Char('D'));
    assert!(!ui.state().config.tab_states.contains_key("6"));
}

#[test]
fn escape_cancels_a_fieldless_confirmation_dialog() {
    let mut config = saved_views_config();
    config.active_tab = 4;
    let mut ui = mount(config, None);
    palette_command(&mut ui, "Delete saved tab");
    assert!(matches!(dialog(&ui).kind, DialogKind::Confirm(_)));
    type_text(&mut ui, "DPIM1234567890");
    assert!(matches!(dialog(&ui).kind, DialogKind::Confirm(_)));
    assert_eq!(ui.state().config.active_tab, 4);
    key(&mut ui, KeyCode::Esc);
    assert!(
        ui.state().dialog.is_none(),
        "Esc must bubble from the modal to Cronk when there is no input"
    );
    assert_eq!(ui.state().config.views.len(), 3);
    assert_eq!(ui.state().config.active_tab, 4);
}

#[test]
fn enter_submits_a_fieldless_confirmation_dialog() {
    let mut config = saved_views_config();
    config.active_tab = 4;
    let mut ui = mount(config, None);
    palette_command(&mut ui, "Delete saved tab");
    assert!(matches!(dialog(&ui).kind, DialogKind::Confirm(_)));
    key(&mut ui, KeyCode::Enter);
    assert!(
        ui.state().dialog.is_none(),
        "Enter must submit even when there is no input interceptor"
    );
    assert_eq!(ui.state().config.views.len(), 2);
    assert_eq!(ui.state().config.active_tab, 0);
}

#[test]
fn mouse_project_checkbox_toggles_the_clicked_project_without_activating_the_row() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('P'));
    click_text(&mut ui, "[x]", 1);
    assert_eq!(
        ui.state().config.active_tab,
        1,
        "checkbox clicks must not bubble into project activation"
    );
    assert_eq!(ui.state().scope, Scope::List);
    assert_eq!(ui.state().scroll.selected, 1);
    assert!(ui.state().config.projects[0].visible);
    assert!(!ui.state().config.projects[1].visible);
    assert!(ui.state().config.route.is_none());
    let hidden = ui.state().config.projects[1].id;
    key(&mut ui, KeyCode::Char('I'));
    assert!(
        ui.state()
            .visible_items()
            .iter()
            .all(|i| i.key.project != hidden)
    );
    key(&mut ui, KeyCode::Char('P'));
    key(&mut ui, KeyCode::Char(' '));
    assert!(ui.state().config.projects[1].visible);
}

#[test]
fn mouse_project_and_work_rows_activate_the_clicked_identity() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('P'));
    click_text(&mut ui, "Meteor cache", 0);
    assert_eq!(ui.state().config.active_tab, 2);
    let project = &demo::projects()[1];
    assert_eq!(
        ui.state().query_text(),
        format!("project:\"{}\"", project.path)
    );
    assert!(!ui.state().visible_items().is_empty());
    assert!(
        ui.state()
            .visible_items()
            .iter()
            .all(|i| i.key.project == project.id)
    );
    let clicked = ui.state().visible_items()[2].key.clone();
    click_text(&mut ui, &format!("#{}", clicked.iid), 0);
    assert_eq!(ui.state().scope, Scope::Details);
    assert_eq!(ui.state().config.route.as_ref(), Some(&clicked));
    assert_eq!(ui.state().details.as_ref().unwrap().item.key, clicked);
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.state().scroll.selected, 2);
}

#[test]
fn hiding_or_removing_a_project_invalidates_all_cached_tab_routes() {
    for remove in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.toml");
        let mut config = numbered_views_config(2);
        config.views[0].kind = ItemKind::MergeRequest;
        let project = config.projects[0].id;
        let mut ui = mount(config, Some(&path));
        for ch in ['M', '1'] {
            key(&mut ui, KeyCode::Char(ch));
            key(&mut ui, KeyCode::Enter);
            assert_eq!(ui.state().config.route.as_ref().unwrap().project, project);
            cache_finished_job(&mut ui, "Detail from a project being hidden");
            persisted_tab(&ui, &path);
        }
        key(&mut ui, KeyCode::Char('2'));
        let visible = ui
            .state()
            .visible_items()
            .iter()
            .position(|i| i.key.project != project)
            .unwrap();
        for _ in 0..visible {
            key(&mut ui, KeyCode::Down);
        }
        key(&mut ui, KeyCode::Enter);
        open_section(&mut ui, "Fields");
        key(&mut ui, KeyCode::Down);
        let valid_route = ui.state().config.route.clone().unwrap();
        key(&mut ui, KeyCode::Char('P'));
        key(&mut ui, KeyCode::Home);
        assert_eq!(
            ui.state().config.projects[ui.state().scroll.selected].id,
            project
        );
        if remove {
            palette_command(&mut ui, "Remove project from workspace");
            assert!(
                matches!(dialog(&ui).kind, DialogKind::Confirm(Confirmation::RemoveProject(id)) if id == project)
            );
            key(&mut ui, KeyCode::Enter);
            assert!(ui.state().project(project).is_none());
        } else {
            key(&mut ui, KeyCode::Char(' '));
            assert!(!ui.state().project(project).unwrap().visible);
        }
        let saved = persisted_tab(&ui, &path);
        assert!(saved.tab_states.values().all(|tab| {
            tab.route
                .as_ref()
                .is_none_or(|route| route.project != project)
        }));
        assert_eq!(saved.tab_states["5"].route.as_ref(), Some(&valid_route));
        for ch in ['M', '1'] {
            key(&mut ui, KeyCode::Char(ch));
            active_list_responds(&mut ui);
            assert!(ui.state().traces.is_empty());
            assert!(ui.state().expanded.is_empty());
            assert_eq!(ui.state().content_offset, 0);
            assert!(
                ui.state()
                    .visible_items()
                    .iter()
                    .all(|i| i.key.project != project)
            );
            persisted_tab(&ui, &path);
        }
        drop(ui);
        let mut restarted = mount(Config::load(&path).unwrap(), Some(&path));
        active_list_responds(&mut restarted);
        key(&mut restarted, KeyCode::Char('2'));
        assert_eq!(restarted.state().config.route.as_ref(), Some(&valid_route));
        assert_eq!(restarted.state().scope, Scope::Section);
        assert_eq!(restarted.state().config.field, 1);
        if !remove {
            key(&mut restarted, KeyCode::Char('P'));
            key(&mut restarted, KeyCode::Home);
            key(&mut restarted, KeyCode::Char(' '));
            assert!(restarted.state().project(project).unwrap().visible);
            for ch in ['M', '1'] {
                key(&mut restarted, KeyCode::Char(ch));
                assert_eq!(restarted.state().scope, Scope::List);
                assert!(
                    restarted.state().config.route.is_none(),
                    "showing a project must not resurrect invalidated routes"
                );
                assert!(restarted.state().details.is_none());
                assert!(restarted.state().traces.is_empty());
                assert!(restarted.state().expanded.is_empty());
            }
        }
    }
}

#[test]
fn restart_clears_active_and_inactive_routes_for_hidden_or_removed_projects() {
    for remove in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.toml");
        let mut config = numbered_views_config(2);
        config.views[0].kind = ItemKind::MergeRequest;
        let project = config.projects[0].id;
        let items = demo::items();
        let invalid = items
            .iter()
            .find(|i| i.key.project == project && i.key.kind == ItemKind::MergeRequest)
            .unwrap()
            .key
            .clone();
        let valid = items
            .iter()
            .find(|i| i.key.project != project && i.key.kind == ItemKind::Issue)
            .unwrap()
            .key
            .clone();
        let job = demo::details(&invalid).jobs[0].id;
        let stale = TabState {
            route: Some(invalid.clone()),
            section: Some(3),
            field: 1,
            section_cursor: 3,
            content_offset: 4,
            expanded: BTreeSet::from([job]),
            ..TabState::default()
        };
        config.active_tab = 3;
        config.route = Some(invalid);
        config.section = stale.section;
        config.field = stale.field;
        config.tab_states.insert("3".into(), stale.clone());
        config.tab_states.insert("4".into(), stale);
        config.tab_states.insert(
            "5".into(),
            TabState {
                route: Some(valid.clone()),
                section: Some(0),
                field: 2,
                ..TabState::default()
            },
        );
        if remove {
            config.projects.remove(0);
        } else {
            config.projects[0].visible = false;
        }
        config.save(&path).unwrap();
        let mut ui = mount(Config::load(&path).unwrap(), Some(&path));
        assert_eq!(ui.state().config.active_tab, 3);
        active_list_responds(&mut ui);
        assert!(ui.state().expanded.is_empty());
        assert!(ui.state().traces.is_empty());
        assert_eq!(ui.state().content_offset, 0);
        key(&mut ui, KeyCode::Char('1'));
        active_list_responds(&mut ui);
        assert!(ui.state().expanded.is_empty());
        assert!(ui.state().traces.is_empty());
        assert_eq!(ui.state().content_offset, 0);
        let saved = persisted_tab(&ui, &path);
        assert!(saved.tab_states.values().all(|tab| {
            tab.route
                .as_ref()
                .is_none_or(|route| route.project != project)
        }));
        key(&mut ui, KeyCode::Char('2'));
        assert_eq!(ui.state().config.route.as_ref(), Some(&valid));
        assert_eq!(ui.state().scope, Scope::Section);
        assert_eq!(ui.state().config.section, Some(0));
        assert_eq!(ui.state().config.field, 2);
        assert_eq!(ui.state().details.as_ref().unwrap().item.key, valid);
    }
}

#[test]
fn a_list_request_started_before_a_successful_write_cannot_replace_newer_rows() {
    let mut ui = mount(config(), None);
    let old_epoch = ui.state().list_epoch;
    let project = ui.state().config.projects[0].id;
    ui.state_mut().list_pending.insert(project);
    ui.dispatch(Msg::MutationDone(Ok(()))).unwrap();
    assert!(ui.state().list_epoch > old_epoch);
    let before = ui.state().items.clone();
    ui.dispatch(Msg::ProjectLoaded(project, old_epoch, Ok((vec![], None))))
        .unwrap();
    assert_eq!(ui.state().items, before);
}

#[test]
fn late_detail_success_and_failure_cannot_replace_a_new_route_or_reopened_item() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    key(&mut ui, KeyCode::Enter);
    let old_key = ui.state().config.route.clone().unwrap();
    let old_epoch = ui.state().detail_epoch;
    key(&mut ui, KeyCode::Esc);
    key(&mut ui, KeyCode::Tab);
    key(&mut ui, KeyCode::Enter);
    let current = ui.state().config.route.clone().unwrap();
    let epoch = ui.state().detail_epoch;
    let mut fresh = demo::details(&current);
    // The demo's informational warning is not part of this successful transport response.
    fresh.warnings.clear();
    fresh.item.title = "Fresh detail response".into();
    ui.dispatch(Msg::DetailsLoaded(
        current.clone(),
        epoch,
        Ok(Box::new(fresh)),
    ))
    .unwrap();
    // Only request bookkeeping is injected; the route and epochs came from real navigation.
    ui.state_mut().detail_pending = Some((current.clone(), epoch));
    for (key, stale_epoch) in [(old_key, old_epoch), (current.clone(), old_epoch)] {
        let mut stale = demo::details(&key);
        stale.item.title = "STALE DETAIL MUST NOT APPEAR".into();
        ui.dispatch(Msg::DetailsLoaded(
            key.clone(),
            stale_epoch,
            Ok(Box::new(stale)),
        ))
        .unwrap();
        ui.dispatch(Msg::DetailsLoaded(
            key,
            stale_epoch,
            Err("stale failure".into()),
        ))
        .unwrap();
        assert_eq!(
            ui.state().details.as_ref().unwrap().item.title,
            "Fresh detail response"
        );
        assert_eq!(ui.state().detail_pending, Some((current.clone(), epoch)));
        assert!(ui.state().error.is_none());
        assert_eq!(ui.state().failures, 0);
    }
    key(&mut ui, KeyCode::Esc);
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().config.route.as_ref(), Some(&current));
    assert!(ui.state().detail_epoch > epoch);
    ui.dispatch(Msg::DetailsLoaded(
        current.clone(),
        epoch,
        Err("old visit".into()),
    ))
    .unwrap();
    assert!(ui.state().error.is_none());
    assert_eq!(ui.state().details.as_ref().unwrap().item.key, current);
}

fn traces(state: &State) -> BTreeMap<u64, (String, u64, bool, Option<String>)> {
    state
        .traces
        .iter()
        .map(|(&id, t)| (id, (t.text.clone(), t.offset, t.finished, t.error.clone())))
        .collect()
}

fn cache_finished_job(ui: &mut Ui, marker: &str) -> u64 {
    let route = ui.state().config.route.clone().unwrap();
    let epoch = ui.state().detail_epoch;
    let mut details = ui.state().details.clone().unwrap();
    details.warnings.clear();
    details.item.title = marker.into();
    let index = details.jobs.iter().position(|j| !j.running()).unwrap();
    let job = details.jobs[index].id;
    ui.dispatch(Msg::DetailsLoaded(
        route.clone(),
        epoch,
        Ok(Box::new(details)),
    ))
    .unwrap();
    open_section(ui, "Jobs");
    for _ in 0..index {
        key(ui, KeyCode::Down);
    }
    key(ui, KeyCode::Char(' '));
    assert!(ui.state().expanded.contains(&job));
    let text = format!("{marker} λ cached trace\n").repeat(40);
    let offset = text.len() as u64;
    ui.dispatch(Msg::TraceLoaded(
        route.clone(),
        epoch,
        job,
        true,
        Ok(TraceChunk {
            text,
            next_offset: offset,
            reset: true,
        }),
    ))
    .unwrap();
    ui.dispatch(Msg::TraceLoaded(
        route,
        epoch,
        job,
        true,
        Ok(TraceChunk {
            text: String::new(),
            next_offset: offset,
            reset: false,
        }),
    ))
    .unwrap();
    assert!(ui.state().traces[&job].finished);
    job
}

#[test]
fn tab_switches_restore_independent_caches_and_reject_responses_from_previous_visits() {
    let mut config = numbered_views_config(1);
    config.views[0].kind = ItemKind::MergeRequest;
    let mut ui = mount(config, None);
    key(&mut ui, KeyCode::Char('M'));
    key(&mut ui, KeyCode::Enter);
    let job = cache_finished_job(&mut ui, "Built-in cached detail");
    settle_layout(&mut ui);
    ui.dispatch(Msg::DetailScrolled(3)).unwrap();
    settle_layout(&mut ui);
    let builtin = tab_state(ui.state());
    let builtin_selected = ui.state().scroll.selected;
    let builtin_traces = traces(ui.state());
    let old_epoch = ui.state().detail_epoch;
    let route = ui.state().config.route.clone().unwrap();

    key(&mut ui, KeyCode::Char('1'));
    active_list_responds(&mut ui);
    assert!(ui.state().traces.is_empty());
    assert!(ui.state().expanded.is_empty());
    assert_eq!(ui.state().content_offset, 0);
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().config.route.as_ref(), Some(&route));
    assert_eq!(cache_finished_job(&mut ui, "Saved tab cached detail"), job);
    settle_layout(&mut ui);
    ui.dispatch(Msg::DetailScrolled(5)).unwrap();
    settle_layout(&mut ui);
    let saved = tab_state(ui.state());
    let saved_selected = ui.state().scroll.selected;
    let saved_traces = traces(ui.state());
    assert_ne!(
        builtin_traces, saved_traces,
        "even the same route has per-tab caches"
    );
    let saved_epoch = ui.state().detail_epoch;
    assert!(saved_epoch > old_epoch);

    key(&mut ui, KeyCode::Char('M'));
    assert_tab_state(&ui, &builtin, builtin_selected);
    assert_eq!(
        ui.state().details.as_ref().unwrap().item.title,
        "Built-in cached detail"
    );
    assert_eq!(traces(ui.state()), builtin_traces);
    let epoch = ui.state().detail_epoch;
    assert!(
        epoch > saved_epoch,
        "restoring a cached route starts a new request generation"
    );
    // Only request bookkeeping is synthetic; route changes and epochs came from tab navigation.
    ui.state_mut().detail_pending = Some((route.clone(), epoch));
    ui.state_mut().trace_pending.insert((epoch, job));
    for stale_epoch in [old_epoch, saved_epoch] {
        let mut stale = demo::details(&route);
        stale.item.title = "STALE RESTORED DETAIL".into();
        ui.dispatch(Msg::DetailsLoaded(
            route.clone(),
            stale_epoch,
            Ok(Box::new(stale)),
        ))
        .unwrap();
        ui.dispatch(Msg::DetailsLoaded(
            route.clone(),
            stale_epoch,
            Err("stale detail failure".into()),
        ))
        .unwrap();
        ui.dispatch(Msg::TraceLoaded(
            route.clone(),
            stale_epoch,
            job,
            true,
            Ok(TraceChunk {
                text: "STALE RESTORED TRACE".into(),
                next_offset: 1,
                reset: true,
            }),
        ))
        .unwrap();
        ui.dispatch(Msg::TraceLoaded(
            route.clone(),
            stale_epoch,
            job,
            true,
            Err("stale trace failure".into()),
        ))
        .unwrap();
        assert_tab_state(&ui, &builtin, builtin_selected);
        assert_eq!(
            ui.state().details.as_ref().unwrap().item.title,
            "Built-in cached detail"
        );
        assert_eq!(traces(ui.state()), builtin_traces);
        assert_eq!(ui.state().detail_pending, Some((route.clone(), epoch)));
        assert!(ui.state().trace_pending.contains(&(epoch, job)));
        assert!(ui.state().error.is_none());
        assert_eq!(ui.state().failures, 0);
        assert_eq!(ui.state().blocked_until, Duration::ZERO);
    }
    // Re-selecting the current tab pops to its list and discards pending detail requests.
    key(&mut ui, KeyCode::Char('M'));
    assert_tab_state(&ui, &popped_to_list(&builtin), builtin_selected);
    assert!(ui.state().details.is_none());
    assert!(ui.state().traces.is_empty());
    assert!(ui.state().detail_pending.is_none());
    assert!(ui.state().detail_epoch > epoch);
    key(&mut ui, KeyCode::Char('1'));
    assert_tab_state(&ui, &saved, saved_selected);
    assert_eq!(
        ui.state().details.as_ref().unwrap().item.title,
        "Saved tab cached detail"
    );
    assert_eq!(traces(ui.state()), saved_traces);
}

#[test]
fn late_trace_success_and_failure_do_not_repopulate_or_corrupt_the_new_route() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('M'));
    key(&mut ui, KeyCode::Enter);
    let old = ui.state().config.route.clone().unwrap();
    let old_epoch = ui.state().detail_epoch;
    let job = ui.state().details.as_ref().unwrap().jobs[0].id;
    key(&mut ui, KeyCode::Esc);
    key(&mut ui, KeyCode::Tab);
    key(&mut ui, KeyCode::Enter);
    let current = ui.state().config.route.clone().unwrap();
    let epoch = ui.state().detail_epoch;
    let current_job = ui.state().details.as_ref().unwrap().jobs[0].id;
    let before = traces(ui.state());
    ui.state_mut().trace_pending.insert((epoch, current_job));
    for (key, id) in [(old, job), (current, current_job)] {
        ui.dispatch(Msg::TraceLoaded(
            key.clone(),
            old_epoch,
            id,
            true,
            Ok(TraceChunk {
                text: "STALE TRACE MUST NOT APPEAR".into(),
                next_offset: 100,
                reset: true,
            }),
        ))
        .unwrap();
        ui.dispatch(Msg::TraceLoaded(
            key,
            old_epoch,
            id,
            true,
            Err("stale trace failure".into()),
        ))
        .unwrap();
        assert_eq!(traces(ui.state()), before);
        assert!(ui.state().trace_pending.contains(&(epoch, current_job)));
        assert!(ui.state().error.is_none());
        assert_eq!(ui.state().failures, 0);
    }
}

#[test]
fn failed_list_refresh_retains_last_good_rows_and_selected_identity() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    key(&mut ui, KeyCode::Tab);
    let selected = selected_key(ui.state());
    let project = selected.project;
    let mut refreshed: Vec<_> = ui
        .state()
        .items
        .iter()
        .filter(|i| i.key.project == project)
        .cloned()
        .collect();
    refreshed
        .iter_mut()
        .find(|i| i.key == selected)
        .unwrap()
        .updated_at = "2099-01-01T00:00:00Z".into();
    // Simulate a queued refresh/completion without constructing a client or starting threads.
    ui.state_mut().list_epoch = 2;
    ui.state_mut().list_pending.insert(project);
    ui.dispatch(Msg::ProjectLoaded(project, 2, Ok((refreshed, None))))
        .unwrap();
    assert_eq!(selected_key(ui.state()), selected);
    assert_eq!(ui.state().scroll.selected, 0);
    let good = ui.state().items.clone();
    let scroll = ui.state().scroll.clone();
    ui.state_mut().list_pending.insert(project);
    ui.dispatch(Msg::ProjectLoaded(
        project,
        2,
        Err("503 unavailable; Retry-After: 120".into()),
    ))
    .unwrap();
    assert_eq!(ui.state().items, good);
    assert_eq!(ui.state().scroll, scroll);
    assert_eq!(selected_key(ui.state()), selected);
    assert!(ui.state().list_pending.is_empty());
    assert!(ui.state().error.as_deref().unwrap().contains("503"));
    assert!(ui.state().blocked_until.as_secs() >= 120);
    ui.dispatch(Msg::ProjectLoaded(project, 1, Ok((vec![], None))))
        .unwrap();
    assert_eq!(
        ui.state().items,
        good,
        "an older successful refresh must not erase the last good list"
    );
    ui.render();
    assert!(
        ui.capture_frame()
            .plain_text()
            .contains(&format!("#{}", selected.iid))
    );
}

#[test]
fn a_finished_job_is_not_marked_drained_until_a_no_progress_eof_chunk() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('M'));
    key(&mut ui, KeyCode::Enter);
    for _ in 0..3 {
        key(&mut ui, KeyCode::Tab);
    }
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().section_name(), "Jobs");
    let key = ui.state().config.route.clone().unwrap();
    let epoch = ui.state().detail_epoch;
    let job = ui.state().details.as_ref().unwrap().jobs[0].id;
    assert!(!ui.state().details.as_ref().unwrap().jobs[0].running());
    // Enter focuses and expands the selected job; only the transport's bounded chunks are synthetic.
    self::key(&mut ui, KeyCode::Enter);
    assert!(ui.state().expanded.contains(&job));
    let mut text = String::new();
    for (index, chunk) in ["first λ chunk\n", "last β chunk\n"]
        .into_iter()
        .enumerate()
    {
        text.push_str(chunk);
        ui.dispatch(Msg::TraceLoaded(
            key.clone(),
            epoch,
            job,
            true,
            Ok(TraceChunk {
                text: chunk.into(),
                next_offset: text.len() as u64,
                reset: index == 0,
            }),
        ))
        .unwrap();
        let trace = &ui.state().traces[&job];
        assert_eq!(trace.text, text);
        assert_eq!(trace.offset, text.len() as u64);
        assert!(
            !trace.finished,
            "job completion is not trace EOF while the byte offset advances"
        );
    }
    ui.dispatch(Msg::TraceLoaded(
        key,
        epoch,
        job,
        true,
        Ok(TraceChunk {
            text: String::new(),
            next_offset: text.len() as u64,
            reset: false,
        }),
    ))
    .unwrap();
    assert!(ui.state().traces[&job].finished);
    ui.dispatch(Msg::LoadTraces).unwrap();
    assert_eq!(
        ui.state().traces[&job].text,
        text,
        "a drained completed job must not be fetched again, even in demo mode"
    );
    assert_eq!(ui.state().traces[&job].offset, text.len() as u64);
}

#[test]
fn every_tabs_navigation_persists_immediately_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let mut config = numbered_views_config(3);
    config.views[1].kind = ItemKind::MergeRequest;
    // Keep Description and Jobs scrollable before and after restart, without relying on cached text.
    let viewport = Rect {
        x: 0,
        y: 0,
        w: 80,
        h: 14,
    };
    let mut ui = mount_with_viewport(config, Some(&path), viewport);
    let mut visits = Vec::new();
    for (index, ch) in "DPIM123".chars().enumerate() {
        key(&mut ui, KeyCode::Char(ch));
        key(&mut ui, KeyCode::End);
        persisted_tab(&ui, &path);
        key(&mut ui, KeyCode::Up);
        persisted_tab(&ui, &path);
        if index != 1 && index != 6 {
            assert!(ui.state().scroll.offset > 0);
            key(&mut ui, KeyCode::Enter);
            persisted_tab(&ui, &path);
            match index {
                0 | 5 => {
                    // Details scope remembers its highlighted section without opening it.
                    for _ in 0..=index % 3 {
                        key(&mut ui, KeyCode::Down);
                        persisted_tab(&ui, &path);
                    }
                    assert_eq!(ui.state().config.section, None);
                }
                2 => {
                    open_section(&mut ui, "Fields");
                    persisted_tab(&ui, &path);
                    for _ in 0..2 {
                        key(&mut ui, KeyCode::Down);
                        persisted_tab(&ui, &path);
                    }
                    assert_eq!(ui.state().config.field, 2);
                }
                3 => {
                    open_section(&mut ui, "Jobs");
                    persisted_tab(&ui, &path);
                    key(&mut ui, KeyCode::Down);
                    persisted_tab(&ui, &path);
                    let job = ui.state().details.as_ref().unwrap().jobs[1].id;
                    key(&mut ui, KeyCode::Enter);
                    assert!(
                        persisted_tab(&ui, &path).tab_states["3"]
                            .expanded
                            .contains(&job)
                    );
                    // Exercise the scroll callback without depending on detail widget geometry.
                    ui.dispatch(Msg::ContentScroll(3)).unwrap();
                    settle_layout(&mut ui);
                    assert_eq!(persisted_tab(&ui, &path).tab_states["3"].content_offset, 3);
                }
                4 => {
                    open_section(&mut ui, "Description");
                    settle_layout(&mut ui);
                    persisted_tab(&ui, &path);
                    // Description is anchored inside one continuous document, not a new scroll view.
                    let description_offset = ui.state().content_offset;
                    assert!(description_offset > 0);
                    key(&mut ui, KeyCode::Down);
                    settle_layout(&mut ui);
                    assert_eq!(
                        persisted_tab(&ui, &path).tab_states["4"].content_offset,
                        description_offset + 1
                    );
                    ui.dispatch(Msg::ContentScroll(description_offset + 4))
                        .unwrap();
                    settle_layout(&mut ui);
                    assert_eq!(
                        persisted_tab(&ui, &path).tab_states["4"].content_offset,
                        description_offset + 4
                    );
                }
                _ => unreachable!(),
            }
        }
        settle_layout(&mut ui);
        persisted_tab(&ui, &path);
        visits.push((tab_state(ui.state()), ui.state().scroll.selected));
    }
    // Identical routes in different tabs must still restore independent document offsets.
    assert_eq!(visits[2].0.route, visits[4].0.route);
    assert_ne!(visits[2].0.content_offset, visits[4].0.content_offset);
    key(&mut ui, KeyCode::Char('D'));
    settle_layout(&mut ui);
    let saved = persisted_tab(&ui, &path);
    for (index, (expected, selected)) in visits.iter().enumerate() {
        let tab = index.to_string();
        assert_eq!(saved.selections[&tab], *selected);
        assert_eq!(
            serde_json::to_value(&saved.tab_states[&tab]).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
    drop(ui);

    let mut restarted = mount_with_viewport(Config::load(&path).unwrap(), Some(&path), viewport);
    settle_layout(&mut restarted);
    assert_tab_state(&restarted, &visits[0].0, visits[0].1);
    for (index, ch) in "DPIM123".chars().enumerate() {
        // The restored Dashboard is already active, so selecting it pops to its list.
        let expected = if index == 0 {
            popped_to_list(&visits[index].0)
        } else {
            visits[index].0.clone()
        };
        key(&mut restarted, KeyCode::Char(ch));

        settle_layout(&mut restarted);
        assert_eq!(restarted.state().config.active_tab, index);
        assert_tab_state(&restarted, &expected, visits[index].1);
        persisted_tab(&restarted, &path);
    }
}

#[test]
fn restored_tab_uses_selections_for_its_list_cursor_not_the_detail_route() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let mut config = config();
    let route = demo::items()
        .into_iter()
        .find(|i| i.key.kind == ItemKind::Issue)
        .unwrap()
        .key;
    let expected = TabState {
        route: Some(route.clone()),
        section_cursor: 2,
        ..TabState::default()
    };
    config.active_tab = 2;
    config.route = Some(route.clone());
    config.selections.insert("2".into(), 1);
    config.tab_states.insert("2".into(), expected.clone());
    config.save(&path).unwrap();
    let mut ui = mount(Config::load(&path).unwrap(), Some(&path));
    assert_tab_state(&ui, &expected, 1);
    assert_ne!(selected_key(ui.state()), route);
    key(&mut ui, KeyCode::Char('D'));
    key(&mut ui, KeyCode::Char('I'));
    assert_tab_state(&ui, &expected, 1);
    assert_eq!(persisted_tab(&ui, &path).selections["2"], 1);
}

#[test]
fn list_scroll_callbacks_persist_offsets_for_builtin_and_saved_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let mut ui = mount(numbered_views_config(1), Some(&path));
    ui.set_viewport(Rect {
        x: 0,
        y: 0,
        w: 100,
        h: 14,
    });
    let mut visits = Vec::new();
    for ch in "DPIM1".chars() {
        key(&mut ui, KeyCode::Char(ch));
        // List scroll messages carry cell offsets; navigation stores row offsets.
        ui.dispatch(Msg::ListScroll(6)).unwrap();
        assert!(ui.state().scroll.offset > 0);
        persisted_tab(&ui, &path);
        visits.push((tab_state(ui.state()), ui.state().scroll.selected));
    }
    for (index, ch) in "DPIM1".chars().enumerate() {
        key(&mut ui, KeyCode::Char(ch));
        assert_tab_state(&ui, &visits[index].0, visits[index].1);
        persisted_tab(&ui, &path);
    }
}

#[test]
fn committed_keyboard_events_persist_immediately_and_restart_restores_the_route() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let mut ui = mount(config(), Some(&path));
    assert!(!path.exists(), "mounting is not a committed workspace edit");
    key(&mut ui, KeyCode::Tab);
    assert_eq!(persisted(&ui, &path).selections["0"], 1);
    key(&mut ui, KeyCode::Char('P'));
    persisted(&ui, &path);
    key(&mut ui, KeyCode::Char(' '));
    assert!(!persisted(&ui, &path).projects[0].visible);
    key(&mut ui, KeyCode::Char('I'));
    persisted(&ui, &path);
    key(&mut ui, KeyCode::Char('/'));
    ui.send_paste("state:opened").unwrap();
    assert!(
        !Config::load(&path).unwrap().filters.contains_key("2"),
        "drafts are not committed filters"
    );
    key(&mut ui, KeyCode::Enter);
    assert_eq!(persisted(&ui, &path).filters["2"], "state:opened");
    key(&mut ui, KeyCode::Char('s'));
    ui.send_paste("Restart queue").unwrap();
    key(&mut ui, KeyCode::Enter);
    assert_eq!(persisted(&ui, &path).views[0].name, "Restart queue");
    key(&mut ui, KeyCode::Tab);
    persisted(&ui, &path);
    let selected = selected_key(ui.state());
    key(&mut ui, KeyCode::Enter);
    assert_eq!(persisted(&ui, &path).route.as_ref(), Some(&selected));
    key(&mut ui, KeyCode::Enter);
    assert_eq!(persisted(&ui, &path).section, Some(0));
    key(&mut ui, KeyCode::Tab);
    settle_layout(&mut ui);
    assert_eq!(persisted_tab(&ui, &path).field, 1);
    let committed = std::fs::read(&path).unwrap();
    key(&mut ui, KeyCode::Enter);
    replace_input(&mut ui, "reopen");
    key(&mut ui, KeyCode::Esc);
    settle_layout(&mut ui);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        committed,
        "cancelling a draft must not persist it"
    );
    drop(ui);

    let mut restarted = mount(Config::load(&path).unwrap(), Some(&path));
    assert_eq!(restarted.state().scope, Scope::Section);
    assert_eq!(restarted.state().tab_name(), "Restart queue");
    assert_eq!(restarted.state().config.route.as_ref(), Some(&selected));
    assert_eq!(
        restarted.state().details.as_ref().unwrap().item.key,
        selected
    );
    assert_eq!(restarted.state().section_name(), "Fields");
    assert_eq!(restarted.state().config.field, 1);
    settle_layout(&mut restarted);
    let mut navigation = tab_state(restarted.state());
    let list_selected = restarted.state().scroll.selected;
    assert_eq!(
        navigation.content_offset, 0,
        "early Fields stay within the boundary band without scrolling to the top"
    );
    key(&mut restarted, KeyCode::Esc);
    settle_layout(&mut restarted);
    navigation.section = None;
    assert_tab_state(&restarted, &navigation, list_selected);
    assert_eq!(persisted_tab(&restarted, &path).section, None);
    key(&mut restarted, KeyCode::Esc);
    assert!(persisted(&restarted, &path).route.is_none());
    assert_eq!(selected_key(restarted.state()), selected);
}

#[test]
fn clicking_a_field_persists_the_cursor_even_when_the_edit_is_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let mut ui = mount(config(), Some(&path));
    key(&mut ui, KeyCode::Char('I'));
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Enter);
    assert_eq!(persisted(&ui, &path).field, 0);
    click_text(&mut ui, "Labels", 0);
    assert!(matches!(&dialog(&ui).kind, DialogKind::Edit(_, field) if field == "labels"));
    assert_eq!(ui.state().config.field, 2);
    key(&mut ui, KeyCode::Esc);
    assert!(ui.state().dialog.is_none());
    assert_eq!(
        Config::load(&path).unwrap().field,
        2,
        "mouse selection must persist just like keyboard selection, independently of saving the draft"
    );
}

#[test]
fn list_refresh_reordering_persists_the_new_index_of_the_selected_item() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let mut ui = mount(config(), Some(&path));
    key(&mut ui, KeyCode::Char('I'));
    key(&mut ui, KeyCode::Tab);
    let selected = selected_key(ui.state());
    assert_eq!(persisted(&ui, &path).selections["2"], 1);
    let mut items: Vec<_> = ui
        .state()
        .items
        .iter()
        .filter(|i| i.key.project == selected.project)
        .cloned()
        .collect();
    items
        .iter_mut()
        .find(|i| i.key == selected)
        .unwrap()
        .updated_at = "2099-01-01T00:00:00Z".into();
    let epoch = ui.state().list_epoch;
    ui.dispatch(Msg::ProjectLoaded(
        selected.project,
        epoch,
        Ok((items, None)),
    ))
    .unwrap();
    assert_eq!(selected_key(ui.state()), selected);
    assert_eq!(ui.state().scroll.selected, 0);
    assert_eq!(
        Config::load(&path).unwrap().selections["2"],
        0,
        "a refresh-induced cursor move must reach disk without another key event"
    );
}

#[test]
fn retry_job_confirmation_uses_the_pipeline_project_and_falls_back_to_the_mr_project() {
    for jobs_project in [Some(42_424), None] {
        let mut ui = mount(config(), None);
        key(&mut ui, KeyCode::Char('M'));
        key(&mut ui, KeyCode::Enter);
        let route = ui.state().config.route.clone().unwrap();
        let epoch = ui.state().detail_epoch;
        let mut details = demo::details(&route);
        details.jobs_project = jobs_project;
        details.warnings.clear();
        details.jobs[1].status = "failed".into();
        let job = details.jobs[1].id;
        let expected_project = jobs_project.unwrap_or(route.project);
        if jobs_project.is_some() {
            assert_ne!(expected_project, route.project);
        }
        ui.dispatch(Msg::DetailsLoaded(
            route.clone(),
            epoch,
            Ok(Box::new(details)),
        ))
        .unwrap();
        for _ in 0..3 {
            key(&mut ui, KeyCode::Tab);
        }
        key(&mut ui, KeyCode::Enter);
        assert_eq!(ui.state().section_name(), "Jobs");
        key(&mut ui, KeyCode::Tab);
        assert_eq!(ui.state().config.field, 1);
        key(&mut ui, KeyCode::Char('r'));
        assert!(
            matches!(
                &dialog(&ui).kind,
                DialogKind::Confirm(Confirmation::Mutate(Mutation::RetryJob {
                    project, job: selected_job,
                })) if *project == expected_project && *selected_job == job
            ),
            "the retry must target the selected job's pipeline project, not the MR's project"
        );
        assert_eq!(ui.state().config.route.as_ref(), Some(&route));
        assert!(
            !ui.state().mutation_pending,
            "opening confirmation must not submit a retry"
        );
        key(&mut ui, KeyCode::Esc);
        assert!(ui.state().dialog.is_none());
    }
}

#[test]
fn optional_detail_warnings_do_not_back_off_or_stall_live_traces() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('M'));
    key(&mut ui, KeyCode::Enter);
    let route = ui.state().config.route.clone().unwrap();
    let epoch = ui.state().detail_epoch;
    let mut details = demo::details(&route);
    let warnings = vec![
        "Iterations unavailable: HTTP 403 Forbidden".to_owned(),
        "Some diffs are collapsed or too large on GitLab; patch content is incomplete".to_owned(),
    ];
    details.warnings = warnings.clone();
    details.item.title = "Updated MR with partial optional details".into();
    let live_job = details.jobs[0].id;
    assert!(!ui.state().traces.contains_key(&live_job));
    details.jobs[0].status = "running".into();
    ui.dispatch(Msg::DetailsLoaded(
        route.clone(),
        epoch,
        Ok(Box::new(details)),
    ))
    .unwrap();
    let loaded = ui.state().details.as_ref().unwrap();
    assert_eq!(loaded.item.key, route);
    assert_eq!(
        loaded.item.title,
        "Updated MR with partial optional details"
    );
    assert_eq!(
        loaded.warnings, warnings,
        "partial-feature warnings must remain available to the view"
    );
    assert_eq!(ui.state().failures, 0);
    assert_eq!(ui.state().blocked_until, Duration::ZERO);
    assert!(ui.state().error.is_none());
    assert!(
        ui.state()
            .traces
            .get(&live_job)
            .is_some_and(|trace| !trace.text.is_empty() && !trace.finished),
        "DetailsLoaded must still schedule live traces when optional features are unavailable"
    );
}

#[test]
fn numeric_retry_after_in_a_detail_warning_survives_shorter_errors_from_other_requests() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('M'));
    key(&mut ui, KeyCode::Enter);
    let route = ui.state().config.route.clone().unwrap();
    let epoch = ui.state().detail_epoch;
    let mut details = demo::details(&route);
    let job = details.jobs[0].id;
    let rate_limit = "Jobs incomplete (0 entries loaded): HTTP 429 Too Many Requests; Retry-After: 7200 (retry later; no automatic retry)";
    details.warnings = vec![
        rate_limit.into(),
        "Iterations unavailable: HTTP 403 Forbidden".into(),
    ];
    ui.dispatch(Msg::DetailsLoaded(
        route.clone(),
        epoch,
        Ok(Box::new(details)),
    ))
    .unwrap();
    assert_eq!(ui.state().failures, 1);
    assert_eq!(ui.state().error.as_deref(), Some(rate_limit));
    let deadline = ui.state().blocked_until;
    assert!(deadline >= Duration::from_secs(7200));

    let short_error =
        "Trace HTTP 503 unavailable; Retry-After: 5 (retry later; no automatic retry)";
    ui.dispatch(Msg::TraceLoaded(
        route.clone(),
        epoch,
        job,
        false,
        Err(short_error.into()),
    ))
    .unwrap();
    assert_eq!(ui.state().failures, 2);
    assert_eq!(ui.state().error.as_deref(), Some(short_error));
    assert!(
        ui.state().blocked_until >= deadline,
        "a different request's shorter Retry-After cannot shorten the global deadline"
    );
    let list_epoch = ui.state().list_epoch;
    ui.dispatch(Msg::ProjectLoaded(
        route.project,
        list_epoch,
        Err("connection reset".into()),
    ))
    .unwrap();
    assert_eq!(ui.state().failures, 3);
    assert_eq!(ui.state().error.as_deref(), Some("connection reset"));
    assert!(
        ui.state().blocked_until >= deadline,
        "ordinary exponential backoff cannot replace a longer Retry-After"
    );
}

#[test]
fn http_date_retry_after_is_honored_and_a_later_short_error_cannot_shorten_it() {
    let mut ui = mount(config(), None);
    let date = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(3600));
    let error =
        format!("HTTP 503 unavailable; Retry-After: {date} (retry later; no automatic retry)");
    ui.dispatch(Msg::UserLoaded(Err(error.clone()))).unwrap();
    assert_eq!(ui.state().error.as_deref(), Some(error.as_str()));
    assert_eq!(ui.state().failures, 1);
    let deadline = ui.state().blocked_until;
    assert!(
        deadline >= Duration::from_secs(3590),
        "HTTP-date must retain roughly an hour, not fall back to ten seconds: {deadline:?}"
    );

    let project = ui.state().config.projects[0].id;
    let epoch = ui.state().list_epoch;
    let short_error =
        "HTTP 429 Too Many Requests; Retry-After: 1 (retry later; no automatic retry)";
    ui.dispatch(Msg::ProjectLoaded(project, epoch, Err(short_error.into())))
        .unwrap();
    assert_eq!(ui.state().failures, 2);
    assert_eq!(ui.state().error.as_deref(), Some(short_error));
    assert!(
        ui.state().blocked_until >= deadline,
        "the global deadline must be the maximum across concurrent failures"
    );
}

#[test]
fn failed_mutation_clears_pending_but_keeps_all_dialog_fields_and_an_editable_draft() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    key(&mut ui, KeyCode::Enter);
    let route = ui.state().config.route.clone().unwrap();
    let epoch = ui.state().detail_epoch;
    let items = ui.state().items.clone();
    key(&mut ui, KeyCode::Char('n'));
    assert!(matches!(dialog(&ui).kind, DialogKind::NewIssue));
    key(&mut ui, KeyCode::Tab);
    ui.send_paste("Retain this title λ").unwrap();
    key(&mut ui, KeyCode::Tab);
    ui.send_paste("First draft line\nSecond draft line β")
        .unwrap();
    focused_field(&ui, 2);
    let draft: Vec<_> = dialog(&ui)
        .fields
        .iter()
        .map(|field| field.value().to_owned())
        .collect();
    assert_eq!(draft[1], "Retain this title λ");
    assert_eq!(draft[2], "First draft line\nSecond draft line β");
    // Demo mode never submits remotely; inject only the in-flight marker and transport result.
    ui.state_mut().mutation_pending = true;
    key(&mut ui, KeyCode::Esc);
    assert!(ui.state().mutation_pending);
    assert!(
        ui.state().dialog.is_some(),
        "an in-flight draft cannot be dismissed"
    );
    let error = "HTTP 502 while saving the issue";
    ui.dispatch(Msg::MutationDone(Err(error.into()))).unwrap();
    assert!(!ui.state().mutation_pending);
    assert!(matches!(dialog(&ui).kind, DialogKind::NewIssue));
    assert_eq!(dialog(&ui).error.as_deref(), Some(error));
    assert_eq!(
        dialog(&ui)
            .fields
            .iter()
            .map(|field| field.value().to_owned())
            .collect::<Vec<_>>(),
        draft
    );
    assert_eq!(dialog(&ui).selected, 2);
    focused_field(&ui, 2);
    assert_eq!(ui.state().config.route.as_ref(), Some(&route));
    assert_eq!(ui.state().detail_epoch, epoch);
    assert_eq!(ui.state().items, items);
    assert_eq!(ui.state().error.as_deref(), Some(error));
    assert_eq!(ui.state().failures, 1);
    assert!(ui.state().blocked_until >= Duration::from_secs(10));
    ui.send_paste(" amended").unwrap();
    assert_eq!(
        dialog(&ui).fields[2].value(),
        format!("{} amended", draft[2])
    );
    assert_eq!(dialog(&ui).fields[1].value(), draft[1]);
    key(&mut ui, KeyCode::Esc);
    assert!(
        ui.state().dialog.is_none(),
        "after failure the retained draft can be cancelled"
    );
    assert_eq!(ui.state().scope, Scope::Details);
    assert_eq!(ui.state().config.route.as_ref(), Some(&route));
}

#[test]
fn configured_user_name_pattern_is_used_in_the_interface() {
    let config = Config {
        user_display: UserDisplay::Name,
        user_name_pattern: Some(
            r"^(?P<surname>\S+)\s+(?P<firstname>\S+)\s+(?P<staff_id>\d+)$".into(),
        ),
        user_name_format: Some("$firstname".into()),
        ..config()
    };
    let mut ui = mount(config, None);
    let user = User {
        id: 9876,
        username: "jdoe".into(),
        name: "Doe John 12345678".into(),
    };
    ui.state_mut().user = user.clone();
    ui.state_mut().items[0].author = user;
    ui.render();

    let text = ui.capture_frame().plain_text();
    assert!(text.contains("John"), "{text}");
    assert!(!text.contains("@jdoe"), "{text}");
    assert!(!text.contains("Doe John 12345678"), "{text}");
}

#[test]
fn dashboard_rows_show_user_role_separately_from_attention_reason() {
    let ui = mount(config(), None);
    let frame = ui.capture_frame();
    let text = frame.plain_text();
    let lines = frame.to_lines();
    let attention_index = lines
        .iter()
        .position(|line| line.contains("Why: Pipeline failed"))
        .expect("dashboard reason should be rendered");
    let attention_line = &lines[attention_index];
    let spacer_line: String = lines[attention_index + 1]
        .chars()
        .take(frame.width as usize - 1)
        .collect();

    assert!(
        attention_line.starts_with("▕   Why: Pipeline failed"),
        "{text}"
    );
    assert!(
        attention_line.contains("Pipeline failed, 2 unresolved threads"),
        "{text}"
    );
    assert!(attention_line.contains(" ·  You: author"), "{text}");
    assert!(spacer_line.trim().is_empty(), "{text}");
    assert!(lines[attention_index + 2].contains("!101"), "{text}");
}

#[test]
fn small_project_viewport_snapshot() {
    let mut ui = mount(config(), None);
    ui.set_viewport(Rect {
        x: 0,
        y: 0,
        w: 48,
        h: 14,
    });
    key(&mut ui, KeyCode::Char('P'));
    ui.render();
    let frame = ui.capture_frame();
    assert_eq!((frame.width, frame.height), (48, 14));
    let lines = frame.to_lines();
    let tag = build_info::display_version()
        .split_once('@')
        .map_or(build_info::display_version(), |(tag, _)| tag.to_owned());
    assert!(lines[0].contains("@demo-arin  ·  midnight"), "{}", lines[0]);
    assert!(lines[0].contains(&format!("{tag}@")), "{}", lines[0]);
    assert_eq!(
        &lines[1..],
        &[
            "  Dashboard   Projects   Issues   Merge Request",
            "",
            "  Projects",
            "  5 PROJECTS   Space toggles visibility · Ent…",
            "",
            "▕ [x] Orbit scheduler · DEMO         8 items   █",
            "▕     demo-lab/constellation/platform/runtime… █",
            "                                               ▀",
            "  [x] Meteor cache · DEMO            8 items",
            "      demo-lab/constellation/platform/storage…",
            "",
            " LIST   DEMO  Demo workspace · no requests or r…",
            " D/P/I/M tabs   1–0 views   ↑ ↓ move   Enter op…",
        ]
    );
}

#[test]
fn small_empty_filter_viewport_snapshot() {
    let mut ui = mount(config(), None);
    key(&mut ui, KeyCode::Char('I'));
    key(&mut ui, KeyCode::Char('/'));
    ui.send_paste("no-such-fixture").unwrap();
    key(&mut ui, KeyCode::Enter);
    assert!(ui.state().visible_items().is_empty());
    ui.set_viewport(Rect {
        x: 0,
        y: 0,
        w: 40,
        h: 12,
    });
    ui.render();
    let frame = ui.capture_frame();
    assert_eq!((frame.width, frame.height), (40, 12));
    let lines = frame.to_lines();
    let tag = build_info::display_version()
        .split_once('@')
        .map_or(build_info::display_version(), |(tag, _)| tag.to_owned());
    assert!(lines[0].contains("@demo-arin  ·  midnight"), "{}", lines[0]);
    assert!(lines[0].contains(&format!("{tag}@")), "{}", lines[0]);
    assert_eq!(
        &lines[1..],
        &[
            "  Dashboard   Projects   Issues   Merge",
            "",
            "  Issues",
            "  0 ITEMS   Filter: no-such-fixture",
            "",
            "",
            "   No items match this filter",
            "",
            "",
            " LIST   DEMO  Demo workspace · no reque…",
            " D/P/I/M tabs   1–0 views   ↑ ↓ move   …",
        ]
    );
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().scope, Scope::List);
    assert!(ui.state().config.route.is_none());
}

#[test]
fn selecting_the_current_tab_returns_to_its_list_and_other_tabs_restore_drill_down() {
    let mut cfg = config();
    cfg.active_tab = 3;
    let mut ui = mount(cfg, None);
    key(&mut ui, KeyCode::Enter);
    let route = ui
        .state()
        .config
        .route
        .clone()
        .expect("opened a merge request");
    assert_eq!(ui.state().scope, Scope::Details);

    // Leaving and returning restores the drilled-down state.
    ui.dispatch(Msg::Tab(2)).unwrap();
    assert!(ui.state().config.route.is_none());
    ui.dispatch(Msg::Tab(3)).unwrap();
    assert_eq!(ui.state().config.route.as_ref(), Some(&route));
    assert_eq!(ui.state().scope, Scope::Details);

    // Re-selecting the current tab (click or shortcut) pops back to the list.
    ui.dispatch(Msg::Tab(3)).unwrap();
    assert!(ui.state().config.route.is_none());
    assert_eq!(ui.state().scope, Scope::List);
    assert!(ui.state().details.is_none());

    // From the list it is a no-op, and other tabs now restore the list too.
    ui.dispatch(Msg::Tab(3)).unwrap();
    assert_eq!(ui.state().scope, Scope::List);
    ui.dispatch(Msg::Tab(2)).unwrap();
    ui.dispatch(Msg::Tab(3)).unwrap();
    assert!(ui.state().config.route.is_none());
}
