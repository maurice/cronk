//! Project pipelines: visibility gating, window paging, drill-in and the list indicator.
use cronk::{
    config::Config,
    demo,
    gitlab::PIPELINE_PAGE,
    model::{ItemKind, Project},
    ui::{Cronk, Msg, Scope},
};
use tui_lipan::{
    TestBackend,
    core::event::{MouseButton, MouseKind},
    prelude::*,
};

type Ui = TestBackend<Cronk>;

const PROJECT: u64 = 9001;

fn mount_projects(projects: Vec<Project>, height: u16) -> Ui {
    let mut ui = TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config: Config {
                projects,
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

fn mount(height: u16) -> Ui {
    mount_projects(demo::projects(), height)
}

fn settle(ui: &mut Ui) {
    for _ in 0..3 {
        ui.render();
        ui.pump().unwrap();
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

fn open_pipelines(ui: &mut Ui) {
    key(ui, KeyCode::Enter);
    assert_eq!(ui.state().scope, Scope::Details);
    key(ui, KeyCode::Down);
    assert_eq!(ui.state().section_name(), "Pipelines");
}

fn text(ui: &mut Ui) -> String {
    ui.render();
    ui.capture_frame().plain_text()
}

fn click(ui: &mut Ui, key: &str) {
    settle(ui);
    let rect = ui.rect_of_key(&key.to_owned().into()).unwrap();
    for kind in [
        MouseKind::Down(MouseButton::Left),
        MouseKind::Up(MouseButton::Left),
    ] {
        ui.send_mouse(MouseEvent {
            kind,
            x: rect.x as u16 + 6,
            y: rect.y as u16,
            mods: KeyMods::NONE,
        })
        .unwrap();
    }
    settle(ui);
}

#[test]
fn pipelines_follow_merge_request_visibility() {
    let mut hidden = demo::projects();
    hidden[0].merge_requests_visible = false;
    let mut ui = mount_projects(hidden, 40);
    key(&mut ui, KeyCode::Enter);
    assert_eq!(
        ui.state().sections(),
        &["Fields", "Forget this project"],
        "no Pipelines section without merge request sync"
    );
    assert!(
        !ui.state().project_pipelines.contains_key(&PROJECT),
        "hidden projects make no pipeline requests"
    );
    let shown = text(&mut ui);
    assert!(shown.contains("pipelines not synced"), "{shown}");
    assert!(!shown.contains("Pipelines ─"), "{shown}");

    // Toggling merge requests on from Fields adds the section and starts syncing.
    key(&mut ui, KeyCode::Enter);
    ui.dispatch(Msg::Move(3)).unwrap();
    key(&mut ui, KeyCode::Char(' '));
    assert_eq!(ui.state().sections().len(), 3);
    assert!(ui.state().project_pipelines.contains_key(&PROJECT));
    assert!(text(&mut ui).contains("also syncs pipelines"));

    // Toggling it off again while on the Pipelines heading re-clamps the cursor.
    key(&mut ui, KeyCode::Esc);
    key(&mut ui, KeyCode::Down);
    assert_eq!(ui.state().section_name(), "Pipelines");
    ui.dispatch(Msg::ToggleProjectKind(ItemKind::MergeRequest))
        .unwrap();
    assert_eq!(ui.state().sections().len(), 2);
    assert_eq!(ui.state().section_cursor, 1);
    assert!(ui.state().drilled.is_none());
}

#[test]
fn newest_pipeline_auto_expands_and_rows_show_source_ref_and_time() {
    let mut ui = mount(40);
    key(&mut ui, KeyCode::Enter);
    let window = ui.state().project_pipelines[&PROJECT].clone();
    assert!(window.loaded);
    assert_eq!(window.pipelines.len(), PIPELINE_PAGE);
    assert_eq!(window.pages_loaded, 1);
    assert!(window.can_load_more());
    let newest = window.pipelines[0].id;
    assert!(ui.state().pipeline_expanded(PROJECT, newest));
    assert!(
        !ui.state()
            .pipeline_expanded(PROJECT, window.pipelines[1].id)
    );
    assert_eq!(
        window.jobs.keys().copied().collect::<Vec<_>>(),
        vec![newest],
        "only the expanded pipeline fetches detail and jobs"
    );
    assert_eq!(
        window.pipelines[0]
            .user
            .as_ref()
            .map(|u| u.username.as_str()),
        Some("demo-ci-bot"),
        "detail fetch fills in who started it"
    );
    assert!(
        window.pipelines[1].user.is_none(),
        "collapsed rows use list fields only"
    );
    key(&mut ui, KeyCode::Down);
    let shown = text(&mut ui);
    assert!(
        shown.contains("▼ #60999  running  main  schedule   2m 52s · just now   nightly"),
        "{shown}"
    );
    assert!(shown.contains("by @demo-ci-bot"), "{shown}");
    assert!(shown.contains("2 running"), "{shown}");
    assert!(shown.contains("8 total"), "{shown}");
    assert!(
        shown.contains("▶ #50000  failed  !101  merge request   16 min ago"),
        "{shown}"
    );
    assert!(
        !shown.contains("integration:linux"),
        "jobs appear only when drilled in: {shown}"
    );
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::End);
    assert!(text(&mut ui).contains("Show 20 older pipelines…   (20 loaded)"));
}

#[test]
fn keyboard_navigation_drills_into_jobs_and_escapes_back_out() {
    let mut ui = mount(40);
    open_pipelines(&mut ui);
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().scope, Scope::Section);
    assert_eq!(ui.state().config.field, 0);
    assert!(ui.state().drilled.is_none());
    let window = ui.state().project_pipelines[&PROJECT].clone();
    let newest = window.pipelines[0].id;
    let second = window.pipelines[1].id;

    // Tab moves between pipelines; Space toggles the one under the cursor.
    key(&mut ui, KeyCode::Tab);
    assert_eq!(ui.state().config.field, 1);
    key(&mut ui, KeyCode::Char(' '));
    assert!(ui.state().pipeline_expanded(PROJECT, second));
    assert!(
        ui.state().project_pipelines[&PROJECT]
            .jobs
            .contains_key(&second)
    );
    key(&mut ui, KeyCode::Char(' '));
    assert!(!ui.state().pipeline_expanded(PROJECT, second));
    key(&mut ui, KeyCode::BackTab);
    key(&mut ui, KeyCode::Char(' '));
    assert!(
        !ui.state().pipeline_expanded(PROJECT, newest),
        "an explicit collapse beats the newest-is-open default"
    );
    key(&mut ui, KeyCode::Char(' '));

    // Enter drills into the pipeline: the cursor is now on its jobs.
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().drilled, Some(newest));
    assert_eq!(ui.state().config.field, 0);
    let job = ui
        .state()
        .selected_job()
        .expect("first job selected")
        .clone();
    assert!(job.running(), "most recently started job first");
    assert!(
        ui.state().traces.contains_key(&job.id),
        "running jobs stream logs"
    );
    let shown = text(&mut ui);
    assert!(shown.contains("integration:"), "{shown}");
    assert!(shown.contains("LIVE"), "{shown}");
    key(&mut ui, KeyCode::Down);
    assert_eq!(ui.state().config.field, 1);
    key(&mut ui, KeyCode::End);
    assert_eq!(
        ui.state().config.field,
        7,
        "cursor stays inside this pipeline's jobs"
    );
    key(&mut ui, KeyCode::Home);

    // Space collapses a job; Enter focuses its logs; z zooms; Esc unwinds one level at a time.
    key(&mut ui, KeyCode::Char(' '));
    assert!(ui.state().collapsed.contains(&job.id));
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().log_focus, Some(job.id));
    key(&mut ui, KeyCode::Char('z'));
    assert!(ui.state().log_zoom);
    key(&mut ui, KeyCode::Esc);
    assert!(!ui.state().log_zoom);
    assert_eq!(ui.state().log_focus, Some(job.id));
    key(&mut ui, KeyCode::Esc);
    assert!(ui.state().log_focus.is_none());
    assert_eq!(ui.state().drilled, Some(newest));
    key(&mut ui, KeyCode::Esc);
    assert!(
        ui.state().drilled.is_none(),
        "Esc leaves the jobs for the pipeline row"
    );
    assert_eq!(ui.state().config.field, 0);
    assert_eq!(ui.state().scope, Scope::Section);
    assert!(
        ui.state().pipeline_expanded(PROJECT, newest),
        "the pipeline stays expanded"
    );
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.state().scope, Scope::Details);
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.state().scope, Scope::List);
}

