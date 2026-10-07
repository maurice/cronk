# Large-list performance investigation

## Findings and changes

### List construction was not virtualised

`src/ui/view.rs::main_list` built a complete widget tree for every matching work
item on every redraw. `ScrollView` defaults to virtualised **measurement/mounting**,
but accepts already-constructed elements: this did not avoid allocating titles,
label spans, user formatting, status tooltips, mouse regions and callbacks for
all 3,000 MRs. Hover, ticks, selection and scroll updates could all incur this
cost, as could the library's hashing/layout of these trees.

The list now constructs rich rows only for the viewport and one viewport of
overscan on either side (at most 60 rich rows for 20 visible items). Leading and
trailing spacers represent the remaining fixed-height rows. The bounded tree is
measured exactly, avoiding estimated heights being skewed by the spacers. The
existing scrollbar, wheel behaviour, row heights and global activation indices
are retained. Projects and dashboard rows use the same windowing strategy.

This bounds **widget construction**, not all application work: the filtered item
vector still scales with the collection size. The native widget's coordinate
limits also remain; spacer chunking prevents truncating `Length::Px` heights but
does not establish support for arbitrarily large terminal-content coordinates.

### Filtering and sorting were repeated for count-only consumers

`State::visible_items()` parsed the query, filtered, allocated a vector and sorted
it every time, including calls that only needed a length (navigation,
normalisation, viewport callbacks and the header). Those paths now share a
count-only iterator with the same filtering semantics, without allocation or
sorting. The Projects tab no longer sorts the work-item list just to render
projects.

Sorting the actual work list and filtering for counts are still performed on
redraw/navigation. A revision-keyed derived-view cache is a possible follow-up,
with explicit invalidation for item enrichment, project visibility, current
iteration, user identity, filters and saved-view changes. Simply caching by
collection length would return stale results.

### SQLite was indexed and batched, but lock scope and statement reuse were wasteful

`src/cache.rs` uses one mutex-protected connection. Snapshot queries restrict by
project; identity queries restrict by `(project, kind, iid)`. These match existing
primary-key indexes. Page upserts, enrichment and reconciliation already use
transactions, and enrichment already writes only enriched identities rather
than rewriting all historical items on every refresh. There is no SQLite access
in ordinary list scrolling; database contention alone cannot explain steady-state
scrolling slowness.

Changes:

- Prepare/cache each repeated upsert, enrichment update and deletion statement,
  then reuse it throughout the transaction instead of parsing SQL for each row.
- Serialise item batches and details **before acquiring the connection mutex**.
- Copy snapshot JSON bodies under the mutex, then decode outside it. Detail
  decoding also happens after releasing the connection guard. Snapshot reads
  briefly retain the JSON strings alongside decoded items, trading some peak
  memory for shorter lock holds; profile this for description-heavy histories.
- Skip empty enrichment transactions.
- Retain transaction atomicity, resumable cursors, credential isolation and
  `synchronous=FULL`. No schema migration or durability downgrade.

Tests cover 3,000-row page batches, replay, enrichment alongside concurrent reads,
reconciliation, malformed-page rollback, and query plans using indexes without
full-table scans or a temporary sort.

WAL alone would not make concurrent operations through the **same mutex/connection**
parallel. A separate read connection or pool would be needed to realise that
benefit, plus careful handling of cache-clear/generation ordering and secure WAL
sidecars. This change intentionally does not add that complexity without measured
lock-wait evidence. The HTTP cache generation guard still protects persistent
writes against concurrent GitLab mutations.

## Controlled before/after benchmarks

After rebasing and clearing the host memory pressure, three serial paired runs
showed a **54.1× speedup for the 3,000-MR list workload**: median **87.062 s →
1.609 s**, a **98.2% reduction**. With 300 MRs the median improved **3.74×**.
Increasing the collection tenfold now increases this workload's median time by
only about 15%, rather than rebuilding thousands of rich widget trees per view.

### List: 30 selection/scroll updates

All values are seconds; setup and compilation are outside the timed section.

| Pair | 300 MRs before | 300 MRs after | 3,000 MRs before | 3,000 MRs after |
| --- | ---: | ---: | ---: | ---: |
| 1 | 5.232 | 1.399 | 88.393 | 1.511 |
| 2 | 5.234 | 1.534 | 87.062 | 1.902 |
| 3 | 4.799 | 1.375 | 85.586 | 1.609 |
| **Median** | **5.232** | **1.399** | **87.062** | **1.609** |

