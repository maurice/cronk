//! Deterministic, wholly fictional demo data. No network, clock, randomness, or company data.
//! `trace` returns a cumulative, bounded log snapshot (not a byte-range delta). Running jobs
//! remain running after 40 synthetic progress steps; two jobs run simultaneously in each
//! running pipeline. Unknown item/job IDs are explicitly identified rather than fabricated.

/// Demo "now": 2026-09-28T12:00:00Z, after every fixture timestamp.
pub const NOW_UNIX: i64 = 1_790_596_800;

use crate::model::{
    Details, Diff, Discussion, ItemKey, ItemKind, Job, Label, Note, Pipeline, Project, User,
    WorkItem,
};

pub fn projects() -> Vec<Project> {
    [
        (
            "demo-lab/constellation/platform/runtime/orbit-scheduler",
            "Orbit scheduler · DEMO",
        ),
        (
            "demo-lab/constellation/platform/storage/meteor-cache",
            "Meteor cache · DEMO",
        ),
        (
            "demo-lab/constellation/experience/terminal/starboard",
            "Starboard TUI · DEMO",
        ),
        (
            "demo-lab/constellation/research/telemetry/pulsar-agent",
            "Pulsar agent · DEMO",
        ),
        (
            "demo-lab/constellation/delivery/tooling/launch-kit",
            "Launch kit · DEMO",
        ),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (path, alias))| Project {
        id: 9_001 + index as u64,
        path: path.into(),
        alias: alias.into(),
        visible: true,
        ..Project::default()
    })
    .collect()
}

pub fn user() -> User {
    User {
        id: 101,
        username: "demo-arin".into(),
        name: "Arin Example (demo)".into(),
        ..User::default()
    }
}

fn reviewer() -> User {
    User {
        id: 102,
        username: "demo-mila".into(),
        name: "Mila Sample (demo)".into(),
        ..User::default()
    }
}

fn teammate() -> User {
    User {
        id: 103,
        username: "demo-sam".into(),
        name: "Sam Fiction (demo)".into(),
        ..User::default()
    }
}

fn bot() -> User {
    User {
        id: 104,
        username: "demo-ci-bot".into(),
        name: "Constellation CI (demo)".into(),
        bot: true,
        ..User::default()
    }
}

fn label(name: &str, color: &str, text_color: &str) -> Label {
    Label {
        name: name.into(),
        color: color.into(),
        text_color: text_color.into(),
    }
}

const ISSUE_TITLES: [[&str; 4]; 5] = [
    [
        "A stalled worker can hold the scheduling lease",
        "Expose queue age by priority",
        "Document fair-share scheduling",
        "Retire the legacy polling loop",
    ],
    [
        "Cache eviction spikes during snapshot restore",
        "Add a bounded read-through cache",
        "Publish the cache key compatibility guide",
        "Remove obsolete cache warmup flags",
    ],
    [
        "Preserve selection when a project refreshes",
        "Support nested groups in the project picker",
        "Document keyboard-only review workflow",
        "Fix narrow-terminal label wrapping",
    ],
    [
        "Collector retries amplify an upstream outage",
        "Export per-source sampling counters",
        "Explain trace redaction guarantees",
        "Delete unused telemetry v1 adapters",
    ],
    [
        "Release job loses artifacts after a retry",
        "Add reproducible package manifests",
        "Write a rollback rehearsal checklist",
        "Archive the old release checklist",
    ],
];

const MR_TITLES: [[&str; 4]; 5] = [
    [
        "Fix lease handoff after worker cancellation",
        "Run scheduler integration suites in parallel",
        "Add queue-depth diagnostics",
        "Replace polling with bounded notifications",
    ],
    [
        "Keep eviction inside the memory budget",
        "Stream cache snapshots without blocking readers",
        "Add cache hit-rate panels",
        "Remove the v1 key serializer",
    ],
    [
        "Keep focus anchored to stable item keys",
        "Render live traces for concurrent jobs",
        "Add a compact discussion navigator",
        "Normalize deeply nested project paths",
    ],
    [
        "Bound retries with per-source backpressure",
        "Exercise exporters against slow receivers",
        "Add structured sampling diagnostics",
        "Drop deprecated collector headers",
    ],
    [
        "Retain signed artifacts when packaging retries",
        "Build Linux and Windows packages concurrently",
        "Verify reproducible release manifests",
        "Switch releases to the shared pipeline",
    ],
];

