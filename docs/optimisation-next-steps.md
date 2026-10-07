# Remaining optimisation work

**Already merged:** viewport-windowed lists, efficient SQLite cache batches, local
navigation in SQLite with coalesced background writes, and pinned
`ubuntu-26.04` CI runners. Do not redo these. See
[measured findings](large-list-performance.md) and
[navigation design](navigation-state.md).

This is a plan of at most five small PRs, in order. PRs 3–5 are **gated on the
numbers from PR 1**; if the evidence does not show the cost matters, close that
item instead of implementing it.

## What was checked against the code

| Claim | Verdict |
| --- | --- |
| Old `UserLoaded` can overwrite a newly validated user. | **True.** `Msg::UserLoaded` (`src/ui/controller.rs`) carries no epoch/generation. `save_setup` (`src/ui/onboarding.rs`) swaps `self.api`, sets `state.user` and bumps `list_epoch`, but neither invalidates an in-flight `LoadUser` nor clears `user_pending`. `Tick` issues `LoadUser` whenever `user.id == 0`, so the realistic trigger is a broken URL/token whose request is still pending when the user opens setup and fixes it. A late `Ok` replaces the validated user (wrong `@me`/dashboard predicates); a late `Err` calls `network_error`, which sets `blocked_until` (up to 320 s) and so stalls the new client. |
| Filter work is repeated for unchanged navigation. | **True.** One `Move` in the list runs `visible_item_count()` (filter), `persist` → `remember_selection` → `visible_items()` (filter + sort), then the render does a header `visible_item_count()` and `main_list` `visible_items()` (filter + sort): about four filter passes and two sorts per keypress at N items. Each pass is O(N) with no caching. |
| Text filtering allocates. | **True, only with a query.** `filter.rs::contains/equal/user_matches` call `to_lowercase()` on the haystack for every term and item (title *and* description, labels, usernames, iteration). An empty query allocates nothing. |
| Dashboard sort recomputes ranks. | **True but small.** `sort_by` calls `attention_rank` twice per comparison on tab 0 (N log N calls); the function is cheap and branch-only. |
| Project lookup/kind scan per item. | **True but negligible.** `State::project` is a linear scan of the configured projects (tens), `kind()` is cheap. Not a PR on its own. |
| Progressive sync merges the whole collection per page. | **True.** `merge_items` (once per 100-item page, plus once for the cached snapshot) moves all items through a `HashMap`, then runs `visible_items()` twice (selection capture and re-find), `restore_selection` and `normalize`: O(N) per page, O(N·pages) per cold sync. Each page also sends a second progress message with an empty item list (`saving` phase), which `merge_items` skips but still marks the UI dirty. Order after the merge is arbitrary (`into_values`); harmless because the sort is total. |
| `persist` saves TOML during scrolling (`large-list-performance.md` "Next profiling priorities" #1). | **Stale.** `persist` writes `Config::save` only when *portable* config changed (`portable_eq`); navigation goes to the bounded SQLite writer (see `navigation-state.md`). Drop this priority and correct that section in PR 1. |
| SQLite/HTTP mutex contention, WAL, pool. | **Unsupported.** Scrolling does no SQLite access; page writes were already batched and moved off the lock. No wait/hold evidence exists. Deferred. |
| Overscan tuning; index-captured row callbacks. | **Unsupported.** Rows are windowed to ≤ 60 rich rows; row callbacks use global indices by design and click identity is already covered by `tests/list_performance.rs`. Deferred. |
| Detail virtualisation / memory profiling. | **Unsupported.** No measurement; large and speculative. Deferred. |
| Node-20 action warnings / runner pinning. | **Done or out of scope.** Warnings were addressed in `a4881ea`, runners pinned in #26, and Dependabot now scans `.github/actions/setup`. Not part of the optimisation plan. |

Caveat: the earlier "1.609 s" result is **30 debug headless updates** (about
54 ms per update, unoptimised), not one input and not production FPS.

## PR 1 — Phase-separated latency harness (measurement only)

**Scope.** Extend the informational headless workload so a maintainer can
reproduce per-gesture latency in release mode with realistic data. No production
behaviour change.

- Record, per gesture (`Move`, `Select` deep jump, wheel, scrollbar drag, tab
  switch, text-query edit), the samples and median / p95 / max for: `dispatch`
  (update), `render` (view + layout), `pump`, and total.
- Fixtures: 3,000 MRs with description-heavy bodies (≈2 KB, some non-ASCII),
  several labels/assignees; scenarios: empty query, one text term, `@me`
  assignee filter, dashboard tab (tab 0), and a path-backed `Cronk`
  (temp workspace, persistence enabled) versus `path: None`.
- A separate progressive-sync scenario: dispatch 30 `ProjectProgress` messages
  of 100 items each (pre-built, no network) and record per-message update time
  plus total.
- Correct `large-list-performance.md` ("persist saves TOML" is stale) and add
  the measured table to this document.

**Files/areas.** `tests/latency_harness.rs` (new; the existing
`tests/list_performance.rs` is untouched); `docs/large-list-performance.md`, this
doc. The full workload is `#[ignore]`d; run it with
`just test --release --test latency_harness -E 'test(latency_harness_release)' --run-ignored only --test-threads=1 --success-output immediate`
(a release build is a task-specific need here; AGENTS.md otherwise forbids it).
A small debug-mode `latency_harness_smoke` runs in the normal test suite so the
harness stays working.

**Acceptance.**
- Prints all raw samples and the median/p95/max table; no timing assertions.
- Existing correctness assertions in the file are unchanged and still pass in
  the normal debug `just test --test list_performance`.
- PR description records host, profile, warm-up, run order and limitations
  (headless; no real terminal write time).
- States which of PRs 3–5 the data justifies, with the numbers.

**Out of scope.** Any optimisation, CI timing thresholds, real-terminal output
timing, live-GitLab runs, benchmarking framework dependencies.

### PR 1 results (3,000 MRs, release)

**Method.** Host: Linux/WSL2 `6.18.33.2-microsoft-standard-WSL2`, Intel Core
i7-1185G7 (8 logical CPUs), 7.7 GiB guest RAM; Rust 1.99.0 `--release` (thin LTO),
`CARGO_BUILD_JOBS=2`, `NEXTEST_TEST_THREADS=2`, one test thread. Built on
`origin/main` `4afd053`. The binary was prebuilt, the host load settled
(load average ≈ 3 → 2) and nothing else ran during measurement. One full unmeasured
run came first, then three measured runs back to back; the table is **run 1**
(chosen before looking at the numbers), and the last column lists each run's total
median to show run-to-run noise. Every gesture also does one unmeasured warm-up pass
of the same steps (so e.g. `Move` samples are moves 31–60). Scenarios run in the
order empty → text → `@me` → dashboard, each `path: None` then path-backed, then
the two sync scenarios. Fixture: 3,000 MRs over 5 projects, mean description
1,969 bytes (1,000 items contain non-ASCII text), 3–4 labels, up to 2 assignees and
2 reviewers, pseudo-random `updated_at` order, ≈ 1/3 assigned to the demo user.
`handoff` matches 10% of items (near the end of the description). 100×68 viewport
(20 visible rows). All values are **ms**, as median unless stated.

**Limitations.** Headless: no real terminal output. `dispatch` is
`TestBackend::update_level` (update only). For wheel and scrollbar-drag the only
available entry point is `send_mouse`, so their `dispatch` also contains input
routing and the backend's own render/pump, and their totals contain about two
frames; compare those gestures only with each other. The 15/15-step wheel median
mixes down and up scrolls. `render` is view + layout and cannot be split further
here. Tab-switch samples alternate between the two directions (the other tab is the
empty-query dashboard or MR tab). "Text-query edit" is a `Submit` of eight different
queries ending on the scenario's own query. Path-backed uses demo mode with a real
temporary workspace (so the background navigation SQLite writer and synchronous
`Config::save` are live) but no GitLab client. Small-n p95/max are the same sample.
The harness renders and pumps unconditionally after every dispatch (in `sample`),
even when the message did not make the UI dirty, so a no-op message such as an empty `saving` report is measured with a
full frame that the real event loop might skip; its 2–3 ms is therefore an upper
bound on the real cost of those reports.
Raw per-sample microseconds for every phase are printed by the test (`RAW |` lines).

| Scenario | Gesture | n | dispatch med | render med | pump med | total med / p95 / max | total med, runs 1 / 2 / 3 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| empty query (MR tab) · path: None | Move(1) | 30 | 0.637 | 4.767 | 0.180 | 5.576 / 8.311 / 8.794 | 5.576 / 8.317 / 5.800 |
| empty query (MR tab) · path: None | Select deep jump | 8 | 0.651 | 5.384 | 0.159 | 6.259 / 9.154 / 9.154 | 6.259 / 8.204 / 5.338 |
| empty query (MR tab) · path: None | wheel (15 down, 15 up) | 30 | 5.755 | 4.900 | 5.092 | 16.820 / 23.854 / 24.331 | 16.820 / 15.984 / 15.454 |
| empty query (MR tab) · path: None | scrollbar drag | 18 | 7.338 | 5.583 | 0.159 | 13.609 / 21.206 / 21.206 | 13.609 / 11.536 / 10.696 |
| empty query (MR tab) · path: None | tab switch (3↔0) | 10 | 1.801 | 3.521 | 0.184 | 5.446 / 8.936 / 8.936 | 5.446 / 6.642 / 4.908 |
| empty query (MR tab) · path: None | text-query edit (Submit) | 8 | 2.184 | 7.160 | 1.902 | 11.668 / 17.057 / 17.057 | 11.668 / 11.667 / 12.291 |
| empty query (MR tab) · path-backed | Move(1) | 30 | 0.727 | 5.599 | 0.195 | 6.605 / 9.110 / 9.462 | 6.605 / 5.546 / 6.636 |
| empty query (MR tab) · path-backed | Select deep jump | 8 | 1.420 | 10.805 | 0.308 | 12.299 / 17.587 / 17.587 | 12.299 / 6.636 / 6.187 |
| empty query (MR tab) · path-backed | wheel (15 down, 15 up) | 30 | 5.548 | 4.415 | 4.403 | 15.867 / 19.031 / 21.565 | 15.867 / 15.383 / 15.754 |
| empty query (MR tab) · path-backed | scrollbar drag | 18 | 5.936 | 4.199 | 0.110 | 10.314 / 14.892 / 14.892 | 10.314 / 12.469 / 15.258 |
| empty query (MR tab) · path-backed | tab switch (3↔0) | 10 | 2.128 | 5.412 | 0.242 | 7.709 / 11.088 / 11.088 | 7.709 / 5.846 / 12.503 |
| empty query (MR tab) · path-backed | text-query edit (Submit) | 8 | 6.189 | 6.700 | 1.660 | 14.648 / 19.966 / 19.966 | 14.648 / 15.508 / 17.538 |
| text term `handoff` (MR tab) · path: None | Move(1) | 30 | 4.246 | 9.813 | 2.234 | 16.700 / 23.109 / 26.814 | 16.700 / 14.593 / 16.480 |
| text term `handoff` (MR tab) · path: None | Select deep jump | 8 | 3.454 | 7.951 | 1.712 | 13.401 / 16.472 / 16.472 | 13.401 / 14.200 / 16.230 |
| text term `handoff` (MR tab) · path: None | wheel (15 down, 15 up) | 30 | 12.695 | 8.163 | 10.471 | 32.646 / 38.980 / 39.655 | 32.646 / 65.275 / 33.109 |
| text term `handoff` (MR tab) · path: None | scrollbar drag | 18 | 18.856 | 15.527 | 3.015 | 35.980 / 62.819 / 62.819 | 35.980 / 29.118 / 30.454 |
| text term `handoff` (MR tab) · path: None | tab switch (3↔0) | 10 | 8.851 | 9.313 | 1.461 | 20.649 / 35.017 / 35.017 | 20.649 / 11.409 / 10.621 |
| text term `handoff` (MR tab) · path: None | text-query edit (Submit) | 8 | 5.040 | 18.695 | 3.989 | 25.051 / 45.356 / 45.356 | 25.051 / 13.008 / 12.163 |
| text term `handoff` (MR tab) · path-backed | Move(1) | 30 | 7.069 | 15.079 | 3.351 | 25.320 / 38.004 / 41.680 | 25.320 / 14.282 / 17.382 |
| text term `handoff` (MR tab) · path-backed | Select deep jump | 8 | 4.256 | 9.574 | 2.206 | 15.023 / 19.663 / 19.663 | 15.023 / 13.694 / 32.466 |
| text term `handoff` (MR tab) · path-backed | wheel (15 down, 15 up) | 30 | 12.945 | 8.492 | 11.109 | 33.220 / 49.473 / 77.453 | 33.220 / 34.279 / 45.574 |
| text term `handoff` (MR tab) · path-backed | scrollbar drag | 18 | 16.239 | 10.670 | 2.302 | 29.170 / 42.022 / 42.022 | 29.170 / 24.181 / 22.647 |
| text term `handoff` (MR tab) · path-backed | tab switch (3↔0) | 10 | 7.279 | 7.768 | 1.222 | 16.227 / 26.695 / 26.695 | 16.227 / 11.723 / 10.809 |
| text term `handoff` (MR tab) · path-backed | text-query edit (Submit) | 8 | 8.326 | 9.747 | 2.748 | 20.484 / 33.043 / 33.043 | 20.484 / 16.183 / 17.361 |
| `assignee:@me` (MR tab) · path: None | Move(1) | 30 | 0.856 | 6.151 | 0.545 | 8.080 / 13.953 / 16.755 | 8.080 / 7.959 / 5.956 |
| `assignee:@me` (MR tab) · path: None | Select deep jump | 8 | 0.783 | 5.548 | 0.556 | 7.271 / 10.931 / 10.931 | 7.271 / 8.191 / 6.214 |
| `assignee:@me` (MR tab) · path: None | wheel (15 down, 15 up) | 30 | 14.992 | 10.892 | 10.933 | 36.092 / 47.146 / 47.456 | 36.092 / 23.147 / 17.300 |
| `assignee:@me` (MR tab) · path: None | scrollbar drag | 18 | 12.928 | 9.595 | 0.776 | 22.797 / 45.364 / 45.364 | 22.797 / 13.655 / 11.038 |
| `assignee:@me` (MR tab) · path: None | tab switch (3↔0) | 10 | 1.967 | 4.014 | 0.409 | 6.238 / 13.810 / 13.810 | 6.238 / 9.909 / 5.299 |
| `assignee:@me` (MR tab) · path: None | text-query edit (Submit) | 8 | 3.927 | 16.148 | 2.954 | 22.031 / 38.863 / 38.863 | 22.031 / 16.247 / 12.139 |
| `assignee:@me` (MR tab) · path-backed | Move(1) | 30 | 1.247 | 8.488 | 0.708 | 10.111 / 17.944 / 18.001 | 10.111 / 8.583 / 10.015 |
| `assignee:@me` (MR tab) · path-backed | Select deep jump | 8 | 2.154 | 13.930 | 1.097 | 17.163 / 24.026 / 24.026 | 17.163 / 7.457 / 5.483 |
| `assignee:@me` (MR tab) · path-backed | wheel (15 down, 15 up) | 30 | 8.630 | 6.074 | 6.923 | 22.345 / 28.617 / 33.620 | 22.345 / 36.976 / 23.478 |
| `assignee:@me` (MR tab) · path-backed | scrollbar drag | 18 | 7.032 | 5.518 | 0.544 | 12.875 / 18.130 / 18.130 | 12.875 / 18.606 / 20.306 |
| `assignee:@me` (MR tab) · path-backed | tab switch (3↔0) | 10 | 1.692 | 3.112 | 0.321 | 5.058 / 9.015 / 9.015 | 5.058 / 7.915 / 15.915 |
| `assignee:@me` (MR tab) · path-backed | text-query edit (Submit) | 8 | 6.155 | 7.514 | 1.706 | 15.710 / 18.561 / 18.561 | 15.710 / 14.357 / 14.940 |
| dashboard (tab 0) · path: None | Move(1) | 30 | 0.764 | 5.118 | 0.249 | 6.323 / 8.811 / 10.966 | 6.323 / 5.588 / 6.569 |
| dashboard (tab 0) · path: None | Select deep jump | 8 | 0.797 | 4.352 | 0.263 | 5.460 / 8.600 / 8.600 | 5.460 / 5.582 / 4.602 |
| dashboard (tab 0) · path: None | wheel (15 down, 15 up) | 30 | 5.632 | 4.343 | 4.613 | 15.560 / 19.484 / 19.729 | 15.560 / 17.374 / 15.664 |
| dashboard (tab 0) · path: None | scrollbar drag | 18 | 5.667 | 4.688 | 0.282 | 10.617 / 14.674 / 14.674 | 10.617 / 14.420 / 10.744 |
| dashboard (tab 0) · path: None | tab switch (0↔3) | 10 | 1.785 | 3.765 | 0.178 | 6.036 / 7.371 / 7.371 | 6.036 / 5.727 / 5.452 |
| dashboard (tab 0) · path: None | text-query edit (Submit) | 8 | 1.452 | 5.462 | 1.168 | 8.279 / 10.284 / 10.284 | 8.279 / 8.296 / 9.780 |
| dashboard (tab 0) · path-backed | Move(1) | 30 | 0.893 | 5.324 | 0.292 | 6.800 / 8.698 / 9.378 | 6.800 / 5.674 / 5.794 |
| dashboard (tab 0) · path-backed | Select deep jump | 8 | 0.756 | 4.754 | 0.262 | 5.790 / 8.952 / 8.952 | 5.790 / 6.004 / 5.997 |
| dashboard (tab 0) · path-backed | wheel (15 down, 15 up) | 30 | 6.027 | 4.873 | 4.943 | 16.749 / 31.814 / 32.671 | 16.749 / 16.327 / 16.250 |
| dashboard (tab 0) · path-backed | scrollbar drag | 18 | 6.209 | 4.865 | 0.295 | 11.559 / 15.554 / 15.554 | 11.559 / 12.936 / 12.880 |
| dashboard (tab 0) · path-backed | tab switch (0↔3) | 10 | 2.416 | 4.694 | 0.238 | 7.437 / 9.800 / 9.800 | 7.437 / 6.908 / 14.544 |
| dashboard (tab 0) · path-backed | text-query edit (Submit) | 8 | 5.670 | 5.079 | 1.056 | 11.808 / 16.301 / 16.301 | 11.808 / 13.980 / 11.652 |
| progressive sync, tab 3, 3000 items | ProjectProgress page (100 items) | 30 | 0.975 | 4.121 | 0.082 | 5.360 / 7.789 / 8.199 | 5.360 / 4.995 / 5.094 |
| progressive sync, tab 3, 3000 items | ProjectProgress `saving` (empty) | 30 | 0.003 | 3.131 | 0.001 | 3.136 / 5.742 / 6.218 | 3.136 / 3.211 / 2.988 |
| progressive sync, tab 3, 3000 items | all messages | 60 | 0.027 | 3.559 | 0.007 | 3.777 / 7.209 / 8.199 | 3.777 / 3.651 / 3.662 |
| progressive sync, tab 0, 3000 items | ProjectProgress page (100 items) | 30 | 0.996 | 2.948 | 0.117 | 4.033 / 7.601 / 7.789 | 4.033 / 4.312 / 4.149 |
| progressive sync, tab 0, 3000 items | ProjectProgress `saving` (empty) | 30 | 0.003 | 2.346 | 0.001 | 2.350 / 4.659 / 4.901 | 2.350 / 2.702 / 2.396 |
| progressive sync, tab 0, 3000 items | all messages | 60 | 0.028 | 2.721 | 0.006 | 3.165 / 5.665 / 7.789 | 3.165 / 3.226 / 3.543 |

**Sync scenario.** 30 pages × 100 items (3,000 total) round-robin over 5 projects,
each page preceded by the empty `saving` report that `sync_project` sends, then
rendered and pumped. Wall time for the 60 messages: 233–255 ms (tab 3) and
203–220 ms (tab 0) over the three runs.

**What the data says.**

- *Text queries* are the only filter case that matters: `Move` totals 14.6–16.7 ms
  across the three runs versus 5.6–8.3 ms for the empty query (run 1: 16.7 vs 5.6, with
  `dispatch` 0.6 → 4.2 ms and `render` 4.8 → 9.8 ms; p95 23 ms, i.e. above a 60 Hz
  frame). The extra ≈ 9–11 ms is roughly five passes over 3,000 items with 2 KB
  lowercase-allocating descriptions (≈ 1.2 ms per pass, inferred from the update/render
  difference, not measured per pass). `assignee:@me` (6.0–8.1) and the dashboard with
  its repeated `attention_rank` sort (5.6–6.6) are within the empty query's
  run-to-run spread.
- With no text term the update is 0.6–0.9 ms and the 5 ms floor is `render`
  (view + layout of ≤ 60 rich rows), not filter or sort.
- Path-backed navigation (the stale "persist saves TOML" concern) is **not** visible
  for `Move`/`Select`: totals are within the run-to-run spread of `path: None`.
  The one real cost is a text-query edit, whose `dispatch` (run 1) is 5.7–8.3 ms
  with a path versus 1.5–5.0 ms without. The difference, 2.2–4.2 ms by scenario
  (≈ 2–4 ms), is attributed to the synchronous `Config::save` (TOML + fsync) on a
  portable-config change, which is rare. It is inferred from that difference, not
  timed directly.
- Progressive sync: the update for a 100-item page grows with the collection (the
  O(N·pages) merge). Table medians are 0.98 ms (tab 3) and 1.00 ms (tab 0) per page.
  First-page and last-page figures are not in the table; they are from two later
  reruns on a quiet host (load average ≈ 1.6), read from the `RAW` lines: the first
  page's `dispatch` is 0.03–0.06 ms and the 30th (2,900 → 3,000 items) is
  2.0–3.8 ms, the single last sample, so noisy. The render after each message
  dominates: median page total 5.4 ms (tab 3) / 4.0 ms (tab 0), max 8.2 / 7.8 ms,
  of which `render` is 4.1 / 2.9 ms. The empty `saving` reports have a 0.003 ms
  update but a median total of 3.1 / 2.3 ms, all of it a full redraw (see below).

**Gate verdicts.**

- **PR 3: justified, narrowed.** Keep the ASCII fast path for text matching (the
  measured 11 ms/gesture text-query cost). Cutting repeated filter passes per event
  is justified only as a multiplier on that same cost. The precomputed dashboard
  rank sort is **not** justified (dashboard ≈ empty query); drop it unless a later
  profile disagrees.
- **PR 4: not justified at 3,000 items.** Update time stays ≤ 3.5 ms per page and the
  per-message cost is dominated by rendering; no hitch is visible. Close it, or
  re-measure with a much larger collection first. (Skipping the redraw for empty
  progress reports would need a status-bar redraw decision and is not part of the
  data's ask.)
- **PR 5: not justified.** Filter + sort is not a dominant cost for any non-text
  scenario, and the text scenario is addressed by PR 3. Re-run the harness after
  PR 3; only if text-query `Move` totals remain well above the empty query should
  it be reconsidered.

## PR 2 — Ignore stale `UserLoaded` results

**Scope.** A correctness fix, independent of PR 1's data; can land first if
convenient.

- Add a user-request generation to `State` (e.g. `user_epoch`), bumped in
  `save_setup` (where `self.api` and `state.user` are replaced) and reset
  `user_pending = false` there.
- `LoadUser` captures the generation; the result message carries it.
  `UserLoaded` with a stale generation is dropped entirely: no `state.user`
  write, no `network_error`/backoff, no `restore_selection`/`persist`, and no
  clearing of the new request's `user_pending`.
- `Msg::UserLoaded` is a `pub` variant and `tests/navigation.rs` dispatches
  `Msg::UserLoaded(Ok(demo::user()))` directly (lines ~104, 224, 258). Either
  keep that variant as "current generation" and have `LoadUser` send a new
  generation-tagged message, or update those call sites; prefer the former to
  keep the diff small.

**Files/areas.** `src/ui/mod.rs` (`Msg`, `State`), `src/ui/controller.rs`
(`LoadUser`, `UserLoaded`), `src/ui/onboarding.rs` (`save_setup`), plus tests
alongside `onboarding.rs` tests.

**Acceptance.**
- Test: start a `LoadUser`, complete setup with a different validated user,
  then deliver the old `Ok(user)`; `state.user` is unchanged.
- Test: late stale `Err` leaves `blocked_until`, `failures` and `error` unchanged.
- Test: the current-generation `Ok` still sets the user and restores a pending
  `@me`/dashboard selection (existing late-authentication tests still pass).
- `just validate` passes.

**Out of scope.** Epochs for other messages (detail/project/list already have
them), changing backoff policy, token or credential storage changes.

## PR 3 — Allocation-free filtering and one evaluation per event

**Gate:** PR 1 shows filter/sort is a material share of update or render time
for a text/`@me` query or the dashboard tab.

**Scope.** Cut the constant factors without introducing a cache.

- In `filter.rs`, add a fast path for `contains`/`equal`/`user_matches`: when
  haystack and needle are ASCII, compare with `eq_ignore_ascii_case` windows;
  otherwise fall back to the current `to_lowercase()` code. Results must be
  identical, including non-ASCII (e.g. final-sigma `Σ`, `İ`) which keeps using
  the old path.
- Sort the dashboard with precomputed ranks (decorate-sort or
  `sort_by_cached_key`) instead of two `attention_rank` calls per comparison.
- Remove avoidable repeat passes per event: `remember_selection` and
  `merge_items` should not run a full filter + sort to find one identity
  (add a `selected_item`/`position_of` helper that filters once and, for the
  position, counts sort-predecessors without allocating the sorted vector, or
  reuse one computed view within the call).

**Files/areas.** `src/filter.rs` (and its tests), `src/ui/mod.rs`
(`matching_items`, `visible_items`, `selection_index`, `remember_selection`),
`src/ui/controller.rs` (`merge_items`, `move_selection`).

**Acceptance.**
- Existing filter tests pass; new differential tests compare the fast path with
  the old implementation over ASCII, mixed-case, non-ASCII, empty, and
  multi-term inputs.
- Selection identity/restore tests in `tests/navigation.rs` and `tests/ui.rs`
  still pass unchanged.
- Release harness (PR 1) before/after table for the affected scenarios, all
  samples reported.

**Out of scope.** A revision-keyed derived-view cache (PR 5), normalised-text
caches stored on `WorkItem`, changing query syntax or matching semantics, a
Unicode case-folding crate.

## PR 4 — Cheaper progressive-sync merging

**Gate:** PR 1 shows `ProjectProgress` update time grows with collection size
enough to hitch input during a cold sync.

**Scope.**
- Replace the per-page `HashMap` rebuild in `merge_items` with an in-place
  upsert by key (a transient `HashMap<ItemKey, usize>` index, or a persistent
  index in `State` if PR 1 shows that is needed), keeping the first-page
  fast path.
- Skip normalisation/selection work when a progress message carries no items
  (the `saving`/`reconciling` reports), and avoid re-finding the selection when
  nothing visible changed.
- Do not change message cadence unless PR 1 shows redraw pressure; any
  coalescing must keep every item delta.

**Files/areas.** `src/ui/controller.rs` (`merge_items`, `ProjectProgress`,
`ProjectCached`, `ProjectLoaded`), possibly `src/ui/mod.rs`; `src/gitlab.rs` only
if message cadence is changed.

**Acceptance.**
- Every item delta is applied (test: replayed page sequence yields the same set
  and sorted order as before); prompt first-page visibility, bounded buffers,
  selection identity across merges, stale-epoch guards and
  hidden-project guards are covered by tests that fail if broken.
- Existing sync/navigation tests (`tests/navigation.rs`, `tests/ui.rs`) pass.
- PR 1 sync scenario shows lower per-message time at 3,000 items.

**Out of scope.** Changing the SQLite page format, sync planning, the final
`cache.items()` read, WAL/connection pools, new background threads.

## PR 5 — Revision-keyed derived-view cache (conditional)

**Gate:** only if, after PR 3, the release harness still shows filter + sort as
a dominant cost; otherwise drop it. This is the riskiest item (stale views).

**Scope.** One cached view (matching + sorted item *indices*) in `State`, behind
an interior-mutability cell because `view` receives `&Context`. Key it by:
an `items` revision, projects (ids + visibility), active tab, effective query
text, user id/username and a current-iterations revision. `State.items` is a
`pub` field written in about seven places (`ProjectLoaded`, `merge_items`,
`DetailsLoaded` in-place enrichment, clear/retain/setup paths) and by tests that
assign it directly; replace those writes with methods that bump the revision, or
make the cache validate against a cheap content stamp. Length alone is never a key.

**Files/areas.** `src/ui/mod.rs`, `src/ui/controller.rs`, `src/ui/onboarding.rs`,
`src/ui/view.rs` (callers), tests.

**Acceptance.**
- Invalidation tests for: in-place item edit that changes sort or filter
  result without changing length, project hide/show, filter/saved-view change,
  user change, iteration change, `ProjectLoaded` replacing a project.
- Navigation with no changes performs one filter + sort per revision (assert via
  a test-only counter, not time).
- Memory: indices only; no per-frame item clones.

**Out of scope.** Caching in `Config`, persisting the cache, caching rendered
rows, metadata-only list models or DB-backed paging.

## Deferred / dropped (no PR until measured evidence exists)

- Overscan tuning, widget pooling, row-callback audit.
- Large detail (Markdown/discussion/diff) layout and variable-height
  virtualisation. Needs its own profile first; must preserve semantic
  navigation and scroll anchors.
- SQLite/HTTP mutex wait/hold profiling, WAL, read connection or pool.
- Retained-memory work and larger-than-few-thousand collections.
- Real-terminal output profiling (needs an interactive release run; headless
  tests cannot measure it).
- CI/runner/Node-20 housekeeping (already handled; separate from this work).

## Delivery constraints

Start each PR from freshly fetched `origin/main` and read current `AGENTS.md`.
Use `just` tasks only (never `cargo test`). One implementation agent, serial
heavy commands, `CARGO_BUILD_JOBS=2` and `NEXTEST_TEST_THREADS=2` on this host.
Benchmark PRs prebuild/warm both versions, measure serially with identical
fixtures, and report all samples and limitations; no flaky CI timing
thresholds. Batch changes, run targeted tests, then `just validate` before
publishing. Keep each PR to its listed scope.
