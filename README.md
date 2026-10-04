# Cronk

A keyboard-first, multi-project GitLab terminal workspace, built with Rust and [tui-lipan](https://github.com/tui-lipan/tui-lipan). Short project aliases, real GitLab label colors, scoped navigation, and concurrent pipeline logs—without opening another browser tab.

**Status:** runnable first version, not complete GitLab UI parity. The REST client is tested against local mock servers; it has not yet been verified against a real installation. See [current limits](#current-limits) before relying on it for daily work.

![Cronk dashboard with colored GitLab labels](docs/dashboard.png)

## Try it

Requires a current stable Rust toolchain and a UTF-8 terminal. Truecolor and mouse reporting give the intended appearance; no Nerd Font or emoji font is required.

```sh
cargo run --locked -- --demo
```

Demo mode includes five deeply nested fictional projects, forty issues/MRs, discussions, diffs, and simulated live traces. **It never contacts GitLab and does not simulate successful remote writes.** Filters, project visibility, saved views, aliases, and themes work normally. Its workspace is saved separately as `config.demo.toml`.

For a faster optimized build:

```sh
cargo build --release --locked
./target/release/cronk --demo
```

### Connect to enterprise GitLab

```sh
cargo run --locked -- --init --host https://gitlab.company.example
```

The command prints the configuration path. Set `GITLAB_TOKEN` using your usual secret-management workflow, then:

```sh
cargo run --locked -- --check
cargo run --locked
```

Create a personal access token with `read_api` for browsing, or `api` for editing, posting, creating issues/MRs, and retrying jobs. Your account must also have the required project permissions. Do not put the token in a command argument or the TOML file. `token_env` can name a different environment variable.

Open **Ctrl+P → Add existing GitLab project**. Search by project name, choose a suggestion, and optionally set a short alias. A numeric ID or full `group/subgroup/project` path also works. This registers an existing project locally; it does not create a project on GitLab. Use separate `--config path/to/workspace.toml` files for different hosts. `--host` / `GITLAB_URL` cannot silently repoint a populated workspace to another host.

Enterprise URL prefixes such as `https://host.example/gitlab` are supported. HTTPS and native system certificate roots are used; install your company CA into the system trust store. There is deliberately no insecure-TLS switch. Redirects are rejected rather than forwarding credentials. HTTP is permitted only for loopback development/test servers.

## Navigation

| Key / gesture | Action |
| --- | --- |
| `↑` / `↓`, `Tab` / `Shift+Tab`, `k` / `j` | Move within the current scope |
| `Enter` | Open item → enter section → edit field; submit a dialog |
| `Esc` | Cancel editor, or go back exactly one scope |
| `Shift+D` / `Shift+P` / `Shift+I` / `Shift+M` | Open Dashboard / Projects / Issues / Merge Requests |
| `1`–`9`, `0` | Open saved views 1–9, then the tenth view |
| `←` / `→` | Switch to the previous / next tab (no wraparound) |
| `Esc` from a list | No action; the list remains active |
| `Home` / `End`, `PageUp` / `PageDown` | Move through a list/document |
| Mouse click / wheel | Open rows, choose tabs/sections, select fields, scroll |
| Scrollbar track click / thumb drag | Scroll the corresponding viewport without activating its contents |
| `Space` in Projects | Include/hide the selected project in other lists |
| `Ctrl+P` or `:` | Searchable command palette |
| `/` | Edit the current list's filter |
| `s` | Save an Issues/Merge Requests filter as a new tab |
| `n` | Create an issue, or MR when browsing an MR list |
| `c` in a detail | Add a comment |
| `R` / `Enter` in Discussions | Reply to the selected thread |
| `x` in Discussions | Confirm resolve/reopen |
| `r` in Jobs | Confirm retry of the selected finished job |
| `r` elsewhere / `Ctrl+R` | Refresh all visible projects and the open detail |
| `Ctrl+J` in a multiline editor | Insert a newline (`Enter` submits) |
| `?` | Keyboard guide |
| `q` / `Ctrl+C` | Quit outside text editors |

Each tab's list is active as soon as you switch to it; there is no separate tab-navigation mode. Left/Right switch tabs without wrapping; Up/Down navigate the current scope. Modified arrows do not switch tabs. The built-in tab initials are underlined, saved tabs display their number shortcuts, and the bottom gutter repeats the hints. Tab shortcuts are inactive inside dialogs/editors. More than ten saved views remain accessible by Left/Right or mouse; the tab strip scrolls horizontally when necessary.

Lists use a quarter-viewport boundary: the cursor moves to the lower/upper quarter margin, the viewport follows further movement, then the cursor reaches the actual final/first row. No wraparound. Three terminal rows form one list item; the margin rounds down to whole items.

Overflowing lists, detail panes, dialogs, and multiline editors show a theme-colored scrollbar at their right edge. The thumb indicates the visible fraction and current position; click the track or drag the thumb to scroll. Split panes scroll independently. Mouse scrolling keeps list selection visible, keyboard navigation continues from there, and unchanged refreshes do not pull the viewport back. Scrollbars disappear when content fits.

The Dashboard shows your open authored, assigned, or requested-review work. It orders failed pipelines, unresolved discussions, passing non-draft review candidates, other MRs, then issues. A review candidate is a hint, **not** a claim that approvals, mergeability, or company policy checks are satisfied. Enter opens the real detail while preserving the Dashboard as the return destination.

Issues have Fields, Description, and Activity scopes. MRs have Fields, Description, Pipeline, Jobs, Discussions, and Changes. Project rows open a project-filtered Issues list; the visibility checkbox works independently.

## Name-based field completion

Assignees, reviewers, milestones, and iterations suggest GitLab matches as you type. Use **↑ / ↓** to choose, **Enter** to insert a suggestion, then **Enter** again to save the form. Clicking a suggestion only inserts it; it never submits. **Tab / Shift+Tab** still switch fields and **Esc** cancels the editor. Press an arrow key to open suggestions without changing the text.

For assignees and reviewers, separate people with commas: `@alex, Sam`. Each lookup uses the trimmed token at the caret after the previous comma, so spaces around entries are fine. Accepting a suggestion replaces only that token, leaving the other people untouched. Suggestions show display names and usernames; accepted people use unambiguous `@username` tokens. Milestones and iterations are single-valued and may contain commas in their names. Existing assignments retain their IDs behind their readable values; an empty field clears the assignment. Unresolved names must be chosen from suggestions before saving. Numeric IDs remain available as a fallback.

Searches are debounced for **300 ms**, with at most one lookup request in flight; obsolete responses cannot replace newer results. Each request fetches at most the first **20 matches**, and up to 32 queries are cached per field for the lifetime of the dialog. Refine your search to find more specific matches. Project members are searched by name/username; milestone and iteration lookups include ancestor groups. Untitled, automatically scheduled iterations show their date range and can be searched by cadence title where supported by GitLab. Lookup errors and rate limits retain your draft.

New issue/MR forms suggest projects from your workspace by alias or path. **Add existing project** searches your GitLab memberships. Demo mode uses local fictional matches and makes no lookup requests.

## Pipelines without tab switching

![Two running jobs with concurrent live tails](docs/jobs.png)

All running jobs in the open MR's head pipeline are expanded automatically and fetched independently. Finished jobs collapse unless explicitly expanded. The Pipeline view prioritizes live panels; the Jobs view includes the whole pipeline and lets you select a finished job to inspect or retry it. Wider pipelines still require scrolling when their panels cannot fit on one screen.

Logs are **near-real-time REST polling**, not a push stream. Each job keeps a byte offset, requests new data with HTTP Range, and appends only new text. UTF-8 and terminal escape sequences are handled across chunk boundaries; OSC/DCS and terminal controls are stripped. Responses are capped at 256 KiB of source bytes; the UI retains the latest 128 KiB per job. Completed logs are drained to EOF. Panels display trailing lines rather than a full searchable log archive.

## Filters and saved tabs

Filters run locally across the loaded visible projects. Space-separated terms are ANDed; repeat `label:` to require multiple labels, negate an attribute with `-`, and quote values containing spaces:

```text
project:checkout label:"team::payments" label:"type::bug" state:opened
iteration:"Sprint 24" assignee:@me -label:blocked
reviewer:@me pipeline:success draft:false
"lease handoff" -state:closed
```

Supported attributes: `project`, `label`, `state` (alias `status`), `assignee`, `author`, `reviewer`, `iteration`, `milestone`, `pipeline`, `draft`, and `kind`. All matching is case-insensitive. Labels, states, usernames and pipeline statuses are exact matches; project aliases/paths, iteration/milestone names, and free text are substring matches. Use GitLab's `opened`, `closed`, or `merged` states. `@me` resolves to the authenticated username. Unknown syntax is rejected, not silently ignored.

Save a filter with `s` or **Save filter as a new tab**. Rename/remove it through the command palette. Changing a saved tab's active filter persists that working filter; saving again creates another named tab. The four built-in tabs cannot be renamed/deleted.

## Configuration and persistence

Default location follows the platform configuration directory (`~/.config/cronk/config.toml` on Linux). `--config` overrides it. Demo mode always substitutes the `.demo.toml` extension, even with `--config`.

Configuration contains project IDs, full paths, local aliases, visibility, views, theme overrides, active tab, list selections, open item, entered section, and selected field. Committed workspace/navigation events save immediately with a same-directory temporary file, file sync, and atomic replacement—not just on quit. Unix files are mode `0600`, with newly created directories `0700`. Invalid existing configuration is reported and not overwritten. **Do not run two instances against the same file**; there is no interprocess locking. Edit configuration while Cronk is stopped.

A minimal configuration (or use `config.example.toml` as a reference):

```toml
gitlab_url = "https://gitlab.company.example"
token_env = "GITLAB_TOKEN"
theme = "midnight"
list_refresh_secs = 60
detail_refresh_secs = 10

[[projects]]
id = 1234
path = "company/division/platform/team/planning"
alias = "Team planning"
visible = true

[[views]]
name = "My iteration"
kind = "issue"
query = 'project:planning assignee:@me state:opened'
```

Theme names are `midnight`, `dracula`, and `light`, also selectable in the palette. Optional `[colors]` overrides: `background`, `surface`, `selection`, `foreground`, `muted`, `accent`, each `"#RRGGBB"`. Overrides remain active after choosing another preset. Label colors always come from GitLab and remain intact over selection backgrounds.

### Refresh and request budget

- Visible project lists: every **60 seconds**, only while at list/tab scope.
- Active item details: every **10 seconds**, configurable down to 2 seconds (set 5 if preferred).
- Running/expanded job traces: every **2 seconds**, only for the open item; batches of at most four concurrent trace requests.
- Manual refresh: all visible projects and the active detail; does not bypass server backoff.
- Conditional GET with ETag/Last-Modified, shared across client clones; bounded **128-entry / 16 MiB** HTTP cache. Successful writes invalidate cached validators/bodies.
- Ordinary polls do not overlap for the same resource. Navigation and successful writes can supersede in-flight requests; their late completions are ignored. Failures retain the last successful rows and editor drafts, and are displayed instead of becoming empty lists.
- Exponential backoff up to 320 seconds, honoring both numeric and HTTP-date `Retry-After` values. Requests already in flight may still finish.

No background service, telemetry, third-party data service, or agent invocation is involved. Runtime network traffic goes only to your configured GitLab instance. Lists/details/traces stay in memory; the workspace file contains configuration/navigation, not cached GitLab content or tokens.

## Current limits

- **Not every GitLab field/filter is exposed.** Editors cover title, description, labels, state, assignees, milestones, issue iterations, MR target branch, and reviewers. Name completion currently covers ID-backed fields and projects, not branches or labels. Iteration lookups require GitLab Premium/Ultimate; older versions may not support cadence-title search. Permission/version failures are shown rather than treated as empty results.
- Issue activity is the reverse-chronological notes/system-notes feed, not a merged feed of every GitLab resource-event endpoint. General comments and discussion replies are supported; inline diff comments, review submission/approvals, merging, artifacts, and interactive manual jobs are not yet implemented.
- Lists currently paginate **all history** on refresh, conditionally revalidating each page. Very large projects will need incremental/keyset synchronization and periodic reconciliation before this is an efficient daily driver. There is no offline content cache. Hidden projects are not newly polled, although requests already started may finish.
- Dashboard enrichment is bounded: at most 12 recently updated open MRs per project receive pipeline/discussion enrichment, with up to 12 additional discussion-page requests. Unknown values remain unknown. Open an MR to fetch its complete detail. A passed pipeline is not sufficient evidence that an MR is ready to merge.
- Child/downstream pipelines are not traversed. Fork-owned head-pipeline jobs use the pipeline's own project. Older GitLab versions/permissions may omit optional data; warnings identify incomplete details. Server-truncated diffs are reported when detectable.
- If a server ignores Range, trace reads stream past the already-seen prefix (up to 8 MiB per request); larger prefixes produce an explicit error. Same-length log replacement cannot reliably be detected. No full-history log search/pause/archive yet.
- Unsubmitted dialog text, expansion toggles, exact document offsets, and cached data are **not restored after process death**. List selection and the open item/section/field are restored, then content is fetched again. Failed submissions retain the draft while the process is running. Writes are never automatically retried: after an ambiguous timeout, verify GitLab before retrying a create/comment to avoid duplicates.
- No live enterprise validation yet. Start with a test project/read-only token. Terminal appearance and key encodings should also be checked in your actual terminal/OS.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked --all-targets
cargo run --locked -- --demo --snapshot dashboard.png
cargo run --locked --example gallery
```

The pre-commit hook runs the formatting check and Clippy with warnings denied. It is installed by `cargo-husky` the first time you run `cargo test` after fetching dependencies.

`gallery` writes five deterministic PNG/Markdown captures to `.snapshots/` without touching your workspace or opening a terminal. Small-viewport and keyboard/mouse integration tests run through tui-lipan's actual headless runtime.

Module layout:

- `src/model.rs`: GitLab-independent domain types and mutations.
- `src/gitlab.rs`: REST pagination, conditional caching, safe trace decoding, writes.
- `src/config.rs`, `src/filter.rs`, `src/scroll.rs`: persistence, query grammar, boundary navigation.
- `src/ui/controller.rs`: explicit scopes, commands, polling, stale-response guards, dialogs.
- `src/ui/view.rs`: declarative views, themes, exact label styling, live panels.
- `src/demo.rs`: deterministic fictional fixtures.
- `tests/ui.rs`: keyboard, mouse, route/persistence, and async-result regression tests.
- `tests/scrollbars.rs`: scrollbar geometry, dragging, scrolling, resize/refresh, and dialog-focus regression tests.
- `src/ui/completion.rs`, `tests/autocomplete.rs`: comma-aware identity resolution, lookup scheduling, and autocomplete regression tests.

Licensed under Apache-2.0. tui-lipan is a separate MPL-2.0 dependency; using it does not change this application's license.
