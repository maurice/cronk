use std::time::Duration;

use cronk::{
    config::Config,
    demo,
    model::{ItemKind, Job, TraceChunk},
    ui::{Cronk, Msg, Scope, Trace},
};
use tui_lipan::{
    TestBackend,
    core::event::{MouseButton, MouseKind},
    prelude::*,
};

type Ui = TestBackend<Cronk>;
const JOB: u64 = 900_001;

fn settle(ui: &mut Ui) {
    for _ in 0..4 {
        ui.pump().unwrap();
        ui.render();
    }
}

fn mount(theme: &str) -> Ui {
    let route = demo::items()
        .into_iter()
        .find(|i| i.key.kind == ItemKind::MergeRequest)
        .unwrap()
        .key;
    let config = Config {
        projects: demo::projects(),
        active_tab: 3,
        route: Some(route),
        section: Some(3),
        theme: theme.into(),
        animations: false,
        onboarding: false,
        ..Config::default()
    };
    let mut ui = TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config,
            path: None,
            api: None,
            demo: true,
        },
        (),
        Rect {
            x: 0,
            y: 0,
            w: 100,
            h: 30,
        },
    );
    settle(&mut ui);
    ui.state_mut().details.as_mut().unwrap().jobs = vec![
        Job {
            id: JOB,
            name: "compile".into(),
            stage: "build".into(),
            status: "failed".into(),
            allow_failure: false,
            web_url: String::new(),
        },
        Job {
            id: JOB + 1,
            name: "test".into(),
            stage: "test".into(),
            status: "success".into(),
            allow_failure: false,
            web_url: String::new(),
        },
    ];
    ui.state_mut().traces.clear();
    let mut trace = Trace {
        finished: true,
        ..Trace::default()
    };
    trace.append(
        &(0..100)
            .map(|n| {
                format!(
                    "LINE_{n:03} {}\n",
                    if n == 20 || n == 70 {
                        "Needle λ"
                    } else {
                        "output"
                    }
                )
            })
            .collect::<String>(),
        false,
    );
    trace.offset = trace.text.len() as u64;
    ui.state_mut().traces.insert(JOB, trace);
    ui.state_mut().next_traces = Duration::MAX;
    ui.state_mut().next_details = Duration::MAX;
    ui.state_mut().expanded.clear();
    ui.state_mut().collapsed.clear();
    ui.state_mut().log_views.clear();
    ui.state_mut().config.field = 0;
    ui.state_mut().content_offset = 0;
    ui.dispatch(Msg::Section(3)).unwrap();
    settle(&mut ui);
    let job = ui.rect_of_key(&format!("job-{JOB}").into()).unwrap();
    ui.dispatch(Msg::DetailScrolled(
        (job.y - 6).max(0) as usize + ui.state().content_offset,
    ))
    .unwrap();
    settle(&mut ui);
    ui
}

fn key(ui: &mut Ui, code: KeyCode) {
    modified(ui, code, KeyMods::NONE);
}
fn modified(ui: &mut Ui, code: KeyCode, mods: KeyMods) {
    ui.send_key(KeyEvent { code, mods }).unwrap();
    settle(ui);
}
fn text(ui: &Ui) -> String {
    ui.capture_frame().plain_text()
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
fn append(ui: &mut Ui, value: &str, reset: bool) {
    let route = ui.state().config.route.clone().unwrap();
    let offset = if reset {
        value.len() as u64
    } else {
        ui.state().traces[&JOB].offset + value.len() as u64
    };
    ui.dispatch(Msg::TraceLoaded(
        route,
        ui.state().detail_epoch,
        JOB,
        false,
        Ok(TraceChunk {
            text: value.into(),
            next_offset: offset,
            reset,
        }),
    ))
    .unwrap();
    settle(ui);
}
fn position(ui: &Ui) -> usize {
    ui.state().log_views[&JOB].position(
        ui.state().traces[&JOB].line_count(),
        ui.state().log_height(ui.viewport().h, JOB),
    )
}

#[test]
fn space_toggles_enter_focuses_and_navigation_remains_consistent() {
    let mut ui = mount("midnight");
    key(&mut ui, KeyCode::Char(' '));
    assert!(ui.state().expanded.contains(&JOB));
    assert_eq!(ui.state().log_focus, None);
    assert!(
        text(&ui).contains("LINE_099"),
        "offset={} reveal={} job={:?}\n{}",
        ui.state().content_offset,
        ui.state().reveal_content,
        ui.rect_of_key(&format!("job-{JOB}").into()),
        text(&ui)
    );
    modified(&mut ui, KeyCode::Up, KeyMods::CTRL);
    assert_eq!(position(&ui), 81);
    assert_eq!(ui.state().log_focus, None);
    modified(&mut ui, KeyCode::PageUp, KeyMods::CTRL);
    assert_eq!(position(&ui), 63);
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().log_focus, Some(JOB));
    key(&mut ui, KeyCode::Up);
    assert_eq!(position(&ui), 62);
    assert_eq!(ui.state().config.field, 0);
    key(&mut ui, KeyCode::Home);
    assert_eq!(position(&ui), 0);
    assert!(text(&ui).contains("LINE_000"));
    key(&mut ui, KeyCode::End);
    assert!(ui.state().log_views[&JOB].follow);
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.state().scope, Scope::Section);
    assert_eq!(ui.state().log_focus, None);
    key(&mut ui, KeyCode::Char(' '));
    assert!(ui.state().collapsed.contains(&JOB));
    assert!(!text(&ui).contains("LINE_099"));
    key(&mut ui, KeyCode::Down);
    assert_eq!(ui.state().config.field, 1);
}