### SQLite: 3,000 minimal item bodies

Writes comprise 30 separate 100-item page transactions, preserving
`synchronous=FULL`. Reads include retrieving and JSON-decoding all 3,000 items.
All values are milliseconds.

| Pair | Write before | Write after | Read/decode before | Read/decode after |
| --- | ---: | ---: | ---: | ---: |
| 1 | 372.674 | 207.812 | 37.371 | 48.121 |
| 2 | 253.695 | 193.713 | 37.978 | 35.995 |
| 3 | 254.039 | 191.776 | 36.090 | 36.763 |
| **Median** | **254.039** | **193.713** | **37.371** | **36.763** |

Page writes improved **23.7%** (1.31×). Snapshot read/decode throughput was
effectively unchanged: the intended read-side improvement is moving JSON work
outside the mutex, not making the JSON parser faster. These timings do not
quantify mutex-wait improvements; the concurrent regression test checks
correctness, and lock-wait profiling remains a follow-up.

### Environment and method

- Linux/WSL2 `6.18.33.2-microsoft-standard-WSL2`; Intel Core i7-1185G7, 4 physical
  cores / 8 logical CPUs; 7.7 GiB guest RAM on a 16 GiB Windows host.
- Repository-managed Rust 1.99.0 and nextest 0.9.146. Both versions use the same
  current test/dev profile (`debug=1`, unoptimised), dependencies and tool pins;
  neither worktree enables optional mold/sccache configuration.
- Baseline: latest main at `dd01356`, with only the identical list test file and
  cache workload test function copied in. Feature: rebased `8d2ef73`. Separate
  worktree target directories; no baseline production code was modified.
- Both targets were prebuilt using `just ... --no-run`. One unmeasured warm-up
  per version/workload preceded the three measured pairs. No compilation or
  other validation ran concurrently with measurements.
