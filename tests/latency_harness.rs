//! Phase-separated, informational latency harness for the headless list UI.
//!
//! Nothing here asserts on timings. `latency_harness_smoke` runs a tiny plan in the
//! normal debug test run so the harness itself stays correct; the full measurement is
//! `#[ignore]`d and meant for release builds:
//!
//! ```sh
//! just test --release --test latency_harness -E 'test(latency_harness_release)' \
//!     --run-ignored only --test-threads=1 --success-output immediate
//! ```
//!
//! Per gesture sample the harness records, in order:
//! * `dispatch`: `TestBackend::update_level` (the message goes straight to
//!   `Cronk::update`, nothing is drained or rendered). Mouse gestures use
//!   `send_mouse`, whose `dispatch` additionally includes input routing and the
//!   backend's own pump/render when the update was dirty.
//! * `render`: `TestBackend::render` (view + layout).
//! * `pump`: `TestBackend::pump` (drains queued messages such as viewport callbacks).
//! * `total`: wall time around the three. No real terminal writes are involved.
//!
//! `render` and `pump` run unconditionally after every dispatch (in `sample`), even when the
//! message did not make the UI dirty, so a no-op message still pays for a full frame.
use std::time::{Duration, Instant};

use cronk::{
    config::Config,
    demo,
    model::{ItemKey, ItemKind, Label, Pipeline, User, WorkItem},
    ui::{Action, Cronk, Msg},
};
use tui_lipan::{
    TestBackend,
    core::event::{MouseButton, MouseKind},
    prelude::*,
};

type Ui = TestBackend<Cronk>;

const PROJECT_COUNT: usize = 5;
const PAGE_SIZE: usize = 100;

#[derive(Clone, Copy)]
struct Scenario {
    name: &'static str,
    tab: usize,
    query: &'static str,
    path_backed: bool,
}

const EMPTY: Scenario = Scenario {
    name: "empty query (MR tab)",
    tab: 3,
    query: "",
    path_backed: false,
};
const TEXT: Scenario = Scenario {
    name: "text term `handoff` (MR tab)",
    tab: 3,
    query: "handoff",
    path_backed: false,
};
const ME: Scenario = Scenario {
    name: "`assignee:@me` (MR tab)",
    tab: 3,
    query: "assignee:@me",
    path_backed: false,
};
const DASHBOARD: Scenario = Scenario {
    name: "dashboard (tab 0)",
    tab: 0,
    query: "",
    path_backed: false,
};

impl Scenario {
    fn path_backed(self) -> Self {
        Self {
            path_backed: true,
            ..self
        }
    }
    fn label(&self) -> String {
        format!(
            "{} · {}",
            self.name,
            if self.path_backed {
                "path-backed"
            } else {
                "path: None"
            }
        )
    }
}

struct Plan {
    items: usize,
    scenarios: Vec<Scenario>,
    /// Sync pages sent per sync scenario (each page is `PAGE_SIZE` items).
    pages: usize,
    /// Each gesture's step count is divided by this; 1 is the full workload.
    divisor: usize,
}

// ---------------------------------------------------------------- fixtures

const WORDS: [&str; 48] = [
    "scheduler",
    "Worker",
    "queue",
    "pipeline",
    "Regression",
    "latency",
    "snapshot",
    "Index",
    "migration",
    "terminal",
    "Render",
    "layout",
    "label",
    "reviewer",
    "assignee",
    "budget",
    "Telemetry",
    "sampling",
    "backoff",
    "timeout",
    "Config",
    "flag",
    "rollout",
    "storage",
    "checksum",
    "Compaction",
    "journal",
    "upstream",
    "release",
    "artifact",
    "manifest",
    "Dependency",
    "bump",
    "toolchain",
    "documentation",
    "keyboard",
    "Selection",
    "viewport",
    "scrollbar",
    "palette",
    "session",
    "Project",
    "iteration",
    "milestone",
    "discussion",
    "approval",
    "Threshold",
    "benchmark",
];
const NON_ASCII: [&str; 6] = [
    "naïve café résumé",
    "Ünïcödé Straße",
    "日本語のテスト",
    "Σίσυφος ΌΣΟΣ",
    "İstanbul ǅ",
    "Привет, мир",
];
const LABELS: [&str; 12] = [
    "backend",
    "frontend",
    "bug",
    "feature",
    "needs review",
    "blocked",
    "performance",
    "docs",
    "security",
    "tech-debt",
    "ux",
    "ci",
];