#[test]
fn older_pipelines_load_from_the_footer_until_exhausted_or_capped() {
    let mut ui = mount(40);
    open_pipelines(&mut ui);
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::End);
    assert_eq!(
        ui.state().config.field,
        PIPELINE_PAGE,
        "End lands on the footer"
    );
    key(&mut ui, KeyCode::Down);
    assert_eq!(
        ui.state().config.field,
        PIPELINE_PAGE,
        "nothing below the footer"
    );
    key(&mut ui, KeyCode::Enter);
    let window = ui.state().project_pipelines[&PROJECT].clone();
    assert_eq!(window.pages_loaded, 2);
    assert_eq!(window.pipelines.len(), 2 * PIPELINE_PAGE);
    assert_eq!(
        ui.state().config.field,
        2 * PIPELINE_PAGE,
        "cursor stays on the footer"
    );
    assert!(
        window
            .pipelines
            .windows(2)
            .all(|w| w[0].created_at >= w[1].created_at),
        "newest first"
    );
    key(&mut ui, KeyCode::Enter);
    let window = ui.state().project_pipelines[&PROJECT].clone();
    assert_eq!(window.pages_loaded, 3);
    assert_eq!(
        window.pipelines.len(),
        49,
        "demo project 9001: 45 synthetic + 4 MR heads"
    );
    assert!(window.exhausted);
    assert!(!window.can_load_more());
    assert_eq!(window.row_count(), 49);
    key(&mut ui, KeyCode::End);
    assert_eq!(ui.state().config.field, 48, "no footer once exhausted");
    assert!(!text(&mut ui).contains("older pipelines"));

    // Refresh merges page 1 without discarding older pages or the selected row.
    key(&mut ui, KeyCode::Up);
    let selected = ui.state().project_pipelines[&PROJECT].pipelines[47].id;
    ui.dispatch(Msg::LoadPipelines).unwrap();
    settle(&mut ui);
    let window = ui.state().project_pipelines[&PROJECT].clone();
    assert_eq!(window.pipelines.len(), 49);
    assert_eq!(window.pipelines[ui.state().config.field].id, selected);

    // Reopening the project drops the older pages but keeps page 1 as a snapshot.
    key(&mut ui, KeyCode::Esc);
    key(&mut ui, KeyCode::Esc);
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.state().scope, Scope::List);
    key(&mut ui, KeyCode::Enter);
    let window = ui.state().project_pipelines[&PROJECT].clone();
    assert_eq!(window.pipelines.len(), PIPELINE_PAGE);
    assert!(window.can_load_more());
}

