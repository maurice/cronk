//! Navigation persistence contracts use explicit flushes and deterministic
//! hydration messages, not sleeps or latency thresholds.
use cronk::{
    config::{Config, SavedView, TabState},
    demo,
    gitlab::GitLab,
    model::{ItemKey, ItemKind, WorkItem},
    ui::{Action, Confirmation, Cronk, DialogKind, Msg, Scope},
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tui_lipan::{TestBackend, prelude::*};

type Ui = TestBackend<Cronk>;

fn config() -> Config {
    Config {
        onboarding: false,
        projects: demo::projects(),
        ..Config::default()
    }
}

fn mount(config: Config, path: Option<&Path>, demo: bool) -> Box<Ui> {
    let mut ui = Box::new(TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config,
            path: path.map(Path::to_path_buf),
            api: None,
            demo,
        },
        (),
        Rect {
            x: 0,
            y: 0,
            w: 80,
            h: 24,
        },
    ));
    ui.pump().unwrap();
    ui
}

fn dbpath(path: &Path, config: &Config, demo: bool) -> PathBuf {
    let namespace = Sha256::digest(format!(
        "{}\0{demo}",
        config.gitlab_url.trim_end_matches('/')
    ));
    path.with_extension("state")
        .join(format!("{namespace:x}.sqlite3"))
}

fn selected(ui: &Ui) -> ItemKey {
    ui.state().visible_items()[ui.state().scroll.selected]
        .key
        .clone()
}

fn seed_live(path: &Path) -> ItemKey {
    seed_live_kind(path, ItemKind::Issue)
}

fn seed_live_kind(path: &Path, kind: ItemKind) -> ItemKey {
    seed_live_config(path, config(), kind)
}

fn seed_live_config(path: &Path, config: Config, kind: ItemKind) -> ItemKey {
    config.save(path).unwrap();
    let mut ui = mount(config, Some(path), false);
    ui.state_mut().items = demo::items();
    ui.state_mut().user = demo::user();
    ui.dispatch(Msg::Tab(if kind == ItemKind::Issue { 2 } else { 3 }))
        .unwrap();
    let target = ui
        .state()
        .visible_items()
        .iter()
        .position(|i| i.key.project == 9001)
        .unwrap();
    ui.dispatch(Msg::Select(target)).unwrap();
    let key = selected(&ui);
    ui.dispatch(Msg::Enter).unwrap();
    ui.dispatch(Msg::Enter).unwrap();
    ui.dispatch(Msg::Field(1)).unwrap();
    ui.state().flush_navigation().unwrap();
    key
}

fn start_hydration(ui: &mut Ui) {
    let ids: Vec<_> = ui
        .state()
        .config
        .projects
        .iter()
        .filter(|p| p.visible)
        .map(|p| p.id)
        .collect();
    ui.state_mut().list_pending.extend(ids);
    ui.dispatch(Msg::UserLoaded(Ok(demo::user()))).unwrap();
}

fn finish_project(ui: &mut Ui, project: u64, items: Vec<WorkItem>) {
    ui.dispatch(Msg::ProjectLoaded(
        project,
        ui.state().list_epoch,
        Ok((items, None)),
    ))
    .unwrap();
}

#[test]
fn selected_identity_survives_restart_reordering_and_partial_hydration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let target = seed_live(&path);
    let mut ui = mount(Config::load(&path).unwrap(), Some(&path), false);
    assert_eq!(ui.state().config.route.as_ref(), Some(&target));
    start_hydration(&mut ui);
    let mut items = demo::items();
    items
        .iter_mut()
        .find(|i| i.key == target)
        .unwrap()
        .updated_at = "2099-01-01T00:00:00Z".into();
    for project in [9002, 9003, 9004, 9005] {
        finish_project(
            &mut ui,
            project,
            items
                .iter()
                .filter(|i| i.key.project == project)
                .cloned()
                .collect(),
        );
        assert_eq!(
            ui.state().config.route.as_ref(),
            Some(&target),
            "partial pages must not delete the restored route"
        );
        ui.state().flush_navigation().unwrap(); // Partial progress must not replace the pending identity.
    }
    finish_project(
        &mut ui,
        9001,
        items
            .into_iter()
            .filter(|i| i.key.project == 9001)
            .collect(),
    );
    assert_eq!(selected(&ui), target);
    assert_eq!(
        ui.state().scroll.selected,
        0,
        "identity, not its previous index, controls restoration"
    );
    assert_eq!(ui.state().config.route.as_ref(), Some(&target));
}