fn lcg(state: &mut u64) -> usize {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*state >> 33) as usize
}

/// About 2 KB of markdown-ish text. `lease handoff` appears in every 10th item, `retry` in
/// every 7th and `cache` in every 5th, near the end so a text search scans most of the body.
fn description(n: usize) -> String {
    let mut state = n as u64 + 1;
    let mut text = format!("## Summary for change {n}\n\n");
    let mut line = 0usize;
    while text.len() < 1_950 {
        let word = WORDS[lcg(&mut state) % WORDS.len()];
        text.push_str(word);
        line += 1;
        text.push(if line.is_multiple_of(12) { '\n' } else { ' ' });
    }
    if n.is_multiple_of(4) {
        text.push_str(NON_ASCII[n / 4 % NON_ASCII.len()]);
        text.push('\n');
    }
    if n.is_multiple_of(10) {
        text.push_str("Fix lease Handoff after cancellation. ");
    }
    if n.is_multiple_of(7) {
        text.push_str("Retry with backoff. ");
    }
    if n.is_multiple_of(5) {
        text.push_str("Cache invalidation. ");
    }
    text
}

fn user(id: u64) -> User {
    User {
        id,
        username: format!("user-{id}"),
        name: format!("User Number {id}"),
        ..User::default()
    }
}

/// Item `n` of the fixture: key and project are overridden by the caller.
fn fixture_item(n: usize, project: u64) -> WorkItem {
    let pool = |k: usize| user(102 + (k % 8) as u64);
    let me = demo::user();
    let title_tail = if n.is_multiple_of(9) {
        "naïve fallback für Σ"
    } else {
        "handle scheduler timeouts"
    };
    let seconds = (n * 104_729) % 2_592_000;
    WorkItem {
        key: ItemKey {
            project,
            iid: n as u64 + 1,
            kind: ItemKind::MergeRequest,
        },
        title: format!("Large list MR {}: {title_tail}", n + 1),
        description: description(n),
        state: match n {
            n if n.is_multiple_of(11) => "merged",
            n if n.is_multiple_of(13) => "closed",
            _ => "opened",
        }
        .into(),
        labels: (0..3 + n % 2)
            .map(|k| Label {
                name: LABELS[(n * 5 + k * 3) % LABELS.len()].into(),
                color: "#428bca".into(),
                text_color: "#ffffff".into(),
            })
            .collect(),
        author: if n.is_multiple_of(7) {
            me.clone()
        } else {
            pool(n + 1)
        },
        assignees: if n.is_multiple_of(3) {
            vec![me.clone(), pool(n)]
        } else if n.is_multiple_of(2) {
            vec![pool(n)]
        } else {
            Vec::new()
        },
        reviewers: if n.is_multiple_of(5) {
            vec![me]
        } else {
            vec![pool(n + 3), pool(n + 5)]
        },
        milestone: "Milestone 1".into(),
        milestone_id: Some(1),
        iteration: format!("Sprint {}", n % 6),
        iteration_id: Some(1_000 + (n % 6) as u64),
        source_branch: format!("feature/change-{n}"),
        target_branch: "main".into(),
        updated_at: format!(
            "2026-09-{:02}T{:02}:{:02}:{:02}Z",
            1 + seconds / 86_400,
            seconds / 3_600 % 24,
            seconds / 60 % 60,
            seconds % 60
        ),
        web_url: format!("https://gitlab.example.invalid/mr/{n}"),
        draft: n.is_multiple_of(17),
        pipeline: Some(Pipeline {
            id: n as u64,
            status: ["success", "failed", "running"][n % 3].into(),
            web_url: String::new(),
        }),
        unresolved: Some(if n.is_multiple_of(4) { 2 } else { 0 }),
        ..WorkItem::default()
    }
}

fn fixture(count: usize) -> Vec<WorkItem> {
    (0..count)
        .map(|n| fixture_item(n, 9_001 + (n % PROJECT_COUNT) as u64))
        .collect()
}

// ---------------------------------------------------------------- statistics

/// Timings of one gesture sample.
#[derive(Clone, Copy)]
struct Phases {
    dispatch: Duration,
    render: Duration,
    pump: Duration,
    total: Duration,
}

#[derive(Default)]
struct Series {
    dispatch: Vec<Duration>,
    render: Vec<Duration>,
    pump: Vec<Duration>,
    total: Vec<Duration>,
}

