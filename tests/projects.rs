//! Project visibility, shared detail presentation, and settings navigation.
use cronk::{
    config::{Config, SavedView, StarredItem, THEMES},
    demo,
    model::{ItemKey, ItemKind, Project, WorkItem},
    ui::{Confirmation, Cronk, DialogKind, Msg, Scope},
};
use tui_lipan::{
    TestBackend,
    core::event::{MouseButton, MouseKind},
    prelude::*,
};

fn mount(project: Project, height: u16) -> TestBackend<Cronk> {
    let mut ui = TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config: Config {
                projects: vec![project],
                active_tab: 1,
                onboarding: false,
                animations: false,
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
            w: 120,
            h: height,
        },
    );
    settle(&mut ui);
    ui
}

fn settle(ui: &mut TestBackend<Cronk>) {
    for _ in 0..3 {
        ui.render();
        ui.pump().unwrap();
    }
}

fn click(ui: &mut TestBackend<Cronk>, key: &str) {
    settle(ui);
    let rect = ui.rect_of_key(&key.to_owned().into()).unwrap();
    let x = rect.x as u16 + rect.w / 2;
    let y = rect.y as u16 + rect.h / 2;
    for kind in [
        MouseKind::Down(MouseButton::Left),
        MouseKind::Up(MouseButton::Left),
    ] {
        ui.send_mouse(MouseEvent {
            kind,
            x,
            y,
            mods: KeyMods::NONE,
        })
        .unwrap();
    }
    settle(ui);
}

#[test]
fn legacy_projects_default_to_both_collections_and_preferences_round_trip() {
    for visible in [false, true] {
        let mut config: Config = toml::from_str(&format!(
            "onboarding = false\n[[projects]]\nid = 9001\npath = 'team/project'\nvisible = {visible}"
        )).unwrap();
        let project = &config.projects[0];
        assert!(project.issues_visible && project.merge_requests_visible);
        assert_eq!(project.kind_visible(ItemKind::Issue), visible);
        assert_eq!(project.kind_visible(ItemKind::MergeRequest), visible);
        config.projects[0].issues_visible = false;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        config.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.projects, config.projects);
    }
}

#[test]
fn project_list_and_details_share_visibility_dot_color_and_lowercase_status() {
    for master in [false, true] {
        for issues in [false, true] {
            for mrs in [false, true] {
                let project = Project {
                    visible: master,
                    issues_visible: issues,
                    merge_requests_visible: mrs,
                    ..demo::projects().remove(0)
                };
                let expected = if !master || (!issues && !mrs) {
                    "○"
                } else if issues && mrs {
                    "●"
                } else {
                    "◐"
                };
                let mut ui = mount(project, 40);
                let rect = ui.rect_of_key(&"project-visible-tip-9001".into()).unwrap();
                let frame = ui.capture_frame();
                let cell = frame.cell(rect.x as u16, rect.y as u16).clone();
                assert_eq!(cell.symbol, expected);
                ui.dispatch(Msg::Enter).unwrap();
                settle(&mut ui);
                let rect = ui.rect_of_key(&"detail-project-status".into()).unwrap();
                let frame = ui.capture_frame();
                let detail_cell = frame.cell(rect.x as u16, rect.y as u16);
                assert_eq!(detail_cell.symbol, expected);
                assert_eq!(detail_cell.fg, cell.fg);
                let text = frame.plain_text();
                assert!(
                    !text.contains("Visible") && !text.contains("Hidden"),
                    "{text}"
                );
                let status_row = text.lines().find(|line| line.contains(expected)).unwrap();
                assert!(status_row.contains(if master && (issues || mrs) {
                    "visible"
                } else {
                    "hidden"
                }));
            }
        }
    }
}