#[test]
fn migrated_open_mr_without_or_with_stale_index_survives_first_refresh_reordering() {
    for has_index in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.toml");
        let mut legacy = config();
        let resolver = mount(legacy.clone(), None, true);
        let target = resolver
            .state()
            .items
            .iter()
            .find(|i| i.key.project == 9001 && i.key.kind == ItemKind::MergeRequest)
            .unwrap()
            .key
            .clone();
        legacy.active_tab = 3;
        legacy.route = Some(target.clone());
        legacy.section = Some(0);
        if has_index {
            legacy.selections.insert("3".into(), 7);
        }
        fs::write(&path, toml::to_string(&legacy).unwrap()).unwrap();
        let mut ui = mount(Config::load(&path).unwrap(), Some(&path), false);
        start_hydration(&mut ui);
        let mut items: Vec<_> = demo::items()
            .into_iter()
            .filter(|i| i.key.project == target.project)
            .collect();
        items
            .iter_mut()
            .find(|i| i.key == target)
            .unwrap()
            .updated_at = "2099-01-01T00:00:00Z".into();
        finish_project(&mut ui, target.project, items);
        assert_eq!(selected(&ui), target);
        assert_eq!(ui.state().scroll.selected, 0);
        ui.dispatch(Msg::Back).unwrap();
        ui.dispatch(Msg::Back).unwrap();
        assert_eq!(ui.state().scope, Scope::List);
        assert_eq!(selected(&ui), target);
    }
}

#[test]
fn owning_project_success_rejects_deleted_or_filtered_identity_despite_unrelated_failure() {
    for filtered in [false, true] {
        for failed_first in [false, true] {
            for late_user in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("workspace.toml");
                let mut config = config();
                if filtered {
                    config.filters.insert("2".into(), "state:opened".into());
                }
                let target = seed_live_config(&path, config, ItemKind::Issue);
                let mut ui = mount(Config::load(&path).unwrap(), Some(&path), false);
                ui.state_mut()
                    .list_pending
                    .extend([9001, 9002, 9003, 9004, 9005]);
                if !late_user {
                    ui.dispatch(Msg::UserLoaded(Ok(demo::user()))).unwrap();
                }
                if failed_first {
                    ui.dispatch(Msg::ProjectLoaded(
                        9002,
                        ui.state().list_epoch,
                        Err("unrelated offline project".into()),
                    ))
                    .unwrap();
                }
                let mut items: Vec<_> = demo::items()
                    .into_iter()
                    .filter(|i| i.key.project == target.project)
                    .collect();
                if filtered {
                    items.iter_mut().find(|i| i.key == target).unwrap().state = "closed".into();
                } else {
                    items.retain(|i| i.key != target);
                }
                finish_project(&mut ui, target.project, items);
                if !failed_first {
                    ui.dispatch(Msg::ProjectLoaded(
                        9002,
                        ui.state().list_epoch,
                        Err("unrelated offline project".into()),
                    ))
                    .unwrap();
                }
                if late_user {
                    assert_eq!(
                        ui.state().config.route.as_ref(),
                        Some(&target),
                        "authentication still guards predicate evaluation"
                    );
                    ui.dispatch(Msg::UserLoaded(Ok(demo::user()))).unwrap();
                }
                assert!(
                    !ui.state().list_pending.is_empty(),
                    "global hydration is deliberately incomplete"
                );
                assert!(
                    ui.state().config.route.is_none(),
                    "owning project proved the restored object absent from this view"
                );
                assert_eq!(ui.state().scope, Scope::List);
                assert_ne!(selected(&ui), target);
                ui.state().flush_navigation().unwrap();
                // A later import must not resurrect the rejected identity.
                finish_project(
                    &mut ui,
                    target.project,
                    demo::items()
                        .into_iter()
                        .filter(|i| i.key.project == target.project)
                        .collect(),
                );
                assert!(ui.state().config.route.is_none());
            }
        }
    }
}