impl Series {
    fn push(&mut self, phases: Phases) {
        self.dispatch.push(phases.dispatch);
        self.render.push(phases.render);
        self.pump.push(phases.pump);
        self.total.push(phases.total);
    }
    fn len(&self) -> usize {
        self.total.len()
    }
    fn phases(&self) -> [(&'static str, &Vec<Duration>); 4] {
        [
            ("dispatch", &self.dispatch),
            ("render", &self.render),
            ("pump", &self.pump),
            ("total", &self.total),
        ]
    }
}

fn sorted(samples: &[Duration]) -> Vec<Duration> {
    let mut sorted = samples.to_vec();
    sorted.sort();
    sorted
}

fn median(samples: &[Duration]) -> Duration {
    let s = sorted(samples);
    let mid = s.len() / 2;
    if s.len() % 2 == 1 {
        s[mid]
    } else {
        (s[mid - 1] + s[mid]) / 2
    }
}

/// Nearest-rank 95th percentile.
fn p95(samples: &[Duration]) -> Duration {
    let s = sorted(samples);
    s[(s.len() * 95).div_ceil(100).max(1) - 1]
}

fn ms(duration: Duration) -> String {
    format!("{:.3}", duration.as_secs_f64() * 1_000.0)
}

fn print_raw(scenario: &str, gesture: &str, series: &Series) {
    for (phase, samples) in series.phases() {
        let micros: Vec<String> = samples.iter().map(|d| d.as_micros().to_string()).collect();
        eprintln!(
            "RAW | {scenario} | {gesture} | {phase} | n={} | µs: {}",
            samples.len(),
            micros.join(" ")
        );
    }
}

/// One markdown row: median / p95 / max (ms) for each phase.
fn table_row(scenario: &str, gesture: &str, series: &Series) -> String {
    let cells: Vec<String> = series
        .phases()
        .iter()
        .map(|(_, s)| {
            format!(
                "{} / {} / {}",
                ms(median(s)),
                ms(p95(s)),
                ms(*s.iter().max().unwrap())
            )
        })
        .collect();
    format!(
        "| {scenario} | {gesture} | {} | {} |",
        series.len(),
        cells.join(" | ")
    )
}

const TABLE_HEADER: &str = "| Scenario | Gesture | n | dispatch med / p95 / max (ms) | render | pump | total |\n| --- | --- | ---: | ---: | ---: | ---: | ---: |";

// ---------------------------------------------------------------- mounting

fn mount(scenario: Scenario, items: Vec<WorkItem>) -> (Box<Ui>, Option<tempfile::TempDir>) {
    let directory = scenario
        .path_backed
        .then(|| tempfile::tempdir().expect("temporary workspace"));
    let mut config = Config {
        projects: demo::projects(),
        active_tab: scenario.tab,
        onboarding: false,
        ..Config::default()
    };
    if !scenario.query.is_empty() {
        config
            .filters
            .insert(scenario.tab.to_string(), scenario.query.into());
    }
    let mut ui = Box::new(TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config,
            path: directory
                .as_ref()
                .map(|dir| dir.path().join("workspace.toml")),
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
    ));
    ui.pump().unwrap();
    ui.state_mut().items = items;
    ui.dispatch(Msg::Select(0)).unwrap();
    for _ in 0..3 {
        ui.render();
        ui.pump().unwrap();
    }
    (ui, directory)
}

// ---------------------------------------------------------------- gestures

enum Step {
    Msg(Msg),
    Mouse(MouseEvent),
    /// Open the filter dialog (unmeasured), then submit this query (measured).
    Query(String),
}

fn mouse(x: u16, y: u16, kind: MouseKind) -> Step {
    Step::Mouse(MouseEvent {
        x,
        y,
        kind,
        mods: KeyMods::NONE,
    })
}

/// Times `dispatch` (the only part that varies per entry point), then the render and pump
/// that follow it.
fn sample<R>(ui: &mut Ui, dispatch: impl FnOnce(&mut Ui) -> R) -> Phases {
    let start = Instant::now();
    dispatch(ui);
    // Rendering is unconditional: even a message that did not mark the UI dirty is
    // followed by a full render, so an empty progress report costs a whole frame here.
    let dispatched = Instant::now();
    ui.render();
    let rendered = Instant::now();
    ui.pump().unwrap();
    let done = Instant::now();
    Phases {
        dispatch: dispatched - start,
        render: rendered - dispatched,
        pump: done - rendered,
        total: done - start,
    }
}

