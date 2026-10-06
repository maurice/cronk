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

## Initial debug observations

On this shared Linux/WSL development host, the unchanged 30-update headless
workload produced these single-run timings (setup/compilation excluded from the
measured section):

| MR count | Original full-tree construction | Windowed construction |
| --- | ---: | ---: |
| 300 | 11.991 s | 3.917 s |
| 3,000 | 665.806 s | 5.317 s |

These runs overlapped other compilation/testing and are **not a controlled paired
benchmark**. In particular, the original 3,000-row run suffered substantial
shared-host contention. Do not interpret their ratio as a reliable production
speedup or FPS figure. The structural improvement is independently checkable:
3,000 rich row trees per view becomes at most 60 plus spacers at this viewport
size. Release-mode measurements on an otherwise idle host and a real GitLab
workspace are still needed to quantify end-user latency.

After the final build and full test suite finished, a standalone serial run of
just the updated workload recorded **3.293 s for 300 MRs** and **8.591 s for
3,000 MRs**. The cache workload recorded **1.674 s for 30 durable page
transactions containing 3,000 items**, and **205.745 ms to read/decode the
snapshot**. These remain shared-host debug observations, not isolated-machine
or production measurements; their variance reinforces the need for controlled
release profiling rather than quoting a precise speedup.

The baseline was built from `origin/main` at `4359bf8` with only
`tests/list_performance.rs` added; the timed fixture and 30-update loop were identical. To reproduce the
comparison, copy that test into an isolated main worktree, prebuild both
versions, and execute the workload serially after builds finish. The SQLite
workload uses minimal `WorkItem` bodies; description-rich production snapshots
will have higher byte counts and potentially different timings.

## Reproduce

```sh
just test-integration list_performance --test-threads=1 --success-output immediate
just test-lib -E 'test(cache::tests::large_snapshot)' --success-output immediate
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

1. **Workspace persistence on the UI thread.** `Cronk::persist` calls
   `Config::save` during navigation/scrolling. Saving serialises TOML, atomically
   replaces the file and syncs the file and parent directory. This is separate
   from SQLite and can cause frame stalls, especially on WSL/network/slow disks.
   Measure a path-backed workload against the no-path benchmark; consider
   coalescing navigation saves or a single ordered background writer with a
   shutdown flush and visible error reporting. Configuration mutations should
   not lose their persistence guarantees.
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