#[test]
fn anonymous_legacy_position_waits_for_global_successful_hydration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let mut config = config();
    config.active_tab = 2;
    config.selections.insert("2".into(), 1);
    fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let mut ui = mount(Config::load(&path).unwrap(), Some(&path), false);
    start_hydration(&mut ui);
    finish_project(
        &mut ui,
        9001,
        demo::items()
            .into_iter()
            .filter(|i| i.key.project == 9001)
            .collect(),
    );
    ui.dispatch(Msg::ProjectLoaded(
        9002,
        ui.state().list_epoch,
        Err("offline".into()),
    ))
    .unwrap();
    ui.state().flush_navigation().unwrap();
    let db = Connection::open(dbpath(&path, &ui.state().config, false)).unwrap();
    let read_selection = || {
        let body: String = db
            .query_row("SELECT body FROM snapshot", [], |r| r.get(0))
            .unwrap();
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["tabs"]["builtin:2"]["selection"]
            .clone()
    };
    assert_eq!(read_selection(), serde_json::json!({"Legacy": 1}));
    for project in [9002, 9003, 9004, 9005] {
        finish_project(
            &mut ui,
            project,
            demo::items()
                .into_iter()
                .filter(|i| i.key.project == project)
                .collect(),
        );
    }
    ui.state().flush_navigation().unwrap();
    assert_eq!(
        read_selection()["Item"],
        serde_json::to_value(selected(&ui)).unwrap()
    );
}

#[test]
fn portable_project_list_resolves_migrated_identity_before_authentication() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let mut config = config();
    config.active_tab = 1;
    config.selections.insert("1".into(), 1);
    let project = config.projects[1].id;
    fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let ui = mount(Config::load(&path).unwrap(), Some(&path), false);
    ui.state().flush_navigation().unwrap();
    drop(ui);
    let mut config = Config::load(&path).unwrap();
    config.projects.reverse();
    config.save(&path).unwrap();
    let ui = mount(config, Some(&path), false);
    assert_eq!(ui.state().user.id, 0);
    assert_eq!(
        ui.state().config.projects[ui.state().scroll.selected].id,
        project
    );
    assert_eq!(ui.state().scroll.selected, 3);
}

#[test]
fn detail_refresh_reordering_persists_open_identity_and_modern_restart_restores_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let config = config();
    config.save(&path).unwrap();
    let mut ui = mount(config, Some(&path), true);
    ui.dispatch(Msg::Tab(3)).unwrap();
    ui.dispatch(Msg::Select(3)).unwrap();
    let target = selected(&ui);
    ui.dispatch(Msg::Enter).unwrap();
    let mut details = demo::details(&target);
    details.item.updated_at = "2099-01-01T00:00:00Z".into();
    ui.dispatch(Msg::DetailsLoaded(
        target.clone(),
        ui.state().detail_epoch,
        Ok(Box::new(details)),
    ))
    .unwrap();
    ui.dispatch(Msg::Section(0)).unwrap();
    ui.dispatch(Msg::Move(1)).unwrap(); // Commit while the old slot now names a different row.
    assert_eq!(selected(&ui), target);
    assert_eq!(ui.state().scroll.selected, 0);
    ui.state().flush_navigation().unwrap();
    let db = Connection::open(dbpath(&path, &ui.state().config, true)).unwrap();
    let body: String = db
        .query_row("SELECT body FROM snapshot", [], |r| r.get(0))
        .unwrap();
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        body["tabs"]["builtin:3"]["selection"]["Item"],
        serde_json::to_value(&target).unwrap()
    );
    drop(ui);
    let mut restarted = mount(Config::load(&path).unwrap(), Some(&path), true);
    assert_eq!(restarted.state().config.route.as_ref(), Some(&target));
    assert_eq!(
        selected(&restarted),
        target,
        "restart resolves identity against the original fixture order"
    );
    restarted.dispatch(Msg::Back).unwrap();
    restarted.dispatch(Msg::Back).unwrap();
    assert_eq!(selected(&restarted), target);
}

#[test]
fn live_detail_update_reordering_returns_back_to_the_open_item() {
    let mut ui = mount(config(), None, false);
    ui.dispatch(Msg::UserLoaded(Ok(demo::user()))).unwrap();
    for project in [9001, 9002, 9003, 9004, 9005] {
        finish_project(
            &mut ui,
            project,
            demo::items()
                .into_iter()
                .filter(|i| i.key.project == project)
                .collect(),
        );
    }
    ui.dispatch(Msg::Tab(3)).unwrap();
    ui.dispatch(Msg::Select(3)).unwrap();
    let target = selected(&ui);
    ui.dispatch(Msg::Enter).unwrap();
    let mut details = demo::details(&target);
    details.item.updated_at = "2099-01-01T00:00:00Z".into();
    ui.dispatch(Msg::DetailsLoaded(
        target.clone(),
        ui.state().detail_epoch,
        Ok(Box::new(details)),
    ))
    .unwrap();
    ui.dispatch(Msg::Back).unwrap();
    assert_eq!(ui.state().scope, Scope::List);
    assert_eq!(selected(&ui), target);
    assert_eq!(ui.state().scroll.selected, 0);
}