fn sample_update(ui: &mut Ui, msg: Msg) -> Phases {
    sample(ui, |ui| ui.update_level(msg).unwrap())
}

fn run_step(ui: &mut Ui, step: Step, series: Option<&mut Series>) {
    let phases = match step {
        Step::Mouse(event) => sample(ui, |ui| ui.send_mouse(event).unwrap()),
        Step::Msg(msg) => sample_update(ui, msg),
        Step::Query(query) => {
            ui.dispatch(Msg::Action(Action::Filter)).unwrap();
            let end = query.chars().count();
            ui.state_mut().dialog.as_mut().unwrap().fields[0].set_text_and_cursor(query, end);
            sample_update(ui, Msg::Submit)
        }
    };
    if let Some(series) = series {
        series.push(phases);
    }
}

/// One unmeasured warm-up pass of `steps`, then one measured pass.
fn measure(ui: &mut Ui, steps: impl Fn() -> Vec<Step>) -> Series {
    for step in steps() {
        run_step(ui, step, None);
    }
    let mut series = Series::default();
    for step in steps() {
        run_step(ui, step, Some(&mut series));
    }
    series
}

fn run_scenario(scenario: Scenario, items: &[WorkItem], divisor: usize) -> Vec<(String, Series)> {
    let (mut ui, _workspace) = mount(scenario, items.to_vec());
    let len = ui.state().visible_item_count();
    assert!(len > 100, "{}: only {len} visible items", scenario.label());
    let tab = scenario.tab;
    let mut results = Vec::new();

    results.push((
        "Move(1)".to_owned(),
        measure(&mut ui, || {
            (0..30 / divisor).map(|_| Step::Msg(Msg::Move(1))).collect()
        }),
    ));

    results.push((
        "Select deep jump".to_owned(),
        measure(&mut ui, || {
            [
                len / 2,
                len - 1,
                0,
                len / 100,
                len * 2 / 3,
                len / 30,
                len * 5 / 6,
                len / 5,
            ]
            .into_iter()
            .take(8 / divisor)
            .map(|index| Step::Msg(Msg::Select(index)))
            .collect()
        }),
    ));

    let rect = ui.rect_of_key(&format!("main-list-{tab}").into()).unwrap();
    let bar_x = rect.x.max(0) as u16 + rect.w - 1;
    let top = rect.y.max(0) as u16;
    let bottom = top + rect.h - 1;

    results.push((
        "wheel (15 down, 15 up)".to_owned(),
        measure(&mut ui, || {
            let wheel = |kind| mouse(10, top + 1, kind);
            (0..15 / divisor)
                .map(|_| wheel(MouseKind::ScrollDown))
                .chain((0..15 / divisor).map(|_| wheel(MouseKind::ScrollUp)))
                .collect()
        }),
    ));

    let drag = measure(&mut ui, || {
        let drags = 16 / divisor;
        let mut steps = vec![mouse(bar_x, top, MouseKind::Down(MouseButton::Left))];
        for i in 1..=drags {
            let y = top + ((bottom - top) as usize * i / drags) as u16;
            steps.push(mouse(bar_x, y, MouseKind::Drag(MouseButton::Left)));
        }
        steps.push(mouse(bar_x, bottom, MouseKind::Up(MouseButton::Left)));
        steps
    });
    assert!(
        ui.state().scroll.offset > 0,
        "scrollbar drag should have moved the list"
    );
    results.push(("scrollbar drag".to_owned(), drag));

    let other = if tab == 0 { 3 } else { 0 };
    results.push((
        format!("tab switch ({tab}↔{other})"),
        measure(&mut ui, || {
            (0..10 / divisor)
                .map(|i| Step::Msg(Msg::Tab(if i.is_multiple_of(2) { other } else { tab })))
                .collect()
        }),
    ));
    assert_eq!(ui.state().config.active_tab, tab);

    results.push((
        "text-query edit (Submit)".to_owned(),
        measure(&mut ui, || {
            [
                "lease",
                "lease handoff",
                "handoff",
                "retry",
                "retry cache",
                "cache",
                "zzz-nomatch",
            ]
            .into_iter()
            .map(str::to_owned)
            .chain([scenario.query.to_owned()])
            .take(8 / divisor)
            .map(Step::Query)
            .collect()
        }),
    ));
    results
}

// ---------------------------------------------------------------- sync