#[test]
fn running_jobs_can_be_collapsed_and_refresh_respects_the_choice() {
    let mut ui = mount("midnight");
    ui.state_mut().details.as_mut().unwrap().jobs[0].status = "running".into();
    settle(&mut ui);
    assert!(text(&ui).contains("LINE_099"));
    key(&mut ui, KeyCode::Char(' '));
    assert!(ui.state().collapsed.contains(&JOB));
    assert!(!text(&ui).contains("LINE_099"));
    let details = ui.state().details.as_ref().unwrap().clone();
    ui.dispatch(Msg::DetailsLoaded(
        details.item.key.clone(),
        ui.state().detail_epoch,
        Ok(Box::new(details)),
    ))
    .unwrap();
    settle(&mut ui);
    assert!(ui.state().collapsed.contains(&JOB));
    assert!(ui.state().config.tab_states["3"].collapsed.contains(&JOB));
    assert!(!text(&ui).contains("LIVE ·"));
    key(&mut ui, KeyCode::Char(' '));
    assert!(!ui.state().collapsed.contains(&JOB));
    assert!(text(&ui).contains("LIVE ·"));
}

#[test]
fn whole_job_selection_covers_expanded_logs_in_every_theme() {
    for theme in ["midnight", "dracula", "light"] {
        let mut ui = mount(theme);
        key(&mut ui, KeyCode::Char(' '));
        settle(&mut ui);
        // Header and trace remain stable document children, with one continuous
        // selection edge and background across both.
        let rect = ui.rect_of_key(&format!("job-{JOB}").into()).unwrap();
        assert_eq!(rect.h, 1);
        let window = ui.rect_of_key(&format!("log-window-{JOB}").into()).unwrap();
        let frame = ui.capture_frame();
        let start = window.y.max(0) as u16;
        let end = (start + window.h).min(ui.viewport().h - 2);
        for y in start..end {
            assert_eq!(frame.cell(rect.x as u16, y).symbol, "▕", "{theme} row {y}");
            assert_eq!(
                frame.cell(window.x as u16, y).bg,
                frame.cell(rect.x as u16 + 1, y).bg,
                "{theme} row {y}"
            );
        }
    }
}

#[test]
fn zoom_from_selection_defaults_to_tail_and_preserves_manual_reading_positions() {
    let mut ui = mount("midnight");
    key(&mut ui, KeyCode::Char('z'));
    assert!(ui.state().log_zoom);
    assert_eq!(
        ui.rect_of_key(&format!("log-window-{JOB}").into())
            .unwrap()
            .h,
        ui.viewport().h - 2,
        "zoom reclaims normal application chrome"
    );
    assert!(text(&ui).contains("LINE_099"));
    assert!(!text(&ui).contains("Fields"));
    key(&mut ui, KeyCode::PageUp);
    let before = position(&ui);
    assert!(!ui.state().log_views[&JOB].follow);
    key(&mut ui, KeyCode::Char('z'));
    assert!(!ui.state().log_zoom);
    assert_eq!(ui.state().log_focus, None);
    assert_eq!(position(&ui), before);
    key(&mut ui, KeyCode::Char('z'));
    assert_eq!(position(&ui), before);
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.state().log_focus, None);
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Char('z'));
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.state().log_focus, Some(JOB));
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.state().log_focus, None);
}

