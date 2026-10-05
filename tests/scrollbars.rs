use std::time::Duration;

use cronk::{
    config::{Config, SavedView},
    demo,
    model::{Details, Diff, Discussion, ItemKind, Job, Note, Pipeline},
    scroll::BoundaryScroll,
    ui::{Cronk, DialogKind, Msg, Scope, Trace, list_height_for_tab, list_row_height},
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
    assert_eq!(rect.h as usize, visible_slots(ui) * row_height(ui));
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

fn row_height(ui: &Ui) -> usize {
    list_row_height(ui.state().config.active_tab)
}

fn visible_slots(ui: &Ui) -> usize {
    list_height_for_tab(ui.viewport().h, ui.state().config.active_tab)
}

fn content_height(ui: &Ui, items: usize) -> usize {
    items * row_height(ui)
}

fn assert_list_content(ui: &Ui, area: Rect) {
    let offset = ui.state().scroll.offset;
    let slots = visible_slots(ui);
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
            area.y + (row_height(ui) * row) as i16,
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
        assert!(len > visible_slots(&ui), "fixture must overflow: tab {tab}");
        assert_eq!(
            proportional(&ui, area, content_height(&ui, len)).start,
            0,
            "{theme}, tab {tab}"
        );

        ui.set_viewport(Rect {
            x: 0,
            y: 0,
            w: 120,
            h: (content_height(&ui, len) + 8) as u16,
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
        let slots = visible_slots(&ui);
        drag_to(&mut ui, area, true);
        assert_eq!(ui.state().scope, Scope::List);
        assert_eq!(
            ui.state().scroll.offset,
            len - slots,
            "drag must reach the final complete list page"
        );
        let bar = proportional(&ui, area, content_height(&ui, len));
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
        assert_eq!(proportional(&ui, area, content_height(&ui, len)).start, 0);
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
    let initial = proportional(&ui, area, content_height(&ui, list_len(&ui)));
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
    Issue,
    MergeRequest,
    // Keep the editor-dialog fixture mounted directly in its action scope.
    Description,
}

impl Screen {
    fn kind(self) -> ItemKind {
        if matches!(self, Self::Issue) {
            ItemKind::Issue
        } else {
            ItemKind::MergeRequest
        }
    }

    fn last_marker(self) -> &'static str {
        if matches!(self, Self::Issue) {
            "ACTIVITY_000_004"
        } else {
            "CHANGE_089"
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

fn detail_fixture(details: &mut Details) {
    details.item.title = "SCROLL_TITLE".into();
    details.item.description = document("DESCRIPTION", 90);
    details.item.web_url = "https://example.invalid/SCROLL_END".into();
    details.item.pipeline = Some(Pipeline {
        id: 700,
        status: "running".into(),
        web_url: "https://example.invalid/pipeline".into(),
    });
    details.warnings.clear();
    // Deliberately interleave timestamps and tie them across different IDs. Neither
    // input order, reverse input order, nor an ID-only sort is newest-first.
    details.notes = (0..8)
        .map(|n| Note {
            id: 20_000 + n,
            author: demo::user(),
            body: document(&format!("ACTIVITY_{n:03}"), 5),
            created_at: format!("2026-01-0{}T00:00:00Z", n % 4 + 1),
            ..Note::default()
        })
        .collect();
    details.jobs = (0..8)
        .map(|n| Job {
            id: 30_000 + n,
            name: format!("JOB_{n:03}"),
            stage: "test".into(),
            status: if n == 1 { "running" } else { "success" }.into(),
            ..Job::default()
        })
        .collect();
    details.discussions = (0..4)
        .map(|n| Discussion {
            id: format!("scroll-thread-{n}"),
            notes: (0..2)
                .map(|reply| Note {
                    id: 40_000 + n * 2 + reply,
                    author: demo::user(),
                    body: format!(
                        "THREAD_{n:03} reply {reply}\n\n{}",
                        document(&format!("DISCUSSION_{n:03}_NOTE_{reply}"), 20)
                    ),
                    resolvable: true,
                    ..Note::default()
                })
                .collect(),
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
    let kind = screen.kind();
    let route = demo::items()
        .into_iter()
        .find(|item| item.key.kind == kind)
        .unwrap()
        .key;
    let mut config = config(if kind == ItemKind::Issue { 2 } else { 3 }, theme);
    let key = format!(
        "detail-{}-{}-{}-{}",
        config.active_tab,
        route.project,
        kind.segment(),
        route.iid
    );
    config.route = Some(route);
    config.section = matches!(screen, Screen::Description).then_some(1);
    let mut ui = mount(config, width, height);
    detail_fixture(ui.state_mut().details.as_mut().unwrap());
    ui.state_mut().traces.clear();
    ui.state_mut().expanded.clear();
    ui.state_mut().expanded.insert(30_000);
    // Unknown running demo job IDs produce stable offline traces across polling.
    // Finished traces also let us prove that collapsed jobs do not render logs.
    let jobs = ui.state().details.as_ref().unwrap().jobs.clone();
    for job in jobs {
        ui.state_mut().traces.insert(
            job.id,
            Trace {
                text: if job.running() {
                    demo::trace(job.id, 0)
                } else {
                    format!("TRACE_{}_BEGIN\nTRACE_{}_END", job.id, job.id)
                },
                finished: !job.running(),
                ..Trace::default()
            },
        );
    }
    settle(&mut ui);
    (ui, key)
}

fn detail_area(ui: &Ui, viewport_key: &str) -> Rect {
    let area = rect(ui, viewport_key);
    assert_eq!(
        area,
        content_rect(ui),
        "the document must fill the main pane"
    );
    area
}

fn single_detail_bar(ui: &Ui, area: Rect, overflowing: bool) {
    let frame = ui.capture_frame();
    for y in top(area)..=bottom(area) {
        for x in u16::try_from(area.x).unwrap()..=right(area) {
            if matches!(frame.cell(x, y).symbol.as_str(), "█" | "▀" | "▄") {
                assert!(
                    overflowing,
                    "fitting document has a scrollbar at ({x}, {y})"
                );
                assert_eq!(x, right(area), "nested or inset scrollbar at ({x}, {y})");
            }
        }
    }
    if overflowing {
        visible_thumb(ui, area);
    } else {
        assert!(thumb(ui, area).is_none());
        assert_eq!(ui.state().content_offset, 0);
    }
}

fn detail_markers(screen: Screen) -> Vec<String> {
    let mut markers = vec!["SCROLL_TITLE".into(), "SCROLL_END".into()];
    markers.extend((0..90).map(|n| format!("DESCRIPTION_{n:03}")));
    if screen.kind() == ItemKind::Issue {
        // Timestamp descending, then ID descending within each timestamp.
        for n in [7, 3, 6, 2, 5, 1, 4, 0] {
            markers.extend((0..5).map(|line| format!("ACTIVITY_{n:03}_{line:03}")));
        }
    } else {
        markers.push("Pipeline #700".into());
        for n in 0..8 {
            markers.push(format!("JOB_{n:03}"));
            if n == 0 {
                markers.extend(["TRACE_30000_BEGIN".into(), "TRACE_30000_END".into()]);
            } else if n == 1 {
                markers.push("fixture for job 30001.".into());
            }
        }
        for n in 0..4 {
            for reply in 0..2 {
                markers.extend(
                    (0..20).map(|line| format!("DISCUSSION_{n:03}_NOTE_{reply}_{line:03}")),
                );
            }
        }
        markers.extend((0..90).map(|n| format!("CHANGE_{n:03}")));
    }
    markers
}

// Record document-relative rows so overlapping pages neither double-count content
// nor conceal duplicated job headers/logs in the Pipeline summary. No layout row
// counts are assumed: keyboard paging/native wheel must expose every marker in order.
fn scan_detail_pages(ui: &mut Ui, area: Rect, markers: &[String]) {
    let mut positions = vec![None; markers.len()];
    let scope = ui.state().scope;
    let section = ui.state().config.section;
    let field = ui.state().config.field;
    let selected = ui.state().section_cursor;
    let mut reached_end = false;
    for _ in 0..1024 {
        assert_eq!(ui.state().scope, scope);
        assert_eq!(
            ui.state().section_cursor,
            selected,
            "paging selected a section"
        );
        assert_eq!(ui.state().config.section, section);
        assert_eq!(ui.state().config.field, field);
        assert!(ui.state().dialog.is_none());
        single_detail_bar(ui, area, true);
        let page = content(ui, area);
        for (row, line) in page.lines().enumerate() {
            for (index, marker) in markers.iter().enumerate() {
                if line.contains(marker) {
                    let position = ui.state().content_offset + row;
                    if let Some(previous) = positions[index].replace(position) {
                        assert_eq!(previous, position, "duplicated or moving content: {marker}");
                    }
                }
            }
        }
        for id in 30_002..30_008 {
            assert!(
                !page.contains(&format!("TRACE_{id}_")),
                "collapsed job log is visible"
            );
        }
        let before = ui.state().content_offset;
        if scope == Scope::Details {
            key(ui, KeyCode::PageDown);
        } else {
            mouse(ui, 5, top(area) + 1, MouseKind::ScrollDown);
            settle(ui);
        }
        let after = ui.state().content_offset;
        assert!(
            after >= before && after - before <= area.h as usize,
            "paging skipped content"
        );
        if after == before {
            reached_end = true;
            break;
        }
    }
    assert!(reached_end, "paging never reached the document end");
    let mut previous = None;
    for (marker, position) in markers.iter().zip(positions) {
        let position = position.unwrap_or_else(|| panic!("content never appeared: {marker}"));
        if let Some(previous) = previous {
            assert!(position > previous, "out-of-order content: {marker}");
        }
        previous = Some(position);
    }
    let bar = visible_thumb(ui, area);
    assert_eq!(bar.start + bar.len, 2 * area.h as usize);
    assert!(content(ui, area).contains(markers.last().unwrap()));
}

fn whole_document_scrolls(screen: Screen) {
    for theme in THEMES {
        for (width, height) in [(120, 24), (80, 26)] {
            let (mut ui, viewport_key) = detail_mount(screen, theme, width, height);
            let area = detail_area(&ui, &viewport_key);
            assert_eq!(ui.state().scope, Scope::Details);
            assert_eq!(visible_thumb(&ui, area).start, 0);
            assert!(content(&ui, area).contains("SCROLL_TITLE"));
            scan_detail_pages(&mut ui, area, &detail_markers(screen));
            retain_content(&mut ui, area);
            let end = content(&ui, area);
            let offset = ui.state().content_offset;
            key(&mut ui, KeyCode::PageUp);
            assert!(ui.state().content_offset < offset);
            assert_ne!(content(&ui, area), end, "PageUp must move actual content");
            assert_eq!(ui.state().scope, Scope::Details);
            assert_eq!(ui.state().section_cursor, 0);
            retain_content(&mut ui, area);
        }
    }
}

#[test]
fn issue_whole_document_pages_through_description_and_newest_first_activity() {
    whole_document_scrolls(Screen::Issue);
}

#[test]
fn merge_request_whole_document_pages_through_jobs_all_notes_and_final_diff() {
    whole_document_scrolls(Screen::MergeRequest);
}

#[test]
fn section_scope_wheel_reaches_all_middle_sections_and_final_content() {
    for screen in [Screen::Issue, Screen::MergeRequest] {
        let (mut ui, viewport_key) = detail_mount(screen, "midnight", 80, 26);
        let area = detail_area(&ui, &viewport_key);
        key(&mut ui, KeyCode::Enter);
        assert_eq!(ui.state().scope, Scope::Section);
        drag_to(&mut ui, area, false);
        scan_detail_pages(&mut ui, area, &detail_markers(screen));
        retain_content(&mut ui, area);
    }
}

#[test]
fn detail_single_right_edge_track_drag_and_refresh_retain_real_content() {
    for screen in [Screen::Issue, Screen::MergeRequest] {
        for theme in THEMES {
            let (mut ui, viewport_key) = detail_mount(screen, theme, 120, 24);
            let area = detail_area(&ui, &viewport_key);
            let initial = content(&ui, area);
            let route = ui.state().config.route.clone();
            let expanded = ui.state().expanded.clone();
            click(&mut ui, right(area), bottom(area));
            assert!(ui.state().content_offset > 0);
            assert_ne!(content(&ui, area), initial, "track click moved no content");
            retain_content(&mut ui, area);
            drag_to(&mut ui, area, false);
            assert_eq!(content(&ui, area), initial);

            let bar = visible_thumb(&ui, area);
            let from = top(area) + (bar.len / 4) as u16;
            mouse(
                &mut ui,
                right(area),
                from,
                MouseKind::Down(MouseButton::Left),
            );
            mouse(
                &mut ui,
                right(area),
                top(area) + area.h / 2,
                MouseKind::Drag(MouseButton::Left),
            );
            // Reconcile and refresh while the mouse is still captured, not only
            // after mouse-up; a controlled reveal must not snap back mid-drag.
            settle(&mut ui);
            assert!(ui.state().content_offset > 0);
            assert_ne!(content(&ui, area), initial);
            retain_content(&mut ui, area);
            mouse(
                &mut ui,
                right(area),
                bottom(area),
                MouseKind::Drag(MouseButton::Left),
            );
            mouse(
                &mut ui,
                right(area),
                bottom(area),
                MouseKind::Up(MouseButton::Left),
            );
            settle(&mut ui);
            assert!(content(&ui, area).contains(screen.last_marker()));
            single_detail_bar(&ui, area, true);
            let bar = visible_thumb(&ui, area);
            assert_eq!(bar.start + bar.len, 2 * area.h as usize);
            retain_content(&mut ui, area);

            let end = content(&ui, area);
            let offset = ui.state().content_offset;
            mouse(&mut ui, 5, top(area) + 1, MouseKind::ScrollUp);
            settle(&mut ui);
            assert!(ui.state().content_offset < offset);
            assert_ne!(content(&ui, area), end);
            retain_content(&mut ui, area);
            drag_to(&mut ui, area, false);
            assert_eq!(ui.state().content_offset, 0);
            assert_eq!(visible_thumb(&ui, area).start, 0);
            assert_eq!(content(&ui, area), initial);
            assert_eq!(ui.state().scope, Scope::Details);
            assert_eq!(ui.state().config.route, route);
            assert!(ui.state().config.section.is_none());
            assert_eq!(ui.state().section_cursor, 0);
            assert_eq!(ui.state().config.field, 0);
            assert_eq!(ui.state().expanded, expanded, "scrolling activated a job");
            assert!(ui.state().dialog.is_none(), "scrolling edited a field");
            assert!(!ui.state().mutation_pending);
        }
    }
}

fn section_visible(ui: &Ui, area: Rect, index: usize) {
    let header = rect(ui, &format!("detail-section-{index}"));
    assert!(
        header.y >= area.y && header.y + header.h as i16 <= area.y + area.h as i16,
        "selected section header is not fully visible: {index} in {area:?}: {header:?}"
    );
    assert!(content(ui, area).contains(ui.state().sections()[index]));
}

fn section_gap_visible(ui: &Ui, area: Rect, index: usize) {
    let gap = rect(ui, &format!("detail-section-gap-{index}"));
    assert!(
        gap.y >= area.y && gap.y + gap.h as i16 <= area.y + area.h as i16,
        "selected section end is not fully visible: {index} in {area:?}: {gap:?}"
    );
}

fn select_detail_section(ui: &mut Ui, area: Rect, index: usize) {
    assert_eq!(ui.state().scope, Scope::Details);
    key(ui, KeyCode::Home);
    for _ in 0..index {
        key(ui, KeyCode::Down);
    }
    assert_eq!(ui.state().section_cursor, index);
    section_visible(ui, area, index);
}

#[test]
fn detail_keyboard_selects_and_reveals_headers_on_short_and_narrow_terminals() {
    for screen in [Screen::Issue, Screen::MergeRequest] {
        for (width, height) in [(120, 14), (80, 26)] {
            let (mut ui, viewport_key) = detail_mount(screen, "midnight", width, height);
            let area = detail_area(&ui, &viewport_key);
            let last = ui.state().sections().len() - 1;
            for (code, index) in [
                (KeyCode::Down, 1),
                (KeyCode::Up, 0),
                (KeyCode::Tab, 1),
                (KeyCode::End, last),
                (KeyCode::Home, 0),
            ] {
                key(&mut ui, code);
                assert_eq!(ui.state().scope, Scope::Details);
                assert!(ui.state().config.section.is_none());
                assert_eq!(ui.state().section_cursor, index);
                section_visible(&ui, area, index);
                single_detail_bar(&ui, area, true);
            }
        }
    }
}

fn assert_detail_boundary(ui: &Ui, area: Rect, target: &str, previous_offset: usize) {
    let cursor = rect(ui, target);
    let document_row = i32::from(cursor.y) - i32::from(area.y) + ui.state().content_offset as i32;
    assert!(document_row >= 0);
    let mut expected = BoundaryScroll {
        selected: document_row as usize,
        offset: previous_offset,
    };
    expected.normalize(
        ui.state().content_max_offset + area.h as usize,
        area.h as usize,
    );
    assert_eq!(
        ui.state().content_offset,
        expected.offset,
        "{target} must use the quarter-viewport cursor band, not top alignment: area={area:?}, cursor={cursor:?}, previous={previous_offset}, document_row={document_row}, max={}",
        ui.state().content_max_offset
    );
    assert!(
        cursor.y >= area.y && cursor.y < area.y + area.h as i16,
        "{target} cursor is offscreen: {cursor:?}"
    );
}

fn assert_detail_section_revealed(ui: &Ui, area: Rect, index: usize, previous_offset: usize) {
    let target = format!("detail-section-{index}");
    let cursor = rect(ui, &target);
    let document_row = i32::from(cursor.y) - i32::from(area.y) + ui.state().content_offset as i32;
    assert!(document_row >= 0);
    let mut expected = BoundaryScroll {
        selected: document_row as usize,
        offset: previous_offset,
    };
    expected.normalize(
        ui.state().content_max_offset + area.h as usize,
        area.h as usize,
    );
    assert!(
        cursor.y >= area.y && cursor.y < area.y + area.h as i16,
        "{target} cursor is offscreen: {cursor:?}"
    );
    if expected.offset > 0 {
        assert!(
            ui.state().content_offset > 0,
            "{target} must still scroll into the document instead of top-aligning"
        );
    }
}

#[test]
fn detail_selection_reveals_the_full_section_when_it_fits() {
    for screen in [Screen::Issue, Screen::MergeRequest] {
        let (mut ui, viewport_key) = detail_mount(screen, "midnight", 80, 17);
        replace_details(&mut ui, |details| {
            compact_details(details);
            details.item.description = document("FIT_DESCRIPTION", 2);
        });
        let area = detail_area(&ui, &viewport_key);
        key(&mut ui, KeyCode::Home);
        key(&mut ui, KeyCode::Down);
        assert_eq!(ui.state().scope, Scope::Details);
        assert_eq!(ui.state().section_cursor, 1);
        section_visible(&ui, area, 1);
        section_gap_visible(&ui, area, 1);
        assert!(content(&ui, area).contains("FIT_DESCRIPTION_001"));
    }
}

#[test]
fn detail_section_navigation_uses_boundary_scrolling_in_both_directions() {
    for screen in [Screen::Issue, Screen::MergeRequest] {
        for (width, height) in [(120, 26), (40, 18)] {
            let (mut ui, viewport_key) = detail_mount(screen, "midnight", width, height);
            replace_details(&mut ui, compact_details);
            let area = detail_area(&ui, &viewport_key);
            let last = ui.state().sections().len() - 1;
            for code in std::iter::repeat_n(KeyCode::Down, last)
                .chain(std::iter::repeat_n(KeyCode::Up, last))
                .chain([KeyCode::End, KeyCode::Home])
            {
                let previous = ui.state().content_offset;
                key(&mut ui, code);
                assert_detail_section_revealed(&ui, area, ui.state().section_cursor, previous);
            }
            // Manual paging does not change selection. A clamped Up must still
            // recover that same header inside the band instead of at the top.
            drag_to(&mut ui, area, true);
            let previous = ui.state().content_offset;
            key(&mut ui, KeyCode::Up);
            assert_detail_section_revealed(&ui, area, 0, previous);
        }
    }
}

#[test]
fn focused_detail_items_use_the_same_boundary_band_and_preserve_it_on_reversal() {
    for screen in [Screen::Issue, Screen::MergeRequest] {
        for (width, height) in [(120, 26), (40, 18)] {
            let (mut ui, viewport_key) = detail_mount(screen, "midnight", width, height);
            let area = detail_area(&ui, &viewport_key);
            let sections: &[usize] = if matches!(screen, Screen::Issue) {
                &[0]
            } else {
                &[0, 3, 4]
            };
            for &section in sections {
                select_detail_section(&mut ui, area, section);
                let previous = ui.state().content_offset;
                key(&mut ui, KeyCode::Enter);
                let (keys, len): (Vec<String>, usize) = match section {
                    0 => {
                        let len = ui.state().fields().len();
                        ((0..len).map(|i| format!("edit-field-{i}")).collect(), len)
                    }
                    3 => {
                        let jobs = &ui.state().details.as_ref().unwrap().jobs;
                        (
                            jobs.iter().map(|j| format!("job-{}", j.id)).collect(),
                            jobs.len(),
                        )
                    }
                    4 => {
                        let discussions = &ui.state().details.as_ref().unwrap().discussions;
                        (
                            discussions
                                .iter()
                                .map(|d| format!("discussion-{}", d.id))
                                .collect(),
                            discussions.len(),
                        )
                    }
                    _ => unreachable!(),
                };
                assert_detail_boundary(&ui, area, &keys[0], previous);
                let mut moved_without_scrolling = false;
                let mut scrolled_at_boundary = false;
                for code in std::iter::repeat_n(KeyCode::Down, len - 1)
                    .chain(std::iter::repeat_n(KeyCode::Up, len - 1))
                    .chain([KeyCode::End, KeyCode::Home])
                {
                    let previous = ui.state().content_offset;
                    let field = ui.state().config.field;
                    key(&mut ui, code);
                    assert_detail_boundary(&ui, area, &keys[ui.state().config.field], previous);
                    moved_without_scrolling |=
                        field != ui.state().config.field && previous == ui.state().content_offset;
                    scrolled_at_boundary |= previous != ui.state().content_offset;
                }
                if section == 0 {
                    assert!(
                        moved_without_scrolling,
                        "cursor never traversed the viewport band"
                    );
                    assert!(
                        scrolled_at_boundary,
                        "Fields never scrolled at the band boundary"
                    );
                }
                key(&mut ui, KeyCode::Esc);
            }
        }
    }
}

#[test]
fn selected_detail_section_has_a_distinct_background_in_every_theme() {
    for theme in THEMES {
        let (mut ui, viewport_key) = detail_mount(Screen::Issue, theme, 120, 120);
        replace_details(&mut ui, compact_details);
        let area = detail_area(&ui, &viewport_key);
        let selected = match theme {
            "light" => Color::hex_u24(0xdce5fa),
            "dracula" => Color::hex_u24(0x44475a),
            _ => Color::hex_u24(0x293654),
        };
        let first = rect(&ui, "detail-section-0");
        let second = rect(&ui, "detail-section-1");
        let frame = ui.capture_frame();
        assert_eq!(frame.cell(right(area) - 2, top(first)).bg, selected);
        assert_ne!(frame.cell(right(area) - 2, top(second)).bg, selected);
        key(&mut ui, KeyCode::Tab);
        let frame = ui.capture_frame();
        let first = rect(&ui, "detail-section-0");
        let second = rect(&ui, "detail-section-1");
        assert_ne!(frame.cell(right(area) - 2, top(first)).bg, selected);
        assert_eq!(frame.cell(right(area) - 2, top(second)).bg, selected);
        shift_tab(&mut ui);
        assert_eq!(ui.state().section_cursor, 0);
    }
}

#[test]
fn keyboard_recovers_the_same_selected_section_after_dragging_away() {
    let (mut ui, viewport_key) = detail_mount(Screen::Issue, "midnight", 80, 26);
    let area = detail_area(&ui, &viewport_key);
    assert_eq!(ui.state().section_cursor, 0);
    drag_to(&mut ui, area, true);
    key(&mut ui, KeyCode::Up);
    assert_eq!(ui.state().section_cursor, 0);
    section_visible(&ui, area, 0);
    key(&mut ui, KeyCode::Tab);
    shift_tab(&mut ui);
    section_visible(&ui, area, 0);
}

#[test]
fn every_section_keeps_the_whole_document_and_escape_retains_the_dragged_viewport() {
    for screen in [Screen::Issue, Screen::MergeRequest] {
        for theme in THEMES {
            let (mut ui, viewport_key) = detail_mount(screen, theme, 80, 26);
            let area = detail_area(&ui, &viewport_key);
            for index in 0..ui.state().sections().len() {
                select_detail_section(&mut ui, area, index);
                key(&mut ui, KeyCode::Enter);
                assert_eq!(ui.state().scope, Scope::Section);
                assert_eq!(ui.state().config.section, Some(index));
                assert_eq!(detail_area(&ui, &viewport_key), area);
                drag_to(&mut ui, area, false);
                assert!(content(&ui, area).contains("SCROLL_TITLE"));
                drag_to(&mut ui, area, true);
                assert!(content(&ui, area).contains(screen.last_marker()));
                single_detail_bar(&ui, area, true);
                retain_content(&mut ui, area);
                let page = content(&ui, area);
                let offset = ui.state().content_offset;
                let bar = visible_thumb(&ui, area);
                key(&mut ui, KeyCode::Esc);
                assert_eq!(ui.state().scope, Scope::Details);
                assert!(ui.state().config.section.is_none());
                assert_eq!(ui.state().section_cursor, index);
                assert_eq!(ui.state().content_offset, offset, "Esc reset the viewport");
                assert_eq!(visible_thumb(&ui, area), bar);
                assert_eq!(content(&ui, area), page, "Esc changed the visible document");
                retain_content(&mut ui, area);
            }
        }
    }
}

fn replace_details(ui: &mut Ui, update: impl FnOnce(&mut Details)) {
    let mut details = ui.state().details.clone().unwrap();
    update(&mut details);
    ui.dispatch(Msg::DetailsLoaded(
        details.item.key.clone(),
        ui.state().detail_epoch,
        Ok(Box::new(details)),
    ))
    .unwrap();
    settle(ui);
}

fn compact_details(details: &mut Details) {
    details.item.description = "SHORT_DESCRIPTION".into();
    details.notes.truncate(1);
    details.notes[0].body = "SHORT_ACTIVITY".into();
    details.jobs.truncate(1);
    details.discussions.truncate(1);
    details.discussions[0].notes.truncate(1);
    details.discussions[0].notes[0].body = "SHORT_DISCUSSION".into();
    details.diffs[0].diff = "@@ -0,0 +1 @@\n+SHORT_CHANGE".into();
}

fn final_section_document(details: &mut Details, prefix: &str, count: usize) {
    if details.item.key.kind == ItemKind::Issue {
        details.notes[0].body = document(prefix, count);
    } else {
        details.diffs[0].diff = format!(
            "@@ -0,0 +1,{count} @@\n{}",
            (0..count)
                .map(|n| format!("+{prefix}_{n:03}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

#[test]
fn whole_detail_thumb_updates_on_shrink_growth_and_resize_without_stale_offsets() {
    for screen in [Screen::Issue, Screen::MergeRequest] {
        for theme in THEMES {
            let (mut ui, viewport_key) = detail_mount(screen, theme, 120, 32);
            let area = detail_area(&ui, &viewport_key);
            let long = visible_thumb(&ui, area);
            drag_to(&mut ui, area, true);
            replace_details(&mut ui, |details| {
                compact_details(details);
                final_section_document(details, "SHORTER", 80);
            });
            assert!(visible_thumb(&ui, area).len > long.len);
            assert!(
                content(&ui, area).contains("SHORTER_079"),
                "shrink must clamp to real last content"
            );
            retain_content(&mut ui, area);

            replace_details(&mut ui, compact_details);
            ui.set_viewport(Rect {
                x: 0,
                y: 0,
                w: 120,
                h: 160,
            });
            settle(&mut ui);
            let fitting = detail_area(&ui, &viewport_key);
            single_detail_bar(&ui, fitting, false);
            assert!(content(&ui, fitting).contains("SCROLL_TITLE"));
            assert!(content(&ui, fitting).contains("SHORT_DESCRIPTION"));
            assert!(
                content(&ui, fitting).contains(if screen.kind() == ItemKind::Issue {
                    "SHORT_ACTIVITY"
                } else {
                    "SHORT_CHANGE"
                })
            );

            replace_details(&mut ui, |details| {
                final_section_document(details, "GROWN", 240)
            });
            let grown = visible_thumb(&ui, fitting);
            assert_eq!(grown.start, 0, "growth resurrected a stale offset");
            assert!(content(&ui, fitting).contains("SCROLL_TITLE"));
            ui.set_viewport(Rect {
                x: 0,
                y: 0,
                w: 83,
                h: 40,
            });
            settle(&mut ui);
            let narrow = detail_area(&ui, &viewport_key);
            single_detail_bar(&ui, narrow, true);
            let smaller = visible_thumb(&ui, narrow);
            assert!(smaller.len < grown.len);
            drag_to(&mut ui, narrow, true);
            assert!(content(&ui, narrow).contains("GROWN_239"));
            retain_content(&mut ui, narrow);

            ui.set_viewport(Rect {
                x: 0,
                y: 0,
                w: 120,
                h: 80,
            });
            settle(&mut ui);
            let taller = detail_area(&ui, &viewport_key);
            single_detail_bar(&ui, taller, true);
            assert!(visible_thumb(&ui, taller).len > smaller.len);
            assert!(
                content(&ui, taller).contains("GROWN_239"),
                "resize lost the last page"
            );
            retain_content(&mut ui, taller);
        }
    }
}

#[test]
fn fitting_and_empty_whole_details_have_no_scrollbars_in_either_scope_or_any_theme() {
    for screen in [Screen::Issue, Screen::MergeRequest] {
        for theme in THEMES {
            let (mut ui, viewport_key) = detail_mount(screen, theme, 120, 120);
            replace_details(&mut ui, compact_details);
            let area = detail_area(&ui, &viewport_key);
            for scope in [Scope::Details, Scope::Section] {
                if scope == Scope::Section {
                    key(&mut ui, KeyCode::Enter);
                }
                assert_eq!(ui.state().scope, scope);
                single_detail_bar(&ui, area, false);
                let page = content(&ui, area);
                assert!(page.contains("SCROLL_TITLE") && page.contains("SHORT_DESCRIPTION"));
                if screen.kind() == ItemKind::Issue {
                    assert!(page.contains("SHORT_ACTIVITY"));
                } else {
                    for marker in [
                        "Pipeline #700",
                        "JOB_000",
                        "SHORT_DISCUSSION",
                        "SHORT_CHANGE",
                    ] {
                        assert!(page.contains(marker), "fitting document omitted {marker}");
                    }
                }
                for index in 0..ui.state().sections().len() {
                    section_visible(&ui, area, index);
                }
            }
            replace_details(&mut ui, |details| {
                details.item.description.clear();
                details.item.pipeline = None;
                details.notes.clear();
                details.jobs.clear();
                details.discussions.clear();
                details.diffs.clear();
            });
            for _ in 0..2 {
                single_detail_bar(&ui, area, false);
                assert!(content(&ui, area).contains("SCROLL_TITLE"));
                for index in 0..ui.state().sections().len() {
                    section_visible(&ui, area, index);
                }
                key(&mut ui, KeyCode::Esc);
            }
        }
    }
}

fn page_to_detail_key(ui: &mut Ui, area: Rect, target: &str) {
    for _ in 0..256 {
        let target_rect = rect(ui, target);
        if target_rect.y >= area.y && target_rect.y < area.y + area.h as i16 {
            return;
        }
        let before = ui.state().content_offset;
        key(
            ui,
            if target_rect.y < area.y {
                KeyCode::PageUp
            } else {
                KeyCode::PageDown
            },
        );
        assert_ne!(ui.state().content_offset, before, "cannot reveal {target}");
    }
    panic!("paging did not expose {target}");
}

fn click_detail_key(ui: &mut Ui, area: Rect, target: &str) {
    let target = rect(ui, target);
    assert!(target.y >= area.y && target.y < area.y + area.h as i16);
    click(
        ui,
        u16::try_from(target.x.max(area.x)).unwrap() + 1,
        top(target),
    );
}

#[test]
fn refresh_preserves_the_visible_note_when_content_above_it_changes() {
    for screen in [Screen::Issue, Screen::MergeRequest] {
        let (mut ui, viewport_key) = detail_mount(screen, "midnight", 100, 18);
        let area = detail_area(&ui, &viewport_key);
        let target = if screen.kind() == ItemKind::Issue {
            "note-20006"
        } else {
            "note-40002"
        };
        page_to_detail_key(&mut ui, area, target);
        let note = rect(&ui, target);
        let offset = ui.state().content_offset + usize::try_from(note.y - area.y).unwrap() + 1;
        ui.dispatch(Msg::DetailScrolled(offset)).unwrap();
        settle(&mut ui);
        let before = content(&ui, area);
        replace_details(&mut ui, |details| {
            if screen.kind() == ItemKind::Issue {
                details.notes.push(Note {
                    id: 99_999,
                    author: demo::user(),
                    created_at: "2026-02-01T00:00:00Z".into(),
                    body: "A new event above the current viewport".into(),
                    ..Note::default()
                });
            } else {
                details.jobs[1].status = "success".into();
            }
        });
        assert_eq!(
            content(&ui, area),
            before,
            "refresh changed the note being read"
        );
        retain_content(&mut ui, area);
    }
}

#[test]
fn detail_header_and_field_clicks_select_or_edit_in_the_continuous_document() {
    for screen in [Screen::Issue, Screen::MergeRequest] {
        let (mut ui, viewport_key) = detail_mount(screen, "midnight", 80, 26);
        let area = detail_area(&ui, &viewport_key);
        page_to_detail_key(&mut ui, area, "detail-section-1");
        click_detail_key(&mut ui, area, "detail-section-1");
        assert_eq!(
            ui.state().scope,
            Scope::Details,
            "header click entered action scope"
        );
        assert_eq!(ui.state().section_cursor, 1);
        assert!(ui.state().config.section.is_none());
        assert!(ui.state().dialog.is_none());
        key(&mut ui, KeyCode::Home);
        page_to_detail_key(&mut ui, area, "edit-field-0");
        click_detail_key(&mut ui, area, "edit-field-0");
        assert!(matches!(&ui.state().dialog.as_ref().unwrap().kind,
            DialogKind::Edit(_, field) if field == "title"));
        assert_eq!(
            ui.state().dialog.as_ref().unwrap().fields[0].value(),
            "SCROLL_TITLE"
        );
        key(&mut ui, KeyCode::Esc);
        assert!(ui.state().dialog.is_none());
        assert!(!ui.state().mutation_pending);
        // Re-enter Fields through the section header and exercise keyboard actions too.
        drag_to(&mut ui, area, false);
        click_detail_key(&mut ui, area, "detail-section-0");
        key(&mut ui, KeyCode::Enter);
        assert_eq!(ui.state().scope, Scope::Section);
        key(&mut ui, KeyCode::Down);
        assert_eq!(ui.state().config.field, 1);
        key(&mut ui, KeyCode::Enter);
        assert!(matches!(&ui.state().dialog.as_ref().unwrap().kind,
            DialogKind::Edit(_, field) if field == "state_event"));
        key(&mut ui, KeyCode::Esc);
        drag_to(&mut ui, area, true);
        assert!(content(&ui, area).contains(screen.last_marker()));
    }
}

fn wheel_to_detail_text(ui: &mut Ui, area: Rect, marker: &str) {
    for _ in 0..256 {
        if content(ui, area).contains(marker) {
            return;
        }
        let before = ui.state().content_offset;
        mouse(ui, 5, top(area) + 1, MouseKind::ScrollDown);
        settle(ui);
        assert_ne!(
            ui.state().content_offset,
            before,
            "content never appeared: {marker}"
        );
    }
    panic!("scrolling did not expose {marker}");
}

#[test]
fn detail_job_clicks_and_keyboard_actions_expand_inline_logs() {
    let (mut ui, viewport_key) = detail_mount(Screen::MergeRequest, "midnight", 80, 26);
    let area = detail_area(&ui, &viewport_key);
    select_detail_section(&mut ui, area, 3);
    page_to_detail_key(&mut ui, area, "job-30002");
    assert!(!ui.state().expanded.contains(&30_002));
    click_detail_key(&mut ui, area, "job-30002");
    assert!(ui.state().expanded.contains(&30_002));
    wheel_to_detail_text(&mut ui, area, "TRACE_30002_END");
    assert!(ui.state().dialog.is_none());
    retain_content(&mut ui, area);
    if ui.state().scope == Scope::Section {
        key(&mut ui, KeyCode::Esc);
    }
    select_detail_section(&mut ui, area, 3);
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().scope, Scope::Section);
    key(&mut ui, KeyCode::Down);
    key(&mut ui, KeyCode::Down);
    assert_eq!(ui.state().config.field, 2);
    key(&mut ui, KeyCode::Enter);
    assert!(!ui.state().expanded.contains(&30_002));
    assert!(!content(&ui, area).contains("TRACE_30002_END"));
    key(&mut ui, KeyCode::Enter);
    assert!(ui.state().expanded.contains(&30_002));
    wheel_to_detail_text(&mut ui, area, "TRACE_30002_END");
    drag_to(&mut ui, area, true);
    assert!(content(&ui, area).contains("CHANGE_089"));
}

#[test]
fn detail_discussion_click_focuses_the_correct_inline_thread_for_reply_and_resolve() {
    let (mut ui, viewport_key) = detail_mount(Screen::MergeRequest, "midnight", 80, 26);
    let area = detail_area(&ui, &viewport_key);
    select_detail_section(&mut ui, area, 4);
    page_to_detail_key(&mut ui, area, "discussion-scroll-thread-2");
    click_detail_key(&mut ui, area, "discussion-scroll-thread-2");
    assert_eq!(ui.state().scope, Scope::Section);
    assert_eq!(ui.state().config.section, Some(4));
    assert_eq!(ui.state().section_cursor, 4);
    assert_eq!(ui.state().config.field, 2);
    assert!(ui.state().dialog.is_none());
    key(&mut ui, KeyCode::Char('R'));
    assert!(matches!(&ui.state().dialog.as_ref().unwrap().kind,
        DialogKind::Reply(_, thread) if thread == "scroll-thread-2"));
    key(&mut ui, KeyCode::Esc);
    key(&mut ui, KeyCode::Char('x'));
    assert!(matches!(&ui.state().dialog.as_ref().unwrap().kind,
        DialogKind::Confirm(cronk::ui::Confirmation::Mutate(cronk::model::Mutation::Resolve {
            discussion, resolved: true, ..
        })) if discussion == "scroll-thread-2"));
    key(&mut ui, KeyCode::Esc);
    assert!(!ui.state().mutation_pending);
    key(&mut ui, KeyCode::Down);
    assert_eq!(ui.state().config.field, 3);
    key(&mut ui, KeyCode::Enter);
    assert!(matches!(&ui.state().dialog.as_ref().unwrap().kind,
        DialogKind::Reply(_, thread) if thread == "scroll-thread-3"));
    key(&mut ui, KeyCode::Esc);
    drag_to(&mut ui, area, true);
    assert!(content(&ui, area).contains("CHANGE_089"));
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