pub fn items() -> Vec<WorkItem> {
    let mut items = Vec::with_capacity(40);
    for (index, project) in projects().into_iter().enumerate() {
        for slot in 0..8 {
            let mr = slot >= 4;
            let local = slot % 4;
            let kind = if mr {
                ItemKind::MergeRequest
            } else {
                ItemKind::Issue
            };
            let iid = if mr {
                101 + local as u64
            } else {
                1 + local as u64
            };
            let state = match (mr, local) {
                (false, 3) => "closed",
                (true, 3) if index % 2 == 0 => "merged",
                (true, 3) => "closed",
                _ => "opened",
            };
            let title = if mr {
                MR_TITLES[index][local]
            } else {
                ISSUE_TITLES[index][local]
            };
            let url = format!(
                "https://gitlab.demo.invalid/{}/-/{}/{iid}",
                project.path,
                kind.segment()
            );
            let pipeline = mr.then(|| {
                let status = match local {
                    0 => "failed",
                    1 => "running",
                    2 => "success",
                    _ if state == "merged" => "success",
                    _ => "canceled",
                };
                let id = 50_000 + index as u64 * 100 + local as u64;
                head_pipeline(id, status, &project.path, local, index)
            });
            let mut labels = vec![
                label("demo::fictional", "#6f42c1", "#ffffff"),
                label(
                    [
                        "area::runtime",
                        "area::storage",
                        "area::terminal",
                        "area::telemetry",
                        "area::delivery",
                    ][index],
                    "#1f78d1",
                    "#ffffff",
                ),
                label(
                    if local == 0 {
                        "priority::high"
                    } else {
                        "priority::normal"
                    },
                    if local == 0 { "#c0392b" } else { "#f1c40f" },
                    if local == 0 { "#ffffff" } else { "#000000" },
                ),
            ];
            labels.push(match local {
                0 => label("type::bug", "#d35400", "#ffffff"),
                1 => label("type::feature", "#117864", "#ffffff"),
                2 => label("review::wanted", "#d5f5e3", "#000000"),
                _ => label("status::archived", "#5d6d7e", "#ffffff"),
            });
            let description = format!(
                "## Fictional demo: {title}\n\nThis is synthetic sample data for Cronk, not a real company or repository.\n\n\
                 ### Context\nThe {} subsystem needs predictable behavior during concurrent work and graceful shutdown.\n\n\
                 ### Acceptance criteria\n- [x] Reproduce the behavior with a deterministic fixture\n- [x] Preserve compatibility with the existing API\n- [{}] Verify failure recovery on Linux and Windows\n- [ ] Have another demo reviewer check the documentation\n\n\
                 ### Verification\n```sh\ncargo test --all-targets\ncargo clippy --all-targets -- -D warnings\n```\n\n\
                 Related demo issue: #{} · demo review: !{}\n\n> All names, URLs, logs, and discussion messages in this workspace are fictional.",
                project.alias,
                if local >= 2 { "x" } else { " " },
                local + 1,
                101 + local,
            );
            items.push(WorkItem {
                key: ItemKey {
                    project: project.id,
                    iid,
                    kind,
                },
                title: format!("[DEMO] {title}"),
                description,
                state: state.into(),
                labels,
                author: if local % 2 == 0 { user() } else { teammate() },
                assignees: if local == 1 {
                    vec![user(), teammate()]
                } else {
                    vec![user()]
                },
                reviewers: if mr {
                    vec![reviewer(), teammate()]
                } else {
                    Vec::new()
                },
                milestone: "Demo milestone · Constellation 0.4".into(),
                milestone_id: Some(401),
                iteration: format!("Demo iteration {} · predictable concurrency", 12 + index),
                iteration_id: Some(1200 + index as u64),
                epic: if mr {
                    String::new()
                } else {
                    "Demo epic · Reliable delivery".into()
                },
                epic_id: if mr { None } else { Some(701) },
                source_branch: if mr {
                    format!(
                        "demo/{}/{}",
                        ["runtime", "storage", "terminal", "telemetry", "delivery"][index],
                        ["fix-recovery", "parallel-ci", "diagnostics", "cleanup"][local]
                    )
                } else {
                    String::new()
                },
                target_branch: if mr { "main".into() } else { String::new() },
                updated_at: format!("2026-09-{:02}T{:02}:24:00Z", 28 - index, 16 - slot),
                web_url: url,
                draft: mr && local == 1 && index % 2 == 1,
                pipeline,
                unresolved: Some(if mr && local == 0 {
                    2
                } else if mr && local == 1 {
                    1
                } else {
                    0
                }),
            });
        }
    }
    items
}