#[test]
fn live_append_pins_paused_output_and_end_resumes_following() {
    for zoom in [false, true] {
        let mut ui = mount("midnight");
        key(
            &mut ui,
            if zoom {
                KeyCode::Char('z')
            } else {
                KeyCode::Enter
            },
        );
        ui.state_mut().details.as_mut().unwrap().jobs[0].status = "running".into();
        ui.state_mut().next_traces = Duration::MAX;
        settle(&mut ui);
        key(&mut ui, KeyCode::PageUp);
        let before = position(&ui);
        let marker = format!("LINE_{before:03}");
        assert!(text(&ui).contains(&marker));
        append(&mut ui, "NEW_100\nNEW_101\n", false);
        assert_eq!(position(&ui), before);
        assert!(text(&ui).contains(&marker));
        assert!(!text(&ui).contains("NEW_101"));
        key(&mut ui, KeyCode::End);
        assert!(text(&ui).contains("NEW_101"));
        append(&mut ui, "NEW_102\n", false);
        assert!(text(&ui).contains("NEW_102"));
        assert!(ui.state().log_views[&JOB].follow);
        append(&mut ui, "REPLACED\n", true);
        assert!(text(&ui).contains("REPLACED"));
        assert_eq!(position(&ui), 0);
    }
}

#[test]
fn zoom_scrollbar_click_and_drag_control_real_log_positions() {
    let mut ui = mount("midnight");
    key(&mut ui, KeyCode::Char('z'));
    let rect = ui
        .rect_of_key(&format!("log-scrollbar-{JOB}").into())
        .unwrap();
    let x = rect.x as u16;
    let y = rect.y as u16;
    mouse(&mut ui, x, y, MouseKind::Down(MouseButton::Left));
    mouse(&mut ui, x, y, MouseKind::Up(MouseButton::Left));
    assert_eq!(position(&ui), 0);
    assert!(text(&ui).contains("LINE_000"));
    mouse(&mut ui, x, y, MouseKind::Down(MouseButton::Left));
    mouse(
        &mut ui,
        x,
        y + rect.h - 1,
        MouseKind::Drag(MouseButton::Left),
    );
    mouse(&mut ui, x, y + rect.h - 1, MouseKind::Up(MouseButton::Left));
    assert!(ui.state().log_views[&JOB].follow);
    assert!(text(&ui).contains("LINE_099"));
    assert!(ui.state().log_zoom);
    assert!(ui.state().expanded.contains(&JOB));
}

#[test]
fn search_jumps_between_matches_and_escape_unwinds_one_level() {
    let mut ui = mount("midnight");
    key(&mut ui, KeyCode::Char('z'));
    key(&mut ui, KeyCode::Home);
    key(&mut ui, KeyCode::Char('/'));
    assert_eq!(ui.focused_key(), Some(&"log-search".into()));
    ui.send_paste("needle λ").unwrap();
    key(&mut ui, KeyCode::Enter);
    assert_eq!(position(&ui), 20);
    assert!(text(&ui).contains("Needle λ"));
    assert!(!ui.state().log_views[&JOB].follow);
    key(&mut ui, KeyCode::Char('n'));
    assert_eq!(position(&ui), 70);
    key(&mut ui, KeyCode::Char('n'));
    assert_eq!(position(&ui), 20);
    key(&mut ui, KeyCode::Char('N'));
    assert_eq!(position(&ui), 70);
    key(&mut ui, KeyCode::Char('/'));
    key(&mut ui, KeyCode::Esc);
    assert!(ui.state().log_search.is_none());
    assert!(ui.state().log_zoom);
    key(&mut ui, KeyCode::Esc);
    assert!(!ui.state().log_zoom);
    assert_eq!(ui.state().scope, Scope::Section);
}