/// Pages of `PAGE_SIZE` items, round-robin across projects, each preceded by the empty
/// `saving` report that `GitLab::sync_project` sends before it.
fn run_sync(tab: usize, pages: usize) -> Vec<(String, Series)> {
    let scenario = Scenario {
        name: "progressive sync",
        tab,
        query: "",
        path_backed: false,
    };
    let (mut ui, _) = mount(scenario, Vec::new());
    let epoch = ui.state().list_epoch;
    let progress = |kind, loaded, page, phase| cronk::gitlab::SyncProgress {
        kind,
        loaded,
        total: None,
        page,
        phase,
    };
    let messages: Vec<(Msg, bool)> = (0..pages)
        .flat_map(|page| {
            let project = 9_001 + (page % PROJECT_COUNT) as u64;
            let items: Vec<_> = (0..PAGE_SIZE)
                .map(|k| fixture_item(page * PAGE_SIZE + k, project))
                .collect();
            let loaded = (page + 1) * PAGE_SIZE;
            [
                (
                    Msg::ProjectProgress(
                        project,
                        epoch,
                        progress(ItemKind::MergeRequest, loaded, page + 1, "saving"),
                        Vec::new(),
                    ),
                    false,
                ),
                (
                    Msg::ProjectProgress(
                        project,
                        epoch,
                        progress(ItemKind::MergeRequest, loaded, page + 1, "fetching"),
                        items,
                    ),
                    true,
                ),
            ]
        })
        .collect();
    let (mut with_items, mut saving, mut all) =
        (Series::default(), Series::default(), Series::default());
    let started = Instant::now();
    for (msg, has_items) in messages {
        let phases = sample_update(&mut ui, msg);
        if has_items {
            with_items.push(phases);
        } else {
            saving.push(phases);
        }
        all.push(phases);
    }
    let wall = started.elapsed();
    assert_eq!(ui.state().items.len(), pages * PAGE_SIZE);
    eprintln!(
        "SYNC tab {tab}: {pages} pages x {PAGE_SIZE} items, {} messages, wall time {wall:?}",
        all.len()
    );
    vec![
        (
            format!("ProjectProgress page ({PAGE_SIZE} items)"),
            with_items,
        ),
        ("ProjectProgress `saving` (empty)".to_owned(), saving),
        ("all messages".to_owned(), all),
    ]
}

// ---------------------------------------------------------------- entry points

fn run(plan: &Plan) {
    eprintln!(
        "HARNESS profile={} items={} pages={} divisor={} parallelism={:?}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        plan.items,
        plan.pages,
        plan.divisor,
        std::thread::available_parallelism().map(usize::from)
    );
    let items = fixture(plan.items);
    let description_bytes: usize = items.iter().map(|i| i.description.len()).sum::<usize>();
    eprintln!(
        "FIXTURE {} MRs, mean description {} bytes, {} with non-ASCII text",
        items.len(),
        description_bytes / items.len(),
        items
            .iter()
            .filter(|i| !i.description.is_ascii() || !i.title.is_ascii())
            .count()
    );
    let mut rows = Vec::new();
    for scenario in &plan.scenarios {
        eprintln!("RUN scenario {}", scenario.label());
        for (gesture, series) in run_scenario(*scenario, &items, plan.divisor) {
            assert!(series.len() > 0, "{gesture} produced no samples");
            print_raw(&scenario.label(), &gesture, &series);
            rows.push(table_row(&scenario.label(), &gesture, &series));
        }
    }
    for tab in [3, 0] {
        eprintln!("RUN sync tab {tab}");
        for (gesture, series) in run_sync(tab, plan.pages) {
            let scenario = format!(
                "progressive sync, tab {tab}, {} items",
                plan.pages * PAGE_SIZE
            );
            print_raw(&scenario, &gesture, &series);
            rows.push(table_row(&scenario, &gesture, &series));
        }
    }
    eprintln!("{TABLE_HEADER}");
    for row in rows {
        eprintln!("{row}");
    }
}

#[test]
fn latency_harness_smoke() {
    run(&Plan {
        items: 400,
        scenarios: vec![EMPTY, DASHBOARD.path_backed()],
        pages: 3,
        divisor: 5,
    });
}

#[test]
#[ignore = "release-mode measurement; see the module documentation"]
fn latency_harness_release() {
    // Runs path: None before path-backed for each scenario.
    let scenarios = [EMPTY, TEXT, ME, DASHBOARD]
        .into_iter()
        .flat_map(|s| [s, s.path_backed()])
        .collect();
    run(&Plan {
        items: 3_000,
        scenarios,
        pages: 30,
        divisor: 1,
    });
}