fn note(
    id: u64,
    author: User,
    body: &str,
    minute: u8,
    system: bool,
    resolvable: bool,
    resolved: bool,
) -> Note {
    Note {
        id,
        author,
        body: body.into(),
        created_at: format!("2026-09-28T09:{minute:02}:00Z"),
        system,
        resolvable,
        resolved,
    }
}

fn jobs(item: &WorkItem) -> Vec<Job> {
    let Some(pipeline) = &item.pipeline else {
        return Vec::new();
    };
    pipeline_jobs(pipeline, &item.web_url)
}

fn pipeline_jobs(pipeline: &Pipeline, base_url: &str) -> Vec<Job> {
    let entries = [
        ("format", "check"),
        ("clippy", "check"),
        ("unit-tests", "test"),
        ("integration:linux", "integration"),
        ("integration:windows", "integration"),
        ("benchmark:advisory", "integration"),
        ("package", "package"),
        ("publish:preview", "deploy"),
    ];
    let mut jobs: Vec<Job> = entries
        .into_iter()
        .enumerate()
        .map(|(index, (name, stage))| {
            let status = match (pipeline.status.as_str(), index) {
                ("running", 3 | 4) => "running",
                ("running", 5) => "pending",
                ("running", 6 | 7) => "created",
                ("failed", 3 | 5) => "failed",
                ("failed", 6 | 7) => "skipped",
                ("canceled", 3..=7) => "canceled",
                ("success", 5) => "failed", // Advisory benchmark is allowed to fail.
                ("success", 7) => "manual",
                _ => "success",
            };
            let id = pipeline.id * 100 + index as u64 + 1;
            Job {
                id,
                name: name.into(),
                stage: stage.into(),
                status: status.into(),
                allow_failure: index == 5,
                started_at: matches!(status, "running" | "success" | "failed" | "canceled")
                    .then(|| format!("2025-01-01T10:{index:02}:00Z")),
                web_url: format!(
                    "{}/-/jobs/{id}",
                    base_url
                        .split("/-/")
                        .next()
                        .unwrap_or("https://gitlab.demo.invalid")
                ),
            }
        })
        .collect();
    crate::model::sort_jobs(&mut jobs);
    jobs
}

/// A merge request's head pipeline, as embedded in the MR response (full detail fields).
fn head_pipeline(id: u64, status: &str, path: &str, slot: usize, project_index: usize) -> Pipeline {
    let created = NOW_UNIX - ((slot as i64 + 1) * 23 * 60 + project_index as i64 * 180);
    let duration = (status != "running").then_some(6 * 60 + 40 + slot as u64 * 11);
    let started = created + 15;
    Pipeline {
        id,
        status: status.into(),
        web_url: format!("https://gitlab.demo.invalid/{path}/-/pipelines/{id}"),
        source: Some("merge_request_event".into()),
        ref_name: Some(format!("refs/merge-requests/{}/head", 101 + slot)),
        sha: Some(format!("{id:040x}")),
        name: None,
        created_at: Some(timestamp(created)),
        updated_at: Some(timestamp(
            duration.map_or(NOW_UNIX - 5, |d| started + d as i64),
        )),
        started_at: Some(timestamp(started)),
        finished_at: duration.map(|d| timestamp(started + d as i64)),
        duration,
        queued_duration: Some(15),
        user: Some(if slot.is_multiple_of(2) {
            user()
        } else {
            teammate()
        }),
    }
}

