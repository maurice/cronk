# Portable configuration and machine-local navigation

## Boundary and identity

`Config::load` reads/validates portable TOML without disk writes. `Config::save`
atomically and promptly saves deliberate configuration, retaining any on-disk
legacy navigation keys until migration succeeds. The public navigation fields
remain in memory for existing `Cronk` construction sites, tests and examples;
new saves do not include them. UI initialization restores local navigation
separately, including in demo or client-less onboarding. A `Cronk` with `path:
None` does not open a database, start a writer or save configuration.
`--demo --snapshot` uses that no-path construction: it previews portable TOML
(and any legacy navigation still present there) in memory, not the machine-local
SQLite session. Preview selection/viewport changes are ephemeral; only the
requested PNG/Markdown output is written. Existing TOML and state databases are
neither migrated nor changed.

`workspace.toml` uses `workspace.state/<namespace>.sqlite3`. The namespace hashes
the installation URL (ignoring trailing slashes) and demo/live mode, **not the
token or token environment-variable name**. The snapshot also includes that
namespace, so copying a foreign database under another filename is rejected.
This directory is separate from credential-isolated `.cache/` content databases.
Clearing the content cache preserves navigation, as does token rotation.
On Unix, state directories are `0700` and databases `0600`; directory/database
and SQLite journal/WAL/SHM symlinks are rejected. As with the existing content
cache, this assumes trusted parent directories, not protection against an
attacker concurrently replacing paths in a writable parent. State is not encrypted.

Snapshots contain only active tab identity and each tab's item/project selection,
item/project route, section/field cursors, list/document offsets and explicit job
expansion/collapse choices. No credentials, item bodies, trace bodies, log-reading
positions, searches, focus/zoom state or unsent drafts are included. A temporary
`Legacy(index)` selection is permitted only when imported TOML (or compatibility
construction input) truly has no identity. Migration prefers the open item/project
route even without a selections map, and resolves valid Projects indices against
the portable project list. Only anonymous item indices wait for global successful
hydration before being converted to an identity.

Built-in tabs have fixed identities. Saved tabs use a hash of name/kind/query,
not their numeric position. Reordering/deleting views cannot restore a neighbor's
state. Renaming through the UI preserves that tab's navigation in the next
snapshot. An external rename, replacement or query-definition edit resets local
state conservatively rather than guessing which old view it meant. Per-tab
context hashes also invalidate state when effective filters or the set of project
IDs/visibility changes; project ordering alone does not invalidate it.

Item identities are resolved against filtered, currently visible rows rather than
an old sorted index. Pending restoration survives partial hydration and waits for
the authenticated user needed by `@me`/dashboard predicates. The owning project's
successful snapshot resolves or rejects its known selections/routes independently
of unrelated project failures. Project identities use the authoritative portable
project list. Failed requests are not evidence of deletion. Explicit user list navigation takes ownership, so
later pages cannot snap the cursor back to a previously restored identity.
Section/field movement, document scrolling and dialog input do not cancel the
list's pending identity. Hiding/removing its project discards impossible pending
state immediately; a committed filter establishes a new list context and clears
the old pending detail/selection. In detail/section scope, selection follows the
open route rather than its old sorted slot: in-place detail refreshes can reorder
that slot. Back resolves the open identity against the current view and highlights
it when still eligible, or leaves it pending for its owning project's hydration.

## Migration ordering

1. Open/check the private state database and schema version.
2. Read any already-committed local snapshot; it is authoritative on retry.
3. If legacy keys exist but no local snapshot does, commit the complete imported
   navigation snapshot in one SQLite transaction (`synchronous=FULL`, DELETE
   journal). Sync the state directory and its workspace parent on Unix before
   removing legacy keys, including on interrupted-cleanup retries.
4. Only then atomically save portable TOML without those keys.

Opening, reading, schema or migration-commit failures leave TOML recoverable and
use memory navigation with a visible warning. Cleanup failures report that
SQLite migration committed and leave legacy keys for a later retry. An interrupted
cleanup cannot overwrite the committed snapshot with stale TOML. Main's pre-UI
config save preserves original on-disk legacy values, rather than replacing them
with defaults or subsequently changed in-memory navigation.