#[test]
fn failed_hydration_is_not_evidence_that_a_restored_item_was_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let target = seed_live(&path);
    let mut ui = mount(Config::load(&path).unwrap(), Some(&path), false);
    start_hydration(&mut ui);
    ui.dispatch(Msg::ProjectLoaded(
        target.project,
        ui.state().list_epoch,
        Err("offline".into()),
    ))
    .unwrap();
    for project in [9002, 9003, 9004, 9005] {
        finish_project(
            &mut ui,
            project,
            demo::items()
                .into_iter()
                .filter(|i| i.key.project == project)
                .collect(),
        );
    }
    assert!(ui.state().list_pending.is_empty());
    assert_eq!(ui.state().config.route.as_ref(), Some(&target));
    ui.state().flush_navigation().unwrap();
    // A later successful project import can still resolve the original identity.
    finish_project(
        &mut ui,
        target.project,
        demo::items()
            .into_iter()
            .filter(|i| i.key.project == target.project)
            .collect(),
    );
    assert_eq!(selected(&ui), target);
}

#[test]
fn late_authentication_after_failed_project_sync_preserves_pending_identity_and_route() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let target = seed_live(&path);
    let mut ui = mount(Config::load(&path).unwrap(), Some(&path), false);
    ui.state_mut()
        .list_pending
        .extend([9001, 9002, 9003, 9004, 9005]);
    ui.dispatch(Msg::ProjectLoaded(
        target.project,
        ui.state().list_epoch,
        Err("offline".into()),
    ))
    .unwrap();
    for project in [9002, 9003, 9004, 9005] {
        finish_project(
            &mut ui,
            project,
            demo::items()
                .into_iter()
                .filter(|i| i.key.project == project)
                .collect(),
        );
    }
    ui.dispatch(Msg::UserLoaded(Ok(demo::user()))).unwrap();
    assert_eq!(ui.state().config.route.as_ref(), Some(&target));
    ui.state().flush_navigation().unwrap();
    drop(ui);
    let mut ui = mount(Config::load(&path).unwrap(), Some(&path), false);
    start_hydration(&mut ui);
    for project in [9001, 9002, 9003, 9004, 9005] {
        finish_project(
            &mut ui,
            project,
            demo::items()
                .into_iter()
                .filter(|i| i.key.project == project)
                .collect(),
        );
    }
    assert_eq!(selected(&ui), target);
}

#[test]
fn detail_field_section_and_document_movement_keep_pending_mr_identity_until_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let target = seed_live_kind(&path, ItemKind::MergeRequest);
    let mut ui = mount(Config::load(&path).unwrap(), Some(&path), false);
    start_hydration(&mut ui);
    ui.dispatch(Msg::DetailsLoaded(
        target.clone(),
        ui.state().detail_epoch,
        Ok(Box::new(demo::details(&target))),
    ))
    .unwrap();
    finish_project(
        &mut ui,
        9002,
        demo::items()
            .into_iter()
            .filter(|i| i.key.project == 9002)
            .collect(),
    );
    assert_eq!(ui.state().scope, Scope::Section);
    ui.dispatch(Msg::Move(1)).unwrap(); // Move a field, not a list item.
    ui.dispatch(Msg::Select(2)).unwrap(); // Mouse field selection also leaves the list identity alone.
    ui.dispatch(Msg::Back).unwrap();
    ui.dispatch(Msg::Move(1)).unwrap(); // Move a section header.
    ui.dispatch(Msg::Section(1)).unwrap();
    ui.dispatch(Msg::ScrollContent(2)).unwrap();
    ui.dispatch(Msg::ContentScroll(3)).unwrap();
    ui.dispatch(Msg::Back).unwrap(); // Return from Description to section headers.
    ui.dispatch(Msg::Action(Action::Help)).unwrap();
    ui.dispatch(Msg::Move(1)).unwrap(); // Dialog navigation is not list ownership.
    ui.dispatch(Msg::CloseDialog).unwrap();
    ui.dispatch(Msg::Back).unwrap(); // Back before the missing object's page arrives.
    assert_eq!(ui.state().scope, Scope::List);
    for project in [9003, 9004, 9005, 9001] {
        let mut items: Vec<_> = demo::items()
            .into_iter()
            .filter(|i| i.key.project == project)
            .collect();
        if let Some(item) = items.iter_mut().find(|i| i.key == target) {
            item.updated_at = "2099-01-01T00:00:00Z".into();
        }
        finish_project(&mut ui, project, items);
    }
    assert_eq!(
        selected(&ui),
        target,
        "Back must not strand the cursor on a partial-list placeholder"
    );
    assert!(ui.state().config.route.is_none());
}