#[test]
fn wheel_scrolls_focused_logs_without_moving_the_detail_document() {
    let mut ui = mount("midnight");
    key(&mut ui, KeyCode::Char('z'));
    let rect = ui.rect_of_key(&format!("log-window-{JOB}").into()).unwrap();
    let before = position(&ui);
    mouse(
        &mut ui,
        rect.x as u16 + 1,
        rect.y as u16 + 1,
        MouseKind::ScrollUp,
    );
    assert_eq!(position(&ui), before - 3);
    assert!(text(&ui).contains(&format!("LINE_{:03}", before - 3)));
    mouse(
        &mut ui,
        rect.x as u16 + 1,
        rect.y as u16 + 1,
        MouseKind::ScrollDown,
    );
    assert!(ui.state().log_views[&JOB].follow);
    assert!(text(&ui).contains("LINE_099"));
}

#[test]
fn complete_large_history_is_scrollable_searchable_and_not_limited_by_terminal_coordinates() {
    let mut ui = mount("midnight");
    key(&mut ui, KeyCode::Char('z'));
    let history: String = (0..70_000)
        .map(|n| format!("HISTORY_{n:06} λ output\n"))
        .collect();
    append(&mut ui, &history, true);
    assert!(ui.state().traces[&JOB].text.len() > 128 * 1024);
    assert_eq!(ui.state().traces[&JOB].line_count(), 70_000);
    assert!(text(&ui).contains("HISTORY_069999"));
    key(&mut ui, KeyCode::Home);
    assert!(text(&ui).contains("HISTORY_000000"));
    key(&mut ui, KeyCode::Char('/'));
    ui.send_paste("HISTORY_012345").unwrap();
    key(&mut ui, KeyCode::Enter);
    assert_eq!(position(&ui), 12_345);
    assert!(text(&ui).contains("HISTORY_012345"));
    let rect = ui
        .rect_of_key(&format!("log-scrollbar-{JOB}").into())
        .unwrap();
    let x = rect.x as u16;
    let frame = ui.capture_frame();
    let thumb_y = (rect.y as u16..rect.y as u16 + rect.h)
        .find(|&y| frame.cell(x, y).symbol == "█")
        .unwrap();
    mouse(&mut ui, x, thumb_y, MouseKind::Down(MouseButton::Left));
    assert_eq!(
        position(&ui),
        12_345,
        "pressing a quantized thumb must not move the source position"
    );
    mouse(&mut ui, x, thumb_y, MouseKind::Up(MouseButton::Left));
    key(&mut ui, KeyCode::End);
    assert!(text(&ui).contains("HISTORY_069999"));
    assert!(
        text(&ui).lines().last().unwrap().contains("Search"),
        "the log status must stay visible after searching"
    );
}

#[test]
fn collapsed_choices_round_trip_and_tab_visits_restore_log_positions() {
    let mut ui = mount("midnight");
    ui.state_mut().details.as_mut().unwrap().jobs[0].status = "running".into();
    key(&mut ui, KeyCode::Char(' '));
    assert!(ui.state().collapsed.contains(&JOB));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("workspace.toml");
    ui.state().config.save(&path).unwrap();
    let persisted_ui = TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config: ui.state().config.clone(),
            path: Some(path.clone()),
            api: None,
            demo: true,
        },
        (),
        ui.viewport(),
    );
    persisted_ui.state().flush_navigation().unwrap();
    drop(persisted_ui);
    let config = Config::load(&path).unwrap();
    assert!(
        config.tab_states.is_empty(),
        "job choices belong in local SQLite, not TOML"
    );
    let mut restarted = TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config,
            path: Some(path.clone()),
            api: None,
            demo: true,
        },
        (),
        ui.viewport(),
    );
    settle(&mut restarted);
    assert!(restarted.state().collapsed.contains(&JOB));
    // Restore a completed fixture so offline polling cannot replace the source.
    ui.state_mut().details.as_mut().unwrap().jobs[0].status = "failed".into();
    let trace = ui.state_mut().traces.get_mut(&JOB).unwrap();
    trace.append(&"restored output\n".repeat(100), true);
    trace.finished = true;
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Home);
    key(&mut ui, KeyCode::PageDown);
    let before = position(&ui);
    ui.dispatch(Msg::Tab(2)).unwrap();
    settle(&mut ui);
    assert!(ui.state().log_focus.is_none());
    assert!(!ui.state().log_zoom);
    ui.dispatch(Msg::Tab(3)).unwrap();
    settle(&mut ui);
    assert_eq!(position(&ui), before);
    assert!(!ui.state().log_views[&JOB].follow);
    assert!(ui.state().log_focus.is_none());
}

