use cronk::{
    config::Config,
    demo,
    model::{ItemKey, ItemKind, LookupKind, LookupOption},
    ui::{Completion, Cronk, Msg},
};
use std::time::Duration;
use tui_lipan::{
    TestBackend,
    core::event::{MouseButton, MouseKind},
    prelude::*,
};

type Ui = TestBackend<Cronk>;
fn mount(kind: ItemKind, field: usize) -> Ui {
    let mut ui = TestBackend::new_with_app(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config: Config {
                projects: demo::projects(),
                active_tab: if kind == ItemKind::Issue { 2 } else { 3 },
                route: Some(ItemKey {
                    project: 9001,
                    iid: if kind == ItemKind::Issue { 1 } else { 101 },
                    kind,
                }),
                section: Some(0),
                field,
                onboarding: false,
                ..Config::default()
            },
            path: None,
            api: None,
            demo: true,
        },
        (),
    );
    ui.set_viewport(Rect {
        x: 0,
        y: 0,
        w: 100,
        h: 30,
    });
    settle(&mut ui);
    key(&mut ui, KeyCode::Enter);
    ui
}
fn settle(ui: &mut Ui) {
    for _ in 0..3 {
        ui.pump().unwrap();
        ui.render();
    }
}
fn key(ui: &mut Ui, code: KeyCode) {
    ui.send_key(KeyEvent {
        code,
        mods: KeyMods::NONE,
    })
    .unwrap();
    settle(ui);
}
fn replace(ui: &mut Ui, text: &str) {
    key(ui, KeyCode::Home);
    ui.send_key(KeyEvent {
        code: KeyCode::End,
        mods: KeyMods::SHIFT,
    })
    .unwrap();
    if text.is_empty() {
        key(ui, KeyCode::Backspace);
    } else {
        ui.send_paste(text).unwrap();
    }
    settle(ui);
}
fn wait(ui: &mut Ui) {
    ui.advance(Duration::from_millis(350));
    settle(ui);
}
fn completion(ui: &Ui) -> &Completion {
    ui.state().dialog.as_ref().unwrap().fields[0]
        .completion
        .as_ref()
        .unwrap()
}
fn value(ui: &Ui) -> &str {
    ui.state().dialog.as_ref().unwrap().fields[0].value()
}
fn ids(ui: &Ui) -> String {
    ui.state().dialog.as_ref().unwrap().fields[0]
        .api_value()
        .unwrap()
}
fn option(id: u64, name: &str, value: &str) -> LookupOption {
    LookupOption {
        id,
        label: name.into(),
        value: value.into(),
        api_value: id.to_string(),
        description: format!("ID {id}"),
        color: String::new(),
        text_color: String::new(),
    }
}

#[test]
fn empty_lookup_edits_submit_without_accepting_suggestions() {
    for (kind, field) in [
        (ItemKind::Issue, 2),
        (ItemKind::Issue, 3),
        (ItemKind::Issue, 4),
        (ItemKind::Issue, 5),
        (ItemKind::Issue, 6),
        (ItemKind::MergeRequest, 3),
        (ItemKind::MergeRequest, 6),
    ] {
        for ready in [false, true] {
            let mut ui = mount(kind, field);
            replace(&mut ui, if ready { "  " } else { "" });
            if ready {
                wait(&mut ui);
                assert!(!completion(&ui).options.is_empty());
            } else {
                assert!(completion(&ui).pending);
                key(&mut ui, KeyCode::Down);
            }
            assert_eq!(completion(&ui).selected, None);
            key(&mut ui, KeyCode::Enter);
            assert!(value(&ui).trim().is_empty());
            assert_eq!(ids(&ui), "");
            assert!(
                ui.state()
                    .dialog
                    .as_ref()
                    .unwrap()
                    .error
                    .as_ref()
                    .unwrap()
                    .contains("Demo is read-only"),
                "Enter should submit the empty value, not insert a suggestion"
            );
            assert!(!ui.state().mutation_pending);
        }
    }
}

