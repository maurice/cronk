//! Project pipelines: visibility gating, window paging, drill-in and the list indicator.
use cronk::{
    config::Config,
    demo,
    gitlab::{PIPELINE_PAGE, Pipelines},
    model::{ItemKind, Job, Pipeline, Project},
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
fn project_pipeline_running_job_first_unfocused_click_keeps_logs_open() {
    let mut ui = mount(60);
    open_pipelines(&mut ui);
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Enter);
    let pipeline = ui.state().drilled.unwrap();
    let job = ui.state().selected_job().unwrap().clone();
    assert!(job.running());
    assert!(ui.state().job_expanded(&job));
    ui.state_mut().scope = Scope::Details;
    settle(&mut ui);

    click(&mut ui, &format!("job-{}", job.id));
    assert!(ui.state().in_jobs());
    assert_eq!(ui.state().selected_job().unwrap().id, job.id);
    assert_eq!(ui.state().drilled, Some(pipeline));
    assert!(ui.state().job_expanded(&job));
    assert!(!ui.state().collapsed.contains(&job.id));
    assert!(
        ui.rect_of_key(&format!("trace-{}", job.id).into())
            .is_some()
    );

    click(&mut ui, &format!("job-{}", job.id));
    assert!(ui.state().collapsed.contains(&job.id));
    assert!(!ui.state().job_expanded(&job));
    ui.state_mut().scope = Scope::Details;
    settle(&mut ui);
    click(&mut ui, &format!("job-{}", job.id));
    assert!(ui.state().job_expanded(&job));
    key(&mut ui, KeyCode::Char(' '));
    assert!(ui.state().collapsed.contains(&job.id));
}

#[test]
fn project_pipeline_failures_pin_in_run_order_and_unfocused_click_keeps_logs_open() {
    let mut ui = mount(60);
    open_pipelines(&mut ui);
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Enter);
    let pipeline = ui.state().drilled.unwrap();
    let selected = ui.state().selected_job().unwrap().id;
    let window = &ui.state().project_pipelines[&PROJECT];
    let mut jobs = window.jobs[&pipeline].jobs[..3].to_vec();
    let first_failure = jobs[1].id;
    let second_failure = jobs[2].id;
    jobs[0].status = "running".into();
    jobs[0].started_at = Some("2025-01-01T10:10:00Z".into());
    for (job, started) in jobs[1..]
        .iter_mut()
        .zip(["2025-01-01T10:00:00Z", "2025-01-01T10:05:00Z"])
    {
        job.status = "failed".into();
        job.allow_failure = false;
        job.started_at = Some(started.into());
    }
    cronk::model::sort_jobs(&mut jobs);
    ui.dispatch(Msg::PipelineLoaded(
        PROJECT,
        pipeline,
        ui.state().pipeline_epoch,
        Ok(Box::new((window.pipelines[0].clone(), jobs, Vec::new()))),
    ))
    .unwrap();
    settle(&mut ui);
    let jobs = &ui.state().project_pipelines[&PROJECT].jobs[&pipeline].jobs;
    assert_eq!(
        jobs.iter().map(|j| j.id).collect::<Vec<_>>(),
        [first_failure, second_failure, selected]
    );
    assert_eq!(ui.state().selected_job().unwrap().id, selected);
    assert!(ui.state().job_expanded(&jobs[0]));
    assert!(ui.state().job_expanded(&jobs[1]));

    ui.dispatch(Msg::Select(0)).unwrap();
    settle(&mut ui);
    ui.state_mut().scope = Scope::Details;
    settle(&mut ui);
    click(&mut ui, &format!("job-{first_failure}"));
    assert_eq!(ui.state().selected_job().unwrap().id, first_failure);
    assert!(!ui.state().collapsed.contains(&first_failure));
    assert_eq!(ui.state().drilled, Some(pipeline));
    click(&mut ui, &format!("job-{first_failure}"));
    assert!(ui.state().collapsed.contains(&first_failure));
}