#[test]
fn refresh_reorders_focused_jobs_by_identity_and_removal_closes_zoom() {
    let mut ui = mount("midnight");
    key(&mut ui, KeyCode::Char('z'));
    key(&mut ui, KeyCode::Home);
    key(&mut ui, KeyCode::PageDown);
    let before = position(&ui);
    let mut details = ui.state().details.as_ref().unwrap().clone();
    details.jobs.reverse();
    ui.dispatch(Msg::DetailsLoaded(
        details.item.key.clone(),
        ui.state().detail_epoch,
        Ok(Box::new(details)),
    ))
    .unwrap();
    settle(&mut ui);
    assert_eq!(ui.state().config.field, 1);
    assert_eq!(ui.state().log_focus, Some(JOB));
    assert!(ui.state().log_zoom);
    assert_eq!(position(&ui), before);
    let mut details = ui.state().details.as_ref().unwrap().clone();
    details.jobs.retain(|job| job.id != JOB);
    ui.dispatch(Msg::DetailsLoaded(
        details.item.key.clone(),
        ui.state().detail_epoch,
        Ok(Box::new(details)),
    ))
    .unwrap();
    settle(&mut ui);
    assert!(ui.state().log_focus.is_none());
    assert!(!ui.state().log_zoom);
}

#[test]
fn search_with_no_matches_keeps_position_and_cancelled_drafts_do_not_replace_query() {
    let mut ui = mount("midnight");
    key(&mut ui, KeyCode::Char('z'));
    key(&mut ui, KeyCode::PageUp);
    let before = position(&ui);
    key(&mut ui, KeyCode::Char('/'));
    ui.send_paste("absent.*").unwrap();
    key(&mut ui, KeyCode::Enter);
    assert_eq!(position(&ui), before);
    assert!(text(&ui).contains("no matches"));
    key(&mut ui, KeyCode::Char('/'));
    ui.send_paste("changed draft").unwrap();
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.state().log_views[&JOB].query, "absent.*");
    assert!(ui.state().log_zoom);
    assert!(ui.state().dialog.is_none());
    assert_eq!(position(&ui), before);
}

#[test]
fn resizing_near_eof_does_not_unanchor_paused_logs_on_the_next_append() {
    let mut ui = mount("midnight");
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Up);
    assert!(!ui.state().log_views[&JOB].follow);
    key(&mut ui, KeyCode::Char('z'));
    let before = position(&ui);
    append(&mut ui, &"NEW_OUTPUT\n".repeat(10), false);
    assert_eq!(position(&ui), before);
    ui.set_viewport(Rect {
        x: 0,
        y: 0,
        w: 60,
        h: 45,
    });
    settle(&mut ui);
    let before = position(&ui);
    append(&mut ui, &"MORE_OUTPUT\n".repeat(20), false);
    assert_eq!(position(&ui), before);
    assert!(!ui.state().log_views[&JOB].follow);
    key(&mut ui, KeyCode::End);
    ui.set_viewport(Rect {
        x: 0,
        y: 0,
        w: 40,
        h: 18,
    });
    settle(&mut ui);
    append(&mut ui, "LATEST_OUTPUT\n", false);
    assert!(text(&ui).contains("LATEST_OUTPUT"));
    assert!(ui.state().log_views[&JOB].follow);
}