#[test]
fn hiding_or_deleting_a_pending_project_does_not_resurrect_selection_if_it_is_added_back() {
    for remove in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.toml");
        let target = seed_live(&path);
        let mut ui = mount(Config::load(&path).unwrap(), Some(&path), false);
        start_hydration(&mut ui);
        let project = ui
            .state()
            .config
            .projects
            .iter()
            .find(|p| p.id == target.project)
            .unwrap()
            .clone();
        ui.dispatch(Msg::Tab(1)).unwrap();
        let index = ui
            .state()
            .config
            .projects
            .iter()
            .position(|p| p.id == target.project)
            .unwrap();
        ui.dispatch(Msg::Select(index)).unwrap();
        if remove {
            ui.dispatch(Msg::Action(Action::RemoveProject)).unwrap();
            assert!(matches!(
                ui.state().dialog.as_ref().unwrap().kind,
                DialogKind::Confirm(Confirmation::RemoveProject(_))
            ));
            ui.dispatch(Msg::Submit).unwrap();
        } else {
            ui.dispatch(Msg::ToggleProject).unwrap();
        }
        finish_project(
            &mut ui,
            9002,
            demo::items()
                .into_iter()
                .filter(|i| i.key.project == 9002)
                .collect(),
        );
        ui.dispatch(Msg::Tab(2)).unwrap();
        let chosen = selected(&ui);
        assert_ne!(chosen, target);
        assert!(ui.state().config.route.is_none());
        ui.dispatch(Msg::Tab(1)).unwrap();
        if remove {
            ui.dispatch(Msg::ProjectResolved(Ok(project))).unwrap();
        } else {
            ui.dispatch(Msg::ToggleProject).unwrap();
        }
        ui.dispatch(Msg::Tab(2)).unwrap();
        for project in [9003, 9004, 9005, 9001] {
            let mut items: Vec<_> = demo::items()
                .into_iter()
                .filter(|i| i.key.project == project)
                .collect();
            if let Some(item) = items.iter_mut().find(|i| i.key == target) {
                item.updated_at = "2099-01-01T00:00:00Z".into();
            }
            finish_project(&mut ui, project, items);
        }
        assert_eq!(
            selected(&ui),
            chosen,
            "invalid pending state must be discarded immediately on hide/remove"
        );
    }
}

#[test]
fn deliberate_filter_commit_cancels_pending_selection_before_later_pages_arrive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let target = seed_live(&path);
    let mut ui = mount(Config::load(&path).unwrap(), Some(&path), false);
    start_hydration(&mut ui);
    finish_project(
        &mut ui,
        9002,
        demo::items()
            .into_iter()
            .filter(|i| i.key.project == 9002)
            .collect(),
    );
    // Filtering is intentionally available only from the list, but Back must
    // preserve the pending selected identity until the deliberate filter commit.
    ui.dispatch(Msg::Back).unwrap();
    ui.dispatch(Msg::Back).unwrap();
    ui.dispatch(Msg::Action(Action::Filter)).unwrap();
    ui.state_mut().dialog.as_mut().unwrap().fields[0]
        .set_text_and_cursor("state:opened".into(), 12);
    ui.dispatch(Msg::Submit).unwrap();
    assert_eq!(ui.state().scope, Scope::List);
    assert!(
        ui.state().config.route.is_none(),
        "the committed filter invalidates the pending detail immediately"
    );
    let chosen = selected(&ui);
    for project in [9003, 9004, 9005, 9001] {
        let mut items: Vec<_> = demo::items()
            .into_iter()
            .filter(|i| i.key.project == project)
            .collect();
        if let Some(item) = items.iter_mut().find(|i| i.key == target) {
            item.updated_at = "2099-01-01T00:00:00Z".into();
        }
        finish_project(&mut ui, project, items);
    }
    assert_eq!(selected(&ui), chosen);
}

