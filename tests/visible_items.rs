//! `visible_item_at` and `visible_position` must agree with `visible_items()`
//! for every index and key, including ties and duplicate keys.
use cronk::{
    config::Config,
    demo,
    model::{ItemKey, ItemKind, WorkItem},
    ui::Cronk,
};
use tui_lipan::{TestBackend, prelude::*};

fn ui_with_tied_items() -> TestBackend<Cronk> {
    let mut ui = TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config: Config {
                projects: demo::projects(),
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
            w: 80,
            h: 24,
        },
    );
    ui.pump().unwrap();
    let template = demo::items().remove(0);
    let projects: Vec<u64> = demo::projects().iter().map(|p| p.id).collect();
    let mut items = Vec::new();
    for n in 0..90_u64 {
        let mut item: WorkItem = template.clone();
        item.key = ItemKey {
            project: projects[n as usize % projects.len()],
            // Few distinct iids and timestamps force ties in every sort key.
            iid: n % 4 + 1,
            kind: if n % 2 == 0 {
                ItemKind::MergeRequest
            } else {
                ItemKind::Issue
            },
        };
        item.title = format!("Tied item {}", n % 7);
        item.updated_at = format!("2026-01-0{}T00:00:00Z", n % 3 + 1);
        item.state = if n % 5 == 0 { "closed" } else { "opened" }.into();
        item.author.id = if n % 3 == 0 { demo::user().id } else { 7 };
        item.assignees.clear();
        items.push(item);
    }
    // Exact duplicate keys: the first one in sorted order must win.
    items.push(items[10].clone());
    items.push(items[11].clone());
    ui.state_mut().items = items;
    ui.state_mut().user = demo::user();
    ui
}

#[test]
fn indexed_lookups_match_the_sorted_view_on_every_tab_and_filter() {
    let mut ui = ui_with_tied_items();
    for (tab, query) in [
        (0, ""),
        (0, "tied"),
        (2, ""),
        (3, ""),
        (3, "item 3"),
        (3, "no such text"),
    ] {
        ui.state_mut().config.active_tab = tab;
        let key = ui.state().tab_key();
        if query.is_empty() {
            ui.state_mut().config.filters.remove(&key);
        } else {
            ui.state_mut().config.filters.insert(key, query.into());
        }
        let state = ui.state();
        let sorted = state.visible_items();
        assert_eq!(
            state.visible_item_count(),
            sorted.len(),
            "tab {tab} {query}"
        );
        assert_eq!(
            sorted.is_empty(),
            query == "no such text",
            "tab {tab} {query}"
        );
        for (index, item) in sorted.iter().enumerate() {
            assert!(
                std::ptr::eq(state.visible_item_at(index).unwrap(), *item),
                "tab {tab} {query}: item at {index}"
            );
            let first = sorted.iter().position(|other| other.key == item.key);
            assert_eq!(
                state.visible_position(&item.key),
                first,
                "tab {tab} {query}: position of {:?}",
                item.key
            );
        }
        assert!(state.visible_item_at(sorted.len()).is_none());
        let missing = ItemKey {
            project: u64::MAX,
            iid: 1,
            kind: ItemKind::Issue,
        };
        assert_eq!(state.visible_position(&missing), None);
    }
}