#[test]
fn enter_requires_explicit_selection_even_when_only_one_suggestion_matches() {
    let mut ui = mount(ItemKind::MergeRequest, 3);
    replace(&mut ui, "Sam Fiction");
    wait(&mut ui);
    assert_eq!(completion(&ui).options.len(), 1);
    assert_eq!(completion(&ui).selected, None);
    let rect = ui.rect_of_key(&"lookup-0-103".into()).unwrap();
    assert_eq!(
        ui.capture_frame().cell(rect.x as u16, rect.y as u16).symbol,
        " "
    );
    key(&mut ui, KeyCode::Enter);
    assert_eq!(value(&ui), "Sam Fiction");
    assert!(
        ui.state()
            .dialog
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("Choose a lookup suggestion")
    );
    assert!(!ui.state().mutation_pending);
    key(&mut ui, KeyCode::Down);
    assert_eq!(completion(&ui).selected, Some(0));
    key(&mut ui, KeyCode::Enter);
    assert_eq!(value(&ui), "@demo-sam");
    assert_eq!(ids(&ui), "103");
    assert!(!completion(&ui).open);
    assert_eq!(completion(&ui).selected, None);
    assert!(ui.state().dialog.as_ref().unwrap().error.is_none());
}

#[test]
fn changing_query_resets_explicit_selection_and_up_selects_the_last_result() {
    let mut ui = mount(ItemKind::MergeRequest, 3);
    replace(&mut ui, "Person");
    let epoch = completion(&ui).epoch;
    ui.dispatch(Msg::LookupLoaded(
        epoch,
        0,
        Ok(vec![
            option(51, "Person one", "@one"),
            option(52, "Person two", "@two"),
        ]),
    ))
    .unwrap();
    settle(&mut ui);
    key(&mut ui, KeyCode::Up);
    assert_eq!(completion(&ui).selected, Some(1));
    key(&mut ui, KeyCode::Enter);
    assert_eq!(value(&ui), "@two");
    assert_eq!(ids(&ui), "52");
    replace(&mut ui, "Sam Fiction");
    wait(&mut ui);
    key(&mut ui, KeyCode::Down);
    assert_eq!(completion(&ui).selected, Some(0));
    replace(&mut ui, "");
    assert_eq!(completion(&ui).selected, None);
    key(&mut ui, KeyCode::Enter);
    assert_eq!(value(&ui), "");
    assert_eq!(ids(&ui), "");
    assert!(
        ui.state()
            .dialog
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("Demo is read-only")
    );
}

#[test]
fn suggestions_use_full_height_selection_edges_and_row_fill() {
    for field in [2, 3, 4, 5, 6] {
        assert_suggestion_selection(field);
    }
}

fn assert_suggestion_selection(field: usize) {
    let mut ui = mount(ItemKind::Issue, field);
    // Distinct overrides make the edge and row fill unambiguous in every cell.
    ui.state_mut().config.colors.surface = Some("#111213".into());
    ui.state_mut().config.colors.selection = Some("#212223".into());
    ui.state_mut().config.colors.accent = Some("#515253".into());
    replace(&mut ui, "Suggestion");
    let epoch = completion(&ui).epoch;
    let mut options = vec![
        option(51, "Suggestion one", "one"),
        option(52, "Suggestion two", "two"),
    ];
    if field == 2 {
        for option in &mut options {
            option.color = "#123456".into();
            option.text_color = "#abcdef".into();
        }
    }
    ui.dispatch(Msg::LookupLoaded(epoch, 0, Ok(options)))
        .unwrap();
    settle(&mut ui);
    assert_eq!(completion(&ui).selected, None);
    for selected in [0, 1] {
        key(&mut ui, KeyCode::Down);
        let c = completion(&ui);
        assert_eq!(c.selected, Some(selected));
        let rect = ui
            .rect_of_key(&format!("lookup-0-{}", c.options[selected].id).into())
            .unwrap();
        let frame = ui.capture_frame();
        let x = u16::try_from(rect.x).unwrap();
        let top = u16::try_from(rect.y).unwrap();
        for y in top..top + rect.h {
            let edge = frame.cell(x, y);
            assert_eq!(edge.symbol, "▕");
            assert_eq!(edge.bg, Color::hex_u24(0x111213));
            assert_eq!(edge.fg, Color::hex_u24(0x515253));
            assert_eq!(frame.cell(x + rect.w - 2, y).bg, Color::hex_u24(0x212223));
        }
        if c.kind == LookupKind::Labels {
            let option = &c.options[selected];
            let color = u32::from_str_radix(option.color.trim_start_matches('#'), 16).unwrap();
            assert_eq!(frame.cell(x + 3, top).bg, Color::hex_u24(color));
        }
    }
}

