# Remaining optimisation work

**Already merged:** viewport-windowed lists, efficient SQLite cache batches, and
local navigation in SQLite with coalesced background writes. Do not redo these.
See [measured findings](large-list-performance.md) and
[navigation design](navigation-state.md) for context.

## Priorities

1. **Measure realistic release latency first.** Use actual key/wheel events,
   3,000 MRs and persistence enabled. Record median, p95 and maximum latency per
   gesture; separate input/update, filtering, rendering and persistence costs.
   The earlier 1.609-second result covers **30 debug headless updates**, not one
   input or production FPS. Headless tests do not measure real terminal output.

2. **Cache derived lists and reduce filter allocations.** Avoid reparsing,
   filtering and sorting during unchanged navigation. Invalidate on item changes
   (including in-place edits), projects, filters/views, user and iterations—not
   merely collection length. Preserve Unicode matching semantics; repeated
   description lowercasing is a candidate hotspot.

3. **Reduce progressive-sync work.** Measure repeated collection merges and
   progress events. Preserve every item delta, prompt first-page visibility,
   bounded buffers, selected identity and stale-response guards. Also investigate
   old `UserLoaded` responses overwriting a newly validated user after a client
   or credential switch.

4. **Tune rendering only with evidence.** Compare current overscan with 2–4 extra
   rows per side. Retain deep-jump, resize, drag/wheel and click-identity coverage.
   Rows already have stable keys; audit index-captured callbacks during reorder.
   Do not introduce widget pooling without a demonstrated bottleneck.

5. **Profile large details and database contention.** Study Markdown/discussion/
   diff layout and retained memory. Measure SQLite/HTTP mutex wait and hold times
   before adding pools or WAL. Variable-height detail virtualisation must preserve
   semantic navigation and scroll anchors.

**Last, separately:** audit Node-20 action warnings (`actions/cache@v4`, including
composite actions) and pin/review Ubuntu runners before the `ubuntu-latest`
migration to Ubuntu 26.

## Delivery constraints

Start each small, independently reviewable PR from freshly fetched `origin/main`;
read current `AGENTS.md`. Keep one implementation agent and serial heavy commands,
with `CARGO_BUILD_JOBS=2` and `NEXTEST_TEST_THREADS=2` on this host.

Prebuild/warm both benchmark versions, measure serially with identical fixtures,
and report all samples and limitations. Avoid flaky CI timing thresholds. Batch
changes, run targeted `just` tests and required final validation, then publish—do
not turn one PR into an open-ended architecture/review exercise.