#[test]
fn keyboard_space_and_enter_toggle_only_the_focused_project_visibility_control() {
    for key in [KeyCode::Char(' '), KeyCode::Enter] {
        for field in 1..=3 {
            let mut ui = mount(demo::projects().remove(0), 40);
            ui.dispatch(Msg::Enter).unwrap();
            ui.dispatch(Msg::Enter).unwrap();
            ui.dispatch(Msg::Move(field)).unwrap();
            ui.send_key(KeyEvent {
                code: key,
                mods: KeyMods::NONE,
            })
            .unwrap();
            settle(&mut ui);
            let project = ui.state().project(9001).unwrap();
            assert_eq!(
                project.visible,
                field != 1,
                "{key:?}, field {field}: master"
            );
            assert_eq!(
                project.issues_visible,
                field != 2,
                "{key:?}, field {field}: issues"
            );
            assert_eq!(
                project.merge_requests_visible,
                field != 3,
                "{key:?}, field {field}: MRs"
            );
            let rect = ui.rect_of_key(&"detail-project-status".into()).unwrap();
            let frame = ui.capture_frame();
            assert_eq!(
                frame.cell(rect.x as u16, rect.y as u16).symbol,
                if field == 1 { "○" } else { "◐" }
            );
            ui.send_key(KeyEvent {
                code: key,
                mods: KeyMods::NONE,
            })
            .unwrap();
            let project = ui.state().project(9001).unwrap();
            assert!(project.visible && project.issues_visible && project.merge_requests_visible);
        }
    }
}

#[test]
fn space_on_alias_or_non_field_sections_does_not_toggle_master_visibility() {
    let mut ui = mount(demo::projects().remove(0), 40);
    ui.dispatch(Msg::Enter).unwrap();
    for section in 0..=2 {
        ui.dispatch(Msg::Section(section)).unwrap();
        ui.send_key(KeyEvent {
            code: KeyCode::Char(' '),
            mods: KeyMods::NONE,
        })
        .unwrap();
        let project = ui.state().project(9001).unwrap();
        assert!(project.visible && project.issues_visible && project.merge_requests_visible);
        assert!(ui.state().dialog.is_none());
    }
}

#[test]
fn project_rows_keep_a_blank_gap_and_mark_all_content_rows_even_when_hidden() {
    for (master, issues) in [(true, true), (true, false), (false, true)] {
        let mut ui = mount(
            Project {
                visible: master,
                issues_visible: issues,
                ..demo::projects().remove(0)
            },
            40,
        );
        ui.state_mut()
            .config
            .projects
            .push(demo::projects().remove(1));
        settle(&mut ui);
        let first = ui.rect_of_key(&"project-9001".into()).unwrap();
        let second = ui.rect_of_key(&"project-9002".into()).unwrap();
        assert_eq!(first.h, 4, "three content rows plus one blank gap");
        assert_eq!(second.y, first.y + 4);
        let frame = ui.capture_frame();
        for y in first.y as u16..first.y as u16 + 3 {
            assert_eq!(
                frame.cell(first.x as u16, y).symbol,
                "▕",
                "{master}/{issues}: row {y}"
            );
        }
        let gap = first.y as u16 + 3;
        assert_eq!(frame.cell(first.x as u16, gap).symbol, " ");
        assert!(
            frame.to_lines()[gap as usize].trim().is_empty(),
            "blank separator row"
        );
    }
}