#[test]
fn user_navigation_during_partial_hydration_is_not_overridden_by_later_pages() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let target = seed_live(&path);
    let mut ui = mount(Config::load(&path).unwrap(), Some(&path), false);
    start_hydration(&mut ui);
    let items = demo::items();
    finish_project(
        &mut ui,
        9002,
        items
            .iter()
            .filter(|i| i.key.project == 9002)
            .cloned()
            .collect(),
    );
    ui.dispatch(Msg::Back).unwrap(); // Leave the restored section.
    ui.dispatch(Msg::Back).unwrap(); // Leave the detail.
    ui.dispatch(Msg::Move(1)).unwrap();
    let chosen = selected(&ui);
    assert_ne!(chosen, target);
    for project in [9003, 9004, 9005, 9001] {
        finish_project(
            &mut ui,
            project,
            items
                .iter()
                .filter(|i| i.key.project == project)
                .cloned()
                .collect(),
        );
        assert_eq!(
            selected(&ui),
            chosen,
            "a restored identity must not snap back after user input"
        );
    }
    ui.state().flush_navigation().unwrap();
    drop(ui);
    let mut restarted = mount(Config::load(&path).unwrap(), Some(&path), false);
    start_hydration(&mut restarted);
    for project in [9001, 9002, 9003, 9004, 9005] {
        finish_project(
            &mut restarted,
            project,
            items
                .iter()
                .filter(|i| i.key.project == project)
                .cloned()
                .collect(),
        );
    }
    assert_eq!(selected(&restarted), chosen);
}

#[test]
fn deletion_and_changed_filters_or_visibility_do_not_restore_stale_details() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let target = seed_live(&path);
    let mut ui = mount(Config::load(&path).unwrap(), Some(&path), false);
    start_hydration(&mut ui);
    for project in [9001, 9002, 9003, 9004, 9005] {
        finish_project(
            &mut ui,
            project,
            demo::items()
                .into_iter()
                .filter(|i| i.key.project == project && i.key != target)
                .collect(),
        );
    }
    assert!(ui.state().config.route.is_none());
    assert_eq!(ui.state().scope, Scope::List);
    assert_eq!(ui.state().scroll.selected, 0);
    drop(ui);
    for changed in ["filter", "visibility"] {
        let target = seed_live(&path);
        let mut config = Config::load(&path).unwrap();
        if changed == "filter" {
            config.filters.insert("2".into(), "state:closed".into());
        } else {
            config
                .projects
                .iter_mut()
                .find(|p| p.id == target.project)
                .unwrap()
                .visible = false;
        }
        config.save(&path).unwrap();
        let ui = mount(Config::load(&path).unwrap(), Some(&path), false);
        assert!(
            ui.state().config.route.is_none(),
            "{changed} changed the restored list context"
        );
        assert_eq!(ui.state().scope, Scope::List);
    }
}

#[test]
fn view_reordering_restores_identity_but_changed_definition_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved-views.toml");
    let mut config = config();
    config.views = ["First", "Last"]
        .into_iter()
        .map(|name| SavedView {
            name: name.into(),
            kind: ItemKind::Issue,
            query: "state:opened".into(),
        })
        .collect();
    config.save(&path).unwrap();
    let mut ui = mount(config, Some(&path), true);
    ui.dispatch(Msg::Tab(5)).unwrap();
    ui.dispatch(Msg::Select(2)).unwrap();
    let target = selected(&ui);
    ui.dispatch(Msg::Enter).unwrap();
    ui.state().flush_navigation().unwrap();
    drop(ui);
    let mut config = Config::load(&path).unwrap();
    config.views.swap(0, 1);
    config.save(&path).unwrap();
    let ui = mount(config, Some(&path), true);
    assert_eq!(ui.state().config.active_tab, 4);
    assert_eq!(selected(&ui), target);
    assert_eq!(ui.state().config.route.as_ref(), Some(&target));
    drop(ui);
    let mut config = Config::load(&path).unwrap();
    config.views[0].query = "state:closed".into();
    config.save(&path).unwrap();
    let ui = mount(config, Some(&path), true);
    assert_eq!(ui.state().config.active_tab, 0);
    assert!(ui.state().config.route.is_none());
}

#[test]
fn ui_rename_preserves_local_view_identity_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let mut config = config();
    config.views.push(SavedView {
        name: "Before".into(),
        kind: ItemKind::Issue,
        query: "state:opened".into(),
    });
    config.save(&path).unwrap();
    let mut ui = mount(config, Some(&path), true);
    ui.dispatch(Msg::Tab(4)).unwrap();
    ui.dispatch(Msg::Select(2)).unwrap();
    let target = selected(&ui);
    ui.dispatch(Msg::Enter).unwrap();
    ui.dispatch(Msg::Action(Action::RenameView)).unwrap();
    let field = &mut ui.state_mut().dialog.as_mut().unwrap().fields[0];
    field.set_text_and_cursor("After".into(), 5);
    ui.dispatch(Msg::Submit).unwrap();
    assert_eq!(
        Config::load(&path).unwrap().views[0].name,
        "After",
        "config edits save immediately"
    );
    drop(ui); // No explicit flush: owned Drop must durably save the latest rename snapshot.
    let ui = mount(Config::load(&path).unwrap(), Some(&path), true);
    assert_eq!(ui.state().tab_name(), "After");
    assert_eq!(selected(&ui), target);
    assert_eq!(ui.state().config.route.as_ref(), Some(&target));
}