#[test]
fn mouse_clicks_select_toggle_and_drill_pipelines() {
    let mut ui = mount(40);
    key(&mut ui, KeyCode::Enter);
    let window = ui.state().project_pipelines[&PROJECT].clone();
    let newest = window.pipelines[0].id;
    let second = window.pipelines[1].id;
    click(&mut ui, &format!("pipeline-{second}"));
    assert_eq!(ui.state().scope, Scope::Section);
    assert_eq!(ui.state().section_name(), "Pipelines");
    assert_eq!(ui.state().config.field, 1);
    assert!(ui.state().pipeline_expanded(PROJECT, second));
    assert!(ui.state().drilled.is_none());
    key(&mut ui, KeyCode::Enter);
    assert_eq!(ui.state().drilled, Some(second));
    let job = ui.state().selected_job().unwrap().clone();
    // Clicking a job inside the drilled pipeline toggles it and keeps the drill.
    click(&mut ui, &format!("job-{}", job.id));
    assert_eq!(ui.state().drilled, Some(second));
    assert!(ui.state().expanded.contains(&job.id) || ui.state().collapsed.contains(&job.id));
    // Clicking another pipeline row leaves the drilled jobs.
    click(&mut ui, &format!("pipeline-{newest}"));
    assert!(ui.state().drilled.is_none());
    assert_eq!(ui.state().config.field, 0);
    assert!(
        !ui.state().pipeline_expanded(PROJECT, newest),
        "click toggled it closed"
    );
    key(&mut ui, KeyCode::End);
    click(&mut ui, "pipelines-older");
    assert_eq!(ui.state().project_pipelines[&PROJECT].pages_loaded, 2);
}

#[test]
fn project_rows_show_the_latest_pipeline_from_the_probe() {
    let mut projects = demo::projects();
    projects[1].merge_requests_visible = false;
    projects[2].visible = false;
    let mut ui = mount_projects(projects, 40);
    assert!(ui.state().latest_pipeline.contains_key(&PROJECT));
    assert!(
        !ui.state().latest_pipeline.contains_key(&9002)
            && !ui.state().latest_pipeline.contains_key(&9003),
        "only MR-visible projects are probed"
    );
    let shown = text(&mut ui);
    assert!(
        shown.contains(
            "3 open issues · 3 open merge requests  ·  ◐ pipeline running · main · just now"
        ),
        "{shown}"
    );
    let lines: Vec<_> = shown.lines().filter(|l| l.contains("pipeline ")).collect();
    assert_eq!(lines.len(), 3, "{shown}");
    assert!(ui.rect_of_key(&"project-pipeline-9001".into()).is_some());
    assert!(ui.rect_of_key(&"project-pipeline-9002".into()).is_none());

    // The probe skips the open project: its own page 1 keeps the indicator current.
    ui.state_mut().probe_pending.clear();
    ui.state_mut().latest_pipeline.clear();
    key(&mut ui, KeyCode::Enter);
    ui.dispatch(Msg::LoadProbes).unwrap();
    settle(&mut ui);
    assert!(ui.state().latest_pipeline.contains_key(&9004));
    assert!(
        ui.state().latest_pipeline.contains_key(&PROJECT),
        "page 1 fills it in"
    );
    assert!(!ui.state().probe_pending.contains(&PROJECT));
}

#[test]
fn pipeline_refresh_cadence_defaults_to_twice_the_list_cadence() {
    let mut config = Config::default();
    assert_eq!(config.pipeline_refresh_secs(), 120);
    config.pipeline_refresh_secs = Some(10);
    assert!(config.validate().is_err());
    config.pipeline_refresh_secs = Some(30);
    assert_eq!(config.pipeline_refresh_secs(), 30);
    config.validate().unwrap();
    let parsed: Config = toml::from_str("onboarding = false\npipeline_refresh_secs = 90").unwrap();
    assert_eq!(parsed.pipeline_refresh_secs(), 90);
}