- Pair order: before → after, after → before, before → after. List and cache
  commands ran sequentially through a `just` lock that has since been removed (see
  [development.md](development.md#concurrency-deliberately-no-locking)), with one nextest test process and captured successful-test output. Each command ran
  only the relevant workload. Reported values are its internal monotonic
  timers, not whole-command wall time or test-runner startup.
- During the runs, `vmstat` recorded at least **3.45 GiB free guest RAM**, no
  swap-out, and only occasional swap-in at up to **20 KiB/s**. No competing Rust
  builds/tests were running. Ordinary Windows desktop services remained active;
  this was a cleaned-up development host, not a laboratory-isolated machine.
- Medians and speedups use unrounded samples. Every warm-up/measured workload
  passed. The fixture has no network requests or workspace-file saves; SQLite
  uses fresh temporary databases and minimal bodies. Real descriptions, filters
  and disk configurations can change the numbers.

This establishes a substantial improvement in the reproducible **debug headless
workload**, not a production FPS claim. Release-mode latency has since been measured
with the [latency harness](#release-latency-harness) (headless: no real terminal
output); real-terminal and real-workspace latency remain unmeasured. The earlier contention-heavy observations (including
the 665.806 s baseline) are superseded by these paired results and should not be
used to calculate a speedup; they also used an older base/toolchain/profile.

### Host-memory investigation

WSL had been up for about 15 days. Before cleanup it was already 95–96% CPU idle,
with about 0.8 GiB actively used and 6.7 GiB available. Approximately 5.8 GiB was
filesystem cache, not a runaway compiler or test process. Clean cache was
reclaimed once before prebuilding; no Linux editor/agent/daemon was terminated.
Several old editor/language-server processes were mostly swapped out, and zombie
processes had no resident memory to reclaim.

Windows was the important pressure source: only **0.43 GiB free**, with
**Windows Terminal committing about 6.8 GiB of private memory** after nearly two
weeks of uptime (private commitment is not the same as resident RAM). Its
resident working set was about 0.8 GiB at the first sample. The user closed
Windows Terminal and reattached through WezTerm; Windows free memory then rose
to **6.86 GiB**, and the user observed overall usage roughly halve. That is
strong evidence the terminal was a major contributor, though it does not by
itself identify the cause of its unusually large allocation.

Builds refill Linux filesystem cache, so WSL's Windows footprint can rise again
after testing without indicating a leak. Check `MemAvailable`, process memory,
and live paging rather than treating all cache or historically occupied swap
as current pressure. Restarting unrelated services or WSL was unnecessary.

## Release latency harness

`tests/latency_harness.rs` records per-gesture `dispatch` / `render` / `pump` / total
samples (median, p95, max, plus every raw sample) for 3,000 description-heavy MRs
across empty-query, text, `@me` and dashboard scenarios, with and without a
workspace path, and a 30-page progressive-sync scenario. It makes no timing
assertions. Run it in release mode (a task-specific exception to AGENTS.md):

```sh
just test --release --test latency_harness --no-run   # prebuild first
just test --release --test latency_harness -E 'test(latency_harness_release)' \
    --run-ignored only --test-threads=1 --success-output immediate
```

Results, method and limitations are in
[optimisation-next-steps.md](optimisation-next-steps.md#pr-1-results-3000-mrs-release).
`latency_harness_smoke` runs a tiny plan in the normal debug suite.

## Reproduce

```sh
# Prebuild both versions first; exclude these builds and one warm-up from results.
just test-integration list_performance --no-run
just test-lib -E 'test(cache::tests::large_snapshot)' --no-run

# Run serially in each worktree, alternating version order across three pairs.
just test-integration list_performance -E 'test(large_list_scroll_workload)' --test-threads=1 --success-output immediate
just test-lib -E 'test(cache::tests::large_snapshot)' --test-threads=1 --success-output immediate
just test-all
just doc
```

The headless workload uses a 100×68 viewport (20 three-line MR rows), identical
MR fixtures, 30 selection/scroll updates, explicit renders and event pumping.
It excludes setup from the timed section, uses no GitLab/network requests and
no workspace-file saves, and prints timings without imposing flaky CI latency
thresholds. Separate correctness coverage exercises deep jumps, returning to the
top, and clicking a globally indexed row. Existing scrollbar tests cover native
track/drag, wheel, keyboard, filtering and resize behaviour.

## Next profiling priorities

1. **Workspace persistence on the UI thread: stale.** Navigation (selection,
   scrolling, tab and route changes) is persisted by a bounded, coalescing SQLite
   writer, see [navigation-state.md](navigation-state.md). `Cronk::persist` calls
   `Config::save` (TOML serialisation, atomic replace, file and directory syncs) only
   when *portable* configuration changed (`portable_eq`), e.g. a filter edit, so it is
   not on the scrolling path. The release harness (below) measures it: `Move`/`Select`
   are indistinguishable from a no-path run, and a text-query edit costs about 2–4 ms
   more with a path (inferred from the difference, not timed directly).
2. **Derived-view work.** Measure query parsing/filtering, sorting and dashboard
   attention-rank comparisons separately. Cache a view by revisions rather than
   rebuilding it for ticks/hover; precompute ranks when sorting dashboard items.
   Project lookup currently scans configured projects for each item, and each
   rendered project counts items with another scan. Text filtering lowercases
   titles/descriptions into new strings on every match (`src/filter.rs`), so
   description-heavy collections and multiple text terms deserve allocation
   profiling too; any normalised-text cache must preserve Unicode semantics.
3. **Progressive-sync merging and event pressure.** Page reports merge the full
   item collection through a `HashMap`, restore selection and normalise the list.
   Investigate incremental keyed storage, throttling/coalescing UI progress
   events, and avoiding repeated historical clones during a cold sync.
4. **Background I/O contention.** Record connection-mutex wait/hold times,
   transaction/commit latency and snapshot bytes, including multi-project sync
   plus opening details. Record HTTP generation-lock contention as well. Only
   consider a read connection/WAL after showing that I/O rather than JSON work
   remains a blocker.
5. **Redraw scheduling and terminal output.** Profile idle ticks, pointer hover,
   reconciliation/layout/hashing and cell diff/write time in a release build on
   the user's terminal. Compare list frames with Markdown/diff/detail frames:
   `scroll_content` deliberately keeps variable-height detail children mounted
   for accurate navigation/measurement, so large descriptions/discussions/diffs
   need separate profiling. A headless debug test is a regression workload, not
   a production FPS guarantee.
6. **Memory and larger-history limits.** Measure retained `WorkItem` descriptions,
   snapshot JSON allocation and detail caches; study metadata-only list models or
   DB-backed paging if collections grow well beyond a few thousand. Native
   terminal-coordinate limits need separate coverage before claiming support for
   tens of thousands of offscreen rows.