#[test]
fn project_pipeline_refresh_groups_active_upcoming_completed_and_manual_jobs() {
    let mut ui = mount(60);
    open_pipelines(&mut ui);
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Enter);
    let pipeline = ui.state().drilled.unwrap();
    let selected = ui.state().selected_job().unwrap().id;
    let window = &ui.state().project_pipelines[&PROJECT];
    let mut jobs = window.jobs[&pipeline].jobs[..5].to_vec();
    let ids: Vec<_> = jobs.iter().map(|j| j.id).collect();
    for (job, status) in jobs
        .iter_mut()
        .zip(["running", "pending", "success", "failed", "manual"])
    {
        job.status = status.into();
        job.allow_failure = true;
    }
    // Even a more recent completed job must not split active/upcoming work.
    jobs[2].started_at = Some("2025-01-01T12:00:00Z".into());
    jobs[3].started_at = Some("2025-01-01T11:00:00Z".into());
    cronk::model::sort_jobs(&mut jobs);
    let full = window.pipelines[0].clone();
    ui.dispatch(Msg::PipelineLoaded(
        PROJECT,
        pipeline,
        ui.state().pipeline_epoch,
        Ok(Box::new((full.clone(), jobs, Vec::new()))),
    ))
    .unwrap();
    settle(&mut ui);
    let mut jobs = ui.state().project_pipelines[&PROJECT].jobs[&pipeline]
        .jobs
        .clone();
    assert_eq!(jobs.iter().map(|j| j.id).collect::<Vec<_>>(), ids);
    assert_eq!(ui.state().selected_job().unwrap().id, selected);
    jobs[0].status = "success".into();
    jobs[0].started_at = Some("2025-01-01T13:00:00Z".into());
    cronk::model::sort_jobs(&mut jobs);
    ui.dispatch(Msg::PipelineLoaded(
        PROJECT,
        pipeline,
        ui.state().pipeline_epoch,
        Ok(Box::new((full, jobs, Vec::new()))),
    ))
    .unwrap();
    settle(&mut ui);
    let jobs = &ui.state().project_pipelines[&PROJECT].jobs[&pipeline].jobs;
    assert_eq!(
        jobs.iter().map(|j| j.id).collect::<Vec<_>>(),
        [ids[1], ids[0], ids[2], ids[3], ids[4]]
    );
    assert_eq!(ui.state().selected_job().unwrap().id, selected);
    assert_eq!(ui.state().config.field, 1);
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
    assert_eq!(
        ui.state().section_cursor,
        0,
        "never slide onto Forget this project"
    );
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

