//! Reproducible headless scrolling workload; timings are informational, not CI thresholds.
use std::time::Instant;

use cronk::{
    config::Config,
    demo,
    model::ItemKind,
    ui::{Cronk, Msg},
};
use tui_lipan::{
    TestBackend,
    core::event::{MouseButton, MouseKind},
    prelude::*,
};

fn large_list(count: usize) -> TestBackend<Cronk> {
    let mut ui = TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config: Config {
                projects: demo::projects(),
                active_tab: 3,
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
            w: 100,
            h: 68,
        },
    );
    ui.pump().unwrap();
    let template = demo::items()
        .into_iter()
        .find(|i| i.key.kind == ItemKind::MergeRequest)
        .unwrap();
    ui.state_mut().items = (0..count)
        .map(|n| {
            let mut item = template.clone();
            item.key.iid = n as u64 + 1;
            item.title = format!("Large list MR {}", n + 1);
            item
        })
        .collect();
    ui.dispatch(Msg::Select(0)).unwrap();
    ui.render();
    ui.pump().unwrap();
    ui
}

#[test]
fn large_list_scroll_workload() {
    for count in [300, 3_000] {
        let mut ui = large_list(count);
        let start = Instant::now();
        for _ in 0..30 {
            ui.dispatch(Msg::Move(1)).unwrap();
            ui.render();
            ui.pump().unwrap();
        }
        eprintln!(
            "{count} MRs: 30 selection/scroll frames: {:?}",
            start.elapsed()
        );
        assert_eq!(ui.state().scroll.selected, 30);
        let offset = ui.state().scroll.offset;
        let items = ui.state().visible_items();
        assert!(
            ui.capture_frame()
                .plain_text()
                .contains(&items[offset].title)
        );
    }
}

#[test]
fn large_list_scrollbar_drag_and_wheel_reach_real_rows() {
    let mut ui = large_list(3_000);
    let rect = ui.rect_of_key(&"main-list-3".into()).unwrap();
    let x = rect.x.max(0) as u16 + rect.w - 1;
    let top = rect.y.max(0) as u16;
    let bottom = top + rect.h - 1;
    for (y, kind) in [
        (top, MouseKind::Down(MouseButton::Left)),
        ((top + bottom) / 2, MouseKind::Drag(MouseButton::Left)),
        (bottom, MouseKind::Drag(MouseButton::Left)),
        (bottom, MouseKind::Up(MouseButton::Left)),
    ] {
        ui.send_mouse(MouseEvent {
            x,
            y,
            kind,
            mods: KeyMods::NONE,
        })
        .unwrap();
        ui.render();
        ui.pump().unwrap();
    }
    for _ in 0..3 {
        ui.render();
        ui.pump().unwrap();
    }
    assert_eq!(ui.state().scroll.offset, 2_980);
    let tail = ui.state().visible_items().last().unwrap().title.clone();
    assert!(ui.capture_frame().plain_text().contains(&tail));
    ui.send_mouse(MouseEvent {
        x: 10,
        y: top + 1,
        kind: MouseKind::ScrollUp,
        mods: KeyMods::NONE,
    })
    .unwrap();
    for _ in 0..3 {
        ui.render();
        ui.pump().unwrap();
    }
    assert!(ui.state().scroll.offset < 2_980);
    let offset = ui.state().scroll.offset;
    let title = ui.state().visible_items()[offset].title.clone();
    assert!(ui.capture_frame().plain_text().contains(&title));
}

#[test]
fn large_list_jumps_preserve_global_indices() {
    let mut ui = large_list(3_000);
    for index in [1_500, 2_999, 0, 30] {
        ui.dispatch(Msg::Select(index)).unwrap();
        for _ in 0..3 {
            ui.render();
            ui.pump().unwrap();
        }
        assert_eq!(ui.state().scroll.selected, index);
        let items = ui.state().visible_items();
        assert!(
            ui.capture_frame()
                .plain_text()
                .contains(&items[index].title)
        );
    }
    ui.dispatch(Msg::Select(1_500)).unwrap();
    ui.render();
    ui.pump().unwrap();
    let key = ui.state().visible_items()[1_500].key.clone();
    let rect = ui
        .rect_of_key(&format!("item-{}-{}-{}", key.project, key.kind.segment(), key.iid).into())
        .unwrap();
    for kind in [
        MouseKind::Down(MouseButton::Left),
        MouseKind::Up(MouseButton::Left),
    ] {
        ui.send_mouse(MouseEvent {
            x: rect.x.max(0) as u16 + 5,
            y: rect.y.max(0) as u16,
            kind,
            mods: KeyMods::NONE,
        })
        .unwrap();
    }
    ui.pump().unwrap();
    assert_eq!(ui.state().config.route.as_ref(), Some(&key));
}
