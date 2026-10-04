use std::time::Duration;

use cronk::{
    config::{Config, SavedView},
    demo,
    model::{Details, Diff, Discussion, ItemKind, Job, Note, Pipeline},
    scroll::BoundaryScroll,
    ui::{Cronk, DialogKind, Msg, Scope, Trace, list_height},
};
use tui_lipan::{
    TestBackend,
    core::event::{MouseButton, MouseKind},
    prelude::*,
};

type Ui = TestBackend<Cronk>;

const THEMES: [&str; 3] = ["midnight", "dracula", "light"];

fn config(tab: usize, theme: &str) -> Config {
    Config {
        projects: demo::projects(),
        views: vec![SavedView {
            name: "Saved issues".into(),
            kind: ItemKind::Issue,
            query: String::new(),
        }],
        active_tab: tab,
        theme: theme.into(),
        ..Config::default()
    }
}

fn mount(config: Config, width: u16, height: u16) -> Ui {
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
            w: width,
            h: height,
        },
    );
    settle(&mut ui);
    ui
}

// Reconciliation can reapply a controlled offset or reveal_key after the native mouse
// handler moved the widget. Do not accept an intermediate, widget-only scroll as success.
fn settle(ui: &mut Ui) {
    ui.advance(Duration::from_millis(250));
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

fn mouse(ui: &mut Ui, x: u16, y: u16, kind: MouseKind) {
    ui.send_mouse(MouseEvent {
        x,
        y,
        kind,
        mods: KeyMods::NONE,
    })
    .unwrap();
}

fn click(ui: &mut Ui, x: u16, y: u16) {
    mouse(ui, x, y, MouseKind::Down(MouseButton::Left));
    mouse(ui, x, y, MouseKind::Up(MouseButton::Left));
    settle(ui);
}

fn rect(ui: &Ui, key: &str) -> Rect {
    ui.rect_of_key(&key.to_owned().into()).unwrap_or_else(|| {
        panic!(
            "missing viewport {key:?}:\n{}",
            ui.capture_frame().plain_text()
        )
    })
}

fn right(rect: Rect) -> u16 {
    u16::try_from(rect.x).unwrap() + rect.w - 1
}

fn top(rect: Rect) -> u16 {
    u16::try_from(rect.y).unwrap()
}

fn bottom(rect: Rect) -> u16 {
    top(rect) + rect.h - 1
}

fn content_rect(ui: &Ui) -> Rect {
    Rect {
        x: 0,
        y: 6,
        w: ui.viewport().w,
        h: ui.viewport().h - 8,
    }
}

fn main_rect(ui: &Ui) -> Rect {
    let rect = rect(ui, &format!("main-list-{}", ui.state().config.active_tab));
    assert_eq!(rect.y, 6, "six header rows must remain outside the list");
    assert_eq!(rect.h as usize, list_height(ui.viewport().h) * 3);
    assert_eq!(
        right(rect),
        ui.viewport().w - 1,
        "scrollbar belongs at the terminal's right edge"
    );
    assert!(
        bottom(rect) < ui.viewport().h - 2,
        "do not cover the footer"
    );
    rect
}

// Read cells, not string byte offsets: half-blocks and wide text must not move mouse targets.
fn content(ui: &Ui, rect: Rect) -> String {
    let frame = ui.capture_frame();
    (top(rect)..=bottom(rect))
        .map(|y| {
            (u16::try_from(rect.x).unwrap()..right(rect))
                .map(|x| frame.cell(x, y).symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Thumb {
    // tui-lipan's explicit '█' thumb has half-cell precision.
    start: usize,
    len: usize,
}

fn thumb(ui: &Ui, rect: Rect) -> Option<Thumb> {
    let frame = ui.capture_frame();
    let mut occupied = Vec::new();
    for (row, y) in (top(rect)..=bottom(rect)).enumerate() {
        let symbol = frame.cell(right(rect), y).symbol.as_str();
        if matches!(symbol, "█" | "▀") {
            occupied.push(2 * row);
        }
        if matches!(symbol, "█" | "▄") {
            occupied.push(2 * row + 1);
        }
    }
    let start = *occupied.first()?;
    assert_eq!(
        occupied,
        (start..start + occupied.len()).collect::<Vec<_>>(),
        "thumb must be contiguous"
    );
    Some(Thumb {
        start,
        len: occupied.len(),
    })
}

fn visible_thumb(ui: &Ui, rect: Rect) -> Thumb {
    let thumb = thumb(ui, rect).unwrap_or_else(|| {
        panic!(
            "missing vertical thumb at x={}, y={}..{}:\n{}",
            right(rect),
            top(rect),
            bottom(rect),
            ui.capture_frame().plain_text()
        )
    });
    assert!(
        thumb.len < 2 * rect.h as usize,
        "overflow must leave clickable track"
    );
    let frame = ui.capture_frame();
    let cell = frame.cell(right(rect), top(rect) + (thumb.start / 2) as u16);
    assert_ne!(cell.fg, cell.bg, "thumb must contrast with its background");
    thumb
}

// Keep edge-placement assertions separate from interaction assertions, so an inset
// scrollbar does not hide a second bug in native hit testing or controlled offsets.
fn rendered_bar(ui: &Ui, area: Rect) -> Rect {
    for x in (u16::try_from(area.x).unwrap()..=right(area)).rev() {
        let candidate = Rect {
            w: x - u16::try_from(area.x).unwrap() + 1,
            ..area
        };
        if thumb(ui, candidate).is_some() {
            return candidate;
        }
    }
    panic!(
        "no rendered scrollbar in {area:?}:\n{}",
        ui.capture_frame().plain_text()
    );
}

fn proportional(ui: &Ui, rect: Rect, total_rows: usize) -> Thumb {
    let thumb = visible_thumb(ui, rect);
    let track = 2 * rect.h as usize;
    let expected = ((track * rect.h as usize + total_rows / 2) / total_rows)
        .max(1)
        .min(track - 1);
    assert_eq!(
        thumb.len, expected,
        "thumb must represent the whole content, not just rendered rows"
    );
    thumb
}

fn drag_to(ui: &mut Ui, rect: Rect, to_bottom: bool) {
    let thumb = visible_thumb(ui, rect);
    let from = top(rect) + ((thumb.start + thumb.len / 2) / 2) as u16;
    let to = if to_bottom { bottom(rect) } else { top(rect) };
    mouse(ui, right(rect), from, MouseKind::Down(MouseButton::Left));
    // Include an intermediate drag so capture must survive an actual reconciliation.
    mouse(
        ui,
        right(rect),
        (from + to) / 2,
        MouseKind::Drag(MouseButton::Left),
    );
    mouse(ui, right(rect), to, MouseKind::Drag(MouseButton::Left));
    mouse(ui, right(rect), to, MouseKind::Up(MouseButton::Left));
    settle(ui);
}

fn retain_content(ui: &mut Ui, rect: Rect) {
    let before = content(ui, rect);
    let bar = visible_thumb(ui, rect);
    settle(ui);
    assert_eq!(
        content(ui, rect),
        before,
        "settled rerender snapped the viewport back"
    );
    for msg in [Msg::Tick, Msg::Refresh] {
        ui.dispatch(msg).unwrap();
        settle(ui);
        assert_eq!(
            content(ui, rect),
            before,
            "Tick/Refresh must retain the visible content"
        );
        assert_eq!(visible_thumb(ui, rect), bar, "Tick/Refresh moved the thumb");
    }
    if let Some(details) = ui.state().details.clone() {
        let route = details.item.key.clone();
        let epoch = ui.state().detail_epoch;
        ui.dispatch(Msg::DetailsLoaded(route, epoch, Ok(Box::new(details))))
            .unwrap();
        settle(ui);
        assert_eq!(
            content(ui, rect),
            before,
            "an unchanged detail refresh must retain the viewport"
        );
        assert_eq!(visible_thumb(ui, rect), bar);
    }
}

fn list_len(ui: &Ui) -> usize {
    if ui.state().config.active_tab == 1 {
        ui.state().config.projects.len()
    } else {
        ui.state().visible_items().len()
    }
}

fn list_height_for(tab: usize) -> u16 {
    if tab == 1 { 14 } else { 32 }
}

fn assert_list_content(ui: &Ui, area: Rect) {
    let offset = ui.state().scroll.offset;
    let slots = list_height(ui.viewport().h);
    let frame = ui.capture_frame();
    for row in 0..slots.min(list_len(ui)) {
        let row_key = if ui.state().config.active_tab == 1 {
            format!("project-{}", ui.state().config.projects[offset + row].id)
        } else {
            let items = ui.state().visible_items();
            let key = &items[offset + row].key;
            format!("item-{}-{}-{}", key.project, key.kind.segment(), key.iid)
        };
        assert_eq!(
            rect(ui, &row_key).y,
            area.y + (3 * row) as i16,
            "rendered row must agree with BoundaryScroll: {row_key}\n{}",
            frame.plain_text()
        );
    }
    let mut normalized = ui.state().scroll.clone();
    normalized.normalize(list_len(ui), slots);
    assert_eq!(
        ui.state().scroll,
        normalized,
        "native scrolling must leave BoundaryScroll normalized"
    );
}

fn list_visibility(tab: usize) {
    for theme in THEMES {
        let mut ui = mount(config(tab, theme), 120, list_height_for(tab));
        let area = main_rect(&ui);
        let len = list_len(&ui);
        assert!(
            len > list_height(ui.viewport().h),
            "fixture must overflow: tab {tab}"
        );
        assert_eq!(
            proportional(&ui, area, len * 3).start,
            0,
            "{theme}, tab {tab}"
        );

        ui.set_viewport(Rect {
            x: 0,
            y: 0,
            w: 120,
            h: (len * 3 + 8) as u16,
        });
        settle(&mut ui);
        assert!(
            thumb(&ui, main_rect(&ui)).is_none(),
            "fitting content must not show a bar: {theme}, tab {tab}"
        );

        ui.state_mut().items.clear();
        if tab == 1 {
            ui.state_mut().config.projects.clear();
        }
        ui.dispatch(Msg::Refresh).unwrap();
        settle(&mut ui);
        assert_eq!(list_len(&ui), 0);
        assert!(
            thumb(&ui, content_rect(&ui)).is_none(),
            "empty content must not leave a stale bar"
        );
    }
}

fn list_track(tab: usize) {
    for theme in THEMES {
        let mut ui = mount(config(tab, theme), 120, list_height_for(tab));
        let area = main_rect(&ui);
        let before = content(&ui, area);
        let bar = visible_thumb(&ui, area);
        assert!(bar.start + bar.len <= 2 * (area.h as usize - 1));
        let projects = ui.state().config.projects.clone();
        click(&mut ui, right(area), bottom(area));
        assert_eq!(
            ui.state().scope,
            Scope::List,
            "track must not open the row underneath"
        );
        assert_eq!(ui.state().config.active_tab, tab);
        assert!(ui.state().config.route.is_none());
        assert!(ui.state().dialog.is_none());
        assert_eq!(
            ui.state().config.projects,
            projects,
            "track must not toggle a project"
        );
        assert!(
            ui.state().scroll.offset > 0,
            "track click must update the controlled viewport"
        );
        assert_ne!(
            content(&ui, area),
            before,
            "track click must move actual rows"
        );
        assert_list_content(&ui, area);
        retain_content(&mut ui, area);
    }
}

fn list_drag_and_navigation(tab: usize) {
    for theme in THEMES {
        let mut ui = mount(config(tab, theme), 120, list_height_for(tab));
        let area = main_rect(&ui);
        let len = list_len(&ui);
        let slots = list_height(ui.viewport().h);
        drag_to(&mut ui, area, true);
        assert_eq!(ui.state().scope, Scope::List);
        assert_eq!(
            ui.state().scroll.offset,
            len - slots,
            "drag must reach the final complete list page"
        );
        let bar = proportional(&ui, area, len * 3);
        assert_eq!(bar.start + bar.len, 2 * area.h as usize);
        assert_list_content(&ui, area);
        retain_content(&mut ui, area);

        let before = ui.state().scroll.offset;
        mouse(&mut ui, 5, top(area) + 1, MouseKind::ScrollUp);
        settle(&mut ui);
        assert!(
            ui.state().scroll.offset < before,
            "wheel must continue from the dragged viewport"
        );
        assert_list_content(&ui, area);
        for (code, delta) in [
            (KeyCode::Up, -1),
            (KeyCode::Down, 1),
            (KeyCode::PageUp, -(slots as isize)),
            (KeyCode::PageDown, slots as isize),
        ] {
            let mut expected = ui.state().scroll.clone();
            expected.move_by(delta, len, slots);
            key(&mut ui, code);
            assert_eq!(
                ui.state().scroll,
                expected,
                "keyboard must use BoundaryScroll after drag"
            );
            assert_list_content(&ui, area);
        }

        drag_to(&mut ui, area, false);
        assert_eq!(ui.state().scroll.offset, 0);
        assert_eq!(proportional(&ui, area, len * 3).start, 0);
        assert_list_content(&ui, area);
        retain_content(&mut ui, area);
        let before = ui.state().scroll.offset;
        mouse(&mut ui, 5, top(area) + 1, MouseKind::ScrollDown);
        settle(&mut ui);
        assert!(ui.state().scroll.offset > before);
        assert_list_content(&ui, area);
        key(&mut ui, KeyCode::Home);
        assert_eq!(ui.state().scroll, BoundaryScroll::default());
        key(&mut ui, KeyCode::End);
        assert_eq!(
            (ui.state().scroll.selected, ui.state().scroll.offset),
            (len - 1, len - slots)
        );
    }
}

macro_rules! list_tests {
    ($visibility:ident, $track:ident, $drag:ident, $tab:expr) => {
        #[test]
        fn $visibility() {
            list_visibility($tab);
        }
        #[test]
        fn $track() {
            list_track($tab);
        }
        #[test]
        fn $drag() {
            list_drag_and_navigation($tab);
        }
    };
}

list_tests!(
    dashboard_bar_visibility_and_proportion,
    dashboard_track_does_not_activate,
    dashboard_drag_wheel_and_keyboard,
    0
);
list_tests!(
    projects_bar_visibility_and_proportion,
    projects_track_does_not_activate,
    projects_drag_wheel_and_keyboard,
    1
);
list_tests!(
    issues_bar_visibility_and_proportion,
    issues_track_does_not_activate,
    issues_drag_wheel_and_keyboard,
    2
);
list_tests!(
    merge_requests_bar_visibility_and_proportion,
    merge_requests_track_does_not_activate,
    merge_requests_drag_wheel_and_keyboard,
    3
);
list_tests!(
    saved_view_bar_visibility_and_proportion,
    saved_view_track_does_not_activate,
    saved_view_drag_wheel_and_keyboard,
    4
);

#[test]
fn list_thumb_updates_when_content_shrinks_filter_changes_and_terminal_resizes() {
    let mut ui = mount(config(2, "midnight"), 120, 26);
    let area = main_rect(&ui);
    let initial = proportional(&ui, area, list_len(&ui) * 3);
    drag_to(&mut ui, area, true);
    let keep = ui
        .state()
        .visible_items()
        .into_iter()
        .take(10)
        .cloned()
        .collect();
    ui.state_mut().items = keep;
    ui.dispatch(Msg::Refresh).unwrap();
    settle(&mut ui);
    let shorter = proportional(&ui, area, 30);
    assert!(shorter.len > initial.len);
    assert_list_content(&ui, area);

    ui.set_viewport(Rect {
        x: 0,
        y: 0,
        w: 83,
        h: 20,
    });
    settle(&mut ui);
    let smaller = main_rect(&ui);
    assert_eq!(right(smaller), 82);
    assert!(proportional(&ui, smaller, 30).len < shorter.len);
    assert_list_content(&ui, smaller);

    // Exercise the real filter dialog, not a made-up scrollbar/controller message.
    let title = ui.state().visible_items()[0].title.clone();
    key(&mut ui, KeyCode::Char('/'));
    ui.send_paste(&title).unwrap();
    key(&mut ui, KeyCode::Enter);
    assert_eq!(list_len(&ui), 1);
    assert!(thumb(&ui, main_rect(&ui)).is_none());
    assert_eq!(ui.state().scroll.offset, 0);
    key(&mut ui, KeyCode::Char('/'));
    key(&mut ui, KeyCode::Home);
    ui.send_key(KeyEvent {
        code: KeyCode::End,
        mods: KeyMods::SHIFT,
    })
    .unwrap();
    ui.send_paste("no-such-scrollbar-fixture").unwrap();
    key(&mut ui, KeyCode::Enter);
    assert_eq!(list_len(&ui), 0);
    assert!(thumb(&ui, content_rect(&ui)).is_none());
    assert_eq!(ui.state().scroll, BoundaryScroll::default());
}

#[derive(Clone, Copy, Debug)]
enum Screen {
    Overview,
    Fields,
    Description,
    Activity,
    Pipeline,
    Jobs,
    Discussions,
    Changes,
}

impl Screen {
    fn section(self) -> Option<usize> {
        match self {
            Self::Overview => None,
            Self::Fields => Some(0),
            Self::Description => Some(1),
            Self::Activity | Self::Pipeline => Some(2),
            Self::Jobs => Some(3),
            Self::Discussions => Some(4),
            Self::Changes => Some(5),
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Fields => "fields",
            Self::Description => "description",
            Self::Activity => "activity",
            Self::Pipeline => "pipeline",
            Self::Jobs => "jobs",
            Self::Discussions => "discussion-notes",
            Self::Changes => "changes",
        }
    }

    fn markers(self) -> (&'static str, &'static str) {
        match self {
            Self::Overview => ("SCROLL_TITLE", "DESCRIPTION_089"),
            Self::Fields => ("Title", "SCROLL_END"),
            Self::Description => ("DESCRIPTION_000", "DESCRIPTION_089"),
            Self::Activity => ("ACTIVITY_000", "ACTIVITY_019"),
            Self::Pipeline => ("Pipeline #", "JOB_023"),
            Self::Jobs => ("JOB_000", "JOB_023"),
            Self::Discussions => ("DISCUSSION_000_000", "DISCUSSION_000_059"),
            Self::Changes => ("+CHANGE_000", "+CHANGE_089"),
        }
    }
}

fn document(prefix: &str, count: usize) -> String {
    format!(
        "```text\n{}\n```",
        (0..count)
            .map(|n| format!("{prefix}_{n:03}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn detail_fixture(details: &mut Details, running: bool) {
    details.item.title = "SCROLL_TITLE".into();
    details.item.description = document("DESCRIPTION", 90);
    details.item.web_url = "https://example.invalid/SCROLL_END".into();
    details.item.pipeline = Some(Pipeline {
        id: 700,
        status: "running".into(),
        web_url: "https://example.invalid/pipeline".into(),
    });
    details.warnings.clear();
    details.notes = (0..20)
        .map(|n| Note {
            id: 20_000 + n,
            author: demo::user(),
            body: document(&format!("ACTIVITY_{n:03}"), 3),
            created_at: "2026-01-01T00:00:00Z".into(),
            ..Note::default()
        })
        .collect();
    details.jobs = (0..24)
        .map(|n| Job {
            id: 30_000 + n,
            name: format!("JOB_{n:03}"),
            stage: "test".into(),
            status: if running { "running" } else { "success" }.into(),
            ..Job::default()
        })
        .collect();
    details.discussions = (0..20)
        .map(|n| Discussion {
            id: format!("scroll-thread-{n}"),
            notes: vec![Note {
                id: 40_000 + n,
                author: demo::user(),
                body: format!(
                    "THREAD_{n:03}\n\n{}",
                    document(&format!("DISCUSSION_{n:03}"), 60)
                ),
                ..Note::default()
            }],
        })
        .collect();
    details.diffs = vec![Diff {
        old_path: "scroll-fixture.txt".into(),
        new_path: "scroll-fixture.txt".into(),
        diff: format!(
            "@@ -0,0 +1,90 @@\n{}",
            (0..90)
                .map(|n| format!("+CHANGE_{n:03}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        ..Diff::default()
    }];
}

fn detail_mount(screen: Screen, theme: &str, width: u16, height: u16) -> (Ui, String) {
    let kind = if matches!(screen, Screen::Activity) {
        ItemKind::Issue
    } else {
        ItemKind::MergeRequest
    };
    let route = demo::items()
        .into_iter()
        .find(|item| item.key.kind == kind)
        .unwrap()
        .key;
    let key = if matches!(screen, Screen::Discussions) {
        "discussion-notes-scroll-thread-0".into()
    } else {
        format!(
            "{}-{}-{}-{}",
            screen.prefix(),
            route.project,
            kind.segment(),
            route.iid
        )
    };
    let mut config = config(if kind == ItemKind::Issue { 2 } else { 3 }, theme);
    config.route = Some(route);
    config.section = screen.section();
    let mut ui = mount(config, width, height);
    detail_fixture(
        ui.state_mut().details.as_mut().unwrap(),
        matches!(screen, Screen::Pipeline),
    );
    ui.state_mut().traces.clear();
    ui.state_mut().expanded.clear();
    // Unknown demo job IDs deliberately produce a stable, single-line offline trace.
    let jobs = ui.state().details.as_ref().unwrap().jobs.clone();
    for job in jobs {
        ui.state_mut().traces.insert(
            job.id,
            Trace {
                text: demo::trace(job.id, 0),
                finished: !job.running(),
                ..Trace::default()
            },
        );
    }
    settle(&mut ui);
    (ui, key)
}

fn detail_scrolls(screen: Screen) {
    for theme in THEMES {
        let height = if matches!(screen, Screen::Fields) {
            14
        } else {
            24
        };
        let (mut ui, key) = detail_mount(screen, theme, 120, height);
        let area = rendered_bar(&ui, rect(&ui, &key));
        let (first, last) = screen.markers();
        let initial = content(&ui, area);
        assert!(
            initial.contains(first),
            "{screen:?}: first content missing:\n{initial}"
        );
        assert!(!initial.contains(last), "{screen:?}: fixture must overflow");
        let initial_thumb = visible_thumb(&ui, area);
        assert_eq!(initial_thumb.start, 0);
        let route = ui.state().config.route.clone();
        let scope = ui.state().scope;
        let expanded = ui.state().expanded.clone();
        let section = ui.state().config.section;

        click(&mut ui, right(area), bottom(area));
        assert_eq!(ui.state().scope, scope, "track must not enter a section");
        assert_eq!(ui.state().config.route, route);
        assert_eq!(ui.state().config.section, section);
        assert!(ui.state().dialog.is_none(), "track must not edit a field");
        assert_eq!(
            ui.state().expanded,
            expanded,
            "track must not activate a job"
        );
        assert_ne!(
            content(&ui, area),
            initial,
            "{screen:?}: track moved no content after settling"
        );
        assert!(
            ui.state().content_offset > 0,
            "{screen:?}: native scroll must reach controller state"
        );
        retain_content(&mut ui, area);

        drag_to(&mut ui, area, true);
        let end = content(&ui, area);
        assert!(
            end.contains(last),
            "{screen:?}: drag must expose the last real content:\n{end}"
        );
        assert!(
            !end.contains(first),
            "{screen:?}: viewport stayed at the beginning"
        );
        let bar = visible_thumb(&ui, area);
        assert_eq!(
            bar.start + bar.len,
            2 * area.h as usize,
            "{screen:?}: bottom thumb position"
        );
        retain_content(&mut ui, area);

        drag_to(&mut ui, area, false);
        assert_eq!(visible_thumb(&ui, area).start, 0);
        assert_eq!(ui.state().content_offset, 0);
        assert_eq!(
            content(&ui, area),
            initial,
            "{screen:?}: drag back must restore the first page"
        );
        retain_content(&mut ui, area);
    }
}

macro_rules! detail_test {
    ($name:ident, $screen:ident) => {
        #[test]
        fn $name() {
            detail_scrolls(Screen::$screen);
        }
    };
}

detail_test!(overview_scrolls_real_content_and_retains_viewport, Overview);
detail_test!(fields_scroll_does_not_snap_back_to_selected_field, Fields);
detail_test!(
    description_document_scrolls_real_content_and_retains_viewport,
    Description
);
detail_test!(activity_scrolls_real_content_and_retains_viewport, Activity);
detail_test!(pipeline_scrolls_real_content_and_retains_viewport, Pipeline);
detail_test!(jobs_scroll_does_not_snap_back_to_selected_job, Jobs);
detail_test!(
    discussion_notes_scroll_independently_and_retain_viewport,
    Discussions
);
detail_test!(changes_scroll_real_content_and_retain_viewport, Changes);

#[test]
fn description_thumb_updates_on_shrink_growth_and_resize() {
    let (mut ui, key) = detail_mount(Screen::Description, "midnight", 120, 24);
    let area = rendered_bar(&ui, rect(&ui, &key));
    let long = visible_thumb(&ui, area);
    drag_to(&mut ui, area, true);
    ui.state_mut().details.as_mut().unwrap().item.description = document("SHORTER", 40);
    ui.dispatch(Msg::Refresh).unwrap();
    settle(&mut ui);
    let shorter = visible_thumb(&ui, area);
    assert!(shorter.len > long.len);
    assert!(
        content(&ui, area).contains("SHORTER_039"),
        "shrinking must clamp to the new last page"
    );
    retain_content(&mut ui, area);

    ui.state_mut().details.as_mut().unwrap().item.description = document("FITS", 1);
    ui.dispatch(Msg::Refresh).unwrap();
    settle(&mut ui);
    assert!(thumb(&ui, area).is_none());
    assert!(content(&ui, area).contains("FITS_000"));

    ui.state_mut().details.as_mut().unwrap().item.description = document("GROWN", 90);
    ui.dispatch(Msg::Refresh).unwrap();
    settle(&mut ui);
    assert_eq!(
        visible_thumb(&ui, area).start,
        0,
        "a stale offset must not return when content grows"
    );
    assert!(content(&ui, area).contains("GROWN_000"));
    ui.set_viewport(Rect {
        x: 0,
        y: 0,
        w: 83,
        h: 38,
    });
    settle(&mut ui);
    let resized_viewport = rect(&ui, &key);
    assert_eq!(right(resized_viewport), 82);
    let resized = rendered_bar(&ui, resized_viewport);
    assert!(visible_thumb(&ui, resized).len > long.len);
    drag_to(&mut ui, resized, true);
    assert!(content(&ui, resized).contains("GROWN_089"));
    retain_content(&mut ui, resized);
}

#[test]
fn discussions_split_panes_have_independent_right_edge_scrollbars() {
    let (mut ui, key) = detail_mount(Screen::Discussions, "midnight", 120, 24);
    let notes = rect(&ui, &key);
    let summaries = Rect {
        x: 0,
        y: notes.y,
        w: u16::try_from(notes.x).unwrap(),
        h: notes.h,
    };
    assert!(summaries.w > 0 && right(summaries) < right(notes));
    let selected = ui.state().config.field;
    let notes_before = content(&ui, notes);
    visible_thumb(&ui, summaries);
    visible_thumb(&ui, notes);
    drag_to(&mut ui, summaries, true);
    assert!(
        content(&ui, summaries).contains("THREAD_019"),
        "left scrollbar must reach the last discussion summary"
    );
    assert_eq!(
        ui.state().config.field,
        selected,
        "scrolling must not activate another discussion"
    );
    assert_eq!(
        content(&ui, notes),
        notes_before,
        "left scrollbar must not scroll the right pane"
    );
    retain_content(&mut ui, summaries);
    let summaries_before = content(&ui, summaries);
    drag_to(&mut ui, notes, true);
    assert!(content(&ui, notes).contains("DISCUSSION_000_059"));
    assert_eq!(
        content(&ui, summaries),
        summaries_before,
        "right scrollbar must not scroll the left pane"
    );
    retain_content(&mut ui, notes);
    drag_to(&mut ui, summaries, false);
    assert!(content(&ui, summaries).contains("THREAD_000"));
}

#[test]
fn overview_navigation_has_its_own_scrollbar_on_short_terminals() {
    let (mut ui, key) = detail_mount(Screen::Overview, "midnight", 120, 14);
    let summary = rect(&ui, &key);
    let navigation = Rect {
        x: 0,
        y: summary.y,
        w: u16::try_from(summary.x).unwrap(),
        h: summary.h,
    };
    let before = content(&ui, summary);
    visible_thumb(&ui, navigation);
    visible_thumb(&ui, summary);
    drag_to(&mut ui, navigation, true);
    assert!(content(&ui, navigation).contains("Changes"));
    assert_eq!(
        ui.state().scope,
        Scope::Details,
        "sidebar scrolling must not enter a section"
    );
    assert_eq!(content(&ui, summary), before);
    retain_content(&mut ui, navigation);
    let sidebar = content(&ui, navigation);
    drag_to(&mut ui, summary, true);
    assert!(content(&ui, summary).contains("DESCRIPTION_089"));
    assert_eq!(content(&ui, navigation), sidebar);
}

#[test]
fn overflowing_detail_scrollbars_use_the_actual_viewport_right_edge() {
    for theme in THEMES {
        for screen in [
            Screen::Overview,
            Screen::Fields,
            Screen::Description,
            Screen::Activity,
            Screen::Pipeline,
            Screen::Jobs,
            Screen::Discussions,
            Screen::Changes,
        ] {
            let (ui, key) = detail_mount(screen, theme, 120, 14);
            let area = rect(&ui, &key);
            let bar = rendered_bar(&ui, area);
            assert_eq!(right(area), 119);
            assert_eq!(
                right(bar),
                right(area),
                "{screen:?}, {theme}: scrollbar is inset from its viewport edge"
            );
            visible_thumb(&ui, area);
        }
    }
}

#[test]
fn fitting_and_empty_detail_content_has_no_scrollbar_in_any_theme() {
    for theme in THEMES {
        for screen in [
            Screen::Overview,
            Screen::Fields,
            Screen::Description,
            Screen::Activity,
            Screen::Pipeline,
            Screen::Jobs,
            Screen::Discussions,
            Screen::Changes,
        ] {
            let (mut ui, viewport_key) = detail_mount(screen, theme, 120, 80);
            let details = ui.state_mut().details.as_mut().unwrap();
            details.item.description = "A short description.".into();
            details.notes.truncate(1);
            details.notes[0].body = "A short note.".into();
            details.jobs.truncate(1);
            details.discussions.truncate(1);
            details.discussions[0].notes[0].body = "A short discussion.".into();
            details.diffs[0].diff = "+A short change.".into();
            settle(&mut ui);
            let area = rect(&ui, &viewport_key);
            assert!(
                thumb(&ui, area).is_none(),
                "{screen:?}, {theme}: fitting content must not have a thumb"
            );

            let details = ui.state_mut().details.as_mut().unwrap();
            details.item.description.clear();
            details.item.pipeline = None;
            details.notes.clear();
            details.jobs.clear();
            details.discussions.clear();
            details.diffs.clear();
            ui.dispatch(Msg::Refresh).unwrap();
            settle(&mut ui);
            assert!(
                thumb(&ui, content_rect(&ui)).is_none(),
                "{screen:?}, {theme}: empty content must not have a thumb"
            );
        }
    }
}

#[test]
fn stacked_discussion_panes_scroll_independently_on_narrow_terminals() {
    let (mut ui, key) = detail_mount(Screen::Discussions, "midnight", 80, 26);
    let notes = rect(&ui, &key);
    assert_eq!(right(notes), 79);
    let summaries = Rect {
        x: 0,
        y: 6,
        w: 80,
        h: u16::try_from(notes.y - 6).unwrap(),
    };
    assert!(summaries.h > 0);
    let before = content(&ui, notes);
    drag_to(&mut ui, summaries, true);
    assert!(content(&ui, summaries).contains("THREAD_019"));
    assert_eq!(content(&ui, notes), before);
    retain_content(&mut ui, summaries);
    let before = content(&ui, summaries);
    drag_to(&mut ui, notes, true);
    assert!(content(&ui, notes).contains("DISCUSSION_000_059"));
    assert_eq!(content(&ui, summaries), before);
    retain_content(&mut ui, notes);
}

// Modal is a portal, so locate its actual rounded frame rather than assuming that
// rect_of_key("dialog") is the inner viewport. Cronk uses one border and one padding cell.
fn dialog_body(ui: &Ui) -> Rect {
    let frame = ui.capture_frame();
    let (left, top) = (0..frame.height)
        .find_map(|y| {
            (0..frame.width)
                .find(|&x| frame.cell(x, y).symbol == "╭")
                .map(|x| (x, y))
        })
        .expect("dialog must render a rounded frame");
    let right = (left + 1..frame.width)
        .find(|&x| frame.cell(x, top).symbol == "╮")
        .unwrap();
    let bottom = (top + 1..frame.height)
        .find(|&y| frame.cell(right, y).symbol == "╯")
        .unwrap();
    Rect {
        x: (left + 2) as i16,
        y: (top + 2) as i16,
        w: right - left - 3,
        h: bottom - top - 3,
    }
}

#[test]
fn help_dialog_scrollbar_reaches_hidden_help_without_closing() {
    let mut ui = mount(config(2, "midnight"), 100, 24);
    key(&mut ui, KeyCode::Char('?'));
    let area = dialog_body(&ui);
    let before = content(&ui, area);
    assert!(before.contains("Navigation"));
    visible_thumb(&ui, area);
    click(&mut ui, right(area), bottom(area));
    assert!(ui.state().dialog.is_some(), "track must not dismiss help");
    assert_ne!(content(&ui, area), before);
    drag_to(&mut ui, area, true);
    assert!(
        content(&ui, area).contains("DEMO"),
        "help scrollbar must expose the last help paragraph"
    );
    retain_content(&mut ui, area);
    drag_to(&mut ui, area, false);
    assert_eq!(content(&ui, area), before);
    drag_to(&mut ui, area, true);
    key(&mut ui, KeyCode::Esc);
    assert!(
        ui.state().dialog.is_none(),
        "scrolled help must still handle Esc"
    );
}

#[test]
fn command_palette_scrollbar_reaches_all_commands_without_running_one() {
    let mut ui = mount(config(2, "midnight"), 100, 24);
    key(&mut ui, KeyCode::Char(':'));
    let area = dialog_body(&ui);
    assert!(content(&ui, area).contains("Refresh now"));
    assert!(!content(&ui, area).contains("Quit"));
    visible_thumb(&ui, area);
    click(&mut ui, right(area), bottom(area));
    assert!(ui.state().dialog.is_some());
    drag_to(&mut ui, area, true);
    assert!(
        content(&ui, area).contains("Quit"),
        "the scrollbar must cover all commands, not a pre-sliced subset"
    );
    assert!(
        ui.state().dialog.is_some(),
        "dragging must not run a command"
    );
    assert_eq!(ui.state().scope, Scope::List);
    retain_content(&mut ui, area);
}

#[test]
fn multiline_dialog_editor_scrollbar_moves_text_without_editing_or_submitting() {
    let (mut ui, _) = detail_mount(Screen::Description, "midnight", 120, 30);
    key(&mut ui, KeyCode::Enter);
    let viewport = rect(&ui, "dialog-field-0");
    let area = rendered_bar(&ui, viewport);
    assert_eq!(
        right(area),
        right(viewport),
        "editor scrollbar must use its own right edge"
    );
    let value = ui.state().dialog.as_ref().unwrap().fields[0]
        .value()
        .to_owned();
    visible_thumb(&ui, area);
    drag_to(&mut ui, area, true);
    assert!(content(&ui, area).contains("DESCRIPTION_089"));
    retain_content(&mut ui, area);
    drag_to(&mut ui, area, false);
    assert!(content(&ui, area).contains("DESCRIPTION_000"));
    retain_content(&mut ui, area);
    assert_eq!(ui.state().dialog.as_ref().unwrap().fields[0].value(), value);
    assert!(!ui.state().mutation_pending);
    assert_eq!(ui.state().scope, Scope::Section);
}

fn dialog_target_visible(ui: &Ui, target: &str) {
    let viewport = rect(ui, "dialog-scroll");
    let target_rect = rect(ui, target);
    assert!(
        target_rect.h > 0
            && target_rect.y >= viewport.y
            && i32::from(target_rect.y) + i32::from(target_rect.h)
                <= i32::from(viewport.y) + i32::from(viewport.h),
        "{target} must be fully revealed in {viewport:?}, got {target_rect:?}:\n{}",
        ui.capture_frame().plain_text()
    );
}

fn dialog_input_focused(ui: &Ui, index: usize) {
    assert_eq!(
        ui.focused_key(),
        Some(&format!("dialog-field-{index}").into()),
        "scrolling/revealing must preserve real input focus"
    );
}

fn palette_selection_visible(ui: &Ui, selected: usize) {
    let dialog = ui.state().dialog.as_ref().unwrap();
    assert!(matches!(dialog.kind, DialogKind::Commands));
    assert_eq!(dialog.selected, selected);
    dialog_input_focused(ui, 0);
    dialog_target_visible(ui, &format!("dialog-option-{selected}"));
    let label = ui.state().command_options()[selected].0;
    assert!(
        content(ui, rect(ui, "dialog-scroll")).contains(&format!("› {label}")),
        "the selected command must be visibly painted, not just present in the tree: {label}"
    );
}

fn shift_tab(ui: &mut Ui) {
    ui.send_key(KeyEvent {
        code: KeyCode::Tab,
        mods: KeyMods::SHIFT,
    })
    .unwrap();
    settle(ui);
}

fn dragged_palette() -> Ui {
    let mut ui = mount(config(2, "midnight"), 100, 16);
    key(&mut ui, KeyCode::Char(':'));
    palette_selection_visible(&ui, 0);
    let viewport = rect(&ui, "dialog-scroll");
    visible_thumb(&ui, viewport);
    drag_to(&mut ui, viewport, true);
    assert!(content(&ui, viewport).contains("Quit"));
    assert!(!content(&ui, viewport).contains("Refresh now"));
    assert_eq!(ui.state().dialog.as_ref().unwrap().selected, 0);
    dialog_input_focused(&ui, 0);
    ui
}

#[test]
fn dialog_regression_palette_keyboard_reveals_selection_after_drag_without_blurring_query() {
    let mut ui = dragged_palette();
    let count = ui.state().command_options().len();
    for selected in 1..count {
        key(&mut ui, KeyCode::Down);
        palette_selection_visible(&ui, selected);
    }
    // Scrolling the selected command into view must not refocus it instead of the Input.
    let viewport = rect(&ui, "dialog-scroll");
    drag_to(&mut ui, viewport, false);
    dialog_input_focused(&ui, 0);
    key(&mut ui, KeyCode::Up);
    palette_selection_visible(&ui, count - 2);
    shift_tab(&mut ui);
    palette_selection_visible(&ui, count - 3);
    key(&mut ui, KeyCode::Tab);
    palette_selection_visible(&ui, count - 2);
    assert_eq!(ui.state().dialog.as_ref().unwrap().fields[0].value(), "");
}

#[test]
fn dialog_regression_palette_clamped_arrow_reveals_selection_after_drag() {
    let mut ui = dragged_palette();
    // The cursor is already zero. A keyboard navigation attempt must still recover
    // the selected command after manual scrolling, even though its key is unchanged.
    key(&mut ui, KeyCode::Up);
    palette_selection_visible(&ui, 0);
}

#[test]
fn dialog_regression_palette_query_and_keyboard_work_after_drag_and_filtering() {
    let mut ui = dragged_palette();
    let count = ui.state().command_options().len();
    let mut query = String::new();
    for ch in "project".chars() {
        key(&mut ui, KeyCode::Char(ch));
        query.push(ch);
        dialog_input_focused(&ui, 0);
        assert_eq!(ui.state().dialog.as_ref().unwrap().fields[0].value(), query);
    }
    let filtered = ui.state().command_options().len();
    assert!(filtered > 1 && filtered < count);
    assert!(
        ui.state()
            .command_options()
            .iter()
            .all(|(label, _)| label.to_lowercase().contains("project"))
    );
    palette_selection_visible(&ui, 0);
    for selected in 1..filtered {
        key(&mut ui, KeyCode::Down);
        palette_selection_visible(&ui, selected);
    }
    shift_tab(&mut ui);
    palette_selection_visible(&ui, filtered - 2);

    key(&mut ui, KeyCode::Home);
    ui.send_key(KeyEvent {
        code: KeyCode::End,
        mods: KeyMods::SHIFT,
    })
    .unwrap();
    key(&mut ui, KeyCode::Backspace);
    assert_eq!(ui.state().dialog.as_ref().unwrap().fields[0].value(), "");
    assert_eq!(ui.state().command_options().len(), count);
    palette_selection_visible(&ui, 0);
    key(&mut ui, KeyCode::Tab);
    palette_selection_visible(&ui, 1);
    shift_tab(&mut ui);
    palette_selection_visible(&ui, 0);
}

fn creation_field_visible(ui: &Ui, selected: usize, draft: &[String]) {
    let dialog = ui.state().dialog.as_ref().unwrap();
    assert!(matches!(dialog.kind, DialogKind::NewMergeRequest));
    assert_eq!(dialog.selected, selected);
    dialog_input_focused(ui, selected);
    dialog_target_visible(ui, &format!("dialog-field-{selected}"));
    assert_eq!(
        dialog
            .fields
            .iter()
            .map(|field| field.value())
            .collect::<Vec<_>>(),
        draft.iter().map(String::as_str).collect::<Vec<_>>(),
        "scrolling/focus changes must retain the entire unsent draft"
    );
    for line in draft[selected].lines() {
        assert!(
            content(ui, rect(ui, "dialog-scroll")).contains(line),
            "focused draft text must be visible: {line}"
        );
    }
    assert!(!ui.state().mutation_pending);
}

#[test]
fn dialog_regression_short_creation_form_tabs_reveal_fields_and_preserve_draft() {
    let mut ui = mount(config(3, "midnight"), 100, 16);
    key(&mut ui, KeyCode::Char('n'));
    let mut draft: Vec<_> = ui
        .state()
        .dialog
        .as_ref()
        .unwrap()
        .fields
        .iter()
        .map(|field| field.value().to_owned())
        .collect();
    assert_eq!(draft.len(), 5);
    let viewport = rect(&ui, "dialog-scroll");
    visible_thumb(&ui, viewport);
    let description = rect(&ui, "dialog-field-4");
    assert!(
        i32::from(description.y) + i32::from(description.h)
            > i32::from(viewport.y) + i32::from(viewport.h),
        "fixture must require scrolling to reach the last field"
    );

    for (index, value) in [
        "9002",
        "Short draft",
        "feature/scroll-form",
        "release/test",
        "Draft line one\nDraft line two",
    ]
    .into_iter()
    .enumerate()
    {
        creation_field_visible(&ui, index, &draft);
        if index < 4 {
            key(&mut ui, KeyCode::Home);
            ui.send_key(KeyEvent {
                code: KeyCode::End,
                mods: KeyMods::SHIFT,
            })
            .unwrap();
        }
        ui.send_paste(value).unwrap();
        settle(&mut ui);
        draft[index] = value.into();
        creation_field_visible(&ui, index, &draft);
        if index < 4 {
            key(&mut ui, KeyCode::Tab);
        }
    }
    assert!(visible_thumb(&ui, rect(&ui, "dialog-scroll")).start > 0);

    // Leave focus on the last editor while moving the actual outer scrollbar away.
    let viewport = rect(&ui, "dialog-scroll");
    drag_to(&mut ui, viewport, false);
    dialog_input_focused(&ui, 4);
    for selected in (0..4).rev() {
        shift_tab(&mut ui);
        creation_field_visible(&ui, selected, &draft);
    }
    shift_tab(&mut ui);
    creation_field_visible(&ui, 4, &draft);
    key(&mut ui, KeyCode::Tab);
    creation_field_visible(&ui, 0, &draft);
    assert_eq!(ui.state().config.active_tab, 3);
}

#[test]
fn dialog_regression_fitting_dialogs_have_no_scrollbar() {
    for (shortcut, height) in [(':', 40), ('n', 40), ('?', 60), ('/', 20)] {
        let mut ui = mount(config(3, "midnight"), 100, height);
        key(&mut ui, KeyCode::Char(shortcut));
        assert!(ui.state().dialog.is_some());
        let viewport = rect(&ui, "dialog-scroll");
        assert!(
            thumb(&ui, viewport).is_none(),
            "fitting {shortcut:?} dialog must not show a scrollbar"
        );
        dialog_target_visible(&ui, "dialog-close");
        if shortcut == ':' {
            for index in 0..ui.state().command_options().len() {
                dialog_target_visible(&ui, &format!("dialog-option-{index}"));
            }
        }
        for index in 0..ui.state().dialog.as_ref().unwrap().fields.len() {
            dialog_target_visible(&ui, &format!("dialog-field-{index}"));
        }
        let before = content(&ui, viewport);
        mouse(
            &mut ui,
            right(viewport),
            top(viewport),
            MouseKind::ScrollDown,
        );
        settle(&mut ui);
        assert_eq!(
            content(&ui, viewport),
            before,
            "a fitting dialog must not scroll"
        );
        assert!(thumb(&ui, viewport).is_none());
    }
}