#[test]
fn a_pipeline_that_finishes_is_fetched_again_and_its_jobs_stop_streaming() {
    let mut ui = mount(40);
    open_pipelines(&mut ui);
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Enter);
    let newest = ui.state().drilled.unwrap();
    let running = ui.state().selected_job().unwrap().clone();
    assert!(running.running());
    assert!(
        ui.state().wanted_pipelines(PROJECT).contains(&newest),
        "active pipelines re-fetch"
    );
    let mut finished = ui.state().project_pipelines[&PROJECT]
        .get(newest)
        .unwrap()
        .clone();
    finished.status = "success".into();
    finished.updated_at = Some("2026-09-28T11:59:59Z".into());
    let epoch = ui.state().pipeline_epoch;
    // Page 1 now reports the pipeline as finished with list-only fields.
    ui.dispatch(Msg::PipelinesLoaded(
        PROJECT,
        epoch,
        1,
        Ok(Pipelines::Available(vec![Pipeline {
            user: None,
            duration: None,
            started_at: None,
            ..finished.clone()
        }])),
    ))
    .unwrap();
    settle(&mut ui);
    let window = ui.state().project_pipelines[&PROJECT].clone();
    assert_eq!(
        window.pipelines.len(),
        1,
        "page 1 is authoritative for the window head"
    );
    assert!(
        window.get(newest).unwrap().user.is_some(),
        "who started it is retained"
    );
    assert!(
        !ui.state().wanted_pipelines(PROJECT).contains(&newest)
            || window.jobs[&newest].stamp != finished.updated_at,
        "the stale job list is marked for re-fetch"
    );
    let jobs: Vec<Job> = window.jobs[&newest]
        .jobs
        .iter()
        .map(|job| Job {
            status: "success".into(),
            ..job.clone()
        })
        .collect();
    ui.dispatch(Msg::PipelineLoaded(
        PROJECT,
        newest,
        epoch,
        Ok(Box::new((finished.clone(), jobs, Vec::new()))),
    ))
    .unwrap();
    settle(&mut ui);
    assert_eq!(ui.state().drilled, Some(newest));
    let job = ui.state().selected_job().unwrap().clone();
    assert_eq!(job.id, running.id, "cursor keeps its job identity");
    assert!(!job.running());
    assert!(
        !ui.state().trace_wanted(&job)
            || ui.state().traces.get(&job.id).is_none_or(|t| !t.finished)
    );
    assert!(
        ui.state().wanted_pipelines(PROJECT).is_empty(),
        "finished and current: nothing to fetch"
    );
    // A pipeline that vanishes from the window drops the drill.
    ui.dispatch(Msg::PipelinesLoaded(
        PROJECT,
        epoch,
        1,
        Ok(Pipelines::Available(Vec::new())),
    ))
    .unwrap();
    settle(&mut ui);
    assert_eq!(
        ui.state().drilled,
        Some(newest),
        "expanded/drilled rows survive a page-1 refresh"
    );
    ui.state_mut().drilled = None;
    ui.state_mut().pipeline_expanded.clear();
    ui.dispatch(Msg::PipelinesLoaded(
        PROJECT,
        epoch,
        1,
        Ok(Pipelines::Available(Vec::new())),
    ))
    .unwrap();
    assert!(ui.state().project_pipelines[&PROJECT].pipelines.is_empty());
    assert!(!ui.state().latest_pipeline.contains_key(&PROJECT));
}

#[test]
fn unreadable_pipelines_do_not_trigger_global_backoff() {
    let mut ui = mount(40);
    let epoch = ui.state().pipeline_epoch;
    let blocked = ui.state().blocked_until;
    ui.dispatch(Msg::ProbeLoaded(
        9004,
        epoch,
        Ok(Pipelines::Unavailable("GitLab returned 403".into())),
    ))
    .unwrap();
    assert_eq!(ui.state().blocked_until, blocked);
    assert!(ui.state().error.is_none());
    assert!(!ui.state().latest_pipeline.contains_key(&9004));
    assert!(ui.state().pipelines_unavailable.contains_key(&9004));
    ui.state_mut().probe_pending.clear();
    ui.dispatch(Msg::LoadProbes).unwrap();
    assert!(
        !ui.state().probe_pending.contains(&9004)
            && !ui.state().latest_pipeline.contains_key(&9004),
        "not probed again"
    );
    key(&mut ui, KeyCode::Enter);
    ui.dispatch(Msg::PipelinesLoaded(
        PROJECT,
        epoch,
        1,
        Ok(Pipelines::Unavailable("GitLab returned 404".into())),
    ))
    .unwrap();
    assert_eq!(ui.state().blocked_until, blocked);
    assert!(ui.state().error.is_none());
    let shown = text(&mut ui);
    assert!(shown.contains("GitLab returned 404"), "{shown}");
    // Transient probe failures back off quietly; explicit refresh clears unavailability.
    ui.dispatch(Msg::ProbeLoaded(9005, epoch, Err("HTTP 503".into())))
        .unwrap();
    assert!(ui.state().blocked_until > blocked);
    assert!(
        ui.state().error.is_none(),
        "no banner for a background probe"
    );
    ui.state_mut().blocked_until = std::time::Duration::ZERO;
    ui.dispatch(Msg::Refresh).unwrap();
    assert!(ui.state().pipelines_unavailable.is_empty());
}