## Scheduling and lifetime

One owned worker serializes snapshots and commits SQLite. The UI compares
portable fields directly; selection/scrolling never serialize or fsync TOML.
Navigation commits replace one bounded pending snapshot under a short mutex.
The worker never holds that mutex while serializing, acquiring a database lock or
fsyncing. At most one snapshot is pending and one is in flight, independent of
input volume. Installation changes reuse this worker and open the new namespace
on it, not during UI commits.

The first dirty update sets a **2.5-second** deadline. Subsequent input replaces
the snapshot without extending that deadline. Continuous scrolling therefore
flushes periodically rather than waiting for idle time forever. Unchanged UI
snapshots are not submitted. Failed writes retain the latest snapshot (a newer
one wins) and retry on the same bounded schedule. SQLite lock waits are limited
to 250 ms; slow filesystem operations can still delay durability beyond the
nominal interval. Writer errors, including caught write panics, surface through
the UI's existing tick/status reporting.

`State::flush_navigation()` explicitly waits for the latest accepted revision and
returns failure; it is a durability boundary for headless clients and tests.
Orderly Quit flushes. A failed flush keeps the UI open with an error; Quit again
retries, then permits exit with a stderr warning if saving still fails. Unmount
also reports errors. The worker's Drop stops scheduling, attempts the latest
pending snapshot and joins exactly once, with no detached thread or retry loop.
Drop cannot return an error, so failures are printed to stderr.

Do not run multiple Cronk instances against the same workspace. SQLite commits
are atomic, but there is no cross-process session ownership or conflict policy;
portable configuration retains the existing single-instance contract. Killing
the process can lose the most recent coalescing window, not the last committed
snapshot. Startup migration is intentionally synchronous; routine UI commits
are not.

## Informational write-count evidence and tests

```sh
just test --lib -E 'test(navigation::tests)' --success-output immediate
just test --test navigation
just test --test ui -E 'test(restart) | test(flush) | test(persist) | test(deleting) | test(restored)'
```

The informational workload simulates **10,000 navigation updates over 10 seconds**
using the same first-dirty scheduling logic with fake instants. It makes **4
background SQLite snapshot commits** (three periodic deadlines plus the final
flush) and **0 TOML writes**, and prints actual snapshot/database byte counts.
One development-profile run recorded **453 snapshot bytes** and an **8,192-byte
SQLite file** for this fixture. This is deterministic scheduling/write-count evidence, not a release latency or
live-GitLab benchmark. The fixture uses actual item identity and document offset
changes, not a timing threshold. A blocked-writer test submits another 9,999
snapshots without waiting for the write, and verifies only the latest survives.

Integration coverage checks unchanged TOML bytes/inode during navigation after
legacy migration, explicit
and Drop restart durability, partial hydration with/without intervening user
navigation, item reordering/deletion, saved-view reorder/rename/redefinition,
filter/visibility invalidation (including hide/delete/re-add while pending), late
authentication after failed hydration, detail movement before the saved MR arrives,
SQLite failures/recovery and warned exit with permanent failures, migration fallback,
content-cache independence and no-path operation. Same-installation setup keeps
navigation but clears credential-specific detail/trace/iteration metadata. Demo
rebuilds current iterations from fresh fictional items, using the same seed logic
as startup, while live mode waits for new hydration. CLI
coverage checks missing storage, legacy TOML and an existing database remain
untouched by snapshots. Unit coverage additionally
checks atomic migration failure/cleanup retry and pre-cleanup directory-sync
failure after SQLite has committed, private paths, demo/installation
separation, namespace changes using one worker, queue pressure, fake-clock
continuous activity, explicit flush, failed Drop and panic reporting. Existing
per-tab restart, view-deletion/reindexing and job-collapse tests now use explicit
local durability boundaries instead of claiming immediate TOML navigation saves.