#[test]
fn toggles_preserve_preferences_and_hide_items_on_every_work_tab_without_evicting_cache() {
    let mut ui = mount(demo::projects().remove(0), 40);
    let original = ui.state().items.clone();
    let project_items: Vec<_> = original.iter().filter(|i| i.key.project == 9001).collect();
    assert!(project_items.iter().any(|i| i.key.kind == ItemKind::Issue));
    assert!(
        project_items
            .iter()
            .any(|i| i.key.kind == ItemKind::MergeRequest)
    );
    ui.dispatch(Msg::Enter).unwrap();
    click(&mut ui, "edit-field-2");
    assert!(!ui.state().project(9001).unwrap().issues_visible);
    assert_eq!(ui.state().config.field, 2);
    assert_eq!(ui.state().scope, Scope::Section);
    click(&mut ui, "edit-field-1");
    assert!(!ui.state().project(9001).unwrap().visible);
    assert!(ui.state().project(9001).unwrap().merge_requests_visible);
    click(&mut ui, "edit-field-1");
    assert!(!ui.state().project(9001).unwrap().issues_visible);
    assert!(ui.state().project(9001).unwrap().merge_requests_visible);

    ui.state_mut().config.views.push(SavedView {
        name: "All issues".into(),
        kind: ItemKind::Issue,
        query: String::new(),
    });
    ui.state_mut().config.starred = project_items
        .iter()
        .map(|i| StarredItem {
            project: ui.state().project(9001).unwrap().path.clone(),
            kind: i.key.kind,
            iid: i.key.iid,
            title: i.title.clone(),
        })
        .collect();
    for tab in [0, 2, 3, 4, 5] {
        ui.dispatch(Msg::Tab(tab)).unwrap();
        assert!(
            ui.state()
                .visible_items()
                .iter()
                .all(|i| i.key.kind == ItemKind::MergeRequest)
        );
        if matches!(tab, 2 | 4) {
            assert!(ui.state().visible_items().is_empty());
        }
    }
    assert_eq!(ui.state().items, original);
    ui.dispatch(Msg::Tab(1)).unwrap();
    ui.dispatch(Msg::ToggleProjectKind(ItemKind::Issue))
        .unwrap();
    ui.dispatch(Msg::Tab(2)).unwrap();
    assert!(!ui.state().visible_items().is_empty());
    assert_eq!(ui.state().items, original);
}

#[test]
fn stale_skipped_or_paused_sync_results_cannot_evict_items_after_visibility_changes() {
    for (master, paused) in [(false, false), (false, true), (true, true)] {
        let mut ui = mount(
            Project {
                issues_visible: paused,
                ..demo::projects().remove(0)
            },
            40,
        );
        ui.dispatch(Msg::Enter).unwrap();
        let cached = ui.state().items.clone();
        let epoch = ui.state().list_epoch;
        ui.state_mut().list_pending.insert(9001);
        if paused {
            ui.dispatch(if master {
                Msg::ToggleProject
            } else {
                Msg::ToggleProjectKind(ItemKind::Issue)
            })
            .unwrap();
        }
        ui.dispatch(if master {
            Msg::ToggleProject
        } else {
            Msg::ToggleProjectKind(ItemKind::Issue)
        })
        .unwrap();
        assert!(
            ui.state()
                .project(9001)
                .unwrap()
                .kind_visible(ItemKind::Issue)
        );
        assert!(ui.state().list_epoch > epoch);
        assert!(
            ui.state().list_pending.is_empty(),
            "obsolete imports must not block replacement sync"
        );
        let old_result: Vec<_> = cached
            .iter()
            .filter(|i| i.key.project == 9001)
            .filter(|i| {
                if paused {
                    i.key.iid == 1
                } else {
                    i.key.kind == ItemKind::MergeRequest
                }
            })
            .cloned()
            .collect();
        ui.dispatch(Msg::ProjectLoaded(9001, epoch, Ok((old_result, None))))
            .unwrap();
        ui.dispatch(Msg::ProjectCached(9001, epoch, Ok(Vec::new())))
            .unwrap();
        assert_eq!(
            ui.state().items,
            cached,
            "stale partial results must leave cached history intact"
        );
    }
}

#[test]
fn hiding_one_collection_invalidates_only_its_detail_routes_and_restores_the_other() {
    let mut ui = mount(demo::projects().remove(0), 40);
    ui.dispatch(Msg::Tab(3)).unwrap();
    ui.dispatch(Msg::Enter).unwrap();
    let mr = ui.state().config.route.clone().unwrap();
    ui.dispatch(Msg::Tab(2)).unwrap();
    ui.dispatch(Msg::Enter).unwrap();
    let issue = ui.state().config.route.clone().unwrap();
    ui.dispatch(Msg::Tab(1)).unwrap();
    ui.dispatch(Msg::Enter).unwrap();
    ui.dispatch(Msg::ToggleProjectKind(ItemKind::Issue))
        .unwrap();
    assert!(ui.state().config.tab_states["2"].route.is_none());
    assert_eq!(ui.state().config.tab_states["3"].route, Some(mr.clone()));
    // Stale detail completions cannot restore the now-hidden route.
    ui.dispatch(Msg::DetailFinished(
        issue.clone(),
        0,
        Ok(Box::new(demo::details(&issue))),
    ))
    .unwrap();
    ui.dispatch(Msg::Tab(2)).unwrap();
    assert_eq!(ui.state().scope, Scope::List);
    assert!(ui.state().config.route.is_none());
    ui.dispatch(Msg::Tab(3)).unwrap();
    assert_eq!(ui.state().config.route, Some(mr));
    assert_eq!(ui.state().scope, Scope::Details);
    ui.dispatch(Msg::Tab(1)).unwrap();
    ui.dispatch(Msg::ToggleProjectKind(ItemKind::Issue))
        .unwrap();
    ui.dispatch(Msg::Tab(2)).unwrap();
    assert_eq!(ui.state().scope, Scope::List);
    assert!(
        ui.state().config.route.is_none(),
        "showing items must not resurrect old routes"
    );
}

