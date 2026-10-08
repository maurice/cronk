# Project pipelines and the MR pipeline section

Design for filling in the **Pipelines** placeholder in project details, and for tidying up the **Pipeline** / **Jobs** sections of merge request details so both views share one pipeline renderer and one job interaction model.

Status: implemented on the `project-pipelines-design` branch; this document records the design and the deviations noted under **Implementation notes** at the end.

## Goals

- Show a project's recent pipelines regardless of how they were triggered (schedule, push after merge, web, API, trigger, MR event), newest first, like GitLab's **CI/CD → Pipelines** page.
- Reuse the existing job/trace machinery (expand/collapse, live tails, zoom, search, retry) for any pipeline, not just an MR head pipeline.
- Keep the request budget predictable: no per-pipeline fan-out for collapsed rows, no polling for projects nobody is looking at beyond one cheap "latest pipeline" probe.
- Keep project pipelines memory-only. Pipelines are ephemeral and potentially number in the tens of thousands per project; the project pipeline window is never written to the content cache (SQLite). (MR head pipelines are already part of cached `Details` and stay that way, including the extra fields below.)

## Non-goals (for the first iteration)

- Child/downstream pipelines (`/bridges`), manual-job play, pipeline retry/cancel, variables, test reports, coverage.
- Filtering pipelines by ref/source/status/user. The window is small enough to scan.
- A third per-project "pipelines" visibility toggle. Pipelines follow merge request visibility (see below).

## Part 1 — Merge request details: one `Pipeline` section (decided)

Today `sections()` for an MR is `Fields, Description, Pipeline, Jobs, Discussions, Changes`. `Pipeline` is focusable (Tab/Shift+Tab, click), but Enter only enters a scope where ↑/↓ scroll the document, so it is "focusable but inert". `Jobs` is the real interactive section.

**Change:** collapse the two into a single `Pipeline` section whose rows are, in order:

1. Read-only summary rows (same role as the read-only fields in **Fields**):
   - `#<id>  <status dot> <status>   <source> · <ref> · <duration> · by <user>`
   - `Web URL   <url>` (hyperlinked as today)
   - job counts: `◐ 2 running   ○ 1 pending   ● 7 passed   10 total` (only non-zero states, as today)
   - `No pipeline is attached to this merge request.` when there is none
2. One job row per job (plus its expanded log panel), exactly as `Jobs` renders now.

Behaviour:

- **Enter** on the section heading enters the section with the cursor on the first job (index 0, which `sort_jobs` makes the most recently started job). Summary rows are never cursor targets, just as read-only fields are skipped in **Fields**. With zero jobs the section behaves like today's **Jobs** section: Enter enters it, `field` is clamped to 0 and ↑/↓ do nothing; PgUp/PgDn/wheel still scroll the document.
- `config.field` keeps indexing `details.jobs`; the summary rows are not counted.
- Everything else (Space, Enter → `FocusLog`, `z`, `r`, Ctrl+arrows, clicking a job toggles it, `expanded`/`collapsed` persistence by job id) is unchanged.
- Warnings currently attached to `DetailPart::Pipeline` render once at the end of the merged section (today they are duplicated under both headings because `view.rs` maps both to `DetailPart::Pipeline`).
- `Discussions` moves from section index 4 to 3 and `Changes` from 5 to 4. Persisted `config.section` values from older sessions are indices; the restore path clamps them already, so the worst case is landing on a neighbouring section once after upgrade. That is acceptable; no migration.

Touch points for the rename/renumber (all `"Jobs"` string matches become `"Pipeline"`, and hard-coded section indices become lookups by name via `sections().iter().position(..)`):

