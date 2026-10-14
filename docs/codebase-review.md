# Codebase review: structure, navigation, and Rust best practices

A review of `cronk` as it stands on `main` (~25.8k lines of `src/`, ~14.4k of
tests, plus tooling and docs). The goal is twofold:

1. **Make the code easier to navigate** — so a new contributor can answer
   "where does X live?" without reading three thousand lines first.
2. **Bring the project in line with Rust / TUI / binary conventions** — flag
   the places it currently goes against the grain.

This document is deliberately split into **small, self-contained slices** (see
[Suggested slicing](#suggested-slicing)). Each slice is meant to become its own
PR, with the cheap / low-risk extractions first and the structural ones last.

This is a documentation-only change; it proposes work, it does not perform it.

---

## Table of contents

- [Executive summary](#executive-summary)
- [Part 1 — Making the code easier to navigate](#part-1--making-the-code-easier-to-navigate)
  - [1a. Introduce a `Backend` trait](#1a-introduce-a-backend-trait-highest-value)
  - [1b. Split `gitlab.rs` along a Transport / resource axis](#1b-split-gitlabrs-along-a-transport--resource-axis)
  - [1c. Split `ui/view.rs` by screen](#1c-split-uiviewrs-by-screen)
  - [1d. Fix three inverted / duplicated dependencies](#1d-fix-three-inverted--duplicated-dependencies)
  - [1e. Consolidate the "private file" helper](#1e-consolidate-the-private-file-helper)
  - [1f. Smaller, high-clarity moves](#1f-smaller-high-clarity-moves)
  - [1g. Tests: extract the harness](#1g-tests-extract-the-harness)
- [Part 2 — Rust / binary best practices](#part-2--rust--binary-best-practices)
  - [2a. `anyhow` with structure smuggled back out via downcast](#2a-anyhow-with-structure-smuggled-back-out-via-downcast)
  - [2b. A `#[cfg(test)]` method inside a production `impl`](#2b-a-cfgtest-method-inside-a-production-impl)
  - [2c. No `[lints]` table, no `clippy.toml`, no crate attributes](#2c-no-lints-table-no-clippytoml-no-crate-attributes)
  - [2d. `lib.rs` `pub` means "reachable from tests", not "public API`](#2d-librs-pub-means-reachable-from-tests-not-public-api)
  - [2e. Blocking HTTP + 24 mutex-poison recoveries](#2e-blocking-http--24-mutex-poison-recoveries)
  - [2f. Version `0.0.0` in `Cargo.toml`](#2f-version-000-in-cargotoml)
  - [2g. Duplicated micro-logic the compiler could catch](#2g-duplicated-micro-logic-the-compiler-could-catch)
  - [2h. `filter.rs` silently swallows malformed queries](#2h-filterrs-silently-swallows-malformed-queries)
  - [2i. Docs: duplication and staleness as onboarding friction](#2i-docs-duplication-and-staleness-as-onboarding-friction)
- [Suggested slicing](#suggested-slicing)
- [What is already good](#what-is-already-good)

---

## Executive summary

Cronk is a careful codebase. It has zero `unsafe`, no `unwrap()` in production
paths, a genuinely well-tested `scroll.rs` with property-based tests, a
hand-rolled HTTP mock instead of an extra dependency, and good invariants
documented in module headers. The problems below are about **organisation and
convention**, not correctness.

The single biggest structural observation:

> Three files are ~60% of the codebase — `gitlab.rs` (6.6k), `ui/view.rs`
> (4.8k), `ui/controller.rs` (4.1k) — and none is organised by anything a
> newcomer can hold in their head. **Line count is a symptom; the disease is
> that there is no axis of classification.** Everything is flat.

The single highest-value change is introducing a **`Backend` trait** (§1a). It
removes the *conceptual* duplication (the fake API silently kept in sync with
the real one) rather than just textual duplication, and it eliminates a whole
category of "how do these two things stay in sync?" from a reader's head.

---

# Part 1 — Making the code easier to navigate

## 1a. Introduce a `Backend` trait (highest value)

Today `demo` is a **parallel API kept in sync by hand**, not an implementation
of a shared interface. Every network call site branches:

```
src/ui/controller.rs:98    let items = if is_demo { demo::items() } else { vec![] };
src/ui/controller.rs:762   ctx.state.details = ctx.state.demo.then(|| demo::details(&key));
src/ui/controller.rs:2470  trace.append(&demo::trace(id, ctx.state.tick), true);
src/ui/controller.rs:2828  let fresh = demo::project_pipelines(project, page);
src/ui/controller.rs:2868  let result = demo::pipeline_with_jobs(project, id)
src/ui/controller.rs:2928  let latest = demo::project_pipelines(id, 1).into_iter().next();
src/ui/controller.rs:3221  Some(demo::details(&key))
src/ui/mod.rs:1026         (NOW_UNIX)
```

Plus the smell that gives it away: `demo.rs:489` imports
`crate::gitlab::PIPELINE_PAGE`. The *fake* data source depends on the *real*
client for a constant — a strong sign the boundary isn't real.

Suggested shape (sketch, not final):

```rust
// src/backend/mod.rs
pub trait Backend: Clone + Send + 'static {
    fn projects(&self) -> Vec<Project>;
    fn list(&self, project: u64, kind: ItemKind, page: usize) -> Result<Page<WorkItem>, Error>;
    fn details(&self, key: &ItemKey) -> Result<Details>;
    fn project_pipelines(&self, project: u64, page: usize) -> Result<Pipelines<ProjectPipelines>>;
    fn pipeline_with_jobs(&self, project: u64, id: u64) -> Result<PipelineJobs>;
    fn trace(&self, job: &JobRef) -> Result<TraceChunk>;
    fn mutate(&self, m: &Mutation) -> Result<...>;
    fn current_user(&self) -> Result<User>;
}
```

Why it pays for itself four ways:

- The ~8 branch sites collapse into one `state.backend.clone()` at the top of
  each handler.
- The signature compatibility that currently *silently drifts*
  (`demo::project_pipelines` returns `Result<_, String>` while `gitlab::`
  returns `anyhow::Result`) becomes a **compile error**.
- `Error` finally becomes one type instead of `anyhow::Result` + `String` +
  `&str` (see §2a).
- `demo` becomes a first-class *federated* backend, which is also how you'd add
  a GitHub or local-git backend later.

**Bonus**: this is what lets you collapse `Cronk { api: Option<GitLab>, demo:
bool }` (a 3-state boolean soup) into one `backend: Backend`.

---

## 1b. Split `gitlab.rs` along a Transport / resource axis

The transport layer is cleanly identifiable and depends on only 4 fields
(`client`, `api`, `token`, `cache`):

```
url 543 · redacted 805 · context 814 · send 819 · status_error 845
document 877 · decode 939 · get 971 · pages* 1018-1071 · next_page 1073
optional 1153 · HttpFailure 204 · Document 228 · Cache 234 · with_page 2446
```

Extract that as `src/gitlab/transport.rs` → `struct Transport { client, api,
token, cache }` (~600 lines). Then the 1,982-line `impl GitLab` immediately
decomposes into per-resource files that each stay under ~500 lines:

```
src/gitlab/
  mod.rs              GitLab struct, new(), enable_persistence
  transport.rs        URL/redaction/send/status/decode/paginate/cache   ~600
  error.rs            Error enum (replaces HttpFailure + SyncPaused + http_status())
  items.rs            list_project, sync_project, reconciliation       ~450
  details.rs          details, details_progress, detail_part          ~250
  pipelines.rs        project_pipelines, latest_pipeline, with_jobs    ~150
  pipelines/telemetry.rs  (project → pipelines, jobs, jobs → trace)   ~230
  trace.rs            trace reading + Sanitizer/EscapeMode DFA         ~730
  sanitizer.rs        EscapeMode + Sanitizer  ← ZERO GitLab dependency
  write.rs            mutate, set_iteration, edit_value whitelist     ~200
  lookup.rs           lookup, current_iteration, IterationMatch        ~200
  wire.rs             ApiItem/ApiPipeline/ApiLabel → model conversion ~190
  tests/              the mock HTTP server + all tests
```

**`sanitizer.rs` is the easiest win in the repo**: `EscapeMode` + `Sanitizer`
(`gitlab.rs:2750–2941`) have no dependency on `GitLab`, no `reqwest`, nothing.
It's a pure streaming state machine over bytes that just happens to live
inside an HTTP client file. Moving it changes zero behaviour and removes ~190
lines from the worst file.

**Tests inside the client**: `gitlab.rs:3692–3869` is a 180-line hand-rolled
`TcpListener` HTTP server. It's reusable — `tests/extra_ca_certs.rs` builds a
real TLS server. Extract to `tests/support/mock.rs` so both can use it.

---

## 1c. Split `ui/view.rs` by screen

The seams are clean because `controller.rs` contains **zero** `Text::new`,
`HStack`, `Span::new`, `Style::new`, or `Color::` — the view/controller
separation is already correct and worth keeping. So the split is mechanical:

```
src/ui/
  mod.rs           State, Msg, Action, COMMANDS  (currently 1,395 lines)
  controller.rs    the event loop
  theme.rs         Colors, parse_color, SyncColorCycle     ← ~330 lines, pure
  format.rs        status_help, user_help, project_name, HELP,
                   PROJECT_FIELD_LABELS, pipeline_row_spans  ← ~420 lines, PURE STRINGS
  keys.rs          the keymap  ← fixes an inverted dependency (see 1d)
  chrome.rs        brand, tab_bar, breadcrumb, context_line, footer, view()
  list.rs          main_list, work_row, project_row
  project.rs       project_detail, project_fields, project_pipelines, project_forget
  detail.rs        detail_document, fields, note_view, discussions, changes
  logs.rs          log_window, job_panel, jobs   (extend the existing file)
  dialog.rs        dialog_view, completion_view, completion_row
```

The `format.rs` extraction is disproportionately valuable: ~420 lines of pure
string logic that today can only be tested through a headless `TestBackend`.
Once extracted, `status_help`, `job_header`, `pipeline_facts` etc. become
trivial unit tests. Notably `model::pipeline_status` (`model.rs:903`) has
**exactly one production caller (`view.rs:2899`) and its only test lives in
`view.rs:4411`** — the function is already misplaced and its tests are telling
you so.

---

## 1d. Fix three inverted / duplicated dependencies

These are small, and each is a real correctness risk, not just tidiness.

**(i) `view.rs` produces `Msg`s.** `dialog_key` (`view.rs:3623–3648`) is a
key→`Msg` function living in the view layer, plus three inline `key_handler`
closures (`view.rs:546`, `3673`, `3943`). Meanwhile `tab_shortcut`
(`controller.rs:4023`) lives in the controller. There is one input model; it's
split across two layers in five places. Move it all to `src/ui/keys.rs`.
Bonus: the `HELP` text (`view.rs:4162`) and the README keybinding table can
then be *derived* from the same table instead of restated in six places (see
§2i).

**(ii) Element keys built as strings in two files.**
`format!("dialog-field-{i}")` is constructed in `controller.rs:1696,1737,3348,3422`
for focus requests and in `view.rs:3684,3704,3759,3760,3763,3927` for
`.key(...)`. A rename in one file silently breaks focus with no compile error.
One `fn dialog_field_key(i: usize) -> String` in `dialog.rs`.

**(iii) "How many rows are in the list?" is computed five times.** `view.rs:929`,
`controller.rs:1410`, `2274`, `3245`, `3303` all re-derive
`if active_tab == 1 { projects.len() } else { visible_item_count() }`. Collapse
to `State::list_metrics(viewport_h) -> ListMetrics`, which the view *and* the
controller both call. This is the actual reason `controller.rs:2167` has to call
into `view::list_render_window` — the controller depends on the view because the
geometry rule lives in the wrong module. Move the rule to `scroll.rs` (which
already owns exactly this kind of invariant) and the cross-module call
disappears.

---

## 1e. Consolidate the "private file" helper

This is the clearest piece of textual duplication in the repo. `cache.rs` and
`navigation.rs` each independently implemented the same chain: reject a
symlink → create the directory 0700 → re-apply `set_permissions(0700)` → open
the database with `OpenOptions` mode 0600 → re-apply `set_permissions(0600)`.
`config.rs` shares the first three steps. The comment in `navigation.rs` ("As
with the content cache, restrict the private directory and database") is the
code admitting it. The two hand-rolled hex-SHA-256 helpers were byte-identical.

One `src/fsx.rs` — exporting `is_symlink`, `reject_symlink`, `private_dir`,
`owned_dir`, `private_file`, `set_mode_handle`, `sync_dir` and `hex_sha256` —
collapses the first five steps of that chain. It does **not** collapse the
`user_version` gate and PRAGMA block, which `cache.rs` and `navigation.rs`
still duplicate verbatim; that is the natural follow-up if these are ever
unified further.

**Caveat, learned the hard way while landing this.** The three sites are not
the same, and flattening them into one helper introduces real regressions:

- `cache.rs` and `navigation.rs` reject a symlinked directory and re-apply 0700
  to one that already exists. `config.rs` does **neither** — and must not. The
  config path is the one user-supplied path in the crate: a dotfiles layout
  symlinking `~/.config/cronk` is ordinary, and `--config config.toml` resolves
  its parent to the *working directory*. Re-permissioning or refusing either
  would change something the user owns. Hence two helpers: `owned_dir`
  (Cronk-owned, enforce 0700) and `private_dir` (caller-supplied, create 0700
  but leave an existing directory alone), with symlink rejection left to the
  caller.
- The directory fsync was `#[cfg(unix)]` at the call site, guard and `?`
  included. Opening a directory for sync is not portable, so on Windows the
  original *skipped* the step; hoisting it into an unguarded helper turned a
  skip into an error and would have failed every config save. The guard has to
  travel with the code — including into any test that asserts its behaviour.

---

## 1f. Smaller, high-clarity moves

| Move | Why |
|---|---|
| `UserFormatter` (`config.rs:161–232`, regex + size limits) → `src/ui/users.rs` | A rendering component with zero config-I/O business, consumed only by `view.rs`. `config.rs` is not a config module today; it's config + starred store + user formatter + theme catalogue. |
| `starred_tab/is_starred/starred_position/toggle_star/swap_stars` (`config.rs:404–484`) → `src/starred.rs` | 80 lines of starred-item domain logic on a TOML struct. |
| `Config::THEMES` (`config.rs:19–31`) ↔ the palette `match` (`view.rs:36`) → `ui/theme.rs` | Documented duplication *in place* ("both must be extended"). Adding a theme to one list but not the other **passes `validate()`** and silently renders as `Default`. Make it a single `Theme` enum so the compiler enforces exhaustiveness. |
| `cache.rs:366 invalidate_details` | No caller found in `src/`. Dead code the compiler can't flag (module is private but items are `pub`). |
| `model.rs` tests (`536–671`, `673–795`, `1026–1184`) → `model/tests.rs` | Three interleaved `#[cfg(test)]` islands with production code resuming between each. Production code should be one contiguous read. |
| `ui/interaction.rs` → `ui/keys.rs` | See 1d(i). |

---

## 1g. Tests: extract the harness

14,395 lines across 19 files with **zero shared support**. Ten copies of:

```rust
fn settle(ui: &mut Ui) { for _ in 0..3 { ui.pump().unwrap(); ui.render(); } }
```

Eight are byte-identical. `job_logs.rs:18` uses `0..4`; `pipelines.rs:50` and
`projects.rs:41` **reverse the statement order** (`render()` then `pump()`) —
that reads like copy-paste drift, not intent, and it's worth checking whether it
hides a real difference. `type Ui = TestBackend<Cronk>;` appears verbatim in 11
files; `mount()` in 5; `key()` in 4 under three different names; `fn config()`
in 3 with fields in different orders; `const THEMES` re-declared in
`scrollbars.rs:18` and `status_tooltips.rs:14` while `pointer_feedback.rs:16`
does it properly.

A `tests/support/mod.rs` with `settle`, `mount`, `key`, `mouse`, `text`,
`rect`, `demo_config`, and `Ui` would delete several hundred lines and — more
importantly — make every integration test start from the same definitions.

**First slice:** shared mechanics now live in `tests/support/ui.rs`, used by
the pipelines, starred-item and job-log suites. Update order and round counts
remain explicit; extraction does not assume the differing `settle` functions
are interchangeable. Closure-based `scenario`, `given`, `when`, `then` and
`and` helpers add executable steps, runtime phase checks and failure context
while keeping ordinary assertions and nextest. Eight functional UI scenarios
use this style, with mixed behaviours split into independently runnable tests.
It is **not** a project-wide test syntax: unit, property-based, performance and
other non-functional tests keep their natural Rust styles, including those
located in `tests/`. Shared helper reuse does not require scenario steps; wider
helper extraction is follow-up work. See the
[scenario writing guide](../tests/support/README.md).

---

# Part 2 — Rust / binary best practices

## 2a. `anyhow` with structure smuggled back out via downcast

This is the clearest anti-pattern in the codebase. `gitlab.rs` uses
`anyhow::Result` everywhere (50 `anyhow!`/`bail!` sites) and then *recovers*
structure by downcasting:

- `HttpFailure` (private type, `gitlab.rs:204`) exists only so `http_status()`
  (`:218`) can pull a `StatusCode` back out — and that result is then matched in
  **6+ places** (`:585, :1508, :1556, :2264, :2279`, plus
  `unsupported_enrichment`).
- `SyncPaused` (`:157`) is a sentinel error used as **control flow** for
  pagination cancellation, checked with `error.is::<SyncPaused>()` at `:1505` and
  `:1615`.

This is `anyhow` being used as a poor man's error enum. Replace with:

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http {status}: {message}")] Http { status: StatusCode, message: String },
    #[error("paused")] Paused,
    #[error("schema: {0}")] Schema(String),
    #[error("pagination cycle at {0}")] PaginationCycle(String),
    #[error("transport: {0}")] Transport(String),
}
```

Then `if let Error::Http { status: NOT_FOUND | FORBIDDEN, .. }` replaces every
downcast site, `Paused` stops being a fake error, and you get a free `match`
exhaustiveness check instead of 6 scattered status comparisons.

Note this **contradicts your own test policy**: nextest is mandatory because
libtest's 2 MiB thread stacks are too small for the recursive layout. An `Error`
enum would let `RUST_MIN_STACK` go away for error-path tests, and `thiserror` is
a single-file, no-std-clean dependency.

---

## 2b. A `#[cfg(test)]` method inside a production `impl`

`gitlab.rs:1146`:

```rust
#[cfg(test)]
fn all<T: DeserializeOwned>(&self, url: Url) -> Result<Vec<T>> { ... }
```

Test-only code living in the shipping `impl GitLab` block. With `just lint`
(`--all-targets`) it compiles, but it's a dead-code shim in production scope.
Move it to the test module as a free function or an extension trait.

---

## 2c. No `[lints]` table, no `clippy.toml`, no crate attributes

`lib.rs` is 13 lines of `mod`/`pub mod` with **no attributes at all**. The only
lint policy is `justfile:33`'s `-D warnings` on stock clippy. For a project this
size, add a manifest table and a crate-level gate:

```toml
[lints.rust]
unsafe_code = "forbid"          # you have zero unsafe — lock it in
missing_debug_implementations = "warn"

[lints.clippy]
pedantic = { level = "warn", priority = -1 }
# then allow the noise you don't want:
module_name_repetitions = "allow"   # `model::WorkItem` is correct here
missing_errors_doc = "allow"
must_use_candidate = "allow"
```

`missing_docs` is worth considering too — for a TUI, having to look up "is it
`pipeline` or `pipelines`?" is exactly the friction this review is trying to
remove.

---

## 2d. `lib.rs` `pub` means "reachable from tests", not "public API"

8 of 13 modules are `pub`. `pub mod build_info` is a 20-line version-string
module; `pub mod demo` is fictional fixture data. Meanwhile `cache`, `dates`,
`ansi`, `log_sections`, `navigation` are private *at the module level* while
declaring 20+ unreachable `pub` items inside.

The clearest tell that this is accidental: `ui/mod.rs` publicly exports
`list_row_height`, `list_height_for_tab`, `list_height` **purely so
`tests/scrollbars.rs` can recompute expected thumb positions** — and
`scrollbars.rs:291` then defines its *own* local
`fn list_height_for_tab(tab)` with the same name. Test-support geometry leaking
into the public API, then being reimplemented at the call site.

Two coherent options:

- **Binary-first**: everything `pub(crate)`, `lib.rs` exists only as a test
  target, and `tests/` move to in-crate `#[cfg(test)]` where they need internals.
- **Library-first**: commit to it, export `cache`/`navigation`, document the
  layering, and put the test geometry helpers behind a `pub mod testing` so their
  status is explicit.

Today it's neither, and a reader can't tell whether `pub` means anything.

---

## 2e. Blocking HTTP + 24 mutex-poison recoveries

`reqwest::blocking` (`gitlab.rs:47`) is fine for a TUI — it's the right call,
and the redirect policy is `Policy::none()` specifically to stop the token
leaking to a redirect target, which is a nice touch. But two consequences worth
naming:

1. **24 sites in `gitlab.rs` alone** do
   `lock().unwrap_or_else(|e| e.into_inner())`. That's a deliberate decision
   (recover from poisoning rather than cascade) but it's written out 43 times
   across the crate (`gitlab.rs:24`, `cache.rs:11`, `navigation.rs:8`). One
   `fn lock<T>(m: &Mutex<T>) -> MutexGuard<T>` helper, or a `parking_lot::Mutex`
   if you want it to be free.
2. **No observability layer at all.** No `tracing`, no structured logging.
   `controller.rs:239` and `:3570` `eprintln!("{error:#}")`, and `navigation.rs:532`
   eprintlns from a `Drop` impl — writing to stderr from a library module. For a
   long-running TUI that syncs continuously, a `--verbose` flag plus `tracing` at
   debug level would be a genuinely user-visible improvement, and it would
   replace the ad-hoc `eprintln!`s.

---

## 2f. Version `0.0.0` in `Cargo.toml`

`version = "0.0.0"` with `release.yml:88` doing:

```bash
if [[ "$(grep -c '^version = "0.0.0"$' Cargo.toml)" != 1 ]]; then ... fi
sed -i "s/^version = \"0.0.0\"$/version = \"${TAG#v}\"/" Cargo.toml
```

The release process greps for a sentinel and rewrites it in place. It works
(and the `--version` verification afterwards is genuinely thorough), but
`cargo install`, `cargo add`, crates.io metadata, and `cargo audit` all see
`0.0.0`. The idiomatic approach is to let CI verify `git describe`/tag matches
`CARGO_PKG_VERSION` rather than mutate the manifest — or, if you want a
nightly-flavoured single source of truth, `cargo-release`/`release-plz` do it
properly.

---

## 2g. Duplicated micro-logic the compiler could catch

- `attention_rank` (`model.rs:425–469`) and `attention_reasons`
  (`model.rs:471–520`) are **the same 7-condition cascade written twice** —
  seven aligned pairs of identical predicates, one returning `u8`, one returning
  `Vec<&str>` of English labels. Add a rank rule without its reason (or vice
  versa) and the sort key silently desynchronises from the tooltip. This should
  be one `const RULES: &[fn(&WorkItem) -> Option<&str>]` with both outputs
  derived from it.
- `portable_eq` (`config.rs:385–402`) hand-lists 15 fields and will silently
  miss any new one. `#[derive(PartialEq)]` + `#[serde(skip)]` on the nav keys
  would be compiler-checked.
- `SyncPaused` control flow (§2a) is the same class of problem: a
  compile-time-unrepresentable "did we abort?" question.

---

## 2h. `filter.rs` silently swallows malformed queries

`ui/mod.rs:1070` does `Query::parse(...).unwrap_or_default()`.
`config::validate` (`config.rs:572`) rejects malformed saved-view queries at
load, and `controller.rs:3745` surfaces the error to the user — but the *render*
path silently converts a bad query into "match everything". A user typing `foo:`
mid-session sees every item rather than an error. `mod.rs:1070` should render
the parse error, matching the other two call sites.

---

## 2i. Docs: duplication and staleness as onboarding friction

- **README is 43 KB / 316 lines with a 1,695-character single line** (`:173`).
  Long because of prose density, not content.
- **The `## Development` module layout lists 8 of 22 source modules.**
  `main.rs`, `cache.rs`, `navigation.rs`, `dates.rs`, `ansi.rs`,
  `log_sections.rs`, `build_info.rs`, `ui/logs.rs`, `ui/onboarding.rs`,
  `ui/interaction.rs` are all missing — including `cache.rs` and
  `navigation.rs`, which own two separate SQLite databases. A new contributor
  reading it gets no pointer to the two persistence layers.
- **README `## Development` (286–302) restates `docs/development.md` (3–24)
  almost point-for-point.** Both cross-link.
- **`docs/large-list-performance.md` (248) and `docs/optimisation-next-steps.md`
  (408) overlap near-totally** — 656 lines where ~300 would do.
  `docs/test-performance.md` is explicitly obsolete but sits alongside live docs
  with a one-line warning.
- **No `docs/README.md` index.** Nothing tells a newcomer which of 8 docs to
  read first.
- **The README keybinding table (74–109) and `view.rs:4162 const HELP` have no
  shared source of truth.** They will drift, and they already restate the same
  bindings.

The fix for most of these is the same as §1d(i): put the keymap in
`src/ui/keys.rs` as data, and *generate* both the in-app help and the README
table from it. That single change removes six hand-written copies of the
bindings.

---

# Suggested slicing

Each row is one PR. Cheap / low-risk extractions first, structural ones last, so
every PR is independently reviewable and reversible.

| # | PR | Risk | Touches | Approx. effect |
|---|---|---|---|---|
| 1 | **Extract private-file + hash helpers** (`src/fsx.rs`) — *done, #55* | Low | `cache.rs`, `navigation.rs`, `config.rs` | Deletes ~120 duplicated lines; host for hex-SHA256 |
| 2 | **Extract test harness + functional scenario pilot** (`tests/support/`) — *first slice implemented; wider helper extraction remains* | Low | `tests/*.rs` | Shares setup/input, preserves update timing, splits mixed behaviours; unit tests stay conventional |
| 3 | **Extract `ui/format.rs` + `ui/theme.rs`**, unify `THEMES` | Low | `ui/view.rs`, `config.rs`, `model.rs` | ~750 lines become unit-testable; closes the theme footgun |
| 4 | **Extract `gitlab/sanitizer.rs`** | Very low | `gitlab.rs` | ~190 lines out of the worst file; zero coupling |
| 5 | **Extract `ui/keys.rs`**, generate `HELP` + README table | Medium | `ui/view.rs`, `ui/controller.rs`, README | Fixes inverted dependency; removes 6 hand-written binding copies |
| 6 | **`gitlab/transport.rs` + `gitlab/error.rs` with `thiserror`** | Medium | `gitlab.rs` | Biggest structural win; kills downcast sites; may drop `RUST_MIN_STACK` |
| 7 | **Introduce the `Backend` trait** (after #6) | Medium | `gitlab.rs`, `demo.rs`, `ui/*` | Removes the demo/real sync burden; enables future backends |
| 8 | **Split `gitlab/` into resource modules** | Medium | `gitlab.rs` | Decomposes the 1,982-line `impl GitLab` |
| 9 | **Split `ui/` into screen modules** | Medium | `ui/view.rs` | Decomposes the 4,766-line `view.rs` |
| 10 | **`State::list_metrics`** — collapse 5-way row-count duplication | Low–Med | `ui/mod.rs`, `ui/view.rs`, `ui/controller.rs` | Removes controller→view dependency |
| 11 | **Add `[lints]` / crate attributes** (`unsafe_code = "forbid"`) | Low | `Cargo.toml`, `lib.rs` | Locks in the zero-unsafe guarantee |
| 12 | **Fix `attention_rank`/`attention_reasons`, `portable_eq`, filter error** | Low | `model.rs`, `config.rs`, `ui/mod.rs` | Removes silent-drift hazards |
| 13 | **Docs cleanup** (index, de-dup README/development, `docs/architecture.md`) | Low | `README.md`, `docs/*` | Fixes onboarding friction |

**Suggested sequencing notes:**

- Do **#6 before #7** so `demo.rs` isn't depending on `gitlab.rs` for
  `PIPELINE_PAGE` in the first place.
- **#11 and #12 are cheap and independent** — they can land any time and don't
  block anything.
- **#1 removes duplicated filesystem mechanics. #2 starts with shared test
  mechanics and a small scenario pilot**, rather than rewriting every suite at
  once. Preserve setup and event-timing differences while extracting helpers.

---

# What is already good

To set expectations for the refactors above, cronk already gets a lot right:

- **Zero `unsafe`**, and no `unwrap()` in production paths that we could find.
- **`src/scroll.rs` is a model module**: a precise module-doc invariant,
  property-based tests, and `reveal_span` that delegates to `BoundaryScroll`
  instead of duplicating the band maths. It's the best-engineered file in the
  crate.
- **The tiny modules are excellent**: `build_info.rs` (34 lines), `log_sections.rs`
  (42), `ansi.rs` (196) are each tightly scoped with colocated tests.
- **Security instincts are strong**: `redacted()` token scrubbing,
  `json_error_summary()` whitelisting, `Policy::none()` redirects (explicitly to
  protect the token), symlink rejection, 0600/0700 file modes, and a
  hand-rolled mock server instead of trusting a dependency.
- **The view/controller split is clean**: the controller has zero rendering
  code. Splitting the view is therefore mechanical, not a rewrite.
- **`filter.rs`'s parse → AST → match pipeline** is textbook and the `match`
  over all 12 predicates means adding a variant is a compile error in both
  `parse` and `matches`. That's exactly the property the rest of the crate
  could use more of.
- **The release process verifies `--version` against the tag and source commit**
  on every target binary, and the pre-commit hook plus CI enforce fmt/lint/nextest.

The proposals above are about **organisation and convention**, not about
fixing broken things. That's a good position to be in.