#[test]
fn current_user_suggestion_is_yellow_but_accepts_the_original_login_and_id() {
    let mut ui = mount(ItemKind::Issue, 3);
    replace(&mut ui, "Arin");
    wait(&mut ui);
    let current = ui.state().user.clone();
    let rect = ui
        .rect_of_key(&format!("lookup-0-{}", current.id).into())
        .unwrap();
    let frame = ui.capture_frame();
    let line = &frame.to_lines()[rect.y as usize];
    let x = line
        .find("You")
        .expect("current user suggestion should say You");
    let column = line[..x].chars().count();
    for offset in 0..3 {
        assert_eq!(
            frame.cell((column + offset) as u16, rect.y as u16).fg,
            Color::hex_u24(0xf0c674)
        );
    }
    key(&mut ui, KeyCode::Down);
    key(&mut ui, KeyCode::Enter);
    assert_eq!(value(&ui), format!("@{}", current.username));
    assert_eq!(ids(&ui), current.id.to_string());
}

#[test]
fn people_complete_trimmed_comma_tokens_without_submitting_or_losing_previous_values() {
    let mut ui = mount(ItemKind::Issue, 3);
    assert_eq!(value(&ui), "@demo-arin");
    assert_eq!(ids(&ui), "101");
    replace(&mut ui, " @demo-arin,   Sam Fiction  ");
    assert_eq!(completion(&ui).query.as_deref(), Some("Sam Fiction"));
    assert!(completion(&ui).pending);
    key(&mut ui, KeyCode::Enter);
    assert!(
        !ui.state().mutation_pending,
        "Enter while loading cannot write"
    );
    wait(&mut ui);
    assert!(ui.capture_frame().plain_text().contains("Sam Fiction"));
    assert_eq!(completion(&ui).options.len(), 1);
    key(&mut ui, KeyCode::Down);
    key(&mut ui, KeyCode::Enter);
    assert_eq!(value(&ui), " @demo-arin,   @demo-sam  ");
    assert_eq!(ids(&ui), "101,103");
    assert!(!completion(&ui).open);
    assert!(ui.state().dialog.as_ref().unwrap().error.is_none());
    assert_eq!(ui.focused_key(), Some(&"dialog-field-0".into()));
    key(&mut ui, KeyCode::Enter);
    assert!(
        ui.state()
            .dialog
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("Demo is read-only")
    );
}

#[test]
fn labels_require_known_choices_and_accept_colored_suggestions() {
    let mut ui = mount(ItemKind::Issue, 2);
    assert_eq!(completion(&ui).kind, LookupKind::Labels);
    assert!(value(&ui).contains("demo::fictional"));
    replace(&mut ui, "demo::fictional, type");
    wait(&mut ui);
    assert!(
        completion(&ui)
            .options
            .iter()
            .any(|option| option.value == "type::bug" && !option.color.is_empty())
    );
    key(&mut ui, KeyCode::Down);
    key(&mut ui, KeyCode::Enter);
    assert_eq!(value(&ui), "demo::fictional, type::bug");
    assert_eq!(ids(&ui), "demo::fictional,type::bug");
    replace(&mut ui, "demo::fictional, typo");
    wait(&mut ui);
    key(&mut ui, KeyCode::Enter);
    assert!(
        ui.state()
            .dialog
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("known label")
    );
}