- `src/ui/mod.rs`: `sections()`, `selected_job`, `detail_target_key`.
- `src/ui/controller.rs`: Activate/`FocusLog` arm, `move_cursor` (`"Jobs" => d.jobs.len()`), the job id re-selection after refresh (~795), `RetryJob` job lookup (~2858), and the hard-coded MR last-section values `5` → `4` in `State::new` (~109) and `prune_tab_routes` (~3174), plus the project `section_last = 2` in `State::new` (~101) which must follow MR-visibility gating; the `r`-key arm (~1785) and the "in the Jobs section" error string (~2865).
- `src/ui/view.rs`: hint table (`section_hint`), the `broad` highlight list (`"Fields" | "Jobs" | "Discussions"`), the `DetailPart` mapping, the job click handler (`Msg::Section(3)`) **and the discussion click handler (`Msg::Section(4)`)**, which would otherwise select **Changes** after the renumbering.
- Tests: `tests/job_logs.rs` and `tests/pointer_feedback.rs` dispatch `Msg::Section(3)` (and `pointer_feedback.rs` `Msg::Section(4)` for Discussions); `tests/ui.rs` opens/asserts `"Jobs"` in several places; `tests/scrollbars.rs` and `tests/status_tooltips.rs` iterate `sections()`.

Status/help strings: the `"Pipeline"` hint becomes the current `"Jobs"` hint (`↑ ↓ choose · Space toggle · Enter logs · z zoom · Ctrl+↑/↓/PgUp/PgDn logs`); the inert `"All running jobs stream together"` hint goes away.

README edits: the key table rows `Space in Jobs`, `r in Jobs`; the detail-document paragraph ("MRs show Fields, Description, Pipeline, Jobs, every Discussion and its notes, and Changes"); the **CI job logs** paragraph ("In Jobs, ↑/↓ choose a job", "before leaving Jobs"); and **Pipelines without tab switching** ("The inline Pipeline section shows the summary; Jobs displays…").

The new summary rows need `source`, `ref`, `duration`, `started_at`/`finished_at`, `user` — fields the MR detail path does not fetch today. See the model changes below; `detail_part(DetailPart::Pipeline)` already fetches the full pipeline object when it falls back from `head_pipeline` to `/merge_requests/:iid/pipelines`, and the `head_pipeline` object embedded in the MR response already contains them, so no extra request is needed.

## Part 2 — Project pipelines

### Data

Extend `model::Pipeline` (serde defaults keep cached MR details decodable):

```rust
pub struct Pipeline {
    pub id: u64,
    pub status: String,
    pub web_url: String,
    pub source: Option<String>,  // push, schedule, merge_request_event, web, api, trigger, pipeline, parent_pipeline, …
    #[serde(rename = "ref")]
    pub ref_name: Option<String>, // "refs/merge-requests/104/head" for MR pipelines
    pub sha: Option<String>,     // moves here from `ApiPipeline`, which currently declares it beside the flattened `Pipeline`
    pub name: Option<String>,    // workflow:name; GitLab sends `null` when unset
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    // Only present on the single-pipeline endpoint and on MR `head_pipeline`:
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub duration: Option<u64>,   // seconds, null while running
    pub queued_duration: Option<u64>,
    pub user: Option<User>,
}
```

Every new field is `Option`: struct-level `#[serde(default)]` only covers *missing* keys, and GitLab sends explicit `null` for e.g. `name`. The `head_pipeline` object is embedded in every MR core response, so a non-nullable field would turn every MR detail into a schema error (which is only retried on explicit refresh). `ApiPipeline.sha` is removed in favour of `Pipeline.sha` (the flattened inner field would otherwise silently default while the outer one consumes the key); `gitlab.rs` `detail_part` adjusts accordingly.

Derived helpers on `Pipeline`:

- `merge_request_iid() -> Option<u64>` parses `refs/merge-requests/<iid>/head|merge`.
- `active()` — `created | waiting_for_resource | preparing | pending | running`.
- `elapsed(now)` — `duration` when known, otherwise `now - started_at.unwrap_or(created_at)` for active pipelines (GitLab's own "in progress" timer does the same). Finished pipelines without a known `duration` (i.e. collapsed rows built from the list endpoint only) show no run time rather than an estimate from `updated_at`, which includes queue time and later status changes.
- `source_label()` — humanised source (`push`, `schedule`, `merge request`, `web`, `api`, `trigger`, `parent pipeline`, …; unknown values pass through).

`dates.rs` currently has only absolute formatters (`format_date`, `format_range*`). A relative formatter `format_relative(timestamp, now) -> String` (`just now`, `2 min ago`, `1 h ago`, `yesterday`, then the absolute date) is added, taking `now` as a parameter so demo snapshots stay deterministic (`just demo --snapshot`).

New in-memory UI state (not persisted, not in `Details`):

```rust
pub struct ProjectPipelines {
    pub pipelines: Vec<Pipeline>,          // newest first, deduplicated by id
    pub pages_loaded: usize,               // 1 after first load
    pub exhausted: bool,                   // last page was short
    pub jobs: HashMap<u64, (Vec<Job>, Duration)>,   // pipeline id → jobs + fetched-at, only for expanded/drilled pipelines
    pub fetched_at: Duration,
    pub warnings: Vec<String>,
}
state.project_pipelines: HashMap<u64 /* project */, ProjectPipelines>
state.latest_pipeline: HashMap<u64 /* project */, Pipeline>   // background probe result
```

Explicit expand/collapse choices for *pipelines* live in `state.pipeline_expanded` / `pipeline_collapsed: HashSet<u64>` and are memory-only for the session. Job choices reuse the existing `expanded` / `collapsed` sets keyed by job id (so they are also persisted, like MR jobs). Traces reuse `state.traces` / `log_views` keyed by job id.

#### Job source abstraction (required before any drill-in work)

Today every piece of job machinery is bound to `state.details: Option<Details>` and `state.config.route: Option<ItemKey>`:

- `load_traces` iterates `details.jobs` and uses `details.jobs_project` / `details.item.key`.
- `Msg::TraceLoaded(key, …)` drops the result unless `config.route == Some(key)`; in project details `route` is `None`.
- `ToggleJob`, `FocusLog`, `ZoomLog` → `selected_job`, `RetryJob` and `log_height` (8 vs 18 lines) all look the job up in `details.jobs`.
- `jobs()` in `view.rs` takes `&Details` and tests `section_name() == "Jobs"`.
- The tick sends `LoadDetails` / `LoadTraces` only when `config.route.is_some()`.

Introduce:

```rust
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum JobSource {
    Item(ItemKey),                                 // MR head pipeline (jobs_project may differ)
    Pipeline { project: u64, pipeline: u64 },      // drilled-in project pipeline
}

impl State {
    /// The jobs the cursor/trace machinery currently operates on, with the project that owns them.
    pub fn job_source(&self) -> Option<JobSource>;
    pub fn current_jobs(&self) -> Option<(u64 /* project */, &[Job])>;
}
```

`selected_job`, `log_height`, `load_traces`, `ToggleJob`, `FocusLog`, `ZoomLog`, `RetryJob` and `jobs()` are rewritten over `current_jobs()`. `TraceLoaded` carries a `JobSource` instead of an `ItemKey` and is accepted when it equals `job_source()` and the epoch matches. `jobs()` takes `&[Job]` plus the selected index instead of `&Details`. This refactor is mechanical and is done as the first step of Phase 3 while MR behaviour is covered by the existing tests.

New messages for the fetch path: `LoadPipelines(project)`, `PipelinesLoaded(project, epoch, page, Result<Vec<Pipeline>>)`, `PipelineExpanded(project, pipeline, epoch, Result<(Pipeline, Vec<Job>)>)`, `LoadOlderPipelines`, `TogglePipeline(id)`, `ProbeLoaded(project, epoch, Result<Option<Pipeline>>)`. The tick gate becomes `config.route.is_some() || state.pipelines_section_open()` for `LoadTraces`, and `LoadPipelines` is scheduled alongside `LoadDetails` when a project route is open.

### GitLab requests

REST only, to stay with the mock-server-tested client. (GraphQL `project.pipelines` would return user/duration/MR link for the whole window in one call; it is a reasonable later optimisation if the per-pipeline detail fetches below turn out to be too chatty, but it needs full project paths and a second schema to mock.)

| Purpose | Request | Fields used |
| --- | --- | --- |
| Window page *n* | `GET /projects/:id/pipelines?order_by=id&sort=desc&per_page=20&page=n` | id, status, source, ref, sha, name, web_url, created_at, updated_at |
| Expanded pipeline detail | `GET /projects/:id/pipelines/:pid` | + user, started_at, finished_at, duration, queued_duration |
| Expanded pipeline jobs | `GET /projects/:id/pipelines/:pid/jobs?include_retried=false` | existing `Job` decoding and `sort_jobs` |
| Traces | existing `trace()` | unchanged |
| Background "latest" probe | `GET /projects/:id/pipelines?order_by=id&sort=desc&per_page=1` | same as window |

All of these are **single requests with explicit `per_page`/`page`**, issued through `get::<Vec<_>>` like `current_iteration`, *not* through `optional()`/`pages()`, which follow `x-next-page`/`Link` until the collection is exhausted and would fetch the whole project history.

The list endpoint deliberately omits `user`/`duration`, so collapsed rows are rendered from list fields only. Detail + jobs are fetched only for pipelines that are **expanded** (auto or explicitly) or **drilled into**, in parallel, bounded to four in flight (same pattern as `load_traces`). With the auto-expansion rule below that is normally one pipeline, so a refresh cycle costs 1 + 2 requests.

Loading older pipelines appends page `pages_loaded + 1`. Hard cap: 5 pages (100 pipelines); the footer row then reads `Older pipelines are on GitLab` with the project's `/-/pipelines` URL. Offset paging is not a snapshot: a pipeline created between two page requests shifts the boundary, so appended pages are deduplicated by id and inserted in id order.

### Refresh cadence

| Situation | What | Cadence |
| --- | --- | --- |
| Project details open, Pipelines section present | page 1; detail + jobs of expanded/drilled pipelines; re-fetch any *active* pipeline already loaded from older pages (bounded 4) | `detail_refresh_secs` (default 10 s), reusing `next_details` |
| Drilled-in pipeline with wanted traces | `load_traces` over that pipeline's jobs | existing 2 s trace cadence |
| Any tab | "latest" probe for every project with `kind_visible(MergeRequest)` **except** the project whose details are open (its page 1 already covers it) | `pipeline_refresh_secs`, default `2 × list_refresh_secs` = 120 s, minimum 30 s |
| Project hidden or MR visibility off | nothing | — |

Older loaded pages are kept while the project detail stays open (including across tab switches via `TabCache`), and dropped when the user goes back to the Projects list or opens another project. Page 1 and the latest probe survive in `project_pipelines` / `latest_pipeline` so re-opening a project shows the last snapshot instantly and then refreshes, following the existing "render the shell, then publish" rule.

Explicit **r / Ctrl+R** refreshes page 1 plus expanded pipelines immediately, same as details.

`pipeline_refresh_secs` is `Option<u64>` in `Config` (the struct is `deny_unknown_fields` with hand-written `Debug`/`PartialEq`; a "2 × list_refresh_secs" default cannot be expressed with `serde(default)`), resolved at use time and validated `>= 30` when set.

Rate-limit/backoff handling reuses `blocked_until` through a `quiet` variant of `network_error` that advances `failures`/`blocked_until` without setting `state.error`; probe failures instead surface in the project row's status tooltip, so a flaky CI endpoint does not nag on every tab. Probes have their own `probe_pending: HashSet<u64>` so one project's probe never overlaps (README: ordinary polls do not overlap for the same resource), and responses carry a dedicated `pipeline_epoch` (bumped on setup replacement and visibility changes, not on every detail open) so stale ones are discarded. `PipelinesLoaded` page 1 also updates `latest_pipeline[project]`, which is why the probe skips the open project.

### Visibility gating (decided)

Pipelines imply code, code implies merge requests. Therefore:

- The **Pipelines** section is present only when `project.kind_visible(ItemKind::MergeRequest)`. Otherwise `sections()` returns `["Fields", "Forget this project"]` and no pipeline requests are made for that project.
- The **Merge requests** visibility field in project **Fields** gains a muted annotation: `[on]  visible · also syncs pipelines` / `[off] hidden · pipelines not synced`.
- README project paragraph gets a sentence to the same effect, and its "Below them is a placeholder for project-wide pipelines" sentence and the `project-details.png` caption ("pipeline placeholder") are updated once Phase 2 lands.

### Rendering

Rows are one, two or three lines following the same "only show what is informative" rule the job summary uses.

Collapsed pipeline (1 line), built from list fields only:

```
▶ ● #48210  main  push  passed   1 h ago
▶ ✖ #48207  !104  merge request  failed   2 h ago
▶ ◐ #48213  main  schedule  running   3m 12s · 2 min ago   nightly
```

- Status dot + status (coloured via `colors.status`, with the existing tooltip).
- `ref`; MR pipelines (`refs/merge-requests/<iid>/…`) show `!<iid>` instead. A hyperlink to that MR is a later nicety.
- `source_label()`.
- Time: `elapsed()` for active pipelines (computed from `created_at`), then `format_relative(updated_at)`. Finished collapsed rows show only the relative time because the list endpoint has no `duration`.
- Trailing `name` when set, muted.

Expanded pipeline (adds up to 2 lines, requires the detail/jobs fetch; shows a muted `loading…` placeholder until then):

```
▼ ◐ #48213  main  schedule  running   3m 12s · 2 min ago   nightly
    by @release-bot  ·  https://gitlab…/-/pipelines/48213
    ◐ 2 running  ○ 1 pending  ● 7 passed   10 total
```

- Line 1 gains the real `duration` once the detail is fetched (finished pipelines then show `8m 02s · 1 h ago`).
- Line 2: started-by (through the existing `UserFormatter`, so **You** stays yellow) and the Web URL (hyperlinked).
- Line 3: job counts, same renderer as the MR summary. When jobs are not loaded yet, this line is `loading jobs…`.
- The `▶ / ▼` toggle marker mirrors the `+ / −` on job headers (`▶` collapsed, `▼` expanded).

Drilled-in pipeline: the expanded rows above, followed by the job rows (indented two columns) rendered by the same `jobs()` function as the MR section. Running and hard-failed jobs auto-expand with live tails, finished jobs collapse unless explicitly expanded; `job_expanded()` / `trace_wanted()` apply unchanged.

Auto-expansion: the **most recent pipeline is expanded by default**; all others are collapsed unless the user expands them. Explicit choices win over the default (`pipeline_collapsed` / `pipeline_expanded`). Unlike jobs, a failed or running older pipeline does *not* auto-expand: a busy project would otherwise open everything.

Footer row (focusable, like the **Forget this project** button is a row):

```
  Show 20 older pipelines…            (20 of ? loaded)
```

Empty and error states:

- No pipelines: `No pipelines yet for this project.`
- First load pending: the same skeleton rows as detail sections (`Loading Pipelines…` + shimmer).
- Request failure after a successful load: keep the last data, add `⚠ Pipelines may be stale: <error>` beneath the rows, consistent with detail warnings.

Height: the fixed `Length::Px(7)` placeholder goes away; the section is `Length::Auto` and the `overflows` estimate in the project detail view is replaced by the real row count. The `broad` highlight rule in `project_detail` (`scope == Details || section == "Pipelines"`) loses its `Pipelines` special case once the section is interactive, so Enter contracts the highlight to the cursor row like the other sections.

### Navigation and keys (decided: nested drill-in)

Introduce a fourth level beneath `Scope::Section` for the Pipelines section only. Conceptually the cursor is on one of: a **pipeline** row, the **footer**, or a **job** inside the drilled-in pipeline; `drilled: Option<u64>` (the pipeline id, persisted as `TabState.drilled`) distinguishes the last case.

`config.field` indexes pipelines (`0..n`) plus the footer (`n`) while `drilled.is_none()`, and indexes the drilled pipeline's jobs while `drilled.is_some()`.

Persistence: `section`/`field` live in `config::TabState` (SQLite navigation, `deny_unknown_fields`), not in `navigation::Selection`. `drilled: Option<u64>` is added to `TabState` with `#[serde(default)]`; older binaries will reject snapshots containing it, which is the existing contract for new navigation fields. Two existing rules change:

- `prune_tab_routes` currently resets `section`/`section_cursor` to 0 for every project route. It instead clamps to the project's section count derived from `kind_visible(MergeRequest)` (it is a free function over `&mut Config`) (so `Pipelines` can be restored) and keeps `field`/`drilled`.
- Because the pipeline window is memory-only, the restored cursor is provisional: after the first `PipelinesLoaded`, the controller re-aligns it (same approach as the job id re-selection after an MR refresh): if `drilled` is not in the window, clear `drilled` and clamp `field` to the pipeline list; if it is, clamp `field` to that pipeline's jobs once they arrive. Until then the section shows the loading skeleton with the heading selected.
- Toggling **Merge requests** visibility off from **Fields** shrinks `sections()` from 3 to 2 entries. `ToggleProjectKind` re-clamps `section_cursor`/`config.section`/`field` and clears `drilled`; `navigation::context()` already hashes `merge_requests_visible`, so a stale restore is not possible.

| Key | On the Pipelines heading (Scope::Details) | On a pipeline row | On the footer | On a job (drilled) |
| --- | --- | --- | --- | --- |
| Enter | enter section, cursor on newest pipeline | drill in: expand if needed, cursor on job 0 (most recently started per `sort_jobs`) | load next page; cursor stays on the footer | focus logs (`FocusLog`) |
| Space | — | toggle expanded | — | toggle job expanded |
| Tab / ↓, Shift+Tab / ↑ | next/previous section | next/previous pipeline; ↓ past the last pipeline lands on the footer | ↑ returns to the last pipeline; ↓ is a no-op | next/previous job within the pipeline (no spill-over into other pipelines) |
| Home / End | first/last section | newest pipeline / footer | — | first/last job |
| Esc | back to the Projects list | back to section navigation | back to section navigation | back to the pipeline row (still expanded) |
| z, r, Ctrl+↑/↓, / | — | — | — | as in the MR Pipeline section |
| Mouse | click heading selects section | click row selects and toggles (like jobs) | click loads | click job toggles; drills automatically |

The `Esc` chain from logs is therefore: search → zoom → log focus → pipeline row → sections → list. `Msg::Back` gains one arm; `move_cursor`/`select` gain a `"Pipelines"` arm that picks the right length.

`detail_target_key()` returns `pipeline-<id>` / `pipelines-older` / `job-<id>` so `reveal_content` scrolling keeps working.

Header hints (`view.rs` section hint table):

- Pipelines heading: `Enter opens the newest pipeline · ↑ ↓ choose section`
- Pipeline rows: `↑ ↓ choose a pipeline · Space expands · Enter opens jobs`
- Drilled jobs: the merged `Pipeline` section hint from Part 1.

### Projects list indicator

The background probe populates the third row of each project list entry, after the open-work counts:

```
 ● Orbit scheduler · DEMO
   demo-lab/constellation/platform/runtime/orbit-scheduler  ·  visible
   3 open issues · 3 open merge requests  ·  ◐ pipeline running · main · 2 min ago
```

Hidden projects and projects with MR visibility off show no pipeline fragment. The dot uses the same status colours and tooltip text as elsewhere. This is also what makes the Dashboard-style "something is red" scan possible from the Projects tab without opening each project.

### Demo mode

`demo.rs` gains per-project pipeline fixtures: 20–25 pipelines per project mixing sources (`schedule`, `push`, `merge_request_event`, `web`), statuses, a running one at the top for two projects, a failed one a few rows down, one pipeline with `name`, and several pages worth for one project so the footer is exercised. MR head pipelines already exist and should be reused by id so the MR view and the project view agree. Demo traces reuse the existing simulated live tail.

Gallery/snapshot: add `docs/project-pipelines.png` and a drilled-in capture to the README.

### Tests

- `tests/projects.rs`: section list with MR visibility on/off; annotation text; placeholder gone.
- New `tests/pipelines.rs` (mock server, like `tests/projects.rs`): first page request parameters; collapsed row text (status, ref, source, MR iid, relative time); newest auto-expanded and fetches detail + jobs; explicit collapse/expand survives refresh; footer appends page 2, dedupes an overlapping id, caps at 5 pages; refresh re-fetches only page 1 + active loaded pipelines; nothing requested for MR-hidden projects; probe request `per_page=1` and project row fragment; probe failure does not set `state.error`.
- `tests/ui.rs`: MR sections are `Fields, Description, Pipeline, Discussions, Changes`; Enter on Pipeline lands on the first job; clicking a discussion selects Discussions (not Changes); Esc chain from a drilled job; `detail_target_key` values; navigation restore of `drilled` + `field` including the re-align after load; MR visibility toggle re-clamps the cursor.
- Unit: `Pipeline::merge_request_iid`, `elapsed`, `source_label`, `format_relative`; `Pipeline` decodes `name: null` and a minimal legacy `{id,status,web_url}` object (cached details).

### Implementation phases

1. **MR merge** — model fields, merged `Pipeline` section, string/index fixes, README. Small, independently shippable, unblocks the shared renderer.
2. **Read-only project pipelines** — `ProjectPipelines` state, page-1 fetch at detail cadence, visibility gating + annotation, collapsed rows, newest auto-expanded with detail + jobs summary, demo fixtures, tests.
3. **Interaction** — `JobSource` refactor first (MR behaviour unchanged, existing tests green), then nested cursor, drill-in to jobs with traces, Space/Enter/Esc, mouse, footer paging, `TabState.drilled` + `prune_tab_routes` change and post-load re-alignment.
4. **Probe + list indicator** — `pipeline_refresh_secs` (`Option<u64>`), quiet backoff, `probe_pending`, project row fragment, README config sample and request-budget paragraph (probe cadence; per open project page 1 + detail + jobs per cycle), README navigation-persistence sentence (`drilled`).

Each phase is its own branch/PR per the project workflow.

## Open questions

- Should drilling into a pipeline from the Projects tab remember per-pipeline job collapses in SQLite (as MR jobs do), or is session memory enough given pipelines are ephemeral? Proposal: reuse the existing persisted `collapsed`/`expanded` sets because the code path is shared and the cost is a few integers.
- MR-event pipelines: show `!104` as a hyperlink to the MR in Cronk (route change) or only to GitLab? Proposal: GitLab URL first; in-app route later.
- Should the probe also feed the Dashboard (e.g. "default-branch pipeline failed on project X")? Out of scope here, but the data would be available.

## Implementation notes

Deviations from the plan above, as built:

- `drilled` is **not persisted** to SQLite. It lives in `State` and survives tab switches through `TabCache`, but a restart reopens the project on its first section as before (`prune_tab_routes` is unchanged). Job expand/collapse choices are persisted as for MR jobs.
- The window is kept **newest-first by `created_at`** (then id) rather than by id alone, so the demo's merge request head pipelines (which have lower synthetic ids) interleave correctly; for real GitLab data the two orders coincide.
- The per-pipeline detail + jobs fetch runs for expanded pipelines that are active or not yet loaded, bounded to four per cycle.
- `Msg::ProjectDetailViewport` now carries the reveal target at render time and ignores viewport events measured for a superseded target, mirroring the issue/MR detail. Without this, rapid Enter/Enter on a long project document cancelled the pending reveal (pipelines made the document long enough to expose it).
- `State.saved_config` and `State.onboarding` are boxed, and `.cargo/config.toml` sets `RUST_MIN_STACK` for test binaries: tui-lipan's recursive layout keeps `State`/`Config`-sized frames in debug builds and a merge request detail already needed ~2.1 MiB, just above libtest's 2 MiB default, so the suite was one field away from stack overflow before this feature.
- Demo fixtures: project 9001 has 45 synthetic pipelines plus its 4 MR heads (three window pages); other projects fit on one short page. MR head pipelines carry source/ref/user/duration so the merged MR Pipeline summary shows them.
