use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, SystemTime},
};

use cronk::{
    config::{Config, SavedView},
    demo,
    model::{ItemKey, ItemKind, Mutation, TraceChunk},
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
    let app = App::new().focus_policy(FocusPolicy::Manual);
    let mut ui = TestBackend::new_with_app(
        app,
        Cronk {
            config,
            path: path.map(Path::to_path_buf),
            api: None,
        },
        (),
    );
    ui.pump().unwrap();
    ui.render();
    ui
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
fn horizontal_arrows_never_switch_tabs_in_any_scope() {
    let mut ui = mount(numbered_views_config(2), None);
    key(&mut ui, KeyCode::Char('I'));
    key(&mut ui, KeyCode::Tab);
    for scope in [Scope::List, Scope::Details, Scope::Section] {
        assert_eq!(ui.state().scope, scope);
        let before = serde_json::to_value(&ui.state().config).unwrap();
        let scroll = ui.state().scroll.clone();
        for mods in modifier_combinations() {
            for code in [KeyCode::Left, KeyCode::Right] {
                modified(&mut ui, code, mods);
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
            assert_eq!(ui.state().scope, scope);
            let before = serde_json::to_value(&ui.state().config).unwrap();
            let scroll = ui.state().scroll.clone();
            let epoch = ui.state().detail_epoch;
            for ch in "1234567890".chars().skip(count) {
                key(&mut ui, KeyCode::Char(ch));
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
fn keyboard_tab_switches_leave_details_and_sections_for_an_active_list() {
    let mut ui = mount(numbered_views_config(10), None);
    for depth in 1..=2 {
        for (index, ch) in "DPIM1234567890".chars().enumerate() {
            key(&mut ui, KeyCode::Char('I'));
            key(&mut ui, KeyCode::Enter);
            if depth == 2 {
                key(&mut ui, KeyCode::Enter);
            }
            assert_eq!(
                ui.state().scope,
                if depth == 1 {
                    Scope::Details
                } else {
                    Scope::Section
                }
            );
            key(&mut ui, KeyCode::Char(ch));
            assert_eq!(ui.state().config.active_tab, index, "key {ch}");
            active_list_responds(&mut ui);
        }
    }
}

#[test]
fn mouse_tab_switches_leave_details_and_sections_for_an_active_list() {
    let mut ui = mount(numbered_views_config(2), None);
    ui.set_viewport(Rect {
        x: 0,
        y: 0,
        w: 160,
        h: 30,
    });
    for depth in 0..=2 {
        for (index, label) in [
            "Dashboard",
            "Projects",
            "Issues",
            "Merge Requests",
            "1 Queue01",
            "2 Queue02",
        ]
        .into_iter()
        .enumerate()
        {
            key(&mut ui, KeyCode::Char('I'));
            for _ in 0..depth {
                key(&mut ui, KeyCode::Enter);
            }
            assert_eq!(
                ui.state().scope,
                match depth {
                    0 => Scope::List,
                    1 => Scope::Details,
                    _ => Scope::Section,
                }
            );
            // The first occurrence is in the rendered header, not the breadcrumb below it.
            click_text(&mut ui, label, 0);
            assert_eq!(ui.state().config.active_tab, index, "clicked {label}");
            active_list_responds(&mut ui);
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
    assert!(ui.capture_frame().plain_text().contains('▎'));
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
fn deleting_the_last_saved_view_does_not_resurrect_its_selection_key() {
    let mut config = saved_views_config();
    config.active_tab = 6;
    let mut ui = mount(config, None);
    palette_command(&mut ui, "Delete saved tab");
    click_text(&mut ui, " Confirm ", 0);
    assert_eq!(ui.state().config.views.len(), 2);
    assert!(!ui.state().config.filters.contains_key("6"));
    assert!(
        !ui.state().config.selections.contains_key("6"),
        "switching away must not recreate the deleted tab's cursor"
    );
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
fn a_list_request_started_before_a_successful_write_cannot_replace_newer_rows() {
    let mut ui = mount(config(), None);
    let old_epoch = ui.state().list_epoch;
    let project = ui.state().config.projects[0].id;
    ui.state_mut().list_pending.insert(project);
    ui.dispatch(Msg::MutationDone(Ok(()))).unwrap();
    assert!(ui.state().list_epoch > old_epoch);
    let before = ui.state().items.clone();
    ui.dispatch(Msg::ProjectLoaded(project, old_epoch, Ok(vec![])))
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
    ui.dispatch(Msg::ProjectLoaded(project, 2, Ok(refreshed)))
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
    ui.dispatch(Msg::ProjectLoaded(project, 1, Ok(vec![])))
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
    // Enter expands the actual selected job; only the transport's bounded chunks are synthetic.
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
    assert_eq!(persisted(&ui, &path).field, 1);
    let committed = std::fs::read(&path).unwrap();
    key(&mut ui, KeyCode::Enter);
    replace_input(&mut ui, "reopen");
    key(&mut ui, KeyCode::Esc);
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
    key(&mut restarted, KeyCode::Esc);
    assert_eq!(persisted(&restarted, &path).section, None);
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
    ui.dispatch(Msg::ProjectLoaded(selected.project, epoch, Ok(items)))
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
    assert_eq!(
        frame.to_lines(),
        [
            "  CRONK   DEMO · offli…@demo-arin  ·  midnight",
            "  Dashboard   Projects   Issues   Merge Request",
            "",
            "  Projects",
            "  5 PROJECTS   Space toggles visibility · Ent…",
            "",
            " [x] Orbit scheduler · DEMO            8 items",
            "     demo-lab/constellation/platform/runtime/or…",
            "",
            " [x] Meteor cache · DEMO               8 items",
            "     demo-lab/constellation/platform/storage/me…",
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
    assert_eq!(
        frame.to_lines(),
        [
            "  CRONK   DEMO…@demo-arin  ·  midnight",
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