#[test]
fn milestone_iteration_epic_and_reviewers_keep_readable_current_values_and_ids() {
    for (kind, field, lookup, expected) in [
        (ItemKind::Issue, 4, LookupKind::Milestones, "401"),
        (ItemKind::Issue, 5, LookupKind::Iterations, "1200"),
        (ItemKind::Issue, 6, LookupKind::Epics, "701"),
        (ItemKind::MergeRequest, 6, LookupKind::Users, "102,103"),
    ] {
        let mut ui = mount(kind, field);
        assert_eq!(completion(&ui).kind, lookup);
        assert_eq!(ids(&ui), expected);
        assert!(!value(&ui).is_empty());
        assert!(!ui.state().dialog.as_ref().unwrap().title.contains("(ID"));
        replace(
            &mut ui,
            if lookup == LookupKind::Users {
                "Mila"
            } else {
                "Demo"
            },
        );
        wait(&mut ui);
        assert!(!completion(&ui).options.is_empty());
        key(&mut ui, KeyCode::Down);
        key(&mut ui, KeyCode::Enter);
        assert!(!completion(&ui).open);
        assert!(ui.state().dialog.is_some());
        replace(&mut ui, "  ");
        assert_eq!(
            ids(&ui),
            "",
            "clearing a field retains the API's clear semantics"
        );
        key(&mut ui, KeyCode::Esc);
        assert!(ui.state().dialog.is_none());
    }
}

#[test]
fn epic_editor_resolves_same_title_choices_and_retains_draft_on_unavailable_lookup() {
    let mut ui = mount(ItemKind::Issue, 6);
    assert_eq!(
        ui.state().dialog.as_ref().unwrap().title,
        "Edit Parent (epic)"
    );
    replace(&mut ui, "Same title");
    let epoch = completion(&ui).epoch;
    ui.dispatch(Msg::LookupLoaded(
        epoch,
        0,
        Ok(vec![
            option(900, "Same title", "Same title · group 60 &1"),
            option(901, "Same title", "Same title · group 6 &1"),
        ]),
    ))
    .unwrap();
    settle(&mut ui);
    key(&mut ui, KeyCode::Down);
    key(&mut ui, KeyCode::Down);
    key(&mut ui, KeyCode::Enter);
    assert_eq!(value(&ui), "Same title · group 6 &1");
    assert_eq!(ids(&ui), "901");
    assert!(!ui.state().mutation_pending);
    replace(&mut ui, "Unfinished parent");
    let epoch = completion(&ui).epoch;
    ui.dispatch(Msg::LookupLoaded(
        epoch,
        0,
        Err("HTTP 403: epics unavailable".into()),
    ))
    .unwrap();
    settle(&mut ui);
    assert_eq!(value(&ui), "Unfinished parent");
    assert!(completion(&ui).error.as_ref().unwrap().contains("403"));
    assert!(
        ui.state().dialog.as_ref().unwrap().fields[0]
            .api_value()
            .is_err()
    );
    replace(&mut ui, "  ");
    assert_eq!(ids(&ui), "");
}

#[test]
fn iteration_lookup_offers_symbolic_current_and_resolves_it_case_insensitively() {
    let mut ui = mount(ItemKind::Issue, 5);
    replace(&mut ui, "current");
    wait(&mut ui);
    let first = completion(&ui).options[0].clone();
    assert_eq!(first.id, 1200);
    assert_eq!(first.description, "Current iteration · demo");
    assert!(
        !first.description.contains("1200"),
        "no raw IDs in suggestions"
    );
    assert_eq!(
        completion(&ui)
            .options
            .iter()
            .filter(|o| o.id == 1200)
            .count(),
        1
    );
    key(&mut ui, KeyCode::Down);
    key(&mut ui, KeyCode::Enter);
    assert_eq!(value(&ui), first.value);
    assert_eq!(ids(&ui), "1200");
    // Typing the symbolic name without choosing a suggestion still resolves.
    replace(&mut ui, "CURRENT");
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ids(&ui), "1200");
}