fn timestamp(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
}

/// Every recent pipeline of a demo project, newest first: scheduled, push, web and API
/// pipelines interleaved with the merge request head pipelines so both views agree.
/// Project 9001 has 45 synthetic entries plus its 4 MR heads (three window pages); the
/// others fit on one short page.
fn all_project_pipelines(project: u64) -> Vec<Pipeline> {
    let Some(index) = project.checked_sub(9_001).filter(|i| *i < 5) else {
        return Vec::new();
    };
    let path = projects()[index as usize].path.clone();
    let heads: Vec<Pipeline> = items()
        .into_iter()
        .filter(|item| item.key.project == project)
        .filter_map(|item| item.pipeline)
        .collect();
    let count = if index == 0 { 45 } else { 12 };
    let mut pipelines = Vec::with_capacity(count);
    for n in 0..count {
        let id = 60_999 + index * 1_000 - n as u64;
        let created = NOW_UNIX - (n as i64 * 47 * 60 + 3 * 60 + 12 + index as i64 * 180);
        let (source, reference, user, status, name) = match n % 6 {
            0 => (
                "schedule",
                "main",
                bot(),
                if index % 2 == 0 && n == 0 {
                    "running"
                } else {
                    "success"
                },
                Some("nightly"),
            ),
            1 => ("push", "main", user(), "success", None),
            2 => ("push", "main", teammate(), "failed", None),
            3 => ("web", "release/2026.09", reviewer(), "success", None),
            4 => ("api", "main", bot(), "canceled", None),
            _ => ("schedule", "main", bot(), "success", Some("nightly")),
        };
        let duration = if status == "running" {
            None
        } else {
            Some(8 * 60 + 2 + (n as u64 * 37) % 300)
        };
        let started = created + 20;
        let finished = duration.map(|d| started + d as i64);
        pipelines.push(Pipeline {
            id,
            status: status.into(),
            web_url: format!("https://gitlab.demo.invalid/{path}/-/pipelines/{id}"),
            source: Some(source.into()),
            ref_name: Some(reference.into()),
            sha: Some(format!("{id:040x}")),
            name: name.map(Into::into),
            created_at: Some(timestamp(created)),
            updated_at: Some(timestamp(finished.unwrap_or(NOW_UNIX - 5))),
            started_at: Some(timestamp(started)),
            finished_at: finished.map(timestamp),
            duration,
            queued_duration: Some(20),
            user: Some(user),
        });
    }
    // Merge request head pipelines sit between the first synthetic rows.
    pipelines.extend(heads);
    pipelines.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    pipelines
}

/// One window page (`crate::gitlab::PIPELINE_PAGE` entries) of a demo project's pipelines,
/// with only the fields GitLab's list endpoint returns.
pub fn project_pipelines(project: u64, page: usize) -> Vec<Pipeline> {
    let size = crate::gitlab::PIPELINE_PAGE;
    all_project_pipelines(project)
        .into_iter()
        .skip(page.max(1).saturating_sub(1) * size)
        .take(size)
        .map(|pipeline| Pipeline {
            started_at: None,
            finished_at: None,
            duration: None,
            queued_duration: None,
            user: None,
            ..pipeline
        })
        .collect()
}

/// The single-pipeline endpoint plus its jobs. Unknown ids are reported, not fabricated.
pub fn pipeline_with_jobs(project: u64, id: u64) -> Result<(Pipeline, Vec<Job>), String> {
    let pipeline = all_project_pipelines(project)
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("[DEMO] Pipeline {id} is not part of the demo dataset"))?;
    let jobs = pipeline_jobs(&pipeline, &pipeline.web_url);
    Ok((pipeline, jobs))
}