#[test]
fn hiding_the_project_or_replacing_setup_stops_drilled_polling() {
    let mut ui = mount(40);
    open_pipelines(&mut ui);
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Enter);
    assert!(ui.state().drilled.is_some());
    assert!(ui.state().job_source().is_some());
    ui.dispatch(Msg::ToggleProject).unwrap();
    assert!(!ui.state().config.projects[0].visible);
    assert_eq!(ui.state().sections().len(), 2);
    assert!(ui.state().section_cursor <= 1);
    assert!(ui.state().drilled.is_none());
    assert!(
        ui.state().job_source().is_none(),
        "no trace polling for a hidden project"
    );
    ui.dispatch(Msg::ToggleProject).unwrap();
    assert_eq!(ui.state().sections().len(), 3);
    assert!(ui.state().project_pipelines.contains_key(&PROJECT));

    // Toggling issue visibility must not restart pipeline sync.
    let epoch = ui.state().pipeline_epoch;
    ui.dispatch(Msg::ToggleProjectKind(ItemKind::Issue))
        .unwrap();
    assert_eq!(ui.state().pipeline_epoch, epoch);

    // Replacing the GitLab setup (demo: save setup) drops windows and indicators.
    ui.dispatch(Msg::Action(cronk::ui::Action::Onboarding))
        .unwrap();
    settle(&mut ui);
    ui.dispatch(Msg::SetupValidate(ui.state().onboarding.epoch))
        .unwrap();
    for _ in 0..20 {
        settle(&mut ui);
        if ui.state().onboarding.validated.is_some() {
            break;
        }
    }
    ui.dispatch(Msg::Submit).unwrap();
    settle(&mut ui);
    assert!(ui.state().dialog.is_none(), "setup saved");
    assert!(ui.state().pipeline_epoch > epoch);
    assert!(ui.state().drilled.is_none());
    assert!(
        ui.state()
            .project_pipelines
            .get(&PROJECT)
            .is_none_or(|w| w.pages_loaded <= 1),
        "old windows were cleared before the fresh page-1 load"
    );
}

#[test]
fn space_in_project_details_never_toggles_visibility_outside_a_field() {
    let mut ui = mount(40);
    key(&mut ui, KeyCode::Enter);
    for section in 0..3 {
        ui.dispatch(Msg::DetailSection(section)).unwrap();
        key(&mut ui, KeyCode::Char(' '));
        assert!(ui.state().config.projects[0].visible, "section {section}");
    }
    // Inside Pipelines, Space acts on the pipeline under the cursor only.
    ui.dispatch(Msg::DetailSection(1)).unwrap();
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Char(' '));
    assert!(ui.state().config.projects[0].visible);
    let newest = ui.state().project_pipelines[&PROJECT].pipelines[0].id;
    assert!(!ui.state().pipeline_expanded(PROJECT, newest));
    // The Visibility field still toggles with Space when focused.
    key(&mut ui, KeyCode::Esc);
    ui.dispatch(Msg::DetailSection(0)).unwrap();
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Down);
    key(&mut ui, KeyCode::Char(' '));
    assert!(!ui.state().config.projects[0].visible);
}

#[test]
fn tab_visits_restore_the_drilled_pipeline_and_job() {
    let mut ui = mount(40);
    open_pipelines(&mut ui);
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Enter);
    key(&mut ui, KeyCode::Down);
    let drilled = ui.state().drilled.unwrap();
    let job = ui.state().selected_job().unwrap().id;
    ui.dispatch(Msg::Tab(2)).unwrap();
    settle(&mut ui);
    ui.dispatch(Msg::Tab(1)).unwrap();
    settle(&mut ui);
    assert_eq!(ui.state().scope, Scope::Section);
    assert_eq!(ui.state().section_name(), "Pipelines");
    assert_eq!(ui.state().drilled, Some(drilled));
    assert_eq!(ui.state().selected_job().map(|j| j.id), Some(job));

    // Restart: the saved tab state brings the cursor back to the same job once
    // the (memory-only) window has loaded again.
    let config = ui.state().config.clone();
    let mut restarted = TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config,
            path: None,
            api: None,
            demo: true,
        },
        (),
        ui.viewport(),
    );
    settle(&mut restarted);
    assert_eq!(restarted.state().section_name(), "Pipelines");
    assert_eq!(restarted.state().drilled, Some(drilled));
    assert_eq!(restarted.state().selected_job().map(|j| j.id), Some(job));
}