#[test]
fn open_work_summary_is_identical_and_only_counts_open_enabled_item_types() {
    for (issues, mrs, expected) in [
        (true, true, "1 open issue · 2 open merge requests"),
        (true, false, "1 open issue"),
        (false, true, "2 open merge requests"),
        (false, false, ""),
    ] {
        let mut ui = mount(
            Project {
                issues_visible: issues,
                merge_requests_visible: mrs,
                ..demo::projects().remove(0)
            },
            40,
        );
        ui.state_mut().items = [
            (9001, ItemKind::Issue, "opened"),
            (9001, ItemKind::Issue, "closed"),
            (9001, ItemKind::MergeRequest, "opened"),
            (9001, ItemKind::MergeRequest, "opened"),
            (9001, ItemKind::MergeRequest, "merged"),
            (9001, ItemKind::MergeRequest, "closed"),
            (9002, ItemKind::Issue, "opened"),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (project, kind, state))| WorkItem {
            key: ItemKey {
                project,
                iid: i as u64 + 1,
                kind,
            },
            state: state.into(),
            ..WorkItem::default()
        })
        .collect();
        settle(&mut ui);
        let list = ui.capture_frame().plain_text();
        assert!(!list.contains(" items"));
        ui.dispatch(Msg::Enter).unwrap();
        settle(&mut ui);
        let details = ui.capture_frame().plain_text();
        if expected.is_empty() {
            assert!(!list.contains("open issue") && !list.contains("open merge request"));
            assert!(!details.contains("Open work"));
        } else {
            assert!(list.contains(expected), "{list}");
            assert!(details.contains(expected), "{details}");
        }
        ui.dispatch(Msg::ToggleProject).unwrap();
        settle(&mut ui);
        assert!(!ui.capture_frame().plain_text().contains("Open work"));
    }
}

#[test]
fn projects_tab_shortcut_and_click_pop_details_to_list() {
    for method in 0..3 {
        let mut ui = mount(demo::projects().remove(0), 40);
        ui.dispatch(Msg::Enter).unwrap();
        ui.dispatch(Msg::Section(2)).unwrap();
        match method {
            0 => ui
                .send_key(KeyEvent {
                    code: KeyCode::Char('P'),
                    mods: KeyMods::NONE,
                })
                .unwrap(),
            1 => ui
                .send_key(KeyEvent {
                    code: KeyCode::Char('p'),
                    mods: KeyMods::SHIFT,
                })
                .unwrap(),
            _ => {
                click(&mut ui, "workspace-tab-1");
                true
            }
        };
        settle(&mut ui);
        assert_eq!(ui.state().scope, Scope::List);
        assert!(ui.state().config.project_route.is_none());
        assert_eq!(ui.state().scroll.selected, 0);
    }
}