fn find_job(job: u64) -> Option<Job> {
    items()
        .into_iter()
        .find_map(|item| jobs(&item).into_iter().find(|entry| entry.id == job))
        .or_else(|| {
            (9_001..=9_005).find_map(|project| {
                all_project_pipelines(project)
                    .into_iter()
                    .find_map(|pipeline| {
                        pipeline_jobs(&pipeline, &pipeline.web_url)
                            .into_iter()
                            .find(|entry| entry.id == job)
                    })
            })
        })
}

pub fn details(key: &ItemKey) -> Details {
    let Some(item) = items().into_iter().find(|item| item.key == *key) else {
        return Details {
            item: WorkItem { key: key.clone(), title: "[DEMO] Unknown fixture".into(), ..WorkItem::default() },
            warnings: vec!["This key is not part of the deterministic demo dataset; no network request was made.".into()],
            ..Details::default()
        };
    };
    let base = key.project * 100_000 + key.iid * 100;
    let unresolved = item.unresolved.unwrap_or(0);
    let resolvable = key.kind == ItemKind::MergeRequest;
    let mut discussions = vec![
        Discussion {
            id: format!("demo-{}-{}-recovery", key.project, key.iid),
            notes: vec![
                note(
                    base + 11,
                    reviewer(),
                    "[DEMO] Could cancellation race with the final lease update? Please cover the interleaving where the receiver closes first.",
                    12,
                    false,
                    resolvable,
                    resolvable && unresolved == 0,
                ),
                note(
                    base + 12,
                    user(),
                    "[DEMO] Added a controlled barrier in the regression fixture. The remaining question is whether to retry or surface an explicit error.",
                    21,
                    false,
                    resolvable,
                    resolvable && unresolved == 0,
                ),
            ],
        },
        Discussion {
            id: format!("demo-{}-{}-bounds", key.project, key.iid),
            notes: vec![
                note(
                    base + 21,
                    teammate(),
                    "[DEMO] Please make the resource bound explicit in the API docs rather than silently dropping work.",
                    17,
                    false,
                    resolvable,
                    resolvable && unresolved < 2,
                ),
                note(
                    base + 22,
                    user(),
                    "[DEMO] Agreed. The new error includes the configured byte budget and a suggested recovery action.",
                    26,
                    false,
                    resolvable,
                    resolvable && unresolved < 2,
                ),
            ],
        },
        Discussion {
            id: format!("demo-{}-{}-docs", key.project, key.iid),
            notes: vec![note(
                base + 31,
                reviewer(),
                "[DEMO] The example now explains the shutdown contract clearly. No further questions from me.",
                32,
                false,
                resolvable,
                resolvable,
            )],
        },
    ];
    let mut notes = vec![
        note(
            base + 1,
            bot(),
            "[DEMO] added the demo::fictional label and assigned the Constellation milestone",
            1,
            true,
            false,
            false,
        ),
        note(
            base + 2,
            user(),
            "[DEMO] Reproduction is ready. Run the deterministic concurrency fixture; it requires no external services.",
            5,
            false,
            false,
            false,
        ),
        note(
            base + 3,
            teammate(),
            "[DEMO] Tested the recovery path locally. The Linux and Windows integration jobs should run at the same time; compare their live logs.",
            35,
            false,
            false,
            false,
        ),
        note(
            base + 4,
            bot(),
            match item.pipeline.as_ref().map(|p| p.status.as_str()) {
                Some("failed") => {
                    "[DEMO] pipeline failed: integration:linux exceeded the recovery deadline. Advisory benchmark failure is non-blocking."
                }
                Some("running") => {
                    "[DEMO] pipeline running: integration:linux and integration:windows are executing concurrently."
                }
                Some("success") => {
                    "[DEMO] required jobs passed; the optional benchmark remains advisory."
                }
                _ => "[DEMO] linked the regression fixture and updated the acceptance checklist.",
            },
            42,
            true,
            false,
            false,
        ),
    ];
    for entry in notes
        .iter_mut()
        .chain(discussions.iter_mut().flat_map(|d| d.notes.iter_mut()))
    {
        entry
            .created_at
            .replace_range(..13, &format!("{}T08", &item.updated_at[..10]));
    }
    notes.extend(
        discussions
            .iter()
            .flat_map(|discussion| discussion.notes.iter().cloned()),
    );
    notes.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    let diffs = if key.kind == ItemKind::MergeRequest {
        vec![
        Diff {
            old_path: "src/worker.rs".into(), new_path: "src/worker.rs".into(),
            diff: "@@ -18,7 +18,13 @@ impl Worker {\n     // Fictional demo patch; not code from a company repository.\n     pub fn finish(&mut self) -> Result<()> {\n-        self.queue.flush()?;\n-        self.lease.release();\n+        let pending = self.queue.drain_bounded(MAX_PENDING)?;\n+        for task in pending {\n+            if self.cancelled() {\n+                return Err(Error::Cancelled);\n+            }\n+            self.dispatch(task)?;\n+        }\n+        self.lease.release_after_ack()?;\n         Ok(())\n     }\n".into(),
            ..Diff::default()
        },
        Diff {
            old_path: "tests/recovery.rs".into(), new_path: "tests/recovery.rs".into(), new_file: true,
            diff: "@@ -0,0 +1,10 @@\n+// Synthetic demo test.\n+#[test]\n+fn cancellation_releases_the_lease() {\n+    let mut fixture = Fixture::deterministic();\n+    fixture.pause_before_ack();\n+    fixture.cancel_receiver();\n+    fixture.resume();\n+    assert!(fixture.lease_is_available());\n+    assert_eq!(fixture.pending_tasks(), 0);\n+}\n".into(),
            ..Diff::default()
        },
        Diff {
            old_path: "docs/legacy-polling.md".into(), new_path: "docs/legacy-polling.md".into(), deleted_file: true,
            diff: "@@ -1,4 +0,0 @@\n-# Legacy polling (fictional demo)\n-\n-The old worker polls without a bound.\n-Use the recovery guide instead.\n".into(),
            ..Diff::default()
        },
    ]
    } else {
        Vec::new()
    };
    Details { jobs: jobs(&item), jobs_project: Some(item.key.project), item, notes, discussions, diffs,
        warnings: vec!["DEMO: all projects, people, patches, and logs are fictional. No GitLab instance is contacted.".into()],
        loaded: std::collections::HashSet::from([crate::model::DetailPart::Core,crate::model::DetailPart::Activity,crate::model::DetailPart::Discussions,crate::model::DetailPart::Pipeline,crate::model::DetailPart::Changes]) }
}