#[test]
fn suggestions_debounce_cache_and_discard_old_queries_and_closed_dialogs() {
    let mut ui = mount(ItemKind::Issue, 3);
    replace(&mut ui, "S");
    let old = completion(&ui).epoch;
    ui.advance(Duration::from_millis(150));
    ui.send_paste("am").unwrap();
    settle(&mut ui);
    let current = completion(&ui).epoch;
    assert_ne!(old, current);
    ui.advance(Duration::from_millis(160));
    settle(&mut ui);
    assert!(
        completion(&ui).pending,
        "the second input restarts the debounce"
    );
    ui.dispatch(Msg::LookupLoaded(
        old,
        0,
        Ok(vec![option(999, "Wrong", "@wrong")]),
    ))
    .unwrap();
    assert!(completion(&ui).options.is_empty());
    wait(&mut ui);
    assert_eq!(completion(&ui).options[0].id, 103);
    let count = completion(&ui).cache.len();
    ui.dispatch(Msg::Tick).unwrap();
    ui.dispatch(Msg::Refresh).unwrap();
    settle(&mut ui);
    assert_eq!(completion(&ui).epoch, current);
    assert_eq!(completion(&ui).cache.len(), count);
    key(&mut ui, KeyCode::Esc);
    key(&mut ui, KeyCode::Enter);
    ui.dispatch(Msg::LookupLoaded(current, 0, Err("old failure".into())))
        .unwrap();
    assert!(!completion(&ui).open);
    assert!(completion(&ui).error.is_none());
    assert_eq!(ids(&ui), "101");
}

#[test]
fn duplicate_labels_require_explicit_choice_and_mouse_only_accepts() {
    let mut ui = mount(ItemKind::Issue, 4);
    replace(&mut ui, "Release");
    let epoch = completion(&ui).epoch;
    ui.dispatch(Msg::LookupLoaded(
        epoch,
        0,
        Ok(vec![
            option(51, "Release", "Release"),
            option(52, "Release", "Release"),
        ]),
    ))
    .unwrap();
    settle(&mut ui);
    let rect = ui.rect_of_key(&"lookup-0-52".into()).unwrap();
    for kind in [
        MouseKind::Down(MouseButton::Left),
        MouseKind::Up(MouseButton::Left),
    ] {
        ui.send_mouse(MouseEvent {
            x: (rect.x + 2) as u16,
            y: rect.y as u16,
            kind,
            mods: KeyMods::NONE,
        })
        .unwrap();
    }
    settle(&mut ui);
    assert_eq!(ids(&ui), "52");
    assert_eq!(value(&ui), "Release");
    assert!(!ui.state().mutation_pending);
    assert!(ui.state().dialog.as_ref().unwrap().error.is_none());
}

#[test]
fn slow_lookup_coalesces_queries_and_ignores_stale_acceptance() {
    let mut ui = mount(ItemKind::Issue, 3);
    replace(&mut ui, "old");
    let old = completion(&ui).epoch;
    ui.state_mut().lookup_inflight = Some(old);
    replace(&mut ui, "Sam Fiction");
    let latest = completion(&ui).epoch;
    wait(&mut ui);
    assert!(completion(&ui).pending);
    assert!(completion(&ui).options.is_empty());
    ui.dispatch(Msg::LookupLoaded(
        old,
        0,
        Ok(vec![option(999, "Old", "@old")]),
    ))
    .unwrap();
    settle(&mut ui);
    assert!(ui.state().lookup_inflight.is_none());
    assert_eq!(completion(&ui).epoch, latest);
    assert_eq!(completion(&ui).options[0].id, 103);
    ui.dispatch(Msg::LookupAccept(0, old, 0)).unwrap();
    assert_eq!(
        value(&ui),
        "Sam Fiction",
        "a stale click must not choose a different result"
    );
    key(&mut ui, KeyCode::Down);
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ids(&ui), "103");
}