#[test]
fn project_fields_are_left_aligned_editable_first_and_match_shared_label_width() {
    let mut ui = mount(demo::projects().remove(0), 40);
    ui.dispatch(Msg::Enter).unwrap();
    settle(&mut ui);
    let text = ui.capture_frame().plain_text();
    let labels = [
        "Alias",
        "Visibility",
        "Issues",
        "Merge requests",
        "Path",
        "Project ID",
        "Open work",
    ];
    let mut last = 0;
    let mut column = None;
    for label in labels {
        let (y, row) = text
            .lines()
            .enumerate()
            .find(|(y, row)| *y > 4 && row.contains(label))
            .unwrap();
        assert!(y > last, "{text}");
        if label == "Path" {
            assert_eq!(y - last, 2, "blank line before readonly fields");
        }
        last = y;
        let x = row[..row.find(label).unwrap()].chars().count();
        assert_eq!(*column.get_or_insert(x), x, "{label}: {text}");
    }
    let alias = text.lines().find(|row| row.contains("Alias")).unwrap();
    let value_x = alias[..alias.find("Orbit scheduler").unwrap()]
        .chars()
        .count();
    assert_eq!(value_x - column.unwrap(), "Merge requests".len() + 2);
    assert!(text.find("Pipelines").unwrap() > text.find("Open work").unwrap());
    assert!(
        text.contains("also syncs pipelines"),
        "MR visibility annotates pipeline sync: {text}"
    );
    // Pipelines now fill the section, pushing the destructive button below the fold.
    assert!(ui.rect_of_key(&"pipeline-60999".into()).is_some(), "{text}");
}

#[test]
fn forget_button_is_three_rows_centered_contrasting_and_accessible_on_small_screens() {
    for theme in THEMES {
        let mut ui = mount(demo::projects().remove(0), 40);
        ui.state_mut().config.theme = theme.into();
        ui.dispatch(Msg::Enter).unwrap();
        settle(&mut ui);
        // Pipelines fill the document: focus the last section to reveal the button.
        ui.dispatch(Msg::Move(2)).unwrap();
        ui.dispatch(Msg::Enter).unwrap();
        settle(&mut ui);
        let rect = ui.rect_of_key(&"project-remove".into()).unwrap();
        assert_eq!(rect.h, 3);
        let frame = ui.capture_frame();
        let row = rect.y as u16 + 1;
        let text = "Forget this project";
        let line = frame
            .plain_text()
            .lines()
            .nth(row as usize)
            .unwrap()
            .to_owned();
        let x = line[..line.find(text).unwrap()].chars().count() as u16;
        // The detail gutter occupies four cells; the button fills the remaining width.
        let left = rect.x as u16 + 4;
        assert!(
            (i32::from(x - left) - i32::from(rect.x as u16 + rect.w - x - text.len() as u16)).abs()
                <= 1
        );
        let cell = frame.cell(x, row);
        let (light, dark) = if cell.fg.luminance() > cell.bg.luminance() {
            (cell.fg, cell.bg)
        } else {
            (cell.bg, cell.fg)
        };
        assert!(
            (light.luminance() + 0.05) / (dark.luminance() + 0.05) >= 4.5,
            "{theme}"
        );
        for y in rect.y as u16..rect.y as u16 + 3 {
            assert_eq!(frame.cell(left, y).bg, cell.bg);
            assert_eq!(frame.cell(rect.x as u16 + rect.w - 1, y).bg, cell.bg);
        }
        click(&mut ui, "project-remove");
        assert!(matches!(
            ui.state().dialog.as_ref().unwrap().kind,
            DialogKind::Confirm(Confirmation::RemoveProject(9001))
        ));
    }
    for height in [16, 24] {
        let mut ui = mount(demo::projects().remove(0), height);
        ui.dispatch(Msg::Enter).unwrap();
        settle(&mut ui);
        ui.dispatch(Msg::Move(2)).unwrap();
        ui.dispatch(Msg::Enter).unwrap();
        settle(&mut ui);
        assert_eq!(ui.state().section_name(), "Forget this project");
        let rect = ui.rect_of_key(&"project-remove".into()).unwrap();
        assert!(rect.y >= 0 && rect.y as u16 + rect.h <= height - 2);
        assert!(ui.state().content_offset > 0);
        ui.dispatch(Msg::Enter).unwrap();
        assert!(matches!(
            ui.state().dialog.as_ref().unwrap().kind,
            DialogKind::Confirm(Confirmation::RemoveProject(9001))
        ));
    }
}