#[test]
fn navigation_after_legacy_migration_hover_scroll_and_tick_do_not_rewrite_toml_and_quit_flushes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let config = config();
    config.save(&path).unwrap();
    // A real old workspace is migrated before measuring routine events.
    fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let mut ui = mount(config, Some(&path), true);
    let bytes = fs::read(&path).unwrap();
    assert!(Config::load(&path).unwrap().tab_states.is_empty());
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(&path).unwrap().ino()
    };
    for _ in 0..20 {
        ui.dispatch(Msg::Tab(2)).unwrap();
        ui.dispatch(Msg::Move(1)).unwrap();
        ui.dispatch(Msg::ListScroll(4)).unwrap();
        ui.dispatch(Msg::Hover("tab-2".into(), true)).unwrap();
        ui.dispatch(Msg::Tick).unwrap();
        ui.dispatch(Msg::Tab(3)).unwrap();
    }
    ui.dispatch(Msg::Select(2)).unwrap();
    let target = selected(&ui);
    ui.dispatch(Msg::Action(Action::Quit)).unwrap();
    assert!(ui.state().error.is_none());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            fs::metadata(&path).unwrap().ino(),
            inode,
            "even same-byte atomic TOML rewrites are forbidden"
        );
    }
    let snapshot: String = Connection::open(dbpath(&path, &ui.state().config, true))
        .unwrap()
        .query_row("SELECT body FROM snapshot", [], |r| r.get(0))
        .unwrap();
    assert!(snapshot.contains(&serde_json::to_string(&target).unwrap()));
    assert!(!snapshot.contains("trace"));
}

#[test]
fn failed_background_write_is_visible_live_navigation_keeps_working_and_retry_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let config = config();
    config.save(&path).unwrap();
    let mut ui = mount(config, Some(&path), true);
    let db = Connection::open(dbpath(&path, &ui.state().config, true)).unwrap();
    db.execute_batch("CREATE TRIGGER fail_insert BEFORE INSERT ON snapshot BEGIN SELECT RAISE(ABORT, 'injected write failure'); END; CREATE TRIGGER fail_update BEFORE UPDATE ON snapshot BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;").unwrap();
    ui.dispatch(Msg::Tab(2)).unwrap();
    assert!(ui.state().flush_navigation().is_err());
    ui.dispatch(Msg::Tick).unwrap();
    assert!(
        ui.state()
            .error
            .as_ref()
            .unwrap()
            .contains("injected write failure")
    );
    let before = ui.state().scroll.selected;
    ui.dispatch(Msg::Move(1)).unwrap();
    assert_ne!(ui.state().scroll.selected, before);
    let chosen = selected(&ui);
    ui.dispatch(Msg::Action(Action::Quit)).unwrap();
    assert!(ui.state().error.as_ref().unwrap().contains("Quit again"));
    db.execute_batch("DROP TRIGGER fail_insert; DROP TRIGGER fail_update;")
        .unwrap();
    ui.state().flush_navigation().unwrap();
    ui.dispatch(Msg::Tick).unwrap();
    assert!(ui.state().error.is_none());
    drop(ui);
    let ui = mount(Config::load(&path).unwrap(), Some(&path), true);
    assert_eq!(selected(&ui), chosen);
}

#[test]
fn permanently_failed_worker_allows_an_explicit_warned_exit_without_trapping_the_ui() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let config = config();
    config.save(&path).unwrap();
    let mut ui = mount(config, Some(&path), true);
    let db = Connection::open(dbpath(&path, &ui.state().config, true)).unwrap();
    db.execute_batch("CREATE TRIGGER fail_insert BEFORE INSERT ON snapshot BEGIN SELECT RAISE(ABORT, 'permanent disk failure'); END; CREATE TRIGGER fail_update BEFORE UPDATE ON snapshot BEGIN SELECT RAISE(ABORT, 'permanent disk failure'); END;").unwrap();
    ui.dispatch(Msg::Action(Action::Quit)).unwrap();
    assert!(ui.state().error.as_ref().unwrap().contains("Quit again"));
    ui.dispatch(Msg::Action(Action::Quit)).unwrap();
    assert!(
        ui.state()
            .error
            .as_ref()
            .unwrap()
            .contains("exiting with unsaved local navigation")
    );
    drop(ui); // Final failed attempt joins; no infinite retry during teardown.
}