/// A bounded cumulative snapshot. Two running job IDs can be polled with the same tick.
pub fn trace(job: u64, tick: u64) -> String {
    let Some(fixture) = find_job(job) else {
        return format!("[DEMO] No synthetic trace fixture for job {job}.\n");
    };
    let mut text = format!(
        "[DEMO — FICTIONAL LOG, NO REMOTE EXECUTION]\nJob {job}: {} / {}\nRunner: demo-runner-{} · isolated synthetic workspace\n",
        fixture.name,
        fixture.stage,
        if fixture.name.ends_with("windows") {
            "windows"
        } else {
            "linux"
        }
    );
    if matches!(
        fixture.status.as_str(),
        "created" | "pending" | "manual" | "skipped" | "canceled"
    ) {
        text.push_str(&format!(
            "Job state: {}. No runner output is available.\n",
            fixture.status
        ));
        return text;
    }
    let steps = if fixture.running() {
        tick.min(40)
    } else {
        tick.min(4)
    };
    text.push_str(
        "Fetching synthetic sources... done\nRestoring deterministic build cache... done\n",
    );
    for step in 0..steps {
        text.push_str(&format!(
            "[00:{:02}] {}: fixture {:02}/40 ... {}\n",
            step + 1,
            fixture.name,
            step + 1,
            if fixture.status == "failed" && step == 3 {
                "FAILED"
            } else {
                "ok"
            }
        ));
    }
    match fixture.status.as_str() {
        "running" => text.push_str("[DEMO] Job is still running; another integration job is running alongside it.\n"),
        "failed" if tick >= 4 => text.push_str("\nFAIL recovery_deadline\n  expected: lease available within 250ms\n    actual: lease held after cancellation barrier\n  hint: inspect the acknowledgement path in the fictional worker patch\nUploading synthetic test report... done\nERROR: Job failed: exit code 1\n"),
        "success" if tick >= 4 => text.push_str("\nAll required checks passed.\nUploading synthetic artifacts... done\nJob succeeded\n"),
        _ => {},
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn deterministic_nested_fictional_dataset() {
        let projects = projects();
        assert_eq!(projects.len(), 5);
        assert!(
            projects
                .iter()
                .all(|p| p.path.split('/').count() >= 5 && p.path.starts_with("demo-lab/"))
        );
        let items = items();
        assert_eq!(items.len(), 40);
        assert_eq!(
            items.iter().map(|i| &i.key).collect::<HashSet<_>>().len(),
            items.len()
        );
        assert!(
            items
                .iter()
                .all(|i| i.labels.len() >= 3
                    && i.web_url.starts_with("https://gitlab.demo.invalid/"))
        );
        assert!(items.iter().any(|i| i.state == "merged"));
        assert!(items.iter().any(|i| i.state == "closed"));
        assert_eq!(
            serde_json::to_string(&items).unwrap(),
            serde_json::to_string(&super::items()).unwrap()
        );
    }

    #[test]
    fn rich_details_and_simultaneous_running_jobs() {
        let item = items()
            .into_iter()
            .find(|i| i.pipeline.as_ref().is_some_and(|p| p.status == "running"))
            .unwrap();
        let details = details(&item.key);
        let running: Vec<_> = details.jobs.iter().filter(|j| j.running()).collect();
        assert_eq!(running.len(), 2);
        assert_ne!(running[0].id, running[1].id);
        assert!(
            details
                .notes
                .windows(2)
                .all(|pair| pair[0].created_at >= pair[1].created_at)
        );
        assert_eq!(details.discussions.len(), 3);
        assert_eq!(details.diffs.len(), 3);
        assert_eq!(
            details.item.unresolved,
            Some(
                details
                    .discussions
                    .iter()
                    .filter(|d| d.notes.iter().any(|n| n.resolvable && !n.resolved))
                    .count()
            )
        );
        for job in running {
            assert_ne!(trace(job.id, 1), trace(job.id, 2));
            assert_eq!(trace(job.id, u64::MAX), trace(job.id, 40));
            assert!(trace(job.id, 10).contains("still running"));
        }
    }

    #[test]
    fn failed_jobs_issues_and_unknown_keys_are_explicit() {
        let items = items();
        let mr = items
            .iter()
            .find(|i| i.pipeline.as_ref().is_some_and(|p| p.status == "failed"))
            .unwrap();
        let details = super::details(&mr.key);
        let failed = details
            .jobs
            .iter()
            .find(|j| j.status == "failed" && !j.allow_failure)
            .unwrap();
        assert!(trace(failed.id, 5).contains("exit code 1"));
        let issue = items
            .iter()
            .find(|i| i.key.kind == ItemKind::Issue)
            .unwrap();
        let issue_details = super::details(&issue.key);
        assert!(issue_details.jobs.is_empty());
        assert!(issue_details.diffs.is_empty());
        assert_eq!(issue_details.item.unresolved, Some(0));
        assert!(
            issue_details
                .discussions
                .iter()
                .flat_map(|d| &d.notes)
                .all(|n| !n.resolvable && !n.resolved)
        );
        for item in &items {
            let details = super::details(&item.key);
            assert!(
                details
                    .notes
                    .iter()
                    .all(|note| note.created_at <= item.updated_at)
            );
        }
        assert!(super::details(&ItemKey::default()).warnings[0].contains("not part"));
        assert!(trace(0, 0).contains("No synthetic trace"));
    }
}