#[test]
fn rate_limited_lookup_retains_draft_and_respects_retry_after() {
    let mut ui = mount(ItemKind::Issue, 5);
    replace(&mut ui, "Sprint");
    let epoch = completion(&ui).epoch;
    ui.state_mut().lookup_inflight = Some(epoch);
    ui.dispatch(Msg::LookupLoaded(
        epoch,
        0,
        Err("GET iterations: HTTP 429; Retry-After: 60 (retry later; no automatic retry)".into()),
    ))
    .unwrap();
    assert!(ui.state().blocked_until >= Duration::from_secs(60));
    assert!(completion(&ui).error.as_ref().unwrap().contains("429"));
    assert_eq!(value(&ui), "Sprint");
    assert!(!ui.state().mutation_pending);
    ui.state_mut().demo = false;
    replace(&mut ui, "Sprint two");
    wait(&mut ui);
    assert!(completion(&ui).error.as_ref().unwrap().contains("backoff"));
    assert!(ui.state().lookup_inflight.is_none());
}

#[test]
fn short_suggestion_view_scrolls_with_keyboard_and_preserves_input_focus() {
    let mut ui = mount(ItemKind::Issue, 3);
    ui.set_viewport(Rect {
        x: 0,
        y: 0,
        w: 100,
        h: 16,
    });
    settle(&mut ui);
    replace(&mut ui, "Person");
    let epoch = completion(&ui).epoch;
    ui.dispatch(Msg::LookupLoaded(
        epoch,
        0,
        Ok((0..20)
            .map(|i| option(i + 1, &format!("Person {i:02}"), &format!("@person-{i}")))
            .collect()),
    ))
    .unwrap();
    settle(&mut ui);
    for _ in 0..20 {
        key(&mut ui, KeyCode::Down);
    }
    assert_eq!(completion(&ui).selected, Some(19));
    assert!(ui.capture_frame().plain_text().contains("Person 19"));
    assert_eq!(ui.focused_key(), Some(&"dialog-field-0".into()));
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ids(&ui), "20");
    assert!(ui.state().dialog.as_ref().unwrap().error.is_none());
}

#[test]
fn unresolved_names_never_silently_become_ids_or_clear_existing_values() {
    let mut ui = mount(ItemKind::Issue, 3);
    replace(&mut ui, "@demo-arin, no-such-person");
    wait(&mut ui);
    assert!(completion(&ui).options.is_empty());
    key(&mut ui, KeyCode::Enter);
    assert!(!ui.state().mutation_pending);
    assert!(
        ui.state()
            .dialog
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("Choose a lookup suggestion")
    );
    assert_eq!(value(&ui), "@demo-arin, no-such-person");
    let epoch = completion(&ui).epoch;
    ui.dispatch(Msg::LookupLoaded(
        epoch,
        0,
        Err("Lookup unavailable".into()),
    ))
    .unwrap();
    assert_eq!(completion(&ui).error.as_deref(), Some("Lookup unavailable"));
    assert_eq!(value(&ui), "@demo-arin, no-such-person");
}

#[test]
fn creation_project_picker_matches_workspace_aliases_and_tab_keeps_form_navigation() {
    let mut ui = mount(ItemKind::MergeRequest, 0);
    key(&mut ui, KeyCode::Esc);
    key(&mut ui, KeyCode::Esc);
    key(&mut ui, KeyCode::Char('n'));
    assert_eq!(completion(&ui).kind, LookupKind::Projects);
    assert_eq!(ids(&ui), "9001");
    replace(&mut ui, "Meteor");
    wait(&mut ui);
    assert_eq!(completion(&ui).options.len(), 1);
    key(&mut ui, KeyCode::Down);
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ids(&ui), "9002");
    assert_eq!(value(&ui), demo::projects()[1].path);
    key(&mut ui, KeyCode::Tab);
    assert_eq!(ui.state().dialog.as_ref().unwrap().selected, 1);
    assert_eq!(ui.focused_key(), Some(&"dialog-field-1".into()));
    ui.send_paste("New merge request").unwrap();
    assert!(!ui.state().mutation_pending);
}