#[test]
fn ansi_colors_and_yellow_substring_highlights_render_inline_and_fullscreen() {
    for theme in ["midnight", "dracula", "light"] {
        for zoom in [false, true] {
            let mut ui = mount(theme);
            key(
                &mut ui,
                if zoom {
                    KeyCode::Char('z')
                } else {
                    KeyCode::Enter
                },
            );
            let body = format!(
                "\x1b[1;38;2;12;34;56;48;5;235mFIRST\n{}",
                "rgb nee\x1b[32mdle plain NEEDLE tail\x1b[38;2;12;34;56m\n".repeat(100)
            );
            append(&mut ui, &body, true);
            key(&mut ui, KeyCode::Home);
            key(&mut ui, KeyCode::PageDown);
            key(&mut ui, KeyCode::Char('/'));
            ui.send_paste("needle").unwrap();
            key(&mut ui, KeyCode::Enter);
            let rect = ui.rect_of_key(&format!("log-window-{JOB}").into()).unwrap();
            let frame = ui.capture_frame();
            let phrase = "rgb needle plain NEEDLE tail";
            let location = (rect.y.max(0) as u16
                ..(rect.y.max(0) as u16 + rect.h).min(ui.viewport().h))
                .find_map(|y| {
                    let row: String = (rect.x as u16..rect.x as u16 + rect.w)
                        .map(|x| frame.cell(x, y).symbol.as_str())
                        .collect();
                    row.find(phrase)
                        .map(|column| (rect.x as u16 + column as u16, y))
                })
                .expect("colored log row should be visible after searching");
            let (x, y) = location;
            assert_eq!(
                frame.cell(x, y).fg,
                Color::Rgb(12, 34, 56),
                "{theme}, zoom={zoom}: inherited truecolor"
            );
            assert_eq!(frame.cell(x, y).bg, Color::Indexed(235));
            assert_eq!(frame.cell(x + 11, y).fg, Color::Green);
            assert_eq!(frame.cell(x + 11, y).bg, Color::Indexed(235));
            for offset in (4..10).chain(17..23) {
                assert_eq!(
                    frame.cell(x + offset, y).bg,
                    Color::Rgb(255, 215, 0),
                    "{theme}, zoom={zoom}: current match column {offset}"
                );
                assert_eq!(frame.cell(x + offset, y).fg, Color::Black);
            }
            assert_eq!(
                frame.cell(x + 4, y + 1).bg,
                Color::Rgb(225, 211, 150),
                "other matching lines use a softer yellow"
            );
            assert!(
                !text(&ui).contains("▶"),
                "matching text is highlighted without altering log content"
            );
            key(&mut ui, KeyCode::Char('n'));
            key(&mut ui, KeyCode::Up);
            let frame = ui.capture_frame();
            assert_eq!(
                frame.cell(x + 4, y).bg,
                Color::Rgb(225, 211, 150),
                "the previous match becomes soft yellow"
            );
            assert_eq!(
                frame.cell(x + 4, y + 1).bg,
                Color::Rgb(255, 215, 0),
                "the next match becomes strong yellow even after manual scrolling"
            );
            key(&mut ui, KeyCode::Char('N'));
            let frame = ui.capture_frame();
            assert_eq!(
                frame.cell(x + 4, y).bg,
                Color::Rgb(255, 215, 0),
                "previous-match navigation restores the strong highlight"
            );
            assert_eq!(frame.cell(x + 4, y + 1).bg, Color::Rgb(225, 211, 150));
            key(&mut ui, KeyCode::Char('/'));
            key(&mut ui, KeyCode::Home);
            modified(&mut ui, KeyCode::End, KeyMods::SHIFT);
            key(&mut ui, KeyCode::Backspace);
            key(&mut ui, KeyCode::Enter);
            assert!(ui.state().log_views[&JOB].query.is_empty());
            let frame = ui.capture_frame();
            assert_eq!(
                frame.cell(x + 4, y).bg,
                Color::Indexed(235),
                "clearing search restores the original background"
            );
            assert_eq!(frame.cell(x + 4, y).fg, Color::Rgb(12, 34, 56));
        }
    }
}

#[test]
fn empty_and_error_traces_keep_fullscreen_status_visible_and_hide_the_scrollbar() {
    for theme in ["midnight", "dracula", "light"] {
        let mut ui = mount(theme);
        key(&mut ui, KeyCode::Char('z'));
        append(&mut ui, "", true);
        assert!(
            ui.rect_of_key(&format!("log-scrollbar-{JOB}").into())
                .is_none()
        );
        assert!(text(&ui).contains("Waiting for log output"));
        let route = ui.state().config.route.clone().unwrap();
        ui.dispatch(Msg::TraceLoaded(
            route,
            ui.state().detail_epoch,
            JOB,
            false,
            Err("cannot read job log".into()),
        ))
        .unwrap();
        ui.set_viewport(Rect {
            x: 0,
            y: 0,
            w: 60,
            h: 18,
        });
        settle(&mut ui);
        assert!(text(&ui).contains("Trace error: cannot read job log"));
        assert!(text(&ui).lines().last().unwrap().contains("Loading trace"));
        assert!(ui.state().log_zoom);
    }
}