#[test]
fn portable_config_save_failure_is_not_discarded_by_the_navigation_exit_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let config = config();
    config.save(&path).unwrap();
    let mut ui = mount(config, Some(&path), true);
    ui.state().flush_navigation().unwrap();
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    ui.state_mut().config.theme = "light".into();
    for _ in 0..2 {
        ui.dispatch(Msg::Action(Action::Quit)).unwrap();
        assert!(
            ui.state()
                .error
                .as_ref()
                .unwrap()
                .contains("Workspace not saved")
        );
        assert!(
            !ui.state()
                .error
                .as_ref()
                .unwrap()
                .contains("exiting with unsaved")
        );
    }
    assert!(path.is_dir());
}

#[test]
fn unavailable_sqlite_keeps_legacy_recoverable_while_memory_navigation_and_config_edits_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let mut config = config();
    config.active_tab = 2;
    config.selections.insert("2".into(), 1);
    config.tab_states.insert(
        "2".into(),
        TabState {
            list_offset: 1,
            ..TabState::default()
        },
    );
    fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    fs::write(path.with_extension("state"), "not a directory").unwrap();
    let mut ui = mount(Config::load(&path).unwrap(), Some(&path), true);
    assert!(
        ui.state()
            .error
            .as_ref()
            .unwrap()
            .contains("legacy TOML retained")
    );
    assert_eq!(ui.state().scroll.selected, 1);
    ui.dispatch(Msg::Move(1)).unwrap();
    assert_eq!(ui.state().scroll.selected, 2);
    ui.state_mut().config.theme = "light".into();
    ui.dispatch(Msg::Move(1)).unwrap();
    let saved = Config::load(&path).unwrap();
    assert_eq!(saved.theme, "light");
    assert_eq!(
        saved.selections["2"], 1,
        "failed migration must preserve original recoverable navigation"
    );
    assert!(ui.state().flush_navigation().is_err());
    ui.dispatch(Msg::Action(Action::Quit)).unwrap();
    assert!(ui.state().error.as_ref().unwrap().contains("Quit again"));
    ui.dispatch(Msg::Action(Action::Quit)).unwrap(); // Explicitly permits exit rather than trapping the UI.
}

#[test]
fn clearing_content_cache_preserves_live_navigation_and_no_path_clients_need_no_storage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    seed_live(&path);
    let mut ui = mount(Config::load(&path).unwrap(), Some(&path), false);
    start_hydration(&mut ui);
    for project in [9001, 9002, 9003, 9004, 9005] {
        finish_project(
            &mut ui,
            project,
            demo::items()
                .into_iter()
                .filter(|i| i.key.project == project)
                .collect(),
        );
    }
    let key = selected(&ui);
    let route = ui.state().config.route.clone();
    let section = ui.state().config.section;
    let field = ui.state().config.field;
    let mut api = GitLab::new("https://gitlab.invalid", "fake-token").unwrap();
    api.enable_persistence(&path).unwrap();
    ui.component_mut().api = Some(api);
    ui.dispatch(Msg::ClearCache).unwrap();
    assert!(ui.state().items.is_empty());
    assert_eq!(ui.state().config.route, route);
    assert_eq!(ui.state().config.section, section);
    assert_eq!(ui.state().config.field, field);
    ui.state().flush_navigation().unwrap();
    ui.component_mut().api = None; // No network commands are required by the test.
    drop(ui);
    let mut ui = mount(Config::load(&path).unwrap(), Some(&path), false);
    start_hydration(&mut ui);
    for project in [9001, 9002, 9003, 9004, 9005] {
        finish_project(
            &mut ui,
            project,
            demo::items()
                .into_iter()
                .filter(|i| i.key.project == project)
                .collect(),
        );
    }
    assert_eq!(selected(&ui), key);
    assert_eq!(ui.state().config.route, route);
    let mut headless = mount(config(), None, true);
    headless.dispatch(Msg::Tab(3)).unwrap();
    headless.dispatch(Msg::Move(1)).unwrap();
    headless.state().flush_navigation().unwrap();
    assert!(headless.state().error.is_none());
}
